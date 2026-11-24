//! 外部渠道的 Hub 和投递器（`docs/ip/m2-spec.md` D-03 到 D-08）。
//!
//! - **Hub**：渠道收到消息、卡片被点击时交给它。它负责去重、准入、找或建会话绑定、分派给员工，
//!   以及按 `approvers` 决定点击是否生效。
//! - **投递器**：每个渠道一个后台任务，从事件日志按 `seq` 往后读，挑出该发到这个渠道的
//!   员工回复、“这轮失败”、审批卡片和卡片选定结果，按顺序交给渠道发出。读到哪里记在
//!   `channel_cursors` 里，重启后接着读；发过什么记在 `channel_deliveries` 里，不会重发。
//!
//! 用 [`start`] 把一个渠道接到 [`App`] 上。

use std::sync::Arc;
use std::time::Duration;

use ip_connectors::{
    CancellationToken, CardClick, Channel, ConversationKey, DeliverError, Hub, HubError, Inbound, Outbound, Settle,
    Stashed,
};
use ip_core::{AgentId, Approval, ApprovalSubject, Author, Body, Message, Notice, ThreadId, Via, now_ms};
use ip_store::{Binding, Delivery, Store, StoredEvent};
use tokio::task::JoinHandle;

use crate::App;

pub mod report_delivery;

/// 去重记录保留多久。
const INBOX_RETENTION: Duration = Duration::from_secs(7 * 24 * 3600);
/// 投递器一次从事件日志读多少条。
const BATCH: usize = 256;
/// 漏掉了实时通知也没关系：最多这么久就再看一次事件日志。
const POLL: Duration = Duration::from_secs(5);
/// 可重试的失败，第一次等多久；之后每次翻倍。
const FIRST_BACKOFF: Duration = Duration::from_secs(1);
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// 顺着派活关系找卡片所属会话时最多往上找几层（D-06）。比委派层数上限留些余量。
const MAX_HOPS: usize = 8;

/// 一个渠道的接入设置，对应 `ip.yaml` 里 `channels.<名字>` 的 `allow.users`、`approvers`、`agent`（D-16）。
/// `allow.channels` 这类和外部会话格式有关的过滤由各渠道自己做。
#[derive(Clone, Debug, Default)]
pub struct ChannelSettings {
    /// 只接这些用户的消息；空表示所有人。
    pub allow_users: Vec<String>,
    pub approvers: Approvers,
    /// 这个渠道的消息都交给固定员工，不走路由。
    pub agent: Option<AgentId>,
}

/// 谁能点卡片（D-07）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Approvers {
    /// 只有开启这段会话的人。
    #[default]
    Requester,
    Anyone,
    /// 只有列出的用户 id。
    Only(Vec<String>),
}

/// 接上的渠道。`stop` 停掉渠道和它的投递器；只是丢弃句柄的话它们会一直运行到服务退出。
pub struct ChannelHandle {
    shutdown: CancellationToken,
    tasks: Vec<JoinHandle<()>>,
}

