//! 专用写线程。一次取出所有已排队的写请求，放进同一个事务；每个请求有自己的保存点，
//! 一个请求失败（比如 id 重复）只回滚它自己。

use std::sync::mpsc;

use ip_core::{Approval, ApprovalId, ApprovalState, Message, Task};

use crate::{Binding, Delivery, EngineSession};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Transaction, params};
use tokio::sync::oneshot;

use crate::{Result, Seq, StoreError, segment};

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
    sender: mpsc::Sender<Op>,
}

impl Handle {
    pub fn send(&self, op: Op) -> Result<()> {
        self.sender.send(op).map_err(|_| StoreError::Closed)
    }
}

pub fn spawn(connection: Connection) -> Result<Handle> {
    let (sender, receiver) = mpsc::channel();
    std::thread::Builder::new()
        .name("ip-store-writer".into())
        .spawn(move || run(connection, &receiver))
        .map_err(|_| StoreError::Closed)?;
    Ok(Handle { sender })
}

fn run(mut connection: Connection, receiver: &mpsc::Receiver<Op>) {
    // 所有 Handle 都被丢弃后 recv 返回错误，线程结束，连接随之关闭。
    while let Ok(first) = receiver.recv() {
        let mut batch = vec![first];
        while batch.len() < MAX_BATCH {
            match receiver.try_recv() {
                Ok(op) => batch.push(op),
                Err(_) => break,
            }
        }
        commit(&mut connection, batch);
    }
}

/// 事务提交后把结果交给调用方；参数是整批失败时的原因。
type Finish = Box<dyn FnOnce(Option<&str>) + Send>;

/// 在保存点里执行一个请求：失败只回滚它自己。
fn step<T: Send + 'static>(
    transaction: &mut Transaction<'_>,
    reply: oneshot::Sender<Result<T>>,
    run: impl FnOnce(&Connection) -> Result<T>,
) -> rusqlite::Result<Finish> {
    let mut savepoint = transaction.savepoint()?;
    let result = run(&savepoint);
    if result.is_ok() {
        savepoint.commit()?;
    } else {
        savepoint.rollback()?;
    }
    Ok(Box::new(move |batch_error| {
        let _ = reply.send(match batch_error {
            None => result,
            Some(message) => Err(StoreError::Batch(message.to_string())),
        });
    }))
}

