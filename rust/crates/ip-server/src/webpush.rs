//! Web Push（backlog F-10，`docs/ip/apps-plan.md` 5.3，`docs/ip/m2-spec.md` D-33）。
//!
//! 主机生成自己的 VAPID 密钥；浏览器订阅后，主机按 RFC 8291 加密、RFC 8292 签名，直接发给浏览器厂商的
//! 推送服务。推送服务只见密文，项目方不运营任何服务。哪些事推、推给谁，和 `/ws` 上的通知同一份规则和设置
//! （`notify.rs`）；这台设备的页面在前台时不推，由页面自己显示。

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use aws_lc_rs::aead::{AES_128_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
use aws_lc_rs::agreement::{self, ECDH_P256, UnparsedPublicKey};
use aws_lc_rs::hkdf::{HKDF_SHA256, KeyType, Salt};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ip_store::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Semaphore, broadcast};
use ts_rs::TS;

use crate::notify::{Alert, AlertLevel, Notifier, Outgoing};

const VAPID_KEY: &str = "webpush_vapid";
const SUBSCRIPTIONS_KEY: &str = "push_subscriptions";
/// 一台设备最多几个订阅（同一台设备上的几个浏览器）。
const MAX_PER_DEVICE: usize = 5;
/// 全部最多几个订阅。
const MAX_SUBSCRIPTIONS: usize = 500;
/// 页面报“在前台”后多久内不推（页面每 20 秒报一次）。
const ACTIVE_MS: i64 = 30 * 1000;
/// 推送服务替我们留多久（秒）：设备离线时一天内上线还能收到。
const TTL_SECS: u32 = 24 * 3600;
/// 加密后的记录大小（RFC 8188）。负载加上分隔符和标签要放得进一条记录。
const RECORD_SIZE: u32 = 4096;
/// 负载上限。通知的标题和两句预览远小于它。手机 App 的推送经中转转给 APNs、FCM（D-35），
/// 它们的消息上限是 4 KB，密文转成 base64 再加上外层字段要放得下，所以不是 3000。
const MAX_PAYLOAD: usize = 2800;
/// 同时在发的推送最多几个。
const MAX_IN_FLIGHT: usize = 16;
/// 每个订阅记着最近推过的几个标签，事情在别处处理过时用同一个标签盖掉。
const SENT_TAGS: usize = 64;

/// 已知的推送服务：浏览器厂商的四家。手机 App 的推送中转（D-35）写在 `ip.yaml` 的 `push.hosts` 里。
/// 订阅地址只能是这些主机上的 `https://` 地址，免得有人登记一个内网地址，
/// 让主机替他往内网发请求。
const PUSH_HOSTS: &[&str] = &["fcm.googleapis.com", "web.push.apple.com"];
const PUSH_HOST_SUFFIXES: &[&str] = &[".push.services.mozilla.com", ".notify.windows.com"];

/// `push_key` 的回复：浏览器订阅时要用的主机公钥（VAPID）。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct VapidKey {
    /// 未压缩的 P-256 公钥，base64url。
    pub public_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Subscription {
    /// 设备 id；共用令牌时是 `notify::SHARED`。
    who: String,
    endpoint: String,
    /// 浏览器的 P-256 公钥，base64url。
    p256dh: String,
    /// 16 字节的认证密钥，base64url。
    auth: String,
    created_ms: i64,
}

pub struct WebPush {
    store: Store,
    /// VAPID 的 `sub`：推送服务有问题时找谁（`mailto:` 或 `https://`）。Safari 要有。
    contact: Option<String>,
    vapid: OnceLock<Vapid>,
    subs: RwLock<Vec<Subscription>>,
    /// 串行化订阅和密钥的读改写。
    write: tokio::sync::Mutex<()>,
    /// 设备（或共用令牌）的页面在前台到什么时候。
    active: Mutex<HashMap<String, i64>>,
    /// 每个订阅最近推过的标签和标题。
    sent: Mutex<HashMap<String, VecDeque<(String, String)>>>,
    http: reqwest::Client,
    in_flight: Arc<Semaphore>,
    started: AtomicBool,
    /// `ip.yaml` 的 `push.hosts`：另外放行的推送服务主机名（自己搭的分发器或中转），已小写。
    extra_hosts: Vec<String>,
    /// 测试时允许 `http://127.0.0.1` 的订阅地址（假推送服务）。
    loopback_ok: AtomicBool,
}