impl ChannelHandle {
    pub async fn stop(self) {
        self.shutdown.cancel();
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

/// 把 `channel` 接到 `app` 上：启动 `channel.run` 和这个渠道的投递器。
///
/// 第一次接入的渠道从事件日志的末尾开始投递，不会把以前的历史补发出去。
pub async fn start(app: &App, channel: Arc<dyn Channel>, settings: ChannelSettings) -> Result<ChannelHandle, String> {
    let name = channel.name().to_string();
    let store = app.runtime().store().clone();
    if store.cursor(&name).await.map_err(|e| e.to_string())?.is_none() {
        let end = store.last_seq().await.map_err(|e| e.to_string())?;
        store.put_cursor(&name, end).await.map_err(|e| e.to_string())?;
    }
    let shutdown = CancellationToken::new();
    let hub: Arc<dyn Hub> = Arc::new(ChannelHub {
        app: app.clone(),
        channel: name.clone(),
        settings,
        serial: tokio::sync::Mutex::new(()),
    });
    let dispatcher = Dispatcher {
        app: app.clone(),
        channel: channel.clone(),
        name,
        shutdown: shutdown.clone(),
    };
    let tasks = vec![
        tokio::spawn(channel.run(hub, shutdown.clone())),
        tokio::spawn(dispatcher.run()),
    ];
    Ok(ChannelHandle { shutdown, tasks })
}

fn failed(e: impl std::fmt::Display) -> HubError {
    HubError::Failed(e.to_string())
}

/// 顺着派活关系找对话所属的、这个渠道里的会话绑定（D-06）：对话自己有绑定就用它；
/// 没有就找它所属任务的派活对话，再看一次。找不到或绑在别的渠道上时返回 `None`。
async fn binding_for(store: &Store, channel: &str, thread: &ThreadId) -> Result<Option<Binding>, ip_store::StoreError> {
    let mut thread = thread.clone();
    for _ in 0..MAX_HOPS {
        if let Some(b) = store.binding_by_thread(&thread).await? {
            return Ok((b.channel == channel).then_some(b));
        }
        let Some(origin) = store.task_by_thread(&thread).await?.and_then(|t| t.origin_thread_id) else {
            return Ok(None);
        };
        thread = origin;
    }
    Ok(None)
}

fn card_thread(approval: &Approval) -> Option<&ThreadId> {
    match &approval.subject {
        ApprovalSubject::RouteMessage { thread_id, .. } | ApprovalSubject::AgentQuestion { thread_id, .. } => {
            Some(thread_id)
        }
        ApprovalSubject::Question => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Hub
// ---------------------------------------------------------------------------------------------

struct ChannelHub {
    app: App,
    channel: String,
    settings: ChannelSettings,
    /// 同一个渠道的消息和点击一件一件处理：查去重、办事、记去重之间不会插进同一件事的另一次重推（D-28）。
    serial: tokio::sync::Mutex<()>,
}

/// 渠道启动时一次最多接着处理多少个没处理完的信封。正常只有出错期间才会攒下，远到不了这个数。
const STASHED_LIMIT: usize = 100_000;

impl ChannelHub {
    fn store(&self) -> &Store {
        self.app.runtime().store()
    }

    /// 去重（D-03 第 1 步、D-28）：去重 key 记过就跳过；没记过就办 `work`，办完（成功、按规则忽略或拒绝）
    /// 才记下。内部错误不记，渠道重试时还能再办一次，不会被当成重复丢掉。
    async fn once(
        &self,
        key: &str,
        work: impl std::future::Future<Output = Result<(), HubError>>,
    ) -> Result<(), HubError> {
        let _serial = self.serial.lock().await;
        if self.store().inbound_seen(&self.channel, key).await.map_err(failed)? {
            return Ok(());
        }
        let result = work.await;
        if !matches!(result, Err(HubError::Failed(_))) {
            self.store()
                .remember_inbound(&self.channel, key)
                .await
                .map_err(failed)?;
        }
        result
    }

    async fn on_message(&self, inbound: Inbound) -> Result<(), HubError> {
        let key = inbound.dedupe_key.clone();
        self.once(&key, self.handle_message(inbound)).await
    }

    /// 对话里还没有消息：上次开会话时绑定写好了、第一条消息没写进去就出错了。
    async fn never_started(&self, thread: &ThreadId) -> Result<bool, HubError> {
        let messages = self.store().thread_messages(thread, None, 1).await.map_err(failed)?;
        Ok(messages.is_empty())
    }

    async fn handle_message(&self, inbound: Inbound) -> Result<(), HubError> {
        let allow = &self.settings.allow_users;
        if !allow.is_empty() && !allow.contains(&inbound.sender.external_id) {
            tracing::debug!(channel = %self.channel, user = %inbound.sender.external_id, "名单外的人，忽略");
            return Ok(());
        }
        let author = Author::Human {
            name: inbound.sender.display_name.clone(),
            via: Some(Via {
                channel: self.channel.clone(),
                user_id: inbound.sender.external_id.clone(),
            }),
        };
        let store = self.store();
        let conversation = &inbound.conversation.0;
        let existing = store.binding(&self.channel, conversation).await.map_err(failed)?;
        let binding = match existing {
            Some(b) if b.agent_id.is_none() && self.never_started(&b.thread_id).await? => {
                // 上次开会话出错，重试时接着开，不当成“等路由卡片”。
                self.start(b, inbound.text, author).await?;
                return Ok(());
            }
            Some(b) => b,
            None if !inbound.may_start => return Ok(()),
            None => {
                // 先写绑定再发消息：员工的回复可能很快，投递器读到时绑定必须已经在了。
                let claimed = store
                    .claim_binding(Binding {
                        channel: self.channel.clone(),
                        conversation: conversation.clone(),
                        thread_id: ThreadId::new(),
                        agent_id: self.settings.agent.clone(),
                        requester: inbound.sender.external_id.clone(),
                        created_at_ms: now_ms(),
                    })
                    .await
                    .map_err(failed)?;
                self.start(claimed, inbound.text, author).await?;
                return Ok(());
            }
        };
        match binding.agent_id {
            // 粘住原来的员工，不重新路由（D-03）。
            Some(agent) => {
                crate::work::send_as(&self.app, Some(agent), Some(binding.thread_id), inbound.text, author)
                    .await
                    .map_err(HubError::Failed)?;
            }
            // 路由卡片还没人选：只记进对话，等选定后员工一并看到。
            None => {
                self.app
                    .runtime()
                    .post(binding.thread_id, author, inbound.text)
                    .await
                    .map_err(failed)?;
            }
        }
        Ok(())
    }

    /// 用会话的第一条消息开启对话：发给固定员工或按路由分派，路由有结果时把员工补进绑定。
    async fn start(&self, binding: Binding, text: String, author: Author) -> Result<(), HubError> {
        let result = crate::work::send_as(
            &self.app,
            binding.agent_id.clone(),
            Some(binding.thread_id.clone()),
            text,
            author,
        )
        .await
        .map_err(HubError::Failed)?;
        if binding.agent_id.is_none()
            && let Some(agent) = &result.agent_id
        {
            self.store()
                .set_binding_agent(&self.channel, &binding.conversation, agent)
                .await
                .map_err(failed)?;
        }
        Ok(())
    }

    async fn on_click(&self, click: CardClick) -> Result<(), HubError> {
        let key = click.dedupe_key.clone();
        self.once(&key, self.handle_click(click)).await
    }

    async fn handle_click(&self, click: CardClick) -> Result<(), HubError> {
        let store = self.store();
        let Some(approval) = store.approval(&click.approval_id).await.map_err(failed)? else {
            return Err(HubError::Rejected("找不到这张卡片".into()));
        };
        let binding = match card_thread(&approval) {
            Some(thread) => binding_for(store, &self.channel, thread).await.map_err(failed)?,
            None => None,
        };
        let user = &click.sender.external_id;
        let allowed = match &self.settings.approvers {
            Approvers::Anyone => true,
            Approvers::Only(ids) => ids.contains(user),
            Approvers::Requester => binding.as_ref().is_some_and(|b| &b.requester == user),
        };
        if !allowed {
            let who = match (&self.settings.approvers, &binding) {
                (Approvers::Requester, Some(_)) => "只有提需求的人".to_string(),
                (Approvers::Requester, None) => "这张卡片不属于这个渠道里的会话，".to_string(),
                (Approvers::Only(_), _) => "只有指定的审批人".to_string(),
                (Approvers::Anyone, _) => unreachable!(),
            };
            return Err(HubError::Rejected(format!("{who}能选这张卡片")));
        }
        let by = format!("{}（{}:{}）", click.sender.display_name, self.channel, user);
        crate::work::resolve_approval(&self.app, &click.approval_id, click.choice, by)
            .await
            .map_err(HubError::Rejected)?;
        Ok(())
    }
}

/// 测试用：直接拿到一个渠道的 Hub，不起渠道和投递器，用来并发地交消息。
#[cfg(test)]
pub(crate) fn hub_for_tests(app: &App, channel: &str, settings: ChannelSettings) -> Arc<dyn Hub> {
    Arc::new(ChannelHub {
        app: app.clone(),
        channel: channel.to_string(),
        settings,
        serial: tokio::sync::Mutex::new(()),
    })
}

impl Hub for ChannelHub {
    fn message(&self, inbound: Inbound) -> ip_connectors::BoxFuture<'_, Result<(), HubError>> {
        Box::pin(self.on_message(inbound))
    }

    fn click(&self, click: CardClick) -> ip_connectors::BoxFuture<'_, Result<(), HubError>> {
        Box::pin(self.on_click(click))
    }

    fn stash(&self, id: &str, payload: &str) -> ip_connectors::BoxFuture<'_, Result<bool, HubError>> {
        let (id, payload) = (id.to_string(), payload.to_string());
        Box::pin(async move {
            self.store()
                .stash_envelope(&self.channel, &id, &payload)
                .await
                .map_err(failed)
        })
    }

    fn stashed(&self) -> ip_connectors::BoxFuture<'_, Result<Vec<Stashed>, HubError>> {
        Box::pin(async move {
            let envelopes = self
                .store()
                .pending_envelopes(&self.channel, STASHED_LIMIT)
                .await
                .map_err(failed)?;
            Ok(envelopes
                .into_iter()
                .map(|e| Stashed {
                    id: e.id,
                    payload: e.payload,
                    attempts: e.attempts,
                })
                .collect())
        })
    }

