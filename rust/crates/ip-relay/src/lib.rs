//! 手机 App 锁屏推送的中转（`docs/ip/m2-spec.md` D-35，backlog F-21）。
//!
//! 在主机眼里它就是一个 Web Push 推送服务：
//!
//! - `POST /register {platform, token}`：手机拿 APNs 或 FCM 的设备推送令牌来换一个订阅地址
//!   `<公开地址>/w/<封条>`。令牌用中转自己的密钥封在地址里，中转不存。
//! - `POST /w/<封条>`：主机按 Web Push 发来的密文（`aes128gcm`），原样转给 APNs 或 FCM。
//!   成功回 201，令牌失效回 410（主机会删掉这个订阅），封条打不开回 404。
//!
//! 中转不解密、不记推送内容和令牌，日志里只有次数和上游的状态码。

pub mod seal;
pub mod upstream;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Json, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use std::sync::Arc;

pub use seal::{Platform, Sealer, Target};
pub use upstream::{Apns, ApnsConfig, Fcm, Outcome, Push, ServiceAccount};

/// 一条推送的密文最多多少字节。主机的负载上限是 2800 字节（D-35），加密后约 2900；
/// 转成 base64 再加上外层字段，放得进 APNs、FCM 的 4 KB。
pub const MAX_BODY: usize = 3000;
/// 一个来源地址 10 分钟内最多登记几次。
const REGISTER_PER_WINDOW: u32 = 30;
/// 一个订阅地址 1 分钟内最多推几次。正常的通知远少于它。
const PUSH_PER_WINDOW: u32 = 60;
/// 限流表最多记多少个键，超了先清掉过期的。
const LIMIT_KEYS: usize = 100_000;
/// TTL 最长多少秒（4 周，和 APNs、FCM 的上限一致）。
const MAX_TTL: u32 = 28 * 24 * 3600;

/// 固定窗口的计数限流。
pub struct Limiter {
    per_window: u32,
    window_ms: u64,
    map: Mutex<HashMap<String, (u64, u32)>>,
}

impl Limiter {
    pub fn new(per_window: u32, window_ms: u64) -> Self {
        Self {
            per_window,
            window_ms,
            map: Mutex::new(HashMap::new()),
        }
    }

    pub fn allow(&self, key: &str, now_ms: u64) -> bool {
        let mut map = self.map.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if map.len() >= LIMIT_KEYS && !map.contains_key(key) {
            let window = self.window_ms;
            map.retain(|_, (start, _)| now_ms.saturating_sub(*start) < window);
            if map.len() >= LIMIT_KEYS {
                return false;
            }
        }
        let entry = map.entry(key.to_string()).or_insert((now_ms, 0));
        if now_ms.saturating_sub(entry.0) >= self.window_ms {
            *entry = (now_ms, 0);
        }
        entry.1 += 1;
        entry.1 <= self.per_window
    }
}

/// 只记次数。
#[derive(Default)]
pub struct Stats {
    pub registered: AtomicU64,
    pub sent: AtomicU64,
    pub gone: AtomicU64,
    pub failed: AtomicU64,
}

pub struct Relay {
    sealer: Sealer,
    /// 对外的地址，比如 `https://push.example.com`，不带结尾的 `/`。
    public_url: String,
    apns: Option<Apns>,
    fcm: Option<Fcm>,
    /// 在反向代理后面时，用 `X-Forwarded-For` 的第一个地址限流。
    trust_proxy: bool,
    register_limit: Limiter,
    push_limit: Limiter,
    pub stats: Stats,
}

impl Relay {
    pub fn new(sealer: Sealer, public_url: &str, apns: Option<Apns>, fcm: Option<Fcm>, trust_proxy: bool) -> Self {
        Self {
            sealer,
            public_url: public_url.trim_end_matches('/').to_string(),
            apns,
            fcm,
            trust_proxy,
            register_limit: Limiter::new(REGISTER_PER_WINDOW, 10 * 60 * 1000),
            push_limit: Limiter::new(PUSH_PER_WINDOW, 60 * 1000),
            stats: Stats::default(),
        }
    }

    fn serves(&self, p: Platform) -> bool {
        match p {
            Platform::Apns => self.apns.is_some(),
            Platform::Fcm => self.fcm.is_some(),
        }
    }

    fn client_ip(&self, peer: SocketAddr, headers: &HeaderMap) -> String {
        if self.trust_proxy
            && let Some(first) = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(',').next())
        {
            return first.trim().to_string();
        }
        peer.ip().to_string()
    }
}