struct Vapid {
    key: EcdsaKeyPair,
    public: String,
}

impl WebPush {
    pub fn new(store: Store, contact: Option<String>) -> Self {
        Self {
            store,
            contact,
            vapid: OnceLock::new(),
            subs: RwLock::new(Vec::new()),
            write: tokio::sync::Mutex::new(()),
            active: Mutex::new(HashMap::new()),
            sent: Mutex::new(HashMap::new()),
            // 不跟随跳转：推送服务不会让人跳，跟了就可能被引到白名单外的地址。
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
            in_flight: Arc::new(Semaphore::new(MAX_IN_FLIGHT)),
            started: AtomicBool::new(false),
            extra_hosts: Vec::new(),
            loopback_ok: AtomicBool::new(false),
        }
    }

    /// 另外放行的推送服务主机名（`push.hosts`，配置检查时已核对过格式）。
    pub fn with_hosts(mut self, hosts: &[String]) -> Self {
        self.extra_hosts = hosts.iter().map(|h| h.to_ascii_lowercase()).collect();
        self
    }

    /// 启动时读订阅和 VAPID 密钥（没有就生成一个存起来）。
    pub async fn load(&self) -> Result<(), String> {
        let subs: Vec<Subscription> = self.stored(SUBSCRIPTIONS_KEY).await?.unwrap_or_default();
        *self.subs.write().unwrap_or_else(std::sync::PoisonError::into_inner) = subs;
        self.vapid().await?;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn allow_loopback_endpoints(&self) {
        self.loopback_ok.store(true, Ordering::SeqCst);
    }

    /// 主机的 VAPID 公钥。
    pub async fn public_key(&self) -> Result<VapidKey, String> {
        Ok(VapidKey {
            public_key: self.vapid().await?.public.clone(),
        })
    }

    /// 登记一个订阅。同一个地址再登记就更新（浏览器换了密钥）。
    pub async fn subscribe(
        &self,
        who: &str,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        now_ms: i64,
    ) -> Result<(), String> {
        self.check_endpoint(endpoint)?;
        let key = B64
            .decode(p256dh.trim_end_matches('='))
            .map_err(|_| "p256dh 不是 base64url")?;
        if key.len() != 65
            || key[0] != 4
            || agreement::ParsedPublicKey::try_from(&UnparsedPublicKey::new(&ECDH_P256, &key)).is_err()
        {
            return Err("p256dh 不是 P-256 公钥".into());
        }
        let secret = B64
            .decode(auth.trim_end_matches('='))
            .map_err(|_| "auth 不是 base64url")?;
        if secret.len() != 16 {
            return Err("auth 要 16 字节".into());
        }
        let _w = self.write.lock().await;
        let mut subs = self.subs_snapshot();
        subs.retain(|s| s.endpoint != endpoint);
        if subs.iter().filter(|s| s.who == who).count() >= MAX_PER_DEVICE {
            return Err(format!(
                "一台设备最多 {MAX_PER_DEVICE} 个推送订阅，先在别的浏览器里关掉推送"
            ));
        }
        if subs.len() >= MAX_SUBSCRIPTIONS {
            return Err(format!("推送订阅已满 {MAX_SUBSCRIPTIONS} 个"));
        }
        subs.push(Subscription {
            who: who.to_string(),
            endpoint: endpoint.to_string(),
            p256dh: B64.encode(&key),
            auth: B64.encode(&secret),
            created_ms: now_ms,
        });
        self.save(subs).await
    }

    /// 删掉 `who` 的一个订阅。
    pub async fn unsubscribe(&self, who: &str, endpoint: &str) -> Result<(), String> {
        let _w = self.write.lock().await;
        let mut subs = self.subs_snapshot();
        let before = subs.len();
        subs.retain(|s| !(s.who == who && s.endpoint == endpoint));
        if subs.len() == before {
            return Ok(());
        }
        self.save(subs).await
    }

    /// 设备吊销后删掉它的全部订阅。
    pub async fn forget(&self, who: &str) -> Result<(), String> {
        let _w = self.write.lock().await;
        let mut subs = self.subs_snapshot();
        let before = subs.len();
        subs.retain(|s| s.who != who);
        if subs.len() == before {
            return Ok(());
        }
        self.save(subs).await
    }

    /// 页面报一次在不在前台。
    pub fn presence(&self, who: &str, active: bool, now_ms: i64) {
        let mut map = self.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if active {
            map.insert(who.to_string(), now_ms + ACTIVE_MS);
        } else {
            map.remove(who);
        }
    }

    fn is_active(&self, who: &str, now_ms: i64) -> bool {
        self.active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(who)
            .is_some_and(|until| now_ms < *until)
    }

    /// 这个 `who` 有几个订阅（测试和“设备”页用）。
    pub fn count(&self, who: &str) -> usize {
        self.subs
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter(|s| s.who == who)
            .count()
    }

    /// 第一次调用时启动推送任务：收通知规则挑出来的事，按各订阅的设置推出去。之后再调用什么都不做。
    pub fn start(self: &Arc<Self>, notifier: &Arc<Notifier>) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        let notifier = notifier.clone();
        let mut rx = notifier.subscribe();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(out) => me.deliver(&notifier, &out, ip_core::now_ms()).await,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "推送跟不上通知规则，跳过了一些通知");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn deliver(self: &Arc<Self>, notifier: &Notifier, out: &Outgoing, now_ms: i64) {
        let subs = self.subs_snapshot();
        if subs.is_empty() {
            return;
        }
        for sub in subs {
            let message = match out {
                Outgoing::Alert(alert) => {
                    if self.is_active(&sub.who, now_ms) {
                        continue;
                    }
                    let Some(alert) = notifier.settings(&sub.who).filter(alert, now_ms) else {
                        continue;
                    };
                    self.remember(&sub.endpoint, &alert.tag, &alert.title);
                    Message::from_alert(&alert)
                }
                // 只盖掉推过的：没推过的设备不会凭空多一条“已处理”。
                Outgoing::Dismiss(tag) => match self.forget_tag(&sub.endpoint, tag) {
                    Some(title) => Message::handled(tag, &title),
                    None => continue,
                },
            };
            let me = self.clone();
            let Ok(permit) = self.in_flight.clone().acquire_owned().await else {
                return;
            };
            tokio::spawn(async move {
                let _permit = permit;
                match me.send(&sub, &message).await {
                    Ok(Sent::Delivered) => {}
                    Ok(Sent::Gone) => {
                        tracing::info!(who = %sub.who, "推送订阅已失效，删掉");
                        if let Err(e) = me.unsubscribe(&sub.who, &sub.endpoint).await {
                            tracing::warn!("删失效的推送订阅失败：{e}");
                        }
                    }
                    Err(e) => tracing::warn!(who = %sub.who, "推送失败：{e}"),
                }
            });
        }
    }

