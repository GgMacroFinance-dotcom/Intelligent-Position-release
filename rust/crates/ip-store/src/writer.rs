//! 专用写任务。一次取出所有已排队的写请求，放进同一个事务；每个请求有自己的保存点，
//! 一个请求失败（比如 id 重复）只回滚它自己。
//!
//! 每个事务开头取一把事务级 advisory lock（[`schema::LOCK_EVENTS`]），提交时自动释放。
//! 这样即使另一个进程（`ip context`、`ip device pair`）同时在写同一个库，事件的 seq 也按提交顺序递增，
//! 按 seq 读事件日志的渠道投递器不会漏掉晚提交、seq 却更小的事件。

use std::collections::HashMap;
use std::sync::Mutex;

use ip_core::{Approval, ApprovalId, ApprovalState, Message, Task};
use tokio::sync::{mpsc, oneshot};
use tokio_postgres::error::SqlState;
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, Row, Statement, Transaction};

use crate::{Binding, Delivery, EngineSession, Result, Seq, StoreError, schema, segment};

/// 单个事务最多合并的请求数，避免一个事务持锁太久。
const MAX_BATCH: usize = 512;

pub enum Op {
    AppendMessage {
        message: Message,
        reply: oneshot::Sender<Result<Seq>>,
    },
    PutTask {
        task: Task,
        reply: oneshot::Sender<Result<Seq>>,
    },
    PutApproval {
        approval: Approval,
        reply: oneshot::Sender<Result<Seq>>,
    },
    PutSetting {
        key: String,
        value: serde_json::Value,
        reply: oneshot::Sender<Result<Seq>>,
    },
    ResolveApproval {
        id: ApprovalId,
        choice: u32,
        by: String,
        reply: oneshot::Sender<Result<Approval>>,
    },
    PutEngineSession {
        session: EngineSession,
        reply: oneshot::Sender<Result<()>>,
    },
    PutThreadMemory {
        memory: crate::ThreadMemory,
        reply: oneshot::Sender<Result<()>>,
    },
    AddMemoryNote {
        role: ip_core::RoleId,
        thread: Option<ip_core::ThreadId>,
        text: String,
        at_ms: i64,
        reply: oneshot::Sender<Result<i64>>,
    },
    ApplyMemoryNotes {
        role: ip_core::RoleId,
        ids: Vec<i64>,
        at_ms: i64,
        reply: oneshot::Sender<Result<()>>,
    },
    /// 把对话记忆的内容抹掉但留着这一行（当作“提炼过、没什么可记的”），免得下一轮又提炼出来。
    /// `role` 和 `thread` 都给时抹一条；只给一个时抹这个岗位的全部或这个对话在各岗位下的全部。
    ForgetThreadMemories {
        role: Option<ip_core::RoleId>,
        thread: Option<ip_core::ThreadId>,
        reply: oneshot::Sender<Result<usize>>,
    },
    /// 删更正笔记：给了编号删一条，没给删这个岗位的全部。
    DeleteMemoryNotes {
        role: ip_core::RoleId,
        id: Option<i64>,
        reply: oneshot::Sender<Result<usize>>,
    },
    TouchThreadMemory {
        role: ip_core::RoleId,
        thread: ip_core::ThreadId,
        at_ms: i64,
        reply: oneshot::Sender<Result<bool>>,
    },
    ClaimBinding {
        binding: Binding,
        reply: oneshot::Sender<Result<Binding>>,
    },
    SetBindingAgent {
        channel: String,
        conversation: String,
        agent: ip_core::AgentId,
        reply: oneshot::Sender<Result<Option<Binding>>>,
    },
    RememberInbound {
        channel: String,
        key: String,
        reply: oneshot::Sender<Result<bool>>,
    },
    PruneInbound {
        before_ms: i64,
        reply: oneshot::Sender<Result<usize>>,
    },
    StashEnvelope {
        channel: String,
        id: String,
        payload: String,
        reply: oneshot::Sender<Result<bool>>,
    },
    SettleEnvelope {
        channel: String,
        id: String,
        settle: crate::Settle,
        reply: oneshot::Sender<Result<()>>,
    },
    PutDelivery {
        delivery: Delivery,
        reply: oneshot::Sender<Result<()>>,
    },
    PutCursor {
        channel: String,
        seq: Seq,
        reply: oneshot::Sender<Result<()>>,
    },
}

