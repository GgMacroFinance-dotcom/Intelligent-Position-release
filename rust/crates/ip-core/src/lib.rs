//! 领域模型：组织、岗位、数字员工、对话、消息、任务、审批。只放数据结构和纯函数，不做 IO。
//!
//! 负责人：工程师 A。

mod ids;
mod message;
mod org;
mod work;

pub use ids::{AgentId, ApprovalId, MessageId, RoleId, TaskId, ThreadId};
pub use message::{Author, Body, Message, Notice, Via};
pub use org::{
    Agent, CELESTIAL_STYLES, ModelSlot, ORBIT_TRACKS, OrbitPin, OrgError, OrgSpec, Role, celestial_style_ok,
};
pub use work::{
    Approval, ApprovalOption, ApprovalState, ApprovalSubject, COMPLETION_APPROVE, COMPLETION_REWORK, Task, TaskStatus,
};

/// 当前 Unix 时间，毫秒。
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}
