//! 定时任务：`ip.yaml` 的 `schedules`，到点按 cron 表达式给岗位或员工发一条消息；
//! 不写 `to` 时按路由规则分派，和人在控制台里选“自动分派”一样。
//!
//! 每次运行开一段新对话，消息署名“定时任务·<id>”。服务停机期间错过的时刻不补跑；
//! 上次运行的时间和结果存在 `settings` 表的 `schedule.last.<id>` 里，控制台任务页能看到。

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use ip_core::{AgentId, Author, OrgSpec, ThreadId, now_ms};
use ip_router::Target;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::App;
use crate::protocol::SendResult;

/// 一条定时任务的配置。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ScheduleSpec {
    pub id: String,
    #[serde(default)]
    pub description: String,
    /// 五段 cron：分 时 日 月 周（0 和 7 都是周日）。每段可写 `*`、`a`、`a-b`、逗号列表，
    /// 后面可加 `/n` 表示步长（如每 15 分钟：分钟段写星号加 `/15`）。
    pub cron: String,
    /// 按哪个时区解读 cron，写成相对 UTC 的偏移，如 `+08:00`。默认 `+00:00`。
    #[serde(default = "utc")]
    pub utc_offset: String,
    /// 发给哪个岗位（可指定员工、可要求升级模型）；省略时按路由规则分派。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub to: Option<Target>,
    /// 到点发出的消息。
    pub text: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn utc() -> String {
    "+00:00".into()
}

fn yes() -> bool {
    true
}

/// 上次运行的情况。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LastRun {
    #[ts(type = "number")]
    pub at_ms: i64,
    pub ok: bool,
    /// 成功时是新对话的 id，失败时是原因。
    pub detail: String,
    /// 成功时是新对话。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_id: Option<ThreadId>,
}

/// 控制台看到的一条定时任务。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct ScheduleView {
    pub spec: ScheduleSpec,
    /// 下次运行的时间；停用或 cron 永远不会命中时为空。
    #[ts(type = "number | null")]
    pub next_run_ms: Option<i64>,
    pub last_run: Option<LastRun>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ScheduleError {
    #[error("定时任务 id `{0}` 只能包含字母、数字、`-` 和 `_`")]
    BadId(String),
    #[error("定时任务 id `{0}` 重复")]
    Duplicate(String),
    #[error("定时任务 `{0}` 的 cron 有误：{1}")]
    Cron(String, String),
    #[error("定时任务 `{0}` 的 utc_offset `{1}` 应写成 +08:00 这样的格式")]
    Offset(String, String),
    #[error("定时任务 `{0}` 的消息不能为空")]
    EmptyText(String),
    #[error("定时任务 `{0}` 发给的岗位 `{1}` 不存在")]
    UnknownRole(String, String),
    #[error("定时任务 `{0}` 指定的员工 `{1}` 不在岗位 `{2}` 里")]
    UnknownAgent(String, String, String),
}

/// 编译好的定时任务表，挂在 [`App`] 上。
#[derive(Default)]
pub struct Schedules {
    items: Vec<(ScheduleSpec, Cron, i64)>,
    last: Mutex<HashMap<String, LastRun>>,
}

impl Schedules {
    /// 校验配置。`agents` 是 (员工 id, 岗位 id)。
    pub fn new(specs: Vec<ScheduleSpec>, org: &OrgSpec, agents: &[(String, String)]) -> Result<Self, ScheduleError> {
        let mut items = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for spec in specs {
            let id = spec.id.clone();
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(ScheduleError::BadId(id));
            }
            if !seen.insert(id.clone()) {
                return Err(ScheduleError::Duplicate(id));
            }
            let cron = Cron::parse(&spec.cron).map_err(|e| ScheduleError::Cron(id.clone(), e))?;
            let offset = parse_offset(&spec.utc_offset)
                .ok_or_else(|| ScheduleError::Offset(id.clone(), spec.utc_offset.clone()))?;
            if spec.text.trim().is_empty() {
                return Err(ScheduleError::EmptyText(id));
            }
            if let Some(to) = &spec.to {
                if org.role(&to.role).is_none() {
                    return Err(ScheduleError::UnknownRole(id, to.role.to_string()));
                }
                if let Some(agent) = &to.agent
                    && !agents.iter().any(|(a, r)| a == agent.as_str() && r == to.role.as_str())
                {
                    return Err(ScheduleError::UnknownAgent(id, agent.to_string(), to.role.to_string()));
                }
            }
            items.push((spec, cron, offset));
        }
        Ok(Self {
            items,
            last: Mutex::default(),
        })
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    fn get(&self, id: &str) -> Option<&ScheduleSpec> {
        self.items.iter().map(|(s, ..)| s).find(|s| s.id == id)
    }

