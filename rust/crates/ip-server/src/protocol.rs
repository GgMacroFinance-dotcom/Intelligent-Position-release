//! WebSocket 协议的消息格式。类型由 ts-rs 导出到 `web/src/types`，前端直接引用。

use ip_core::{AgentId, ApprovalId, Message, RoleId, TaskId, TaskStatus, ThreadId};
use ip_router::{Decision, RouterSpec};
use ip_runtime::RuntimeEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

/// 客户端发来的一帧：可选的请求 id（原样带回）加上请求本身。
/// 请求里的字段因此不能叫 `id`，指代对象时用 `task_id`、`approval_id` 这类名字。
#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Envelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub id: Option<Value>,
    #[serde(flatten)]
    pub request: Request,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum Request {
    /// 回复 `Role[]`。
    ListRoles,
    /// 回复 `Agent[]`。
    ListAgents,
    /// 回复 `SpendToday`：今天（UTC）各岗位和语义分派的模型费用。
    SpendToday,
    /// 回复 `ThreadSummary[]`，新的在前。
    ListThreads {
        #[serde(default)]
        #[ts(optional)]
        limit: Option<usize>,
    },
    /// 回复 `Message[]`，按时间正序。
    History {
        thread_id: ThreadId,
        #[serde(default)]
        #[ts(optional, type = "number")]
        before_ms: Option<i64>,
        #[serde(default)]
        #[ts(optional)]
        limit: Option<usize>,
    },
    /// 回复 `SearchHit[]`。
    Search {
        query: String,
        #[serde(default)]
        #[ts(optional)]
        thread_id: Option<ThreadId>,
        #[serde(default)]
        #[ts(optional)]
        limit: Option<usize>,
    },
    /// 试一试路由：这句话会分给谁。回复 `Decision`，不发送。
    Route {
        text: String,
        #[serde(default)]
        #[ts(optional)]
        author: Option<String>,
    },
    /// 回复 `RulesView`：当前生效的路由规则和它的来源。
    ListRules,
    /// 保存一整套路由规则，校验通过后立即生效并持久化，回复保存后的 `RouterSpec`。
    /// `accepted_draft` 是这次采纳的草案 id，保存成功后从草案里移除。
    SaveRules {
        spec: RouterSpec,
        #[serde(default)]
        #[ts(optional)]
        accepted_draft: Option<String>,
    },
    /// 回复 `RuleDraft[]`，新的在前。
    ListRuleDrafts,
    /// 回复 `ScheduleView[]`：定时任务、下次运行时间、上次运行结果。
    ListSchedules,
    /// 立即运行一次定时任务（不影响原定的下次运行），回复 `SendResult`。
    RunSchedule { schedule_id: String },
    /// 忽略一条草案，回复剩下的 `RuleDraft[]`。
    DismissRuleDraft { draft_id: String },
    /// 回复 `Task[]`，最近更新的在前。
    ListTasks {
        /// 只要未完成的。
        #[serde(default)]
        #[ts(optional)]
        open_only: Option<bool>,
        #[serde(default)]
        #[ts(optional)]
        limit: Option<usize>,
    },
    /// 新建任务，回复 `Task`。
    CreateTask {
        title: String,
        #[serde(default)]
        #[ts(optional)]
        description: Option<String>,
        #[serde(default)]
        #[ts(optional)]
        role: Option<RoleId>,
        #[serde(default)]
        #[ts(optional)]
        assignee: Option<AgentId>,
        #[serde(default)]
        #[ts(optional)]
        thread_id: Option<ThreadId>,
        #[serde(default)]
        #[ts(optional, type = "number")]
        due_ms: Option<i64>,
        #[serde(default)]
        #[ts(optional)]
        author: Option<String>,
    },
    /// 改任务的状态或负责人，回复 `Task`。
    UpdateTask {
        task_id: TaskId,
        #[serde(default)]
        #[ts(optional)]
        status: Option<TaskStatus>,
        #[serde(default)]
        #[ts(optional)]
        assignee: Option<AgentId>,
        /// 交回的结论或受阻原因。
        #[serde(default)]
        #[ts(optional)]
        result: Option<String>,
        #[serde(default)]
        #[ts(optional)]
        author: Option<String>,
    },
    /// 回复 `Approval[]`，新的在前。
    ListApprovals {
        #[serde(default)]
        #[ts(optional)]
        pending_only: Option<bool>,
        #[serde(default)]
        #[ts(optional)]
        limit: Option<usize>,
    },
    /// 选定审批卡片的一个选项（从 0 开始），回复 `Approval`。
    ResolveApproval {
        approval_id: ApprovalId,
        choice: u32,
        #[serde(default)]
        #[ts(optional)]
        author: Option<String>,
    },
    /// 回复 `MemoryView`：一个岗位的决策者笔记、岗位记忆摘要、各对话的记忆、人要求的更正。
    GetMemory { role_id: RoleId },
    /// 整体改写决策者笔记或岗位记忆摘要，空文本等于清空这一层。回复 `MemoryView`。
    SaveMemoryText {
        role_id: RoleId,
        layer: MemoryLayer,
        text: String,
    },
    /// 抹掉一个对话在这个岗位下的记忆，下次合并时岗位记忆摘要里来自它的内容会被拿掉。回复 `MemoryView`。
    ForgetThreadMemory { role_id: RoleId, thread_id: ThreadId },
    /// 删一条人要求的更正。回复 `MemoryView`。
    DeleteMemoryNote {
        role_id: RoleId,
        #[ts(type = "number")]
        note_id: i64,
    },
    /// 清空一个岗位的记忆（摘要、各对话的记忆、更正；决策者笔记不动）。回复 `MemoryView`。
    ClearMemory { role_id: RoleId },
    /// 回复 `ThreadId[]`：关掉了记忆的对话。
    ListMemoryOffThreads,
    /// 关掉或重新打开一个对话的记忆。回复 `ThreadId[]`：改完后关掉了记忆的对话。
    SetThreadMemory { thread_id: ThreadId, enabled: bool },
    /// 回复 `ReportInfo[]`：员工工作目录 `reports/` 下的报告，新改的在前。省略 `agent_id` 时列所有员工的。
    ListReports {
        #[serde(default)]
        #[ts(optional)]
        agent_id: Option<AgentId>,
        #[serde(default)]
        #[ts(optional)]
        limit: Option<usize>,
    },
    /// 回复 `ReportText`：读一份报告，`path` 是 `list_reports` 给出的 `reports/<文件名>.md`。
    ReadReport { agent_id: AgentId, path: String },
    /// 回复 `SendResult`；员工的回复随后以事件推送。
    Send {
        /// 省略时由路由规则决定。
        #[serde(default)]
        #[ts(optional)]
        agent_id: Option<AgentId>,
        /// 省略时开一个新对话。
        #[serde(default)]
        #[ts(optional)]
        thread_id: Option<ThreadId>,
        text: String,
        /// 发消息的人；登录体系上线前由客户端给出。
        #[serde(default)]
        #[ts(optional)]
        author: Option<String>,
    },
}

