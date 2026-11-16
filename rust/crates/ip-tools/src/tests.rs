use std::time::{Duration, Instant};

use crate::{Limits, Sandbox, Toolbox};

/// CI 设置 `IP_REQUIRE_BWRAP=1`，没有可用的 bwrap 时测试失败而不是跳过。
async fn sandboxes() -> Vec<Sandbox> {
    let mut v = vec![Sandbox::None];
    match Sandbox::bubblewrap(false).await {
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
    let Ok(sb) = Sandbox::bubblewrap(false).await else {
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
