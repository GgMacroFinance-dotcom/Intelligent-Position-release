//! 上下文里按岗位存的两层内容：决策者笔记和岗位记忆摘要（`docs/ip/context-memory-design.md` 5.1）。
//!
//! 两样都存在设置表里，一个岗位各一条，每次请求读出来放进系统提示词。决策者笔记由人写（`ip context`）；
//! 岗位记忆摘要可以由人写，开了记忆后也会由后台从对话里提炼（[`extract`]）再合并（[`consolidate`]）出来。

pub mod consolidate;
pub mod extract;
pub mod redact;
pub mod search;

use ip_core::{RoleId, ThreadId};
use ip_store::{Store, StoreError};
use serde::{Deserialize, Serialize};

/// 岗位记忆的开关和大小上限。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfig {
    /// 是否把岗位记忆摘要放进系统提示词。默认关。决策者笔记不受这个开关影响。
    #[serde(default)]
    pub enabled: bool,
    /// 只对这些岗位生效；省略表示所有岗位。
    #[serde(default)]
    pub roles: Vec<RoleId>,
    /// 决策者笔记最多放进提示词多少字，超出的截掉。
    #[serde(default = "default_notes_max_chars")]
    pub notes_max_chars: usize,
    /// 岗位记忆摘要最多放进提示词多少字，超出的截掉。
    #[serde(default = "default_summary_max_chars")]
    pub summary_max_chars: usize,
    /// 后台每隔多少分钟找一次可以提炼的对话。
    #[serde(default = "default_interval_minutes")]
    pub interval_minutes: u64,
    /// 对话闲置满这么多小时才提炼，免得提炼还在进行的对话。
    #[serde(default = "default_min_idle_hours")]
    pub min_idle_hours: u64,
    /// 最后一条消息早于这么多天的对话不提炼。
    #[serde(default = "default_max_age_days")]
    pub max_age_days: u64,
    /// 每次最多提炼几份（一个对话在一个岗位下算一份，一份调一次模型）。
    #[serde(default = "default_max_threads_per_pass")]
    pub max_threads_per_pass: usize,
    /// 交给模型提炼时，每条工具结果最多保留多少字。
    #[serde(default = "default_tool_result_chars")]
    pub tool_result_chars: usize,
    /// 交给模型提炼的对话记录最多多少字，超出时只留最后这么多。
    #[serde(default = "default_input_max_chars")]
    pub input_max_chars: usize,
    /// 岗位设了每日预算时，当天剩余不到这个百分比就不提炼，把钱留给正事。
    #[serde(default = "default_min_budget_remaining_percent")]
    pub min_budget_remaining_percent: f64,
    /// 提炼用的模型（`<端点>/<模型>`）；省略时用岗位自己的模型。
    #[serde(default)]
    pub extract_model: Option<String>,
    /// 合并用的模型（`<端点>/<模型>`）；省略时用岗位自己的模型。
    #[serde(default)]
    pub consolidation_model: Option<String>,
    /// 合并时，这么多天没被用到（没被用过的看生成时间）的对话摘要不再选。
    #[serde(default = "default_max_unused_days")]
    pub max_unused_days: u64,
    /// 合并时最多选多少份对话摘要。
    #[serde(default = "default_max_summaries")]
    pub max_summaries: usize,
    /// 外部渠道（如 Slack）进来的对话要不要进记忆，从这些对话派出去的执行对话跟着它走。默认不进。
    #[serde(default)]
    pub include_channels: bool,
}

fn default_notes_max_chars() -> usize {
    4_000
}

fn default_summary_max_chars() -> usize {
    10_000
}

fn default_interval_minutes() -> u64 {
    60
}

fn default_min_idle_hours() -> u64 {
    6
}

fn default_max_age_days() -> u64 {
    10
}

fn default_max_threads_per_pass() -> usize {
    2
}

fn default_tool_result_chars() -> usize {
    500
}

fn default_input_max_chars() -> usize {
    40_000
}

fn default_min_budget_remaining_percent() -> f64 {
    25.0
}

fn default_max_unused_days() -> u64 {
    30
}

