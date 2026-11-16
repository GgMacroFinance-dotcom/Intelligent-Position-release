//! 外部引擎：把一个岗位的每一轮工作交给外部的编程智能体（`docs/ip/m2-spec.md` 第 6 节）。
//!
//! 目前只有 Codex，并且先只给运维测试岗位用。每轮起一个 `codex exec --json` 进程，跑完就退出，
//! 空闲员工不占进程；同一对话的后续几轮用 `codex exec resume` 续接。Codex 有自己的 shell 和
//! 改文件工具，跑在它自带的沙箱里；ip 的组织工具通过 `/mcp` 开放给它，用本轮令牌鉴权。
//!
//! 命令行参数和 JSONL 输出格式按 2026-11 的 Codex 写成，各版本略有出入；用真实 Codex 验收时
//! （规格 M2-6）要核对一遍。不认识的输出事件只打调试日志，不会让一轮失败。

mod codex;

pub use codex::{CodexConfig, CodexEngine, CodexSandbox};

use serde::Deserialize;

/// `ip.yaml` 里 `engines` 下的一项：某个岗位用哪种外部引擎、怎么配置。
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EngineConfig {
    Codex(CodexConfig),
}

impl EngineConfig {
    /// 计费用的价格；没配时这个岗位的引擎用量不计费。
    pub fn price(&self) -> Option<&ip_models::Price> {
        match self {
            Self::Codex(c) => c.price.as_ref(),
        }
    }

    /// 存放 API key 的环境变量名。
    pub fn api_key_env(&self) -> Option<&str> {
        match self {
            Self::Codex(c) => c.api_key_env.as_deref(),
        }
    }
}
