//! 存储：PostgreSQL、只追加事件日志、专用写任务批量提交、中文全文搜索。
//!
//! 负责人：工程师 A。方案见 `docs/ip/postgres-migration-plan.md`（backlog D-6）。
//!
//! - **库**：只用标准 PostgreSQL，不装扩展。表都在一个 schema 里（默认 `ip`），连接时
//!   `search_path` 指向它；和团队上下文系统（schema `tcms`）共用一个库、互不碰表。
//! - **写**：所有写入经一个专用任务和一条专用连接。它把同时到达的写请求合并进一个事务提交，
//!   调用方 `await` 结果。
//! - **读**：连接池里的连接，预编译语句按连接缓存。
//! - **只追加**：每次写入先记一条事件（`events` 表），再更新投影表（如 `messages`）。

#[cfg(any(test, feature = "import-sqlite"))]
mod import;
mod schema;
mod segment;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod writer;

use std::sync::Arc;

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use ip_core::{AgentId, Approval, ApprovalId, Message, MessageId, RoleId, Task, TaskId, ThreadId};
use tokio::sync::oneshot;
use tokio_postgres::types::ToSql;
use tokio_postgres::{NoTls, Row};

#[cfg(any(test, feature = "import-sqlite"))]
pub use import::ImportReport;
pub use segment::{index_text, match_query, tokens};

/// 全库搜索时参与相关度排序的最近命中条数。SQLite 时是 2000；PostgreSQL 的 `ts_rank`
/// 逐条解析位置信息，2000 条要 7 毫秒左右，减到 1000。
pub const RANK_WINDOW: i64 = 1000;

/// 全库搜索时先只看最新的这么多条消息，见 [`Store::search`]。
pub const SEARCH_RECENT_ROWS: i64 = 10_000;

/// 事件日志里的序号，单调递增。
pub type Seq = i64;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库错误：{0}")]
    Db(#[from] tokio_postgres::Error),
    #[error("数据库连接池：{0}")]
    Pool(String),
    #[error("数据库配置：{0}")]
    Config(String),
    #[error("序列化错误：{0}")]
    Json(#[from] serde_json::Error),
    #[error("消息 id 已存在：{0}")]
    DuplicateMessage(MessageId),
    #[error("事务提交失败，本批写入全部回滚：{0}")]
    Batch(String),
    #[error("存储已关闭")]
    Closed,
    #[error("找不到{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

/// 一条搜索结果。
#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct SearchHit {
    pub message_id: MessageId,
    pub thread_id: ThreadId,
    #[ts(type = "number")]
    pub at_ms: i64,
    pub text: String,
}

/// 对话列表的一项。
#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ThreadSummary {
    pub thread_id: ThreadId,
    #[ts(type = "number")]
    pub last_at_ms: i64,
    #[ts(type = "number")]
    pub message_count: i64,
    /// 最后一条文字消息的开头。
    pub preview: Option<String>,
}

/// 提炼的结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryStatus {
    /// 有摘要。
    Ok,
    /// 提炼过了，没有值得记的。
    Empty,
    /// 提炼失败，到 `retry_after_ms` 之后再试。
    Failed,
}

impl MemoryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Ok => "ok",
            MemoryStatus::Empty => "empty",
            MemoryStatus::Failed => "failed",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "ok" => MemoryStatus::Ok,
            "empty" => MemoryStatus::Empty,
            _ => MemoryStatus::Failed,
        }
    }
}

/// 一个对话在一个岗位下的记忆摘要（`docs/ip/context-memory-design.md` 5.3）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadMemory {
    pub role_id: RoleId,
    pub thread_id: ThreadId,
    /// 提炼时这个对话最后一条消息的时间；对话之后又有新消息就要重新提炼。
    pub source_updated_ms: i64,
    pub generated_ms: i64,
    pub status: MemoryStatus,
    /// 给人看的短标题。
    pub slug: Option<String>,
    pub summary: Option<String>,
    /// 连续失败的次数；成功后归零。
    pub attempts: u32,
    pub retry_after_ms: Option<i64>,
}

/// 人要求的一条记忆更正（`docs/ip/context-memory-design.md` 5.6）：“记住……”“以后别……”“忘掉……”。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryNote {
    pub id: i64,
    pub role_id: RoleId,
    pub created_ms: i64,
    /// 在哪个对话里提的。
    pub thread_id: Option<ThreadId>,
    pub text: String,
    /// 哪一次合并用上了它；还没用上是 `None`。
    pub applied_ms: Option<i64>,
}

/// 外部会话和对话的绑定（`docs/ip/m2-spec.md` D-08）。走事件日志（`binding.put`）。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Binding {
    /// `ip.yaml` 里的渠道名。
    pub channel: String,
    /// 外部会话 key，格式由渠道决定。
    pub conversation: String,
    pub thread_id: ThreadId,
    /// 粘住的员工；路由卡片还没人选时为空。
    pub agent_id: Option<AgentId>,
    /// 开启这段会话的外部用户 id（D-07 的 `requester`）。
    pub requester: String,
    pub created_at_ms: i64,
}

