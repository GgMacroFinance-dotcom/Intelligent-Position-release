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

/// 一个平台（APNs 或 FCM）的转发结果计数，和最近一次成功、失败的时间（Unix 秒，0 表示还没有过）。
#[derive(Default)]
pub struct PlatformStats {
    pub sent: AtomicU64,
    pub gone: AtomicU64,
    pub too_large: AtomicU64,
    pub failed: AtomicU64,
    pub last_sent_s: AtomicU64,
    pub last_failed_s: AtomicU64,
}

impl PlatformStats {
    fn record(&self, outcome: &Outcome, now_s: u64) {
        let counter = match outcome {
            Outcome::Sent => {
                self.last_sent_s.store(now_s, Ordering::Relaxed);
                &self.sent
            }
            Outcome::Gone => &self.gone,
            Outcome::TooLarge => &self.too_large,
            Outcome::Failed(_) => {
                self.last_failed_s.store(now_s, Ordering::Relaxed);
                &self.failed
            }
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn view(&self) -> serde_json::Value {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        serde_json::json!({
            "sent": get(&self.sent),
            "gone": get(&self.gone),
            "too_large": get(&self.too_large),
            "failed": get(&self.failed),
            "last_sent_s": get(&self.last_sent_s),
            "last_failed_s": get(&self.last_failed_s),
        })
    }
}

/// 只记次数和时间，不记令牌、封条和推送内容。`GET /stats` 原样给出（见 [`Relay::stats_view`]）。
#[derive(Default)]
pub struct Stats {
    pub registered: AtomicU64,
    /// 被限流拒掉的登记和推送。
    pub rate_limited: AtomicU64,
    /// 封条打不开（地址不对，或换过封条密钥）。
    pub not_found: AtomicU64,
    /// 请求本身不合格（不是 aes128gcm、太大、令牌格式不对、平台没配）。
    pub bad_request: AtomicU64,
    pub apns: PlatformStats,
    pub fcm: PlatformStats,
    /// 最近一次转发失败的原因（只有上游的状态码或连不上，不含令牌）。
    last_error: Mutex<Option<(u64, String)>>,
}

impl Stats {
    fn platform(&self, p: Platform) -> &PlatformStats {
        match p {
            Platform::Apns => &self.apns,
            Platform::Fcm => &self.fcm,
        }
    }

    fn bump(c: &AtomicU64) {
        c.fetch_add(1, Ordering::Relaxed);
    }
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
    /// 看 `GET /stats` 要带的令牌。没配时 `/stats` 回 404。
    stats_token: Option<String>,
    started_s: u64,
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
            stats_token: None,
            started_s: now_ms() / 1000,
            stats: Stats::default(),
        }
    }

    /// 打开 `GET /stats`：请求要带 `Authorization: Bearer <令牌>`。令牌太短（少于 16 个字符）时不打开。
    pub fn with_stats_token(mut self, token: Option<String>) -> Result<Self, String> {
        if let Some(t) = &token
            && t.len() < 16
        {
            return Err("IP_RELAY_STATS_TOKEN 至少要 16 个字符".into());
        }
        self.stats_token = token;
        Ok(self)
    }

    /// `GET /stats` 的内容：版本、启动时间、配了哪些平台、各项计数。
    pub fn stats_view(&self) -> serde_json::Value {
        let get = |c: &AtomicU64| c.load(Ordering::Relaxed);
        let s = &self.stats;
        let last_error = s
            .last_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
            .map(|(at, why)| serde_json::json!({ "at_s": at, "reason": why }));
        let now = now_ms() / 1000;
        serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "started_s": self.started_s,
            "uptime_s": now.saturating_sub(self.started_s),
            "now_s": now,
            "platforms": {
                "apns": self.apns.is_some(),
                "fcm": self.fcm.is_some(),
            },
            "registered": get(&s.registered),
            "rate_limited": get(&s.rate_limited),
            "not_found": get(&s.not_found),
            "bad_request": get(&s.bad_request),
            "apns": s.apns.view(),
            "fcm": s.fcm.view(),
            "last_error": last_error,
        })
    }

    fn stats_allowed(&self, headers: &HeaderMap) -> Option<bool> {
        let want = self.stats_token.as_deref()?;
        let got = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .unwrap_or("");
        Some(aws_lc_rs::constant_time::verify_slices_are_equal(got.as_bytes(), want.as_bytes()).is_ok())
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
        .route("/stats", get(stats))
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

async fn stats(State(relay): State<Arc<Relay>>, headers: HeaderMap) -> Response {
    match relay.stats_allowed(&headers) {
        None => StatusCode::NOT_FOUND.into_response(),
        Some(false) => error(StatusCode::UNAUTHORIZED, "令牌不对"),
        Some(true) => Json(relay.stats_view()).into_response(),
    }
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
        Stats::bump(&relay.stats.rate_limited);
        return error(StatusCode::TOO_MANY_REQUESTS, "登记得太频繁，过一会儿再试");
    }
    if !relay.serves(req.platform) {
        Stats::bump(&relay.stats.bad_request);
        return error(StatusCode::BAD_REQUEST, "这个中转没有配这个平台");
    }
    let target = Target {
        platform: req.platform,
        token: req.token,
    };
    if let Err(e) = target.check() {
        Stats::bump(&relay.stats.bad_request);
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
        Stats::bump(&relay.stats.bad_request);
        return error(StatusCode::BAD_REQUEST, "只收 aes128gcm 加密的 Web Push");
    }
    if body.is_empty() || body.len() > MAX_BODY {
        Stats::bump(&relay.stats.bad_request);
        return error(StatusCode::PAYLOAD_TOO_LARGE, "推送太大");
    }
    let Some(target) = relay.sealer.open(&sealed) else {
        Stats::bump(&relay.stats.not_found);
        return error(StatusCode::NOT_FOUND, "没有这个订阅");
    };
    if !relay.push_limit.allow(&sealed, now_ms()) {
        Stats::bump(&relay.stats.rate_limited);
        return error(StatusCode::TOO_MANY_REQUESTS, "推得太频繁");
    }
    let p = parse_push(&headers, &body);
    let outcome = match (target.platform, &relay.apns, &relay.fcm) {
        (Platform::Apns, Some(apns), _) => apns.send(&target.token, &p).await,
        (Platform::Fcm, _, Some(fcm)) => fcm.send(&target.token, &p).await,
        // 中转换了配置、不再管这个平台：当成订阅失效，主机会删掉它，App 下次打开时重新登记。
        _ => Outcome::Gone,
    };
    let now_s = now_ms() / 1000;
    relay.stats.platform(target.platform).record(&outcome, now_s);
    match outcome {
        Outcome::Sent => StatusCode::CREATED.into_response(),
        Outcome::Gone => error(StatusCode::GONE, "订阅已失效"),
        Outcome::TooLarge => error(StatusCode::PAYLOAD_TOO_LARGE, "推送太大"),
        Outcome::Failed(why) => {
            let platform = match target.platform {
                Platform::Apns => "APNs",
                Platform::Fcm => "FCM",
            };
            tracing::warn!("{platform} 转发失败：{why}");
            *relay
                .stats
                .last_error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some((now_s, why));
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
