//! 转给 APNs 和 FCM。两边都只拿到 Web Push 的密文（base64），解密在手机上做。

use std::sync::Mutex;
use std::time::Duration;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, RSA_PKCS1_SHA256, RsaKeyPair};
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use serde::Deserialize;
use serde_json::json;

/// APNs 的令牌（JWT）多久换一次。苹果要求 20 到 60 分钟之间。
const APNS_JWT_SECS: i64 = 40 * 60;
/// Google 的访问令牌提前多久换。
const OAUTH_MARGIN_SECS: i64 = 5 * 60;
/// 解不开时手机上显示的话。正常情况下通知扩展会换成解密后的标题和正文。
const FALLBACK_TITLE: &str = "intelligent-position";
const FALLBACK_BODY: &str = "有一条新通知";
const FCM_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// 一条要转出去的推送。
#[derive(Clone, Debug)]
pub struct Push {
    /// Web Push 的密文（`aes128gcm`），base64。
    pub body: String,
    /// `Urgency: high`。
    pub urgent: bool,
    /// 多少秒内送不到就不送了。
    pub ttl: u32,
    /// 同一件事的推送用同一个 `Topic`，新的替换旧的。
    pub topic: Option<String>,
}

/// 转发的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Sent,
    /// 令牌失效了（App 删了、换了令牌）。主机收到 410 会删掉这个订阅。
    Gone,
    /// 上游嫌太大。
    TooLarge,
    /// 其他错误，只带状态码，不带令牌。
    Failed(String),
}

pub(crate) fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

/// PEM 里的 DER（只认一段，`-----BEGIN ...-----` 和 `-----END ...-----` 之间）。
pub(crate) fn pem_der(pem: &str) -> Result<Vec<u8>, String> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("-----"))
        .collect();
    STANDARD.decode(body).map_err(|_| "PEM 里的密钥读不出来".to_string())
}

fn jwt(
    header: &serde_json::Value,
    claims: &serde_json::Value,
    sign: impl Fn(&[u8]) -> Result<Vec<u8>, String>,
) -> Result<String, String> {
    let signing = format!("{}.{}", B64.encode(header.to_string()), B64.encode(claims.to_string()));
    let sig = sign(signing.as_bytes())?;
    Ok(format!("{signing}.{}", B64.encode(sig)))
}

// ---------- APNs ----------

pub struct ApnsConfig {
    /// `.p8` 文件的内容（PKCS#8 PEM）。
    pub key_pem: String,
    pub key_id: String,
    pub team_id: String,
    /// App 的 Bundle ID。
    pub topic: String,
    /// `https://api.push.apple.com`，测试用 `https://api.sandbox.push.apple.com`。
    pub base_url: String,
}

pub struct Apns {
    cfg: ApnsConfig,
    key: EcdsaKeyPair,
    rng: SystemRandom,
    /// 缓存的 JWT 和签发时间。
    jwt: Mutex<Option<(String, i64)>>,
    http: reqwest::Client,
}

impl Apns {
    pub fn new(cfg: ApnsConfig) -> Result<Self, String> {
        let der = pem_der(&cfg.key_pem)?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &der)
            .map_err(|e| format!("APNs 密钥不是 P-256 的 PKCS#8：{e}"))?;
        Ok(Self {
            cfg,
            key,
            rng: SystemRandom::new(),
            jwt: Mutex::new(None),
            http: http(),
        })
    }

    fn token(&self, now: i64) -> Result<String, String> {
        let mut cached = self.jwt.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((t, iat)) = cached.as_ref()
            && now - iat < APNS_JWT_SECS
        {
            return Ok(t.clone());
        }
        let t = jwt(
            &json!({"alg": "ES256", "kid": self.cfg.key_id}),
            &json!({"iss": self.cfg.team_id, "iat": now}),
            |m| {
                self.key
                    .sign(&self.rng, m)
                    .map(|s| s.as_ref().to_vec())
                    .map_err(|_| "APNs 签名失败".to_string())
            },
        )?;
        *cached = Some((t.clone(), now));
        Ok(t)
    }

    /// 自检：用配好的密钥签一个 APNs 令牌，不发网络请求（`ip-relay check`）。
    pub fn self_check(&self) -> Result<(), String> {
        self.token(now_s()).map(|_| ())
    }

    /// 苹果那边要的请求体：一条可以被通知扩展改写的通知，密文放在 `ip` 里。
    pub fn body(push: &Push) -> serde_json::Value {
        json!({
            "aps": {
                "alert": {"title": FALLBACK_TITLE, "body": FALLBACK_BODY},
                "mutable-content": 1,
                "sound": "default",
            },
            "ip": push.body,
        })
    }

    pub async fn send(&self, token: &str, push: &Push) -> Outcome {
        let now = now_s();
        let jwt = match self.token(now) {
            Ok(t) => t,
            Err(e) => return Outcome::Failed(e),
        };
        let mut req = self
            .http
            .post(format!("{}/3/device/{token}", self.cfg.base_url))
            .bearer_auth(jwt)
            .header("apns-topic", &self.cfg.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", if push.urgent { "10" } else { "5" })
            .header(
                "apns-expiration",
                if push.ttl == 0 { 0 } else { now + i64::from(push.ttl) }.to_string(),
            )
            .json(&Self::body(push));
        if let Some(topic) = &push.topic {
            req = req.header("apns-collapse-id", topic);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(format!("连不上 APNs：{}", e.without_url())),
        };
        let status = resp.status().as_u16();
        if status == 200 {
            return Outcome::Sent;
        }
        let reason = resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(str::to_string))
            .unwrap_or_default();
        if status == 403 && reason == "ExpiredProviderToken" {
            *self.jwt.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
        match (status, reason.as_str()) {
            (410, _) | (400, "BadDeviceToken" | "DeviceTokenNotForTopic") => Outcome::Gone,
            (413, _) => Outcome::TooLarge,
            _ => Outcome::Failed(format!("APNs 回 {status} {reason}")),
        }
    }
}

