use std::time::{Duration, Instant};

use crate::{CommandLimits, Limits, Sandbox, Toolbox};

/// CI 设置 `IP_REQUIRE_BWRAP=1`，没有可用的 bwrap 时测试失败而不是跳过。
async fn sandboxes() -> Vec<Sandbox> {
    let mut v = vec![Sandbox::None];
    match Sandbox::bubblewrap(false, &CommandLimits::default()).await {
        Ok(s) => v.push(s),
        Err(e) => {
            assert!(std::env::var("IP_REQUIRE_BWRAP").is_err(), "要求 bwrap 但不可用：{e}");
            eprintln!("跳过 bwrap 测试：{e}");
        }
    }
    v
}

fn toolbox(sb: Sandbox, limits: Limits) -> (tempfile::TempDir, Toolbox) {
    let dir = tempfile::tempdir().unwrap();
    let tb = Toolbox::new(dir.path(), sb, limits).unwrap();
    (dir, tb)
}

fn args(v: serde_json::Value) -> String {
    v.to_string()
}

#[tokio::test]
async fn shell_runs_in_workspace_with_clean_env() {
    // 测试进程自己的环境（cargo 设置了几十个变量）不能出现在工具进程里，API key 同理。
    for sb in sandboxes().await {
        let (dir, tb) = toolbox(sb.clone(), Limits::default());
        std::fs::write(dir.path().join("hello.txt"), "你好").unwrap();
        let out = tb
            .call(
                "shell",
                &args(serde_json::json!({"command": "cat hello.txt; echo; env | sort"})),
            )
            .await;
        assert!(out.ok, "{sb:?}: {}", out.output);
        assert!(out.output.starts_with("你好\n"), "{}", out.output);
        let vars: Vec<&str> = out
            .output
            .lines()
            .filter_map(|l| l.split_once('=').map(|(k, _)| k))
            .collect();
        for k in &vars {
            assert!(
                ["PATH", "HOME", "LANG", "PWD", "SHLVL", "_", "OLDPWD"].contains(k),
                "{sb:?} 泄漏环境变量 {k}"
            );
        }
        assert!(out.output.ends_with("[退出码 0]"));
    }
}

#[tokio::test]
async fn shell_reports_exit_code_and_stderr() {
    for sb in sandboxes().await {
        let (_d, tb) = toolbox(sb, Limits::default());
        let out = tb
            .call("shell", &args(serde_json::json!({"command": "echo oops >&2; exit 3"})))
            .await;
        assert!(!out.ok);
        assert_eq!(out.output, "oops\n[退出码 3]");
    }
}

#[tokio::test]
async fn shell_timeout_kills_background_children() {
    for sb in sandboxes().await {
        let (_d, tb) = toolbox(sb.clone(), Limits::default());
        let start = Instant::now();
        let out = tb
            .call(
                "shell",
                &args(serde_json::json!({"command": "sleep 30 & sleep 30", "timeout_secs": 1})),
            )
            .await;
        assert!(!out.ok);
        assert!(out.output.contains("[超时 1 秒，已终止]"), "{}", out.output);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{sb:?} 用时 {:?}",
            start.elapsed()
        );
    }
}

#[tokio::test]
async fn shell_background_child_does_not_hold_call_open() {
    for sb in sandboxes().await {
        let (_d, tb) = toolbox(sb.clone(), Limits::default());
        let start = Instant::now();
        let out = tb
            .call(
                "shell",
                &args(serde_json::json!({"command": "sleep 30 & echo started"})),
            )
            .await;
        assert!(out.ok, "{}", out.output);
        assert!(out.output.starts_with("started\n"));
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "{sb:?} 用时 {:?}",
            start.elapsed()
        );
    }
}

