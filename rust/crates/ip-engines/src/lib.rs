//! 外部引擎：把一个岗位的每一轮工作交给外部的编程智能体（`docs/ip/m2-spec.md` 第 6 节）。
//!
//! 有 Codex（D-20 到 D-25）和 Claude Code（D-36）两种。每轮起一个进程，跑完就退出，空闲员工不占进程；
//! 同一对话的后续几轮续接引擎自己的会话。引擎有自己的 shell 和改文件工具；ip 的组织工具通过 `/mcp`
//! 开放给它，用本轮令牌鉴权。两种引擎共用 [`process`] 里的进程管理，只是参数和输出格式不同。
//!
//! 命令行参数和输出格式按 2026-11 的版本写成，各版本略有出入，换版本时要核对一遍。
//! 不认识的输出事件只打调试日志，不会让一轮失败。

mod claude_code;
mod codex;
mod process;

pub use claude_code::{ClaudeCodeAccess, ClaudeCodeConfig, ClaudeCodeEngine};
pub use codex::{CodexConfig, CodexEngine, CodexSandbox};

use serde::Deserialize;

/// `ip.yaml` 里 `engines` 下的一项：某个岗位用哪种外部引擎、怎么配置。
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineConfig {
    Codex(CodexConfig),
    ClaudeCode(ClaudeCodeConfig),
}

impl EngineConfig {
    /// 给人看的名字，用在 `ip check` 和日志里。
    pub fn label(&self) -> &'static str {
        match self {
            Self::Codex(_) => "Codex",
            Self::ClaudeCode(_) => "Claude Code",
        }
    }

    /// 可执行文件。
    pub fn command(&self) -> &str {
        match self {
            Self::Codex(c) => &c.command,
            Self::ClaudeCode(c) => &c.command,
        }
    }

    /// 计费用的价格；没配时这个岗位的引擎用量不计费。
    pub fn price(&self) -> Option<&ip_models::Price> {
        match self {
            Self::Codex(c) => c.price.as_ref(),
            Self::ClaudeCode(c) => c.price.as_ref(),
        }
    }

    /// 存放 API key 的环境变量名。
    pub fn api_key_env(&self) -> Option<&str> {
        match self {
            Self::Codex(c) => c.api_key_env.as_deref(),
            Self::ClaudeCode(c) => Some(&c.api_key_env),
        }
    }

    /// 反序列化之外的检查（交给命令行的值）。
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Codex(_) => Ok(()),
            Self::ClaudeCode(c) => c.validate(),
        }
    }
}
