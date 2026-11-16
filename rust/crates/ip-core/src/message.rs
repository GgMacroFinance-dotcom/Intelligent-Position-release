//! 对话里的消息。

use serde::{Deserialize, Serialize};

use crate::{AgentId, MessageId, ThreadId};

/// 一条消息。写入后不再修改；更正用新消息表达。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Message {
    pub id: MessageId,
    pub thread_id: ThreadId,
    /// Unix 时间，毫秒。
    #[ts(type = "number")]
    pub at_ms: i64,
    pub author: Author,
    pub body: Body,
    /// 系统消息的类型；人和员工的消息没有这一项。外部渠道按它决定发不发（`docs/ip/m2-spec.md` D-26）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub notice: Option<Notice>,
}

/// 系统消息的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Notice {
    /// 派活说明，写在负责人的执行对话里。
    TaskBrief,
    /// 任务结束（完成、受阻、取消），结果写回派活的对话。
    TaskReport,
    /// 审批卡片被选定后写回对话的说明。
    ApprovalAnswer,
    /// 员工这一轮失败（费用用满、模型出错、步数超限）。
    TurnFailed,
}

/// 消息由谁发出。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Author {
    Human {
        name: String,
        /// 从外部渠道来的人才有：哪个渠道、渠道里的用户 id。控制台用户没有这一项。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        via: Option<Via>,
    },
    Agent {
        agent_id: AgentId,
    },
    System,
}

/// 外部渠道里的一个用户（`docs/ip/m2-spec.md` D-27）。
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Via {
    /// `ip.yaml` 里的渠道名，如 `slack`。
    pub channel: String,
    /// 渠道里稳定不变的用户 id，如 Slack 的 `U012ABCDEF`。
    pub user_id: String,
}

impl Via {
    /// 路由规则 `from`、审批记录里用的写法：`<渠道>:<用户 id>`，如 `slack:U012ABCDEF`。
    pub fn key(&self) -> String {
        format!("{}:{}", self.channel, self.user_id)
    }
}

impl Author {
    /// 控制台里的人。
    pub fn human(name: impl Into<String>) -> Self {
        Self::Human {
            name: name.into(),
            via: None,
        }
    }
}

/// 消息内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Body {
    Text {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        ok: bool,
        output: String,
    },
    /// 上下文压缩：把 `covers_through`（含）之前的对话浓缩成一段摘要。
    /// 之后给模型的历史从这段摘要开始，被覆盖的原消息仍保留在存储里可查。
    Summary {
        text: String,
        covers_through: crate::MessageId,
    },
}

impl Message {
    /// 新建一条文本消息，ID 和时间自动生成。
    pub fn text(thread_id: ThreadId, author: Author, text: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            thread_id,
            at_ms: crate::now_ms(),
            author,
            body: Body::Text { text: text.into() },
            notice: None,
        }
    }

    /// 新建一条系统消息，带上类型。
    pub fn notice(thread_id: ThreadId, notice: Notice, text: impl Into<String>) -> Self {
        Self {
            notice: Some(notice),
            ..Self::text(thread_id, Author::System, text)
        }
    }

    /// 进入全文索引的文字。工具调用和结果不进索引，避免大段输出挤占搜索结果。
    pub fn searchable_text(&self) -> Option<&str> {
        match &self.body {
            Body::Text { text } => Some(text),
            Body::ToolCall { .. } | Body::ToolResult { .. } | Body::Summary { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_with_kind_tags() {
        let message = Message {
            id: "m1".into(),
            thread_id: "t1".into(),
            at_ms: 1,
            author: Author::human("张三"),
            body: Body::Text { text: "你好".into() },
            notice: None,
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["author"]["kind"], "human");
        assert_eq!(json["body"]["kind"], "text");
        // 没有的可选字段不写出来，存储里的旧消息也就和以前一样。
        assert!(
            json.get("notice").is_none() && json["author"].get("via").is_none(),
            "{json}"
        );
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), message);
    }

    #[test]
    fn notice_and_via_round_trip_and_default_when_missing() {
        let notice = Message::notice("t1".into(), Notice::TurnFailed, "小宏 的这轮工作失败：超时");
        assert_eq!(notice.author, Author::System);
        let json = serde_json::to_value(&notice).unwrap();
        assert_eq!(json["notice"], "turn_failed");
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), notice);

        let via = Via {
            channel: "slack".into(),
            user_id: "U012ABCDEF".into(),
        };
        assert_eq!(via.key(), "slack:U012ABCDEF");
        let author = Author::Human {
            name: "张三".into(),
            via: Some(via),
        };
        let json = serde_json::to_value(&author).unwrap();
        assert_eq!(json["via"]["user_id"], "U012ABCDEF");
        assert_eq!(serde_json::from_value::<Author>(json).unwrap(), author);

        // 加字段之前写入的消息照样能读。
        let old = serde_json::json!({
            "id": "m1", "thread_id": "t1", "at_ms": 1,
            "author": {"kind": "human", "name": "张三"},
            "body": {"kind": "text", "text": "你好"}
        });
        let old: Message = serde_json::from_value(old).unwrap();
        assert_eq!(old.author, Author::human("张三"));
        assert_eq!(old.notice, None);
    }

    #[test]
    fn only_text_is_searchable() {
        let mut message = Message::text("t".into(), Author::System, "合同");
        assert_eq!(message.searchable_text(), Some("合同"));
        message.body = Body::ToolResult {
            call_id: "c".into(),
            ok: true,
            output: "x".into(),
        };
        assert_eq!(message.searchable_text(), None);
    }
}