#[tokio::test]
async fn shell_output_is_truncated_but_drained() {
    let limits = Limits {
        max_output: 1000,
        ..Limits::default()
    };
    for sb in sandboxes().await {
        let (_d, tb) = toolbox(sb, limits.clone());
        // 输出 2 MB，远大于管道缓冲区；如果不继续读走，子进程会卡住直到超时。
        let out = tb
            .call(
                "shell",
                &args(serde_json::json!({"command": "head -c 2000000 /dev/zero | tr '\\0' 'a'", "timeout_secs": 10})),
            )
            .await;
        assert!(out.ok, "{}", out.output);
        assert!(out.output.starts_with(&"a".repeat(1000)));
        assert!(out.output.contains("省略 1999000 字节"), "{}", &out.output[1000..]);
    }
}

#[tokio::test]
async fn bwrap_isolates_filesystem_and_network() {
    let Ok(sb) = Sandbox::bubblewrap(false, &CommandLimits::default()).await else {
        assert!(std::env::var("IP_REQUIRE_BWRAP").is_err(), "要求 bwrap 但不可用");
        return;
    };
    let (_d, tb) = toolbox(sb, Limits::default());
    let out = tb
        .call("shell", &args(serde_json::json!({"command": "touch /usr/x"})))
        .await;
    assert!(!out.ok, "系统目录应只读：{}", out.output);
    let out = tb
        .call("shell", &args(serde_json::json!({"command": "ls /root /home"})))
        .await;
    assert!(
        !out.ok && out.output.matches("No such file").count() == 2,
        "宿主目录应不可见：{}",
        out.output
    );
    let out = tb
        .call(
            "shell",
            &args(serde_json::json!({"command": "tail -n +3 /proc/net/dev"})),
        )
        .await;
    let ifaces: Vec<&str> = out
        .output
        .lines()
        .filter_map(|l| l.split_once(':').map(|(k, _)| k.trim()))
        .collect();
    assert_eq!(ifaces, vec!["lo"], "{}", out.output);
    let out = tb
        .call(
            "shell",
            &args(serde_json::json!({"command": "pwd; touch made && echo ok"})),
        )
        .await;
    assert!(out.ok, "{}", out.output);
    assert!(out.output.starts_with("/workspace\nok\n"), "{}", out.output);
    assert!(tb.workspace().join("made").exists());
}

#[tokio::test]
async fn file_tools_roundtrip() {
    let (_d, tb) = toolbox(Sandbox::None, Limits::default());
    let out = tb
        .call(
            "write_file",
            &args(serde_json::json!({"path": "a/b/研报.md", "content": "# 标题\n"})),
        )
        .await;
    assert!(out.ok, "{}", out.output);
    let out = tb
        .call("read_file", &args(serde_json::json!({"path": "./a/b/研报.md"})))
        .await;
    assert_eq!(out.output, "# 标题\n");
    let out = tb.call("list_dir", "{}").await;
    assert_eq!(out.output, "a/");
    let out = tb.call("list_dir", &args(serde_json::json!({"path": "a/b"}))).await;
    assert_eq!(out.output, "研报.md");
}

/// 两种打开方式各来一份：`openat2(RESOLVE_BENEATH)`，以及不支持它的系统上的逐级 `O_NOFOLLOW`。
fn file_toolboxes() -> Vec<(tempfile::TempDir, Toolbox)> {
    let (d1, tb1) = toolbox(Sandbox::None, Limits::default());
    let (d2, tb2) = toolbox(Sandbox::None, Limits::default());
    vec![(d1, tb1), (d2, tb2.without_openat2())]
}

fn file_call(tool: &str, path: &str) -> String {
    let mut a = serde_json::json!({"path": path});
    match tool {
        "write_file" => a["content"] = "x".into(),
        "edit_file" => {
            a["old_string"] = "s".into();
            a["new_string"] = "x".into();
        }
        _ => {}
    }
    a.to_string()
}

