//! `/mcp`：外部引擎拿本轮令牌调组织工具（`docs/ip/m2-spec.md` D-17 到 D-19）。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use ip_core::{ApprovalSubject, Author, ThreadId};
use rmcp::ServiceExt;
use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use serde_json::json;

use crate::ws_tests::start;

async fn client(addr: std::net::SocketAddr, token: &str) -> RunningService<RoleClient, ()> {
    let config = StreamableHttpClientTransportConfig::with_uri(format!("http://{addr}/mcp")).auth_header(token);
    ().serve(StreamableHttpClientTransport::from_config(config))
        .await
        .expect("用有效令牌应能连上 /mcp")
}

fn text(content: &[ContentBlock]) -> String {
    content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn call(
    peer: &RunningService<RoleClient, ()>,
    name: &'static str,
    args: serde_json::Value,
) -> Result<(bool, String), rmcp::ServiceError> {
    let serde_json::Value::Object(args) = args else {
        unreachable!()
    };
    let result = peer
        .call_tool(CallToolRequestParams::new(name).with_arguments(args))
        .await?;
    Ok((!result.is_error.unwrap_or(false), text(&result.content)))
}

/// 没有令牌、令牌不对，都在 HTTP 层被挡掉；控制台的访问令牌在这里也不算数。
#[tokio::test]
async fn requests_without_a_turn_token_are_rejected() {
    let s = start(Some("console-secret")).await;
    let http = reqwest::Client::new();
    let url = format!("http://{}/mcp", s.addr);
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"});
    for auth in [None, Some("Bearer ipt_not-a-real-token"), Some("Bearer console-secret")] {
        let mut req = http
            .post(&url)
            .header("accept", "application/json, text/event-stream")
            .json(&body);
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        assert_eq!(req.send().await.unwrap().status(), 401, "{auth:?}");
    }
    assert!(
        s.app
            .runtime()
            .grant_turn(&"nobody".into(), &ThreadId::new(), Duration::from_secs(60))
            .is_err()
    );
}

/// 引擎带着令牌调工具：以令牌对应的员工、在令牌对应的对话里执行，和员工自己调一样。
#[tokio::test]
async fn turn_token_acts_as_its_agent_in_its_thread() {
    let s = start(None).await;
    let rt = s.app.runtime();
    let thread = ThreadId::new();
    let token = rt
        .grant_turn(&"macro-1".into(), &thread, Duration::from_secs(60))
        .unwrap();
    let peer = client(s.addr, &token).await;

    // 只开放组织工具（这个员工没有数据源和外部 MCP），没有 shell 和文件工具。
    let mut names: Vec<String> = peer
        .list_all_tools()
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.name.to_string())
        .collect();
    names.sort();
    assert_eq!(names, ["ask_human", "create_task", "list_tasks", "update_task"]);

    let (ok, out) = call(&peer, "create_task", json!({"title": "整理 10 月 PPI"}))
        .await
        .unwrap();
    assert!(ok, "{out}");
    let tasks = rt.store().tasks(false, 10).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "整理 10 月 PPI");
    assert_eq!(
        tasks[0].created_by,
        Author::Agent {
            agent_id: "macro-1".into()
        }
    );
    assert_eq!(tasks[0].origin_thread_id.as_ref(), Some(&thread));

    // ask_human 的卡片记在令牌对应的员工和对话名下：人选定后叫回的是他，渠道也能据此找到会话（D-06）。
    let (ok, out) = call(
        &peer,
        "ask_human",
        json!({"question": "先写 CPI 还是 PPI？", "options": [{"label": "CPI", "consequence": "先写 CPI"}, {"label": "PPI", "consequence": "先写 PPI"}], "recommended": 1}),
    )
    .await
    .unwrap();
    assert!(ok && out.contains("已发出"), "{out}");
    let cards = rt.store().approvals(true, 10).await.unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0].subject,
        ApprovalSubject::AgentQuestion {
            agent_id: "macro-1".into(),
            thread_id: thread.clone()
        }
    );

    // 名字不认识的工具（比如 shell）得到工具级错误，不会落到别的工具上。
    let (ok, out) = call(&peer, "shell", json!({"command": "ls"})).await.unwrap();
    assert!(!ok && out.contains("没有工具 shell"), "{out}");

    // 这一轮结束、宽限期过后，同一个连接上的调用被拒绝，新连接在 HTTP 层就被挡掉。
    rt.end_turn(&token, Duration::ZERO);
    assert!(!rt.turn_token_valid(&token));
    assert!(call(&peer, "list_tasks", json!({})).await.is_err());
    let _ = peer.cancel().await;
}