    async fn send(&self, sub: &Subscription, message: &Message) -> Result<Sent, String> {
        let payload = serde_json::to_vec(message).map_err(|e| e.to_string())?;
        if payload.len() > MAX_PAYLOAD {
            return Err(format!("负载 {} 字节，超过 {MAX_PAYLOAD}", payload.len()));
        }
        let ua_public = B64.decode(&sub.p256dh).map_err(|e| e.to_string())?;
        let auth = B64.decode(&sub.auth).map_err(|e| e.to_string())?;
        let rng = SystemRandom::new();
        let sender = agreement::PrivateKey::generate(&ECDH_P256).map_err(|_| "生成临时密钥失败")?;
        let mut salt = [0u8; 16];
        rng.fill(&mut salt).map_err(|_| "生成随机数失败")?;
        let body = encrypt(&payload, &ua_public, &auth, &sender, &salt)?;
        let url = reqwest::Url::parse(&sub.endpoint).map_err(|e| e.to_string())?;
        let authorization = self.vapid_header(&url, ip_core::now_ms() / 1000).await?;
        let res = self
            .http
            .post(url)
            .header("Authorization", authorization)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("TTL", TTL_SECS.to_string())
            .header("Urgency", message.urgency)
            .header("Topic", topic(&message.tag))
            .body(body)
            .send()
            .await
            // 订阅地址里有浏览器的令牌，不写进日志。
            .map_err(|e| format!("连不上推送服务：{}", e.without_url()))?;
        let status = res.status();
        if status.is_success() {
            return Ok(Sent::Delivered);
        }
        if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE {
            return Ok(Sent::Gone);
        }
        let text = res.text().await.unwrap_or_default();
        Err(format!(
            "推送服务回 {status}：{}",
            text.chars().take(200).collect::<String>()
        ))
    }