#[tokio::test]
async fn file_tools_reject_escapes() {
    for (dir, tb) in file_toolboxes() {
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "s").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("link")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret"), dir.path().join("leak")).unwrap();
        std::os::unix::fs::symlink(outside.path().join("absent"), dir.path().join("dangling")).unwrap();
        std::os::unix::fs::symlink("../", dir.path().join("up")).unwrap();

        for (tool, path) in [
            ("read_file", "/etc/passwd"),
            ("read_file", "../secret"),
            ("read_file", "a/../../secret"),
            ("read_file", "link/secret"),
            ("read_file", "leak"),
            ("read_file", "up/secret"),
            ("read_file", "/proc/self/root/etc/passwd"),
            ("write_file", "link/new"),
            ("write_file", "link/deep/new"),
            ("write_file", "leak"),
            ("write_file", "dangling"),
            ("write_file", "up/new"),
            ("edit_file", "leak"),
            ("edit_file", "link/secret"),
            ("list_dir", "link"),
            ("list_dir", "up"),
        ] {
            let out = tb.call(tool, &file_call(tool, path)).await;
            assert!(!out.ok, "{tool} {path} 应被拒绝：{}", out.output);
        }
        assert_eq!(std::fs::read_to_string(outside.path().join("secret")).unwrap(), "s");
        for name in ["new", "deep", "absent"] {
            assert!(!outside.path().join(name).exists(), "{name}");
        }
    }
}

#[tokio::test]
async fn relative_links_inside_the_workspace_still_work_with_openat2() {
    let (dir, tb) = toolbox(Sandbox::None, Limits::default());
    std::fs::create_dir_all(dir.path().join("reports/2026")).unwrap();
    std::fs::write(dir.path().join("reports/2026/r.md"), "s").unwrap();
    std::os::unix::fs::symlink("reports/2026", dir.path().join("latest")).unwrap();
    for (tool, path) in [
        ("read_file", "latest/r.md"),
        ("edit_file", "latest/r.md"),
        ("write_file", "latest/new/w.md"),
        ("list_dir", "latest"),
    ] {
        let out = tb.call(tool, &file_call(tool, path)).await;
        assert!(out.ok, "{tool} {path}：{}", out.output);
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("reports/2026/r.md")).unwrap(),
        "x"
    );
    assert!(dir.path().join("reports/2026/new/w.md").exists());

    // 不支持 openat2 的系统上不跟随任何符号链接，给出说明。
    let tb = tb.without_openat2();
    let out = tb.call("read_file", &file_call("read_file", "latest/r.md")).await;
    assert!(!out.ok && out.output.contains("符号链接"), "{}", out.output);
    let out = tb.call("read_file", &file_call("read_file", "reports/2026/r.md")).await;
    assert!(out.ok, "{}", out.output);
}

/// #66：另一个进程反复把工作目录里的子目录换成指向外部的符号链接，同时调文件工具。
/// 不管换在哪一刻，都不能读到、改到、建到外部。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_tools_survive_directory_swaps() {
    for (dir, tb) in file_toolboxes() {
        let ws = dir.path().to_path_buf();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("f"), "A-outside").unwrap();
        std::fs::write(outside.path().join("only-outside"), "").unwrap();
        std::fs::create_dir(ws.join("sub")).unwrap();
        std::fs::write(ws.join("sub/f"), "A-inside").unwrap();

        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let racer = {
            let (ws, target, stop) = (ws.clone(), outside.path().to_path_buf(), stop.clone());
            std::thread::spawn(move || {
                let mut flips = 0u64;
                while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                    // 每一步都是原子的 rename：sub 在真目录和指向外部的链接之间来回切换。
                    let _ = std::os::unix::fs::symlink(&target, ws.join(".lnk"));
                    let _ = std::fs::rename(ws.join("sub"), ws.join(".real"));
                    let _ = std::fs::rename(ws.join(".lnk"), ws.join("sub"));
                    let _ = std::fs::rename(ws.join("sub"), ws.join(".lnk"));
                    let _ = std::fs::rename(ws.join(".real"), ws.join("sub"));
                    flips += 1;
                }
                flips
            })
        };
        let deadline = Instant::now() + Duration::from_millis(600);
        let mut calls = 0;
        let edit =
            |old: &str, new: &str| args(serde_json::json!({"path": "sub/f", "old_string": old, "new_string": new}));
        while Instant::now() < deadline {
            let out = tb.call("read_file", &file_call("read_file", "sub/f")).await;
            assert!(!out.output.contains("outside"), "读到了外部文件：{}", out.output);
            tb.call(
                "write_file",
                &args(serde_json::json!({"path": "sub/w", "content": "W"})),
            )
            .await;
            tb.call(
                "write_file",
                &args(serde_json::json!({"path": "sub/d/w", "content": "W"})),
            )
            .await;
            tb.call("edit_file", &edit("A-", "B-")).await;
            tb.call("edit_file", &edit("B-", "A-")).await;
            let out = tb.call("list_dir", &args(serde_json::json!({"path": "sub"}))).await;
            assert!(!out.output.contains("only-outside"), "列到了外部目录：{}", out.output);
            calls += 1;
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let flips = racer.join().unwrap();
        assert!(calls > 10 && flips > 10, "竞态没跑起来：{calls} 次调用，{flips} 次切换");
        assert_eq!(std::fs::read_to_string(outside.path().join("f")).unwrap(), "A-outside");
        let leaked: Vec<_> = std::fs::read_dir(outside.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        let mut leaked = leaked;
        leaked.sort();
        assert_eq!(leaked, ["f", "only-outside"], "外部目录里多了东西");
    }
}

