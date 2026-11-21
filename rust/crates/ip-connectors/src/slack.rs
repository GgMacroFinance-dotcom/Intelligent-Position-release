//! Slack 渠道（`docs/ip/m2-spec.md` D-11 到 D-15）：用 Socket Mode 收事件，用 Web API 发消息。
//!
//! - 收：`apps.connections.open` 拿到 wss 地址连上；每个信封先原样存下（`Hub::stash`），再回 `envelope_id`
//!   确认，再交给后台任务按顺序处理，确认不会被处理慢的消息耽误。处理时 `auth.test` 或 Hub 暂时失败就退避重试，
//!   成功后才算处理完；进程在确认之后、处理完之前退出，重启时接着处理（D-28）。
//!   收到 `disconnect` 就换新地址重连，断线退避重连。设了 `HTTPS_PROXY` 时 WebSocket 经代理的 CONNECT 隧道连接，
//!   `NO_PROXY` 里的主机直连，规则和 Web API 一样（D-37，见 `ws_proxy`）。
//! - 会话：`<team>/<channel>/<thread_ts>`，顶层消息的 `thread_ts` 就是它自己的 `ts`，回复总在线程里。
//! - 发：`chat.postMessage`（文字用 `markdown` 块，卡片用 Block Kit 按钮）、`chat.update`（卡片选定后去掉按钮）、
//!   `chat.postEphemeral`（不符合 `approvers` 的点击，只给点的人看原因）。
//!
//! 接口和字段按 2026-11 的资料写成，没有对照真实工作区核对；真实验收（规格 M2-5）时要过一遍。

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use ip_core::{Approval, ApprovalState};
use serde_json::{Value, json};
use tokio::sync::{OnceCell, mpsc};
use tokio_tungstenite::tungstenite::Message as WsMessage;

use crate::ws_proxy::WsConnector;
use crate::{
    BoxFuture, CancellationToken, CardClick, Channel, DeliverError, Delivered, Hub, HubError, Inbound, Outbound,
    Sender, Settle, Stashed,
};

pub const DEFAULT_API_URL: &str = "https://slack.com/api";
/// 显示名缓存多久（D-13）。
const NAME_TTL: Duration = Duration::from_secs(24 * 3600);
/// 一条消息里 `markdown` 块的总字数上限是 12,000；留些余量，超过就按段落切成几条发。
const MAX_MARKDOWN: usize = 11_000;
/// section 文字的上限是 3,000 字。
const MAX_SECTION: usize = 2_900;
/// 按钮文字的上限是 75 字。
const MAX_BUTTON: usize = 75;
const FIRST_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(30);
/// 卡片按钮的 `action_id` 前缀，后面接选项序号。
const ACTION_PREFIX: &str = "ip_choice_";

/// 一个 Slack 渠道的连接配置。令牌由调用方从环境变量读好。
#[derive(Clone, Debug)]
pub struct SlackConfig {
    /// `ip.yaml` 里的渠道名。
    pub name: String,
    /// 机器人令牌（`xoxb-`），调 Web API 用。
    pub bot_token: String,
    /// 应用级令牌（`xapp-`，权限 `connections:write`），开 Socket Mode 连接用。
    pub app_token: String,
    /// Web API 地址，默认 [`DEFAULT_API_URL`]；测试里指向假服务器。
    pub api_url: String,
    /// 只接这些频道（含私信的 D 开头 id）里的消息；空表示所有频道。
    pub allow_channels: Vec<String>,
    /// 处理信封暂时失败时怎么重试。
    pub retry: RetryPolicy,
}

/// 处理一个信封暂时失败（`auth.test` 出错、Hub 内部错误）时的重试（D-28）。
/// 重试期间后面的信封照常确认、存下，排在它后面按顺序处理。
#[derive(Clone, Debug)]
pub struct RetryPolicy {
    /// 第一次重试前等多久，之后每次翻倍。
    pub first: Duration,
    pub max: Duration,
    /// 一个信封最多处理几次（含第一次，重启前后累计），之后放弃并留档。
    pub attempts: u32,
}

impl Default for RetryPolicy {
    /// 默认 6 次，间隔 1、2、4、8、16 秒，共约半分钟。
    fn default() -> Self {
        Self {
            first: Duration::from_secs(1),
            max: Duration::from_secs(30),
            attempts: 6,
        }
    }
}

