use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig as ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};
use serde_json::json;

use super::*;

/// 测试用服务器：`echo` 原样返回 text，`fail` 返回工具级错误，`slow` 永不返回。
#[derive(Clone)]
struct Fake;

fn schema(v: serde_json::Value) -> Arc<serde_json::Map<String, serde_json::Value>> {
    match v {
        serde_json::Value::Object(m) => Arc::new(m),
        _ => unreachable!(),
    }
}

impl ServerHandler for Fake {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let s = schema(json!({"type": "object", "properties": {"text": {"type": "string"}}}));
        Ok(ListToolsResult::with_all_items(vec![
            Tool::new("echo", "原样返回", s.clone()),
            Tool::new("fail", "总是失败", s.clone()),
            Tool::new("slow", "不返回", s.clone()),
            Tool::new("odd.name/x", "名字里有怪字符", s),
        ]))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let text = request
            .arguments
            .as_ref()
            .and_then(|a| a.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        match request.name.as_ref() {
            "echo" | "odd.name/x" => Ok(CallToolResult::success(vec![ContentBlock::text(text)]).into()),
            "fail" => Ok(CallToolResult::error(vec![ContentBlock::text("坏了")]).into()),
            "slow" => {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(CallToolResult::success(vec![]).into())
            }
            _ => Err(ErrorData::invalid_params("没有这个工具", None)),
        }
    }
}

async fn hub(roles: Vec<RoleId>) -> McpHub {
    let (client, server) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = Fake.serve(server).await.unwrap();
        let _ = running.waiting().await;
    });
    let hub = McpHub::default();
    hub.add("fake", client, roles, Duration::from_millis(500))
        .await
        .unwrap();
    hub
}

#[tokio::test]
async fn lists_and_calls_tools() {
    let hub = hub(Vec::new()).await;
    let role = RoleId::from("any");
    let names: Vec<String> = hub.specs_for(&role).into_iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            "mcp__fake__echo",
            "mcp__fake__fail",
            "mcp__fake__odd_name_x",
            "mcp__fake__slow"
        ]
    );
    let spec = hub
        .specs_for(&role)
        .into_iter()
        .find(|s| s.name == "mcp__fake__echo")
        .unwrap();
    assert_eq!(spec.description, "[fake] 原样返回");
    assert_eq!(spec.parameters["properties"]["text"]["type"], "string");

    let out = hub.call(&role, "mcp__fake__echo", r#"{"text": "你好"}"#).await.unwrap();
    assert!(out.ok);
    assert_eq!(out.output, "你好");
    let out = hub
        .call(&role, "mcp__fake__odd_name_x", r#"{"text": "x"}"#)
        .await
        .unwrap();
    assert_eq!(out.output, "x", "按原名调用服务器");

    let out = hub.call(&role, "mcp__fake__fail", "{}").await.unwrap();
    assert!(!out.ok);
    assert_eq!(out.output, "坏了");

    let out = hub.call(&role, "mcp__fake__slow", "").await.unwrap();
    assert!(!out.ok && out.output.contains("没有返回"), "{}", out.output);

    let out = hub.call(&role, "mcp__fake__echo", "[1]").await.unwrap();
    assert!(!out.ok && out.output.contains("JSON 对象"));
    let out = hub.call(&role, "mcp__fake__nope", "{}").await.unwrap();
    assert!(!out.ok);
    assert!(
        hub.call(&role, "shell", "{}").await.is_none(),
        "非 MCP 工具交给别人处理"
    );
    hub.shutdown().await;
}

#[tokio::test]
async fn servers_can_be_limited_to_roles() {
    let hub = hub(vec!["ops".into()]).await;
    assert!(hub.specs_for(&"macro".into()).is_empty());
    assert_eq!(hub.specs_for(&"ops".into()).len(), 4);
    let out = hub.call(&"macro".into(), "mcp__fake__echo", "{}").await.unwrap();
    assert!(!out.ok && out.output.contains("不能用"));
}

/// `tools` 白名单：不在单子里的工具员工看不到、也调不了；harness 自己按服务器调（`call_json`）不受影响。
#[tokio::test]
async fn tools_can_be_limited_to_a_list() {
    let (client, server) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = Fake.serve(server).await.unwrap();
        let _ = running.waiting().await;
    });
    let hub = McpHub::default();
    let tools = vec!["echo".to_string(), "odd.name/x".to_string(), "missing".to_string()];
    hub.add_filtered("fake", client, Vec::new(), &tools, Duration::from_millis(500))
        .await
        .unwrap();
    let role = RoleId::from("any");
    let names: Vec<String> = hub.specs_for(&role).into_iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        ["mcp__fake__echo", "mcp__fake__odd_name_x"],
        "按原名匹配，写错的名字只记警告"
    );
    let out = hub.call(&role, "mcp__fake__fail", "{}").await.unwrap();
    assert!(!out.ok && out.output.contains("没有工具"), "{}", out.output);
    let out = hub
        .call(&role, "mcp__fake__echo", r#"{"text": "在单子里"}"#)
        .await
        .unwrap();
    assert_eq!(out.output, "在单子里");
    let e = hub
        .call_json("fake", "fail", serde_json::Map::new(), Duration::from_millis(500))
        .await
        .unwrap_err();
    assert_eq!(e, "坏了", "harness 按服务器调，不看白名单");
    hub.shutdown().await;
}