    /// RFC 8292 的 `Authorization: vapid t=<JWT>, k=<公钥>`。
    async fn vapid_header(&self, endpoint: &reqwest::Url, now_s: i64) -> Result<String, String> {
        let vapid = self.vapid().await?;
        let mut claims = serde_json::json!({
            "aud": endpoint.origin().ascii_serialization(),
            "exp": now_s + 12 * 3600,
        });
        if let Some(contact) = &self.contact {
            claims["sub"] = contact.clone().into();
        }
        let input = format!(
            "{}.{}",
            B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#),
            B64.encode(claims.to_string())
        );
        let signature = vapid
            .key
            .sign(&SystemRandom::new(), input.as_bytes())
            .map_err(|_| "VAPID 签名失败")?;
        Ok(format!(
            "vapid t={input}.{}, k={}",
            B64.encode(signature.as_ref()),
            vapid.public
        ))
    }

    async fn vapid(&self) -> Result<&Vapid, String> {
        if let Some(v) = self.vapid.get() {
            return Ok(v);
        }
        let _w = self.write.lock().await;
        if let Some(v) = self.vapid.get() {
            return Ok(v);
        }
        let pkcs8 = match self.stored::<String>(VAPID_KEY).await? {
            Some(saved) => B64.decode(saved).map_err(|e| format!("VAPID 密钥格式不对：{e}"))?,
            None => {
                let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &SystemRandom::new())
                    .map_err(|_| "生成 VAPID 密钥失败")?;
                let bytes = doc.as_ref().to_vec();
                self.put(VAPID_KEY, &B64.encode(&bytes)).await?;
                tracing::info!("生成了这台主机的 VAPID 密钥（Web Push 用）");
                bytes
            }
        };
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8)
            .map_err(|e| format!("VAPID 密钥读不出来：{e}"))?;
        let public = B64.encode(key.public_key().as_ref());
        Ok(self.vapid.get_or_init(|| Vapid { key, public }))
    }

    fn check_endpoint(&self, endpoint: &str) -> Result<(), String> {
        let url = reqwest::Url::parse(endpoint).map_err(|_| "订阅地址不是网址")?;
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if self.loopback_ok.load(Ordering::SeqCst) && url.scheme() == "http" && host == "127.0.0.1" {
            return Ok(());
        }
        let known = PUSH_HOSTS.contains(&host.as_str())
            || PUSH_HOST_SUFFIXES.iter().any(|s| host.ends_with(s))
            || self.extra_hosts.contains(&host);
        if url.scheme() != "https" || !known || url.port().is_some() || !url.username().is_empty() {
            return Err(format!("订阅地址不是已知的推送服务：{host}"));
        }
        Ok(())
    }

    fn remember(&self, endpoint: &str, tag: &str, title: &str) {
        let mut sent = self.sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let tags = sent.entry(endpoint.to_string()).or_default();
        tags.retain(|(t, _)| t != tag);
        tags.push_back((tag.to_string(), title.to_string()));
        while tags.len() > SENT_TAGS {
            tags.pop_front();
        }
    }

    fn forget_tag(&self, endpoint: &str, tag: &str) -> Option<String> {
        let mut sent = self.sent.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let tags = sent.get_mut(endpoint)?;
        let at = tags.iter().position(|(t, _)| t == tag)?;
        tags.remove(at).map(|(_, title)| title)
    }

    fn subs_snapshot(&self) -> Vec<Subscription> {
        self.subs
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    async fn save(&self, subs: Vec<Subscription>) -> Result<(), String> {
        self.put(SUBSCRIPTIONS_KEY, &subs).await?;
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|endpoint, _| subs.iter().any(|s| &s.endpoint == endpoint));
        *self.subs.write().unwrap_or_else(std::sync::PoisonError::into_inner) = subs;
        Ok(())
    }

    async fn stored<T: serde::de::DeserializeOwned>(&self, key: &str) -> Result<Option<T>, String> {
        match self.store.setting(key).await.map_err(|e| e.to_string())? {
            Some(v) => serde_json::from_value(v)
                .map(Some)
                .map_err(|e| format!("设置 {key} 格式不对：{e}")),
            None => Ok(None),
        }
    }

    async fn put<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let v = serde_json::to_value(value).map_err(|e| e.to_string())?;
        self.store
            .put_setting(key, v)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