impl RetryPolicy {
    /// 第 `failures` 次失败之后等多久。
    fn wait(&self, failures: u32) -> Duration {
        let factor = 2u32.saturating_pow(failures.saturating_sub(1));
        self.first.saturating_mul(factor).min(self.max)
    }
}

/// 机器人自己（`auth.test`）。
#[derive(Clone, Debug)]
struct Me {
    user_id: String,
    team_id: String,
}

pub struct SlackChannel {
    config: SlackConfig,
    http: reqwest::Client,
    /// 建 Socket Mode 连接用，按 `HTTPS_PROXY`/`NO_PROXY` 走代理（D-37）。
    ws: WsConnector,
    me: OnceCell<Me>,
    names: Mutex<HashMap<String, (String, Instant)>>,
}

/// 一次 Web API 调用失败。
#[derive(Debug)]
enum ApiError {
    /// 网络错误、超时。
    Network(String),
    /// HTTP 层面的错误码；429 带 `Retry-After`。
    Status(u16, Option<Duration>),
    /// `ok: false` 和 Slack 给的错误名，比如 `channel_not_found`。
    Slack(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Network(e) => write!(f, "网络错误：{e}"),
            Self::Status(code, _) => write!(f, "HTTP {code}"),
            Self::Slack(e) => write!(f, "Slack 返回 {e}"),
        }
    }
}

impl From<ApiError> for DeliverError {
    fn from(e: ApiError) -> Self {
        let reason = e.to_string();
        match e {
            ApiError::Network(_) => Self::Retry { after: None, reason },
            ApiError::Status(429, after) => Self::Retry { after, reason },
            ApiError::Status(code, _) if code >= 500 => Self::Retry { after: None, reason },
            ApiError::Status(..) => Self::Permanent(reason),
            ApiError::Slack(name) => match name.as_str() {
                "ratelimited" | "internal_error" | "fatal_error" | "service_unavailable" | "request_timeout" => {
                    Self::Retry { after: None, reason }
                }
                _ => Self::Permanent(reason),
            },
        }
    }
}

/// 一次连接为什么结束。
enum Ended {
    /// 被要求停止。
    Stopped,
    /// Slack 让换连接（`disconnect`），马上重连。
    Refresh,
}

impl SlackChannel {
    pub fn new(config: SlackConfig) -> Arc<Self> {
        Self::with_connector(config, WsConnector::from_env())
    }

    fn with_connector(config: SlackConfig, ws: WsConnector) -> Arc<Self> {
        Arc::new(Self {
            config,
            http: reqwest::Client::new(),
            ws,
            me: OnceCell::new(),
            names: Mutex::default(),
        })
    }

    fn url(&self, method: &str) -> String {
        format!("{}/{method}", self.config.api_url.trim_end_matches('/'))
    }

