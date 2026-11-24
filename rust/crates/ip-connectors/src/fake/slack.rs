//! 假 Slack 服务器：Web API 的几个方法和 Socket Mode 的 WebSocket，供测试用。
//!
//! - `apps.connections.open` 返回本服务器的 `ws://…/ws`；连上后先发 `hello`，再把 [`FakeSlack::push`]
//!   推的信封转发过去，并记下客户端回的确认。
//! - `auth.test` 说机器人是 `UBOT`，工作区是 `T1`；`users.info` 按 [`FakeSlack::set_user`] 回答。
//! - `chat.postMessage`、`chat.update`、`chat.postEphemeral` 记下请求体，`ts` 依次是 `100.1`、`100.2`……
//!   [`FakeSlack::fail_next`] 可以让某个方法接下来返回 HTTP 错误或 `ok: false`。

use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use serde_json::{Value, json};
use tokio::sync::broadcast;

pub const BOT_USER: &str = "UBOT";
pub const TEAM: &str = "T1";

/// 让一次调用失败的方式。
#[derive(Clone, Debug)]
pub enum Failure {
    /// HTTP 错误码，可带 `Retry-After` 秒数。
    Status(u16, Option<u64>),
    /// `{"ok": false, "error": …}`。
    Error(String),
    /// 为上传地址检查等测试指定回应。
    Reply(Value),
}

#[derive(Default)]
struct Inner {
    addr: Mutex<Option<SocketAddr>>,
    calls: Mutex<Vec<(String, Value)>>,
    acks: Mutex<Vec<String>>,
    scopes: Mutex<Option<String>>,
    files: AtomicUsize,
    opens: AtomicUsize,
    sockets: AtomicUsize,
    ts: AtomicUsize,
    envelopes: AtomicUsize,
    users: Mutex<HashMap<String, String>>,
    failures: Mutex<HashMap<String, VecDeque<Failure>>>,
    push: Mutex<Option<broadcast::Sender<String>>>,
}

pub struct FakeSlack {
    addr: SocketAddr,
    state: Arc<Inner>,
}

impl FakeSlack {
    pub async fn start() -> Self {
        let (push, _) = broadcast::channel(256);
        let state = Arc::new(Inner::default());
        *state.push.lock().unwrap() = Some(push);
        let app = Router::new()
            .route("/api/{method}", any(api))
            .route("/ws", get(ws))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        *state.addr.lock().unwrap() = Some(addr);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, state }
    }

    /// 给 `SlackConfig::api_url` 用。
    pub fn api_url(&self) -> String {
        format!("http://{}/api", self.addr)
    }

    /// 设置 auth.test 返回的权限列表。
    pub fn set_scopes(&self, scopes: &str) {
        *self.state.scopes.lock().unwrap() = Some(scopes.to_owned());
    }

    pub fn set_user(&self, id: &str, display_name: &str) {
        self.state
            .users
            .lock()
            .unwrap()
            .insert(id.to_string(), display_name.to_string());
    }

    /// 等第 `n` 个 WebSocket 连上（已发 `hello`，能收推送了）。
    pub async fn wait_connected(&self, n: usize) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while self.state.sockets.load(Ordering::SeqCst) < n {
            assert!(
                tokio::time::Instant::now() < deadline,
                "等不到第 {n} 个 Socket Mode 连接"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    /// `apps.connections.open` 被调用的次数。
    pub fn opens(&self) -> usize {
        self.state.opens.load(Ordering::SeqCst)
    }

    /// 往所有连着的 WebSocket 推一条原样的消息。
    pub fn push(&self, message: Value) {
        if let Some(tx) = self.state.push.lock().unwrap().as_ref() {
            let _ = tx.send(message.to_string());
        }
    }

    fn envelope_id(&self) -> String {
        format!("env-{}", self.state.envelopes.fetch_add(1, Ordering::SeqCst) + 1)
    }

    /// 推一个 Events API 事件（`event` 是 `message`、`app_mention` 这类），返回信封 id。
    pub fn event(&self, event: Value) -> String {
        let id = self.envelope_id();
        self.push(json!({
            "envelope_id": id,
            "type": "events_api",
            "accepts_response_payload": false,
            "retry_attempt": 0,
            "payload": {
                "type": "event_callback",
                "team_id": TEAM,
                "event_id": format!("Ev{id}"),
                "event": event,
            },
        }));
        id
    }

    /// 推一次按钮点击（`block_actions`），返回信封 id。
    pub fn click(
        &self,
        user: &str,
        channel: &str,
        thread_ts: &str,
        action_id: &str,
        value: &str,
        action_ts: &str,
    ) -> String {
        let id = self.envelope_id();
        self.push(json!({
            "envelope_id": id,
            "type": "interactive",
            "accepts_response_payload": false,
            "payload": {
                "type": "block_actions",
                "user": { "id": user, "team_id": TEAM },
                "team": { "id": TEAM },
                "channel": { "id": channel },
                "container": { "type": "message", "channel_id": channel },
                "message": { "ts": "100.0", "thread_ts": thread_ts },
                "actions": [{ "type": "button", "action_id": action_id, "value": value, "action_ts": action_ts }],
            },
        }));
        id
    }

    /// 让客户端换连接。
    pub fn disconnect(&self) {
        self.push(json!({ "type": "disconnect", "reason": "refresh_requested" }));
    }

    pub fn acks(&self) -> Vec<String> {
        self.state.acks.lock().unwrap().clone()
    }

    /// 某个方法收到的请求（请求体，或查询参数拼成的对象）。
    pub fn calls(&self, method: &str) -> Vec<Value> {
        self.state
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, b)| b.clone())
            .collect()
    }

    /// 等某个方法至少被调用 `n` 次。
    pub async fn wait_calls(&self, method: &str, n: usize) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let calls = self.calls(method);
            if calls.len() >= n {
                return calls;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "等不到第 {n} 次 {method}：{calls:?}"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn fail_next(&self, method: &str, failure: Failure) {
        self.state
            .failures
            .lock()
            .unwrap()
            .entry(method.to_string())
            .or_default()
            .push_back(failure);
    }
}