#[test]
fn tools_list_is_checked() {
    let with = |tools: &[&str]| ServerConfig {
        command: "x".into(),
        tools: tools.iter().map(|t| t.to_string()).collect(),
        ..Default::default()
    };
    assert!(with(&[]).validate().is_ok());
    assert!(with(&["browser_navigate", "browser_snapshot"]).validate().is_ok());
    assert!(with(&["a", "a"]).validate().unwrap_err().contains("两次"));
    assert!(with(&[" "]).validate().unwrap_err().contains("空的"));
    let parsed: ServerConfig = serde_json::from_value(json!({"command": "x", "tools": ["a"]})).unwrap();
    assert_eq!(parsed.tools, ["a"]);
}

#[test]
fn names_are_checked_and_sanitized() {
    assert!(check_name("github").is_ok());
    for bad in ["", "a__b", "有中文", "a b", &"x".repeat(25)] {
        assert!(check_name(bad).is_err(), "{bad}");
    }
    let long = exposed_name("srv", &"t".repeat(100));
    assert_eq!(long.len(), 64);
    assert!(long.starts_with("mcp__srv__"));
}

/// 连不上的服务器只是它自己的工具用不了：连接不报错，记进 `unavailable`，接着连下一个。
#[tokio::test]
async fn unreachable_servers_are_recorded_not_fatal() {
    let configs = BTreeMap::from([
        (
            "nope".to_string(),
            ServerConfig {
                command: "/nonexistent/mcp-server".into(),
                roles: vec!["ops".into()],
                ..ServerConfig::default()
            },
        ),
        (
            "nokey".to_string(),
            ServerConfig {
                command: "true".into(),
                env_from: [("TOKEN".to_string(), "IP_TEST_SURELY_UNSET_VAR".to_string())].into(),
                ..ServerConfig::default()
            },
        ),
    ]);
    let hub = McpHub::connect(&configs).await.unwrap();
    assert!(hub.is_empty());
    let down = hub.unavailable();
    assert_eq!(
        down.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(),
        ["nokey", "nope"]
    );
    assert!(
        matches!(down[0].error, McpError::MissingEnv { .. }),
        "{}",
        down[0].error
    );
    assert!(matches!(down[1].error, McpError::Spawn { .. }), "{}", down[1].error);
    assert_eq!(down[1].roles, [RoleId::from("ops")]);
    assert!(hub.specs_for(&"ops".into()).is_empty());

    // 对话里留着上次运行调过的工具名：告诉模型服务器这次没连上。
    let out = hub.call(&"ops".into(), "mcp__nope__get", "{}").await.unwrap();
    assert!(!out.ok && out.output.contains("没连上"), "{}", out.output);
    let out = hub.call(&"macro".into(), "mcp__nope__get", "{}").await.unwrap();
    assert!(
        !out.ok && out.output.contains("不能用"),
        "岗位本来就不能用的服务器，不说它的状态：{}",
        out.output
    );
}