enum Sent {
    Delivered,
    /// 订阅失效（404、410），要删掉。
    Gone,
}

/// 推给 Service Worker 的内容（`sw.js` 的 `push`）。
#[derive(Serialize)]
struct Message {
    title: String,
    body: String,
    tag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<String>,
    /// 盖掉已处理的通知时不响铃。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    silent: bool,
    #[serde(skip)]
    urgency: &'static str,
}

impl Message {
    fn from_alert(alert: &Alert) -> Self {
        Self {
            title: alert.title.clone(),
            body: alert.body.clone(),
            tag: alert.tag.clone(),
            thread_id: alert.thread_id.as_ref().map(ToString::to_string),
            silent: false,
            urgency: if alert.level == AlertLevel::NeedsMe {
                "high"
            } else {
                "normal"
            },
        }
    }

    fn handled(tag: &str, title: &str) -> Self {
        Self {
            title: "已在别处处理".into(),
            body: title.to_string(),
            tag: tag.to_string(),
            thread_id: None,
            silent: true,
            urgency: "low",
        }
    }
}

/// `Topic` 头：同一件事还没送达的旧推送被新的替换。最多 32 个 base64url 字符。
fn topic(tag: &str) -> String {
    B64.encode(Sha256::digest(tag.as_bytes()))[..32].to_string()
}

struct Len(usize);

impl KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]) -> Result<(), String> {
    Salt::new(HKDF_SHA256, salt)
        .extract(ikm)
        .expand(&[info], Len(out.len()))
        .and_then(|okm| okm.fill(out))
        .map_err(|_| "HKDF 失败".to_string())
}

/// RFC 8291：用浏览器的公钥 `ua_public` 和认证密钥 `auth` 加密，`aes128gcm` 编码，单条记录。
/// `sender` 是这次用的临时密钥，`salt` 是 16 字节随机数；测试时用 RFC 附录的固定值。
fn encrypt(
    payload: &[u8],
    ua_public: &[u8],
    auth: &[u8],
    sender: &agreement::PrivateKey,
    salt: &[u8; 16],
) -> Result<Vec<u8>, String> {
    let as_public = sender.compute_public_key().map_err(|_| "算公钥失败")?;
    let as_public = as_public.as_ref();
    let ecdh = agreement::agree(
        sender,
        UnparsedPublicKey::new(&ECDH_P256, ua_public),
        "浏览器的公钥不对".to_string(),
        |secret| Ok(secret.to_vec()),
    )?;
    let mut key_info = Vec::with_capacity(14 + 65 + 65);
    key_info.extend_from_slice(b"WebPush: info\0");
    key_info.extend_from_slice(ua_public);
    key_info.extend_from_slice(as_public);
    let mut ikm = [0u8; 32];
    hkdf(auth, &ecdh, &key_info, &mut ikm)?;
    let mut cek = [0u8; 16];
    hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut cek)?;
    let mut nonce = [0u8; 12];
    hkdf(salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce)?;

    let mut data = Vec::with_capacity(payload.len() + 1 + 16);
    data.extend_from_slice(payload);
    // 最后一条记录的分隔符，不加填充。
    data.push(2);
    if data.len() + 16 > RECORD_SIZE as usize {
        return Err("负载太大，一条记录放不下".into());
    }
    let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).map_err(|_| "内容密钥不对")?);
    key.seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data)
        .map_err(|_| "加密失败")?;

    let id_len = u8::try_from(as_public.len()).map_err(|_| "公钥太长")?;
    let mut out = Vec::with_capacity(16 + 4 + 1 + as_public.len() + data.len());
    out.extend_from_slice(salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(id_len);
    out.extend_from_slice(as_public);
    out.extend_from_slice(&data);
    Ok(out)
}

