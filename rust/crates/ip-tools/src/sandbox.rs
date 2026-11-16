//! 进程沙箱。Linux 上用 bubblewrap：新的 user/pid/net/ipc/uts/cgroup 命名空间，
//! 系统目录只读，工作目录挂到 `/workspace` 可写，其余文件系统不可见。

use std::path::{Path, PathBuf};

use tokio::process::Command;

/// 沙箱里固定的 PATH。
pub(crate) const SANDBOX_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
/// 工作目录在沙箱里的挂载点。
pub(crate) const SANDBOX_WORKDIR: &str = "/workspace";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sandbox {
    Bubblewrap {
        bwrap: PathBuf,
        /// 是否保留网络。默认 false：只有 lo。
        network: bool,
    },
    /// 不隔离，只清空环境变量并限制工作目录。仅用于开发机或没有 bwrap 的环境，需要显式选择。
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
    /// 找到 bwrap 并实际起一次空沙箱确认可用。
    ///
    /// 和真正执行工具时一样挂新的 `/proc`、`/dev` 和 `/tmp`：在 Docker 这类容器里，`/proc` 有一部分被
    /// 遮住时挂新的 `/proc` 会被拒绝。只试 `--unshare-all` 的话这里能过，员工第一次跑 shell 才失败。
    pub async fn bubblewrap(network: bool) -> Result<Self, SandboxError> {
        let bwrap = find_in_path("bwrap").ok_or(SandboxError::NotFound)?;
        let out = Command::new(&bwrap)
            .args([
                "--ro-bind",
                "/",
                "/",
                "--proc",
                "/proc",
                "--dev",
                "/dev",
                "--tmpfs",
                "/tmp",
            ])
            .args(["--unshare-all", "--die-with-parent", "true"])
            .env_clear()
            .output()
            .await
            .map_err(|e| SandboxError::Unusable(e.to_string()))?;
        if !out.status.success() {
            return Err(SandboxError::Unusable(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ));
        }
        Ok(Self::Bubblewrap { bwrap, network })
    }

    pub fn is_isolated(&self) -> bool {
        matches!(self, Self::Bubblewrap { .. })
    }

    /// 构造执行 `sh -c <script>` 的命令。环境变量一律清空，只设置 PATH/HOME/LANG。
    pub(crate) fn command(&self, workspace: &Path, script: &str) -> Command {
        match self {
            Self::Bubblewrap { bwrap, network } => {
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
                cmd.args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
                cmd.arg("--bind").arg(workspace).arg(SANDBOX_WORKDIR);
                cmd.args(["--chdir", SANDBOX_WORKDIR, "--unshare-all"]);
                if *network {
                    cmd.arg("--share-net");
                }
                cmd.args(["--die-with-parent", "--new-session", "--clearenv"]);
                cmd.args(["--setenv", "PATH", SANDBOX_PATH, "--setenv", "HOME", SANDBOX_WORKDIR]);
                cmd.args(["--setenv", "LANG", "C.UTF-8", "--", "/bin/sh", "-c", script]);
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

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}
