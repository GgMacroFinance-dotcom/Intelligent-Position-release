//! 设备配对和设备令牌（backlog F-7，`docs/ip/apps-plan.md` 4.2、4.4，`docs/ip/m2-spec.md` D-30）。
//!
//! - 配对码：6 位数字，2 分钟内有效，只能用一次。`ip device pair` 或控制台“设备”页生成。
//! - 换到的设备令牌只给这一台设备，服务端只存它的 SHA-256。
//! - 用设备令牌连上的请求，署名一律用设备名。吊销后这台设备的连接立即断开。
//! - 每台设备一档权限（[`Scope`]，backlog F-8）：新配对的是“日常”，“管理”要在“设备”页单独给。
//!
//! 设备和待用的配对码都存在设置表里（`devices`、`device_pairings` 两个键），数量很少，整份读写。
//! 设备列表只有服务进程写（串行）；配对码 `ip device pair` 也会写，所以“只能用一次”不靠删掉配对码保证，
//! 而是看有没有设备已经用过这个配对码的 id。

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, RwLock};

use ip_store::Store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::broadcast;
use ts_rs::TS;

/// 配对码多久后失效。
pub const PAIRING_TTL_MS: i64 = 2 * 60 * 1000;
/// 一个有效期内最多猜错几次；再错就作废所有待用的配对码。
const MAX_FAILED_CLAIMS: usize = 10;
/// 最后在线时间至少隔这么久才写一次库。
const TOUCH_EVERY_MS: i64 = 60 * 1000;
/// 设备名最长几个字。
const MAX_NAME_CHARS: usize = 40;

const DEVICES_KEY: &str = "devices";
const PAIRINGS_KEY: &str = "device_pairings";

/// 设备权限（`docs/ip/apps-plan.md` 4.3）。哪些请求算“日常”见 `ws.rs` 的 `daily_allows`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Scope {
    /// 对话、审批和员工提问、任务、看报告、看岗位和花费。新配对的设备都是这一档。
    #[default]
    Daily,
    /// 另外能改路由规则、定时任务、记忆、管理设备。共用令牌等同这一档。
    Admin,
}

/// 控制台“设备”页看到的一台设备。
#[derive(Clone, Debug, Serialize, TS, PartialEq, Eq)]
#[ts(export)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub scope: Scope,
    #[ts(type = "number")]
    pub paired_at_ms: i64,
    #[ts(type = "number")]
    pub last_seen_ms: i64,
    /// 现在有没有连着。
    pub online: bool,
}

/// `create_pairing` 的回复：配对码只在这里出现一次。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct NewPairing {
    /// 6 位数字。
    pub code: String,
    /// 配对成功后设备叫什么。
    pub name: String,
    #[ts(type = "number")]
    pub expires_ms: i64,
    /// 扫码打开的地址（`<控制台地址>/#/pair/<配对码>`）；没给控制台地址时没有。
    #[ts(optional)]
    pub link: Option<String>,
    /// `link` 的二维码，每行一串 `1`（黑）和 `0`（白），不含四周留白。
    #[ts(optional)]
    pub qr: Option<Vec<String>>,
}

/// `POST /pair` 成功时的回复。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct Claimed {
    /// 这台设备专属的令牌，只在这里出现一次。
    pub token: String,
    pub device: DeviceInfo,
}

/// 连上的是哪台设备。
#[derive(Clone, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub scope: Scope,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DeviceRecord {
    id: String,
    name: String,
    token_sha256: String,
    /// F-8 之前配对的设备没有这一项，按“日常”算。
    #[serde(default)]
    scope: Scope,
    paired_at_ms: i64,
    last_seen_ms: i64,
    /// 用的是哪个配对码（的 id）。同一个配对码只能换一次。
    pairing_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Pairing {
    id: String,
    name: String,
    /// SHA-256(id + ":" + 配对码)。
    code_sha256: String,
    expires_ms: i64,
}

