//! 解析 OpenAI 兼容端点的流式响应（Server-Sent Events），把分片的工具调用参数拼完整。

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{FinishReason, ModelError, StreamEvent, ToolCall, Usage};

/// 增量解析器：喂入任意切分的字节块，吐出完整事件。
#[derive(Default)]
pub struct Decoder {
    buffer: Vec<u8>,
    calls: BTreeMap<u32, PartialCall>,
    finish: Option<FinishReason>,
    usage: Option<Usage>,
    done: bool,
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    delta: Delta,
    finish_reason: Option<String>,
}

#[derive(Default, Deserialize)]
struct Delta {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallDelta>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    #[serde(default)]
    index: u32,
    id: Option<String>,
    function: Option<FunctionDelta>,
}

#[derive(Deserialize)]
struct FunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    prompt_tokens_details: Option<PromptDetails>,
    /// DeepSeek 的字段名。
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct PromptDetails {
    #[serde(default)]
    cached_tokens: u32,
    #[serde(default)]
    cache_write_tokens: u32,
}

impl Decoder {
    /// 喂入一块字节，返回其中已完整的事件。
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<StreamEvent>, ModelError> {
        if self.done {
            return Ok(Vec::new());
        }
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = std::str::from_utf8(&line).map_err(|error| ModelError::Decode(error.to_string()))?;
            self.line(line.trim_end_matches(['\r', '\n']), &mut events)?;
            if self.done {
                self.buffer.clear();
                break;
            }
        }
        Ok(events)
    }

    /// 连接关闭时处理最后一行。只有收到结束标记或 finish_reason 才补结束事件；
    /// 没有完成信号的断流必须报错，不能把未完成的工具调用交给执行方。
    pub fn finish(&mut self) -> Result<Vec<StreamEvent>, ModelError> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            let line = String::from_utf8(rest).map_err(|error| ModelError::Decode(error.to_string()))?;
            self.line(line.trim_end(), &mut events)?;
        }
        if !self.done {
            if self.finish.is_none() {
                return Err(ModelError::Http("响应流在 finish_reason 或 [DONE] 之前断开".into()));
            }
            self.complete(&mut events);
        }
        Ok(events)
    }

    fn line(&mut self, line: &str, events: &mut Vec<StreamEvent>) -> Result<(), ModelError> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(()); // 注释、event:、空行
        };
        let data = data.trim();
        if data == "[DONE]" {
            self.complete(events);
            return Ok(());
        }
        let chunk: Chunk =
            serde_json::from_str(data).map_err(|error| ModelError::Decode(format!("{error}: {data}")))?;
        if let Some(usage) = chunk.usage {
            let details = usage.prompt_tokens_details.as_ref();
            let cached = details
                .map(|d| d.cached_tokens)
                .or(usage.prompt_cache_hit_tokens)
                .unwrap_or(0);
            self.usage = Some(Usage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                cached_tokens: cached,
                cache_write_tokens: details.map_or(0, |d| d.cache_write_tokens),
            });
        }
        for choice in chunk.choices {
            if let Some(text) = choice.delta.content
                && !text.is_empty()
            {
                events.push(StreamEvent::TextDelta(text));
            }
            for delta in choice.delta.tool_calls {
                let call = self.calls.entry(delta.index).or_default();
                if let Some(id) = delta.id {
                    call.id = id;
                }
                if let Some(function) = delta.function {
                    if let Some(name) = function.name {
                        call.name.push_str(&name);
                    }
                    if let Some(arguments) = function.arguments {
                        call.arguments.push_str(&arguments);
                    }
                }
            }
            if let Some(reason) = choice.finish_reason {
                self.finish = Some(match reason.as_str() {
                    "stop" => FinishReason::Stop,
                    "tool_calls" | "function_call" => FinishReason::ToolCalls,
                    "length" => FinishReason::Length,
                    _ => FinishReason::Other,
                });
            }
        }
        Ok(())
    }

    fn complete(&mut self, events: &mut Vec<StreamEvent>) {
        self.done = true;
        let calls = std::mem::take(&mut self.calls);
        let had_calls = !calls.is_empty();
        for (index, call) in calls {
            let id = if call.id.is_empty() {
                format!("call_{index}")
            } else {
                call.id
            };
            events.push(StreamEvent::ToolCall(ToolCall {
                id,
                name: call.name,
                arguments: call.arguments,
            }));
        }
        let finish = self.finish.unwrap_or(if had_calls {
            FinishReason::ToolCalls
        } else {
            FinishReason::Stop
        });
        events.push(StreamEvent::Done {
            finish,
            usage: self.usage,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_in_pieces(body: &str, piece: usize) -> Vec<StreamEvent> {
        let mut decoder = Decoder::default();
        let mut events = Vec::new();
        for chunk in body.as_bytes().chunks(piece) {
            events.extend(decoder.push(chunk).unwrap());
        }
        events.extend(decoder.finish().unwrap());
        events
    }

    const TOOL_STREAM: &str = concat!(
        ": keep-alive\n\n",
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\",\"content\":\"好的，\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"我来查\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"shell\",\"arguments\":\"{\\\"comm\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"and\\\":\\\"ls\\\"}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":120,\"completion_tokens\":9,\"prompt_cache_hit_tokens\":96}}\n\n",
        "data: [DONE]\n\n",
    );

    #[test]
    fn assembles_tool_calls_across_any_split() {
        let expected = vec![
            StreamEvent::TextDelta("好的，".into()),
            StreamEvent::TextDelta("我来查".into()),
            StreamEvent::ToolCall(ToolCall {
                id: "call_a".into(),
                name: "shell".into(),
                arguments: r#"{"command":"ls"}"#.into(),
            }),
            StreamEvent::Done {
                finish: FinishReason::ToolCalls,
                usage: Some(Usage {
                    prompt_tokens: 120,
                    completion_tokens: 9,
                    cached_tokens: 96,
                    cache_write_tokens: 0,
                }),
            },
        ];
        // 包括把一个汉字的 UTF-8 字节切开的切法。
        for piece in [1, 2, 3, 7, 64, TOOL_STREAM.len()] {
            assert_eq!(decode_in_pieces(TOOL_STREAM, piece), expected, "piece size {piece}");
        }
    }

    /// OpenAI 的写法：命中和写入缓存的数都在 `prompt_tokens_details` 里（2026-11-14 真实响应的字段）。
    #[test]
    fn reads_openai_cache_details() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"好\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":1500,\"completion_tokens\":4,",
            "\"prompt_tokens_details\":{\"cached_tokens\":0,\"cache_write_tokens\":1466,\"audio_tokens\":0}}}\n\n",
            "data: [DONE]\n\n",
        );
        let events = decode_in_pieces(body, 64);
        assert_eq!(
            events.last(),
            Some(&StreamEvent::Done {
                finish: FinishReason::Stop,
                usage: Some(Usage {
                    prompt_tokens: 1500,
                    completion_tokens: 4,
                    cached_tokens: 0,
                    cache_write_tokens: 1466,
                }),
            })
        );
    }

    #[test]
    fn closes_without_done_marker() {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}";
        assert_eq!(
            decode_in_pieces(body, 5),
            vec![
                StreamEvent::TextDelta("hi".into()),
                StreamEvent::Done {
                    finish: FinishReason::Stop,
                    usage: None
                }
            ]
        );
    }

    #[test]
    fn reports_bad_json() {
        let mut decoder = Decoder::default();
        assert!(matches!(decoder.push(b"data: {oops\n"), Err(ModelError::Decode(_))));
    }

    #[test]
    fn truncated_stream_does_not_emit_success_or_partial_tool_calls() {
        for body in [
            "",
            "data: {\"choices\":[{\"delta\":{\"content\":\"unfinished\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"shell\",\"arguments\":\"{}\"}}]}}]}\n\n",
        ] {
            let mut decoder = Decoder::default();
            let events = decoder.push(body.as_bytes()).unwrap();
            assert!(events.iter().all(|e| matches!(e, StreamEvent::TextDelta(_))));
            assert!(matches!(decoder.finish(), Err(ModelError::Http(_))));
        }
    }

    #[test]
    fn done_is_terminal_even_when_more_data_arrives() {
        let body = concat!(
            "data: [DONE]\n\n",
            "data: [DONE]\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n",
            "data: invalid\n\n",
        );
        for piece in [1, 5, body.len()] {
            assert_eq!(
                decode_in_pieces(body, piece),
                vec![StreamEvent::Done {
                    finish: FinishReason::Stop,
                    usage: None,
                }]
            );
        }
    }
}