    fn settle(&self, id: &str, settle: Settle) -> ip_connectors::BoxFuture<'_, Result<(), HubError>> {
        let id = id.to_string();
        let settle = match settle {
            Settle::Done => ip_store::Settle::Done,
            Settle::Failed(e) => ip_store::Settle::Failed(e),
            Settle::GaveUp(e) => ip_store::Settle::GaveUp(e),
        };
        Box::pin(async move {
            self.store()
                .settle_envelope(&self.channel, &id, settle)
                .await
                .map_err(failed)
        })
    }
}

// ---------------------------------------------------------------------------------------------
// 投递器
// ---------------------------------------------------------------------------------------------

struct Dispatcher {
    app: App,
    channel: Arc<dyn Channel>,
    name: String,
    shutdown: CancellationToken,
}

/// 投递器为什么停下：被要求停止，或存储出错（打日志后过一会儿重来）。
enum Halt {
    Stopped,
    Store(ip_store::StoreError),
}

impl From<ip_store::StoreError> for Halt {
    fn from(e: ip_store::StoreError) -> Self {
        Self::Store(e)
    }
}

impl Dispatcher {
    fn store(&self) -> &Store {
        self.app.runtime().store()
    }

    async fn run(self) {
        let mut wake = self.app.runtime().subscribe();
        let mut pruned_at: Option<tokio::time::Instant> = None;
        loop {
            if pruned_at.is_none_or(|t| t.elapsed() >= Duration::from_secs(24 * 3600)) {
                let before = now_ms() - i64::try_from(INBOX_RETENTION.as_millis()).unwrap_or(i64::MAX);
                if let Err(e) = self.store().prune_inbound(before).await {
                    tracing::warn!(channel = %self.name, error = %e, "清理去重记录失败");
                }
                pruned_at = Some(tokio::time::Instant::now());
            }
            let idle = match self.drain().await {
                Ok(idle) => idle,
                Err(Halt::Stopped) => return,
                Err(Halt::Store(e)) => {
                    tracing::warn!(channel = %self.name, error = %e, "投递器读写存储失败，稍后重试");
                    true
                }
            };
            if !idle {
                continue;
            }
            // 有新事件的通知就醒；漏了通知（Lagged）也没关系，最多 POLL 之后再看一次。
            tokio::select! {
                () = self.shutdown.cancelled() => return,
                _ = wake.recv() => {}
                () = tokio::time::sleep(POLL) => {}
            }
        }
    }

