//! OpenAI Responses 接口（`POST /responses`）：请求体的转换和流式响应的解析。
//!
//! OpenAI 的推理模型（如 gpt-6-astra、gpt-6-luna）在 Chat Completions 里不能同时带工具和推理，
//! 只能走这个接口。对话历史每次全量发送（`store: false`），不依赖服务端保存的上下文。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{ChatMessage, ChatRequest, FinishReason, ModelError, StreamEvent, ToolCall, Usage};

#[derive(Serialize)]
pub(crate) struct WireRequest<'a> {
    model: &'a str,
    input: Vec<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Value>,
    stream: bool,
    store: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning: Option<Value>,
}

/// 把通用请求转成 Responses 格式。
///
/// `temperature` 不发送：推理模型一律拒绝这个参数。`max_tokens` 映射为 `max_output_tokens`，
/// 它同时计入推理用的 token，太小时可能一个字都没输出就停了。
pub(crate) fn wire_request<'a>(request: &'a ChatRequest, reasoning_effort: Option<&str>) -> WireRequest<'a> {
    let input = request
        .messages
        .iter()
        .flat_map(|message| match message {
            ChatMessage::System { content } => vec![json!({"role": "system", "content": content})],
            ChatMessage::User { content } => vec![json!({"role": "user", "content": content})],
            ChatMessage::Assistant { content, tool_calls } => {
                let mut items = Vec::new();
                if let Some(text) = content.as_deref().filter(|text| !text.is_empty()) {
                    items.push(json!({"role": "assistant", "content": text}));
                }
                items.extend(tool_calls.iter().map(|call| {
                    json!({"type": "function_call", "call_id": call.id, "name": call.name, "arguments": call.arguments})
                }));
                items
            }
            ChatMessage::Tool { tool_call_id, content } => {
                vec![json!({"type": "function_call_output", "call_id": tool_call_id, "output": content})]
            }
        })
        .collect();
    let tools = request
        .tools
        .iter()
        .map(|tool| {
            // strict 默认是 true，要求 schema 满足严格模式；外部 MCP 工具的 schema 多半不满足。
            json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
                "strict": false,
            })
        })
        .collect();
    WireRequest {
        model: &request.model,
        input,
        tools,
        stream: true,
        store: false,
        max_output_tokens: request.max_tokens,
        reasoning: reasoning_effort.map(|effort| json!({"effort": effort})),
    }
}

/// 增量解析器：喂入任意切分的字节块，吐出完整事件。工具调用在 `output_item.done` 时整个给出。
#[derive(Default)]
pub(crate) struct Decoder {
    buffer: Vec<u8>,
    had_calls: bool,
    done: bool,
}

#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    delta: Option<String>,
    #[serde(default)]
    item: Option<Item>,
    #[serde(default)]
    response: Option<Response>,
    /// `error` 事件的内容。
    #[serde(default)]
    message: Option<String>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    arguments: String,
}

#[derive(Deserialize)]
struct Response {
    #[serde(default)]
    usage: Option<WireUsage>,
    #[serde(default)]
    error: Option<Value>,
    #[serde(default)]
    incomplete_details: Option<Value>,
}

#[derive(Deserialize)]
struct WireUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    input_tokens_details: Option<InputDetails>,
}

#[derive(Deserialize)]
struct InputDetails {
    #[serde(default)]
    cached_tokens: u32,
    #[serde(default)]
    cache_write_tokens: u32,
}

