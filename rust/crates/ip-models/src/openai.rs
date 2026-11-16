//! OpenAI 兼容端点的客户端：默认走 Chat Completions，可配置为 Responses 接口。

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::{Semaphore, mpsc};

use crate::{ChatMessage, ChatRequest, EventStream, ModelClient, ModelError, StreamEvent, ToolSpec, responses, sse};

/// 端点说哪种协议。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireApi {
    /// `POST /chat/completions`：DeepSeek、vLLM、SGLang、llama.cpp server 等都支持。
    #[default]
    ChatCompletions,
    /// `POST /responses`：OpenAI 的推理模型带工具时只能用这个。
    Responses,
}

/// 一个端点的配置。
#[derive(Clone, Debug)]
pub struct OpenAiConfig {
    /// 例如 `https://api.deepseek.com/v1`、`http://gpu-box:8000/v1`。
    pub base_url: String,
    pub api_key: Option<String>,
    /// 同时在途的请求上限；超出的请求排队，不会把对方打到限流。
    pub max_concurrency: usize,
    /// 连接失败、429、5xx 时的重试次数（只在还没收到任何输出时重试）。
    pub max_retries: u32,
    /// 每次尝试等待响应头的时限，读取错误正文也用这个时限。正常流式生成不受这个时限限制。
    pub connect_timeout: Duration,
    pub api: WireApi,
    /// 只对 Responses 生效：`reasoning.effort`，省略时用模型默认值。
    pub reasoning_effort: Option<String>,
}

impl OpenAiConfig {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: None,
            max_concurrency: 16,
            max_retries: 3,
            connect_timeout: Duration::from_secs(30),
            api: WireApi::ChatCompletions,
            reasoning_effort: None,
        }
    }
}

#[derive(Clone)]
pub struct OpenAiCompatible {
    inner: Arc<Inner>,
}

struct Inner {
    config: OpenAiConfig,
    url: String,
    http: reqwest::Client,
    permits: Semaphore,
}

impl OpenAiCompatible {
    pub fn new(config: OpenAiConfig) -> Result<Self, ModelError> {
        let http = reqwest::Client::builder()
            .connect_timeout(config.connect_timeout)
            .build()
            .map_err(|error| ModelError::Http(error.to_string()))?;
        let path = match config.api {
            WireApi::ChatCompletions => "chat/completions",
            WireApi::Responses => "responses",
        };
        let url = format!("{}/{path}", config.base_url.trim_end_matches('/'));
        let permits = Semaphore::new(config.max_concurrency.max(1));
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                url,
                http,
                permits,
            }),
        })
    }
}

impl ModelClient for OpenAiCompatible {
    fn stream(&self, request: ChatRequest) -> EventStream {
        let (sender, receiver) = mpsc::channel(64);
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            // 调用方丢掉接收端就取消整个请求：排队等名额、重试退避、等响应头、读正文，哪一步都立即停下，
            // 丢掉响应（断开连接）并交还名额。只靠发事件失败来察觉，静默的连接会一直占着名额。
            // biased：接收端已经关了就不再发请求。
            tokio::select! {
                biased;
                () = sender.closed() => {}
                result = inner.run(&request, &sender) => {
                    if let Err(error) = result {
                        let _ = sender.send(Err(error)).await;
                    }
                }
            }
        });
        receiver
    }
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<WireTool<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Serialize)]
struct WireTool<'a> {
    #[serde(rename = "type")]
    kind: &'static str,
    function: &'a ToolSpec,
}

impl Inner {
    async fn run(
        &self,
        request: &ChatRequest,
        sender: &mpsc::Sender<Result<StreamEvent, ModelError>>,
    ) -> Result<(), ModelError> {
        let _permit = self.permits.acquire().await.map_err(|_| ModelError::Cancelled)?;
        let (mut response, mut decoder) = match self.config.api {
            WireApi::ChatCompletions => {
                let body = WireRequest {
                    model: &request.model,
                    messages: &request.messages,
                    tools: request
                        .tools
                        .iter()
                        .map(|function| WireTool {
                            kind: "function",
                            function,
                        })
                        .collect(),
                    stream: true,
                    stream_options: StreamOptions { include_usage: true },
                    max_tokens: request.max_tokens,
                    temperature: request.temperature,
                };
                (
                    self.send_with_retry(&body).await?,
                    Decoder::Chat(sse::Decoder::default()),
                )
            }
            WireApi::Responses => {
                let body = responses::wire_request(request, self.config.reasoning_effort.as_deref());
                (
                    self.send_with_retry(&body).await?,
                    Decoder::Responses(responses::Decoder::default()),
                )
            }
        };
        loop {
            let chunk = response
                .chunk()
                .await
                .map_err(|error| ModelError::Http(error.to_string()))?;
            let events = match &chunk {
                Some(bytes) => decoder.push(bytes)?,
                None => decoder.finish()?,
            };
            for event in events {
                let done = matches!(event, StreamEvent::Done { .. });
                if sender.send(Ok(event)).await.is_err() {
                    return Ok(()); // 调用方不要了：丢弃响应即中断连接。
                }
                if done {
                    // 协议已经结束（Chat 的 [DONE]、Responses 的 response.completed），不等 HTTP EOF：
                    // 有的服务端发完结束事件还挂着连接。返回即丢弃响应、交还名额。
                    return Ok(());
                }
            }
            if chunk.is_none() {
                return Ok(());
            }
        }
    }