#[tokio::test]
async fn bad_calls_are_reported_to_model() {
    let (_d, tb) = toolbox(Sandbox::None, Limits::default());
    assert!(!tb.call("rm_rf", "{}").await.ok);
    let out = tb.call("shell", "{\"cmd\": \"ls\"}").await;
    assert!(!out.ok && out.output.contains("参数不合法"), "{}", out.output);
    let out = tb.call("read_file", "not json").await;
    assert!(!out.ok);
    assert_eq!(tb.specs().len(), 5);
}

#[test]
fn truncate_respects_char_boundaries() {
    let s = crate::truncate_output("汉字汉字".into(), 4, 0);
    assert_eq!(s, "汉\n[输出超过上限，省略 9 字节]");
}

#[tokio::test]
async fn edit_file_replaces_unique_text() {
    let (dir, tb) = toolbox(Sandbox::None, Limits::default());
    std::fs::write(
        dir.path().join("r.md"),
        "# 研报\nCPI 同比 0.2%\n结论：温和\n结论：温和\n",
    )
    .unwrap();
    let edit = |old: &str, new: &str, all: bool| {
        args(serde_json::json!({"path": "r.md", "old_string": old, "new_string": new, "replace_all": all}))
    };
    let out = tb.call("edit_file", &edit("0.2%", "0.3%", false)).await;
    assert!(out.ok, "{}", out.output);
    let out = tb.call("edit_file", &edit("结论：温和", "结论：回升", false)).await;
    assert!(!out.ok && out.output.contains("出现了 2 次"), "{}", out.output);
    let out = tb.call("edit_file", &edit("结论：温和", "结论：回升", true)).await;
    assert!(out.ok && out.output.contains("替换 2 处"), "{}", out.output);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("r.md")).unwrap(),
        "# 研报\nCPI 同比 0.3%\n结论：回升\n结论：回升\n"
    );
    assert!(!tb.call("edit_file", &edit("不存在", "x", false)).await.ok);
    assert!(!tb.call("edit_file", &edit("", "x", false)).await.ok);
    let escape = args(serde_json::json!({"path": "../r.md", "old_string": "a", "new_string": "b"}));
    assert!(!tb.call("edit_file", &escape).await.ok);
}

const MIB: u64 = 1024 * 1024;

/// 按给定上限起 bwrap 沙箱。CI（`IP_REQUIRE_BWRAP=1`）要求沙箱可用、每一项上限都加得上；本机没有 bwrap 时跳过。
async fn bwrap_with(limits: CommandLimits) -> Option<Sandbox> {
    let required = std::env::var("IP_REQUIRE_BWRAP").is_ok();
    match Sandbox::bubblewrap(false, &limits).await {
        Ok(sb) => {
            assert!(
                !required || sb.missing_limits().is_empty(),
                "有上限加不上：{:?}",
                sb.missing_limits()
            );
            Some(sb)
        }
        Err(e) => {
            assert!(!required, "要求 bwrap 但不可用：{e}");
            eprintln!("跳过 bwrap 测试：{e}");
            None
        }
    }
}