/// 令牌到期自动作废；结束一轮不会延长原有期限。
#[tokio::test]
async fn turn_tokens_expire() {
    let s = start(None).await;
    let rt = s.app.runtime();
    let short = rt
        .grant_turn(&"macro-1".into(), &ThreadId::new(), Duration::from_millis(50))
        .unwrap();
    let long = rt
        .grant_turn(&"macro-1".into(), &ThreadId::new(), Duration::from_secs(60))
        .unwrap();
    assert_ne!(short, long);
    assert!(rt.turn_token_valid(&short) && rt.turn_token_valid(&long));
    rt.end_turn(&short, ip_runtime::TURN_TOKEN_GRACE);
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert!(!rt.turn_token_valid(&short), "宽限期不应延长原有期限");
    assert!(rt.turn_token_valid(&long));
}

/// `mcp_servers` 里写 `url` 的远程服务器：McpHub 用 Streamable HTTP 连上，带上环境变量里的令牌。
/// 这里拿本服务自己的 `/mcp` 当远程服务器。
#[tokio::test]
async fn hub_connects_to_a_remote_server_over_http() {
    let s = start(None).await;
    let rt = s.app.runtime();
    let thread = ThreadId::new();
    let token = rt
        .grant_turn(&"macro-1".into(), &thread, Duration::from_secs(60))
        .unwrap();
    let server = |token_env: Option<&str>| {
        let config = ip_mcp::ServerConfig {
            url: Some(format!("http://{}/mcp", s.addr)),
            bearer_token_env: token_env.map(str::to_string),
            roles: vec!["macro-researcher".into()],
            timeout_secs: Some(10),
            ..ip_mcp::ServerConfig::default()
        };
        std::collections::BTreeMap::from([("remote".to_string(), config)])
    };
    let env = |var: &str| match var {
        "GOOD" => Some(token.clone()),
        "BAD" => Some("ipt_not-a-real-token".to_string()),
        _ => None,
    };

    let hub = ip_mcp::McpHub::connect_with_env(&server(Some("GOOD")), &env)
        .await
        .unwrap();
    let names: Vec<String> = hub
        .specs_for(&"macro-researcher".into())
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(
        names,
        [
            "mcp__remote__ask_human",
            "mcp__remote__create_task",
            "mcp__remote__list_tasks",
            "mcp__remote__update_task"
        ]
    );
    assert!(hub.specs_for(&"ops-tester".into()).is_empty(), "只给限定的岗位");

    let out = hub
        .call(
            &"macro-researcher".into(),
            "mcp__remote__create_task",
            r#"{"title": "整理 10 月 CPI"}"#,
        )
        .await
        .unwrap();
    assert!(out.ok, "{}", out.output);
    let tasks = rt.store().tasks(false, 10).await.unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].title, "整理 10 月 CPI");
    hub.shutdown().await;

    // 令牌不对、没带令牌、令牌没设：这个服务器连不上，记进 unavailable，不报错。
    for (token_env, connect) in [(Some("BAD"), true), (None, true), (Some("UNSET"), false)] {
        let hub = ip_mcp::McpHub::connect_with_env(&server(token_env), &env)
            .await
            .unwrap();
        assert!(hub.specs_for(&"macro-researcher".into()).is_empty());
        let down = hub.unavailable();
        let err = &down[0].error;
        if connect {
            assert!(matches!(err, ip_mcp::McpError::Connect { .. }), "{err}");
        } else {
            assert!(matches!(err, ip_mcp::McpError::MissingEnv { .. }), "{err}");
        }
    }

    // 另一个服务器连不上，不影响这一个。
    let mut configs = server(Some("GOOD"));
    configs.insert(
        "broken".to_string(),
        ip_mcp::ServerConfig {
            command: "/nonexistent/mcp-server".into(),
            ..ip_mcp::ServerConfig::default()
        },
    );
    let hub = ip_mcp::McpHub::connect_with_env(&configs, &env).await.unwrap();
    assert_eq!(hub.specs_for(&"macro-researcher".into()).len(), 4);
    assert_eq!(hub.unavailable().len(), 1);
    assert_eq!(hub.unavailable()[0].name, "broken");
    hub.shutdown().await;
}

