//! 进程沙箱。Linux 上用 bubblewrap：新的 user/pid/net/ipc/uts/cgroup 命名空间，
//! 系统目录只读，工作目录挂到 `/workspace` 可写，其余文件系统不可见。
//!
//! 每条命令还有资源上限（backlog E-15 第二步）：沙箱里先用 `prlimit` 设进程数、每个进程的内存、单个文件大小，
//! 再执行 `sh -c`；`/tmp` 的 tmpfs 用 bwrap 的 `--size` 限总大小。
//!
//! `prlimit` 放在沙箱里面执行，而不是包在 bwrap 外面：进程数（RLIMIT_NPROC）按真实用户计数，
//! 服务进程、MCP 子进程和所有员工的命令都是同一个用户。Linux 5.14 起这个计数按用户命名空间分开，
//! bwrap 建命名空间时自己不受限，沙箱里的上限就只数这条命令自己的进程；包在外面的话，
//! bwrap 建命名空间时会拿整台机器上这个用户的进程总数去比，服务线程一多，员工的命令就起不来。

use std::path::{Path, PathBuf};

use tokio::process::Command;

/// 沙箱里固定的 PATH。
pub(crate) const SANDBOX_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
/// 工作目录在沙箱里的挂载点。
pub(crate) const SANDBOX_WORKDIR: &str = "/workspace";
/// 沙箱里能看到的宿主目录（只读）。`prlimit` 必须在这些目录下，沙箱里才找得到。
const VISIBLE_DIRS: [&str; 8] = ["/usr", "/etc", "/opt", "/bin", "/sbin", "/lib", "/lib32", "/lib64"];

const MIB: u64 = 1024 * 1024;

/// 每条 shell 命令的资源上限。`None` 表示不限。
///
/// 进程数、内存、文件大小是每个进程的 rlimit，子进程继承，沙箱里的命令没有权限调高；
/// 内存是每个进程各算各的，不是整条命令的总和，总量的兜底是容器的 `mem_limit`（见 docs/ip/deploy-docker.md）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandLimits {
    /// 沙箱里同时存在的进程和线程数（RLIMIT_NPROC）。
    pub processes: Option<u64>,
    /// 每个进程可写的私有内存，字节（RLIMIT_DATA）。不用 RLIMIT_AS：Node.js、JVM 这类程序会预留大片
    /// 不可写的地址空间，按地址空间限会连启动都失败；RLIMIT_DATA 只数真的能写的那部分。
    pub memory: Option<u64>,
    /// 单个文件最多写多大，字节（RLIMIT_FSIZE）。超过时写的进程收到 SIGXFSZ。
    pub file_size: Option<u64>,
    /// 沙箱里 `/tmp` 的总大小，字节。tmpfs 占的是内存，不限时最多能用到一半物理内存。
    pub tmp_size: Option<u64>,
}

impl Default for CommandLimits {
    fn default() -> Self {
        Self {
            processes: Some(256),
            memory: Some(1024 * MIB),
            file_size: Some(1024 * MIB),
            tmp_size: Some(256 * MIB),
        }
    }
}

impl CommandLimits {
    /// 全都不限。
    pub const NONE: Self = Self {
        processes: None,
        memory: None,
        file_size: None,
        tmp_size: None,
    };

    /// `observed`（沙箱里实际读到的）是否满足这些上限：这里设了的每一项都要一致。没设的项不比，
    /// 那一项沿用服务进程自己的 rlimit（比如发行版给普通用户的默认进程数），不一定是不限。
    pub fn satisfied_by(&self, observed: &Self) -> bool {
        let same = |want: Option<u64>, got: Option<u64>| want.is_none() || want == got;
        same(self.processes, observed.processes)
            && same(self.memory, observed.memory)
            && same(self.file_size, observed.file_size)
            && same(self.tmp_size, observed.tmp_size)
    }

    fn has_rlimits(&self) -> bool {
        self.processes.is_some() || self.memory.is_some() || self.file_size.is_some()
    }