    /// 处理一批事件。返回 true 表示已经读到末尾。
    async fn drain(&self) -> Result<bool, Halt> {
        let store = self.store();
        let cursor = store.cursor(&self.name).await?.unwrap_or(0);
        let events = store.events_after(cursor, BATCH).await?;
        let Some(last) = events.last().map(|e| e.seq) else {
            return Ok(true);
        };
        for event in &events {
            if self.shutdown.is_cancelled() {
                return Err(Halt::Stopped);
            }
            for (item, out) in self.plan(event).await? {
                self.deliver(item, out).await?;
            }
        }
        // 位置攒到批末再记，省得每条事件都写一次。中途停下的话下次会从这批开头重读，
        // 已发过的东西在 channel_deliveries 里有记录，不会重发。
        store.put_cursor(&self.name, last).await?;
        Ok(events.len() < BATCH)
    }

    /// 一条事件要发到这个渠道的东西（D-04 的表），每样带上防重发用的条目名。
    async fn plan(&self, event: &StoredEvent) -> Result<Vec<(String, Outbound)>, Halt> {
        let store = self.store();
        let out = match event.kind.as_str() {
            "message.appended" => {
                let Ok(message) = serde_json::from_str::<Message>(&event.payload) else {
                    return Ok(vec![]);
                };
                let Body::Text { text } = &message.body else {
                    return Ok(vec![]);
                };
                let wanted = match message.author {
                    Author::Agent { .. } => true,
                    Author::System => message.notice == Some(Notice::TurnFailed),
                    Author::Human { .. } => false,
                };
                if !wanted {
                    return Ok(vec![]);
                }
                // 只发绑定了外部会话的对话；执行对话里的往来留在内部（D-04、T10）。
                let Some(binding) = store
                    .binding_by_thread(&message.thread_id)
                    .await?
                    .filter(|b| b.channel == self.name)
                else {
                    return Ok(vec![]);
                };
                let conversation = ConversationKey(binding.conversation);
                let mut body = text.clone();
                let mut files = Vec::new();
                if let Author::Agent { agent_id } = message.author
                    && text.contains("reports/")
                {
                    let workspaces = self.app.workspaces.clone();
                    let source = text.clone();
                    let room = conversation.clone();
                    match tokio::task::spawn_blocking(move || {
                        report_delivery::prepare(&source, &workspaces, &agent_id, &room)
                    })
                    .await
                    {
                        Ok(prepared) => (body, files) = prepared,
                        Err(e) => tracing::warn!(error=%e,"准备报告失败"),
                    }
                }
                files.insert(
                    0,
                    (
                        format!("message:{}", message.id),
                        Outbound::Text {
                            conversation,
                            text: body,
                        },
                    ),
                );
                files
            }
            "approval.created" => {
                let Ok(approval) = serde_json::from_str::<Approval>(&event.payload) else {
                    return Ok(vec![]);
                };
                let Some(thread) = card_thread(&approval) else {
                    return Ok(vec![]);
                };
                match binding_for(store, &self.name, thread).await? {
                    Some(b) => vec![(
                        format!("card:{}", approval.id),
                        Outbound::Card {
                            conversation: ConversationKey(b.conversation),
                            approval,
                        },
                    )],
                    None => vec![],
                }
            }
            "approval.resolved" => {
                let Ok(approval) = serde_json::from_str::<Approval>(&event.payload) else {
                    return Ok(vec![]);
                };
                match store.delivery(&self.name, &format!("card:{}", approval.id)).await? {
                    Some(Delivery {
                        sent: true,
                        external_ref: Some(external_ref),
                        conversation,
                        ..
                    }) => vec![(
                        format!("resolved:{}", approval.id),
                        Outbound::CardResolved {
                            conversation: ConversationKey(conversation),
                            approval,
                            external_ref,
                        },
                    )],
                    _ => vec![],
                }
            }
            _ => vec![],
        };
        Ok(out)
    }

