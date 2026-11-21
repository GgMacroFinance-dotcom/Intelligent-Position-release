//! 封条：把手机的推送令牌用中转自己的密钥加密，放进订阅地址。中转不用数据库，也不存令牌。

use aws_lc_rs::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde::{Deserialize, Serialize};

/// 附加数据：换了封条的格式就改这里，旧封条一律解不开。
const AAD: &[u8] = b"ip-relay seal v1";
const TAG_LEN: usize = 16;
/// 推送令牌最长多少字符。APNs 是 64 个十六进制字符，FCM 一般 150 到 200 个字符。
const MAX_TOKEN: usize = 512;

/// 推给哪个平台。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Apns,
    Fcm,
}

/// 封条里装的东西。字段名取短的，地址短一点。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    #[serde(rename = "p")]
    pub platform: Platform,
    #[serde(rename = "t")]
    pub token: String,
}

impl Target {
    /// 令牌的格式：APNs 是十六进制，FCM 是字母、数字和 `-_:`。不合格的不封，免得地址里装进奇怪的东西。
    pub fn check(&self) -> Result<(), &'static str> {
        let t = &self.token;
        if t.len() < 16 || t.len() > MAX_TOKEN {
            return Err("推送令牌的长度不对");
        }
        let ok = match self.platform {
            Platform::Apns => t.chars().all(|c| c.is_ascii_hexdigit()),
            Platform::Fcm => t
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':')),
        };
        if ok {
            Ok(())
        } else {
            Err("推送令牌里有不该有的字符")
        }
    }
}

pub struct Sealer {
    key: LessSafeKey,
    rng: SystemRandom,
}

impl Sealer {
    /// `key` 是 32 字节的 AES-256 密钥。
    pub fn new(key: &[u8]) -> Result<Self, String> {
        let key = UnboundKey::new(&AES_256_GCM, key).map_err(|_| "封条密钥要是 32 字节".to_string())?;
        Ok(Self {
            key: LessSafeKey::new(key),
            rng: SystemRandom::new(),
        })
    }

    /// 从 base64url 写的密钥建（环境变量里的写法）。
    pub fn from_b64(key: &str) -> Result<Self, String> {
        let raw = B64
            .decode(key.trim())
            .map_err(|_| "封条密钥要用 base64url 写".to_string())?;
        Self::new(&raw)
    }

    /// 新生成一把密钥，base64url。
    pub fn generate_key() -> Result<String, String> {
        let mut key = [0u8; 32];
        SystemRandom::new()
            .fill(&mut key)
            .map_err(|_| "生成随机数失败".to_string())?;
        Ok(B64.encode(key))
    }

    /// 封起来：随机的 12 字节 nonce 加上密文和标签，base64url。
    pub fn seal(&self, target: &Target) -> Result<String, String> {
        let mut nonce = [0u8; NONCE_LEN];
        self.rng.fill(&mut nonce).map_err(|_| "生成随机数失败".to_string())?;
        let mut buf = serde_json::to_vec(target).map_err(|e| e.to_string())?;
        self.key
            .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(AAD), &mut buf)
            .map_err(|_| "加密失败".to_string())?;
        let mut out = Vec::with_capacity(NONCE_LEN + buf.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&buf);
        Ok(B64.encode(out))
    }

    /// 打开封条。被改过、密钥不对、格式不对都返回 `None`。
    pub fn open(&self, sealed: &str) -> Option<Target> {
        if sealed.len() > 2 * MAX_TOKEN {
            return None;
        }
        let raw = B64.decode(sealed).ok()?;
        if raw.len() < NONCE_LEN + TAG_LEN {
            return None;
        }
        let (nonce, rest) = raw.split_at(NONCE_LEN);
        let nonce = Nonce::try_assume_unique_for_key(nonce).ok()?;
        let mut buf = rest.to_vec();
        let plain = self.key.open_in_place(nonce, Aad::from(AAD), &mut buf).ok()?;
        let target: Target = serde_json::from_slice(plain).ok()?;
        target.check().ok()?;
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apns() -> Target {
        Target {
            platform: Platform::Apns,
            token: "a1".repeat(32),
        }
    }

    #[test]
    fn sealed_targets_open_with_the_same_key_only() {
        let s = Sealer::from_b64(&Sealer::generate_key().unwrap()).unwrap();
        let sealed = s.seal(&apns()).unwrap();
        assert_eq!(s.open(&sealed), Some(apns()));
        // 每次封出来的都不一样（随机 nonce），地址看不出是不是同一台手机
        assert_ne!(s.seal(&apns()).unwrap(), sealed);
        let other = Sealer::new(&[7u8; 32]).unwrap();
        assert_eq!(other.open(&sealed), None);
        // 改一个字符就打不开
        let mut bytes = sealed.into_bytes();
        bytes[20] = if bytes[20] == b'A' { b'B' } else { b'A' };
        assert_eq!(s.open(&String::from_utf8(bytes).unwrap()), None);
        assert_eq!(s.open(""), None);
        assert_eq!(s.open("not base64!"), None);
    }

    #[test]
    fn token_formats_are_checked() {
        assert!(apns().check().is_ok());
        let fcm = |t: &str| Target {
            platform: Platform::Fcm,
            token: t.into(),
        };
        assert!(fcm("dGVzdA:APA91bH-abc_DEF0123456789").check().is_ok());
        assert!(fcm("short").check().is_err());
        assert!(fcm(&"a".repeat(MAX_TOKEN + 1)).check().is_err());
        assert!(fcm("has/slash/in/the/token/here").check().is_err());
        let bad_apns = Target {
            platform: Platform::Apns,
            token: "zz".repeat(32),
        };
        assert!(bad_apns.check().is_err());
        assert!(Sealer::new(&[0u8; 16]).is_err());
    }
}