fn commit(connection: &mut Connection, batch: Vec<Op>) {
    let mut finishes: Vec<Finish> = Vec::with_capacity(batch.len());
    let outcome = (|| -> rusqlite::Result<()> {
        let mut transaction = connection.transaction()?;
        for op in batch {
            let finish = match op {
                Op::AppendMessage { message, reply } => step(&mut transaction, reply, |c| append_message(c, &message))?,
                Op::PutTask { task, reply } => step(&mut transaction, reply, |c| put_task(c, &task))?,
                Op::PutApproval { approval, reply } => step(&mut transaction, reply, |c| put_approval(c, &approval))?,
                Op::PutSetting { key, value, reply } => {
                    step(&mut transaction, reply, |c| put_setting(c, &key, &value))?
                }
                Op::ResolveApproval { id, choice, by, reply } => {
                    step(&mut transaction, reply, |c| resolve_approval(c, &id, choice, &by))?
                }
                Op::PutEngineSession { session, reply } => {
                    step(&mut transaction, reply, |c| put_engine_session(c, &session))?
                }
                Op::PutThreadMemory { memory, reply } => {
                    step(&mut transaction, reply, |c| put_thread_memory(c, &memory))?
                }
                Op::AddMemoryNote {
                    role,
                    thread,
                    text,
                    at_ms,
                    reply,
                } => step(&mut transaction, reply, |c| {
                    c.prepare_cached(
                        "INSERT INTO memory_notes (role_id, created_ms, thread_id, text) VALUES (?1, ?2, ?3, ?4)",
                    )?
                    .execute(params![
                        role.as_str(),
                        at_ms,
                        thread.as_ref().map(ip_core::ThreadId::as_str),
                        text
                    ])?;
                    Ok(c.last_insert_rowid())
                })?,
                Op::ApplyMemoryNotes {
                    role,
                    ids,
                    at_ms,
                    reply,
                } => step(&mut transaction, reply, |c| {
                    let mut statement = c.prepare_cached(
                        "UPDATE memory_notes SET applied_ms = ?3 WHERE role_id = ?1 AND id = ?2 AND applied_ms IS NULL",
                    )?;
                    for id in &ids {
                        statement.execute(params![role.as_str(), id, at_ms])?;
                    }
                    Ok(())
                })?,
                Op::ForgetThreadMemories { role, thread, reply } => step(&mut transaction, reply, |c| {
                    if role.is_none() && thread.is_none() {
                        return Ok(0);
                    }
                    Ok(c.prepare_cached(
                        "UPDATE memory_thread_summaries
                         SET status = 'empty', summary = NULL, slug = NULL, attempts = 0, retry_after_ms = NULL
                         WHERE (?1 IS NULL OR role_id = ?1) AND (?2 IS NULL OR thread_id = ?2) AND summary IS NOT NULL",
                    )?
                    .execute(params![
                        role.as_ref().map(ip_core::RoleId::as_str),
                        thread.as_ref().map(ip_core::ThreadId::as_str)
                    ])?)
                })?,
                Op::DeleteMemoryNotes { role, id, reply } => step(&mut transaction, reply, |c| {
                    Ok(
                        c.prepare_cached("DELETE FROM memory_notes WHERE role_id = ?1 AND (?2 IS NULL OR id = ?2)")?
                            .execute(params![role.as_str(), id])?,
                    )
                })?,
                Op::TouchThreadMemory {
                    role,
                    thread,
                    at_ms,
                    reply,
                } => step(&mut transaction, reply, |c| {
                    touch_thread_memory(c, &role, &thread, at_ms)
                })?,
                Op::ClaimBinding { binding, reply } => step(&mut transaction, reply, |c| claim_binding(c, &binding))?,
                Op::SetBindingAgent {
                    channel,
                    conversation,
                    agent,
                    reply,
                } => step(&mut transaction, reply, |c| {
                    set_binding_agent(c, &channel, &conversation, &agent)
                })?,
                Op::RememberInbound { channel, key, reply } => {
                    step(&mut transaction, reply, |c| remember_inbound(c, &channel, &key))?
                }
                Op::PruneInbound { before_ms, reply } => step(&mut transaction, reply, |c| {
                    let inbox = c
                        .prepare_cached("DELETE FROM channel_inbox WHERE at_ms < ?1")?
                        .execute(params![before_ms])?;
                    let dropped = c
                        .prepare_cached("DELETE FROM channel_envelopes WHERE state != 'pending' AND received_ms < ?1")?
                        .execute(params![before_ms])?;
                    Ok(inbox + dropped)
                })?,
                Op::StashEnvelope {
                    channel,
                    id,
                    payload,
                    reply,
                } => step(&mut transaction, reply, |c| {
                    let inserted = c
                        .prepare_cached(
                            "INSERT OR IGNORE INTO channel_envelopes (channel, id, payload, received_ms)
                             VALUES (?1, ?2, ?3, ?4)",
                        )?
                        .execute(params![channel, id, payload, ip_core::now_ms()])?;
                    Ok(inserted == 1)
                })?,
                Op::SettleEnvelope {
                    channel,
                    id,
                    settle,
                    reply,
                } => step(&mut transaction, reply, |c| settle_envelope(c, &channel, &id, &settle))?,
                Op::PutDelivery { delivery, reply } => step(&mut transaction, reply, |c| put_delivery(c, &delivery))?,
                Op::PutCursor { channel, seq, reply } => step(&mut transaction, reply, |c| {
                    c.prepare_cached(
                        "INSERT INTO channel_cursors (channel, seq) VALUES (?1, ?2)
                         ON CONFLICT(channel) DO UPDATE SET seq = excluded.seq",
                    )?
                    .execute(params![channel, seq])?;
                    Ok(())
                })?,
            };
            finishes.push(finish);
        }
        transaction.commit()
    })();
    let error = outcome.err().map(|e| e.to_string());
    for finish in finishes {
        finish(error.as_deref());
    }
}