fn default_max_summaries() -> usize {
    256
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            roles: Vec::new(),
            notes_max_chars: default_notes_max_chars(),
            summary_max_chars: default_summary_max_chars(),
            interval_minutes: default_interval_minutes(),
            min_idle_hours: default_min_idle_hours(),
            max_age_days: default_max_age_days(),
            max_threads_per_pass: default_max_threads_per_pass(),
            tool_result_chars: default_tool_result_chars(),
            input_max_chars: default_input_max_chars(),
            min_budget_remaining_percent: default_min_budget_remaining_percent(),
            extract_model: None,
            consolidation_model: None,
            max_unused_days: default_max_unused_days(),
            max_summaries: default_max_summaries(),
            include_channels: false,
        }
    }
}

impl MemoryConfig {
    /// 这个岗位的记忆摘要要不要放进提示词。
    pub fn enabled_for(&self, role: &RoleId) -> bool {
        self.enabled && (self.roles.is_empty() || self.roles.contains(role))
    }
}

/// 哪一层。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    /// 决策者直接写给岗位的话。
    Notes,
    /// 岗位记忆摘要。
    Summary,
}

impl Layer {
    fn key(self, role: &RoleId) -> String {
        match self {
            Layer::Notes => format!("context.notes.{role}"),
            Layer::Summary => format!("memory.summary.{role}"),
        }
    }
}

/// 设置表里存的一条。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub text: String,
    pub updated_ms: i64,
    /// 只有合并出来的岗位记忆摘要有：这一版是由哪些对话摘要合并成的。
    /// 下次合并拿它判断有没有变化；人手写的摘要没有这一项。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<Input>,
}

/// 合并时用到的一份对话摘要：哪个对话、当时是哪一版。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Input {
    pub thread_id: ThreadId,
    pub generated_ms: i64,
}

/// 读一层的内容；没写过或已清空时返回 `None`。
pub async fn get(store: &Store, layer: Layer, role: &RoleId) -> Result<Option<Entry>, StoreError> {
    let Some(value) = store.setting(&layer.key(role)).await? else {
        return Ok(None);
    };
    let entry: Entry = serde_json::from_value(value)?;
    Ok((!entry.text.trim().is_empty()).then_some(entry))
}

/// 整体覆盖一层的内容。空文本等于清空。每次写都记一条事件，历史版本可追溯。
pub async fn put(store: &Store, layer: Layer, role: &RoleId, text: &str, now_ms: i64) -> Result<(), StoreError> {
    put_entry(store, layer, role, text, now_ms, Vec::new()).await
}

async fn put_entry(
    store: &Store,
    layer: Layer,
    role: &RoleId,
    text: &str,
    now_ms: i64,
    inputs: Vec<Input>,
) -> Result<(), StoreError> {
    let entry = Entry {
        text: text.trim().to_string(),
        updated_ms: now_ms,
        inputs,
    };
    store.put_setting(layer.key(role), serde_json::to_value(entry)?).await?;
    Ok(())
}

/// 读岗位记忆摘要，连同它是由哪些对话摘要合并成的。和 [`get`] 不同，文本清空了也返回。
async fn summary_entry(store: &Store, role: &RoleId) -> Result<Option<Entry>, StoreError> {
    let Some(value) = store.setting(&Layer::Summary.key(role)).await? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_value(value)?))
}

/// 一次请求里要加进系统提示词的两层内容，已按上限截好。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Extras {
    pub notes: Option<String>,
    pub summary: Option<String>,
}

/// 读出这个岗位的决策者笔记和（开了记忆时的）岗位记忆摘要。
pub async fn load(store: &Store, config: &MemoryConfig, role: &RoleId) -> Result<Extras, StoreError> {
    let notes = get(store, Layer::Notes, role)
        .await?
        .map(|e| clip(&e.text, config.notes_max_chars, role, "决策者笔记"));
    let summary = if config.enabled_for(role) {
        get(store, Layer::Summary, role)
            .await?
            .map(|e| clip(&e.text, config.summary_max_chars, role, "岗位记忆摘要"))
    } else {
        None
    };
    Ok(Extras { notes, summary })
}

/// 按字数截断；截了就在日志里说，免得人以为整段都生效了。
fn clip(text: &str, max_chars: usize, role: &RoleId, what: &str) -> String {
    match text.char_indices().nth(max_chars) {
        None => text.to_string(),
        Some((end, _)) => {
            tracing::warn!(role = %role, max_chars, "{what}超过上限，只用了前面的部分");
            text[..end].to_string()
        }
    }
}

/// 顺着派活关系最多往上查这么多层。正常的委派深度远小于它（`max_delegation_depth` 默认 3）。
const MAX_ORIGIN_HOPS: usize = 32;