    /// 给人和模型看的一句话，比如“最多 256 个进程，每个进程 1024 MiB 内存，单个文件 1024 MiB，/tmp 共 256 MiB”。
    /// 全都不限时返回 `None`。
    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(n) = self.processes {
            parts.push(format!("最多 {n} 个进程"));
        }
        if let Some(b) = self.memory {
            parts.push(format!("每个进程 {} 内存", human_bytes(b)));
        }
        if let Some(b) = self.file_size {
            parts.push(format!("单个文件 {}", human_bytes(b)));
        }
        if let Some(b) = self.tmp_size {
            parts.push(format!("/tmp 共 {}", human_bytes(b)));
        }
        (!parts.is_empty()).then(|| parts.join("，"))
    }
}

fn human_bytes(b: u64) -> String {
    if b >= MIB && b % MIB == 0 {
        format!("{} MiB", b / MIB)
    } else {
        format!("{b} 字节")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sandbox {
    Bubblewrap {
        bwrap: PathBuf,
        /// 是否保留网络。默认 false：只有 lo。
        network: bool,
        /// 实际加上的上限。要求了但这台机器加不上的项已经去掉，原因在 `missing` 里。
        limits: CommandLimits,
        /// 在沙箱里设 rlimit 用的 `prlimit`；不设 rlimit 时为 `None`。
        prlimit: Option<PathBuf>,
        /// 要求了但加不上的上限，每条说明原因。
        missing: Vec<String>,
    },
    /// 不隔离，只清空环境变量并限制工作目录。仅用于开发机或没有 bwrap 的环境，需要显式选择。
    /// 不限资源。
    None,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("PATH 里找不到 bwrap，请安装 bubblewrap")]
    NotFound,
    #[error(
        "bwrap 无法创建命名空间（{0}）。Ubuntu 24.04+ 需要 sysctl kernel.apparmor_restrict_unprivileged_userns=0 或为 bwrap 配置 AppArmor 规则；在 Docker 里运行时，容器要加 --security-opt seccomp=unconfined、apparmor=unconfined、systempaths=unconfined（见 docs/ip/deploy-docker.md）"
    )]
    Unusable(String),
}

impl Sandbox {
    /// 找到 bwrap 并实际起一次空沙箱确认可用，再逐项确认 `limits` 加得上。
    ///
    /// 和真正执行工具时一样挂新的 `/proc`、`/dev` 和 `/tmp`：在 Docker 这类容器里，`/proc` 有一部分被
    /// 遮住时挂新的 `/proc` 会被拒绝。只试 `--unshare-all` 的话这里能过，员工第一次跑 shell 才失败。
    ///
    /// 沙箱本身不可用时报错，不悄悄降级；某项上限加不上（bwrap 太旧不支持 `--size`、找不到 `prlimit`）
    /// 时照样返回沙箱，那一项去掉，原因记在 `missing` 里，由调用方告警（`ip check` 报“要注意”）。
    pub async fn bubblewrap(network: bool, limits: &CommandLimits) -> Result<Self, SandboxError> {
        let bwrap = find_in_path("bwrap").ok_or(SandboxError::NotFound)?;
        probe(&bwrap, None, None).await.map_err(SandboxError::Unusable)?;

        let mut applied = *limits;
        let mut missing = Vec::new();
        if let Some(size) = limits.tmp_size
            && let Err(e) = probe(&bwrap, Some(size), None).await
        {
            applied.tmp_size = None;
            missing.push(format!("/tmp 大小不限：bwrap 不接受 --size，可能是版本太旧：{e}"));
        }
        let mut prlimit = None;
        if limits.has_rlimits() {
            match find_in_path("prlimit") {
                None => missing.push("进程数、内存、文件大小不限：PATH 里找不到 prlimit（在 util-linux 里）".into()),
                Some(p) if !visible_in_sandbox(&p) => missing.push(format!(
                    "进程数、内存、文件大小不限：{} 不在沙箱能看到的目录（{}）下",
                    p.display(),
                    VISIBLE_DIRS.join("、")
                )),
                Some(p) => match probe(&bwrap, None, Some((&p, limits))).await {
                    Ok(()) => prlimit = Some(p),
                    Err(e) => missing.push(format!("进程数、内存、文件大小不限：在沙箱里执行 prlimit 失败：{e}")),
                },
            }
            if prlimit.is_none() {
                applied.processes = None;
                applied.memory = None;
                applied.file_size = None;
            }
        }
        Ok(Self::Bubblewrap {
            bwrap,
            network,
            limits: applied,
            prlimit,
            missing,
        })
    }