#[derive(Clone, Copy)]
enum StartupFailure {
    HandshakeTimeout,
    ToolListError,
    ToolListTimeout,
}

#[derive(Clone)]
struct StartupFailureState {
    failure: StartupFailure,
    methods: Arc<Mutex<Vec<String>>>,
    release: Arc<tokio::sync::Notify>,
}

/// 故意在指定阶段失败。记录请求顺序，免得“工具列表失败”实际测到的是握手失败。
async fn failing_mcp(State(state): State<StartupFailureState>, Json(request): Json<serde_json::Value>) -> Response {
    let method = request["method"].as_str().unwrap_or_default();
    if matches!(method, "initialize" | "notifications/initialized" | "tools/list") {
        state.methods.lock().unwrap().push(method.to_string());
    }
    match method {
        "initialize" if !matches!(state.failure, StartupFailure::HandshakeTimeout) => Json(json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {
                "protocolVersion": request["params"]["protocolVersion"],
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "startup-failure-test", "version": "1"}
            }
        }))
        .into_response(),
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "tools/list" if matches!(state.failure, StartupFailure::ToolListError) => Json(json!({
            "jsonrpc": "2.0",
            "id": request["id"],
            "error": {"code": -32603, "message": "测试服务器无法列出工具"}
        }))
        .into_response(),
        "initialize" | "tools/list" => {
            // 保持连接，直到客户端自己的超时生效；测试收尾时再放行，避免留下挂起的处理器。
            state.release.notified().await;
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        method if method.starts_with("notifications/") => StatusCode::ACCEPTED.into_response(),
        _ => StatusCode::BAD_REQUEST.into_response(),
    }
}