    /// 发一样东西。可重试的失败就退避重试，期间不往后走，保证顺序；永久失败记下来跳过。
    async fn deliver(&self, item: String, out: Outbound) -> Result<(), Halt> {
        let store = self.store();
        if store.delivery(&self.name, &item).await?.is_some() {
            return Ok(());
        }
        let mut backoff = FIRST_BACKOFF;
        loop {
            let record = |external_ref: Option<String>, error: Option<String>| Delivery {
                channel: self.name.clone(),
                item: item.clone(),
                conversation: out.conversation().0.clone(),
                sent: error.is_none(),
                external_ref,
                error,
                at_ms: now_ms(),
            };
            match self.channel.deliver(&out).await {
                Ok(done) => {
                    store.put_delivery(record(Some(done.external_ref), None)).await?;
                    return Ok(());
                }
                Err(DeliverError::Permanent(reason)) => {
                    tracing::warn!(channel = %self.name, %item, %reason, "发不出去，跳过");
                    store.put_delivery(record(None, Some(reason))).await?;
                    return Ok(());
                }
                Err(DeliverError::Retry { after, reason }) => {
                    let wait = after.unwrap_or(backoff).min(MAX_BACKOFF);
                    tracing::info!(channel = %self.name, %item, %reason, ?wait, "暂时发不出去，稍后重试");
                    tokio::select! {
                        () = self.shutdown.cancelled() => return Err(Halt::Stopped),
                        () = tokio::time::sleep(wait) => {}
                    }
                    backoff = (backoff * 2).min(MAX_BACKOFF);
                }
            }
        }
    }
}