/// 发出去的一样东西（D-04）：防重发，以及更新卡片时要用的外部引用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delivery {
    pub channel: String,
    /// `message:<id>`、`card:<审批 id>` 或 `resolved:<审批 id>`。
    pub item: String,
    pub conversation: String,
    /// 外部系统给的引用（Slack 消息的 ts）；失败时为空。
    pub external_ref: Option<String>,
    /// false 表示永久失败、已跳过。
    pub sent: bool,
    pub error: Option<String>,
    pub at_ms: i64,
}

/// 渠道确认之前存下的一个信封（#69），还没处理完。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Envelope {
    /// 外部系统给的 id，Slack 是 `envelope_id`。
    pub id: String,
    /// 原样的内容，由渠道自己解析。
    pub payload: String,
    /// 已经失败过几次。
    pub attempts: u32,
}

/// 一个信封处理的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Settle {
    /// 处理完了（成功，或按规则忽略）：标成 done，清掉内容，过了保留期删掉。
    Done,
    /// 这次没成，留着下次再试；失败次数加一。
    Failed(String),
    /// 重试多次仍失败，不再处理；标成 dropped 留着查，过了保留期删掉。
    GaveUp(String),
}

/// 事件日志里的一条。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredEvent {
    pub seq: Seq,
    /// `message.appended`、`approval.created` 等。
    pub kind: String,
    /// JSON；消息、任务、审批事件就是它们本身。
    pub payload: String,
}

/// 外部引擎在一个对话里的会话（`docs/ip/m2-spec.md` D-23）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineSession {
    pub agent_id: AgentId,
    pub thread_id: ThreadId,
    /// 引擎种类，如 `codex`。
    pub engine: String,
    /// 引擎自己的会话 id，下一轮用它续接。
    pub session_id: String,
    /// 已交给引擎的最后一条消息；下一轮只交它之后的新消息。
    pub last_message: MessageId,
    pub updated_at_ms: i64,
    /// 这个会话到上一轮为止累计报告的用量。续接时引擎报的是累计值，记账只记和它的差值。
    pub used: SessionUsage,
}

/// 引擎会话累计报告的 token 用量（`docs/ip/m2-spec.md` D-23）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionUsage {
    pub prompt_tokens: u32,
    pub cached_tokens: u32,
    pub cache_write_tokens: u32,
    pub completion_tokens: u32,
}

/// 连哪个库、用哪个 schema。
#[derive(Clone)]
pub struct DbConfig {
    /// 连接串，`postgres://user:password@host:5432/db`。含密码，从环境变量读，不进 `ip.yaml`。
    pub url: String,
    /// I.P. 的表放在这个 schema 里，只能是小写字母、数字和下划线。
    pub schema: String,
    /// 读连接池的大小。写入另有一条专用连接。
    pub max_connections: usize,
}

impl DbConfig {
    pub const DEFAULT_SCHEMA: &'static str = "ip";
    pub const DEFAULT_MAX_CONNECTIONS: usize = 8;

    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            schema: Self::DEFAULT_SCHEMA.to_string(),
            max_connections: Self::DEFAULT_MAX_CONNECTIONS,
        }
    }

    fn pg_config(&self) -> Result<tokio_postgres::Config> {
        let valid = !self.schema.is_empty()
            && self.schema.len() <= 63
            && self.schema.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
            && self
                .schema
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid {
            return Err(StoreError::Config(format!(
                "schema 名 {:?} 只能用小写字母、数字和下划线，不能以数字开头",
                self.schema
            )));
        }
        let mut config: tokio_postgres::Config = self
            .url
            .parse()
            .map_err(|e: tokio_postgres::Error| StoreError::Config(format!("连接串解析不了：{e}")))?;
        // plan_cache_mode：预编译语句执行几次以后 PostgreSQL 会改用不看参数值的通用计划，
        // 搜索的范围条件（`pos > $5`）这时会选错索引，p95 从 7 毫秒涨到 12 毫秒以上。每次按参数重新规划。
        config.options(format!(
            "-c search_path={} -c plan_cache_mode=force_custom_plan",
            self.schema
        ));
        if config.get_application_name().is_none() {
            config.application_name("ip");
        }
        Ok(config)
    }
}

/// 不打印连接串：里面有密码。
impl std::fmt::Debug for DbConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbConfig")
            .field("url", &"<redacted>")
            .field("schema", &self.schema)
            .field("max_connections", &self.max_connections)
            .finish()
    }
}

/// 存储的句柄，可以克隆后在多个任务间共享。
#[derive(Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

struct Inner {
    writer: writer::Handle,
    readers: Pool,
    /// 测试用的临时 schema：最后一个句柄丢掉时删掉。
    _guard: Option<Arc<dyn Send + Sync>>,
}

