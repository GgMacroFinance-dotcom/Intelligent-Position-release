//! 按脚本回复的假模型，给测试和基准用，不访问网络。

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::{ChatRequest, EventStream, FinishReason, ModelClient, StreamEvent, ToolCall, Usage};

/// 脚本里的一轮回复。
#[derive(Clone, Debug)]
pub enum ScriptedTurn {
    /// 回复一段文字，按 `chunk` 个字符一片地流出。
    Text(String),
    /// 发起工具调用。
    ToolCalls(Vec<ToolCall>),
}

/// 依次播放脚本；脚本用完后重复最后一轮，便于基准里无限对话。
pub struct ScriptedModel {
    turns: Mutex<VecDeque<ScriptedTurn>>,
    last: Mutex<Option<ScriptedTurn>>,
    chunk: usize,
    delay: Duration,
    usage: Usage,
    requests: Mutex<Vec<ChatRequest>>,
}

impl ScriptedModel {
    pub fn new(turns: impl IntoIterator<Item = ScriptedTurn>) -> Self {
        Self {
            turns: Mutex::new(turns.into_iter().collect()),
            last: Mutex::new(None),
            chunk: 4,
            delay: Duration::ZERO,
            usage: Usage::default(),
            requests: Mutex::new(Vec::new()),
        }
    }

    /// 每片之间的延迟，模拟真实模型的输出速度。
    #[must_use]
    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    /// 每次回复报告的 token 用量（测试计费用）。
    #[must_use]
    pub fn with_usage(mut self, usage: Usage) -> Self {
        self.usage = usage;
        self
    }

    /// 收到过的请求，测试里用来检查提示词。
    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn next_turn(&self) -> ScriptedTurn {
        let next = self
            .turns
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front();
        let mut last = self.last.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match next {
            Some(turn) => {
                *last = Some(turn.clone());
                turn
            }
            None => last.clone().unwrap_or_else(|| ScriptedTurn::Text(String::new())),
        }
    }
}

impl ModelClient for ScriptedModel {
    fn stream(&self, request: ChatRequest) -> EventStream {
        self.requests
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(request);
        let turn = self.next_turn();
        let (chunk, delay, usage) = (self.chunk.max(1), self.delay, self.usage);
        let (sender, receiver) = mpsc::channel(64);
        tokio::spawn(async move {
            let finish = match turn {
                ScriptedTurn::Text(text) => {
                    let chars: Vec<char> = text.chars().collect();
                    for piece in chars.chunks(chunk) {
                        if !delay.is_zero() {
                            tokio::time::sleep(delay).await;
                        }
                        if sender
                            .send(Ok(StreamEvent::TextDelta(piece.iter().collect())))
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                    FinishReason::Stop
                }
                ScriptedTurn::ToolCalls(calls) => {
                    for call in calls {
                        if sender.send(Ok(StreamEvent::ToolCall(call))).await.is_err() {
                            return;
                        }
                    }
                    FinishReason::ToolCalls
                }
            };
            let _ = sender
                .send(Ok(StreamEvent::Done {
                    finish,
                    usage: Some(usage),
                }))
                .await;
        });
        receiver
    }
}
