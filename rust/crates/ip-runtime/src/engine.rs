//! 外部引擎接口（`docs/ip/m2-spec.md` D-20）。
//!
//! 一轮工作默认由内置循环完成：拼提示词 → 调模型 → 执行工具 → 写回。岗位也可以把这一轮交给
//! 外部引擎（如 Codex），引擎有自己的工具循环，通过 [`TurnContext::mcp`] 回调 ip 的组织工具。
//! 运行时负责引擎前后的事：查预算、挑出上一轮之后的新消息、签发本轮令牌、记账、保存会话；
//! 引擎只管跑这一轮，并把过程写回对话。失败照旧走“这轮失败”和“任务受阻”。

use std::path::Path;
use std::time::Duration;

use ip_core::{Agent, Body, Role, ThreadId};
use ip_models::Usage;

pub use ip_mcp::server::BoxFuture;

/// 本轮的 MCP 入口：引擎带着令牌调 `url` 上的组织工具（D-17、D-18）。
#[derive(Clone, Debug)]
pub struct McpGrant {
    pub url: String,
    pub token: String,
}

/// 引擎跑一轮需要的全部信息。
pub struct TurnContext<'a> {
    pub agent: &'a Agent,
    pub role: &'a Role,
    pub thread: &'a ThreadId,
    /// 员工的工作目录；没有工具箱的员工为 `None`。
    pub workspace: Option<&'a Path>,
    /// 引擎在这个对话里已有的会话。为 `None` 时是第一轮，要把 `system_prompt` 一并交给引擎。
    pub session: Option<&'a str>,
    /// 组织规则、岗位说明、同事名单，和内置循环用的一样。
    pub system_prompt: &'a str,
    /// 上一轮之后对话里的新消息（不含员工自己写的），已整理成文字。
    pub input: &'a str,
    /// 没有挂 MCP 服务端（比如不经 `ip serve` 运行的测试）时为 `None`。
    pub mcp: Option<McpGrant>,
    /// 往对话里写消息。
    pub sink: &'a dyn TurnSink,
}

/// 引擎把这一轮的过程写回对话的通道。
pub trait TurnSink: Send + Sync {
    /// 以这位员工的名义写一条消息（文字、工具调用或工具结果），写进存储并广播。
    fn append(&self, body: Body) -> BoxFuture<'_, Result<(), String>>;
}

/// 一轮的结果。即使失败，也要如实报告已经产生的用量，好记账。
#[derive(Clone, Debug, Default)]
pub struct TurnReport {
    /// 引擎内部的步数（工具调用加回复的条数），只用于展示。
    pub steps: u32,
    pub usage: Usage,
    pub cost_usd: f64,
    /// 引擎的会话 id（新开或沿用），下一轮用它续接。
    pub session_id: Option<String>,
    /// 这一轮失败的原因；`None` 表示成功。
    pub error: Option<String>,
}

/// 一种外部引擎。
pub trait Engine: Send + Sync + 'static {
    /// 种类名，如 `codex`，写进引擎会话表。换了引擎种类时旧会话不再续接。
    fn kind(&self) -> &str;
    /// 一轮最长多久；本轮令牌的期限据此确定。
    fn turn_timeout(&self) -> Duration;
    fn turn<'a>(&'a self, ctx: TurnContext<'a>) -> BoxFuture<'a, TurnReport>;
}
