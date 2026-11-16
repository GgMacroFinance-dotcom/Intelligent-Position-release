use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use ip_core::{Agent, ModelSlot, OrgSpec, Role};
use ip_models::{ModelRegistry, Price, ScriptedModel, ScriptedTurn, Usage};
use ip_runtime::{AgentSetup, Runtime, RuntimeConfig};
use ip_store::Store;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

use crate::{App, Config, router};

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(crate) struct Server {
    _dir: tempfile::TempDir,
    pub(crate) addr: std::net::SocketAddr,
    pub(crate) app: App,
}

pub(crate) async fn start(token: Option<&str>) -> Server {
    start_with(token, None).await
}

async fn start_with(token: Option<&str>, rules: Option<&str>) -> Server {
    start_scripted(token, rules, vec![ScriptedTurn::Text("CPI 同比上涨 0.3%".into())]).await
}

async fn start_scripted(token: Option<&str>, rules: Option<&str>, turns: Vec<ScriptedTurn>) -> Server {
    start_with_schedules(token, rules, None, turns).await
}

async fn start_with_schedules(
    token: Option<&str>,
    rules: Option<&str>,
    schedules: Option<&str>,
    turns: Vec<ScriptedTurn>,
) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("ip.db")).unwrap();
    let mut models = ModelRegistry::new();
    let usage = Usage {
        prompt_tokens: 1000,
        completion_tokens: 0,
        cached_tokens: 0,
        cache_write_tokens: 0,
    };
    models.insert("fake", Arc::new(ScriptedModel::new(turns).with_usage(usage)));
    // 只给语义分派用的模型标价：每次调用 1000 输入 token，$0.001。
    models.set_price(
        "fake/judge",
        Price {
            input: 1.0,
            cached_input: None,
            output: 1.0,
            cache_write: None,
        },
    );
    let role = Role {
        id: "macro".into(),
        name: Some("宏观研究员".into()),
        description: "撰写宏观研报。".into(),
        model: ModelSlot {
            default: "fake/m".into(),
            escalate: None,
        },
        budget_per_day_usd: None,
    };
    let runtime = Runtime::new(store, models, RuntimeConfig::default());
    let agent = Agent {
        id: "macro-1".into(),
        role: "macro".into(),
        name: "小宏".into(),
    };
    runtime
        .spawn_agent(AgentSetup {
            agent,
            role: role.clone(),
            toolbox: None,
            mcp: None,
            data: None,
            engine: None,
        })
        .unwrap();
    let org = OrgSpec { roles: vec![role] };
    // 小宏的工作目录里放一份报告，测 list_reports、read_report。
    let ws = dir.path().join("ws-macro-1");
    std::fs::create_dir_all(ws.join("reports")).unwrap();
    std::fs::write(ws.join("reports").join("2026-11-16-cpi.md"), "# CPI\n\n同比 2.9%").unwrap();
    let mut app = App::new(runtime, org.clone(), token.map(String::from))
        .with_workspaces(std::collections::HashMap::from([("macro-1".into(), ws)]));
    if let Some(spec) = rules {
        let agents = app.runtime().agents();
        app = app.with_router(ip_router::Router::new(serde_saphyr::from_str(spec).unwrap(), &org, &agents).unwrap());
    }
    if let Some(yaml) = schedules {
        let specs = serde_saphyr::from_str(yaml).unwrap();
        let agents = vec![("macro-1".to_string(), "macro".to_string())];
        app = app.with_schedules(crate::schedule::Schedules::new(specs, &org, &agents).unwrap());
    }

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let r = router(Some(app.clone()));
    tokio::spawn(async move { axum::serve(listener, r).await.unwrap() });
    Server { _dir: dir, addr, app }
}

async fn connect(addr: std::net::SocketAddr, query: &str) -> Result<Ws, tokio_tungstenite::tungstenite::Error> {
    tokio_tungstenite::connect_async(format!("ws://{addr}/ws{query}"))
        .await
        .map(|(ws, _)| ws)
}

/// 带自定义请求头握手，模拟浏览器发来的 `Origin`、被改写的 `Host`。返回 HTTP 状态码或连接。
async fn connect_headers(addr: std::net::SocketAddr, query: &str, headers: &[(&str, &str)]) -> Result<Ws, u16> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut req = format!("ws://{addr}/ws{query}").into_client_request().unwrap();
    for (k, v) in headers {
        req.headers_mut().insert(
            axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            v.parse().unwrap(),
        );
    }
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(ws),
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Err(resp.status().as_u16()),
        Err(e) => panic!("握手失败：{e}"),
    }
}

async fn next_json(ws: &mut Ws) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), ws.next())
            .await
            .expect("等待超时")
            .unwrap()
            .unwrap();
        if let Message::Text(t) = msg {
            return serde_json::from_str(&t).unwrap();
        }
    }
}

/// 发请求并等对应 id 的回复，途中的事件收集起来返回。
async fn request(ws: &mut Ws, id: u64, body: Value) -> (Value, Vec<Value>) {
    let mut body = body;
    body["id"] = id.into();
    ws.send(Message::Text(body.to_string().into())).await.unwrap();
    let mut events = Vec::new();
    loop {
        let v = next_json(ws).await;
        if v["type"] == "reply" && v["id"] == id {
            return (v, events);
        }
        events.push(v);
    }
}

#[tokio::test]
async fn chat_roundtrip_over_websocket() {
    let s = start(None).await;
    let mut ws = connect(s.addr, "").await.unwrap();

    let (roles, _) = request(&mut ws, 1, json!({"type": "list_roles"})).await;
    assert_eq!(roles["data"][0]["id"], "macro");
    let (agents, _) = request(&mut ws, 2, json!({"type": "list_agents"})).await;
    assert_eq!(agents["data"][0]["name"], "小宏");

    let (sent, mut events) = request(
        &mut ws,
        3,
        json!({"type": "send", "agent_id": "macro-1", "text": "点评一下本月 CPI", "author": "张三"}),
    )
    .await;
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(sent["data"]["agent_id"], "macro-1");
    assert!(sent["data"]["decision"].is_null());
    let thread = sent["data"]["message"]["thread_id"].as_str().unwrap().to_string();
    while !events.iter().any(|e| e["event"]["type"] == "turn_finished") {
        events.push(next_json(&mut ws).await);
    }
    let streamed: String = events
        .iter()
        .filter(|e| e["event"]["type"] == "delta")
        .map(|e| e["event"]["text"].as_str().unwrap())
        .collect();
    assert_eq!(streamed, "CPI 同比上涨 0.3%");

    let (history, _) = request(&mut ws, 4, json!({"type": "history", "thread_id": thread})).await;
    let history = history["data"].as_array().unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["author"], json!({"kind": "human", "name": "张三"}));

    let (threads, _) = request(&mut ws, 5, json!({"type": "list_threads"})).await;
    assert_eq!(threads["data"][0]["thread_id"], thread.as_str());
    assert_eq!(threads["data"][0]["message_count"], 2);

    let (hits, _) = request(&mut ws, 6, json!({"type": "search", "query": "同比上涨"})).await;
    assert_eq!(hits["data"].as_array().unwrap().len(), 1, "{hits}");
}

/// 收事件直到一轮结束，返回途中所有事件帧。
async fn until_turn_finished(ws: &mut Ws, mut frames: Vec<Value>) -> Vec<Value> {
    while !frames.iter().any(|e| e["event"]["type"] == "turn_finished") {
        frames.push(next_json(ws).await);
    }
    frames
}