fn event(connection: &Connection, kind: &str, payload: &str) -> Result<Seq> {
    connection
        .prepare_cached("INSERT INTO events (at_ms, kind, payload) VALUES (?1, ?2, ?3)")?
        .execute(params![ip_core::now_ms(), kind, payload])?;
    Ok(connection.last_insert_rowid())
}

fn put_task(connection: &Connection, task: &Task) -> Result<Seq> {
    let json = serde_json::to_string(task)?;
    let seq = event(connection, "task.put", &json)?;
    let status = serde_json::to_value(task.status)?;
    connection
        .prepare_cached(
            "INSERT INTO tasks (id, status, updated_at_ms, json, seq) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET status = excluded.status, updated_at_ms = excluded.updated_at_ms,
                 json = excluded.json, seq = excluded.seq",
        )?
        .execute(params![
            task.id.as_str(),
            status.as_str().unwrap_or("open"),
            task.updated_at_ms,
            json,
            seq
        ])?;
    Ok(seq)
}

/// 外部会话还没有绑定时写入新绑定（事件 `binding.put`）；已经有了就原样返回已有的，不写事件。
/// 同一个会话几乎同时来两条消息时，只有第一条开启对话。
fn claim_binding(connection: &Connection, b: &Binding) -> Result<Binding> {
    if let Some(existing) = crate::binding_row(connection, &b.channel, &b.conversation)? {
        return Ok(existing);
    }
    write_binding(connection, b)?;
    Ok(b.clone())
}

/// 绑定里还没记员工时补上（D-05）；已经有员工或没有这个绑定时不改，返回现在的绑定。
fn set_binding_agent(
    connection: &Connection,
    channel: &str,
    conversation: &str,
    agent: &ip_core::AgentId,
) -> Result<Option<Binding>> {
    let Some(mut b) = crate::binding_row(connection, channel, conversation)? else {
        return Ok(None);
    };
    if b.agent_id.is_none() {
        b.agent_id = Some(agent.clone());
        write_binding(connection, &b)?;
    }
    Ok(Some(b))
}

fn write_binding(connection: &Connection, b: &Binding) -> Result<()> {
    let json = serde_json::to_string(b)?;
    let seq = event(connection, "binding.put", &json)?;
    connection
        .prepare_cached(
            "INSERT INTO channel_bindings (channel, conversation, thread_id, agent_id, requester, created_at_ms, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(channel, conversation) DO UPDATE SET agent_id = excluded.agent_id, seq = excluded.seq",
        )?
        .execute(params![
            b.channel,
            b.conversation,
            b.thread_id.as_str(),
            b.agent_id.as_ref().map(ip_core::AgentId::as_str),
            b.requester,
            b.created_at_ms,
            seq
        ])?;
    Ok(())
}

fn settle_envelope(connection: &Connection, channel: &str, id: &str, settle: &crate::Settle) -> Result<()> {
    match settle {
        // 处理完的留到保留期满再删：Slack 重推同一个信封时还认得出来。
        crate::Settle::Done => {
            connection
                .prepare_cached(
                    "UPDATE channel_envelopes SET state = 'done', payload = '', error = NULL
                     WHERE channel = ?1 AND id = ?2",
                )?
                .execute(params![channel, id])?;
        }
        crate::Settle::Failed(error) => {
            connection
                .prepare_cached(
                    "UPDATE channel_envelopes SET attempts = attempts + 1, error = ?3 WHERE channel = ?1 AND id = ?2",
                )?
                .execute(params![channel, id, error])?;
        }
        crate::Settle::GaveUp(error) => {
            connection
                .prepare_cached(
                    "UPDATE channel_envelopes SET attempts = attempts + 1, error = ?3, state = 'dropped'
                     WHERE channel = ?1 AND id = ?2",
                )?
                .execute(params![channel, id, error])?;
        }
    }
    Ok(())
}