fn down_configs() -> BTreeMap<String, ServerConfig> {
    BTreeMap::from([
        (
            "nope".to_string(),
            ServerConfig {
                command: "/nonexistent/mcp-server".into(),
                ..ServerConfig::default()
            },
        ),
        (
            "nokey".to_string(),
            ServerConfig {
                command: "true".into(),
                env_from: [("TOKEN".to_string(), "IP_TEST_SURELY_UNSET_VAR".to_string())].into(),
                ..ServerConfig::default()
            },
        ),
    ])
}

fn no_env() -> EnvFn {
    Arc::new(|_: &str| None)
}

/// 等到 `done` 成立，最多 5 秒。
async fn wait_until(what: &str, done: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "等了 5 秒还没有：{what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// 后台重连：每次失败记下次数和原因，试满次数就停；密钥没设的不重连。
#[tokio::test]
async fn reconnect_gives_up_after_the_configured_attempts() {
    let configs = down_configs();
    let hub = Arc::new(McpHub::connect(&configs).await.unwrap());
    let policy = Reconnect {
        attempts: 3,
        first_delay: Duration::from_millis(5),
    };
    hub.spawn_reconnect(&configs, no_env(), policy);
    let down = hub.unavailable();
    assert_eq!(down[0].name, "nokey");
    assert!(!down[0].reconnecting, "密钥没设，重连也没用");
    assert!(down[1].reconnecting);
    let out = hub.call(&"any".into(), "mcp__nope__get", "{}").await.unwrap();
    assert!(!out.ok && out.output.contains("正在重连"), "{}", out.output);

    wait_until("重连试满 3 次", || !hub.unavailable()[1].reconnecting).await;
    let down = hub.unavailable();
    assert_eq!(down[1].attempts, 3);
    assert!(matches!(down[1].error, McpError::Spawn { .. }), "{}", down[1].error);
    assert_eq!((down[0].attempts, down[0].reconnecting), (0, false));
    let out = hub.call(&"any".into(), "mcp__nope__get", "{}").await.unwrap();
    assert!(!out.ok && out.output.contains("重连 3 次也没成功"), "{}", out.output);
    let out = hub.call(&"any".into(), "mcp__nokey__get", "{}").await.unwrap();
    assert!(out.output.contains("启动时没连上"), "{}", out.output);
    hub.shutdown().await;
}

/// `shutdown` 之后后台不再重连。
#[tokio::test]
async fn shutdown_stops_reconnecting() {
    let configs = down_configs();
    let hub = Arc::new(McpHub::connect(&configs).await.unwrap());
    let policy = Reconnect {
        attempts: 5,
        first_delay: Duration::from_millis(30),
    };
    hub.spawn_reconnect(&configs, no_env(), policy);
    hub.shutdown().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    let down = hub.unavailable();
    assert!(down.iter().all(|u| u.attempts == 0 && !u.reconnecting), "{down:?}");
    // 关闭之后也不再开始重连。
    hub.spawn_reconnect(&configs, no_env(), policy);
    assert!(hub.unavailable().iter().all(|u| !u.reconnecting));
}

/// 没连上的服务器后来接上：从 `unavailable` 里去掉，工具出现在列表里，版本号加一。
/// 后台重连成功走的就是这条路（`connect_one` → `add`）。
#[tokio::test]
async fn adding_a_server_later_updates_tools_and_version() {
    let configs = BTreeMap::from([(
        "fake".to_string(),
        ServerConfig {
            command: "/nonexistent/mcp-server".into(),
            ..ServerConfig::default()
        },
    )]);
    let hub = McpHub::connect(&configs).await.unwrap();
    let before = hub.version();
    assert_eq!(hub.unavailable().len(), 1);
    assert!(hub.specs_for(&"any".into()).is_empty());

    let (client, server) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let running = Fake.serve(server).await.unwrap();
        let _ = running.waiting().await;
    });
    hub.add("fake", client, Vec::new(), Duration::from_millis(500))
        .await
        .unwrap();
    assert!(hub.unavailable().is_empty());
    assert_eq!(hub.version(), before + 1);
    assert_eq!(hub.specs_for(&"any".into()).len(), 4);
    let out = hub
        .call(&"any".into(), "mcp__fake__echo", r#"{"text": "回来了"}"#)
        .await
        .unwrap();
    assert!(out.ok && out.output == "回来了", "{}", out.output);

    // 同名的再接一次是重复。
    let (client, _server) = tokio::io::duplex(64);
    let err = hub
        .add("fake", client, Vec::new(), Duration::from_millis(50))
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Duplicate(_)), "{err}");
    hub.shutdown().await;
}