/// 断线重连补发（`docs/ip/m2-spec.md` D-29）。
#[tokio::test]
async fn resumes_events_after_reconnect() {
    let s = start(None).await;
    let mut ws = connect(s.addr, "").await.unwrap();
    let hello = next_json(&mut ws).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["protocol"], crate::events::PROTOCOL_VERSION);
    assert_eq!(hello["resumed"], false);
    assert_eq!(hello["seq"], 0);
    let epoch = hello["epoch"].as_str().unwrap().to_string();

    let (_, events) = request(
        &mut ws,
        1,
        json!({"type": "send", "agent_id": "macro-1", "text": "第一轮"}),
    )
    .await;
    let events = until_turn_finished(&mut ws, events).await;
    let seqs: Vec<u64> = events.iter().map(|e| e["seq"].as_u64().unwrap()).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>(), "序号从 1 起、连续");
    let seen = *seqs.last().unwrap();
    drop(ws);

    // 断开期间另一个连接发了一轮
    let mut other = connect(s.addr, "").await.unwrap();
    assert_eq!(next_json(&mut other).await["seq"], seen);
    let (_, events) = request(
        &mut other,
        1,
        json!({"type": "send", "agent_id": "macro-1", "text": "第二轮"}),
    )
    .await;
    let missed: Vec<Value> = until_turn_finished(&mut other, events).await;
    let latest = missed.last().unwrap()["seq"].as_u64().unwrap();

    // 带上次的序号重连：先 hello，再按顺序补发断开期间的事件
    let mut back = connect(s.addr, &format!("?since={seen}&epoch={epoch}")).await.unwrap();
    let hello = next_json(&mut back).await;
    assert_eq!(
        (hello["resumed"].clone(), hello["seq"].clone()),
        (json!(true), json!(seen))
    );
    let mut replayed = Vec::new();
    for _ in seen..latest {
        replayed.push(next_json(&mut back).await);
    }
    assert_eq!(replayed, missed);
    // 补完之后接着收请求的回复，没有多余的帧
    let (r, extra) = request(&mut back, 2, json!({"type": "list_roles"})).await;
    assert_eq!(r["ok"], true);
    assert!(extra.is_empty(), "{extra:?}");

    // epoch 不对（服务重启过）或序号比服务端还新：不补发，客户端重新拉
    for query in [
        format!("?since={seen}&epoch=old"),
        format!("?since={}&epoch={epoch}", latest + 5),
    ] {
        let mut ws = connect(s.addr, &query).await.unwrap();
        let hello = next_json(&mut ws).await;
        assert_eq!(hello["resumed"], false, "{query}");
        assert_eq!(hello["seq"], latest);
        let (_, extra) = request(&mut ws, 1, json!({"type": "list_roles"})).await;
        assert!(extra.is_empty(), "{query}: {extra:?}");
    }
}

/// 连接跟不上广播时，从留着的事件里补上；留着的也补不上才发 `lagged`。
#[tokio::test]
async fn slow_connections_catch_up_from_kept_events() {
    let s = start(None).await;
    // 广播通道只积压 1 帧，事件一多这个连接必然跟不上。
    let mut app = s.app.clone();
    app.events = Arc::new(crate::events::EventLog::new(4096, usize::MAX, 1));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let r = router(Some(app.clone()));
    tokio::spawn(async move { axum::serve(listener, r).await.unwrap() });
    let delta = |n: u32| ip_runtime::RuntimeEvent::Delta {
        agent_id: "macro-1".into(),
        thread_id: "t".into(),
        text: n.to_string(),
    };

    let mut ws = connect(addr, "").await.unwrap();
    assert_eq!(next_json(&mut ws).await["type"], "hello");
    // 测试跑在单线程运行时上，这个循环里连接的任务插不进来，广播通道一定会溢出。
    for n in 1..=2000 {
        app.events.push(delta(n));
    }
    for seq in 1..=2000u64 {
        let frame = next_json(&mut ws).await;
        assert_eq!(
            (frame["type"].clone(), frame["seq"].clone()),
            (json!("event"), json!(seq))
        );
    }

    // 一下子来 5000 个，超过留着的 4096 个：补不上，发一次 lagged，然后接着推新的。
    for n in 1..=5000 {
        app.events.push(delta(n));
    }
    let lagged = next_json(&mut ws).await;
    assert_eq!(lagged, json!({"type": "lagged", "skipped": 7000 - 4096 - 2000}));
    app.events.push(delta(0));
    assert_eq!(next_json(&mut ws).await["seq"], 7001);
}

#[tokio::test]
async fn bad_requests_get_error_replies() {
    let s = start(None).await;
    let mut ws = connect(s.addr, "").await.unwrap();
    let (r, _) = request(&mut ws, 1, json!({"type": "send", "agent_id": "nobody", "text": "hi"})).await;
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().contains("nobody"));
    let (r, _) = request(&mut ws, 2, json!({"type": "no_such_request"})).await;
    assert_eq!(r["ok"], false);
    let (r, _) = request(&mut ws, 3, json!({"type": "send", "agent_id": "macro-1", "text": "  "})).await;
    assert_eq!(r["ok"], false);
}

#[tokio::test]
async fn token_is_required_when_configured() {
    let s = start(Some("s3cret")).await;
    assert!(connect(s.addr, "").await.is_err());
    assert!(connect(s.addr, "?token=wrong").await.is_err());
    let mut ws = connect(s.addr, "?token=s3cret").await.unwrap();
    let (r, _) = request(&mut ws, 1, json!({"type": "list_agents"})).await;
    assert_eq!(r["ok"], true);
}

/// 设备配对（backlog F-7）：共用令牌生成配对码，设备换到自己的令牌，署名用设备名，吊销后立即断开。
#[tokio::test]
async fn devices_pair_sign_with_their_name_and_can_be_revoked() {
    let s = start(Some("s3cret")).await;
    let mut admin = connect(s.addr, "?token=s3cret").await.unwrap();
    let (me, _) = request(&mut admin, 1, json!({"type": "who_am_i"})).await;
    assert_eq!(me["data"], json!({"device": null, "scope": "admin"}));
    let (p, _) = request(
        &mut admin,
        2,
        json!({"type": "create_pairing", "name": "张三的手机", "link_base": "https://ip.example.com"}),
    )
    .await;
    let code = p["data"]["code"].as_str().unwrap().to_string();
    assert_eq!(p["data"]["link"], format!("https://ip.example.com/#/pair/{code}"));

    let http = reqwest::Client::new();
    let pair = |body: Value, origin: Option<&'static str>| {
        let mut req = http.post(format!("http://{}/pair", s.addr)).json(&body);
        if let Some(o) = origin {
            req = req.header("Origin", o);
        }
        req.send()
    };
    // 别的网页借浏览器来换令牌：挡在来源检查
    assert_eq!(
        pair(json!({"code": code}), Some("https://evil.example"))
            .await
            .unwrap()
            .status(),
        403
    );
    let wrong = if code == "000000" { "000001" } else { "000000" };
    assert_eq!(pair(json!({"code": wrong}), None).await.unwrap().status(), 400);
    let claimed: Value = pair(json!({"code": code}), None).await.unwrap().json().await.unwrap();
    let token = claimed["token"].as_str().unwrap().to_string();
    let device_id = claimed["device"]["id"].as_str().unwrap().to_string();
    assert_eq!(
        pair(json!({"code": code}), None).await.unwrap().status(),
        400,
        "配对码只能用一次"
    );

    let mut phone = connect(s.addr, &format!("?token={token}")).await.unwrap();
    let (me, _) = request(&mut phone, 1, json!({"type": "who_am_i"})).await;
    assert_eq!(me["data"]["device"]["name"], "张三的手机");
    assert_eq!(me["data"]["scope"], "daily", "新配对的设备是“日常”权限");
    let (r, _) = request(&mut phone, 2, json!({"type": "list_devices"})).await;
    assert_eq!(r["ok"], false, "日常设备不能管理设备");
    // 自填的署名不算数
    let (sent, _) = request(
        &mut phone,
        3,
        json!({"type": "send", "agent_id": "macro-1", "text": "看看 CPI", "author": "冒充的名字"}),
    )
    .await;
    assert_eq!(
        sent["data"]["message"]["author"],
        json!({"kind": "human", "name": "张三的手机"})
    );

    let (list, _) = request(&mut admin, 3, json!({"type": "list_devices"})).await;
    assert_eq!(list["data"][0]["online"], true);
    let (list, _) = request(
        &mut admin,
        4,
        json!({"type": "rename_device", "device_id": device_id, "name": "张三的新手机"}),
    )
    .await;
    assert_eq!(list["data"][0]["name"], "张三的新手机");
    // 连着的时候改名，之后的署名就用新名字
    let (sent, _) = request(
        &mut phone,
        4,
        json!({"type": "send", "agent_id": "macro-1", "text": "再看看 PPI"}),
    )
    .await;
    assert_eq!(sent["data"]["message"]["author"]["name"], "张三的新手机");
    let (list, _) = request(&mut admin, 5, json!({"type": "revoke_device", "device_id": device_id})).await;
    assert_eq!(list["data"], json!([]));
    // 吊销后连接立即断开，令牌也不能再用
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(msg)) = phone.next().await {
            if matches!(msg, Message::Close(_)) {
                return true;
            }
        }
        true
    })
    .await
    .unwrap();
    assert!(closed);
    assert!(connect(s.addr, &format!("?token={token}")).await.is_err());
}

