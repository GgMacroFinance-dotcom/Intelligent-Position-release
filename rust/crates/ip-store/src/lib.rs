//! 存储：SQLite、只追加事件日志、专用写线程批量提交、中文全文搜索。
//!
//! 负责人：工程师 A。
//!
//! - **写**：所有写入经一个专用线程。它把同时到达的写请求合并进一个事务提交，
//!   调用方在异步代码里 `await` 结果，不会阻塞 tokio 运行时。
//! - **读**：放在 `spawn_blocking` 线程上，用连接池里的只读连接，WAL 模式下不挡写。
//! - **只追加**：每次写入先记一条事件（`events` 表），再更新投影表（如 `messages`）。
//!   不存在整文件重写。

mod schema;
mod segment;
mod writer;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use ip_core::{AgentId, Approval, ApprovalId, Message, MessageId, RoleId, Task, TaskId, ThreadId};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use tokio::sync::oneshot;

pub use segment::{index_text, match_query};

/// 全库搜索时参与相关度排序的最近命中条数。
pub const RANK_WINDOW: i64 = 2000;

/// 事件日志里的序号，单调递增。
pub type Seq = i64;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("数据库错误：{0}")]
    Sqlite(#[from] rusqlite::Error),
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

/// 存储的句柄，可以克隆后在多个任务间共享。
#[derive(Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    writer: writer::Handle,
    readers: Mutex<Vec<Connection>>,
}