    pub fn is_isolated(&self) -> bool {
        matches!(self, Self::Bubblewrap { .. })
    }

    /// 每条命令实际加上的上限。不隔离时全都不限。
    pub fn limits(&self) -> CommandLimits {
        match self {
            Self::Bubblewrap { limits, .. } => *limits,
            Self::None => CommandLimits::NONE,
        }
    }

    /// 要求了但加不上的上限，每条说明原因。
    pub fn missing_limits(&self) -> &[String] {
        match self {
            Self::Bubblewrap { missing, .. } => missing,
            Self::None => &[],
        }
    }

    /// 在沙箱里读一次 `/proc/self/limits` 和 `/proc/self/mounts`，返回命令实际受到的上限，给 `ip check` 核对。
    /// 用一个临时的空目录当工作目录。
    pub async fn observe_limits(&self) -> Result<CommandLimits, String> {
        let dir = std::env::temp_dir().join(format!("ip-sandbox-check-{}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建临时目录 {} 失败：{e}", dir.display()))?;
        let out = self
            .command(&dir, "cat /proc/self/limits; echo; cat /proc/self/mounts")
            .stdin(std::process::Stdio::null())
            .output()
            .await;
        let _ = std::fs::remove_dir_all(&dir);
        let out = out.map_err(|e| format!("启动沙箱失败：{e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
        }
        Ok(parse_observed(&String::from_utf8_lossy(&out.stdout)))
    }

    /// 构造执行 `sh -c <script>` 的命令。环境变量一律清空，只设置 PATH/HOME/LANG。
    pub(crate) fn command(&self, workspace: &Path, script: &str) -> Command {
        match self {
            Self::Bubblewrap {
                bwrap,
                network,
                limits,
                prlimit,
                ..
            } => {
                let mut cmd = Command::new(bwrap);
                for dir in ["/usr", "/etc", "/opt"] {
                    cmd.args(["--ro-bind-try", dir, dir]);
                }
                // merged-usr 的发行版上 /bin、/lib 是指向 usr/ 的符号链接，照样建链接；否则只读挂载。
                for dir in ["/bin", "/sbin", "/lib", "/lib32", "/lib64"] {
                    match std::fs::read_link(dir) {
                        Ok(target) => {
                            cmd.arg("--symlink").arg(target).arg(dir);
                        }
                        Err(_) => {
                            cmd.args(["--ro-bind-try", dir, dir]);
                        }
                    }
                }
                cmd.args(["--proc", "/proc", "--dev", "/dev"]);
                tmpfs_args(&mut cmd, limits.tmp_size);
                cmd.arg("--bind").arg(workspace).arg(SANDBOX_WORKDIR);
                cmd.args(["--chdir", SANDBOX_WORKDIR, "--unshare-all"]);
                if *network {
                    cmd.arg("--share-net");
                }
                cmd.args(["--die-with-parent", "--new-session", "--clearenv"]);
                cmd.args(["--setenv", "PATH", SANDBOX_PATH, "--setenv", "HOME", SANDBOX_WORKDIR]);
                cmd.args(["--setenv", "LANG", "C.UTF-8", "--"]);
                if let Some(p) = prlimit {
                    prlimit_args(&mut cmd, p, limits);
                }
                cmd.args(["/bin/sh", "-c", script]);
                cmd.env_clear();
                cmd
            }
            Self::None => {
                let mut cmd = Command::new("/bin/sh");
                cmd.args(["-c", script])
                    .current_dir(workspace)
                    .env_clear()
                    .env("PATH", SANDBOX_PATH)
                    .env("HOME", workspace)
                    .env("LANG", "C.UTF-8")
                    // 单独的进程组，超时时整组杀掉，不留后台子进程。
                    .process_group(0);
                cmd
            }
        }
    }
}

/// 起一个空沙箱跑 `true`。`tmp_size`、`prlimit` 用的参数和真正执行命令时是同一份（`tmpfs_args`、`prlimit_args`）。
async fn probe(bwrap: &Path, tmp_size: Option<u64>, prlimit: Option<(&Path, &CommandLimits)>) -> Result<(), String> {
    let mut cmd = Command::new(bwrap);
    cmd.args(["--ro-bind", "/", "/", "--proc", "/proc", "--dev", "/dev"]);
    tmpfs_args(&mut cmd, tmp_size);
    cmd.args(["--unshare-all", "--die-with-parent", "--"]);
    if let Some((p, limits)) = prlimit {
        prlimit_args(&mut cmd, p, limits);
    }
    cmd.arg("true");
    let out = cmd
        .env_clear()
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn tmpfs_args(cmd: &mut Command, size: Option<u64>) {
    // `--size` 只作用于紧跟着的下一个 `--tmpfs`。
    if let Some(size) = size {
        cmd.arg("--size").arg(size.to_string());
    }
    cmd.args(["--tmpfs", "/tmp"]);
}

/// `prlimit --nproc=N --data=B --fsize=B --core=0 --`：软硬上限设成同一个值，沙箱里调不高。
/// 顺带关掉 core dump，免得崩溃的程序往工作目录里写大文件。
fn prlimit_args(cmd: &mut Command, prlimit: &Path, limits: &CommandLimits) {
    cmd.arg(prlimit);
    if let Some(n) = limits.processes {
        cmd.arg(format!("--nproc={n}"));
    }
    if let Some(b) = limits.memory {
        cmd.arg(format!("--data={b}"));
    }
    if let Some(b) = limits.file_size {
        cmd.arg(format!("--fsize={b}"));
    }
    cmd.args(["--core=0", "--"]);
}

fn visible_in_sandbox(path: &Path) -> bool {
    VISIBLE_DIRS.iter().any(|d| path.starts_with(d))
}

/// 解析沙箱里 `cat /proc/self/limits; echo; cat /proc/self/mounts` 的输出。
/// rlimit 取硬上限（软上限可以被命令自己调低，硬上限调不高），`unlimited` 记为 `None`；
/// `/tmp` 取挂载参数里的 `size=`，没写时（tmpfs 的默认值，物理内存的一半）记为 `None`。
pub(crate) fn parse_observed(text: &str) -> CommandLimits {
    let hard = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name))
            .and_then(|rest| rest.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
    };
    let tmp_size = text
        .lines()
        .rev()
        .find(|l| l.split_whitespace().nth(1) == Some("/tmp"))
        .and_then(|l| l.split_whitespace().nth(3))
        .and_then(|opts| opts.split(',').find_map(|o| o.strip_prefix("size=")))
        .and_then(parse_size);
    CommandLimits {
        processes: hard("Max processes"),
        memory: hard("Max data size"),
        file_size: hard("Max file size"),
        tmp_size,
    }
}

/// tmpfs 挂载参数里的大小，内核写成 `262144k`，也认 `m`、`g` 和不带单位的字节数。
fn parse_size(s: &str) -> Option<u64> {
    let (num, mul) = match s.as_bytes().last()? {
        b'k' | b'K' => (&s[..s.len() - 1], 1024),
        b'm' | b'M' => (&s[..s.len() - 1], MIB),
        b'g' | b'G' => (&s[..s.len() - 1], 1024 * MIB),
        _ => (s, 1),
    };
    num.parse::<u64>().ok()?.checked_mul(mul)
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}