/// 对话是不是从外部渠道来的：它自己绑定了渠道会话，或者顺着派活关系（执行对话所属任务的
/// `origin_thread_id`）往上，某一层绑定了。委派出去的执行对话没有自己的绑定，但任务说明和
/// 执行员工的回复都可能带着渠道里的内容，要和根对话一样处理（#67）。
///
/// 来源链成环或长得不正常时说不清来源，按来自渠道处理：宁可少记，不让外部内容混进岗位记忆。
pub(crate) async fn from_channel(store: &Store, thread: &ThreadId) -> Result<bool, StoreError> {
    let mut current = thread.clone();
    let mut visited = std::collections::HashSet::new();
    for _ in 0..MAX_ORIGIN_HOPS {
        if store.binding_by_thread(&current).await?.is_some() {
            return Ok(true);
        }
        let Some(origin) = store.task_by_thread(&current).await?.and_then(|t| t.origin_thread_id) else {
            return Ok(false);
        };
        visited.insert(current);
        if visited.contains(&origin) {
            tracing::warn!(thread = %thread, "对话的派活来源链成环，按来自外部渠道处理");
            return Ok(true);
        }
        current = origin;
    }
    tracing::warn!(thread = %thread, hops = MAX_ORIGIN_HOPS, "对话的派活来源链太长，按来自外部渠道处理");
    Ok(true)
}

/// 关掉了记忆的对话存在设置表的这一条里（一个列表）。
const THREADS_OFF_KEY: &str = "memory.threads_off";

/// 关掉了记忆的对话：不提炼，也不能在里面记更正。
pub async fn threads_off(store: &Store) -> Result<Vec<ThreadId>, StoreError> {
    match store.setting(THREADS_OFF_KEY).await? {
        Some(value) => Ok(serde_json::from_value(value)?),
        None => Ok(Vec::new()),
    }
}

/// 关掉或重新打开一个对话的记忆。关掉时把这个对话在各岗位下已经提炼出的内容一并抹掉，
/// 下次合并时岗位记忆摘要里来自它的内容会被拿掉。重新打开后，等对话有新消息才会再提炼。
pub async fn set_thread_off(store: &Store, thread: &ThreadId, off: bool) -> Result<(), StoreError> {
    let mut threads = threads_off(store).await?;
    threads.retain(|t| t != thread);
    if off {
        threads.push(thread.clone());
    }
    store
        .put_setting(THREADS_OFF_KEY.to_string(), serde_json::to_value(&threads)?)
        .await?;
    if off {
        store.forget_thread_memories(None, Some(thread)).await?;
    }
    Ok(())
}

/// 清空一个岗位的记忆：岗位记忆摘要、各对话的记忆内容、人要求的更正都清掉。决策者笔记不动。
/// 之后的对话照常提炼，记忆从头积累。
pub async fn clear_role(store: &Store, role: &RoleId, now_ms: i64) -> Result<(), StoreError> {
    store.forget_thread_memories(Some(role), None).await?;
    store.delete_memory_notes(role, None).await?;
    put(store, Layer::Summary, role, "", now_ms).await
}

/// 人直接改岗位记忆摘要。合并来源的记录留着：下次合并在人改过的这一版上接着改，
/// 而不是把所有对话摘要当成新的再合并一遍。空文本等于清空，来源记录也一并清掉。
pub async fn edit_summary(store: &Store, role: &RoleId, text: &str, now_ms: i64) -> Result<(), StoreError> {
    let inputs = if text.trim().is_empty() {
        Vec::new()
    } else {
        summary_entry(store, role).await?.map(|e| e.inputs).unwrap_or_default()
    };
    put_entry(store, Layer::Summary, role, text, now_ms, inputs).await
}

/// 岗位记忆摘要这一版是由多少份对话摘要合并成的；人手写的或还没有时是 0。
pub async fn summary_inputs(store: &Store, role: &RoleId) -> Result<usize, StoreError> {
    Ok(summary_entry(store, role).await?.map_or(0, |e| e.inputs.len()))
}

/// 毫秒时间戳转成 UTC 日期 `YYYY-MM-DD`。
pub(crate) fn date(ms: i64) -> String {
    // 公历换算，见 Howard Hinnant 的 civil_from_days 算法。
    let days = ms.div_euclid(86_400_000) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}