fn shell(cmd: &str, timeout: u64) -> String {
    args(serde_json::json!({"command": cmd, "timeout_secs": timeout}))
}

#[tokio::test]
async fn bwrap_reports_applied_limits() {
    let limits = CommandLimits {
        processes: Some(100),
        memory: Some(512 * MIB),
        file_size: Some(64 * MIB),
        tmp_size: Some(32 * MIB),
    };
    let Some(sb) = bwrap_with(limits).await else { return };
    if !sb.missing_limits().is_empty() {
        eprintln!("本机有上限加不上，只核对加上了的：{:?}", sb.missing_limits());
    }
    let observed = sb.observe_limits().await.unwrap();
    assert!(
        sb.limits().satisfied_by(&observed),
        "要求 {:?}，实际 {observed:?}",
        sb.limits()
    );
    if sb.missing_limits().is_empty() {
        assert_eq!(observed.processes, Some(100));
        assert_eq!(observed.tmp_size, Some(32 * MIB));
    }
    let (_d, tb) = toolbox(sb.clone(), Limits::default());
    if let Some(text) = sb.limits().describe() {
        assert!(
            tb.specs()[0].description.contains(&text),
            "{}",
            tb.specs()[0].description
        );
    }
}

#[tokio::test]
async fn bwrap_limits_file_size_and_tmp() {
    let limits = CommandLimits {
        file_size: Some(MIB),
        tmp_size: Some(2 * MIB),
        ..CommandLimits::NONE
    };
    let Some(sb) = bwrap_with(limits).await else { return };
    let (dir, tb) = toolbox(sb.clone(), Limits::default());
    if sb.limits().file_size.is_some() {
        // 单个文件超过 1 MiB 时写的进程被 SIGXFSZ 终止，文件停在上限。
        let out = tb.call("shell", &shell("head -c 3000000 /dev/zero > big", 10)).await;
        assert!(!out.ok, "{}", out.output);
        let len = std::fs::metadata(dir.path().join("big")).unwrap().len();
        assert_eq!(len, MIB, "文件应停在上限");
    }
    if sb.limits().tmp_size.is_some() {
        // /tmp 共 2 MiB：两个 1 MiB 以内的文件写不下 3 MB。
        let out = tb
            .call(
                "shell",
                &shell("head -c 1000000 /dev/zero > /tmp/a && head -c 1000000 /dev/zero > /tmp/b && head -c 1000000 /dev/zero > /tmp/c", 10),
            )
            .await;
        assert!(!out.ok, "{}", out.output);
        assert!(out.output.contains("No space left"), "{}", out.output);
    }
}

#[tokio::test]
async fn bwrap_limits_memory_per_process() {
    let limits = CommandLimits {
        memory: Some(64 * MIB),
        ..CommandLimits::NONE
    };
    let Some(sb) = bwrap_with(limits).await else { return };
    if sb.limits().memory.is_none() {
        return;
    }
    let (_d, tb) = toolbox(sb, Limits::default());
    // 字符串每轮翻倍：10 轮 1 KiB 放得下，28 轮 256 MiB 超过 64 MiB 的上限。
    let grow = |n: u32| {
        shell(
            &format!("awk 'BEGIN{{s=\"x\"; for(i=0;i<{n};i++) s=s s; print length(s)}}'"),
            20,
        )
    };
    let out = tb.call("shell", &grow(10)).await;
    assert!(out.ok && out.output.starts_with("1024\n"), "{}", out.output);
    let out = tb.call("shell", &grow(28)).await;
    assert!(!out.ok, "超过内存上限的命令应失败：{}", out.output);
}