/// 换令牌失败的原因，原样回给客户端。
#[derive(Debug, PartialEq, Eq)]
pub enum ClaimError {
    /// 配对码不对、过期或已经用过。
    Invalid,
    /// 猜错太多次，待用的配对码都作废了。
    TooManyAttempts,
    Store(String),
}

impl std::fmt::Display for ClaimError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid => f.write_str("配对码不对、已过期或已经用过"),
            Self::TooManyAttempts => f.write_str("配对码猜错次数太多，待用的配对码都已作废，请重新生成"),
            Self::Store(e) => write!(f, "读写设备列表失败：{e}"),
        }
    }
}

pub struct Devices {
    store: Store,
    /// 串行化设备列表和配对码的读改写。
    write: tokio::sync::Mutex<()>,
    /// 令牌哈希 → 设备。
    by_token: RwLock<HashMap<String, DeviceRecord>>,
    /// 设备 id → 连着的连接数。
    online: Mutex<HashMap<String, usize>>,
    /// 最近猜错配对码的时间。
    failures: Mutex<VecDeque<i64>>,
    revoked: broadcast::Sender<String>,
}

impl Devices {
    /// 设备列表一开始是空的，启动时调 [`Devices::load`] 从库里读。
    pub fn new(store: Store) -> Self {
        Self {
            store,
            write: tokio::sync::Mutex::new(()),
            by_token: RwLock::new(HashMap::new()),
            online: Mutex::new(HashMap::new()),
            failures: Mutex::new(VecDeque::new()),
            revoked: broadcast::channel(16).0,
        }
    }

    /// 启动时从库里读设备列表。
    pub async fn load(&self) -> Result<(), String> {
        let records = self.records().await?;
        *self.by_token.write().unwrap_or_else(std::sync::PoisonError::into_inner) =
            records.into_iter().map(|r| (r.token_sha256.clone(), r)).collect();
        Ok(())
    }

    /// 这个令牌是哪台设备的。
    pub fn authenticate(&self, token: &str) -> Option<Device> {
        if !token.starts_with(TOKEN_PREFIX) {
            return None;
        }
        let hash = sha256_hex(token.as_bytes());
        self.read_cache().get(&hash).map(device)
    }

    /// 设备现在的名字和权限（连着时可能改过）；已吊销时没有。
    pub fn current(&self, id: &str) -> Option<Device> {
        self.read_cache().values().find(|r| r.id == id).map(device)
    }