impl Decoder {
    pub(crate) fn push(&mut self, bytes: &[u8]) -> Result<Vec<StreamEvent>, ModelError> {
        self.buffer.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(end) = self.buffer.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = std::str::from_utf8(&line).map_err(|error| ModelError::Decode(error.to_string()))?;
            self.line(line.trim_end_matches(['\r', '\n']), &mut events)?;
        }
        Ok(events)
    }

    /// 连接关闭时调用。没收到 `response.completed` 就断开，说明响应不完整，报错而不是当作正常结束。
    pub(crate) fn finish(&mut self) -> Result<Vec<StreamEvent>, ModelError> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let rest = std::mem::take(&mut self.buffer);
            let line = String::from_utf8(rest).map_err(|error| ModelError::Decode(error.to_string()))?;
            self.line(line.trim_end(), &mut events)?;
        }
        if !self.done {
            return Err(ModelError::Http("响应流在 response.completed 之前断开".into()));
        }
        Ok(events)
    }

    fn line(&mut self, line: &str, events: &mut Vec<StreamEvent>) -> Result<(), ModelError> {
        let Some(data) = line.strip_prefix("data:") else {
            return Ok(()); // event:、注释、空行
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" || self.done {
            return Ok(());
        }
        let event: Event =
            serde_json::from_str(data).map_err(|error| ModelError::Decode(format!("{error}: {data}")))?;
        match event.kind.as_str() {
            "response.output_text.delta" => {
                if let Some(text) = event.delta.filter(|text| !text.is_empty()) {
                    events.push(StreamEvent::TextDelta(text));
                }
            }
            "response.output_item.done" => {
                if let Some(item) = event.item.filter(|item| item.kind == "function_call") {
                    self.had_calls = true;
                    events.push(StreamEvent::ToolCall(ToolCall {
                        id: item.call_id.or(item.id).unwrap_or_default(),
                        name: item.name,
                        arguments: item.arguments,
                    }));
                }
            }
            "response.completed" | "response.incomplete" => {
                self.done = true;
                let response = event.response;
                let usage = response.as_ref().and_then(|r| r.usage.as_ref()).map(|usage| Usage {
                    prompt_tokens: usage.input_tokens,
                    completion_tokens: usage.output_tokens,
                    cached_tokens: usage.input_tokens_details.as_ref().map_or(0, |d| d.cached_tokens),
                    cache_write_tokens: usage.input_tokens_details.as_ref().map_or(0, |d| d.cache_write_tokens),
                });
                let finish = if self.had_calls {
                    FinishReason::ToolCalls
                } else if event.kind == "response.incomplete" {
                    let reason = response.and_then(|r| r.incomplete_details).unwrap_or(Value::Null);
                    if reason["reason"] == "max_output_tokens" {
                        FinishReason::Length
                    } else {
                        FinishReason::Other
                    }
                } else {
                    FinishReason::Stop
                };
                events.push(StreamEvent::Done { finish, usage });
            }
            "response.failed" => {
                let error = event.response.and_then(|r| r.error).unwrap_or(Value::Null);
                return Err(ModelError::Status {
                    status: 200,
                    body: format!("response.failed：{error}"),
                });
            }
            "error" => {
                return Err(ModelError::Status {
                    status: 200,
                    body: format!("error：{}", event.message.unwrap_or_else(|| data.to_owned())),
                });
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolSpec;

    fn decode_in_pieces(body: &str, piece: usize) -> Result<Vec<StreamEvent>, ModelError> {
        let mut decoder = Decoder::default();
        let mut events = Vec::new();
        for chunk in body.as_bytes().chunks(piece) {
            events.extend(decoder.push(chunk)?);
        }
        events.extend(decoder.finish()?);
        Ok(events)
    }

    // 按 2026-11-11 gpt-6-luna 的真实响应删减而来；`cache_write_tokens` 是 2026-11-14 真实响应里看到的字段。
    const TOOL_STREAM: &str = concat!(
        "event: response.created\n",
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\",\"status\":\"in_progress\"}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"item\":{\"id\":\"rs_1\",\"type\":\"reasoning\"},\"output_index\":0}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"我来\",\"item_id\":\"msg_1\"}\n\n",
        "data: {\"type\":\"response.output_text.delta\",\"delta\":\"查一下\",\"item_id\":\"msg_1\"}\n\n",
        "data: {\"type\":\"response.function_call_arguments.delta\",\"delta\":\"{\\\"q\\\"\",\"item_id\":\"fc_1\"}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"item\":{\"id\":\"fc_1\",\"type\":\"function_call\",\"status\":\"completed\",\"arguments\":\"{\\\"q\\\":\\\"利率\\\"}\",\"call_id\":\"call_a\",\"name\":\"get_x\"},\"output_index\":2}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp_1\",\"status\":\"completed\",\"usage\":{\"input_tokens\":120,\"input_tokens_details\":{\"cache_write_tokens\":20,\"cache_write_tokens_details\":{\"ttl_12h_tokens\":0,\"ttl_30m_tokens\":20},\"cached_tokens\":96},\"output_tokens\":30,\"output_tokens_details\":{\"reasoning_tokens\":18}}}}\n\n",
    );

    #[test]
    fn assembles_text_and_tool_calls_across_any_split() {
        let expected = vec![
            StreamEvent::TextDelta("我来".into()),
            StreamEvent::TextDelta("查一下".into()),
            StreamEvent::ToolCall(ToolCall {
                id: "call_a".into(),
                name: "get_x".into(),
                arguments: r#"{"q":"利率"}"#.into(),
            }),
            StreamEvent::Done {
                finish: FinishReason::ToolCalls,
                usage: Some(Usage {
                    prompt_tokens: 120,
                    completion_tokens: 30,
                    cached_tokens: 96,
                    cache_write_tokens: 20,
                }),
            },
        ];
        for piece in [1, 2, 3, 7, 64, TOOL_STREAM.len()] {
            assert_eq!(
                decode_in_pieces(TOOL_STREAM, piece).unwrap(),
                expected,
                "piece size {piece}"
            );
        }
    }

    #[test]
    fn incomplete_for_max_tokens_is_length() {
        let body = "data: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n";
        assert_eq!(
            decode_in_pieces(body, 4).unwrap(),
            vec![StreamEvent::Done {
                finish: FinishReason::Length,
                usage: None
            }]
        );
    }

    #[test]
    fn failures_and_truncation_are_errors() {
        let failed = "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"server_error\"}}}\n";
        assert!(matches!(decode_in_pieces(failed, 8), Err(ModelError::Status { .. })));
        let cut = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n";
        assert!(matches!(decode_in_pieces(cut, 8), Err(ModelError::Http(_))));
    }

    #[test]
    fn converts_history_to_input_items() {
        let mut request = ChatRequest::new(
            "gpt-6-luna",
            vec![
                ChatMessage::System { content: "sys".into() },
                ChatMessage::User { content: "hi".into() },
                ChatMessage::Assistant {
                    content: Some("查一下".into()),
                    tool_calls: vec![ToolCall {
                        id: "c1".into(),
                        name: "get_x".into(),
                        arguments: "{}".into(),
                    }],
                },
                ChatMessage::Tool {
                    tool_call_id: "c1".into(),
                    content: "42".into(),
                },
            ],
        );
        request.tools.push(ToolSpec {
            name: "get_x".into(),
            description: "d".into(),
            parameters: json!({"type": "object"}),
        });
        request.temperature = Some(0.0);
        request.max_tokens = Some(200);
        let body = serde_json::to_value(wire_request(&request, Some("low"))).unwrap();
        assert_eq!(
            body,
            json!({
                "model": "gpt-6-luna",
                "input": [
                    {"role": "system", "content": "sys"},
                    {"role": "user", "content": "hi"},
                    {"role": "assistant", "content": "查一下"},
                    {"type": "function_call", "call_id": "c1", "name": "get_x", "arguments": "{}"},
                    {"type": "function_call_output", "call_id": "c1", "output": "42"},
                ],
                "tools": [{"type": "function", "name": "get_x", "description": "d", "parameters": {"type": "object"}, "strict": false}],
                "stream": true,
                "store": false,
                "max_output_tokens": 200,
                "reasoning": {"effort": "low"},
            })
        );
    }
}
