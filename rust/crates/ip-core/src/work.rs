//! 任务和审批。
//!
//! 任务是要有人（或数字员工）负责做完的一件事；审批是需要人拍板的一个问题，以决策卡片出现在控制台。
//! 两者都是可变状态，存储里每次变更记一条事件，再更新投影表。

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{AgentId, ApprovalId, Author, MessageId, RoleId, TaskId, ThreadId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum TaskStatus {
    Open,
    InProgress,
    Blocked,
    Done,
    Cancelled,
}

impl TaskStatus {
    pub fn is_closed(self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Task {
    pub id: TaskId,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: TaskStatus,
    /// 负责的岗位；具体由谁做看 `assignee`。
    pub role: Option<RoleId>,
    pub assignee: Option<AgentId>,
    /// 负责人做这件事的对话。分派时运行时把任务说明写进去并通知负责人。
    pub thread_id: Option<ThreadId>,
    /// 派活的对话。任务完成、受阻或取消时，结果写回这里，派活的员工被叫回来接着做。
    #[serde(default)]
    pub origin_thread_id: Option<ThreadId>,
    /// 负责人交回的结果或受阻原因。
    #[serde(default)]
    pub result: Option<String>,
    /// 委派层级：人或顶层对话派的是 1，员工在执行对话里再往下派加 1。用来挡住无限转派。
    #[serde(default)]
    pub depth: u32,
    pub created_by: Author,
    #[ts(type = "number")]
    pub created_at_ms: i64,
    #[ts(type = "number")]
    pub updated_at_ms: i64,
    #[ts(type = "number | null")]
    pub due_ms: Option<i64>,
}

/// 决策卡片上的一个选项。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ApprovalOption {
    pub label: String,
    /// 选了会发生什么。
    pub consequence: String,
}

/// 审批因何而起；决定选项被选中后系统做什么。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ApprovalSubject {
    /// 路由拿不准的一条消息。选项与 `roles` 一一对应，选中后交给该岗位最空闲的员工。
    RouteMessage {
        message_id: MessageId,
        thread_id: ThreadId,
        text: String,
        roles: Vec<RoleId>,
    },
    /// 员工在工作中提的问题。选定后结果写进该对话，并通知这位员工接着做。
    AgentQuestion { agent_id: AgentId, thread_id: ThreadId },
    /// 一般问题，选中后只记录结果。
    Question,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
#[ts(export)]
pub enum ApprovalState {
    Pending,
    Resolved {
        choice: u32,
        by: String,
        #[ts(type = "number")]
        at_ms: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Approval {
    pub id: ApprovalId,
    pub question: String,
    pub options: Vec<ApprovalOption>,
    /// 推荐的选项序号。
    pub recommended: Option<u32>,
    /// 推荐理由或背景。
    #[serde(default)]
    pub reason: String,
    pub subject: ApprovalSubject,
    pub state: ApprovalState,
    #[ts(type = "number")]
    pub created_at_ms: i64,
}

impl Approval {
    pub fn is_pending(&self) -> bool {
        matches!(self.state, ApprovalState::Pending)
    }
}