/// 加密并签名一条推送要多久（毫秒，`ip bench notify` 用）：每次新生成临时密钥、ECDH、HKDF、AES-GCM，再签一个 VAPID。
pub fn time_pushes(payload: &[u8], rounds: usize) -> Result<Vec<f64>, String> {
    let ua = agreement::PrivateKey::generate(&ECDH_P256).map_err(|_| "生成密钥失败")?;
    let ua_public = ua.compute_public_key().map_err(|_| "算公钥失败")?;
    let vapid = EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).map_err(|_| "生成密钥失败")?;
    let rng = SystemRandom::new();
    let mut times = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let started = std::time::Instant::now();
        let sender = agreement::PrivateKey::generate(&ECDH_P256).map_err(|_| "生成密钥失败")?;
        let mut salt = [0u8; 16];
        rng.fill(&mut salt).map_err(|_| "生成随机数失败")?;
        let body = encrypt(payload, ua_public.as_ref(), &[7u8; 16], &sender, &salt)?;
        let sig = vapid.sign(&rng, &body[..64]).map_err(|_| "签名失败")?;
        std::hint::black_box((body, sig));
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(times)
}

#[cfg(test)]
pub(crate) mod tests {
    use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey as SigPublicKey};

    use super::*;

    fn b64(s: &str) -> Vec<u8> {
        B64.decode(s.replace([' ', '\n'], "")).unwrap()
    }

    /// RFC 8291 的逆过程，测试里当浏览器用。
    pub(crate) fn decrypt(body: &[u8], ua: &agreement::PrivateKey, auth: &[u8]) -> Vec<u8> {
        let salt = &body[..16];
        let id_len = body[20] as usize;
        let as_public = &body[21..21 + id_len];
        let ua_public = ua.compute_public_key().unwrap();
        let ecdh = agreement::agree(
            ua,
            UnparsedPublicKey::new(&ECDH_P256, as_public),
            (),
            |s| Ok(s.to_vec()),
        )
        .unwrap();
        let mut key_info = b"WebPush: info\0".to_vec();
        key_info.extend_from_slice(ua_public.as_ref());
        key_info.extend_from_slice(as_public);
        let mut ikm = [0u8; 32];
        hkdf(auth, &ecdh, &key_info, &mut ikm).unwrap();
        let mut cek = [0u8; 16];
        hkdf(salt, &ikm, b"Content-Encoding: aes128gcm\0", &mut cek).unwrap();
        let mut nonce = [0u8; 12];
        hkdf(salt, &ikm, b"Content-Encoding: nonce\0", &mut nonce).unwrap();
        let key = LessSafeKey::new(UnboundKey::new(&AES_128_GCM, &cek).unwrap());
        let mut data = body[21 + id_len..].to_vec();
        let plain = key
            .open_in_place(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut data)
            .unwrap();
        assert_eq!(plain.last(), Some(&2), "最后一条记录的分隔符");
        plain[..plain.len() - 1].to_vec()
    }

    #[test]
    fn matches_the_rfc_8291_example() {
        let sender =
            agreement::PrivateKey::from_private_key(&ECDH_P256, &b64("yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw"))
                .unwrap();
        let ua_public = b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let body = encrypt(
            b"When I grow up, I want to be a watermelon",
            &ua_public,
            &auth,
            &sender,
            &salt,
        )
        .unwrap();
        let expected = b64("DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml
             mlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPT
             pK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN");
        assert_eq!(B64.encode(&body), B64.encode(&expected));
        // 浏览器那边用自己的私钥解得开
        let ua =
            agreement::PrivateKey::from_private_key(&ECDH_P256, &b64("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"))
                .unwrap();
        assert_eq!(decrypt(&body, &ua, &auth), b"When I grow up, I want to be a watermelon");
    }

    fn push() -> (tempfile::TempDir, WebPush) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("ip.db")).unwrap();
        (dir, WebPush::new(store, Some("mailto:ops@example.com".into())))
    }

    fn browser_keys() -> (agreement::PrivateKey, String, String) {
        let ua = agreement::PrivateKey::generate(&ECDH_P256).unwrap();
        let public = B64.encode(ua.compute_public_key().unwrap().as_ref());
        (ua, public, B64.encode([5u8; 16]))
    }

    #[tokio::test]
    async fn vapid_tokens_verify_with_the_hosts_public_key() {
        let (_dir, p) = push();
        let key = p.public_key().await.unwrap().public_key;
        let url = reqwest::Url::parse("https://fcm.googleapis.com/fcm/send/abc").unwrap();
        let header = p.vapid_header(&url, 1_000).await.unwrap();
        let (t, k) = header
            .strip_prefix("vapid t=")
            .and_then(|rest| rest.split_once(", k="))
            .unwrap();
        assert_eq!(k, key);
        let parts: Vec<&str> = t.split('.').collect();
        let claims: serde_json::Value = serde_json::from_slice(&B64.decode(parts[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://fcm.googleapis.com");
        assert_eq!(claims["exp"], 1_000 + 12 * 3600);
        assert_eq!(claims["sub"], "mailto:ops@example.com");
        let signed = format!("{}.{}", parts[0], parts[1]);
        SigPublicKey::new(&ECDSA_P256_SHA256_FIXED, B64.decode(k).unwrap())
            .verify(signed.as_bytes(), &B64.decode(parts[2]).unwrap())
            .expect("签名对得上公钥");

        // 密钥存在库里，重启后是同一把
        let again = WebPush::new(p.store.clone(), None);
        again.load().await.unwrap();
        assert_eq!(again.public_key().await.unwrap().public_key, key);
    }

    #[tokio::test]
    async fn only_known_push_services_can_be_subscribed() {
        let (_dir, p) = push();
        let (_ua, key, auth) = browser_keys();
        for ok in [
            "https://fcm.googleapis.com/fcm/send/abc",
            "https://updates.push.services.mozilla.com/wpush/v2/abc",
            "https://web.push.apple.com/abc",
            "https://wns2-par02p.notify.windows.com/w/?token=abc",
        ] {
            p.subscribe("dev_1", ok, &key, &auth, 0).await.unwrap();
        }
        assert_eq!(p.count("dev_1"), 4);
        for bad in [
            "http://fcm.googleapis.com/fcm/send/abc",
            "https://fcm.googleapis.com:8443/x",
            "https://evil.example/fcm.googleapis.com",
            "https://fcm.googleapis.com.evil.example/x",
            "https://user:pw@fcm.googleapis.com/x",
            "http://127.0.0.1:9/push",
            "https://10.0.0.1/push",
        ] {
            assert!(p.subscribe("dev_1", bad, &key, &auth, 0).await.is_err(), "{bad}");
        }
        assert!(
            p.subscribe("dev_1", "https://web.push.apple.com/x", "AAAA", &auth, 0)
                .await
                .is_err()
        );
        assert!(
            p.subscribe("dev_1", "https://web.push.apple.com/x", &key, "AAAA", 0)
                .await
                .is_err()
        );
        // 一台设备最多 5 个
        p.subscribe("dev_1", "https://web.push.apple.com/5", &key, &auth, 0)
            .await
            .unwrap();
        assert!(
            p.subscribe("dev_1", "https://web.push.apple.com/6", &key, &auth, 0)
                .await
                .is_err()
        );
        // 同一个地址再登记是更新，不算新的
        p.subscribe("dev_1", "https://web.push.apple.com/5", &key, &auth, 1)
            .await
            .unwrap();
        p.unsubscribe("dev_1", "https://web.push.apple.com/5").await.unwrap();
        assert_eq!(p.count("dev_1"), 4);
        p.forget("dev_1").await.unwrap();
        assert_eq!(p.count("dev_1"), 0);
    }

    #[tokio::test]
    async fn configured_push_hosts_are_allowed() {
        let (_dir, p) = push();
        let p = WebPush::new(p.store, None).with_hosts(&["Push.Example.com".into()]);
        let (_ua, key, auth) = browser_keys();
        p.subscribe("dev_1", "https://push.example.com/w/abc", &key, &auth, 0)
            .await
            .unwrap();
        for bad in [
            "http://push.example.com/w/abc",
            "https://push.example.com:8443/w/abc",
            "https://a.push.example.com/w/abc",
            "https://push.example.com.evil.example/w/abc",
        ] {
            assert!(p.subscribe("dev_1", bad, &key, &auth, 0).await.is_err(), "{bad}");
        }
        assert_eq!(p.count("dev_1"), 1);
    }

    #[test]
    fn presence_expires() {
        let (_dir, p) = push();
        p.presence("dev_1", true, 1_000);
        assert!(p.is_active("dev_1", 1_000 + ACTIVE_MS - 1));
        assert!(!p.is_active("dev_1", 1_000 + ACTIVE_MS));
        p.presence("dev_1", false, 2_000);
        assert!(!p.is_active("dev_1", 2_000));
    }

    #[test]
    fn topics_fit_the_header_limit() {
        let t = topic("approval:01JD8Z6Q4M3K2N1P0R9S8T7V6W");
        assert_eq!(t.len(), 32);
        assert!(t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
    }
}