impl Store {
    /// 打开（必要时新建）数据库文件，启动写线程。
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let connection = Connection::open(&path)?;
        schema::configure(&connection)?;
        schema::migrate(&connection)?;
        let writer = writer::spawn(connection)?;
        Ok(Self {
            inner: Arc::new(Inner {
                path,
                writer,
                readers: Mutex::new(Vec::new()),
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
        let id = id.clone();
        self.read(move |c| {
            let json: Option<String> = c
                .prepare_cached("SELECT json FROM tasks WHERE id = ?1")?
                .query_row(params![id.as_str()], |row| row.get(0))
                .optional()?;
            Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
        })
        .await
    }

    /// 以这个对话为执行对话的任务（没有则为 `None`）。
    ///
    /// 直接扫投影表的 JSON：任务数在千级以内足够快，多了再给 `thread_id` 建列和索引。
    pub async fn task_by_thread(&self, thread: &ThreadId) -> Result<Option<Task>> {
        let thread = thread.clone();
        self.read(move |c| {
            let json: Option<String> = c
                .prepare_cached("SELECT json FROM tasks WHERE json_extract(json, '$.thread_id') = ?1 LIMIT 1")?
                .query_row(params![thread.as_str()], |row| row.get(0))
                .optional()?;
            Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
        })
        .await
    }

    /// 从这个对话派出去的任务（`origin_thread_id` 是它），按创建先后。
    pub async fn tasks_from_thread(&self, origin: &ThreadId) -> Result<Vec<Task>> {
        let origin = origin.clone();
        self.read(move |c| {
            let mut stmt = c.prepare_cached(
                "SELECT json FROM tasks WHERE json_extract(json, '$.origin_thread_id') = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map(params![origin.as_str()], |row| row.get::<_, String>(0))?;
            let mut out = Vec::new();
            for json in rows {
                out.push(serde_json::from_str(&json?)?);
            }
            Ok(out)
        })
        .await
    }

    /// 最近更新的任务，新的在前。`open_only` 时不含已完成和已取消的。
    pub async fn tasks(&self, open_only: bool, limit: usize) -> Result<Vec<Task>> {
        self.read(move |c| {
            let sql = if open_only {
                "SELECT json FROM tasks WHERE status NOT IN ('done', 'cancelled') ORDER BY updated_at_ms DESC LIMIT ?1"
            } else {
                "SELECT json FROM tasks ORDER BY updated_at_ms DESC LIMIT ?1"
            };
            json_rows(c, sql, limit)
        })
        .await
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
        let key = key.to_string();
        self.read(move |c| {
            let json: Option<String> = c
                .prepare_cached("SELECT json FROM settings WHERE key = ?1")?
                .query_row(params![key], |row| row.get(0))
                .optional()?;
            Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
        })
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
        self.read(move |c| {
            let sql = if pending_only {
                "SELECT json FROM approvals WHERE pending = 1 ORDER BY created_at_ms DESC LIMIT ?1"
            } else {
                "SELECT json FROM approvals ORDER BY created_at_ms DESC LIMIT ?1"
            };
            json_rows(c, sql, limit)
        })
        .await
    }

    /// 全文搜索，按相关度排序。`thread` 给出时只搜这个对话。
    ///
    /// 不限对话时，只在最近的 [`RANK_WINDOW`] 条命中里排相关度：常见词可能命中上万条，
    /// 全部算分会让延迟随历史线性增长。
    pub async fn search(&self, query: &str, thread: Option<&ThreadId>, limit: usize) -> Result<Vec<SearchHit>> {
        let Some(matcher) = match_query(query) else {
            return Ok(Vec::new());
        };
        let thread = thread.cloned();
        self.read(move |connection| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let mut statement = connection.prepare_cached(
                "SELECT m.id, m.thread_id, m.at_ms, m.text FROM ( \
                     SELECT rowid, rank FROM messages_fts WHERE messages_fts MATCH ?1 \
                     ORDER BY rowid DESC LIMIT ?4 \
                 ) f JOIN messages m ON m.rowid = f.rowid \
                 WHERE ?2 IS NULL OR m.thread_id = ?2 \
                 ORDER BY f.rank LIMIT ?3",
            )?;
            let window = if thread.is_some() { i64::MAX } else { RANK_WINDOW };
            let rows = statement.query_map(
                params![matcher, thread.as_ref().map(ThreadId::as_str), limit, window],
                |row| {
                    Ok(SearchHit {
                        message_id: MessageId::from(row.get::<_, String>(0)?),
                        thread_id: ThreadId::from(row.get::<_, String>(1)?),
                        at_ms: row.get(2)?,
                        text: row.get(3)?,
                    })
                },
            )?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
    }

    /// 一个对话里 `before_ms` 之前（不含）最近的 `limit` 条消息，按时间正序返回。
    pub async fn thread_messages(
        &self,
        thread: &ThreadId,
        before_ms: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Message>> {
        let thread = thread.clone();
        self.read(move |connection| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let mut statement = connection.prepare_cached(
                "SELECT json FROM messages WHERE thread_id = ?1 AND at_ms < ?2 \
                 ORDER BY at_ms DESC, rowid DESC LIMIT ?3",
            )?;
            let rows = statement
                .query_map(params![thread.as_str(), before_ms.unwrap_or(i64::MAX), limit], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let mut messages = rows
                .iter()
                .map(|json| serde_json::from_str(json))
                .collect::<serde_json::Result<Vec<Message>>>()?;
            messages.reverse();
            Ok(messages)
        })
        .await
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
        let (channel, conversation) = (channel.to_string(), conversation.to_string());
        self.read(move |c| binding_row(c, &channel, &conversation)).await
    }

    /// 对话绑定的外部会话（一个对话最多绑一个）。
    pub async fn binding_by_thread(&self, thread: &ThreadId) -> Result<Option<Binding>> {
        let thread = thread.clone();
        self.read(move |c| {
            let key: Option<(String, String)> = c
                .prepare_cached("SELECT channel, conversation FROM channel_bindings WHERE thread_id = ?1")?
                .query_row(params![thread.as_str()], |row| Ok((row.get(0)?, row.get(1)?)))
                .optional()?;
            match key {
                Some((channel, conversation)) => binding_row(c, &channel, &conversation),
                None => Ok(None),
            }
        })
        .await
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
        let (channel, key) = (channel.to_string(), key.to_string());
        self.read(move |c| {
            Ok(
                c.prepare_cached("SELECT 1 FROM channel_inbox WHERE channel = ?1 AND dedupe_key = ?2")?
                    .query_row(params![channel, key], |_| Ok(()))
                    .optional()?
                    .is_some(),
            )
        })
        .await
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
        let channel = channel.to_string();
        self.read(move |c| {
            let mut statement = c.prepare_cached(
                "SELECT id, payload, attempts FROM channel_envelopes
                 WHERE channel = ?1 AND state = 'pending' ORDER BY rowid LIMIT ?2",
            )?;
            let rows = statement.query_map(params![channel, limit as i64], |row| {
                Ok(Envelope {
                    id: row.get(0)?,
                    payload: row.get(1)?,
                    attempts: row.get(2)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
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
        let (channel, item) = (channel.to_string(), item.to_string());
        self.read(move |c| {
            Ok(c.prepare_cached(
                "SELECT conversation, external_ref, state, error, at_ms FROM channel_deliveries
                 WHERE channel = ?1 AND item = ?2",
            )?
            .query_row(params![channel, item], |row| {
                Ok(Delivery {
                    channel: channel.clone(),
                    item: item.clone(),
                    conversation: row.get(0)?,
                    external_ref: row.get(1)?,
                    sent: row.get::<_, String>(2)? == "sent",
                    error: row.get(3)?,
                    at_ms: row.get(4)?,
                })
            })
            .optional()?)
        })
        .await
    }

    /// 投递器读到事件日志的哪个位置；从没记过时返回 `None`。
    pub async fn cursor(&self, channel: &str) -> Result<Option<Seq>> {
        let channel = channel.to_string();
        self.read(move |c| {
            Ok(c.prepare_cached("SELECT seq FROM channel_cursors WHERE channel = ?1")?
                .query_row(params![channel], |row| row.get(0))
                .optional()?)
        })
        .await
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
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.read(move |c| {
            let mut statement =
                c.prepare_cached("SELECT seq, kind, payload FROM events WHERE seq > ?1 ORDER BY seq LIMIT ?2")?;
            let rows = statement.query_map(params![after, limit], |row| {
                Ok(StoredEvent {
                    seq: row.get(0)?,
                    kind: row.get(1)?,
                    payload: row.get(2)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
        })
        .await
    }

    pub async fn approval(&self, id: &ApprovalId) -> Result<Option<Approval>> {
        let id = id.clone();
        self.read(move |c| {
            let json: Option<String> = c
                .prepare_cached("SELECT json FROM approvals WHERE id = ?1")?
                .query_row(params![id.as_str()], |row| row.get(0))
                .optional()?;
            Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
        })
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
        let (agent, thread) = (agent.clone(), thread.clone());
        self.read(move |c| {
            Ok(c.prepare_cached(
                "SELECT engine, session_id, last_message, updated_at_ms, used_prompt_tokens,
                        used_cached_tokens, used_cache_write_tokens, used_completion_tokens
                 FROM engine_sessions WHERE agent_id = ?1 AND thread_id = ?2",
            )?
            .query_row(params![agent.as_str(), thread.as_str()], |row| {
                Ok(EngineSession {
                    agent_id: agent.clone(),
                    thread_id: thread.clone(),
                    engine: row.get(0)?,
                    session_id: row.get(1)?,
                    last_message: MessageId::from(row.get::<_, String>(2)?),
                    updated_at_ms: row.get(3)?,
                    used: SessionUsage {
                        prompt_tokens: row.get(4)?,
                        cached_tokens: row.get(5)?,
                        cache_write_tokens: row.get(6)?,
                        completion_tokens: row.get(7)?,
                    },
                })
            })
            .optional()?)
        })
        .await
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
        let role = role.clone();
        self.read(move |c| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let rows = c
                .prepare_cached(
                    "SELECT role_id, thread_id, source_updated_ms, generated_ms, status, slug, summary, \
                     attempts, retry_after_ms, usage_count, last_usage_ms FROM memory_thread_summaries \
                     WHERE role_id = ?1 AND summary IS NOT NULL AND status != 'empty' \
                     ORDER BY generated_ms DESC, thread_id LIMIT ?2",
                )?
                .query_map(params![role.as_str(), limit], |row| {
                    Ok((thread_memory_row(row)?, row.get(9)?, row.get(10)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
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
        let role = role.clone();
        self.read(move |c| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let mut rows = c
                .prepare_cached(
                    "SELECT id, role_id, created_ms, thread_id, text, applied_ms FROM memory_notes \
                     WHERE role_id = ?1 AND created_ms >= ?2 ORDER BY created_ms DESC, id DESC LIMIT ?3",
                )?
                .query_map(params![role.as_str(), since_ms, limit], |row| {
                    Ok(MemoryNote {
                        id: row.get(0)?,
                        role_id: RoleId::from(row.get::<_, String>(1)?),
                        created_ms: row.get(2)?,
                        thread_id: row.get::<_, Option<String>>(3)?.map(ThreadId::from),
                        text: row.get(4)?,
                        applied_ms: row.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            rows.reverse();
            Ok(rows)
        })
        .await
    }

    /// 一个岗位还没被合并用上的更正有多少条。
    pub async fn pending_memory_note_count(&self, role: &RoleId) -> Result<usize> {
        let role = role.clone();
        self.read(move |c| {
            let n: i64 = c
                .prepare_cached("SELECT COUNT(*) FROM memory_notes WHERE role_id = ?1 AND applied_ms IS NULL")?
                .query_row(params![role.as_str()], |row| row.get(0))?;
            Ok(usize::try_from(n).unwrap_or(0))
        })
        .await
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
        let (role, thread) = (role.clone(), thread.clone());
        self.read(move |c| {
            Ok(c.prepare_cached(
                "SELECT usage_count, last_usage_ms FROM memory_thread_summaries WHERE role_id = ?1 AND thread_id = ?2",
            )?
            .query_row(params![role.as_str(), thread.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })
            .optional()?)
        })
        .await
    }

    pub async fn thread_memory(&self, role: &RoleId, thread: &ThreadId) -> Result<Option<ThreadMemory>> {
        let (role, thread) = (role.clone(), thread.clone());
        self.read(move |c| {
            Ok(c.prepare_cached(&format!(
                "{THREAD_MEMORY_COLUMNS} WHERE role_id = ?1 AND thread_id = ?2"
            ))?
            .query_row(params![role.as_str(), thread.as_str()], thread_memory_row)
            .optional()?)
        })
        .await
    }

    /// 一个岗位有摘要的对话记忆，最近生成的在前。
    pub async fn thread_memories(&self, role: &RoleId, limit: usize) -> Result<Vec<ThreadMemory>> {
        let role = role.clone();
        self.read(move |c| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let rows = c
                .prepare_cached(&format!(
                    "{THREAD_MEMORY_COLUMNS} WHERE role_id = ?1 AND status = 'ok' \
                     ORDER BY generated_ms DESC, thread_id LIMIT ?2"
                ))?
                .query_map(params![role.as_str(), limit], thread_memory_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
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
        let role = role.clone();
        let terms: Vec<String> = terms.iter().map(|t| t.to_ascii_lowercase()).collect();
        self.read(move |c| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let mut sql = format!("{THREAD_MEMORY_COLUMNS} WHERE role_id = ?1 AND status = 'ok'");
            for i in 0..terms.len() {
                let n = i + 3;
                sql.push_str(&format!(
                    " AND (instr(lower(summary), ?{n}) > 0 OR instr(lower(COALESCE(slug, '')), ?{n}) > 0)"
                ));
            }
            sql.push_str(" ORDER BY generated_ms DESC, thread_id LIMIT ?2");
            let mut values: Vec<rusqlite::types::Value> = vec![role.as_str().to_owned().into(), limit.into()];
            values.extend(terms.into_iter().map(Into::into));
            let rows = c
                .prepare_cached(&sql)?
                .query_map(rusqlite::params_from_iter(values), thread_memory_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    /// 合并岗位记忆时选哪些对话摘要（`docs/ip/context-memory-design.md` 5.4）：有摘要的，
    /// 最近一次被用到（没被用过的看生成时间）不早于 `active_since_ms`；用得多的在前，其次是近的。
    pub async fn thread_memories_for_consolidation(
        &self,
        role: &RoleId,
        active_since_ms: i64,
        limit: usize,
    ) -> Result<Vec<ThreadMemory>> {
        let role = role.clone();
        self.read(move |c| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let rows = c
                .prepare_cached(&format!(
                    "{THREAD_MEMORY_COLUMNS} WHERE role_id = ?1 AND summary IS NOT NULL AND status != 'empty' \
                     AND COALESCE(last_usage_ms, generated_ms) >= ?2 \
                     ORDER BY usage_count DESC, COALESCE(last_usage_ms, generated_ms) DESC, thread_id LIMIT ?3"
                ))?
                .query_map(params![role.as_str(), active_since_ms, limit], thread_memory_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    /// 最后一条消息落在 `[from_ms, until_ms]` 里的对话，新的在前。提炼记忆时用来找“凉下来”的对话。
    pub async fn threads_last_active_between(
        &self,
        from_ms: i64,
        until_ms: i64,
        limit: usize,
    ) -> Result<Vec<ThreadSummary>> {
        self.read(move |connection| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let mut statement = connection.prepare_cached(
                "SELECT id, last_at_ms, message_count, preview FROM threads \
                 WHERE last_at_ms BETWEEN ?1 AND ?2 ORDER BY last_at_ms DESC LIMIT ?3",
            )?;
            let rows = statement
                .query_map(params![from_ms, until_ms, limit], |row| {
                    Ok(ThreadSummary {
                        thread_id: ThreadId::from(row.get::<_, String>(0)?),
                        last_at_ms: row.get(1)?,
                        message_count: row.get(2)?,
                        preview: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    /// 最近有消息的 `limit` 个对话，新的在前。
    pub async fn recent_threads(&self, limit: usize) -> Result<Vec<ThreadSummary>> {
        self.read(move |connection| {
            let limit = i64::try_from(limit).unwrap_or(i64::MAX);
            let mut statement = connection.prepare_cached(
                "SELECT id, last_at_ms, message_count, preview FROM threads ORDER BY last_at_ms DESC LIMIT ?1",
            )?;
            let rows = statement
                .query_map(params![limit], |row| {
                    Ok(ThreadSummary {
                        thread_id: ThreadId::from(row.get::<_, String>(0)?),
                        last_at_ms: row.get(1)?,
                        message_count: row.get(2)?,
                        preview: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
        .await
    }

    /// 事件日志的最大序号；空库为 0。
    pub async fn last_seq(&self) -> Result<Seq> {
        self.read(|connection| {
            Ok(connection.query_row("SELECT COALESCE(MAX(seq), 0) FROM events", [], |row| row.get(0))?)
        })
        .await
    }

    async fn read<T, F>(&self, work: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T> + Send + 'static,
    {
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let connection = inner.take_reader()?;
            let result = work(&connection);
            inner.return_reader(connection);
            result
        })
        .await
        .map_err(|_| StoreError::Closed)?
    }
}

impl Inner {
    fn take_reader(&self) -> Result<Connection> {
        if let Some(connection) = self
            .readers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop()
        {
            return Ok(connection);
        }
        let connection = Connection::open_with_flags(
            &self.path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
        )?;
        schema::configure_reader(&connection)?;
        Ok(connection)
    }

    fn return_reader(&self, connection: Connection) {
        self.readers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(connection);
    }
}

#[cfg(test)]
mod tests;

/// 执行只有一个 `LIMIT ?1` 参数、返回 `json` 列的查询并反序列化。
pub(crate) fn binding_row(c: &Connection, channel: &str, conversation: &str) -> Result<Option<Binding>> {
    Ok(c.prepare_cached(
        "SELECT thread_id, agent_id, requester, created_at_ms FROM channel_bindings
         WHERE channel = ?1 AND conversation = ?2",
    )?
    .query_row(params![channel, conversation], |row| {
        Ok(Binding {
            channel: channel.to_string(),
            conversation: conversation.to_string(),
            thread_id: ThreadId::from(row.get::<_, String>(0)?),
            agent_id: row.get::<_, Option<String>>(1)?.map(AgentId::from),
            requester: row.get(2)?,
            created_at_ms: row.get(3)?,
        })
    })
    .optional()?)
}

fn json_rows<T: serde::de::DeserializeOwned>(c: &Connection, sql: &str, limit: usize) -> Result<Vec<T>> {
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let mut statement = c.prepare_cached(sql)?;
    let rows = statement
        .query_map(params![limit], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .iter()
        .map(|j| serde_json::from_str(j))
        .collect::<serde_json::Result<Vec<T>>>()?)
}

const THREAD_MEMORY_COLUMNS: &str = "SELECT role_id, thread_id, source_updated_ms, generated_ms, status, slug, summary, \
     attempts, retry_after_ms FROM memory_thread_summaries";

fn thread_memory_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ThreadMemory> {
    Ok(ThreadMemory {
        role_id: RoleId::from(row.get::<_, String>(0)?.as_str()),
        thread_id: ThreadId::from(row.get::<_, String>(1)?),
        source_updated_ms: row.get(2)?,
        generated_ms: row.get(3)?,
        status: MemoryStatus::parse(&row.get::<_, String>(4)?),
        slug: row.get(5)?,
        summary: row.get(6)?,
        attempts: row.get(7)?,
        retry_after_ms: row.get(8)?,
    })
}