    /// `after_ms` 之后（不含）第一次运行的时间。
    fn next(&self, i: usize, after_ms: i64) -> Option<i64> {
        let (spec, cron, offset) = &self.items[i];
        if !spec.enabled {
            return None;
        }
        cron.next_after(after_ms, *offset)
    }

    pub fn views(&self) -> Vec<ScheduleView> {
        let now = now_ms();
        let last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        self.items
            .iter()
            .enumerate()
            .map(|(i, (spec, ..))| ScheduleView {
                spec: spec.clone(),
                next_run_ms: self.next(i, now),
                last_run: last.get(&spec.id).cloned(),
            })
            .collect()
    }
}

fn last_key(id: &str) -> String {
    format!("schedule.last.{id}")
}

/// 读回上次运行的记录（服务重启后控制台仍能看到）。
pub(crate) async fn load_last(app: &App) {
    for (spec, ..) in &app.schedules.items {
        if let Ok(Some(v)) = app.runtime.store().setting(&last_key(&spec.id)).await
            && let Ok(run) = serde_json::from_value::<LastRun>(v)
        {
            app.schedules
                .last
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .insert(spec.id.clone(), run);
        }
    }
}

/// 立即运行一次（到点运行和控制台“立即运行”都走这里），记下结果。
pub async fn run(app: &App, id: &str) -> Result<SendResult, String> {
    let spec = app
        .schedules
        .get(id)
        .cloned()
        .ok_or_else(|| format!("没有定时任务 {id}"))?;
    let result = send(app, &spec).await;
    let run = LastRun {
        at_ms: now_ms(),
        ok: result.is_ok(),
        detail: match &result {
            Ok(r) => match &r.agent_id {
                Some(a) => format!("已交给 {a}"),
                None => "路由拿不准，已发审批卡片".into(),
            },
            Err(e) => e.clone(),
        },
        thread_id: result.as_ref().ok().map(|r| r.message.thread_id.clone()),
    };
    match &result {
        Ok(_) => tracing::info!(schedule = id, detail = %run.detail, "定时任务已运行"),
        Err(e) => tracing::warn!(schedule = id, error = %e, "定时任务运行失败"),
    }
    if let Ok(v) = serde_json::to_value(&run)
        && let Err(e) = app.runtime.store().put_setting(last_key(id), v).await
    {
        tracing::warn!(schedule = id, error = %e, "定时任务的运行记录没存上");
    }
    app.schedules
        .last
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(id.to_string(), run);
    result
}