/// 第一次见到这个去重 key 时返回 true。
fn remember_inbound(connection: &Connection, channel: &str, key: &str) -> Result<bool> {
    let inserted = connection
        .prepare_cached("INSERT OR IGNORE INTO channel_inbox (channel, dedupe_key, at_ms) VALUES (?1, ?2, ?3)")?
        .execute(params![channel, key, ip_core::now_ms()])?;
    Ok(inserted == 1)
}

fn put_delivery(connection: &Connection, d: &Delivery) -> Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO channel_deliveries (channel, item, conversation, external_ref, state, error, at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(channel, item) DO UPDATE SET conversation = excluded.conversation,
                 external_ref = excluded.external_ref, state = excluded.state, error = excluded.error,
                 at_ms = excluded.at_ms",
        )?
        .execute(params![
            d.channel,
            d.item,
            d.conversation,
            d.external_ref,
            if d.sent { "sent" } else { "failed" },
            d.error,
            d.at_ms
        ])?;
    Ok(())
}

/// 引擎会话是运行状态，不写事件：每轮都会更新，写进事件日志只会让日志膨胀。
fn put_engine_session(connection: &Connection, s: &EngineSession) -> Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO engine_sessions
                 (agent_id, thread_id, engine, session_id, last_message, updated_at_ms, used_prompt_tokens,
                  used_cached_tokens, used_cache_write_tokens, used_completion_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(agent_id, thread_id) DO UPDATE SET
                 engine = excluded.engine, session_id = excluded.session_id,
                 last_message = excluded.last_message, updated_at_ms = excluded.updated_at_ms,
                 used_prompt_tokens = excluded.used_prompt_tokens,
                 used_cached_tokens = excluded.used_cached_tokens,
                 used_cache_write_tokens = excluded.used_cache_write_tokens,
                 used_completion_tokens = excluded.used_completion_tokens",
        )?
        .execute(params![
            s.agent_id.as_str(),
            s.thread_id.as_str(),
            s.engine,
            s.session_id,
            s.last_message.as_str(),
            s.updated_at_ms,
            s.used.prompt_tokens,
            s.used.cached_tokens,
            s.used.cache_write_tokens,
            s.used.completion_tokens
        ])?;
    Ok(())
}

