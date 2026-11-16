//! 请求和流式事件的数据结构，按 OpenAI Chat Completions 的语义定义。

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// 端点内的模型名，例如 `deepseek-chat`。
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolSpec>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>, messages: Vec<ChatMessage>) -> Self {
        Self {
            model: model.into(),
            messages,
            tools: Vec::new(),
            max_tokens: None,
            temperature: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty", with = "wire_tool_calls")]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

/// 一个可供模型调用的工具。`parameters` 是 JSON Schema。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// 模型发起的一次工具调用。`arguments` 是模型给出的 JSON 字符串，可能不合法，由执行方校验。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    /// 命中提示前缀缓存的 token 数（端点支持时）。
    pub cached_tokens: u32,
    /// 这次写进提示前缀缓存的 token 数（端点报告时，如 OpenAI 的 `cache_write_tokens`）。
    /// OpenAI 按输入价的 1.25 倍另收，见 [`crate::Price::cache_write`]。旧数据里没有这一项，读出来是 0。
    #[serde(default)]
    pub cache_write_tokens: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StreamEvent {
    /// 一段新生成的文字。
    TextDelta(String),
    /// 一个完整的工具调用（参数已拼接完）。
    ToolCall(ToolCall),
    /// 本次生成结束。
    Done { finish: FinishReason, usage: Option<Usage> },
}

/// OpenAI 线上格式把工具调用包在 `{"type":"function","function":{...}}` 里。
mod wire_tool_calls {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::ToolCall;

    #[derive(Serialize, Deserialize)]
    struct Wire {
        id: String,
        #[serde(rename = "type")]
        kind: String,
        function: Function,
    }

    #[derive(Serialize, Deserialize)]
    struct Function {
        name: String,
        arguments: String,
    }

    pub fn serialize<S: Serializer>(calls: &[ToolCall], serializer: S) -> Result<S::Ok, S::Error> {
        let wire: Vec<Wire> = calls
            .iter()
            .map(|call| Wire {
                id: call.id.clone(),
                kind: "function".into(),
                function: Function {
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                },
            })
            .collect();
        wire.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<ToolCall>, D::Error> {
        let wire = Vec::<Wire>::deserialize(deserializer)?;
        Ok(wire
            .into_iter()
            .map(|call| ToolCall {
                id: call.id,
                name: call.function.name,
                arguments: call.function.arguments,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assistant_tool_calls_use_the_openai_shape() {
        let message = ChatMessage::Assistant {
            content: None,
            tool_calls: vec![ToolCall {
                id: "c1".into(),
                name: "shell".into(),
                arguments: "{}".into(),
            }],
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"role": "assistant", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "shell", "arguments": "{}"}}]})
        );
        assert_eq!(serde_json::from_value::<ChatMessage>(json).unwrap(), message);
    }
}