async fn send(app: &App, spec: &ScheduleSpec) -> Result<SendResult, String> {
    let author = format!("定时任务·{}", spec.id);
    let Some(to) = &spec.to else {
        return crate::work::send(app, None, None, spec.text.clone(), author).await;
    };
    let rt = &app.runtime;
    let agent: AgentId = match &to.agent {
        Some(a) => a.clone(),
        None => rt
            .least_busy(&to.role)
            .ok_or_else(|| format!("岗位 {} 没有员工", to.role))?,
    };
    let message = rt
        .send_with(
            &agent,
            ThreadId::new(),
            Author::human(author),
            spec.text.clone(),
            to.escalate,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(SendResult {
        message,
        agent_id: Some(agent),
        decision: None,
        approval_id: None,
    })
}

/// 后台调度：到点运行，服务关闭时退出。没有定时任务时不启动。
pub(crate) fn spawn(app: App) {
    if app.schedules.is_empty() {
        return;
    }
    tokio::spawn(async move {
        let mut closing = app.closing.subscribe();
        let n = app.schedules.items.len();
        let start = now_ms();
        let mut next: Vec<Option<i64>> = (0..n).map(|i| app.schedules.next(i, start)).collect();
        loop {
            let now = now_ms();
            for (i, slot) in next.iter_mut().enumerate() {
                if slot.is_some_and(|t| t <= now) {
                    let id = app.schedules.items[i].0.id.clone();
                    let _ = run(&app, &id).await;
                    *slot = app.schedules.next(i, now);
                }
            }
            // 最多睡一分钟就重新看一次时钟，系统时间被调整时也不会错过太久。
            let wake = next.iter().flatten().min().copied();
            let wait = wake.map_or(60_000, |t| (t - now_ms()).clamp(0, 60_000));
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(wait.unsigned_abs())) => {}
                _ = closing.wait_for(|c| *c) => return,
            }
        }
    });
}

// ---------- cron ----------

/// 解析好的五段 cron。每段是允许值的位图。
#[derive(Clone, Debug, PartialEq)]
pub struct Cron {
    minute: u64,
    hour: u64,
    dom: u64,
    month: u64,
    dow: u64,
    /// 日和周都限定时，两者满足其一即可（标准 cron 的规则）。
    dom_any: bool,
    dow_any: bool,
}

impl Cron {
    pub fn parse(text: &str) -> Result<Self, String> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!("要五段（分 时 日 月 周），这里有 {} 段", fields.len()));
        }
        let mut dow = field(fields[4], 0, 7, "周")?;
        if dow & (1 << 7) != 0 {
            dow |= 1; // 7 也是周日
        }
        Ok(Self {
            minute: field(fields[0], 0, 59, "分")?,
            hour: field(fields[1], 0, 23, "时")?,
            dom: field(fields[2], 1, 31, "日")?,
            month: field(fields[3], 1, 12, "月")?,
            dow: dow & 0x7f,
            dom_any: fields[2] == "*",
            dow_any: fields[4] == "*",
        })
    }

    fn day_matches(&self, d: i64, wd: i64) -> bool {
        let dom = self.dom & (1 << d) != 0;
        let dow = self.dow & (1 << wd) != 0;
        match (self.dom_any, self.dow_any) {
            (false, false) => dom || dow,
            _ => dom && dow,
        }
    }

    /// `after_ms` 之后（不含）第一个命中的整分钟，`offset_min` 是时区相对 UTC 的分钟数。
    /// 四年内都不命中（比如 2 月 30 日）时返回 None。
    pub fn next_after(&self, after_ms: i64, offset_min: i64) -> Option<i64> {
        let mut m = after_ms.div_euclid(60_000) + 1 + offset_min; // 本地时间的分钟序号
        let limit = m + 4 * 366 * 1440;
        while m < limit {
            let day = m.div_euclid(1440);
            let (y, mo, d) = civil_from_days(day);
            if self.month & (1 << mo) == 0 {
                let (ny, nm) = if mo == 12 { (y + 1, 1) } else { (y, mo + 1) };
                m = days_from_civil(ny, nm, 1) * 1440;
                continue;
            }
            let wd = (day + 4).rem_euclid(7); // 1970-01-01 是周四
            if !self.day_matches(d, wd) {
                m = (day + 1) * 1440;
                continue;
            }
            let minute_of_day = m.rem_euclid(1440);
            if self.hour & (1 << (minute_of_day / 60)) == 0 {
                m = day * 1440 + (minute_of_day / 60 + 1) * 60;
                continue;
            }
            if self.minute & (1 << (minute_of_day % 60)) == 0 {
                m += 1;
                continue;
            }
            return Some((m - offset_min) * 60_000);
        }
        None
    }
}