    async fn send_with_retry(&self, body: &impl Serialize) -> Result<reqwest::Response, ModelError> {
        let mut attempt = 0;
        loop {
            let mut builder = self.http.post(&self.url).json(body);
            if let Some(key) = &self.config.api_key {
                builder = builder.bearer_auth(key);
            }
            // reqwest 的 connect_timeout 只管建连，不包含服务器迟迟不回响应头的时间。
            let response = tokio::time::timeout(self.config.connect_timeout, builder.send())
                .await
                .map_err(|_| ModelError::Http("等待模型响应头超时".into()))
                .and_then(|result| result.map_err(|error| ModelError::Http(error.to_string())));
            let (error, retry_after) = match response {
                Ok(response) if response.status().is_success() => return Ok(response),
                Ok(response) => {
                    let status = response.status().as_u16();
                    let retry_after = response
                        .headers()
                        .get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok()?.parse::<u64>().ok())
                        .map(Duration::from_secs);
                    let body = error_body(response, self.config.connect_timeout).await;
                    let error = ModelError::Status { status, body };
                    if status != 429 && status < 500 {
                        return Err(error);
                    }
                    (error, retry_after)
                }
                Err(error) => (error, None),
            };
            if attempt >= self.config.max_retries {
                return Err(error);
            }
            let backoff = retry_backoff(attempt, retry_after);
            tracing::warn!(%error, attempt, ?backoff, "model request failed, retrying");
            tokio::time::sleep(backoff).await;
            attempt += 1;
        }
    }
}

/// 错误正文只作诊断，按字节限制读取量，再按字符限制显示量；上游不结束时也能保留状态码退出。
async fn error_body(mut response: reqwest::Response, wait: Duration) -> String {
    const MAX_BYTES: usize = 8 * 1024;
    let mut body = Vec::new();
    let read = async {
        while body.len() < MAX_BYTES
            && let Ok(Some(chunk)) = response.chunk().await
        {
            let take = chunk.len().min(MAX_BYTES - body.len());
            body.extend_from_slice(&chunk[..take]);
        }
    };
    let _ = tokio::time::timeout(wait, read).await;
    String::from_utf8_lossy(&body).chars().take(2000).collect()
}

fn retry_backoff(attempt: u32, retry_after: Option<Duration>) -> Duration {
    // 指数到 6 时已超过 30 秒上限；先截住指数，避免大重试次数在取 min 之前就溢出。
    retry_after
        .unwrap_or_else(|| Duration::from_millis(500 * (1u64 << attempt.min(6))))
        .min(Duration::from_secs(30))
}

enum Decoder {
    Chat(sse::Decoder),
    Responses(responses::Decoder),
}

impl Decoder {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<StreamEvent>, ModelError> {
        match self {
            Self::Chat(decoder) => decoder.push(bytes),
            Self::Responses(decoder) => decoder.push(bytes),
        }
    }

    fn finish(&mut self) -> Result<Vec<StreamEvent>, ModelError> {
        match self {
            Self::Chat(decoder) => decoder.finish(),
            Self::Responses(decoder) => decoder.finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_bounded_even_for_large_retry_counts() {
        assert_eq!(retry_backoff(0, None), Duration::from_millis(500));
        assert_eq!(retry_backoff(5, None), Duration::from_secs(16));
        for attempt in [6, 32, 64, u32::MAX] {
            assert_eq!(retry_backoff(attempt, None), Duration::from_secs(30));
        }
        assert_eq!(retry_backoff(0, Some(Duration::ZERO)), Duration::ZERO);
        assert_eq!(retry_backoff(0, Some(Duration::from_secs(60))), Duration::from_secs(30));
    }
}