pub struct Handle {
    sender: mpsc::UnboundedSender<Op>,
}

impl Handle {
    pub fn send(&self, op: Op) -> Result<()> {
        self.sender.send(op).map_err(|_| StoreError::Closed)
    }
}

/// 启动写任务。所有 `Handle` 都被丢弃后任务结束，连接随之关闭。
pub fn spawn(client: Client, schema: &str) -> Handle {
    let (sender, receiver) = mpsc::unbounded_channel();
    let lock = format!(
        "SELECT pg_advisory_xact_lock({}, {})",
        schema::LOCK_SPACE,
        schema::lock_key(schema, schema::LOCK_EVENTS)
    );
    tokio::spawn(run(client, receiver, lock));
    Handle { sender }
}

async fn run(mut client: Client, mut receiver: mpsc::UnboundedReceiver<Op>, lock: String) {
    let cache = Cache::default();
    while let Some(first) = receiver.recv().await {
        let mut batch = vec![first];
        while batch.len() < MAX_BATCH {
            match receiver.try_recv() {
                Ok(op) => batch.push(op),
                Err(_) => break,
            }
        }
        commit(&mut client, &cache, &lock, batch).await;
    }
}

/// 写任务自己的预编译语句缓存。语句属于这条连接，事务和保存点里都能用。
#[derive(Default)]
struct Cache(Mutex<HashMap<&'static str, Statement>>);

/// 一个事务（或保存点）加上语句缓存。
struct Tx<'a, 'b> {
    t: &'a Transaction<'b>,
    cache: &'a Cache,
}

impl Tx<'_, '_> {
    async fn statement(&self, sql: &'static str) -> Result<Statement> {
        if let Some(statement) = self
            .cache
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(sql)
        {
            return Ok(statement.clone());
        }
        let statement = self.t.prepare(sql).await?;
        self.cache
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(sql, statement.clone());
        Ok(statement)
    }

    async fn execute(&self, sql: &'static str, params: &[&(dyn ToSql + Sync)]) -> Result<u64> {
        let statement = self.statement(sql).await?;
        Ok(self.t.execute(&statement, params).await?)
    }

    /// 和 `execute` 一样，但把数据库错误原样交回，调用方可以认出唯一约束冲突。
    async fn execute_raw(
        &self,
        sql: &'static str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<std::result::Result<u64, tokio_postgres::Error>> {
        let statement = self.statement(sql).await?;
        Ok(self.t.execute(&statement, params).await)
    }

    async fn query_one(&self, sql: &'static str, params: &[&(dyn ToSql + Sync)]) -> Result<Row> {
        let statement = self.statement(sql).await?;
        Ok(self.t.query_one(&statement, params).await?)
    }

    async fn query_opt(&self, sql: &'static str, params: &[&(dyn ToSql + Sync)]) -> Result<Option<Row>> {
        let statement = self.statement(sql).await?;
        Ok(self.t.query_opt(&statement, params).await?)
    }
}

/// 事务提交后把结果交给调用方；参数是整批失败时的原因。
type Finish = Box<dyn FnOnce(Option<&str>) + Send>;

fn finish<T: Send + 'static>(reply: oneshot::Sender<Result<T>>, result: Result<T>) -> Finish {
    Box::new(move |batch_error| {
        let _ = reply.send(match batch_error {
            None => result,
            Some(message) => Err(StoreError::Batch(message.to_string())),
        });
    })
}

/// 在保存点里执行一个请求：失败只回滚它自己。
macro_rules! step {
    ($transaction:expr, $cache:expr, $finishes:expr, $reply:expr, |$c:ident| $body:expr) => {{
        let savepoint = $transaction.savepoint("op").await?;
        let result = {
            let $c = Tx {
                t: &savepoint,
                cache: $cache,
            };
            $body.await
        };
        if result.is_ok() {
            savepoint.commit().await?;
        } else {
            savepoint.rollback().await?;
        }
        $finishes.push(finish($reply, result));
    }};
}

async fn commit(client: &mut Client, cache: &Cache, lock: &str, batch: Vec<Op>) {
    let mut finishes: Vec<Finish> = Vec::with_capacity(batch.len());
    let outcome = run_batch(client, cache, lock, batch, &mut finishes).await;
    let error = outcome.err().map(|e| e.to_string());
    for finish in finishes {
        finish(error.as_deref());
    }
}