    pub fn list(&self) -> Vec<DeviceInfo> {
        let online = self.online.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out: Vec<DeviceInfo> = self
            .read_cache()
            .values()
            .map(|r| info(r, online.get(&r.id).is_some_and(|n| *n > 0)))
            .collect();
        out.sort_by(|a, b| b.paired_at_ms.cmp(&a.paired_at_ms).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// 生成一个配对码。`link_base` 是控制台的地址（比如 `https://ip.example.com`），给了就附上扫码地址和二维码。
    pub async fn create_pairing(&self, name: &str, link_base: Option<&str>, now_ms: i64) -> Result<NewPairing, String> {
        let _w = self.write.lock().await;
        create_pairing(&self.store, name, link_base, now_ms).await
    }

    /// 用配对码换设备令牌。
    pub async fn claim(&self, code: &str, now_ms: i64) -> Result<Claimed, ClaimError> {
        let store = &self.store;
        let _w = self.write.lock().await;
        if self.recent_failures(now_ms) >= MAX_FAILED_CLAIMS {
            return Err(ClaimError::TooManyAttempts);
        }
        let code = code.trim();
        let pairings = live_pairings(store, now_ms).await.map_err(ClaimError::Store)?;
        let records = self.records().await.map_err(ClaimError::Store)?;
        let found = valid_code(code).then(|| {
            pairings
                .iter()
                .find(|p| p.code_sha256 == code_hash(&p.id, code) && !records.iter().any(|r| r.pairing_id == p.id))
        });
        let Some(pairing) = found.flatten().cloned() else {
            let failures = self.fail(now_ms);
            if failures >= MAX_FAILED_CLAIMS {
                tracing::warn!("配对码猜错 {failures} 次，作废所有待用的配对码");
                put(store, PAIRINGS_KEY, &Vec::<Pairing>::new())
                    .await
                    .map_err(ClaimError::Store)?;
                return Err(ClaimError::TooManyAttempts);
            }
            return Err(ClaimError::Invalid);
        };
        let token = new_token();
        let record = DeviceRecord {
            id: format!("dev_{}", uuid::Uuid::now_v7().simple()),
            name: pairing.name.clone(),
            token_sha256: sha256_hex(token.as_bytes()),
            scope: Scope::Daily,
            paired_at_ms: now_ms,
            last_seen_ms: now_ms,
            pairing_id: pairing.id.clone(),
        };
        let mut next = records;
        next.push(record.clone());
        put(store, DEVICES_KEY, &next).await.map_err(ClaimError::Store)?;
        let rest: Vec<Pairing> = pairings.into_iter().filter(|p| p.id != pairing.id).collect();
        // 删不掉也不要紧：设备列表里记着用过的配对码，不会被换第二次。
        let _ = put(store, PAIRINGS_KEY, &rest).await;
        self.write_cache().insert(record.token_sha256.clone(), record.clone());
        tracing::info!(device = %record.id, name = %record.name, "新设备配对成功");
        Ok(Claimed {
            token,
            device: info(&record, false),
        })
    }

    pub async fn rename(&self, id: &str, name: &str) -> Result<Vec<DeviceInfo>, String> {
        let name = clean_name(name)?;
        self.update(id, |r| r.name = name.clone()).await?;
        Ok(self.list())
    }

    /// 改权限，连着的连接下一个请求就按新权限算。
    pub async fn set_scope(&self, id: &str, scope: Scope) -> Result<Vec<DeviceInfo>, String> {
        self.update(id, |r| r.scope = scope).await?;
        tracing::info!(device = %id, ?scope, "设备权限已改");
        Ok(self.list())
    }

    /// 吊销：删掉这台设备，它的连接立即断开。
    pub async fn revoke(&self, id: &str) -> Result<Vec<DeviceInfo>, String> {
        let store = &self.store;
        let _w = self.write.lock().await;
        let records = self.records().await?;
        if !records.iter().any(|r| r.id == id) {
            return Err(format!("没有设备 {id}"));
        }
        let rest: Vec<DeviceRecord> = records.into_iter().filter(|r| r.id != id).collect();
        put(store, DEVICES_KEY, &rest).await?;
        self.write_cache().retain(|_, r| r.id != id);
        let _ = self.revoked.send(id.to_string());
        tracing::info!(device = %id, "设备已吊销");
        Ok(self.list())
    }

    /// 收听吊销：收到的是被吊销的设备 id。
    pub fn subscribe_revoked(&self) -> broadcast::Receiver<String> {
        self.revoked.subscribe()
    }

    /// 记一个连接；返回的守卫丢掉时减掉。顺便更新最后在线时间。
    pub async fn connected(self: &std::sync::Arc<Self>, device: &Device, now_ms: i64) -> OnlineGuard {
        *self
            .online
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(device.id.clone())
            .or_default() += 1;
        let stale = self
            .read_cache()
            .values()
            .any(|r| r.id == device.id && now_ms - r.last_seen_ms >= TOUCH_EVERY_MS);
        if stale && let Err(e) = self.update(&device.id, |r| r.last_seen_ms = now_ms).await {
            tracing::warn!(device = %device.id, "更新最后在线时间失败：{e}");
        }
        OnlineGuard {
            devices: self.clone(),
            id: device.id.clone(),
        }
    }

    async fn update(&self, id: &str, change: impl Fn(&mut DeviceRecord)) -> Result<(), String> {
        let store = &self.store;
        let _w = self.write.lock().await;
        let mut records = self.records().await?;
        let Some(record) = records.iter_mut().find(|r| r.id == id) else {
            return Err(format!("没有设备 {id}"));
        };
        change(record);
        let record = record.clone();
        put(store, DEVICES_KEY, &records).await?;
        self.write_cache().insert(record.token_sha256.clone(), record);
        Ok(())
    }

    async fn records(&self) -> Result<Vec<DeviceRecord>, String> {
        get(&self.store, DEVICES_KEY).await
    }

    fn fail(&self, now_ms: i64) -> usize {
        let mut failures = self.failures.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        failures.push_back(now_ms);
        while failures.front().is_some_and(|t| now_ms - t > PAIRING_TTL_MS) {
            failures.pop_front();
        }
        failures.len()
    }

    fn recent_failures(&self, now_ms: i64) -> usize {
        let failures = self.failures.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        failures.iter().filter(|t| now_ms - **t <= PAIRING_TTL_MS).count()
    }

    fn read_cache(&self) -> std::sync::RwLockReadGuard<'_, HashMap<String, DeviceRecord>> {
        self.by_token.read().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn write_cache(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<String, DeviceRecord>> {
        self.by_token.write().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// 连接计数的守卫。
pub struct OnlineGuard {
    devices: std::sync::Arc<Devices>,
    id: String,
}

impl Drop for OnlineGuard {
    fn drop(&mut self) {
        let mut online = self
            .devices
            .online
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(n) = online.get_mut(&self.id) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                online.remove(&self.id);
            }
        }
    }
}

/// 生成一个配对码并存进库。`ip device pair` 也用它（不经过运行中的服务）。
pub async fn create_pairing(
    store: &Store,
    name: &str,
    link_base: Option<&str>,
    now_ms: i64,
) -> Result<NewPairing, String> {
    let name = clean_name(name)?;
    let mut pairings = live_pairings(store, now_ms).await?;
    let code = loop {
        let n = uuid::Uuid::new_v4().as_u128() % 1_000_000;
        let code = format!("{n:06}");
        // 同时有效的配对码不重复，免得一个码对上两台设备的名字。
        if !pairings.iter().any(|p| p.code_sha256 == code_hash(&p.id, &code)) {
            break code;
        }
    };
    let id = format!("pair_{}", uuid::Uuid::now_v7().simple());
    let expires_ms = now_ms + PAIRING_TTL_MS;
    pairings.push(Pairing {
        id: id.clone(),
        name: name.clone(),
        code_sha256: code_hash(&id, &code),
        expires_ms,
    });
    put(store, PAIRINGS_KEY, &pairings).await?;
    let link = link_base
        .map(|base| base.trim_end_matches('/'))
        .filter(|base| base.starts_with("https://") || base.starts_with("http://"))
        .map(|base| format!("{base}/#/pair/{code}"));
    let qr = link.as_deref().and_then(qr_rows);
    Ok(NewPairing {
        code,
        name,
        expires_ms,
        link,
        qr,
    })
}

/// 二维码，每行一串 `1`（黑）和 `0`（白）。
pub fn qr_rows(text: &str) -> Option<Vec<String>> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    Some(
        colors
            .chunks(width)
            .map(|row| {
                row.iter()
                    .map(|c| if *c == qrcode::Color::Dark { '1' } else { '0' })
                    .collect()
            })
            .collect(),
    )
}

/// 在终端里显示的二维码（深色背景的终端也能扫）。
pub fn qr_terminal(text: &str) -> Option<String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(
        code.render::<Dense1x2>()
            .dark_color(Dense1x2::Light)
            .light_color(Dense1x2::Dark)
            .quiet_zone(true)
            .build(),
    )
}

const TOKEN_PREFIX: &str = "ipd_";

/// 设备令牌：约 244 位随机数。
fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

fn valid_code(code: &str) -> bool {
    code.len() == 6 && code.bytes().all(|b| b.is_ascii_digit())
}

fn code_hash(pairing_id: &str, code: &str) -> String {
    sha256_hex(format!("{pairing_id}:{code}").as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn clean_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("设备名不能为空".into());
    }
    if name.chars().count() > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(format!("设备名最多 {MAX_NAME_CHARS} 个字，不能有换行等控制字符"));
    }
    Ok(name.to_string())
}