/// 管理端生成配对码，换到一台设备的令牌和 id。
async fn pair_device(s: &Server, admin: &mut Ws, name: &str) -> (String, String) {
    let (p, _) = request(admin, 900, json!({"type": "create_pairing", "name": name})).await;
    let code = p["data"]["code"].as_str().unwrap();
    let claimed: Value = reqwest::Client::new()
        .post(format!("http://{}/pair", s.addr))
        .json(&json!({"code": code}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (
        claimed["token"].as_str().unwrap().to_string(),
        claimed["device"]["id"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn daily_devices_only_get_daily_requests() {
    let s = start(Some("s3cret")).await;
    let mut admin = connect(s.addr, "?token=s3cret").await.unwrap();
    let (token, id) = pair_device(&s, &mut admin, "李四的手机").await;
    let mut phone = connect(s.addr, &format!("?token={token}")).await.unwrap();

    // 日常：对话、任务、审批、报告，以及两样只读的（路由规则、关了记忆的对话）
    for (n, body) in [
        json!({"type": "list_threads"}),
        json!({"type": "list_tasks"}),
        json!({"type": "list_approvals"}),
        json!({"type": "list_reports"}),
        json!({"type": "list_rules"}),
        json!({"type": "list_memory_off_threads"}),
        json!({"type": "spend_today"}),
    ]
    .into_iter()
    .enumerate()
    {
        let (r, _) = request(&mut phone, n as u64 + 1, body.clone()).await;
        assert_eq!(r["ok"], true, "{body}");
    }
    // 管理：改路由、规则草案、定时任务、记忆、设备
    for (n, body) in [
        json!({"type": "list_rule_drafts"}),
        json!({"type": "list_schedules"}),
        json!({"type": "run_schedule", "schedule_id": "x"}),
        json!({"type": "get_memory", "role_id": "macro"}),
        json!({"type": "clear_memory", "role_id": "macro"}),
        json!({"type": "set_thread_memory", "thread_id": "t", "enabled": false}),
        json!({"type": "list_devices"}),
        json!({"type": "set_device_scope", "device_id": id, "scope": "admin"}),
    ]
    .into_iter()
    .enumerate()
    {
        let (r, _) = request(&mut phone, n as u64 + 100, body.clone()).await;
        assert_eq!(r["ok"], false, "{body}");
        assert!(r["error"].as_str().unwrap().contains("日常"), "{body}: {r}");
    }

    // 给了“管理”，连着的连接下一个请求就能用；收回后又不能用
    let (list, _) = request(
        &mut admin,
        1,
        json!({"type": "set_device_scope", "device_id": id, "scope": "admin"}),
    )
    .await;
    assert_eq!(list["data"][0]["scope"], "admin");
    let (me, _) = request(&mut phone, 200, json!({"type": "who_am_i"})).await;
    assert_eq!(me["data"]["scope"], "admin");
    let (r, _) = request(&mut phone, 201, json!({"type": "list_rule_drafts"})).await;
    assert_eq!(r["ok"], true);
    let (r, _) = request(
        &mut phone,
        202,
        json!({"type": "set_device_scope", "device_id": id, "scope": "daily"}),
    )
    .await;
    assert_eq!(r["data"][0]["scope"], "daily", "管理设备也能管理设备，包括自己");
    let (r, _) = request(&mut phone, 203, json!({"type": "list_rule_drafts"})).await;
    assert_eq!(r["ok"], false);
}

#[tokio::test]
async fn untrusted_origins_are_rejected_before_upgrade() {
    let s = start(None).await;
    let same = format!("http://{}", s.addr);
    // 控制台自己（同源）照常连上。
    let mut ws = connect_headers(s.addr, "", &[("origin", &same)]).await.unwrap();
    let (r, _) = request(&mut ws, 1, json!({"type": "list_roles"})).await;
    assert_eq!(r["ok"], true);
    // 别的网页、本机别的端口、沙箱 iframe（null）：升级前就 403，拿不到 list_roles。
    let other_port = format!("http://127.0.0.1:{}", s.addr.port().wrapping_add(1));
    for origin in ["http://untrusted.example", other_port.as_str(), "null"] {
        assert_eq!(
            connect_headers(s.addr, "", &[("origin", origin)]).await.err(),
            Some(403),
            "{origin}"
        );
    }
    // DNS rebinding：Host 和 Origin 都是攻击者的域名。
    let host = format!("evil.example:{}", s.addr.port());
    let origin = format!("http://{host}");
    let headers = [("host", host.as_str()), ("origin", origin.as_str())];
    assert_eq!(connect_headers(s.addr, "", &headers).await.err(), Some(403));
    // 不带 Origin 的本机程序不受影响。
    assert!(connect_headers(s.addr, "", &[]).await.is_ok());
}

#[tokio::test]
async fn token_and_origin_are_both_checked() {
    let s = start(Some("s3cret")).await;
    let same = format!("http://{}", s.addr);
    assert_eq!(connect_headers(s.addr, "", &[("origin", &same)]).await.err(), Some(401));
    assert_eq!(
        connect_headers(s.addr, "?token=wrong", &[("origin", &same)])
            .await
            .err(),
        Some(401)
    );
    assert!(
        connect_headers(s.addr, "?token=s3cret", &[("origin", &same)])
            .await
            .is_ok()
    );
    // 令牌对了，来源不对也不行。
    let bad = [("origin", "http://untrusted.example")];
    assert_eq!(connect_headers(s.addr, "?token=s3cret", &bad).await.err(), Some(403));
    // 配了令牌时 Host 不限：反向代理保留 Host 的情形同源即可。
    let kept = [("host", "ip.example.com"), ("origin", "https://ip.example.com")];
    assert!(connect_headers(s.addr, "?token=s3cret", &kept).await.is_ok());
    // 反向代理把 Host 改成上游地址时，对外地址要写进 allowed_origins。
    let proxied = [("origin", "https://ip.example.com")];
    assert_eq!(
        connect_headers(s.addr, "?token=s3cret", &proxied).await.err(),
        Some(403)
    );
}

#[tokio::test]
async fn allowed_origins_from_config() {
    let mut s = start(Some("s3cret")).await;
    let origins = vec![crate::origin::Origin::parse("https://ip.example.com").unwrap()];
    s.app = s.app.clone().with_allowed_origins(origins);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let r = router(Some(s.app.clone()));
    tokio::spawn(async move { axum::serve(listener, r).await.unwrap() });
    let proxied = [("origin", "https://ip.example.com")];
    assert!(connect_headers(addr, "?token=s3cret", &proxied).await.is_ok());
    assert_eq!(connect_headers(addr, "", &proxied).await.err(), Some(401));
    let bad = [("origin", "https://other.example.com")];
    assert_eq!(connect_headers(addr, "?token=s3cret", &bad).await.err(), Some(403));
    // 配置里写错了，启动前就报出来。
    let text = EXAMPLE.replacen("org_prompt:", "allowed_origins: [ip.example.com]\norg_prompt:", 1);
    let err = Config::from_yaml(&text).unwrap_err().to_string();
    assert!(err.contains("allowed_origins"), "{err}");
}

#[tokio::test]
async fn close_disconnects_sessions() {
    let s = start(None).await;
    let mut ws = connect(s.addr, "").await.unwrap();
    s.app.close();
    let end = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => break,
                _ => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "关闭后会话应断开");
}

const EXAMPLE: &str = r#"
data_dir: ./data
org_prompt: 结论先行。
sandbox: { mode: none }
endpoints:
  deepseek: { base_url: "https://api.deepseek.com/v1", api_key_env: DEEPSEEK_API_KEY, max_concurrency: 8 }
roles:
  - id: macro
    name: 宏观研究员
    description: 撰写宏观研报。
    model: { default: deepseek/deepseek-chat }
agents:
  - { id: macro-1, role: macro, name: 小宏 }
  - { id: macro-2, role: macro, name: 小观, tools: false }
"#;

#[test]
fn parses_example_config() {
    let c = Config::from_yaml(EXAMPLE).unwrap();
    assert_eq!(c.agents.len(), 2);
    assert!(c.agents[0].tools && !c.agents[1].tools);
    assert_eq!(c.endpoints["deepseek"].max_concurrency, Some(8));
    // 没写 memory 时默认关。
    assert!(!c.memory.enabled && c.memory.summary_max_chars == 10_000);
    let on = EXAMPLE.replacen(
        "org_prompt:",
        "memory: { enabled: true, roles: [macro] }\norg_prompt:",
        1,
    );
    let c = Config::from_yaml(&on).unwrap();
    assert!(c.memory.enabled_for(&"macro".into()) && !c.memory.enabled_for(&"ops".into()));
    // 没写 utc_offset 时按 UTC 显示时间；写了就换成分钟数。
    assert_eq!(c.utc_offset_minutes().unwrap(), 0);
    for (offset, minutes) in [("+08:00", 480), ("-05:30", -330), ("UTC", 0)] {
        let text = EXAMPLE.replacen("org_prompt:", &format!("utc_offset: \"{offset}\"\norg_prompt:"), 1);
        assert_eq!(
            Config::from_yaml(&text).unwrap().utc_offset_minutes().unwrap(),
            minutes,
            "{offset}"
        );
    }
}

#[test]
fn rejects_inconsistent_config() {
    for (from, to, why) in [
        ("deepseek/deepseek-chat", "openai/gpt", "未定义的端点"),
        ("deepseek/deepseek-chat", "deepseek-chat", "缺少端点前缀"),
        (
            "{ id: macro-1, role: macro",
            "{ id: macro-2, role: macro",
            "重复的员工 id",
        ),
        ("{ id: macro-1, role: macro", "{ id: macro-9, role: ops", "不存在的岗位"),
        ("{ id: macro-1,", "{ id: ../etc,", "不安全的员工 id"),
        ("org_prompt:", "org_promt:", "拼错的字段"),
        (
            "org_prompt:",
            "memory: { enabled: true, roles: [nobody] }\norg_prompt:",
            "memory.roles 里不存在的岗位",
        ),
        (
            "org_prompt:",
            "memory: { enable: true }\norg_prompt:",
            "memory 里拼错的字段",
        ),
        (
            "org_prompt:",
            "memory: { enabled: true, extract_model: nowhere/m }\norg_prompt:",
            "memory.extract_model 用了没定义的端点",
        ),
        (
            "org_prompt:",
            "utc_offset: \"8\"\norg_prompt:",
            "utc_offset 少了符号和分钟",
        ),
        (
            "org_prompt:",
            "utc_offset: \"+25:00\"\norg_prompt:",
            "utc_offset 超出范围",
        ),
    ] {
        let text = EXAMPLE.replacen(from, to, 1);
        assert!(Config::from_yaml(&text).is_err(), "{why} 应被拒绝");
    }
}

#[tokio::test]
async fn builds_app_from_config_without_secrets_leaking_into_yaml() {
    let dir = tempfile::tempdir().unwrap();
    let text = EXAMPLE.replace(
        "api_key_env: DEEPSEEK_API_KEY, ",
        "api_key_env: IP_TEST_KEY_THAT_IS_NOT_SET, ",
    );
    std::fs::write(dir.path().join("ip.yaml"), &text).unwrap();
    let c = Config::load(&dir.path().join("ip.yaml")).unwrap();
    assert_eq!(c.data_dir, dir.path().join("./data"));
    let err = App::from_config(&c).await.err().expect("缺少密钥应报错");
    assert!(format!("{err:#}").contains("IP_TEST_KEY_THAT_IS_NOT_SET"), "{err:#}");

    let text = EXAMPLE.replace("api_key_env: DEEPSEEK_API_KEY, ", "");
    std::fs::write(dir.path().join("ip.yaml"), &text).unwrap();
    let c = Config::load(&dir.path().join("ip.yaml")).unwrap();
    let app = App::from_config(&c).await.unwrap();
    assert_eq!(app.runtime().agents().len(), 2);
    assert!(dir.path().join("data/workspaces/macro-1").is_dir());
    assert!(!dir.path().join("data/workspaces/macro-2").exists());
    assert!(dir.path().join("data/ip.db").exists());
    drop(app);

    // 指定工作目录：相对路径按配置文件所在目录换算；目录不存在时启动报错。
    let text = text.replace(
        "{ id: macro-1, role: macro, name: 小宏 }",
        "{ id: macro-1, role: macro, name: 小宏, workspace: proj }",
    );
    std::fs::write(dir.path().join("ip.yaml"), &text).unwrap();
    let c = Config::load(&dir.path().join("ip.yaml")).unwrap();
    assert_eq!(
        c.agents[0].workspace.as_deref(),
        Some(dir.path().join("proj").as_path())
    );
    let err = App::from_config(&c).await.err().expect("工作目录不存在应报错");
    assert!(format!("{err:#}").contains("工作目录"), "{err:#}");
    std::fs::create_dir(dir.path().join("proj")).unwrap();
    App::from_config(&c).await.unwrap();
}

/// backlog E-4：一个 MCP 服务器连不上时，服务照常启动，员工都在，只是用不了它的工具。
#[tokio::test]
async fn app_starts_when_an_mcp_server_is_unreachable() {
    let dir = tempfile::tempdir().unwrap();
    let text = EXAMPLE.replace("api_key_env: DEEPSEEK_API_KEY, ", "").replacen(
        "agents:",
        "mcp_servers:\n  broken: { command: /nonexistent/mcp-server, roles: [macro] }\n  \
         nokey: { command: \"true\", env_from: { TOKEN: IP_TEST_SURELY_UNSET_VAR } }\nagents:",
        1,
    );
    std::fs::write(dir.path().join("ip.yaml"), &text).unwrap();
    let c = Config::load(&dir.path().join("ip.yaml")).unwrap();
    let app = App::from_config(&c).await.unwrap();
    assert_eq!(app.runtime().agents().len(), 2);
    let hub = app.mcp.as_ref().unwrap();
    let down = hub.unavailable();
    let names: Vec<&str> = down.iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, ["broken", "nokey"]);
    // 命令不存在的在后台重连；密钥没设的不重连。
    assert!(down[0].reconnecting && !down[1].reconnecting);
    assert!(hub.specs_for(&"macro".into()).is_empty());
}

#[test]
fn shipped_example_config_is_valid() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ip.example.yaml");
    let c = Config::load(&path).unwrap();
    // 示例数据源随仓库一起提供，路径按配置文件所在目录换算后能打开。
    let data = ip_data::DataHub::open(&c.data_sources, std::path::Path::new("/nonexistent")).unwrap();
    assert!(!data.specs_for(&"macro-researcher".into()).is_empty());
    assert!(data.specs_for(&"ops-tester".into()).is_empty());
}

#[tokio::test]
async fn serves_embedded_console() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let s = start(None).await;
    for (path, expect) in [
        ("/", "200"),
        ("/assets/app.js", "200"),
        ("/assets/style.css", "200"),
        ("/assets/markdown.js", "200"),
        ("/assets/icon-192.png", "200"),
        ("/manifest.webmanifest", "200"),
        ("/sw.js", "200"),
        ("/assets/nope.js", "404"),
    ] {
        let mut stream = tokio::net::TcpStream::connect(s.addr).await.unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n");
        stream.write_all(req.as_bytes()).await.unwrap();
        // 图标是二进制，按字节读再转成文字看响应头。
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let response = String::from_utf8_lossy(&raw);
        assert!(
            response.starts_with(&format!("HTTP/1.1 {expect}")),
            "{path}: {response}"
        );
        if expect == "200" {
            assert!(
                response.contains("content-security-policy: default-src 'self'"),
                "{path}"
            );
        }
    }
}

#[tokio::test]
async fn lists_and_reads_reports() {
    let s = start(None).await;
    let mut ws = connect(s.addr, "").await.unwrap();
    let (list, _) = request(&mut ws, 1, json!({"type": "list_reports", "agent_id": "macro-1"})).await;
    assert_eq!(list["ok"], true, "{list}");
    assert_eq!(list["data"][0]["path"], "reports/2026-11-16-cpi.md");
    let (all, _) = request(&mut ws, 2, json!({"type": "list_reports"})).await;
    assert_eq!(all["data"].as_array().unwrap().len(), 1);

    let (report, _) = request(
        &mut ws,
        3,
        json!({"type": "read_report", "agent_id": "macro-1", "path": "reports/2026-11-16-cpi.md"}),
    )
    .await;
    assert_eq!(report["data"]["text"], "# CPI\n\n同比 2.9%");
    assert_eq!(report["data"]["truncated"], false);

    let (bad, _) = request(
        &mut ws,
        4,
        json!({"type": "read_report", "agent_id": "macro-1", "path": "reports/../ip.db"}),
    )
    .await;
    assert_eq!(bad["ok"], false);
}

#[tokio::test]
async fn send_without_agent_follows_router() {
    let rules = "rules: [{ id: 数据点评, when: { any: [CPI, PPI] }, to: { role: macro } }]";
    let s = start_with(None, Some(rules)).await;
    let mut ws = connect(s.addr, "").await.unwrap();

    let (preview, _) = request(&mut ws, 1, json!({"type": "route", "text": "看下 CPI"})).await;
    assert_eq!(preview["data"]["kind"], "routed");
    assert_eq!(preview["data"]["reason"], "命中规则「数据点评」：含「CPI」");

    let (sent, _) = request(&mut ws, 2, json!({"type": "send", "text": "点评 PPI"})).await;
    assert_eq!(sent["ok"], true, "{sent}");
    assert_eq!(sent["data"]["agent_id"], "macro-1");
    assert_eq!(sent["data"]["decision"]["role"], "macro");

    // 没有规则命中：消息先写进对话，建一张审批卡片等人选岗位。
    let (r, _) = request(
        &mut ws,
        3,
        json!({"type": "send", "text": "今天吃什么", "author": "张三"}),
    )
    .await;
    assert_eq!(r["ok"], true, "{r}");
    assert!(r["data"]["agent_id"].is_null());
    assert_eq!(r["data"]["decision"]["kind"], "needs_human");
    let card = r["data"]["approval_id"].as_str().unwrap().to_string();
    let thread = r["data"]["message"]["thread_id"].as_str().unwrap().to_string();

    let (cards, _) = request(&mut ws, 5, json!({"type": "list_approvals", "pending_only": true})).await;
    assert_eq!(cards["data"][0]["id"], card.as_str());
    assert_eq!(cards["data"][0]["options"][0]["label"], "宏观研究员");

    let (resolved, mut events) = request(
        &mut ws,
        6,
        json!({"type": "resolve_approval", "approval_id": card, "choice": 0, "author": "李四"}),
    )
    .await;
    assert_eq!(resolved["ok"], true, "{resolved}");
    assert_eq!(resolved["data"]["state"]["by"], "李四");
    while !events
        .iter()
        .any(|e| e["event"]["type"] == "turn_finished" && e["event"]["thread_id"] == thread.as_str())
    {
        events.push(next_json(&mut ws).await);
    }
    let (again, _) = request(
        &mut ws,
        7,
        json!({"type": "resolve_approval", "approval_id": card, "choice": 0}),
    )
    .await;
    assert_eq!(again["ok"], false, "同一张卡片只能选一次");

    let (history, _) = request(&mut ws, 8, json!({"type": "history", "thread_id": thread})).await;
    let kinds: Vec<&str> = history["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["author"]["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["human", "system", "agent"]);

    let (rules, _) = request(&mut ws, 4, json!({"type": "list_rules"})).await;
    assert_eq!(rules["data"]["spec"]["rules"][0]["id"], "数据点评");
    assert_eq!(rules["data"]["spec"]["fallback"], "ask");
    assert_eq!(rules["data"]["from_console"], false);

    // 人在卡片上选了岗位 → 生成一条规则草案，等人审核。
    let (drafts, _) = request(&mut ws, 9, json!({"type": "list_rule_drafts"})).await;
    let draft = drafts["data"][0].clone();
    assert_eq!(draft["rule"]["to"]["role"], "macro", "{drafts}");
    assert_eq!(draft["rule"]["when"]["any"], json!(["今天吃"]));
    assert!(
        draft["source"]
            .as_str()
            .unwrap()
            .starts_with("李四 把「今天吃什么」交给了"),
        "{draft}"
    );

    // 写错的规则被拒绝，原规则不变。
    let mut bad = rules["data"]["spec"].clone();
    bad["rules"][0]["to"]["role"] = "nobody".into();
    let (r, _) = request(&mut ws, 10, json!({"type": "save_rules", "spec": bad})).await;
    assert_eq!(r["ok"], false, "{r}");

    // 采纳草案：加进规则保存，立即生效，草案移除。
    let mut spec = rules["data"]["spec"].clone();
    spec["rules"].as_array_mut().unwrap().push(draft["rule"].clone());
    let (r, _) = request(
        &mut ws,
        11,
        json!({"type": "save_rules", "spec": spec, "accepted_draft": draft["id"]}),
    )
    .await;
    assert_eq!(r["ok"], true, "{r}");
    let (preview, _) = request(&mut ws, 12, json!({"type": "route", "text": "今天吃火锅"})).await;
    assert_eq!(preview["data"]["kind"], "routed", "{preview}");
    let (drafts, _) = request(&mut ws, 13, json!({"type": "list_rule_drafts"})).await;
    assert_eq!(drafts["data"], json!([]));
    let (rules, _) = request(&mut ws, 14, json!({"type": "list_rules"})).await;
    assert_eq!(rules["data"]["from_console"], true);

    // 重启后沿用控制台保存的规则。
    let store = Store::open(s._dir.path().join("ip.db")).unwrap();
    let runtime = Runtime::new(store, ModelRegistry::new(), RuntimeConfig::default());
    for a in s.app.runtime().agents() {
        let role = s.app.org.roles[0].clone();
        runtime
            .spawn_agent(AgentSetup {
                agent: a,
                role,
                toolbox: None,
                mcp: None,
                data: None,
                engine: None,
            })
            .unwrap();
    }
    let restarted = App::new(runtime, (*s.app.org).clone(), None);
    crate::rules::load_saved(&restarted).await;
    assert_eq!(restarted.router().spec().rules.len(), 2);
}

#[tokio::test]
async fn tasks_create_update_and_list() {
    let s = start(None).await;
    let mut ws = connect(s.addr, "").await.unwrap();
    let (t, events) = request(
        &mut ws,
        1,
        json!({"type": "create_task", "title": "写 10 月 CPI 点评", "assignee": "macro-1", "author": "张三"}),
    )
    .await;
    assert_eq!(t["ok"], true, "{t}");
    assert_eq!(t["data"]["role"], "macro", "只给负责人时岗位取他的岗位");
    assert_eq!(t["data"]["status"], "open");
    let id = t["data"]["id"].as_str().unwrap().to_string();
    let _ = events;

    let (u, _) = request(
        &mut ws,
        2,
        json!({"type": "update_task", "task_id": id, "status": "done"}),
    )
    .await;
    assert_eq!(u["data"]["status"], "done");
    let (open, _) = request(&mut ws, 3, json!({"type": "list_tasks", "open_only": true})).await;
    assert!(open["data"].as_array().unwrap().is_empty());
    let (all, _) = request(&mut ws, 4, json!({"type": "list_tasks"})).await;
    assert_eq!(all["data"][0]["title"], "写 10 月 CPI 点评");

    for bad in [
        json!({"type": "create_task", "title": " "}),
        json!({"type": "create_task", "title": "x", "assignee": "ghost"}),
        json!({"type": "create_task", "title": "x", "role": "nope"}),
        json!({"type": "update_task", "task_id": "missing", "status": "done"}),
    ] {
        let (r, _) = request(&mut ws, 9, bad.clone()).await;
        assert_eq!(r["ok"], false, "{bad}");
    }
}

#[tokio::test]
async fn agent_question_card_wakes_the_agent_when_answered() {
    let ask = ip_models::ToolCall {
        id: "c1".into(),
        name: "ask_human".into(),
        arguments: json!({
            "question": "研报先写 CPI 还是 PPI？",
            "options": [
                {"label": "CPI", "consequence": "先写消费端"},
                {"label": "PPI", "consequence": "先写生产端"}
            ],
            "recommended": 0,
            "reason": "CPI 明天公布"
        })
        .to_string(),
    };
    let turns = vec![
        ScriptedTurn::ToolCalls(vec![ask]),
        ScriptedTurn::Text("好的，等你决定".into()),
        ScriptedTurn::Text("开始写 PPI".into()),
    ];
    let s = start_scripted(None, None, turns).await;
    let mut ws = connect(s.addr, "").await.unwrap();
    let (sent, mut events) = request(
        &mut ws,
        1,
        json!({"type": "send", "agent_id": "macro-1", "text": "写本周研报"}),
    )
    .await;
    let thread = sent["data"]["message"]["thread_id"].as_str().unwrap().to_string();
    let card = loop {
        if let Some(e) = events.iter().find(|e| e["event"]["type"] == "approval_changed") {
            break e["event"]["approval"].clone();
        }
        events.push(next_json(&mut ws).await);
    };
    assert_eq!(card["subject"]["kind"], "agent_question");
    assert_eq!(card["subject"]["agent_id"], "macro-1");
    assert_eq!(card["recommended"], 0);
    while !events.iter().any(|e| e["event"]["type"] == "turn_finished") {
        events.push(next_json(&mut ws).await);
    }

    let (r, mut events) = request(
        &mut ws,
        2,
        json!({"type": "resolve_approval", "approval_id": card["id"], "choice": 1, "author": "李四"}),
    )
    .await;
    assert_eq!(r["ok"], true, "{r}");
    while !events.iter().any(|e| e["event"]["type"] == "turn_finished") {
        events.push(next_json(&mut ws).await);
    }
    let (history, _) = request(&mut ws, 3, json!({"type": "history", "thread_id": thread})).await;
    let texts: Vec<&str> = history["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["body"]["text"].as_str())
        .collect();
    assert_eq!(
        texts,
        [
            "写本周研报",
            "好的，等你决定",
            "李四 回答了「研报先写 CPI 还是 PPI？」：选「PPI」",
            "开始写 PPI"
        ]
    );
    // 只有系统写的选定说明带类型；人和员工的消息不带这个字段。
    let notices: Vec<&serde_json::Value> = history["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m.get("notice"))
        .collect();
    assert_eq!(notices, [&json!("approval_answer")]);
}

#[tokio::test]
async fn rule_drafts_can_be_dismissed() {
    let s = start(None).await;
    let human = ip_core::Author::human("张三");
    let d = crate::rules::draft_from_override(&s.app, "服务器又挂了", &"macro".into(), &human)
        .await
        .unwrap()
        .unwrap();
    // 同样的草案不重复生成。
    assert!(
        crate::rules::draft_from_override(&s.app, "服务器又挂了", &"macro".into(), &human)
            .await
            .unwrap()
            .is_none()
    );
    let mut ws = connect(s.addr, "").await.unwrap();
    let (r, _) = request(&mut ws, 1, json!({"type": "dismiss_rule_draft", "draft_id": d.id})).await;
    assert_eq!(r["data"], json!([]), "{r}");
    let (r, _) = request(&mut ws, 2, json!({"type": "dismiss_rule_draft", "draft_id": "nope"})).await;
    assert_eq!(r["ok"], false);
}

#[tokio::test]
async fn semantic_matching_routes_or_recommends() {
    let rules = r#"
rules: [{ id: 数据点评, when: { any: [CPI] }, to: { role: macro } }]
semantic: { model: fake/judge, min_confidence: 0.7 }
"#;
    let turns = vec![
        ScriptedTurn::Text(r#"{"role": "macro", "confidence": 0.9, "reason": "问的是利率"}"#.into()),
        ScriptedTurn::Text("好的，结论如下：```json\n{\"role\": \"宏观研究员\", \"confidence\": 0.4}\n```".into()),
        ScriptedTurn::Text("我不确定".into()),
        ScriptedTurn::Text(r#"{"role": "macro", "confidence": 0.5, "reason": "可能和经济有关"}"#.into()),
    ];
    let s = start_scripted(None, Some(rules), turns).await;
    let mut ws = connect(s.addr, "").await.unwrap();

    // 规则命中时不问模型。
    let (r, _) = request(&mut ws, 1, json!({"type": "route", "text": "看下 CPI"})).await;
    assert_eq!(r["data"]["by"], "rule", "{r}");

    let (r, _) = request(&mut ws, 2, json!({"type": "route", "text": "美联储今晚会降息吗"})).await;
    assert_eq!(r["data"]["kind"], "routed", "{r}");
    assert_eq!(r["data"]["by"], "semantic");
    assert_eq!(r["data"]["role"], "macro");
    assert!(r["data"]["reason"].as_str().unwrap().contains("问的是利率"));

    // 把握不够：交给人，但带上推荐；显示名也认。
    let (r, _) = request(&mut ws, 3, json!({"type": "route", "text": "看看这个"})).await;
    assert_eq!(r["data"]["kind"], "needs_human", "{r}");
    assert_eq!(r["data"]["suggested"], "macro");

    // 回答没法解析：走兜底，说明原因。
    let (r, _) = request(&mut ws, 4, json!({"type": "route", "text": "随便"})).await;
    assert_eq!(r["data"]["kind"], "needs_human", "{r}");
    assert!(r["data"]["suggested"].is_null());
    assert!(r["data"]["reason"].as_str().unwrap().contains("语义匹配失败"));

    // 问了模型三次，费用记在“自动分派”名下，不算到岗位上。
    let (spend, _) = request(&mut ws, 40, json!({"type": "spend_today"})).await;
    let usd = spend["data"]["routing_usd"].as_f64().unwrap();
    assert!((usd - 0.003).abs() < 1e-9, "{spend}");
    assert_eq!(spend["data"]["roles"], json!({}));
    assert_eq!(spend["data"]["date"].as_str().unwrap().len(), 10);

    // 发消息时把握不够：审批卡片上把模型的判断标为推荐项。
    let (r, _) = request(&mut ws, 5, json!({"type": "send", "text": "最近经济怎么样"})).await;
    assert_eq!(r["data"]["decision"]["kind"], "needs_human", "{r}");
    let (cards, _) = request(&mut ws, 6, json!({"type": "list_approvals", "pending_only": true})).await;
    assert_eq!(cards["data"][0]["recommended"], 0, "{cards}");
}

#[test]
fn budgets_need_prices() {
    let with_budget = EXAMPLE.replace(
        "    model: { default: deepseek/deepseek-chat }",
        "    model: { default: deepseek/deepseek-chat }\n    budget_per_day_usd: 5",
    );
    let err = Config::from_yaml(&with_budget).err().unwrap().to_string();
    assert!(err.contains("没有在端点的 prices 里配置价格"), "{err}");

    let priced = with_budget.replace(
        "max_concurrency: 8 }",
        "max_concurrency: 8, prices: { deepseek-chat: { input: 0.27, cached_input: 0.07, output: 1.1 } } }",
    );
    let c = Config::from_yaml(&priced).unwrap();
    let p = c.price("deepseek/deepseek-chat").unwrap();
    assert_eq!((p.input, p.cached_input, p.output), (0.27, Some(0.07), 1.1));
    assert!(Config::from_yaml(&priced.replace("input: 0.27", "input: -1")).is_err());
}

/// 验收手册（docs/ip/m2-acceptance.md）里的说法，按示例配置的路由规则分派到手册写的岗位。
/// 改了示例规则或手册时，这里会提醒两边保持一致。
#[test]
fn acceptance_runbook_phrases_route_as_described() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ip.example.yaml");
    let router = Config::load(&path).unwrap().router().unwrap();
    for (text, role) in [
        ("点评一下本月 CPI", "macro-researcher"),
        (
            "下周一投资例会要一页材料：这周国内物价和工业品价格怎么走",
            "coordinator",
        ),
        ("帮我准备一下下周的会", "coordinator"),
        ("ops-demo 的测试报错了，帮忙修一下", "ops-tester"),
    ] {
        let decision = router.route(ip_router::RouteInput {
            text,
            author: Some("张三"),
            via: Some("slack:U1"),
        });
        match decision {
            ip_router::Decision::Routed { role: got, .. } => assert_eq!(got.as_str(), role, "{text}"),
            other => panic!("{text}：{other:?}"),
        }
    }
}

#[tokio::test]
async fn schedules_list_and_run_now() {
    let schedules = r#"
- id: weekly
  description: 每周一写宏观周报
  cron: "0 9 * * 1"
  utc_offset: "+08:00"
  to: { role: macro }
  text: 写本周宏观周报
- id: routed
  cron: "*/5 * * * *"
  text: 看下 CPI
  enabled: false
"#;
    let rules = "rules: [{ id: 数据, when: { any: [CPI] }, to: { role: macro } }]";
    let turns = vec![
        ScriptedTurn::Text("周报写好了".into()),
        ScriptedTurn::Text("CPI 看完了".into()),
    ];
    let s = start_with_schedules(None, Some(rules), Some(schedules), turns).await;
    let mut ws = connect(s.addr, "").await.unwrap();

    let (r, _) = request(&mut ws, 1, json!({"type": "list_schedules"})).await;
    let list = r["data"].as_array().unwrap();
    assert_eq!(list.len(), 2, "{r}");
    let next = list[0]["next_run_ms"].as_i64().unwrap();
    assert!(next > ip_core::now_ms(), "{r}");
    // 北京时间周一 09:00 = UTC 周一 01:00。
    assert_eq!(next.div_euclid(60_000).rem_euclid(1440), 60, "{r}");
    assert_eq!((next.div_euclid(86_400_000) + 4).rem_euclid(7), 1, "{r}");
    assert!(list[1]["next_run_ms"].is_null(), "停用的没有下次运行时间：{r}");
    assert!(list[0]["last_run"].is_null());

    // 立即运行：交给岗位里的员工，开一段新对话，署名定时任务。
    let (r, _) = request(&mut ws, 2, json!({"type": "run_schedule", "schedule_id": "weekly"})).await;
    assert_eq!(r["data"]["agent_id"], "macro-1", "{r}");
    assert_eq!(r["data"]["message"]["author"]["name"], "定时任务·weekly", "{r}");
    let thread = r["data"]["message"]["thread_id"].clone();

    // 不写 to 的按路由规则分派；停用的也能手动运行。
    let (r, _) = request(&mut ws, 3, json!({"type": "run_schedule", "schedule_id": "routed"})).await;
    assert_eq!(r["data"]["decision"]["by"], "rule", "{r}");
    assert_ne!(r["data"]["message"]["thread_id"], thread);

    let (r, _) = request(&mut ws, 4, json!({"type": "list_schedules"})).await;
    let last = &r["data"][0]["last_run"];
    assert_eq!(last["ok"], true, "{r}");
    assert_eq!(last["thread_id"], thread);
    let stored = s.app.runtime().store().setting("schedule.last.weekly").await.unwrap();
    assert_eq!(stored.unwrap()["thread_id"], thread);

    let (r, _) = request(&mut ws, 5, json!({"type": "run_schedule", "schedule_id": "nope"})).await;
    assert_eq!(r["ok"], false);
}

#[test]
fn schedules_are_validated() {
    let base = format!(
        "{EXAMPLE}schedules:\n  - {{ id: daily, cron: \"0 9 * * *\", utc_offset: \"+08:00\", to: {{ role: macro, agent: macro-1 }}, text: 早报 }}\n"
    );
    Config::from_yaml(&base).unwrap();
    for (from, to, why) in [
        ("0 9 * * *", "0 25 * * *", "小时超出范围"),
        ("\"+08:00\"", "\"8\"", "时区偏移格式"),
        ("role: macro, agent: macro-1", "role: nope", "岗位不存在"),
        ("agent: macro-1", "agent: macro-9", "员工不存在"),
        ("text: 早报", "text: \" \"", "消息为空"),
        ("id: daily", "id: \"a b\"", "id 不安全"),
    ] {
        let text = base.replacen(from, to, 1);
        assert!(Config::from_yaml(&text).is_err(), "{why} 应被拒绝");
    }
    let dup = format!("{base}  - {{ id: daily, cron: \"* * * * *\", text: x }}\n");
    assert!(Config::from_yaml(&dup).is_err(), "id 重复应被拒绝");
}

#[tokio::test]
async fn memory_background_task_distills_cooled_threads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("ip.db")).unwrap();
    let mut models = ModelRegistry::new();
    let model = Arc::new(ScriptedModel::new(vec![
        ScriptedTurn::Text("好的".into()),
        ScriptedTurn::Text("标题：小数位\n\n对方这次要求收益率写两位小数。".into()),
        ScriptedTurn::Text("## 怎么和这个团队配合\n某次对方要求收益率写两位小数。".into()),
    ]));
    models.insert("fake", model.clone());
    let role = Role {
        id: "macro".into(),
        name: None,
        description: "撰写宏观研报。".into(),
        model: ModelSlot {
            default: "fake/m".into(),
            escalate: None,
        },
        budget_per_day_usd: None,
    };
    // 闲置时间设成 0，对话一结束就算凉下来，测试不用等。
    let config = RuntimeConfig {
        memory: ip_runtime::memory::MemoryConfig {
            enabled: true,
            min_idle_hours: 0,
            ..Default::default()
        },
        ..RuntimeConfig::default()
    };
    let runtime = Runtime::new(store, models, config);
    runtime
        .spawn_agent(AgentSetup {
            agent: Agent {
                id: "macro-1".into(),
                role: "macro".into(),
                name: "小宏".into(),
            },
            role: role.clone(),
            toolbox: None,
            mcp: None,
            data: None,
            engine: None,
        })
        .unwrap();
    let app = App::new(runtime, OrgSpec { roles: vec![role] }, None);
    let mut events = app.runtime().subscribe();
    let thread = ip_core::ThreadId::new();
    app.runtime()
        .send(
            &"macro-1".into(),
            thread.clone(),
            ip_core::Author::human("张三"),
            "收益率写两位小数",
        )
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(event, ip_runtime::RuntimeEvent::TurnFinished { .. }) {
            break;
        }
    }
    // 让“现在”晚于最后一条消息。
    tokio::time::sleep(Duration::from_millis(5)).await;

    app.start_memory();
    let saved = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(m) = app
                .runtime()
                .store()
                .thread_memory(&"macro".into(), &thread)
                .await
                .unwrap()
            {
                return m;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("后台任务应该在启动后马上跑一轮");
    assert_eq!(saved.slug.as_deref(), Some("小数位"));
    assert_eq!(saved.summary.as_deref(), Some("对方这次要求收益率写两位小数。"));
    // 同一轮里接着合并成岗位记忆摘要。
    let summary = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let got = ip_runtime::memory::get(
                app.runtime().store(),
                ip_runtime::memory::Layer::Summary,
                &"macro".into(),
            )
            .await
            .unwrap();
            if let Some(entry) = got {
                return entry.text;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("提炼之后应该接着合并");
    assert_eq!(summary, "## 怎么和这个团队配合\n某次对方要求收益率写两位小数。");
    assert_eq!(model.requests().len(), 3);
    app.close();
}

#[tokio::test]
async fn memory_page_requests_view_edit_forget_and_clear() {
    use ip_core::{RoleId, ThreadId};

    let server = start(None).await;
    let store = server.app.runtime().store();
    let role: RoleId = "macro".into();
    let thread: ThreadId = "t-decimals".into();
    store
        .put_thread_memory(ip_store::ThreadMemory {
            role_id: role.clone(),
            thread_id: thread.clone(),
            source_updated_ms: 10,
            generated_ms: 20,
            status: ip_store::MemoryStatus::Ok,
            slug: Some("小数位".into()),
            summary: Some("张三这次要求收益率写两位小数。".into()),
            attempts: 0,
            retry_after_ms: None,
        })
        .await
        .unwrap();
    let note_id = store
        .add_memory_note(&role, Some(&thread), "CPI 用同比", 30)
        .await
        .unwrap();
    let mut ws = connect(server.addr, "").await.unwrap();

    // 看：这台服务没开记忆，页面要能看出来。
    let (reply, _) = request(&mut ws, 1, json!({"type": "get_memory", "role_id": "macro"})).await;
    assert_eq!(reply["ok"], true, "{reply}");
    let view = &reply["data"];
    assert_eq!(view["enabled"], false);
    assert_eq!(view["notes"], Value::Null);
    assert_eq!(view["threads"][0]["thread_id"], "t-decimals");
    assert_eq!(view["threads"][0]["title"], "小数位");
    assert_eq!(view["threads"][0]["usage_count"], 0);
    assert_eq!(view["corrections"][0]["note_id"], note_id);
    assert_eq!(view["corrections"][0]["applied_ms"], Value::Null);

    // 改决策者笔记和岗位记忆摘要。
    let (reply, _) = request(
        &mut ws,
        2,
        json!({"type": "save_memory_text", "role_id": "macro", "layer": "notes", "text": "报告用中文。"}),
    )
    .await;
    assert_eq!(reply["data"]["notes"]["text"], "报告用中文。", "{reply}");
    let (reply, _) = request(
        &mut ws,
        3,
        json!({"type": "save_memory_text", "role_id": "macro", "layer": "summary", "text": "## 干活的提示\n先取数。"}),
    )
    .await;
    assert_eq!(reply["data"]["summary"]["text"], "## 干活的提示\n先取数。", "{reply}");
    assert_eq!(reply["data"]["summary_from_threads"], 0);
    let (reply, _) = request(
        &mut ws,
        4,
        json!({"type": "save_memory_text", "role_id": "macro", "layer": "notes", "text": "长".repeat(20_001)}),
    )
    .await;
    assert_eq!(reply["ok"], false, "{reply}");

    // 删一条更正，抹掉一个对话的记忆。
    let (reply, _) = request(
        &mut ws,
        5,
        json!({"type": "delete_memory_note", "role_id": "macro", "note_id": note_id}),
    )
    .await;
    assert_eq!(reply["data"]["corrections"], json!([]), "{reply}");
    let (reply, _) = request(
        &mut ws,
        6,
        json!({"type": "forget_thread_memory", "role_id": "macro", "thread_id": "t-decimals"}),
    )
    .await;
    assert_eq!(reply["data"]["threads"], json!([]), "{reply}");

    // 按对话关记忆、再打开。
    let (reply, _) = request(
        &mut ws,
        7,
        json!({"type": "set_thread_memory", "thread_id": "t-private", "enabled": false}),
    )
    .await;
    assert_eq!(reply["data"], json!(["t-private"]), "{reply}");
    let (reply, _) = request(&mut ws, 8, json!({"type": "list_memory_off_threads"})).await;
    assert_eq!(reply["data"], json!(["t-private"]));
    let (reply, _) = request(
        &mut ws,
        9,
        json!({"type": "set_thread_memory", "thread_id": "t-private", "enabled": true}),
    )
    .await;
    assert_eq!(reply["data"], json!([]));

    // 清空：摘要没了，决策者笔记还在。
    let (reply, _) = request(&mut ws, 10, json!({"type": "clear_memory", "role_id": "macro"})).await;
    assert_eq!(reply["data"]["summary"], Value::Null, "{reply}");
    assert_eq!(reply["data"]["notes"]["text"], "报告用中文。");

    // 不认识的岗位一律拒绝，不会往设置表里写东西。
    for (id, body) in [
        (11, json!({"type": "get_memory", "role_id": "nobody"})),
        (
            12,
            json!({"type": "save_memory_text", "role_id": "nobody", "layer": "notes", "text": "x"}),
        ),
        (13, json!({"type": "clear_memory", "role_id": "nobody"})),
    ] {
        let (reply, _) = request(&mut ws, id, body).await;
        assert_eq!(reply["ok"], false, "{reply}");
    }
    assert!(store.setting("context.notes.nobody").await.unwrap().is_none());
    server.app.close();
}