fn query_object(uri: &Uri) -> Value {
    let mut map = serde_json::Map::new();
    for pair in uri.query().unwrap_or_default().split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(k.to_string(), Value::String(v.to_string()));
    }
    Value::Object(map)
}

async fn api(
    State(s): State<Arc<Inner>>,
    Path(method): Path<String>,
    uri: Uri,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> Response {
    let mut request = serde_json::from_slice::<Value>(&body).unwrap_or_else(|_| query_object(&uri));
    if method == "files.getUploadURLExternal" {
        let mut url = reqwest::Url::parse("http://localhost").unwrap();
        url.set_query(Some(std::str::from_utf8(&body).unwrap()));
        request = Value::Object(
            url.query_pairs()
                .map(|(k, v)| (k.into_owned(), Value::String(v.into_owned())))
                .collect(),
        );
    } else if method == "upload" {
        request = json!({"markdown":std::str::from_utf8(&body).unwrap(),"authorization":headers.contains_key("authorization")});
    }
    if request.is_null() {
        request = query_object(&uri);
    }
    s.calls.lock().unwrap().push((method.clone(), request.clone()));
    let failure = s
        .failures
        .lock()
        .unwrap()
        .get_mut(&method)
        .and_then(VecDeque::pop_front);
    match failure {
        Some(Failure::Status(code, retry_after)) => {
            let status = StatusCode::from_u16(code).unwrap();
            return match retry_after {
                Some(secs) => (status, [("retry-after", secs.to_string())], "").into_response(),
                None => (status, "").into_response(),
            };
        }
        Some(Failure::Reply(v)) => return axum::Json(v).into_response(),
        Some(Failure::Error(e)) => return axum::Json(json!({ "ok": false, "error": e })).into_response(),
        None => {}
    }
    let reply = match method.as_str() {
        "files.getUploadURLExternal" => {
            let addr = s.addr.lock().unwrap().unwrap();
            json!({"ok":true,"file_id":format!("F{}",s.files.fetch_add(1, Ordering::SeqCst)+1),"upload_url":format!("http://{addr}/api/upload")})
        }
        "upload" => return "OK".into_response(),
        "files.completeUploadExternal" => json!({"ok":true}),
        "apps.connections.open" => {
            s.opens.fetch_add(1, Ordering::SeqCst);
            let addr = s.addr.lock().unwrap().unwrap();
            json!({ "ok": true, "url": format!("ws://{addr}/ws") })
        }
        "auth.test" => json!({ "ok": true, "user_id": BOT_USER, "bot_id": "BBOT", "team_id": TEAM }),
        "users.info" => {
            let id = request["user"].as_str().unwrap_or_default().to_string();
            let name = s.users.lock().unwrap().get(&id).cloned().unwrap_or_default();
            json!({ "ok": true, "user": { "id": id, "name": id.to_lowercase(), "profile": { "display_name": name } } })
        }
        "chat.postMessage" => {
            let ts = format!("100.{}", s.ts.fetch_add(1, Ordering::SeqCst) + 1);
            json!({ "ok": true, "channel": request["channel"], "ts": ts })
        }
        "chat.update" => json!({ "ok": true, "channel": request["channel"], "ts": request["ts"] }),
        "chat.postEphemeral" => json!({ "ok": true, "message_ts": "200.1" }),
        _ => json!({ "ok": false, "error": "unknown_method" }),
    };
    let mut response = axum::Json(reply).into_response();
    if method == "auth.test"
        && let Some(scopes) = s.scopes.lock().unwrap().as_ref()
    {
        response.headers_mut().insert("x-oauth-scopes", scopes.parse().unwrap());
    }
    response
}

async fn ws(State(s): State<Arc<Inner>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| serve_socket(s, socket))
}

async fn serve_socket(s: Arc<Inner>, mut socket: WebSocket) {
    let Some(mut rx) = s.push.lock().unwrap().as_ref().map(broadcast::Sender::subscribe) else {
        return;
    };
    let hello = json!({ "type": "hello", "num_connections": 1, "connection_info": { "app_id": "A1" } });
    if socket.send(Message::Text(hello.to_string().into())).await.is_err() {
        return;
    }
    s.sockets.fetch_add(1, Ordering::SeqCst);
    loop {
        tokio::select! {
            pushed = rx.recv() => {
                let Ok(text) = pushed else { return };
                let disconnect = text.contains("\"disconnect\"");
                if socket.send(Message::Text(text.into())).await.is_err() {
                    return;
                }
                if disconnect {
                    // 和真实的 Slack 一样，发完 disconnect 就关掉这条连接。
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Text(t))) => {
                    if let Ok(v) = serde_json::from_str::<Value>(t.as_str())
                        && let Some(id) = v.get("envelope_id").and_then(Value::as_str)
                    {
                        s.acks.lock().unwrap().push(id.to_string());
                    }
                }
                Some(Ok(_)) => {}
                _ => return,
            },
        }
    }
}