    /// 调一个 Web API 方法。`body` 为 `None` 时不带请求体（`auth.test` 这类）。
    async fn call(&self, method: &str, token: &str, query: &str, body: Option<&Value>) -> Result<Value, ApiError> {
        let mut url = self.url(method);
        if !query.is_empty() {
            url.push('?');
            url.push_str(query);
        }
        let mut req = self.http.post(url).bearer_auth(token);
        if let Some(b) = body {
            req = req.json(b);
        }
        let resp = req.send().await.map_err(|e| ApiError::Network(e.to_string()))?;
        let status = resp.status().as_u16();
        if status != 200 {
            let after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(ApiError::Status(status, after));
        }
        let v: Value = resp.json().await.map_err(|e| ApiError::Network(e.to_string()))?;
        if v.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(v)
        } else {
            let name = v.get("error").and_then(Value::as_str).unwrap_or("unknown_error");
            Err(ApiError::Slack(name.to_string()))
        }
    }

    async fn bot(&self, method: &str, body: &Value) -> Result<Value, ApiError> {
        self.call(method, &self.config.bot_token, "", Some(body)).await
    }

    async fn me(&self) -> Result<&Me, ApiError> {
        self.me
            .get_or_try_init(|| async {
                let v = self.call("auth.test", &self.config.bot_token, "", None).await?;
                Ok(Me {
                    user_id: str_of(&v, "user_id").to_string(),
                    team_id: str_of(&v, "team_id").to_string(),
                })
            })
            .await
    }

    /// 用户的显示名，缓存 24 小时；查不到时用用户 id。
    async fn display_name(&self, user: &str) -> String {
        if let Some((name, at)) = self.names.lock().unwrap_or_else(|p| p.into_inner()).get(user)
            && at.elapsed() < NAME_TTL
        {
            return name.clone();
        }
        // 用户 id 只含字母和数字，直接放进查询串。
        if !user.chars().all(|c| c.is_ascii_alphanumeric()) {
            return user.to_string();
        }
        let name = match self
            .call("users.info", &self.config.bot_token, &format!("user={user}"), None)
            .await
        {
            Ok(v) => display_of(&v["user"]).unwrap_or_else(|| user.to_string()),
            Err(e) => {
                tracing::warn!(channel = %self.config.name, user, error = %e, "查 Slack 用户名失败");
                return user.to_string();
            }
        };
        self.names
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(user.to_string(), (name.clone(), Instant::now()));
        name
    }

    /// 检查两个令牌都能用：机器人令牌调 `auth.test`，应用级令牌调 `apps.connections.open`（只拿地址，不连）。
    /// 成功时返回工作区和机器人的说明，`ip check` 用。
    pub async fn verify(&self) -> Result<String, String> {
        let me = self
            .me()
            .await
            .map_err(|e| format!("机器人令牌不能用（auth.test）：{e}"))?;
        self.call("apps.connections.open", &self.config.app_token, "", None)
            .await
            .map_err(|e| format!("应用级令牌不能用（apps.connections.open）：{e}"))?;
        Ok(format!(
            "工作区 {}，机器人 {}，Socket Mode 可用",
            me.team_id, me.user_id
        ))
    }

    /// 一次 Socket Mode 连接：连上、存下并确认信封、转交后台，直到要求停止、Slack 让换连接或连接出错。
    async fn connect_once(
        &self,
        hub: &dyn Hub,
        work: &mpsc::UnboundedSender<Stashed>,
        shutdown: &CancellationToken,
    ) -> Result<Ended, String> {
        let open = self
            .call("apps.connections.open", &self.config.app_token, "", None)
            .await
            .map_err(|e| format!("apps.connections.open 失败：{e}"))?;
        let url = str_of(&open, "url");
        let (ws, via) = self
            .ws
            .connect(url)
            .await
            .map_err(|e| format!("连接 Socket Mode 失败：{e}"))?;
        let (mut write, mut read) = ws.split();
        tracing::info!(channel = %self.config.name, proxy = via.as_deref().unwrap_or("直连"), "Slack Socket Mode 已连接");
        loop {
            let frame = tokio::select! {
                () = shutdown.cancelled() => {
                    let _ = write.close().await;
                    return Ok(Ended::Stopped);
                }
                frame = read.next() => frame,
            };
            let text = match frame {
                Some(Ok(WsMessage::Text(t))) => t,
                Some(Ok(WsMessage::Close(_))) | None => return Err("Socket Mode 连接被关闭".into()),
                Some(Ok(_)) => continue,
                Some(Err(e)) => return Err(format!("Socket Mode 连接出错：{e}")),
            };
            let Ok(envelope) = serde_json::from_str::<Value>(text.as_str()) else {
                continue;
            };
            match str_of(&envelope, "type") {
                "hello" => continue,
                "disconnect" => {
                    tracing::info!(channel = %self.config.name, reason = str_of(&envelope, "reason"), "Slack 要求换连接");
                    return Ok(Ended::Refresh);
                }
                _ => {}
            }
            // 要处理的信封都带 envelope_id；不带的不用确认，也没有要处理的。
            let Some(id) = envelope.get("envelope_id").and_then(Value::as_str) else {
                continue;
            };
            // 先存下再确认（Slack 要求 3 秒内确认）：确认之后 Slack 不会再推，这时只在内存里就可能丢。
            // 存不下就不确认，让 Slack 重推。
            let new = match hub.stash(id, text.as_str()).await {
                Ok(new) => new,
                Err(e) => {
                    tracing::warn!(channel = %self.config.name, envelope = id, error = %e, "存信封失败，不确认，等 Slack 重推");
                    continue;
                }
            };
            let ack = json!({ "envelope_id": id }).to_string();
            write
                .send(WsMessage::Text(ack.into()))
                .await
                .map_err(|e| format!("确认信封失败：{e}"))?;
            // 重推的同一个信封：已经处理过，或者已经在排队，确认就行。
            if !new {
                continue;
            }
            let stashed = Stashed {
                id: id.to_string(),
                payload: text.to_string(),
                attempts: 0,
            };
            if work.send(stashed).is_err() {
                return Ok(Ended::Stopped);
            }
        }
    }

    /// 按顺序处理信封：先处理启动前没处理完的（`backlog`），再处理新来的。暂时失败的退避重试，
    /// 处理完、按规则忽略或多次失败放弃后，把结果记给 Hub。停止时正在退避的信封留着，下次启动再处理。
    async fn work(
        self: Arc<Self>,
        backlog: Vec<Stashed>,
        mut queue: mpsc::UnboundedReceiver<Stashed>,
        hub: Arc<dyn Hub>,
        shutdown: CancellationToken,
    ) {
        let mut backlog = VecDeque::from(backlog);
        loop {
            let item = match backlog.pop_front() {
                Some(item) => item,
                None => match queue.recv().await {
                    Some(item) => item,
                    None => break,
                },
            };
            if !self.process(item, hub.as_ref(), &shutdown).await {
                return;
            }
        }
    }

    /// 处理一个信封，直到处理完或放弃。停止时正在退避就返回 false。
    async fn process(&self, item: Stashed, hub: &dyn Hub, shutdown: &CancellationToken) -> bool {
        let policy = &self.config.retry;
        let mut failures = item.attempts;
        loop {
            let outcome = match serde_json::from_str::<Value>(&item.payload) {
                Ok(envelope) => self.handle(&envelope, hub).await,
                Err(_) => Ok(()),
            };
            let settle = match outcome {
                Ok(()) => Settle::Done,
                Err(error) => {
                    failures += 1;
                    if failures >= policy.attempts.max(1) {
                        tracing::error!(channel = %self.config.name, envelope = %item.id, failures, %error, "Slack 事件多次处理失败，放弃");
                        Settle::GaveUp(error)
                    } else {
                        tracing::warn!(channel = %self.config.name, envelope = %item.id, failures, %error, "Slack 事件处理失败，稍后重试");
                        Settle::Failed(error)
                    }
                }
            };
            let retry = matches!(settle, Settle::Failed(_));
            if let Err(e) = hub.settle(&item.id, settle).await {
                tracing::warn!(channel = %self.config.name, envelope = %item.id, error = %e, "记信封处理结果失败");
            }
            if !retry {
                return true;
            }
            tokio::select! {
                () = shutdown.cancelled() => return false,
                () = tokio::time::sleep(policy.wait(failures)) => {}
            }
        }
    }

    /// 处理一个信封。`Err` 表示暂时性的失败，值得重试。
    async fn handle(&self, envelope: &Value, hub: &dyn Hub) -> Result<(), String> {
        let payload = &envelope["payload"];
        match str_of(envelope, "type") {
            "events_api" => self.on_event(payload, hub).await,
            "interactive" if str_of(payload, "type") == "block_actions" => self.on_actions(payload, hub).await,
            _ => Ok(()),
        }
    }

    async fn on_event(&self, payload: &Value, hub: &dyn Hub) -> Result<(), String> {
        let me = self.me().await.map_err(|e| format!("auth.test 失败：{e}"))?.clone();
        let team = payload
            .get("team_id")
            .and_then(Value::as_str)
            .unwrap_or(&me.team_id)
            .to_string();
        let Some(raw) = parse_message(&payload["event"], &me, &team, &self.config.allow_channels) else {
            return Ok(());
        };
        let display_name = self.display_name(&raw.user).await;
        let inbound = Inbound {
            channel: self.config.name.clone(),
            dedupe_key: format!("msg:{}:{}", raw.channel, raw.ts),
            conversation: raw.conversation.as_str().into(),
            sender: Sender {
                external_id: raw.user,
                display_name,
            },
            text: raw.text,
            may_start: raw.may_start,
        };
        match hub.message(inbound).await {
            Ok(()) => Ok(()),
            Err(HubError::Rejected(reason)) => {
                tracing::info!(channel = %self.config.name, %reason, "Slack 消息被拒绝");
                Ok(())
            }
            Err(HubError::Failed(e)) => Err(format!("处理 Slack 消息失败：{e}")),
        }
    }

    /// 一个信封里可能有几次点击；有一次内部出错就整个重试，已经生效的点击由 Hub 去重。
    async fn on_actions(&self, payload: &Value, hub: &dyn Hub) -> Result<(), String> {
        let user = str_of(&payload["user"], "id").to_string();
        let channel = str_of(&payload["channel"], "id").to_string();
        let thread_ts = payload["message"]
            .get("thread_ts")
            .and_then(Value::as_str)
            .map(String::from);
        let Some(actions) = payload.get("actions").and_then(Value::as_array) else {
            return Ok(());
        };
        for action in actions {
            let Some((approval, choice)) = parse_choice(action) else {
                continue;
            };
            let display_name = self.display_name(&user).await;
            let click = CardClick {
                channel: self.config.name.clone(),
                dedupe_key: format!("click:{}:{user}", str_of(action, "action_ts")),
                approval_id: approval.as_str().into(),
                choice,
                sender: Sender {
                    external_id: user.clone(),
                    display_name,
                },
            };
            match hub.click(click).await {
                Ok(()) => {}
                Err(HubError::Rejected(reason)) => {
                    let mut body = json!({ "channel": channel, "user": user, "text": reason });
                    if let Some(ts) = &thread_ts {
                        body["thread_ts"] = json!(ts);
                    }
                    if let Err(e) = self.bot("chat.postEphemeral", &body).await {
                        tracing::warn!(channel = %self.config.name, error = %e, "告诉点击的人失败");
                    }
                }
                Err(HubError::Failed(e)) => return Err(format!("处理卡片点击失败：{e}")),
            }
        }
        Ok(())
    }

    async fn post_text(&self, channel: &str, thread_ts: &str, text: &str) -> Result<Delivered, DeliverError> {
        let mut last = None;
        for chunk in split_text(text, MAX_MARKDOWN) {
            let body = json!({
                "channel": channel,
                "thread_ts": thread_ts,
                "text": preview(&chunk),
                "blocks": [{ "type": "markdown", "text": chunk }],
            });
            let posted = match self.bot("chat.postMessage", &body).await {
                // 工作区不认 markdown 块时退回纯文字，不让回复丢掉。
                Err(ApiError::Slack(e)) if e == "invalid_blocks" => {
                    let plain = json!({ "channel": channel, "thread_ts": thread_ts, "text": chunk });
                    self.bot("chat.postMessage", &plain).await?
                }
                other => other?,
            };
            last = Some(str_of(&posted, "ts").to_string());
        }
        Ok(Delivered {
            external_ref: last.unwrap_or_default(),
        })
    }

    async fn send(&self, out: &Outbound) -> Result<Delivered, DeliverError> {
        let key = out.conversation().0.as_str();
        let Some((_team, channel, thread_ts)) = split_conversation(key) else {
            return Err(DeliverError::Permanent(format!("不是 Slack 会话：{key}")));
        };
        match out {
            Outbound::Text { text, .. } => self.post_text(channel, thread_ts, text).await,
            Outbound::Card { approval, .. } => {
                let body = json!({
                    "channel": channel,
                    "thread_ts": thread_ts,
                    "text": preview(&approval.question),
                    "blocks": card_blocks(approval),
                });
                let posted = self.bot("chat.postMessage", &body).await?;
                Ok(Delivered {
                    external_ref: str_of(&posted, "ts").to_string(),
                })
            }
            Outbound::CardResolved {
                approval, external_ref, ..
            } => {
                let body = json!({
                    "channel": channel,
                    "ts": external_ref,
                    "text": preview(&approval.question),
                    "blocks": resolved_blocks(approval),
                });
                self.bot("chat.update", &body).await?;
                Ok(Delivered {
                    external_ref: external_ref.clone(),
                })
            }
        }
    }
}

