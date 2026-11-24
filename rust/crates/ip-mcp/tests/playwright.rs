//! 用真的 Playwright MCP 跑一遍推荐配置（`docs/ip/browser.md`）：只交出白名单里的工具，
//! 能打开本机起的网页、读快照、点链接，`file://` 被挡住。
//!
//! 要本机装好 Node.js、`@playwright/mcp` 和浏览器，所以默认跳过。跑法：
//!
//! ```sh
//! IP_TEST_PLAYWRIGHT_MCP="npx -y @playwright/mcp@0.0.86" cargo test -p ip-mcp --test playwright -- --nocapture
//! ```
//!
//! 变量的值是启动命令，按空白拆开；推荐的参数由测试自己加。

use std::collections::BTreeMap;

use ip_core::RoleId;
use ip_mcp::{McpHub, ServerConfig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const ENV: &str = "IP_TEST_PLAYWRIGHT_MCP";

/// 和 `docs/ip/browser.md`、`ip.example.yaml` 里推荐的白名单一致。
const TOOLS: [&str; 14] = [
    "browser_navigate",
    "browser_navigate_back",
    "browser_snapshot",
    "browser_find",
    "browser_click",
    "browser_type",
    "browser_fill_form",
    "browser_select_option",
    "browser_press_key",
    "browser_hover",
    "browser_wait_for",
    "browser_tabs",
    "browser_handle_dialog",
    "browser_close",
];

/// 推荐的启动参数（在启动命令之后）。
const FLAGS: [&str; 6] = [
    "--headless",
    "--isolated",
    "--no-webmcp",
    "--image-responses",
    "omit",
    "--blocked-origins=http://169.254.169.254;http://metadata.google.internal",
];

/// 本机起一个只认两个路径的 HTTP 服务，返回地址。
async fn site() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buf[..n]);
                let path = request.split_whitespace().nth(1).unwrap_or("/");
                let (status, body) = match path {
                    "/" => (
                        "200 OK",
                        r#"<!doctype html><meta charset="utf-8"><title>测试页</title><h1>标记-7F3A</h1><a href="/b.html">下一页</a>"#,
                    ),
                    "/b.html" => (
                        "200 OK",
                        r#"<!doctype html><meta charset="utf-8"><title>第二页</title><p>第二页-91QZ</p>"#,
                    ),
                    _ => ("404 Not Found", ""),
                };
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread")]
async fn playwright_mcp_with_recommended_config() {
    let Some(command) = std::env::var(ENV).ok().filter(|v| !v.trim().is_empty()) else {
        eprintln!("没设 {ENV}，跳过 Playwright MCP 测试");
        return;
    };
    let mut parts = command.split_whitespace().map(str::to_string);
    let program = parts.next().unwrap();
    let mut args: Vec<String> = parts.collect();
    args.extend(FLAGS.iter().map(|f| f.to_string()));

    let cwd = std::env::temp_dir().join(format!("ip-playwright-test-{}", std::process::id()));
    std::fs::create_dir_all(&cwd).unwrap();
    let config = ServerConfig {
        command: program,
        args,
        cwd: Some(cwd.clone()),
        roles: vec!["web".into()],
        tools: TOOLS.iter().map(|t| t.to_string()).collect(),
        timeout_secs: Some(120),
        ..Default::default()
    };
    let configs = BTreeMap::from([("browser".to_string(), config)]);
    let hub = McpHub::connect_with_env(&configs, &|var| std::env::var(var).ok())
        .await
        .unwrap();
    assert!(hub.unavailable().is_empty(), "没连上：{:?}", hub.unavailable());

    let role = RoleId::from("web");
    let mut names: Vec<String> = hub.specs_for(&role).into_iter().map(|s| s.name).collect();
    names.sort();
    let mut want: Vec<String> = TOOLS.iter().map(|t| format!("mcp__browser__{t}")).collect();
    want.sort();
    assert_eq!(names, want, "只交出白名单里的工具，且白名单里的这一版都有");
    assert!(hub.specs_for(&"other".into()).is_empty(), "别的岗位看不到");
    let bytes: usize = hub
        .specs_for(&role)
        .iter()
        .map(|s| s.name.len() + s.description.len() + s.parameters.to_string().len())
        .sum();
    eprintln!("白名单里 {} 个工具，定义约 {bytes} 字节", names.len());

    let call = |name: &'static str, args: String| {
        let hub = &hub;
        let role = role.clone();
        async move {
            let out = hub.call(&role, &format!("mcp__browser__{name}"), &args).await.unwrap();
            eprintln!("== {name} ok={}\n{}", out.ok, out.output);
            out
        }
    };

    let out = call("browser_run_code_unsafe", r#"{"code": "async (page) => 1"}"#.into()).await;
    assert!(!out.ok && out.output.contains("没有工具"), "不在白名单里的调不了");

    let base = site().await;
    let out = call("browser_navigate", format!(r#"{{"url": "{base}/"}}"#)).await;
    assert!(out.ok);
    let out = call("browser_snapshot", "{}".into()).await;
    assert!(out.ok && out.output.contains("标记-7F3A"), "快照里有页面内容");
    let line = out.output.lines().find(|l| l.contains("下一页")).expect("快照里有链接");
    let reference = line
        .split("[ref=")
        .nth(1)
        .and_then(|r| r.split(']').next())
        .expect("链接带 ref");
    let out = call("browser_click", format!(r#"{{"target": "{reference}"}}"#)).await;
    assert!(out.ok && out.output.contains("/b.html"), "点链接跳到第二页");
    let out = call("browser_find", r#"{"text": "第二页-91QZ"}"#.into()).await;
    assert!(out.ok && out.output.contains("第二页-91QZ"));

    let out = call("browser_navigate", r#"{"url": "file:///etc/hosts"}"#.into()).await;
    assert!(!out.ok && out.output.contains("blocked"), "file:// 默认被挡");
    let out = call(
        "browser_navigate",
        r#"{"url": "http://169.254.169.254/latest/meta-data/"}"#.into(),
    )
    .await;
    assert!(
        !out.ok && out.output.contains("BLOCKED"),
        "元数据地址在 blocked-origins 里"
    );

    hub.shutdown().await;
    let _ = std::fs::remove_dir_all(&cwd);
}