pub fn router(relay: Arc<Relay>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/register", post(register))
        .route("/w/{sealed}", post(push))
        .with_state(relay)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

fn error(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

#[derive(Deserialize)]
struct RegisterReq {
    platform: Platform,
    token: String,
}

async fn register(
    State(relay): State<Arc<Relay>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(req): Json<RegisterReq>,
) -> Response {
    if !relay.register_limit.allow(&relay.client_ip(peer, &headers), now_ms()) {
        return error(StatusCode::TOO_MANY_REQUESTS, "登记得太频繁，过一会儿再试");
    }
    if !relay.serves(req.platform) {
        return error(StatusCode::BAD_REQUEST, "这个中转没有配这个平台");
    }
    let target = Target {
        platform: req.platform,
        token: req.token,
    };
    if let Err(e) = target.check() {
        return error(StatusCode::BAD_REQUEST, e);
    }
    match relay.sealer.seal(&target) {
        Ok(sealed) => {
            relay.stats.registered.fetch_add(1, Ordering::Relaxed);
            Json(serde_json::json!({ "endpoint": format!("{}/w/{sealed}", relay.public_url) })).into_response()
        }
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// 从 Web Push 的请求头里取 TTL、Urgency、Topic（RFC 8030）。
fn parse_push(headers: &HeaderMap, body: &[u8]) -> Push {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::trim);
    let ttl = header("ttl")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0)
        .min(MAX_TTL);
    let urgent = header("urgency").is_some_and(|v| v.eq_ignore_ascii_case("high"));
    // RFC 8030：Topic 最多 32 个 base64url 字符。不合格的不转，免得被塞进上游的请求头。
    let topic = header("topic")
        .filter(|t| {
            !t.is_empty() && t.len() <= 32 && t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        })
        .map(str::to_string);
    Push {
        body: STANDARD.encode(body),
        urgent,
        ttl,
        topic,
    }
}

async fn push(
    State(relay): State<Arc<Relay>>,
    Path(sealed): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let encoding = headers.get("content-encoding").and_then(|v| v.to_str().ok());
    if !encoding.is_some_and(|e| e.eq_ignore_ascii_case("aes128gcm")) {
        return error(StatusCode::BAD_REQUEST, "只收 aes128gcm 加密的 Web Push");
    }
    if body.is_empty() || body.len() > MAX_BODY {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "推送太大");
    }
    let Some(target) = relay.sealer.open(&sealed) else {
        return error(StatusCode::NOT_FOUND, "没有这个订阅");
    };
    if !relay.push_limit.allow(&sealed, now_ms()) {
        return error(StatusCode::TOO_MANY_REQUESTS, "推得太频繁");
    }
    let p = parse_push(&headers, &body);
    let outcome = match (target.platform, &relay.apns, &relay.fcm) {
        (Platform::Apns, Some(apns), _) => apns.send(&target.token, &p).await,
        (Platform::Fcm, _, Some(fcm)) => fcm.send(&target.token, &p).await,
        // 中转换了配置、不再管这个平台：当成订阅失效，主机会删掉它，App 下次打开时重新登记。
        _ => Outcome::Gone,
    };
    match outcome {
        Outcome::Sent => {
            relay.stats.sent.fetch_add(1, Ordering::Relaxed);
            StatusCode::CREATED.into_response()
        }
        Outcome::Gone => {
            relay.stats.gone.fetch_add(1, Ordering::Relaxed);
            error(StatusCode::GONE, "订阅已失效")
        }
        Outcome::TooLarge => error(StatusCode::PAYLOAD_TOO_LARGE, "推送太大"),
        Outcome::Failed(why) => {
            relay.stats.failed.fetch_add(1, Ordering::Relaxed);
            tracing::warn!("转发失败：{why}");
            error(StatusCode::BAD_GATEWAY, "转发失败")
        }
    }
}

/// `ip bench relay`：打开封条、取请求头、拼好 APNs 的请求体，每条的耗时（毫秒）。不发网络请求。
pub fn time_forward(rounds: usize) -> Result<Vec<f64>, String> {
    let sealer = Sealer::from_b64(&Sealer::generate_key()?)?;
    let sealed = sealer.seal(&Target {
        platform: Platform::Apns,
        token: "ab".repeat(32),
    })?;
    let mut headers = HeaderMap::new();
    headers.insert("ttl", "86400".parse().map_err(|_| "header")?);
    headers.insert("urgency", "high".parse().map_err(|_| "header")?);
    headers.insert(
        "topic",
        "abcdefghijklmnopqrstuvwxyz012345".parse().map_err(|_| "header")?,
    );
    let body = vec![0x5au8; 2900];
    let mut out = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let started = std::time::Instant::now();
        let target = sealer.open(std::hint::black_box(&sealed)).ok_or("封条打不开")?;
        let p = parse_push(&headers, std::hint::black_box(&body));
        let json = Apns::body(&p).to_string();
        std::hint::black_box((target, json));
        out.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
