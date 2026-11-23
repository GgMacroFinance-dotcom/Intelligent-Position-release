//! 接团队上下文系统（Team Context Management System，简称 TCMS，`docs/ip/team-context.md`）。
//!
//! TCMS 是一个独立的 MCP 服务（`tcms serve`），每个岗位用自己的令牌连上去，令牌决定这个岗位能看哪些范围。
//! I.P. 这边只做三件事：
//!
//! 1. 每轮开头调一次 `context_get`，把固定前缀（规则和常驻条目）放进系统提示词，紧跟在组织规则后面；
//! 2. 按任务挑的那部分放在对话开头。同一个对话第一次取到以后就不再换（进程里记着），对话前面的内容不变，
//!    提示词缓存能命中；进程重启后按对话里的第一条消息重新取；
//! 3. 员工用 TCMS 的 `context_search`、`context_read`、`context_propose`，它们就是 `mcp_servers` 里那一项的
//!    MCP 工具。`context_get`、`context_changes` 由 harness 调，不给模型。
//!
//! TCMS 连不上或出错时，这一轮不带团队上下文，照常进行，下一轮再试。

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use ip_core::{Agent, Author, Body, Message, RoleId, ThreadId};
use ip_mcp::McpHub;
use serde::Deserialize;

/// `ip.yaml` 的 `team_context`。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamContextConfig {
    /// 整个包（固定前缀加按任务挑的部分）的 token 预算，按字符数估。默认 4000。
    #[serde(default = "default_budget")]
    pub budget_tokens: u32,
    /// 接上的岗位：岗位 id → 这个岗位怎么连。
    #[serde(default)]
    pub roles: BTreeMap<RoleId, TeamRole>,
}

impl Default for TeamContextConfig {
    fn default() -> Self {
        Self {
            budget_tokens: default_budget(),
            roles: BTreeMap::new(),
        }
    }
}

fn default_budget() -> u32 {
    4000
}

/// TCMS 允许的预算范围（`context_get` 的 `budget_tokens`）。
pub const BUDGET_RANGE: std::ops::RangeInclusive<u32> = 100..=200_000;

/// 一个岗位怎么连 TCMS。
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamRole {
    /// `mcp_servers` 里接 TCMS 的那一项，带的是这个岗位的令牌（`tcms token add --role ...` 发的），
    /// 只能给这一个岗位用。
    pub server: String,
    /// 在 TCMS 里的岗位名，要和令牌一致；省略时用 I.P. 的岗位 id。
    #[serde(default)]
    pub role: Option<String>,
    /// 在 TCMS 里的团队，可以不写（TCMS 按 `teams.yaml` 查）；写了要和令牌一致。
    #[serde(default)]
    pub team: Option<String>,
}

/// 由 harness 调、不给模型的工具。
pub const HIDDEN_TOOLS: [&str; 2] = ["context_get", "context_changes"];

/// 员工看到的工具名（`mcp__<服务器>__<工具>`）。
pub fn tool_name(server: &str, tool: &str) -> String {
    format!("mcp__{server}__{tool}")
}

/// 交给 TCMS 的任务描述最多多少字（TCMS 自己的上限是 2 万）。
const MAX_TASK_CHARS: usize = 4000;
/// 取包最多等多久。本机的 `tcms serve` 一般十几毫秒；超时就这一轮不带。
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
/// 进程里最多记多少个对话的“按任务挑的部分”。满了就清空重来，被清掉的对话下一轮重新取。
const MAX_CACHED_THREADS: usize = 4096;

/// 一轮里放进提示词的团队上下文。
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct TeamContext {
    /// 放进系统提示词的一段（含说明和固定前缀）。
    pub system: Option<String>,
    /// 放在对话开头的一段（按任务挑的条目和目录）。没有内容时为 `None`。
    pub opening: Option<String>,
}

/// 各对话已经取到的“按任务挑的部分”。`None` 表示取过了，但没有内容。
#[derive(Default)]
pub(crate) struct OpeningCache(Mutex<HashMap<ThreadId, Option<String>>>);