async fn assert_mcp_startup_degrades(failure: StartupFailure) {
    let healthy = start(None).await;
    let rt = healthy.app.runtime();
    let token = rt
        .grant_turn(&"macro-1".into(), &ThreadId::new(), Duration::from_secs(60))
        .unwrap();
    let state = StartupFailureState {
        failure,
        methods: Arc::default(),
        release: Arc::default(),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new().route("/mcp", post(failing_mcp)).with_state(state.clone());
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });
    let role = ip_core::RoleId::from("macro-researcher");
    let good = ip_mcp::ServerConfig {
        url: Some(format!("http://{}/mcp", healthy.addr)),
        bearer_token_env: Some("GOOD".into()),
        roles: vec![role.clone()],
        timeout_secs: Some(5),
        ..ip_mcp::ServerConfig::default()
    };
    let bad = ip_mcp::ServerConfig {
        url: Some(format!("http://{addr}/mcp")),
        roles: vec![role.clone()],
        timeout_secs: Some(1),
        ..ip_mcp::ServerConfig::default()
    };
    // BTreeMap 按名字连接：既要保住已经接入的服务器，也要继续接入后面的。
    let configs = std::collections::BTreeMap::from([
        ("a_healthy".into(), good.clone()),
        ("b_broken".into(), bad),
        ("c_healthy".into(), good),
    ]);
    let env = |var: &str| (var == "GOOD").then(|| token.clone());
    let hub = tokio::time::timeout(
        Duration::from_secs(10),
        ip_mcp::McpHub::connect_with_env(&configs, &env),
    )
    .await
    .expect("故障服务器没有在配置的超时后放行")
    .expect("单个 MCP 服务器失败不应阻止其他服务器连接");
    let down = hub.unavailable();
    assert_eq!(down.len(), 1);
    assert_eq!(down[0].name, "b_broken");
    assert_eq!(down[0].roles.as_slice(), std::slice::from_ref(&role));
    let ip_mcp::McpError::Connect { name, error } = &down[0].error else {
        panic!("应记录连接阶段的故障：{}", down[0].error);
    };
    assert_eq!(name, "b_broken");
    let expected_error = match failure {
        StartupFailure::HandshakeTimeout => "握手超过 1 秒没有完成",
        StartupFailure::ToolListError => "测试服务器无法列出工具",
        StartupFailure::ToolListTimeout => "获取工具列表超时",
    };
    assert!(error.contains(expected_error), "{error}");
    let expected_methods = match failure {
        StartupFailure::HandshakeTimeout => vec!["initialize"],
        _ => vec!["initialize", "notifications/initialized", "tools/list"],
    };
    assert_eq!(*state.methods.lock().unwrap(), expected_methods);

    let specs = hub.specs_for(&role);
    assert_eq!(specs.len(), 8);
    assert!(specs.iter().all(|s| !s.name.starts_with("mcp__b_broken__")));
    for name in ["a_healthy", "c_healthy"] {
        let out = hub
            .call(
                &role,
                &format!("mcp__{name}__create_task"),
                &json!({"title": name}).to_string(),
            )
            .await
            .unwrap();
        assert!(out.ok, "{name} 的工具应仍可用：{}", out.output);
    }
    let tasks = rt.store().tasks(false, 10).await.unwrap();
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().any(|t| t.title == "a_healthy"));
    assert!(tasks.iter().any(|t| t.title == "c_healthy"));
    let out = hub.call(&role, "mcp__b_broken__get", "{}").await.unwrap();
    assert!(!out.ok && out.output.contains("b_broken") && out.output.contains("没连上"));
    let out = hub
        .call(&"ops-tester".into(), "mcp__b_broken__get", "{}")
        .await
        .unwrap();
    assert!(!out.ok && out.output.contains("不能用") && !out.output.contains("没连上"));

    state.release.notify_waiters();
    hub.shutdown().await;
    let _ = stop.send(());
    tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("假服务器未正常关闭")
        .unwrap();
    healthy.app.shutdown().await;
}

#[tokio::test]
async fn mcp_handshake_timeout_is_not_fatal() {
    assert_mcp_startup_degrades(StartupFailure::HandshakeTimeout).await;
}

#[tokio::test]
async fn mcp_tool_list_error_after_handshake_is_not_fatal() {
    assert_mcp_startup_degrades(StartupFailure::ToolListError).await;
}

#[tokio::test]
async fn mcp_tool_list_timeout_after_handshake_is_not_fatal() {
    assert_mcp_startup_degrades(StartupFailure::ToolListTimeout).await;
}

/// 先挂后好的远程 MCP 服务器：`up` 为 false 时一律 503，为 true 时有一个 `ping` 工具。
async fn flaky_mcp(
    State(up): State<Arc<std::sync::atomic::AtomicBool>>,
    Json(request): Json<serde_json::Value>,
) -> Response {
    if !up.load(std::sync::atomic::Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": request["params"]["protocolVersion"],
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "flaky-test", "version": "1"}
        }),
        "tools/list" => json!({
            "tools": [{"name": "ping", "description": "回 pong", "inputSchema": {"type": "object"}}]
        }),
        "tools/call" => json!({"content": [{"type": "text", "text": "pong"}], "isError": false}),
        method if method.starts_with("notifications/") => return StatusCode::ACCEPTED.into_response(),
        _ => return StatusCode::BAD_REQUEST.into_response(),
    };
    Json(json!({"jsonrpc": "2.0", "id": request["id"], "result": result})).into_response()
}