async fn run_batch(
    client: &mut Client,
    cache: &Cache,
    lock: &str,
    batch: Vec<Op>,
    finishes: &mut Vec<Finish>,
) -> std::result::Result<(), tokio_postgres::Error> {
    let mut transaction = client.transaction().await?;
    transaction.batch_execute(lock).await?;
    for op in batch {
        match op {
            Op::AppendMessage { message, reply } => {
                step!(transaction, cache, finishes, reply, |c| append_message(&c, &message))
            }
            Op::PutTask { task, reply } => step!(transaction, cache, finishes, reply, |c| put_task(&c, &task)),
            Op::PutApproval { approval, reply } => {
                step!(transaction, cache, finishes, reply, |c| put_approval(&c, &approval))
            }
            Op::PutSetting { key, value, reply } => {
                step!(transaction, cache, finishes, reply, |c| put_setting(&c, &key, &value))
            }
            Op::ResolveApproval { id, choice, by, reply } => {
                step!(transaction, cache, finishes, reply, |c| resolve_approval(
                    &c, &id, choice, &by
                ))
            }
            Op::PutEngineSession { session, reply } => {
                step!(transaction, cache, finishes, reply, |c| put_engine_session(
                    &c, &session
                ))
            }
            Op::PutThreadMemory { memory, reply } => {
                step!(transaction, cache, finishes, reply, |c| put_thread_memory(&c, &memory))
            }
            Op::AddMemoryNote {
                role,
                thread,
                text,
                at_ms,
                reply,
            } => step!(transaction, cache, finishes, reply, |c| async {
                let row = c
                    .query_one(
                        "INSERT INTO memory_notes (role_id, created_ms, thread_id, text) VALUES ($1, $2, $3, $4)
                         RETURNING id",
                        &[
                            &role.as_str(),
                            &at_ms,
                            &thread.as_ref().map(ip_core::ThreadId::as_str),
                            &clean_text(&text),
                        ],
                    )
                    .await?;
                Ok(row.get::<_, i64>(0))
            }),
            Op::ApplyMemoryNotes {
                role,
                ids,
                at_ms,
                reply,
            } => step!(transaction, cache, finishes, reply, |c| async {
                c.execute(
                    "UPDATE memory_notes SET applied_ms = $3
                     WHERE role_id = $1 AND id = ANY($2) AND applied_ms IS NULL",
                    &[&role.as_str(), &ids, &at_ms],
                )
                .await?;
                Ok(())
            }),
            Op::ForgetThreadMemories { role, thread, reply } => {
                step!(transaction, cache, finishes, reply, |c| async {
                    if role.is_none() && thread.is_none() {
                        return Ok(0);
                    }
                    let n = c
                        .execute(
                            "UPDATE memory_thread_summaries
                             SET status = 'empty', summary = NULL, slug = NULL, attempts = 0, retry_after_ms = NULL
                             WHERE ($1::text IS NULL OR role_id = $1) AND ($2::text IS NULL OR thread_id = $2)
                               AND summary IS NOT NULL",
                            &[
                                &role.as_ref().map(ip_core::RoleId::as_str),
                                &thread.as_ref().map(ip_core::ThreadId::as_str),
                            ],
                        )
                        .await?;
                    Ok(count(n))
                })
            }
            Op::DeleteMemoryNotes { role, id, reply } => step!(transaction, cache, finishes, reply, |c| async {
                let n = c
                    .execute(
                        "DELETE FROM memory_notes WHERE role_id = $1 AND ($2::bigint IS NULL OR id = $2)",
                        &[&role.as_str(), &id],
                    )
                    .await?;
                Ok(count(n))
            }),
            Op::TouchThreadMemory {
                role,
                thread,
                at_ms,
                reply,
            } => step!(transaction, cache, finishes, reply, |c| async {
                let changed = c
                    .execute(
                        "UPDATE memory_thread_summaries SET usage_count = usage_count + 1, last_usage_ms = $3
                         WHERE role_id = $1 AND thread_id = $2",
                        &[&role.as_str(), &thread.as_str(), &at_ms],
                    )
                    .await?;
                Ok(changed > 0)
            }),
            Op::ClaimBinding { binding, reply } => {
                step!(transaction, cache, finishes, reply, |c| claim_binding(&c, &binding))
            }
            Op::SetBindingAgent {
                channel,
                conversation,
                agent,
                reply,
            } => step!(transaction, cache, finishes, reply, |c| set_binding_agent(
                &c,
                &channel,
                &conversation,
                &agent
            )),
            Op::RememberInbound { channel, key, reply } => step!(transaction, cache, finishes, reply, |c| async {
                let inserted = c
                    .execute(
                        "INSERT INTO channel_inbox (channel, dedupe_key, at_ms) VALUES ($1, $2, $3)
                         ON CONFLICT DO NOTHING",
                        &[&clean_text(&channel), &clean_text(&key), &ip_core::now_ms()],
                    )
                    .await?;
                Ok(inserted == 1)
            }),
            Op::PruneInbound { before_ms, reply } => step!(transaction, cache, finishes, reply, |c| async {
                let inbox = c
                    .execute("DELETE FROM channel_inbox WHERE at_ms < $1", &[&before_ms])
                    .await?;
                let dropped = c
                    .execute(
                        "DELETE FROM channel_envelopes WHERE state != 'pending' AND received_ms < $1",
                        &[&before_ms],
                    )
                    .await?;
                Ok(count(inbox + dropped))
            }),
            Op::StashEnvelope {
                channel,
                id,
                payload,
                reply,
            } => step!(transaction, cache, finishes, reply, |c| async {
                let inserted = c
                    .execute(
                        "INSERT INTO channel_envelopes (channel, id, payload, received_ms) VALUES ($1, $2, $3, $4)
                         ON CONFLICT DO NOTHING",
                        &[
                            &clean_text(&channel),
                            &clean_text(&id),
                            &clean_text(&payload),
                            &ip_core::now_ms(),
                        ],
                    )
                    .await?;
                Ok(inserted == 1)
            }),
            Op::SettleEnvelope {
                channel,
                id,
                settle,
                reply,
            } => step!(transaction, cache, finishes, reply, |c| settle_envelope(
                &c, &channel, &id, &settle
            )),
            Op::PutDelivery { delivery, reply } => {
                step!(transaction, cache, finishes, reply, |c| put_delivery(&c, &delivery))
            }
            Op::PutCursor { channel, seq, reply } => step!(transaction, cache, finishes, reply, |c| async {
                c.execute(
                    "INSERT INTO channel_cursors (channel, seq) VALUES ($1, $2)
                     ON CONFLICT (channel) DO UPDATE SET seq = excluded.seq",
                    &[&channel, &seq],
                )
                .await?;
                Ok(())
            }),
        }
    }
    transaction.commit().await
}