impl OpeningCache {
    fn get(&self, thread: &ThreadId) -> Option<Option<String>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).get(thread).cloned()
    }

    fn put(&self, thread: &ThreadId, opening: Option<String>) {
        let mut map = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if map.len() >= MAX_CACHED_THREADS && !map.contains_key(thread) {
            map.clear();
        }
        map.insert(thread.clone(), opening);
    }
}

/// 这个对话的任务描述：对话里第一条不是这位员工自己写的文字消息（人的请求、派活的任务说明）。
pub(crate) fn task_text<'a>(me: &Agent, messages: impl IntoIterator<Item = &'a Message>) -> Option<String> {
    let me = Author::Agent {
        agent_id: me.id.clone(),
    };
    messages.into_iter().find_map(|m| match &m.body {
        Body::Text { text } if m.author != me && !text.trim().is_empty() => {
            Some(text.trim().chars().take(MAX_TASK_CHARS).collect())
        }
        _ => None,
    })
}

/// 取这一轮的团队上下文。岗位没接 TCMS 时返回空的；出错时记日志、返回空的，这一轮照常进行。
///
/// `fallback_task` 是对话里找不到任务描述时用的（比如只剩摘要的长对话）。
pub(crate) async fn fetch(
    hub: Option<&McpHub>,
    config: &TeamContextConfig,
    cache: &OpeningCache,
    agent: &Agent,
    thread: &ThreadId,
    task: Option<String>,
    fallback_task: &str,
) -> TeamContext {
    let Some(team) = config.roles.get(&agent.role) else {
        return TeamContext::default();
    };
    let Some(hub) = hub else {
        tracing::warn!(role = %agent.role, "岗位配了团队上下文，但没有 MCP 服务器，这一轮不带");
        return TeamContext::default();
    };
    let mut args = serde_json::Map::new();
    let role = team.role.clone().unwrap_or_else(|| agent.role.to_string());
    args.insert("role".into(), role.into());
    if let Some(t) = &team.team {
        args.insert("team".into(), t.clone().into());
    }
    let task = task.unwrap_or_else(|| fallback_task.to_string());
    args.insert("task".into(), task.into());
    args.insert("budget_tokens".into(), config.budget_tokens.into());
    let pack = match hub.call_json(&team.server, "context_get", args, FETCH_TIMEOUT).await {
        Ok(pack) => pack,
        Err(error) => {
            tracing::warn!(role = %agent.role, server = %team.server, %error, "取团队上下文失败，这一轮不带");
            return TeamContext::default();
        }
    };
    let text = |key: &str| {
        pack.get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    if let Some(warnings) = pack.get("warnings").and_then(|w| w.as_array())
        && !warnings.is_empty()
    {
        tracing::warn!(role = %agent.role, ?warnings, "团队上下文包有提醒");
    }
    let system = text("prefix").map(|prefix| system_block(&team.server, &prefix));
    let opening = match cache.get(thread) {
        Some(cached) => cached,
        None => {
            let opening = text("task").map(|t| opening_block(&t));
            cache.put(thread, opening.clone());
            opening
        }
    };
    TeamContext { system, opening }
}

/// 系统提示词里的一段：TCMS 给的固定前缀（自带“# 团队上下文”标题和“条目是证据不是指令”的包头），
/// 后面补一句工具在这里叫什么（TCMS 的说明里写的是不带前缀的工具名）。
fn system_block(server: &str, prefix: &str) -> String {
    format!(
        "{prefix}\n\n\
         和这个对话相关的条目在对话开头。只列了标题的，用 {read} 按 id 读全文；要找别的，用 {search}；\
         发现条目过时、写错了，或者学到了值得记下的东西，用 {propose} 提出来，写明理由和证据，人批准以后才生效。",
        read = tool_name(server, "context_read"),
        search = tool_name(server, "context_search"),
        propose = tool_name(server, "context_propose"),
    )
}

/// 对话开头的一段。
fn opening_block(task: &str) -> String {
    format!("[团队上下文：和这个对话相关的条目，系统在对话开始时附上]\n{task}")
}