fn device(r: &DeviceRecord) -> Device {
    Device {
        id: r.id.clone(),
        name: r.name.clone(),
        scope: r.scope,
    }
}

fn info(r: &DeviceRecord, online: bool) -> DeviceInfo {
    DeviceInfo {
        id: r.id.clone(),
        name: r.name.clone(),
        scope: r.scope,
        paired_at_ms: r.paired_at_ms,
        last_seen_ms: r.last_seen_ms,
        online,
    }
}

async fn live_pairings(store: &Store, now_ms: i64) -> Result<Vec<Pairing>, String> {
    let mut pairings: Vec<Pairing> = get(store, PAIRINGS_KEY).await?;
    pairings.retain(|p| p.expires_ms > now_ms);
    Ok(pairings)
}

async fn get<T: serde::de::DeserializeOwned>(store: &Store, key: &str) -> Result<Vec<T>, String> {
    match store.setting(key).await.map_err(|e| e.to_string())? {
        Some(v) => serde_json::from_value(v).map_err(|e| format!("设置 {key} 格式不对：{e}")),
        None => Ok(Vec::new()),
    }
}

async fn put<T: Serialize>(store: &Store, key: &str, value: &T) -> Result<(), String> {
    let v = serde_json::to_value(value).map_err(|e| e.to_string())?;
    store.put_setting(key, v).await.map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn devices() -> (tempfile::TempDir, std::sync::Arc<Devices>) {
        let dir = tempfile::tempdir().unwrap();
        let store = ip_store::testing::store_at(dir.path().join("ip.db")).await;
        (dir, std::sync::Arc::new(Devices::new(store)))
    }

    #[tokio::test]
    async fn pairs_once_and_authenticates_by_token() {
        let (_dir, d) = devices().await;
        let p = d
            .create_pairing(" 张三的手机 ", Some("https://ip.example.com/"), 1_000)
            .await
            .unwrap();
        assert_eq!(p.code.len(), 6);
        assert_eq!(p.name, "张三的手机");
        assert_eq!(
            p.link.as_deref(),
            Some(format!("https://ip.example.com/#/pair/{}", p.code).as_str())
        );
        let qr = p.qr.unwrap();
        assert!(qr.len() >= 21 && qr.iter().all(|row| row.len() == qr.len()));

        let claimed = d.claim(&p.code, 2_000).await.unwrap();
        assert!(claimed.token.starts_with("ipd_"));
        assert_eq!(claimed.device.name, "张三的手机");
        let device = d.authenticate(&claimed.token).unwrap();
        assert_eq!(device.id, claimed.device.id);
        assert!(d.authenticate("ipd_wrong").is_none());
        // 只能用一次
        assert_eq!(d.claim(&p.code, 3_000).await.unwrap_err(), ClaimError::Invalid);

        // 库里只有哈希，重启后照样认得
        let raw = d.store.setting(DEVICES_KEY).await.unwrap().unwrap().to_string();
        assert!(!raw.contains(&claimed.token));
        let again = Devices::new(d.store.clone());
        again.load().await.unwrap();
        assert_eq!(again.authenticate(&claimed.token).unwrap().name, "张三的手机");
    }

    #[tokio::test]
    async fn codes_expire() {
        let (_dir, d) = devices().await;
        let p = d.create_pairing("平板", None, 0).await.unwrap();
        assert!(p.link.is_none() && p.qr.is_none());
        assert_eq!(
            d.claim(&p.code, PAIRING_TTL_MS + 1).await.unwrap_err(),
            ClaimError::Invalid
        );
    }

    #[tokio::test]
    async fn too_many_wrong_codes_void_all_pending_codes() {
        let (_dir, d) = devices().await;
        let p = d.create_pairing("电脑", None, 0).await.unwrap();
        let wrong = if p.code == "000000" { "000001" } else { "000000" };
        for _ in 0..MAX_FAILED_CLAIMS - 1 {
            assert_eq!(d.claim(wrong, 10).await.unwrap_err(), ClaimError::Invalid);
        }
        assert_eq!(d.claim("abc", 10).await.unwrap_err(), ClaimError::TooManyAttempts);
        // 正确的码也作废了，要重新生成
        assert_eq!(d.claim(&p.code, 20).await.unwrap_err(), ClaimError::TooManyAttempts);
        assert_eq!(
            d.claim(&p.code, 20 + PAIRING_TTL_MS + 1).await.unwrap_err(),
            ClaimError::Invalid
        );
    }

    #[tokio::test]
    async fn the_cli_cannot_resurrect_a_used_code() {
        let (_dir, d) = devices().await;
        let store = d.store.clone();
        let p = create_pairing(&store, "手机", None, 0).await.unwrap();
        let stale: serde_json::Value = store.setting(PAIRINGS_KEY).await.unwrap().unwrap();
        d.claim(&p.code, 1).await.unwrap();
        // 另一个进程（ip device pair）拿着旧的配对码列表写回去
        store.put_setting(PAIRINGS_KEY, stale).await.unwrap();
        assert_eq!(d.claim(&p.code, 2).await.unwrap_err(), ClaimError::Invalid);
    }

    #[tokio::test]
    async fn rename_revoke_and_online() {
        let (_dir, d) = devices().await;
        let p = d.create_pairing("旧名字", None, 0).await.unwrap();
        let c = d.claim(&p.code, 1).await.unwrap();
        let device = d.authenticate(&c.token).unwrap();
        let guard = d.connected(&device, 2 * TOUCH_EVERY_MS).await;
        let list = d.list();
        assert!(list[0].online);
        assert_eq!(list[0].last_seen_ms, 2 * TOUCH_EVERY_MS);
        drop(guard);
        assert!(!d.list()[0].online);

        assert_eq!(d.rename(&device.id, "新名字").await.unwrap()[0].name, "新名字");
        assert_eq!(d.authenticate(&c.token).unwrap().name, "新名字");
        assert!(d.rename(&device.id, "  ").await.is_err());
        assert!(d.rename(&device.id, "a\nb").await.is_err());

        let mut revoked = d.subscribe_revoked();
        assert!(d.revoke(&device.id).await.unwrap().is_empty());
        assert_eq!(revoked.recv().await.unwrap(), device.id);
        assert!(d.authenticate(&c.token).is_none());
        assert!(d.revoke(&device.id).await.is_err());
    }

    #[tokio::test]
    async fn new_devices_are_daily_until_promoted() {
        let (_dir, d) = devices().await;
        let p = d.create_pairing("手机", None, 0).await.unwrap();
        let c = d.claim(&p.code, 1).await.unwrap();
        assert_eq!(c.device.scope, Scope::Daily);
        let id = c.device.id;
        assert_eq!(d.set_scope(&id, Scope::Admin).await.unwrap()[0].scope, Scope::Admin);
        assert_eq!(d.current(&id).unwrap().scope, Scope::Admin);
        assert_eq!(d.authenticate(&c.token).unwrap().scope, Scope::Admin);
        assert!(d.set_scope("dev_none", Scope::Admin).await.is_err());

        // F-8 之前存下的设备没有 scope，读出来是“日常”
        let mut raw = d.store.setting(DEVICES_KEY).await.unwrap().unwrap();
        raw[0].as_object_mut().unwrap().remove("scope");
        d.store.put_setting(DEVICES_KEY, raw).await.unwrap();
        d.load().await.unwrap();
        assert_eq!(d.current(&id).unwrap().scope, Scope::Daily);
    }
}
