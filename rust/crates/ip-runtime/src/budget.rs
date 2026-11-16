//! 岗位每日模型费用：每次调用模型后按价格记账，岗位设了 `budget_per_day_usd` 时，用满就不再调用。
//!
//! 按 UTC 自然日计。账本存在 `settings` 表的 `spend.<日期>` 里，服务重启后当天已花的钱不会清零。
//! 没登记价格的模型不计费（配置校验会要求设了预算的岗位给模型配价格）。
//! 服务端语义分派调用模型的费用记在 [`ROUTING`] 名下，不占任何岗位的预算。
//!
//! 同一个账本还按岗位累计当天的 token 用量（输入、其中命中前缀缓存的、输出），存在 `tokens.<日期>` 里，
//! 用来算提示词缓存命中率。它每轮结束时记一次，不参与预算判断；没登记价格的模型也照记。

use std::collections::HashMap;

use ip_core::{RoleId, now_ms};
use ip_models::Usage;
use tokio::sync::Mutex;

use crate::RuntimeError;

/// 账本里记语义分派费用的条目名。岗位 id 不会以 `@` 开头。
pub(crate) const ROUTING: &str = "@routing";

#[derive(Default)]
struct Day {
    /// 自 1970-01-01 起的天数；`None` 表示还没加载。
    day: Option<i64>,
    spent: HashMap<RoleId, f64>,
    tokens: HashMap<RoleId, Usage>,
}

/// [`Ledger::today`] 的结果。
pub(crate) struct Today {
    /// `YYYY-MM-DD`（UTC）。
    pub date: String,
    pub spent: HashMap<RoleId, f64>,
    pub tokens: HashMap<RoleId, Usage>,
}

#[derive(Default)]
pub(crate) struct Ledger {
    inner: Mutex<Day>,
}

impl Ledger {
    /// 这个岗位今天已花的美元。
    pub async fn spent(&self, store: &ip_store::Store, role: &RoleId) -> Result<f64, RuntimeError> {
        let mut day = self.inner.lock().await;
        load_today(&mut day, store).await?;
        Ok(day.spent.get(role).copied().unwrap_or(0.0))
    }

    /// 今天的日期（`YYYY-MM-DD`，UTC）、各条目已花的美元和已用的 token。
    pub async fn today(&self, store: &ip_store::Store) -> Result<Today, RuntimeError> {
        let mut day = self.inner.lock().await;
        load_today(&mut day, store).await?;
        Ok(Today {
            date: date(day.day.unwrap_or_default()),
            spent: day.spent.clone(),
            tokens: day.tokens.clone(),
        })
    }

    /// 把一轮（或一次语义分派）的 token 用量累计到这个条目名下，写回存储。全是 0 时不写。
    pub async fn record_tokens(
        &self,
        store: &ip_store::Store,
        role: &RoleId,
        usage: Usage,
    ) -> Result<(), RuntimeError> {
        if usage == Usage::default() {
            return Ok(());
        }
        let mut day = self.inner.lock().await;
        load_today(&mut day, store).await?;
        let total = day.tokens.entry(role.clone()).or_default();
        total.prompt_tokens = total.prompt_tokens.saturating_add(usage.prompt_tokens);
        total.completion_tokens = total.completion_tokens.saturating_add(usage.completion_tokens);
        total.cached_tokens = total.cached_tokens.saturating_add(usage.cached_tokens);
        total.cache_write_tokens = total.cache_write_tokens.saturating_add(usage.cache_write_tokens);
        let value = serde_json::to_value(
            day.tokens
                .iter()
                .map(|(r, u)| (r.to_string(), *u))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .map_err(|e| RuntimeError::Invalid(e.to_string()))?;
        store
            .put_setting(tokens_key(day.day.unwrap_or_default()), value)
            .await?;
        Ok(())
    }

    /// 记一笔，写回存储。
    pub async fn charge(&self, store: &ip_store::Store, role: &RoleId, usd: f64) -> Result<(), RuntimeError> {
        if usd <= 0.0 {
            return Ok(());
        }
        let mut day = self.inner.lock().await;
        load_today(&mut day, store).await?;
        *day.spent.entry(role.clone()).or_default() += usd;
        let value = serde_json::to_value(
            day.spent
                .iter()
                .map(|(r, v)| (r.to_string(), *v))
                .collect::<std::collections::BTreeMap<_, _>>(),
        )
        .map_err(|e| RuntimeError::Invalid(e.to_string()))?;
        store.put_setting(key(day.day.unwrap_or_default()), value).await?;
        Ok(())
    }
}

async fn load_today(day: &mut Day, store: &ip_store::Store) -> Result<(), RuntimeError> {
    let today = now_ms().div_euclid(86_400_000);
    if day.day == Some(today) {
        return Ok(());
    }
    let saved: HashMap<String, f64> = match store.setting(&key(today)).await? {
        Some(v) => serde_json::from_value(v).unwrap_or_default(),
        None => HashMap::new(),
    };
    let tokens: HashMap<String, Usage> = match store.setting(&tokens_key(today)).await? {
        Some(v) => serde_json::from_value(v).unwrap_or_default(),
        None => HashMap::new(),
    };
    day.day = Some(today);
    day.spent = saved.into_iter().map(|(r, v)| (RoleId::from(r.as_str()), v)).collect();
    day.tokens = tokens.into_iter().map(|(r, u)| (RoleId::from(r.as_str()), u)).collect();
    Ok(())
}

fn key(day: i64) -> String {
    format!("spend.{}", date(day))
}

fn tokens_key(day: i64) -> String {
    format!("tokens.{}", date(day))
}

/// 天数 → `YYYY-MM-DD`（公历，UTC）。
pub(crate) fn date(days: i64) -> String {
    // Howard Hinnant 的 civil_from_days。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::date(0), "1970-01-01");
        assert_eq!(super::date(20_402), "2025-11-10");
        assert_eq!(super::date(11_016), "2000-02-29");
    }
}