impl Store {
    /// 连上数据库，schema 不存在时建出来，跑完迁移，启动写任务。
    pub async fn connect(config: &DbConfig) -> Result<Self> {
        Self::connect_with_guard(config, None).await
    }

    async fn connect_with_guard(config: &DbConfig, guard: Option<Arc<dyn Send + Sync>>) -> Result<Self> {
        let pg = config.pg_config()?;
        let (mut client, connection) = pg.connect(NoTls).await?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "数据库写连接断开");
            }
        });
        schema::migrate(&mut client, &config.schema).await?;
        let manager = Manager::from_config(
            pg,
            NoTls,
            ManagerConfig {
                recycling_method: RecyclingMethod::Fast,
            },
        );
        let readers = Pool::builder(manager)
            .max_size(config.max_connections.max(1))
            .build()
            .map_err(|e| StoreError::Pool(e.to_string()))?;
        Ok(Self {
            inner: Arc::new(Inner {
                writer: writer::spawn(client, &config.schema),
                readers,
                _guard: guard,
            }),
        })
    }

    /// 追加一条消息，返回它在事件日志里的序号。
    pub async fn append_message(&self, message: Message) -> Result<Seq> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::AppendMessage { message, reply })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 新建或整体更新一个任务。
    pub async fn put_task(&self, task: Task) -> Result<Seq> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::PutTask { task, reply })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn task(&self, id: &TaskId) -> Result<Option<Task>> {
        self.json_opt("SELECT json::text FROM tasks WHERE id = $1", &[&id.as_str()])
            .await
    }

    /// 以这个对话为执行对话的任务（没有则为 `None`）。
    ///
    /// 直接扫投影表的 JSON：任务数在千级以内足够快，多了再给 `thread_id` 建列和索引。
    pub async fn task_by_thread(&self, thread: &ThreadId) -> Result<Option<Task>> {
        self.json_opt(
            "SELECT json::text FROM tasks WHERE json->>'thread_id' = $1 LIMIT 1",
            &[&thread.as_str()],
        )
        .await
    }

    /// 从这个对话派出去的任务（`origin_thread_id` 是它），按创建先后。
    pub async fn tasks_from_thread(&self, origin: &ThreadId) -> Result<Vec<Task>> {
        self.json_rows(
            "SELECT json::text FROM tasks WHERE json->>'origin_thread_id' = $1 ORDER BY id",
            &[&origin.as_str()],
        )
        .await
    }

    /// 最近更新的任务，新的在前。`open_only` 时不含已完成和已取消的。
    pub async fn tasks(&self, open_only: bool, limit: usize) -> Result<Vec<Task>> {
        let sql = if open_only {
            "SELECT json::text FROM tasks WHERE status NOT IN ('done', 'cancelled') ORDER BY updated_at_ms DESC LIMIT $1"
        } else {
            "SELECT json::text FROM tasks ORDER BY updated_at_ms DESC LIMIT $1"
        };
        self.json_rows(sql, &[&limit_param(limit)]).await
    }

    /// 写一项设置（如控制台保存的路由规则）。整体覆盖，每次写都记一条事件，可追溯历史版本。
    pub async fn put_setting(&self, key: impl Into<String>, value: serde_json::Value) -> Result<Seq> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::PutSetting {
            key: key.into(),
            value,
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn setting(&self, key: &str) -> Result<Option<serde_json::Value>> {
        self.json_opt("SELECT json::text FROM settings WHERE key = $1", &[&key])
            .await
    }

    /// 新建一张审批卡片。
    pub async fn put_approval(&self, approval: Approval) -> Result<Seq> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::PutApproval { approval, reply })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 选定审批卡片的一个选项。已经被选过时返回 `Conflict`，保证只生效一次。
    pub async fn resolve_approval(&self, id: &ApprovalId, choice: u32, by: impl Into<String>) -> Result<Approval> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::ResolveApproval {
            id: id.clone(),
            choice,
            by: by.into(),
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 审批卡片，新的在前。`pending_only` 时只含待处理的。
    pub async fn approvals(&self, pending_only: bool, limit: usize) -> Result<Vec<Approval>> {
        let sql = if pending_only {
            "SELECT json::text FROM approvals WHERE pending ORDER BY created_at_ms DESC LIMIT $1"
        } else {
            "SELECT json::text FROM approvals ORDER BY created_at_ms DESC LIMIT $1"
        };
        self.json_rows(sql, &[&limit_param(limit)]).await
    }

    /// 全文搜索，按相关度排序。`thread` 给出时只搜这个对话。
    ///
    /// 只在最近的 [`RANK_WINDOW`] 条命中里排相关度：常见词可能命中上万条，
    /// 全部算分会让延迟随历史线性增长。只搜一个对话时不限。
    ///
    /// 不限对话时先只看最新的 [`SEARCH_RECENT_ROWS`] 条消息，命中凑不够 [`RANK_WINDOW`] 条再把范围放大四倍，
    /// 直到覆盖全部历史。短语匹配要逐条读出命中消息的位置信息，常见词在全部历史里命中上万条时要十几毫秒；
    /// 这样常见词在最近的消息里就凑够了，少见的词命中本来就少，多搜几次也快。
    pub async fn search(&self, query: &str, thread: Option<&ThreadId>, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(matcher) = match_query(query) else {
            return Ok(Vec::new());
        };
        let limit = limit_param(limit);
        if let Some(thread) = thread {
            let rows = self
                .query(&search_sql(""), &[&matcher, &thread.as_str(), &limit, &i64::MAX])
                .await?;
            return Ok(rows.iter().map(search_hit).collect());
        }
        let none: Option<&str> = None;
        let last: i64 = self
            .query_one("SELECT COALESCE(MAX(pos), 0)::bigint FROM messages", &[])
            .await?
            .get(0);
        let mut rows_back = SEARCH_RECENT_ROWS;
        loop {
            if rows_back >= last {
                let rows = self
                    .query(&search_sql(""), &[&matcher, &none, &limit, &RANK_WINDOW])
                    .await?;
                return Ok(rows.iter().map(search_hit).collect());
            }
            let rows = self
                .query(
                    &search_sql("AND pos > $5"),
                    &[&matcher, &none, &limit, &RANK_WINDOW, &(last - rows_back)],
                )
                .await?;
            if rows.first().is_some_and(|row| row.get::<_, i64>(4) >= RANK_WINDOW) {
                return Ok(rows.iter().map(search_hit).collect());
            }
            rows_back = rows_back.saturating_mul(4);
        }
    }

    /// 一个对话里 `before_ms` 之前（不含）最近的 `limit` 条消息，按时间正序返回。
    pub async fn thread_messages(
        &self,
        thread: &ThreadId,
        before_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let mut messages: Vec<Message> = self
            .json_rows(
                "SELECT json::text FROM messages WHERE thread_id = $1 AND at_ms < $2 \
                 ORDER BY at_ms DESC, pos DESC LIMIT $3",
                &[&thread.as_str(), &before_ms.unwrap_or(i64::MAX), &limit_param(limit)],
            )
            .await?;
        messages.reverse();
        Ok(messages)
    }

    /// 外部会话还没有绑定时写入 `binding`；已经有了就返回已有的（D-03）。
    pub async fn claim_binding(&self, binding: Binding) -> Result<Binding> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::ClaimBinding { binding, reply })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 绑定里还没记员工时补上（D-05），返回现在的绑定；没有这个绑定时返回 `None`。
    pub async fn set_binding_agent(
        &self,
        channel: &str,
        conversation: &str,
        agent: &AgentId,
    ) -> Result<Option<Binding>> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::SetBindingAgent {
            channel: channel.to_string(),
            conversation: conversation.to_string(),
            agent: agent.clone(),
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn binding(&self, channel: &str, conversation: &str) -> Result<Option<Binding>> {
        let row = self.query_opt(BINDING_SQL, &[&channel, &conversation]).await?;
        Ok(row.map(|row| binding_from(channel, conversation, &row)))
    }

    /// 对话绑定的外部会话（一个对话最多绑一个）。
    pub async fn binding_by_thread(&self, thread: &ThreadId) -> Result<Option<Binding>> {
        let row = self
            .query_opt(
                "SELECT channel, conversation FROM channel_bindings WHERE thread_id = $1",
                &[&thread.as_str()],
            )
            .await?;
        match row {
            Some(row) => self.binding(row.get(0), row.get(1)).await,
            None => Ok(None),
        }
    }

    /// 第一次见到这个去重 key 时返回 true（D-03 第 1 步）。
    pub async fn remember_inbound(&self, channel: &str, key: &str) -> Result<bool> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::RememberInbound {
            channel: channel.to_string(),
            key: key.to_string(),
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 这个去重 key 是否已经记过。只读，不记。
    pub async fn inbound_seen(&self, channel: &str, key: &str) -> Result<bool> {
        Ok(self
            .query_opt(
                "SELECT 1 FROM channel_inbox WHERE channel = $1 AND dedupe_key = $2",
                &[&channel, &key],
            )
            .await?
            .is_some())
    }

    /// 确认外部系统之前，把收到的信封原样存下（#69）。同一个 id 已经存过（不管处理完没有）时返回 false，
    /// 不覆盖。返回时已经提交。
    pub async fn stash_envelope(&self, channel: &str, id: &str, payload: &str) -> Result<bool> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::StashEnvelope {
            channel: channel.to_string(),
            id: id.to_string(),
            payload: payload.to_string(),
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 存下了、还没处理完的信封，按收到的先后，最多 `limit` 个。
    pub async fn pending_envelopes(&self, channel: &str, limit: usize) -> Result<Vec<Envelope>> {
        let rows = self
            .query(
                "SELECT id, payload, attempts FROM channel_envelopes
                 WHERE channel = $1 AND state = 'pending' ORDER BY pos LIMIT $2",
                &[&channel, &limit_param(limit)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| Envelope {
                id: row.get(0),
                payload: row.get(1),
                attempts: to_u32(row.get(2)),
            })
            .collect())
    }

    /// 记下一个信封的处理结果。
    pub async fn settle_envelope(&self, channel: &str, id: &str, settle: Settle) -> Result<()> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::SettleEnvelope {
            channel: channel.to_string(),
            id: id.to_string(),
            settle,
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 删掉早于 `before_ms` 的去重记录和处理完、放弃了的信封，返回删了几条。
    pub async fn prune_inbound(&self, before_ms: i64) -> Result<usize> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::PruneInbound { before_ms, reply })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn put_delivery(&self, delivery: Delivery) -> Result<()> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::PutDelivery { delivery, reply })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn delivery(&self, channel: &str, item: &str) -> Result<Option<Delivery>> {
        let row = self
            .query_opt(
                "SELECT conversation, external_ref, state, error, at_ms FROM channel_deliveries
                 WHERE channel = $1 AND item = $2",
                &[&channel, &item],
            )
            .await?;
        Ok(row.map(|row| Delivery {
            channel: channel.to_string(),
            item: item.to_string(),
            conversation: row.get(0),
            external_ref: row.get(1),
            sent: row.get::<_, &str>(2) == "sent",
            error: row.get(3),
            at_ms: row.get(4),
        }))
    }

    /// 投递器读到事件日志的哪个位置；从没记过时返回 `None`。
    pub async fn cursor(&self, channel: &str) -> Result<Option<Seq>> {
        Ok(self
            .query_opt("SELECT seq FROM channel_cursors WHERE channel = $1", &[&channel])
            .await?
            .map(|row| row.get(0)))
    }

    /// 记下投递器读到的位置。不写事件：否则投递器读到自己写的事件又会推进游标，没完没了。
    pub async fn put_cursor(&self, channel: &str, seq: Seq) -> Result<()> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::PutCursor {
            channel: channel.to_string(),
            seq,
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// `after` 之后的事件，按顺序，最多 `limit` 条。
    pub async fn events_after(&self, after: Seq, limit: usize) -> Result<Vec<StoredEvent>> {
        let rows = self
            .query(
                "SELECT seq, kind, payload::text FROM events WHERE seq > $1 ORDER BY seq LIMIT $2",
                &[&after, &limit_param(limit)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| StoredEvent {
                seq: row.get(0),
                kind: row.get(1),
                payload: row.get(2),
            })
            .collect())
    }

    pub async fn approval(&self, id: &ApprovalId) -> Result<Option<Approval>> {
        self.json_opt("SELECT json::text FROM approvals WHERE id = $1", &[&id.as_str()])
            .await
    }

    /// 写入或更新某员工在某对话里的引擎会话。
    pub async fn put_engine_session(&self, session: EngineSession) -> Result<()> {
        let (reply, receive) = oneshot::channel();
        self.inner
            .writer
            .send(writer::Op::PutEngineSession { session, reply })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    pub async fn engine_session(&self, agent: &AgentId, thread: &ThreadId) -> Result<Option<EngineSession>> {
        let row = self
            .query_opt(
                "SELECT engine, session_id, last_message, updated_at_ms, used_prompt_tokens,
                        used_cached_tokens, used_cache_write_tokens, used_completion_tokens
                 FROM engine_sessions WHERE agent_id = $1 AND thread_id = $2",
                &[&agent.as_str(), &thread.as_str()],
            )
            .await?;
        Ok(row.map(|row| EngineSession {
            agent_id: agent.clone(),
            thread_id: thread.clone(),
            engine: row.get(0),
            session_id: row.get(1),
            last_message: MessageId::from(row.get::<_, String>(2)),
            updated_at_ms: row.get(3),
            used: SessionUsage {
                prompt_tokens: to_u32(row.get(4)),
                cached_tokens: to_u32(row.get(5)),
                cache_write_tokens: to_u32(row.get(6)),
                completion_tokens: to_u32(row.get(7)),
            },
        }))
    }

    /// 写入或覆盖一条对话记忆；使用计数不受影响。
    pub async fn put_thread_memory(&self, memory: ThreadMemory) -> Result<()> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::PutThreadMemory { memory, reply })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 抹掉对话记忆的内容（行留着，记成“没什么可记的”，对话没有新消息就不会再提炼）。返回抹了几条。
    /// `role` 和 `thread` 都给时抹一条；只给一个时抹这个岗位的全部，或这个对话在各岗位下的全部。
    pub async fn forget_thread_memories(&self, role: Option<&RoleId>, thread: Option<&ThreadId>) -> Result<usize> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::ForgetThreadMemories {
            role: role.cloned(),
            thread: thread.cloned(),
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 删更正笔记：给了编号删一条，没给删这个岗位的全部。返回删了几条。
    pub async fn delete_memory_notes(&self, role: &RoleId, id: Option<i64>) -> Result<usize> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::DeleteMemoryNotes {
            role: role.clone(),
            id,
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 一个岗位有摘要的对话记忆连同使用次数和最近使用时间，最近生成的在前。给控制台看。
    pub async fn thread_memories_with_usage(
        &self,
        role: &RoleId,
        limit: usize,
    ) -> Result<Vec<(ThreadMemory, u32, Option<i64>)>> {
        let rows = self
            .query(
                "SELECT role_id, thread_id, source_updated_ms, generated_ms, status, slug, summary, \
                 attempts, retry_after_ms, usage_count, last_usage_ms FROM memory_thread_summaries \
                 WHERE role_id = $1 AND summary IS NOT NULL AND status != 'empty' \
                 ORDER BY generated_ms DESC, thread_id LIMIT $2",
                &[&role.as_str(), &limit_param(limit)],
            )
            .await?;
        Ok(rows
            .iter()
            .map(|row| (thread_memory_row(row), to_u32(row.get(9)), row.get(10)))
            .collect())
    }

    /// 记一条人要求的记忆更正，返回它的编号。不写事件日志。
    pub async fn add_memory_note(
        &self,
        role: &RoleId,
        thread: Option<&ThreadId>,
        text: &str,
        at_ms: i64,
    ) -> Result<i64> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::AddMemoryNote {
            role: role.clone(),
            thread: thread.cloned(),
            text: text.to_owned(),
            at_ms,
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 一个岗位 `since_ms` 之后（含）记下的更正，旧的在前，最多取最近 `limit` 条。
    pub async fn memory_notes(&self, role: &RoleId, since_ms: i64, limit: usize) -> Result<Vec<MemoryNote>> {
        let rows = self
            .query(
                "SELECT id, role_id, created_ms, thread_id, text, applied_ms FROM memory_notes \
                 WHERE role_id = $1 AND created_ms >= $2 ORDER BY created_ms DESC, id DESC LIMIT $3",
                &[&role.as_str(), &since_ms, &limit_param(limit)],
            )
            .await?;
        let mut notes: Vec<MemoryNote> = rows
            .iter()
            .map(|row| MemoryNote {
                id: row.get(0),
                role_id: RoleId::from(row.get::<_, String>(1)),
                created_ms: row.get(2),
                thread_id: row.get::<_, Option<String>>(3).map(ThreadId::from),
                text: row.get(4),
                applied_ms: row.get(5),
            })
            .collect();
        notes.reverse();
        Ok(notes)
    }

    /// 一个岗位还没被合并用上的更正有多少条。
    pub async fn pending_memory_note_count(&self, role: &RoleId) -> Result<usize> {
        let row = self
            .query_one(
                "SELECT COUNT(*) FROM memory_notes WHERE role_id = $1 AND applied_ms IS NULL",
                &[&role.as_str()],
            )
            .await?;
        Ok(usize::try_from(row.get::<_, i64>(0)).unwrap_or(0))
    }

    /// 合并成功后，把用上的更正标成已处理。
    pub async fn apply_memory_notes(&self, role: &RoleId, ids: Vec<i64>, at_ms: i64) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::ApplyMemoryNotes {
            role: role.clone(),
            ids,
            at_ms,
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 记一次“这条对话记忆被员工读过”：使用次数加一，记下时间。合并时按它排序和淘汰。
    /// 返回这条记忆在不在。和提炼结果一样不写事件日志。
    pub async fn touch_thread_memory(&self, role: &RoleId, thread: &ThreadId, at_ms: i64) -> Result<bool> {
        let (reply, receive) = oneshot::channel();
        self.inner.writer.send(writer::Op::TouchThreadMemory {
            role: role.clone(),
            thread: thread.clone(),
            at_ms,
            reply,
        })?;
        receive.await.map_err(|_| StoreError::Closed)?
    }

    /// 一条对话记忆被读过几次、最近一次是什么时候。
    pub async fn thread_memory_usage(&self, role: &RoleId, thread: &ThreadId) -> Result<Option<(u32, Option<i64>)>> {
        Ok(self
            .query_opt(
                "SELECT usage_count, last_usage_ms FROM memory_thread_summaries WHERE role_id = $1 AND thread_id = $2",
                &[&role.as_str(), &thread.as_str()],
            )
            .await?
            .map(|row| (to_u32(row.get(0)), row.get(1))))
    }

    pub async fn thread_memory(&self, role: &RoleId, thread: &ThreadId) -> Result<Option<ThreadMemory>> {
        Ok(self
            .query_opt(
                &format!("{THREAD_MEMORY_COLUMNS} WHERE role_id = $1 AND thread_id = $2"),
                &[&role.as_str(), &thread.as_str()],
            )
            .await?
            .map(|row| thread_memory_row(&row)))
    }

    /// 一个岗位有摘要的对话记忆，最近生成的在前。
    pub async fn thread_memories(&self, role: &RoleId, limit: usize) -> Result<Vec<ThreadMemory>> {
        let rows = self
            .query(
                &format!(
                    "{THREAD_MEMORY_COLUMNS} WHERE role_id = $1 AND status = 'ok' \
                     ORDER BY generated_ms DESC, thread_id LIMIT $2"
                ),
                &[&role.as_str(), &limit_param(limit)],
            )
            .await?;
        Ok(rows.iter().map(thread_memory_row).collect())
    }

    /// 在一个岗位有摘要的对话记忆里找：`terms` 里每个词都要出现在标题或正文里（英文字母不分大小写），
    /// 最近生成的在前。没有词时返回空。
    pub async fn search_thread_memories(
        &self,
        role: &RoleId,
        terms: &[String],
        limit: usize,
    ) -> Result<Vec<ThreadMemory>> {
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        // 只折叠 ASCII 字母，和以前 SQLite 的 lower() 一样：词在 Rust 里转小写，库里用 C 排序规则的
        // lower()（只改 ASCII，也比按语言规则转换快得多），再用 strpos 找子串。
        let terms: Vec<String> = terms.iter().map(|t| t.to_ascii_lowercase()).collect();
        let mut sql = format!("{THREAD_MEMORY_COLUMNS} WHERE role_id = $1 AND status = 'ok'");
        for i in 0..terms.len() {
            let n = i + 3;
            sql.push_str(&format!(
                " AND (strpos(lower(summary COLLATE \"C\"), ${n}) > 0 \
                   OR strpos(lower(COALESCE(slug, '') COLLATE \"C\"), ${n}) > 0)"
            ));
        }
        sql.push_str(" ORDER BY generated_ms DESC, thread_id LIMIT $2");
        let role = role.as_str();
        let limit = limit_param(limit);
        let mut params: Vec<&(dyn ToSql + Sync)> = vec![&role, &limit];
        params.extend(terms.iter().map(|t| t as &(dyn ToSql + Sync)));
        let rows = self.query(&sql, &params).await?;
        Ok(rows.iter().map(thread_memory_row).collect())
    }

    /// 合并岗位记忆时选哪些对话摘要（`docs/ip/context-memory-design.md` 5.4）：有摘要的，
    /// 最近一次被用到（没被用过的看生成时间）不早于 `active_since_ms`；用得多的在前，其次是近的。
    pub async fn thread_memories_for_consolidation(
        &self,
        role: &RoleId,
        active_since_ms: i64,
        limit: usize,
    ) -> Result<Vec<ThreadMemory>> {
        let rows = self
            .query(
                &format!(
                    "{THREAD_MEMORY_COLUMNS} WHERE role_id = $1 AND summary IS NOT NULL AND status != 'empty' \
                     AND COALESCE(last_usage_ms, generated_ms) >= $2 \
                     ORDER BY usage_count DESC, COALESCE(last_usage_ms, generated_ms) DESC, thread_id LIMIT $3"
                ),
                &[&role.as_str(), &active_since_ms, &limit_param(limit)],
            )
            .await?;
        Ok(rows.iter().map(thread_memory_row).collect())
    }

    /// 最后一条消息落在 `[from_ms, until_ms]` 里的对话，新的在前。提炼记忆时用来找“凉下来”的对话。
    pub async fn threads_last_active_between(
        &self,
        from_ms: i64,
        until_ms: i64,
        limit: usize,
    ) -> Result<Vec<ThreadSummary>> {
        let rows = self
            .query(
                "SELECT id, last_at_ms, message_count, preview FROM threads \
                 WHERE last_at_ms BETWEEN $1 AND $2 ORDER BY last_at_ms DESC LIMIT $3",
                &[&from_ms, &until_ms, &limit_param(limit)],
            )
            .await?;
        Ok(rows.iter().map(thread_summary_row).collect())
    }

    /// 最近有消息的 `limit` 个对话，新的在前。
    pub async fn recent_threads(&self, limit: usize) -> Result<Vec<ThreadSummary>> {
        let rows = self
            .query(
                "SELECT id, last_at_ms, message_count, preview FROM threads ORDER BY last_at_ms DESC LIMIT $1",
                &[&limit_param(limit)],
            )
            .await?;
        Ok(rows.iter().map(thread_summary_row).collect())
    }

    /// 事件日志的最大序号；空库为 0。
    pub async fn last_seq(&self) -> Result<Seq> {
        Ok(self
            .query_one("SELECT COALESCE(MAX(seq), 0)::bigint FROM events", &[])
            .await?
            .get(0))
    }

    /// 重新统计各表的数据分布，让查询规划器选对索引。平时由 autovacuum 做；
    /// 一次写进大量数据（导入旧库、基准）之后马上要查时手动调一次。
    pub async fn analyze(&self) -> Result<()> {
        let client = self.reader().await?;
        client.batch_execute("ANALYZE").await?;
        Ok(())
    }

    /// 本 schema 里所有表（含索引）占的字节数。给基准用。
    pub async fn size_bytes(&self) -> Result<i64> {
        Ok(self
            .query_one(
                "SELECT COALESCE(SUM(pg_total_relation_size(c.oid)), 0)::bigint FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = current_schema() AND c.relkind = 'r'",
                &[],
            )
            .await?
            .get(0))
    }

    async fn reader(&self) -> Result<deadpool_postgres::Object> {
        self.inner
            .readers
            .get()
            .await
            .map_err(|e| StoreError::Pool(e.to_string()))
    }

    async fn query(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>> {
        let client = self.reader().await?;
        let statement = client.prepare_cached(sql).await?;
        Ok(client.query(&statement, params).await?)
    }

    async fn query_opt(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Option<Row>> {
        let client = self.reader().await?;
        let statement = client.prepare_cached(sql).await?;
        Ok(client.query_opt(&statement, params).await?)
    }

    async fn query_one(&self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Row> {
        let client = self.reader().await?;
        let statement = client.prepare_cached(sql).await?;
        Ok(client.query_one(&statement, params).await?)
    }

    /// 只返回一列 `json::text` 的查询，反序列化成 `T`。
    async fn json_rows<T: serde::de::DeserializeOwned>(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Vec<T>> {
        self.query(sql, params)
            .await?
            .iter()
            .map(|row| Ok(serde_json::from_str(row.get::<_, &str>(0))?))
            .collect()
    }

    async fn json_opt<T: serde::de::DeserializeOwned>(
        &self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<T>> {
        match self.query_opt(sql, params).await? {
            Some(row) => Ok(Some(serde_json::from_str(row.get::<_, &str>(0))?)),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests;

fn limit_param(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

/// 库里存的是 BIGINT；超出 u32 的按上限算（实际不会出现）。
fn to_u32(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

pub(crate) const BINDING_SQL: &str = "SELECT thread_id, agent_id, requester, created_at_ms FROM channel_bindings
     WHERE channel = $1 AND conversation = $2";

pub(crate) fn binding_from(channel: &str, conversation: &str, row: &Row) -> Binding {
    Binding {
        channel: channel.to_string(),
        conversation: conversation.to_string(),
        thread_id: ThreadId::from(row.get::<_, String>(0)),
        agent_id: row.get::<_, Option<String>>(1).map(AgentId::from),
        requester: row.get(2),
        created_at_ms: row.get(3),
    }
}

/// 搜索语句：`$1` 查询、`$2` 对话（可空）、`$3` 返回几条、`$4` 排序窗口；`extra` 是额外条件。
/// 第 5 列是窗口里一共有几条命中。
fn search_sql(extra: &str) -> String {
    format!(
        "WITH f AS ( \
             SELECT pos, id, thread_id, at_ms, text, search FROM messages \
             WHERE search @@ $1::text::tsquery AND ($2::text IS NULL OR thread_id = $2) {extra} \
             ORDER BY pos DESC LIMIT $4 \
         ) \
         SELECT id, thread_id, at_ms, text, (SELECT COUNT(*) FROM f) FROM f \
         ORDER BY ts_rank(search, $1::text::tsquery) DESC, pos DESC LIMIT $3"
    )
}

fn search_hit(row: &Row) -> SearchHit {
    SearchHit {
        message_id: MessageId::from(row.get::<_, String>(0)),
        thread_id: ThreadId::from(row.get::<_, String>(1)),
        at_ms: row.get(2),
        text: row.get(3),
    }
}

fn thread_summary_row(row: &Row) -> ThreadSummary {
    ThreadSummary {
        thread_id: ThreadId::from(row.get::<_, String>(0)),
        last_at_ms: row.get(1),
        message_count: row.get(2),
        preview: row.get(3),
    }
}

const THREAD_MEMORY_COLUMNS: &str = "SELECT role_id, thread_id, source_updated_ms, generated_ms, status, slug, summary, \
     attempts, retry_after_ms FROM memory_thread_summaries";

fn thread_memory_row(row: &Row) -> ThreadMemory {
    ThreadMemory {
        role_id: RoleId::from(row.get::<_, String>(0).as_str()),
        thread_id: ThreadId::from(row.get::<_, String>(1)),
        source_updated_ms: row.get(2),
        generated_ms: row.get(3),
        status: MemoryStatus::parse(row.get(4)),
        slug: row.get(5),
        summary: row.get(6),
        attempts: to_u32(row.get(7)),
        retry_after_ms: row.get(8),
    }
}