impl Channel for SlackChannel {
    fn name(&self) -> &str {
        &self.config.name
    }

    fn run(self: Arc<Self>, hub: Arc<dyn Hub>, shutdown: CancellationToken) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            // 先拿上次没处理完的，再连 Slack：新来的信封都排在它们后面。
            let backlog = hub.stashed().await.unwrap_or_else(|e| {
                tracing::warn!(channel = %self.config.name, error = %e, "读不到上次没处理完的 Slack 事件");
                Vec::new()
            });
            if !backlog.is_empty() {
                tracing::info!(channel = %self.config.name, count = backlog.len(), "接着处理上次没处理完的 Slack 事件");
            }
            let (tx, rx) = mpsc::unbounded_channel();
            let worker = tokio::spawn(self.clone().work(backlog, rx, hub.clone(), shutdown.clone()));
            let mut backoff = FIRST_BACKOFF;
            loop {
                match self.connect_once(hub.as_ref(), &tx, &shutdown).await {
                    Ok(Ended::Stopped) => break,
                    Ok(Ended::Refresh) => backoff = FIRST_BACKOFF,
                    Err(e) => {
                        tracing::warn!(channel = %self.config.name, error = %e, wait = ?backoff, "Slack 连接断开，稍后重连");
                        tokio::select! {
                            () = shutdown.cancelled() => break,
                            () = tokio::time::sleep(backoff) => {}
                        }
                        backoff = (backoff * 2).min(MAX_BACKOFF);
                    }
                }
                if shutdown.is_cancelled() {
                    break;
                }
            }
            drop(tx);
            let _ = worker.await;
        })
    }

    fn deliver<'a>(&'a self, out: &'a Outbound) -> BoxFuture<'a, Result<Delivered, DeliverError>> {
        Box::pin(self.send(out))
    }
}