// ---------- FCM ----------

/// Google 服务账号的 JSON 里用到的字段。
#[derive(Deserialize)]
pub struct ServiceAccount {
    pub project_id: String,
    pub client_email: String,
    pub private_key: String,
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".into()
}

pub struct Fcm {
    account: ServiceAccount,
    key: RsaKeyPair,
    rng: SystemRandom,
    /// `https://fcm.googleapis.com`，测试时换成假的。
    base_url: String,
    /// 缓存的访问令牌和过期时间。
    access: tokio::sync::Mutex<Option<(String, i64)>>,
    http: reqwest::Client,
}

impl Fcm {
    pub fn new(account: ServiceAccount, base_url: String) -> Result<Self, String> {
        let der = pem_der(&account.private_key)?;
        let key = RsaKeyPair::from_pkcs8(&der).map_err(|e| format!("服务账号的私钥读不出来：{e}"))?;
        Ok(Self {
            account,
            key,
            rng: SystemRandom::new(),
            base_url,
            access: tokio::sync::Mutex::new(None),
            http: http(),
        })
    }

    /// 换访问令牌用的 JWT（RS256，服务账号的私钥签）。
    fn assertion(&self, now: i64) -> Result<String, String> {
        jwt(
            &json!({"alg": "RS256", "typ": "JWT"}),
            &json!({
                "iss": self.account.client_email,
                "scope": FCM_SCOPE,
                "aud": self.account.token_uri,
                "iat": now,
                "exp": now + 3600,
            }),
            |m| {
                let mut sig = vec![0u8; self.key.public_modulus_len()];
                self.key
                    .sign(&RSA_PKCS1_SHA256, &self.rng, m, &mut sig)
                    .map_err(|_| "服务账号签名失败".to_string())?;
                Ok(sig)
            },
        )
    }

    /// 自检：用服务账号的私钥签一个换令牌用的 JWT，不发网络请求（`ip-relay check`）。
    pub fn self_check(&self) -> Result<(), String> {
        self.assertion(now_s()).map(|_| ())
    }

    /// 在线自检：真的向 Google 换一次访问令牌（`ip-relay check --online`），不发推送。
    pub async fn check_online(&self) -> Result<(), String> {
        self.access_token().await.map(|_| ())
    }

    /// 用服务账号签一个 JWT 换访问令牌（OAuth 2.0 的 JWT bearer 授权），过期前一直用同一个。
    async fn access_token(&self) -> Result<String, String> {
        let mut cached = self.access.lock().await;
        let now = now_s();
        if let Some((t, exp)) = cached.as_ref()
            && now + OAUTH_MARGIN_SECS < *exp
        {
            return Ok(t.clone());
        }
        let assertion = self.assertion(now)?;
        // JWT 只有 base64url 字符和点，不用再转义。
        let form = format!("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion={assertion}");
        let resp = self
            .http
            .post(&self.account.token_uri)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(form)
            .send()
            .await
            .map_err(|e| format!("换 Google 访问令牌失败：{}", e.without_url()))?;
        if !resp.status().is_success() {
            return Err(format!("换 Google 访问令牌失败：{}", resp.status().as_u16()));
        }
        #[derive(Deserialize)]
        struct Token {
            access_token: String,
            expires_in: i64,
        }
        let t: Token = resp
            .json()
            .await
            .map_err(|_| "Google 回的访问令牌读不出来".to_string())?;
        *cached = Some((t.access_token.clone(), now + t.expires_in));
        Ok(t.access_token)
    }

    /// FCM 的数据消息：不让系统直接显示，App 的后台处理解密后自己弹通知。
    pub fn body(token: &str, push: &Push) -> serde_json::Value {
        let mut android = json!({
            "priority": if push.urgent { "HIGH" } else { "NORMAL" },
            "ttl": format!("{}s", push.ttl),
        });
        if let Some(topic) = &push.topic {
            android["collapse_key"] = json!(topic);
        }
        // `urgency` 给 App 挑通知渠道（要我处理的、其余的），不是秘密：主机的 `Urgency` 头本来就是明文。
        let urgency = if push.urgent { "high" } else { "normal" };
        json!({"message": {"token": token, "data": {"ip": push.body, "urgency": urgency}, "android": android}})
    }

    pub async fn send(&self, token: &str, push: &Push) -> Outcome {
        let access = match self.access_token().await {
            Ok(t) => t,
            Err(e) => return Outcome::Failed(e),
        };
        let url = format!(
            "{}/v1/projects/{}/messages:send",
            self.base_url, self.account.project_id
        );
        let resp = match self
            .http
            .post(url)
            .bearer_auth(access)
            .json(&Self::body(token, push))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return Outcome::Failed(format!("连不上 FCM：{}", e.without_url())),
        };
        match resp.status().as_u16() {
            200 => Outcome::Sent,
            404 => Outcome::Gone,
            401 => {
                *self.access.lock().await = None;
                Outcome::Failed("FCM 回 401".into())
            }
            s => Outcome::Failed(format!("FCM 回 {s}")),
        }
    }
}
