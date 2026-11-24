//! 外部渠道的通用接口（`docs/ip/m2-spec.md` 第 3 节）：Slack、飞书、邮件……都实现 [`Channel`]。
//!
//! 渠道只管和外部系统打交道：收到消息、卡片被点击时交给 [`Hub`]；要发出去的东西由投递器
//! 调 [`Channel::deliver`]。收到的消息怎么分派、发什么、发到哪个会话，都由 `ip-server` 里
//! 实现 [`Hub`] 的一方和投递器决定（D-03 到 D-07），渠道不用知道。
//!
//! 本 crate 只依赖 `ip-core`（D-01）。测试用的假渠道在 `fake` feature 后面（D-09）。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use ip_core::{Approval, ApprovalId};

pub use tokio_util::sync::CancellationToken;

#[cfg(feature = "fake")]
pub mod fake;
pub mod slack;
mod ws_proxy;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 外部会话 key，格式由渠道决定，ip 只当作不透明字符串。Slack 用 `<team>/<channel>/<thread_ts>`。
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConversationKey(pub String);

impl From<&str> for ConversationKey {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl std::fmt::Display for ConversationKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// 渠道里发消息或点卡片的人。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sender {
    /// 渠道里稳定不变的用户 id，如 Slack 的 `U012ABCDEF`。
    pub external_id: String,
    /// 显示名，写进 `Author::Human { name }`。
    pub display_name: String,
}

/// 渠道收到的一条消息（D-03）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inbound {
    /// `ip.yaml` 里的渠道名。
    pub channel: String,
    /// 去重用，由渠道决定（Slack 见 D-12）。外部系统重复推送同一条消息时它不变。
    pub dedupe_key: String,
    pub conversation: ConversationKey,
    pub sender: Sender,
    /// 已去掉 @机器人 之类的标记。
    pub text: String,
    /// 这条消息能不能开启一段新会话。例如 Slack 频道里只有 @机器人 才算。
    pub may_start: bool,
}

/// 卡片上的一次点击。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardClick {
    pub channel: String,
    pub dedupe_key: String,
    pub approval_id: ApprovalId,
    /// 选项序号，从 0 开始。
    pub choice: u32,
    pub sender: Sender,
}

/// 要发出去的东西（D-04）。
#[derive(Clone, Debug, PartialEq)]
pub enum Outbound {
    Text {
        conversation: ConversationKey,
        text: String,
    },
    /// 员工回复里提到的报告，正文作为文件发进同一会话。
    Report {
        conversation: ConversationKey,
        filename: String,
        title: String,
        markdown: String,
    },
    Card {
        conversation: ConversationKey,
        approval: Approval,
    },
    /// 卡片已被选定：更新原卡片，去掉按钮，写明谁选了什么。`external_ref` 是发卡片时拿到的引用。
    CardResolved {
        conversation: ConversationKey,
        approval: Approval,
        external_ref: String,
    },
}

impl Outbound {
    pub fn conversation(&self) -> &ConversationKey {
        match self {
            Self::Text { conversation, .. }
            | Self::Report { conversation, .. }
            | Self::Card { conversation, .. }
            | Self::CardResolved { conversation, .. } => conversation,
        }
    }
}

/// 发送成功后外部系统给的引用。Slack 是消息的 ts，更新卡片时要用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivered {
    pub external_ref: String,
}

/// 发送失败。投递器据此决定重试还是跳过（D-04）。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeliverError {
    /// 可以重试：网络、限流（带建议的等待时间）、5xx。
    #[error("暂时发不出去：{reason}")]
    Retry { after: Option<Duration>, reason: String },
    /// 重试也没用：会话不存在、机器人被移出频道等。记失败并跳过。
    #[error("发不出去：{0}")]
    Permanent(String),
}

/// [`Hub`] 拒绝或没能处理一条消息、一次点击。
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum HubError {
    /// 按规则拒绝，比如不符合 `approvers` 的点击（D-07）。原因要告诉操作的人：
    /// 渠道把它只给这个人看（Slack 用只有本人可见的消息）。
    #[error("{0}")]
    Rejected(String),
    /// 内部错误，比如存储失败。渠道打日志即可，外部系统重推时会再来一次。
    #[error("{0}")]
    Failed(String),
}

/// 渠道确认之前存下的一件事（#69）：外部系统推来的原始内容，还没处理完。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stashed {
    /// 外部系统给的 id，Slack 是 `envelope_id`。
    pub id: String,
    /// 原样的内容，由渠道自己解析。
    pub payload: String,
    /// 已经失败过几次。
    pub attempts: u32,
}

/// 一件事处理的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settle {
    /// 处理完了（成功，或按规则忽略、拒绝）。
    Done,
    /// 这次没成（暂时性的错误），留着再试。
    Failed(String),
    /// 重试多次仍失败，不再处理，留档。
    GaveUp(String),
}

/// 一个渠道的实现：Slack、假渠道……
pub trait Channel: Send + Sync + 'static {
    /// `ip.yaml` 里的渠道名。
    fn name(&self) -> &str;

    /// 连上外部系统，把收到的消息和点击交给 `hub`，直到 `shutdown` 被取消。断线要自己重连。
    fn run(self: Arc<Self>, hub: Arc<dyn Hub>, shutdown: CancellationToken) -> BoxFuture<'static, ()>;

    /// 发出一样东西。投递器对同一个渠道是一件一件顺序调用的。
    fn deliver<'a>(&'a self, out: &'a Outbound) -> BoxFuture<'a, Result<Delivered, DeliverError>>;
}

/// 由 `ip-server` 实现：收消息流程（D-03）、卡片点击（D-05、D-07），以及确认前的持久化（D-28）。
pub trait Hub: Send + Sync + 'static {
    /// 处理一条消息。按规则忽略的消息（重复的、名单外的、不能开启会话的）返回 `Ok`。
    /// 返回 [`HubError::Failed`] 时这条消息没有记为已处理，渠道可以拿同一个去重 key 再交一次。
    fn message(&self, inbound: Inbound) -> BoxFuture<'_, Result<(), HubError>>;

    /// 处理一次点击。重复的点击返回 `Ok`；不符合 `approvers` 的返回 [`HubError::Rejected`]。
    fn click(&self, click: CardClick) -> BoxFuture<'_, Result<(), HubError>>;

    /// 确认外部系统之前，把收到的原始内容可靠地存下来（返回时已落盘），返回 true。同一个 `id` 已经存过
    /// （还没处理完，或者处理完还在保留期内）时什么也不做，返回 false，渠道确认后不用再处理。
    /// 存不下来时渠道不要确认，让外部系统重推。
    fn stash(&self, id: &str, payload: &str) -> BoxFuture<'_, Result<bool, HubError>>;

    /// 存下了、还没处理完的，按收到的先后。渠道启动时先处理这些：上次确认了但没处理完就退出的。
    fn stashed(&self) -> BoxFuture<'_, Result<Vec<Stashed>, HubError>>;

    /// 记下一件事的处理结果。
    fn settle(&self, id: &str, settle: Settle) -> BoxFuture<'_, Result<(), HubError>>;
}
