//! 模型接入：云端模型与可配置的推理端点（本地 / 外接 GPU），流式输出、工具调用、限流、重试。
//!
//! 默认走 OpenAI 兼容的 Chat Completions 接口：DeepSeek、OpenAI、vLLM、SGLang、llama.cpp server
//! 都说这个协议，所以岗位小模型以后外接 GPU 时只是换一个端点地址。OpenAI 的推理模型带工具时
//! 只能走 Responses 接口，端点配置 `api: responses` 即可。
//!
//! 调用方用 [`ModelClient::stream`] 发起请求，拿到一个接收端，依次收到 [`StreamEvent`]。
//! 每个请求在自己的 tokio 任务里跑，调用方不阻塞。

mod openai;
mod pricing;
mod registry;
mod responses;
mod scripted;
mod sse;
mod types;

pub use openai::{OpenAiCompatible, OpenAiConfig, WireApi};
pub use pricing::Price;
pub use registry::{ModelRegistry, ResolveError};
pub use scripted::{ScriptedModel, ScriptedTurn};
pub use types::{ChatMessage, ChatRequest, FinishReason, StreamEvent, ToolCall, ToolSpec, Usage};

use tokio::sync::mpsc;

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("网络错误：{0}")]
    Http(String),
    #[error("模型服务返回 {status}：{body}")]
    Status { status: u16, body: String },
    #[error("无法解析模型输出：{0}")]
    Decode(String),
    #[error("请求被取消")]
    Cancelled,
}

/// 流式事件的接收端。发送端被丢弃即表示流结束。
pub type EventStream = mpsc::Receiver<Result<StreamEvent, ModelError>>;

/// 一个模型端点。实现必须在后台任务里完成请求，`stream` 本身立刻返回。
pub trait ModelClient: Send + Sync + 'static {
    fn stream(&self, request: ChatRequest) -> EventStream;
}