fn count(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

/// PostgreSQL 的 `text` 不能含 NUL 字符（SQLite 可以）。工具输出偶尔带二进制内容，换成替换字符。
pub(crate) fn clean_text(text: &str) -> std::borrow::Cow<'_, str> {
    if text.contains('\0') {
        std::borrow::Cow::Owned(text.replace('\0', "\u{FFFD}"))
    } else {
        std::borrow::Cow::Borrowed(text)
    }
}

/// 序列化成 JSON 文本，准备存进 `jsonb`。`jsonb` 不收 `\u0000`，换成 `\ufffd`；
/// 只换真正的转义（前面的反斜杠是偶数个），字符串里字面的 `\\u0000` 不动。
pub(crate) fn to_jsonb<T: serde::Serialize + ?Sized>(value: &T) -> Result<String> {
    Ok(jsonb_safe(serde_json::to_string(value)?))
}

/// 把 JSON 文本里的 `\u0000` 换成 `\ufffd`，见 [`to_jsonb`]。
pub(crate) fn jsonb_safe(json: String) -> String {
    if !json.contains("\\u0000") {
        return json;
    }
    let bytes = json.as_bytes();
    let mut out = String::with_capacity(json.len());
    let mut i = 0;
    let mut last = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if bytes[i..].starts_with(b"\\u0000") {
                out.push_str(&json[last..i]);
                out.push_str("\\ufffd");
                i += 6;
                last = i;
            } else {
                // 跳过这个转义的下一个字符（可能是另一个反斜杠）。
                i += 2;
            }
        } else {
            i += 1;
        }
    }
    out.push_str(&json[last..]);
    out
}

fn is_unique_violation(error: &tokio_postgres::Error) -> bool {
    error.code() == Some(&SqlState::UNIQUE_VIOLATION)
}