// ---------------------------------------------------------------------------------------------
// 解析和拼装（纯函数，单独测试）
// ---------------------------------------------------------------------------------------------

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `users.info` 返回的用户里，挑一个给人看的名字。
fn display_of(user: &Value) -> Option<String> {
    let profile = &user["profile"];
    [
        str_of(profile, "display_name"),
        str_of(profile, "real_name"),
        str_of(user, "real_name"),
        str_of(user, "name"),
    ]
    .into_iter()
    .find(|s| !s.trim().is_empty())
    .map(String::from)
}

/// 从消息事件里取出的内容。
#[derive(Debug, PartialEq, Eq)]
struct RawMessage {
    conversation: String,
    channel: String,
    user: String,
    ts: String,
    text: String,
    may_start: bool,
}

/// 消息事件 → 要交给 Hub 的内容；该忽略的返回 `None`（D-12、D-13）。
///
/// 同一条 @机器人 的消息会同时来 `app_mention` 和 `message` 两个事件，两者解析出的内容
/// （包括去重 key 和 `may_start`）完全一样，先到的那个生效。
fn parse_message(event: &Value, me: &Me, team: &str, allow_channels: &[String]) -> Option<RawMessage> {
    let kind = str_of(event, "type");
    if kind != "message" && kind != "app_mention" {
        return None;
    }
    // 机器人发的（包括自己）、编辑、删除、进出频道等都不处理。
    if event.get("bot_id").is_some() {
        return None;
    }
    if let Some(sub) = event.get("subtype").and_then(Value::as_str)
        && !matches!(sub, "thread_broadcast" | "file_share")
    {
        return None;
    }
    let user = event.get("user").and_then(Value::as_str)?;
    if user == me.user_id {
        return None;
    }
    let channel = event.get("channel").and_then(Value::as_str)?;
    if !allow_channels.is_empty() && !allow_channels.iter().any(|c| c == channel) {
        return None;
    }
    let ts = event.get("ts").and_then(Value::as_str)?;
    let thread_ts = event.get("thread_ts").and_then(Value::as_str);
    let raw_text = str_of(event, "text");
    let mention = format!("<@{}>", me.user_id);
    let mentioned = kind == "app_mention" || raw_text.contains(&mention);
    let direct = str_of(event, "channel_type") == "im";
    let text = raw_text.replace(&mention, " ").trim().to_string();
    if text.is_empty() {
        return None;
    }
    Some(RawMessage {
        conversation: format!("{team}/{channel}/{}", thread_ts.unwrap_or(ts)),
        channel: channel.to_string(),
        user: user.to_string(),
        ts: ts.to_string(),
        text,
        // 频道里只有 @机器人 能开启会话；私信的顶层消息也能。线程里的追问只在已绑定时生效。
        may_start: mentioned || (direct && thread_ts.is_none()),
    })
}