/// 子进程只看得到基本变量和配置里写明的变量，harness 的其他环境变量（密钥）一个都不带过去。
/// 用 `env` 代替 MCP 服务器，直接看子进程拿到的环境。
#[tokio::test]
async fn stdio_servers_do_not_inherit_harness_env() {
    let config = ServerConfig {
        command: "env".into(),
        env: [("PLAIN".to_string(), "1".to_string())].into(),
        env_inherit: vec!["SEC_USER_AGENT".into(), "IP_TEST_SURELY_UNSET_VAR".into()],
        env_from: [("TOKEN".to_string(), "WHICHAPI_API_KEY".to_string())].into(),
        ..ServerConfig::default()
    };
    let harness = |var: &str| {
        let value = match var {
            "PATH" => "/usr/bin:/bin",
            "HOME" => "/home/ip",
            "SEC_USER_AGENT" => "ip test@example.com",
            "WHICHAPI_API_KEY" => "wk-good",
            "OPENAI_API_KEY" => "sk-must-not-leak",
            "SLACK_BOT_TOKEN" => "xoxb-must-not-leak",
            _ => return None,
        };
        Some(value.to_string())
    };
    let out = stdio_command("t", &config, &harness).unwrap().output().await.unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let seen: BTreeMap<String, String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let expected: BTreeMap<String, String> = [
        ("PATH", "/usr/bin:/bin"),
        ("HOME", "/home/ip"),
        ("SEC_USER_AGENT", "ip test@example.com"),
        ("PLAIN", "1"),
        ("TOKEN", "wk-good"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    // 先只比变量名：失败时不把测试进程（CI）的环境变量值打进日志。
    assert_eq!(seen.keys().collect::<Vec<_>>(), expected.keys().collect::<Vec<_>>());
    assert_eq!(seen, expected);
    // 测试进程自己的环境变量也没有漏进去：它总有 expected 以外的变量（cargo、CI 设的），否则这个测试证明不了什么。
    assert!(std::env::vars_os().any(|(k, _)| !expected.contains_key(k.to_string_lossy().as_ref())));
}

fn remote(url: &str) -> ServerConfig {
    ServerConfig {
        url: Some(url.into()),
        ..ServerConfig::default()
    }
}

#[test]
fn command_and_url_are_exclusive() {
    let stdio = ServerConfig {
        command: "node".into(),
        ..ServerConfig::default()
    };
    assert!(stdio.validate().is_ok());
    assert!(remote("https://example.com/mcp").validate().is_ok());
    assert!(ServerConfig::default().validate().is_err(), "两个都没写");

    let both = ServerConfig {
        command: "node".into(),
        ..remote("https://example.com/mcp")
    };
    assert!(both.validate().is_err(), "两个都写了");

    let token_on_stdio = ServerConfig {
        bearer_token_env: Some("TOKEN".into()),
        ..stdio.clone()
    };
    assert!(token_on_stdio.validate().is_err());

    let env_on_remote = ServerConfig {
        env_from: [("TOKEN".to_string(), "TOKEN".to_string())].into(),
        ..remote("https://example.com/mcp")
    };
    assert!(env_on_remote.validate().is_err());

    let inherit_on_remote = ServerConfig {
        env_inherit: vec!["HTTPS_PROXY".into()],
        ..remote("https://example.com/mcp")
    };
    assert!(inherit_on_remote.validate().is_err());
}

/// 令牌和数据不能明文出网：远程只收 https，http 只给本机。
#[test]
fn remote_url_must_be_https_unless_local() {
    for ok in [
        "https://whichapi-macro.onrender.com/mcp",
        "http://127.0.0.1:3220/mcp",
        "http://localhost:3220/mcp/macro",
        "http://[::1]:3220/mcp",
    ] {
        assert!(remote(ok).validate().is_ok(), "{ok}");
    }
    for bad in [
        "http://example.com/mcp",
        "http://127.0.0.1.example.com/mcp",
        "ws://example.com/mcp",
        "example.com/mcp",
        "https://",
        "",
    ] {
        assert!(remote(bad).validate().is_err(), "{bad}");
    }
}

/// docker compose 里用服务名连另一个容器（tcms 服务）时，要在配置里明写 allow_http 才收 http。
#[test]
fn allow_http_is_an_explicit_opt_in_for_private_network_hosts() {
    let compose = "http://tcms:7341/mcp";
    assert!(remote(compose).validate().is_err(), "没写 allow_http 时照旧拒绝");
    let allowed = ServerConfig {
        allow_http: true,
        ..remote(compose)
    };
    assert!(allowed.validate().is_ok());
    // allow_http 不放宽别的：照样要有主机名，照样只认 http 和 https。
    for bad in ["http:///mcp", "ws://tcms:7341/mcp", "tcms:7341/mcp"] {
        let config = ServerConfig {
            allow_http: true,
            ..remote(bad)
        };
        assert!(config.validate().is_err(), "{bad}");
    }
    let on_stdio = ServerConfig {
        command: "node".into(),
        allow_http: true,
        ..ServerConfig::default()
    };
    assert!(on_stdio.validate().is_err(), "allow_http 只用于 url");
}

#[tokio::test]
async fn remote_config_errors_are_reported_before_connecting() {
    let mut configs = BTreeMap::new();
    configs.insert("bad".to_string(), remote("http://example.com/mcp"));
    let err = McpHub::connect(&configs).await.unwrap_err();
    assert!(matches!(err, McpError::BadConfig { .. }), "{err}");

    configs.insert(
        "bad".to_string(),
        ServerConfig {
            bearer_token_env: Some("MACRO_API_KEY".into()),
            ..remote("https://example.com/mcp")
        },
    );
    let hub = McpHub::connect_with_env(&configs, &|_| None).await.unwrap();
    assert!(
        matches!(hub.unavailable()[0].error, McpError::MissingEnv { .. }),
        "令牌没设只是这个服务器连不上：{}",
        hub.unavailable()[0].error
    );

    // 有一个配置写错时一个都不连，哪怕别的服务器本来能连上。
    configs.insert("bad".to_string(), remote("http://example.com/mcp"));
    configs.insert("good".to_string(), remote("https://example.com/mcp"));
    let err = McpHub::connect(&configs).await.unwrap_err();
    assert!(matches!(err, McpError::BadConfig { .. }), "{err}");
}

#[tokio::test]
async fn harness_calls_tools_by_server_and_gets_json() {
    let hub = hub(vec![RoleId::from("macro")]).await;
    let args = |text: &str| {
        let mut m = serde_json::Map::new();
        m.insert("text".into(), text.into());
        m
    };
    let wait = Duration::from_secs(5);
    // 文字内容是 JSON 时解析出来；不看岗位（服务器只给 macro 用，harness 照样能调）。
    let v = hub
        .call_json("fake", "echo", args(r#"{"prefix":"规则"}"#), wait)
        .await
        .unwrap();
    assert_eq!(v["prefix"], "规则");
    // 不是 JSON、工具报错、没有这个服务器、超时：都是 Err，说清楚原因。
    let e = hub.call_json("fake", "echo", args("纯文字"), wait).await.unwrap_err();
    assert!(e.contains("不是 JSON"), "{e}");
    let e = hub.call_json("fake", "fail", args(""), wait).await.unwrap_err();
    assert_eq!(e, "坏了");
    let e = hub.call_json("nope", "echo", args(""), wait).await.unwrap_err();
    assert!(e.contains("没有 MCP 服务器 nope"), "{e}");
    let e = hub
        .call_json("fake", "slow", args(""), Duration::from_millis(50))
        .await
        .unwrap_err();
    assert!(e.contains("没有返回"), "{e}");
}