/// backlog E-4：启动时没连上的服务器在后台重连上之后，员工下一轮发给模型的工具列表里就有它的工具，也能调用。
#[tokio::test]
async fn agent_gets_tools_of_a_server_that_reconnects() {
    use ip_runtime::RuntimeEvent;

    let up = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = Router::new().route("/mcp", post(flaky_mcp)).with_state(up.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    let configs = std::collections::BTreeMap::from([(
        "flaky".to_string(),
        ip_mcp::ServerConfig {
            url: Some(format!("http://{addr}/mcp")),
            timeout_secs: Some(2),
            ..ip_mcp::ServerConfig::default()
        },
    )]);
    let hub = Arc::new(ip_mcp::McpHub::connect_with_env(&configs, &|_| None).await.unwrap());
    assert_eq!(hub.unavailable().len(), 1);
    hub.spawn_reconnect(
        &configs,
        Arc::new(|_: &str| None),
        ip_mcp::Reconnect {
            attempts: 5,
            first_delay: Duration::from_millis(20),
        },
    );

    let dir = tempfile::tempdir().unwrap();
    let store = ip_store::Store::open(dir.path().join("ip.db")).unwrap();
    let model = Arc::new(ip_models::ScriptedModel::new([
        ip_models::ScriptedTurn::Text("第一轮".into()),
        ip_models::ScriptedTurn::Text("第二轮".into()),
    ]));
    let mut models = ip_models::ModelRegistry::new();
    models.insert("fake", model.clone());
    let rt = ip_runtime::Runtime::new(store, models, ip_runtime::RuntimeConfig::default());
    let role = ip_core::Role {
        id: "macro".into(),
        name: None,
        description: "撰写宏观研报。".into(),
        model: ip_core::ModelSlot {
            default: "fake/m".into(),
            escalate: None,
        },
        budget_per_day_usd: None,
    };
    rt.spawn_agent(ip_runtime::AgentSetup {
        agent: ip_core::Agent {
            id: "macro-1".into(),
            role: "macro".into(),
            name: "小宏".into(),
        },
        role,
        toolbox: None,
        mcp: Some(hub.clone()),
        data: None,
        engine: None,
    })
    .unwrap();
    let turn = |text: &'static str| {
        let rt = rt.clone();
        async move {
            let mut events = rt.subscribe();
            rt.send(&"macro-1".into(), ThreadId::new(), Author::human("张三"), text)
                .await
                .unwrap();
            loop {
                let e = tokio::time::timeout(Duration::from_secs(10), events.recv())
                    .await
                    .expect("等这一轮结束超时")
                    .unwrap();
                if matches!(e, RuntimeEvent::TurnFinished { .. } | RuntimeEvent::TurnFailed { .. }) {
                    return;
                }
            }
        }
    };
    let mcp_tools = |i: usize| -> Vec<String> {
        model.requests()[i]
            .tools
            .iter()
            .map(|t| t.name.clone())
            .filter(|n| n.starts_with("mcp__"))
            .collect()
    };

    turn("第一次").await;
    assert!(mcp_tools(0).is_empty(), "服务器没连上时不给它的工具");

    // 至少失败过一次再恢复，确认是后台重连接上的。
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while hub.unavailable()[0].attempts == 0 {
        assert!(tokio::time::Instant::now() < deadline, "后台没有重连");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    up.store(true, std::sync::atomic::Ordering::SeqCst);
    while !hub.unavailable().is_empty() {
        assert!(tokio::time::Instant::now() < deadline, "服务器恢复后没有重连上");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    turn("第二次").await;
    assert_eq!(mcp_tools(1), ["mcp__flaky__ping"]);
    let out = hub.call(&"macro".into(), "mcp__flaky__ping", "{}").await.unwrap();
    assert!(out.ok && out.output == "pong", "{}", out.output);
    hub.shutdown().await;
    rt.shutdown().await;
}