/// 按钮的 `value` 是 `<审批 id>:<选项序号>`。
fn parse_choice(action: &Value) -> Option<(String, u32)> {
    if !str_of(action, "action_id").starts_with(ACTION_PREFIX) {
        return None;
    }
    let (approval, choice) = str_of(action, "value").rsplit_once(':')?;
    Some((approval.to_string(), choice.parse().ok()?))
}

/// `<team>/<channel>/<thread_ts>`。
fn split_conversation(key: &str) -> Option<(&str, &str, &str)> {
    let mut parts = key.splitn(3, '/');
    let team = parts.next()?;
    let channel = parts.next()?;
    let thread_ts = parts.next()?;
    (!channel.is_empty() && !thread_ts.is_empty()).then_some((team, channel, thread_ts))
}

/// mrkdwn 里 `&`、`<`、`>` 要转义，否则会被当成链接或提及。
fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max.saturating_sub(1)).collect();
    s.push('…');
    s
}

/// 通知里显示的那行字。
fn preview(text: &str) -> String {
    clip(text.lines().next().unwrap_or_default(), 150)
}

/// 太长的文字按段落切开，每段不超过 `max` 个字；一段本身就超长时硬切。
fn split_text(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for para in text.split_inclusive("\n\n") {
        if current.chars().count() + para.chars().count() > max && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        if para.chars().count() > max {
            let chars: Vec<char> = para.chars().collect();
            for piece in chars.chunks(max) {
                out.push(piece.iter().collect());
            }
        } else {
            current.push_str(para);
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    if out.is_empty() {
        out.push(text.to_string());
    }
    out
}

fn question_section(approval: &Approval) -> Value {
    let mut text = format!("*{}*", escape(&approval.question));
    if !approval.reason.trim().is_empty() {
        text.push('\n');
        text.push_str(&escape(&approval.reason));
    }
    json!({ "type": "section", "text": { "type": "mrkdwn", "text": clip(&text, MAX_SECTION) } })
}

/// 审批卡片（D-14）：问题和理由，一排按钮（推荐项高亮），各选项的后果。
fn card_blocks(approval: &Approval) -> Value {
    let buttons: Vec<Value> = approval
        .options
        .iter()
        .enumerate()
        .take(25)
        .map(|(i, o)| {
            let mut b = json!({
                "type": "button",
                "text": { "type": "plain_text", "text": clip(&o.label, MAX_BUTTON) },
                "action_id": format!("{ACTION_PREFIX}{i}"),
                "value": format!("{}:{i}", approval.id),
            });
            if approval.recommended == Some(i as u32) {
                b["style"] = json!("primary");
            }
            b
        })
        .collect();
    let consequences: Vec<String> = approval
        .options
        .iter()
        .map(|o| format!("• *{}*：{}", escape(&o.label), escape(&o.consequence)))
        .collect();
    json!([
        question_section(approval),
        { "type": "actions", "block_id": format!("ip_approval_{}", approval.id), "elements": buttons },
        { "type": "context", "elements": [{ "type": "mrkdwn", "text": clip(&consequences.join("\n"), MAX_SECTION) }] },
    ])
}

/// 选定后的卡片：去掉按钮，写明谁选了什么。
fn resolved_blocks(approval: &Approval) -> Value {
    let outcome = match &approval.state {
        ApprovalState::Resolved { choice, by, .. } => {
            let label = approval.options.get(*choice as usize).map_or("?", |o| o.label.as_str());
            format!("✅ {} 选了「{}」", escape(by), escape(label))
        }
        ApprovalState::Pending => "⏳ 等待选择".to_string(),
    };
    json!([
        question_section(approval),
        { "type": "context", "elements": [{ "type": "mrkdwn", "text": outcome }] },
    ])
}

#[cfg(test)]
mod tests;