async fn event(c: &Tx<'_, '_>, kind: &str, payload: &str) -> Result<Seq> {
    let row = c
        .query_one(
            "INSERT INTO events (at_ms, kind, payload) VALUES ($1, $2, $3::text::jsonb) RETURNING seq",
            &[&ip_core::now_ms(), &kind, &payload],
        )
        .await?;
    Ok(row.get(0))
}

async fn put_task(c: &Tx<'_, '_>, task: &Task) -> Result<Seq> {
    let json = to_jsonb(task)?;
    let seq = event(c, "task.put", &json).await?;
    let status = serde_json::to_value(task.status)?;
    c.execute(
        "INSERT INTO tasks (id, status, updated_at_ms, json, seq) VALUES ($1, $2, $3, $4::text::jsonb, $5)
         ON CONFLICT (id) DO UPDATE SET status = excluded.status, updated_at_ms = excluded.updated_at_ms,
             json = excluded.json, seq = excluded.seq",
        &[
            &clean_text(task.id.as_str()),
            &status.as_str().unwrap_or("open"),
            &task.updated_at_ms,
            &json,
            &seq,
        ],
    )
    .await?;
    Ok(seq)
}

/// 外部会话还没有绑定时写入新绑定（事件 `binding.put`）；已经有了就原样返回已有的，不写事件。
/// 同一个会话几乎同时来两条消息时，只有第一条开启对话。
async fn claim_binding(c: &Tx<'_, '_>, b: &Binding) -> Result<Binding> {
    if let Some(existing) = binding_row(c, &b.channel, &b.conversation).await? {
        return Ok(existing);
    }
    write_binding(c, b).await?;
    Ok(b.clone())
}

/// 绑定里还没记员工时补上（D-05）；已经有员工或没有这个绑定时不改，返回现在的绑定。
async fn set_binding_agent(
    c: &Tx<'_, '_>,
    channel: &str,
    conversation: &str,
    agent: &ip_core::AgentId,
) -> Result<Option<Binding>> {
    let Some(mut b) = binding_row(c, channel, conversation).await? else {
        return Ok(None);
    };
    if b.agent_id.is_none() {
        b.agent_id = Some(agent.clone());
        write_binding(c, &b).await?;
    }
    Ok(Some(b))
}

async fn binding_row(c: &Tx<'_, '_>, channel: &str, conversation: &str) -> Result<Option<Binding>> {
    let row = c
        .query_opt(crate::BINDING_SQL, &[&clean_text(channel), &clean_text(conversation)])
        .await?;
    Ok(row.map(|row| crate::binding_from(channel, conversation, &row)))
}

async fn write_binding(c: &Tx<'_, '_>, b: &Binding) -> Result<()> {
    let json = to_jsonb(b)?;
    let seq = event(c, "binding.put", &json).await?;
    c.execute(
        "INSERT INTO channel_bindings (channel, conversation, thread_id, agent_id, requester, created_at_ms, seq)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (channel, conversation) DO UPDATE SET agent_id = excluded.agent_id, seq = excluded.seq",
        &[
            &clean_text(&b.channel),
            &clean_text(&b.conversation),
            &clean_text(b.thread_id.as_str()),
            &b.agent_id.as_ref().map(|a| clean_text(a.as_str())),
            &clean_text(&b.requester),
            &b.created_at_ms,
            &seq,
        ],
    )
    .await?;
    Ok(())
}

async fn settle_envelope(c: &Tx<'_, '_>, channel: &str, id: &str, settle: &crate::Settle) -> Result<()> {
    match settle {
        // 处理完的留到保留期满再删：Slack 重推同一个信封时还认得出来。
        crate::Settle::Done => {
            c.execute(
                "UPDATE channel_envelopes SET state = 'done', payload = '', error = NULL
                 WHERE channel = $1 AND id = $2",
                &[&channel, &id],
            )
            .await?;
        }
        crate::Settle::Failed(error) => {
            c.execute(
                "UPDATE channel_envelopes SET attempts = attempts + 1, error = $3 WHERE channel = $1 AND id = $2",
                &[&channel, &id, &clean_text(error)],
            )
            .await?;
        }
        crate::Settle::GaveUp(error) => {
            c.execute(
                "UPDATE channel_envelopes SET attempts = attempts + 1, error = $3, state = 'dropped'
                 WHERE channel = $1 AND id = $2",
                &[&channel, &id, &clean_text(error)],
            )
            .await?;
        }
    }
    Ok(())
}