fn field(text: &str, lo: u32, hi: u32, name: &str) -> Result<u64, String> {
    let bad = || format!("{name}字段 `{text}` 无法识别");
    let mut bits = 0u64;
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(bad)?),
            None => (part, 1),
        };
        let (a, b) = if range == "*" {
            (lo, hi)
        } else if let Some((a, b)) = range.split_once('-') {
            (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?)
        } else {
            let v: u32 = range.parse().map_err(|_| bad())?;
            // `5/15` 表示从 5 开始每 15 个。
            if part.contains('/') { (v, hi) } else { (v, v) }
        };
        if a < lo || b > hi || a > b {
            return Err(format!("{name}字段 `{text}` 超出范围 {lo}-{hi}"));
        }
        let mut v = a;
        while v <= b {
            bits |= 1 << v;
            v += step;
        }
    }
    Ok(bits)
}

/// `+08:00` / `-05:30` / `Z` → 分钟。
pub(crate) fn parse_offset(text: &str) -> Option<i64> {
    let t = text.trim();
    if t == "Z" || t == "UTC" {
        return Some(0);
    }
    let sign = match t.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let (h, m) = t[1..].split_once(':')?;
    let (h, m): (i64, i64) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 14 && m < 60 && t.len() == 6).then_some(sign * (h * 60 + m))
}

/// Howard Hinnant 的 days_from_civil。
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Howard Hinnant 的 civil_from_days。
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// UTC 的年月日时分 → 毫秒。
    fn at(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
        (days_from_civil(y, mo, d) * 1440 + h * 60 + mi) * 60_000
    }

    #[test]
    fn calendar_round_trips() {
        for day in [-800_000, -1, 0, 11_016, 20_402, 60_000] {
            let (y, m, d) = civil_from_days(day);
            assert_eq!(days_from_civil(y, m, d), day);
        }
        assert_eq!(civil_from_days(20_402), (2025, 11, 10));
    }

    #[test]
    fn next_runs() {
        let c = |s: &str| Cron::parse(s).unwrap();
        let now = at(2026, 11, 10, 9, 30); // 周二 09:30 UTC
        // 每天 09:00 北京时间 = 01:00 UTC，今天已过，下次是明天。
        assert_eq!(c("0 9 * * *").next_after(now, 8 * 60), Some(at(2026, 11, 11, 1, 0)));
        // 每周一 09:00 北京时间。
        assert_eq!(c("0 9 * * 1").next_after(now, 8 * 60), Some(at(2026, 11, 16, 1, 0)));
        // 每 15 分钟；整点时刻本身不算“之后”。
        assert_eq!(c("*/15 * * * *").next_after(now, 0), Some(at(2026, 11, 10, 9, 45)));
        // 工作日 18:30，跨月。
        assert_eq!(
            c("30 18 * * 1-5").next_after(at(2026, 11, 30, 19, 0), 0),
            Some(at(2026, 12, 1, 18, 30))
        );
        // 每月 1 日或周日（日和周都限定时取并集）。
        assert_eq!(c("0 0 1 * 0").next_after(now, 0), Some(at(2026, 11, 15, 0, 0)));
        // 闰年 2 月 29 日；7 也是周日。
        assert_eq!(c("0 0 29 2 *").next_after(now, 0), Some(at(2028, 2, 29, 0, 0)));
        assert_eq!(c("0 12 * * 7"), c("0 12 * * 0"));
        // 负偏移：纽约 08:00（-05:00）= 13:00 UTC。
        assert_eq!(c("0 8 * * *").next_after(now, -300), Some(at(2026, 11, 10, 13, 0)));
        // 永远不会命中。
        assert_eq!(c("0 0 30 2 *").next_after(now, 0), None);
    }

    #[test]
    fn rejects_bad_cron_and_offsets() {
        for bad in [
            "",
            "* * * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "a * * * *",
            "5-1 * * * *",
        ] {
            assert!(Cron::parse(bad).is_err(), "{bad}");
        }
        assert_eq!(parse_offset("+08:00"), Some(480));
        assert_eq!(parse_offset("-05:30"), Some(-330));
        assert_eq!(parse_offset("Z"), Some(0));
        for bad in ["08:00", "+8", "+08:60", "+15:00", "+08:00:00"] {
            assert_eq!(parse_offset(bad), None, "{bad}");
        }
    }
}