/// 上下文里人能直接改的两层。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MemoryLayer {
    /// 决策者笔记。
    Notes,
    /// 岗位记忆摘要。
    Summary,
}

/// `get_memory` 等请求的回复：一个岗位的记忆全貌。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct MemoryView {
    pub role_id: RoleId,
    /// 这个岗位开没开记忆（`memory.enabled` 和 `memory.roles`）。没开时摘要不进提示词，也不提炼。
    pub enabled: bool,
    pub notes: Option<MemoryText>,
    pub summary: Option<MemoryText>,
    /// 岗位记忆摘要这一版由多少份对话摘要合并而成；人手写的是 0。
    pub summary_from_threads: usize,
    /// 各对话的记忆，最近生成的在前。
    pub threads: Vec<ThreadMemoryView>,
    /// 人要求的更正，旧的在前。
    pub corrections: Vec<CorrectionView>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct MemoryText {
    pub text: String,
    #[ts(type = "number")]
    pub updated_ms: i64,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct ThreadMemoryView {
    pub thread_id: ThreadId,
    pub title: Option<String>,
    pub summary: String,
    #[ts(type = "number")]
    pub generated_ms: i64,
    /// 被员工读过几次。
    pub usage_count: u32,
    #[ts(type = "number | null")]
    pub last_usage_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct CorrectionView {
    #[ts(type = "number")]
    pub note_id: i64,
    pub text: String,
    #[ts(type = "number")]
    pub created_ms: i64,
    pub thread_id: Option<ThreadId>,
    /// 哪次合并用上了它；还没用上是空。
    #[ts(type = "number | null")]
    pub applied_ms: Option<i64>,
}

/// `list_reports` 的一项。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct ReportInfo {
    pub agent_id: AgentId,
    /// 相对员工工作目录的路径，形如 `reports/2026-11-16-cpi.md`。
    pub path: String,
    #[ts(type = "number")]
    pub bytes: u64,
    #[ts(type = "number")]
    pub modified_ms: i64,
}

/// `read_report` 的回复。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct ReportText {
    pub agent_id: AgentId,
    pub path: String,
    pub text: String,
    #[ts(type = "number")]
    pub modified_ms: i64,
    /// 文件超过 1 MB 时只给前 1 MB。
    pub truncated: bool,
}

/// `list_rules` 的回复。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct RulesView {
    pub spec: RouterSpec,
    /// 规则来自控制台保存（true）还是 ip.yaml（false）。
    pub from_console: bool,
}

/// `send` 的回复。
#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
pub struct SendResult {
    pub message: Message,
    /// 实际接手的员工；路由拿不准、等人决定时为空。
    pub agent_id: Option<AgentId>,
    /// 由路由决定时附上路由结果和理由。
    pub decision: Option<Decision>,
    /// 路由拿不准时新建的审批卡片。
    pub approval_id: Option<ApprovalId>,
}

/// 服务发给客户端的一帧。
#[derive(Clone, Debug, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum ServerFrame {
    /// 对某个请求的回复。`ok` 为 true 时看 `data`，否则看 `error`。
    Reply {
        id: Option<Value>,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        data: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        error: Option<String>,
    },
    /// 运行时事件，推给所有连接。装箱是因为事件远比另外两种帧大。
    Event { event: Box<RuntimeEvent> },
    /// 本连接处理太慢，跳过了 `skipped` 个事件；客户端应重新拉取历史。
    Lagged {
        #[ts(type = "number")]
        skipped: u64,
    },
}

impl ServerFrame {
    pub fn reply(id: Option<Value>, result: Result<Value, String>) -> Self {
        match result {
            Ok(data) => Self::Reply {
                id,
                ok: true,
                data: Some(data),
                error: None,
            },
            Err(error) => Self::Reply {
                id,
                ok: false,
                data: None,
                error: Some(error),
            },
        }
    }

    pub fn to_json(&self) -> String {
        // 只含字符串、数字和 serde_json::Value，序列化不会失败。
        serde_json::to_string(self).unwrap_or_else(|e| format!(r#"{{"type":"error","error":"{e}"}}"#))
    }
}