async fn put_delivery(c: &Tx<'_, '_>, d: &Delivery) -> Result<()> {
    c.execute(
        "INSERT INTO channel_deliveries (channel, item, conversation, external_ref, state, error, at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (channel, item) DO UPDATE SET conversation = excluded.conversation,
             external_ref = excluded.external_ref, state = excluded.state, error = excluded.error,
             at_ms = excluded.at_ms",
        &[
            &clean_text(&d.channel),
            &clean_text(&d.item),
            &clean_text(&d.conversation),
            &d.external_ref.as_deref().map(clean_text),
            &if d.sent { "sent" } else { "failed" },
            &d.error.as_deref().map(clean_text),
            &d.at_ms,
        ],
    )
    .await?;
    Ok(())
}

/// 引擎会话是运行状态，不写事件：每轮都会更新，写进事件日志只会让日志膨胀。
async fn put_engine_session(c: &Tx<'_, '_>, s: &EngineSession) -> Result<()> {
    c.execute(
        "INSERT INTO engine_sessions
             (agent_id, thread_id, engine, session_id, last_message, updated_at_ms, used_prompt_tokens,
              used_cached_tokens, used_cache_write_tokens, used_completion_tokens)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (agent_id, thread_id) DO UPDATE SET
             engine = excluded.engine, session_id = excluded.session_id,
             last_message = excluded.last_message, updated_at_ms = excluded.updated_at_ms,
             used_prompt_tokens = excluded.used_prompt_tokens,
             used_cached_tokens = excluded.used_cached_tokens,
             used_cache_write_tokens = excluded.used_cache_write_tokens,
             used_completion_tokens = excluded.used_completion_tokens",
        &[
            &clean_text(s.agent_id.as_str()),
            &clean_text(s.thread_id.as_str()),
            &clean_text(&s.engine),
            &clean_text(&s.session_id),
            &clean_text(s.last_message.as_str()),
            &s.updated_at_ms,
            &i64::from(s.used.prompt_tokens),
            &i64::from(s.used.cached_tokens),
            &i64::from(s.used.cache_write_tokens),
            &i64::from(s.used.completion_tokens),
        ],
    )
    .await?;
    Ok(())
}