/// 这个用户在整台机器上现有的进程和线程数。
fn tasks_of_current_user() -> usize {
    let uid = nix::unistd::getuid().as_raw().to_string();
    let Ok(dir) = std::fs::read_dir("/proc") else { return 0 };
    dir.filter_map(|e| std::fs::read_to_string(e.ok()?.path().join("status")).ok())
        .filter(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Uid:"))
                .and_then(|v| v.split_whitespace().next())
                == Some(uid.as_str())
        })
        .filter_map(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("Threads:"))?
                .trim()
                .parse::<usize>()
                .ok()
        })
        .sum()
}

#[tokio::test]
async fn bwrap_limits_processes_per_command() {
    let limits = CommandLimits {
        processes: Some(16),
        ..CommandLimits::NONE
    };
    let Some(sb) = bwrap_with(limits).await else { return };
    if sb.limits().processes.is_none() {
        return;
    }
    let (_d, tb) = toolbox(sb, Limits::default());
    // 进程数按沙箱的用户命名空间计数（Linux 5.14 起），不算服务进程和这个用户的其他进程：
    // 这个用户在外面的进程超过 16 个时，命令照样能起 8 个子进程。
    let outside = tasks_of_current_user();
    eprintln!("这个用户在沙箱外有 {outside} 个进程和线程");
    let out = tb
        .call(
            "shell",
            &shell("for i in 1 2 3 4 5 6 7 8; do sleep 1 & done; wait; echo done", 20),
        )
        .await;
    assert!(
        out.ok && out.output.starts_with("done\n"),
        "外面有 {outside} 个时：{}",
        out.output
    );
    // 超过 16 个时 fork 失败。
    let out = tb
        .call(
            "shell",
            &shell("i=0; while [ $i -lt 40 ]; do sleep 5 & i=$((i+1)); done; wait", 30),
        )
        .await;
    assert!(out.output.to_lowercase().contains("fork"), "{}", out.output);
}

#[test]
fn parses_observed_limits() {
    let text = "Limit                     Soft Limit           Hard Limit           Units     \n\
                Max cpu time              unlimited            unlimited            seconds   \n\
                Max file size             1048576              1048576              bytes     \n\
                Max data size             unlimited            unlimited            bytes     \n\
                Max core file size        0                    0                    bytes     \n\
                Max processes             100                  256                  processes \n\
                Max file locks            unlimited            unlimited            locks     \n\
                \n\
                tmpfs /tmp tmpfs rw,nosuid,nodev,relatime,size=1024k,uid=1001,gid=1001,inode64 0 0\n\
                proc /proc proc rw,nosuid,nodev,noexec,relatime 0 0\n";
    let text: String = text.lines().map(|l| l.trim_start()).collect::<Vec<_>>().join("\n");
    assert_eq!(
        crate::sandbox::parse_observed(&text),
        CommandLimits {
            processes: Some(256),
            memory: None,
            file_size: Some(MIB),
            tmp_size: Some(MIB),
        }
    );
    let no_size = "tmpfs /tmp tmpfs rw,nosuid,nodev,relatime 0 0\n";
    assert_eq!(crate::sandbox::parse_observed(no_size), CommandLimits::NONE);

    // 没设的项不比：沙箱沿用服务自己的进程数上限。
    let wanted = CommandLimits {
        processes: None,
        ..CommandLimits::default()
    };
    let observed = CommandLimits {
        processes: Some(63432),
        ..CommandLimits::default()
    };
    assert!(wanted.satisfied_by(&observed));
    assert!(!CommandLimits::default().satisfied_by(&observed));
}

#[test]
fn describes_limits() {
    assert_eq!(
        CommandLimits::default().describe().unwrap(),
        "最多 256 个进程，每个进程 1024 MiB 内存，单个文件 1024 MiB，/tmp 共 256 MiB"
    );
    assert_eq!(CommandLimits::NONE.describe(), None);
    assert_eq!(Sandbox::None.limits(), CommandLimits::NONE);
    let (_d, tb) = toolbox(Sandbox::None, Limits::default());
    assert!(!tb.specs()[0].description.contains("资源上限"));
}