/// 覆盖提炼结果，但保留使用计数和“上次合并是否选中”：它们记的是这条记忆被用过多少，重新提炼不该清零。
fn put_thread_memory(connection: &Connection, m: &crate::ThreadMemory) -> Result<()> {
    connection
        .prepare_cached(
            "INSERT INTO memory_thread_summaries
                 (role_id, thread_id, source_updated_ms, generated_ms, status, slug, summary, attempts, retry_after_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(role_id, thread_id) DO UPDATE SET
                 source_updated_ms = excluded.source_updated_ms, generated_ms = excluded.generated_ms,
                 status = excluded.status, slug = excluded.slug, summary = excluded.summary,
                 attempts = excluded.attempts, retry_after_ms = excluded.retry_after_ms",
        )?
        .execute(params![
            m.role_id.as_str(),
            m.thread_id.as_str(),
            m.source_updated_ms,
            m.generated_ms,
            m.status.as_str(),
            m.slug,
            m.summary,
            m.attempts,
            m.retry_after_ms
        ])?;
    Ok(())
}

/// 记一次“这条记忆被读过”。返回这条记忆在不在。
fn touch_thread_memory(
    connection: &Connection,
    role: &ip_core::RoleId,
    thread: &ip_core::ThreadId,
    at_ms: i64,
) -> Result<bool> {
    let changed = connection
        .prepare_cached(
            "UPDATE memory_thread_summaries SET usage_count = usage_count + 1, last_usage_ms = ?3
             WHERE role_id = ?1 AND thread_id = ?2",
        )?
        .execute(params![role.as_str(), thread.as_str(), at_ms])?;
    Ok(changed > 0)
}

fn put_setting(connection: &Connection, key: &str, value: &serde_json::Value) -> Result<Seq> {
    let json = serde_json::to_string(value)?;
    let payload = serde_json::to_string(&serde_json::json!({ "key": key, "value": value }))?;
    let seq = event(connection, "setting.put", &payload)?;
    connection
        .prepare_cached(
            "INSERT INTO settings (key, json, seq) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET json = excluded.json, seq = excluded.seq",
        )?
        .execute(params![key, json, seq])?;
    Ok(seq)
}

fn put_approval(connection: &Connection, approval: &Approval) -> Result<Seq> {
    let json = serde_json::to_string(approval)?;
    let seq = event(connection, "approval.created", &json)?;
    let inserted = connection
        .prepare_cached("INSERT INTO approvals (id, pending, created_at_ms, json, seq) VALUES (?1, ?2, ?3, ?4, ?5)")?
        .execute(params![
            approval.id.as_str(),
            approval.is_pending(),
            approval.created_at_ms,
            json,
            seq
        ]);
    match inserted {
        Ok(_) => Ok(seq),
        Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == ErrorCode::ConstraintViolation => {
            Err(StoreError::Conflict(format!("审批 {} 已存在", approval.id)))
        }
        Err(error) => Err(error.into()),
    }
}

/// 只在写线程里检查并改状态，同一张卡片被两个人同时点也只会生效一次。
fn resolve_approval(connection: &Connection, id: &ApprovalId, choice: u32, by: &str) -> Result<Approval> {
    let json: Option<String> = connection
        .prepare_cached("SELECT json FROM approvals WHERE id = ?1")?
        .query_row(params![id.as_str()], |row| row.get(0))
        .optional()?;
    let Some(json) = json else {
        return Err(StoreError::NotFound(format!("审批 {id}")));
    };
    let mut approval: Approval = serde_json::from_str(&json)?;
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
    let json = serde_json::to_string(&approval)?;
    let seq = event(connection, "approval.resolved", &json)?;
    connection
        .prepare_cached("UPDATE approvals SET pending = 0, json = ?2, seq = ?3 WHERE id = ?1")?
        .execute(params![id.as_str(), json, seq])?;
    Ok(approval)
}

fn append_message(connection: &Connection, message: &Message) -> Result<Seq> {
    let json = serde_json::to_string(message)?;
    let seq = event(connection, "message.appended", &json)?;
    let text = message.searchable_text();
    let inserted = connection
        .prepare_cached("INSERT INTO messages (id, thread_id, at_ms, text, json, seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?
        .execute(params![
            message.id.as_str(),
            message.thread_id.as_str(),
            message.at_ms,
            text,
            json,
            seq
        ]);
    match inserted {
        Ok(_) => {}
        Err(rusqlite::Error::SqliteFailure(error, _)) if error.code == ErrorCode::ConstraintViolation => {
            return Err(StoreError::DuplicateMessage(message.id.clone()));
        }
        Err(error) => return Err(error.into()),
    }
    let rowid = connection.last_insert_rowid();
    let preview = text.map(preview);
    connection
        .prepare_cached(
            "INSERT INTO threads (id, last_at_ms, message_count, preview) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(id) DO UPDATE SET
                 last_at_ms = MAX(last_at_ms, excluded.last_at_ms),
                 message_count = message_count + 1,
                 preview = COALESCE(excluded.preview, preview)",
        )?
        .execute(params![message.thread_id.as_str(), message.at_ms, preview])?;
    if let Some(text) = text {
        connection
            .prepare_cached("INSERT INTO messages_fts (rowid, body) VALUES (?1, ?2)")?
            .execute(params![rowid, segment::index_text(text)])?;
    }
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