/// 覆盖提炼结果，但保留使用计数：它记的是这条记忆被用过多少，重新提炼不该清零。
async fn put_thread_memory(c: &Tx<'_, '_>, m: &crate::ThreadMemory) -> Result<()> {
    c.execute(
        "INSERT INTO memory_thread_summaries
             (role_id, thread_id, source_updated_ms, generated_ms, status, slug, summary, attempts, retry_after_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (role_id, thread_id) DO UPDATE SET
             source_updated_ms = excluded.source_updated_ms, generated_ms = excluded.generated_ms,
             status = excluded.status, slug = excluded.slug, summary = excluded.summary,
             attempts = excluded.attempts, retry_after_ms = excluded.retry_after_ms",
        &[
            &clean_text(m.role_id.as_str()),
            &clean_text(m.thread_id.as_str()),
            &m.source_updated_ms,
            &m.generated_ms,
            &m.status.as_str(),
            &m.slug.as_deref().map(clean_text),
            &m.summary.as_deref().map(clean_text),
            &i64::from(m.attempts),
            &m.retry_after_ms,
        ],
    )
    .await?;
    Ok(())
}

async fn put_setting(c: &Tx<'_, '_>, key: &str, value: &serde_json::Value) -> Result<Seq> {
    let json = to_jsonb(value)?;
    let payload = to_jsonb(&serde_json::json!({ "key": key, "value": value }))?;
    let seq = event(c, "setting.put", &payload).await?;
    c.execute(
        "INSERT INTO settings (key, json, seq) VALUES ($1, $2::text::jsonb, $3)
         ON CONFLICT (key) DO UPDATE SET json = excluded.json, seq = excluded.seq",
        &[&clean_text(key), &json, &seq],
    )
    .await?;
    Ok(seq)
}

async fn put_approval(c: &Tx<'_, '_>, approval: &Approval) -> Result<Seq> {
    let json = to_jsonb(approval)?;
    let seq = event(c, "approval.created", &json).await?;
    let inserted = c
        .execute_raw(
            "INSERT INTO approvals (id, pending, created_at_ms, json, seq) VALUES ($1, $2, $3, $4::text::jsonb, $5)",
            &[
                &clean_text(approval.id.as_str()),
                &approval.is_pending(),
                &approval.created_at_ms,
                &json,
                &seq,
            ],
        )
        .await?;
    match inserted {
        Ok(_) => Ok(seq),
        Err(error) if is_unique_violation(&error) => Err(StoreError::Conflict(format!("审批 {} 已存在", approval.id))),
        Err(error) => Err(error.into()),
    }
}

/// 只在写任务里检查并改状态，同一张卡片被两个人同时点也只会生效一次。
/// `FOR UPDATE` 防的是另一个进程同时在改同一张卡片。
async fn resolve_approval(c: &Tx<'_, '_>, id: &ApprovalId, choice: u32, by: &str) -> Result<Approval> {
    let row = c
        .query_opt(
            "SELECT json::text FROM approvals WHERE id = $1 FOR UPDATE",
            &[&id.as_str()],
        )
        .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound(format!("审批 {id}")));
    };
    let mut approval: Approval = serde_json::from_str(row.get::<_, &str>(0))?;
    if let ApprovalState::Resolved { choice, by, .. } = &approval.state {
        return Err(StoreError::Conflict(format!(
            "这张卡片已由 {by} 选了第 {} 项",
            choice + 1
        )));
    }
    if choice as usize >= approval.options.len() {
        return Err(StoreError::Conflict(format!("没有第 {} 个选项", choice + 1)));
    }
    approval.state = ApprovalState::Resolved {
        choice,
        by: by.to_string(),
        at_ms: ip_core::now_ms(),
    };
    let json = to_jsonb(&approval)?;
    let seq = event(c, "approval.resolved", &json).await?;
    c.execute(
        "UPDATE approvals SET pending = FALSE, json = $2::text::jsonb, seq = $3 WHERE id = $1",
        &[&id.as_str(), &json, &seq],
    )
    .await?;
    Ok(approval)
}

async fn append_message(c: &Tx<'_, '_>, message: &Message) -> Result<Seq> {
    let json = to_jsonb(message)?;
    let seq = event(c, "message.appended", &json).await?;
    let text = message.searchable_text().map(clean_text);
    let search = text.as_deref().and_then(segment::index_text);
    let inserted = c
        .execute_raw(
            "INSERT INTO messages (id, thread_id, at_ms, text, json, seq, search)
             VALUES ($1, $2, $3, $4, $5::text::jsonb, $6, $7::text::tsvector)",
            &[
                &clean_text(message.id.as_str()),
                &clean_text(message.thread_id.as_str()),
                &message.at_ms,
                &text,
                &json,
                &seq,
                &search,
            ],
        )
        .await?;
    match inserted {
        Ok(_) => {}
        Err(error) if is_unique_violation(&error) => return Err(StoreError::DuplicateMessage(message.id.clone())),
        Err(error) => return Err(error.into()),
    }
    let preview = text.as_deref().map(preview);
    c.execute(
        "INSERT INTO threads (id, last_at_ms, message_count, preview) VALUES ($1, $2, 1, $3)
         ON CONFLICT (id) DO UPDATE SET
             last_at_ms = GREATEST(threads.last_at_ms, excluded.last_at_ms),
             message_count = threads.message_count + 1,
             preview = COALESCE(excluded.preview, threads.preview)",
        &[&clean_text(message.thread_id.as_str()), &message.at_ms, &preview],
    )
    .await?;
    Ok(seq)
}

/// 对话列表里显示的摘要：最后一条文字消息的前若干个字符。
fn preview(text: &str) -> &str {
    const MAX_CHARS: usize = 80;
    match text.char_indices().nth(MAX_CHARS) {
        Some((end, _)) => &text[..end],
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonb_text_has_no_nul_escapes() {
        let json = to_jsonb(&serde_json::json!({ "a": "x\0y", "b": "\\u0000" })).unwrap();
        assert!(json.contains("x\\ufffdy"), "{json}");
        // 字面的反斜杠加 u0000 不是 NUL，原样保留。
        assert!(json.contains("\\\\u0000"), "{json}");
    }
}
