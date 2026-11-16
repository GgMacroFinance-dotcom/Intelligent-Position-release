//! 单个数字员工的任务循环。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ip_core::{Agent, Author, Body, Message, MessageId, Notice, Role, ThreadId, now_ms};
use ip_models::{ChatMessage, ChatRequest, StreamEvent, ToolCall, ToolSpec, Usage};
use ip_tools::{ToolOutput, Toolbox};
use tokio::sync::mpsc;

use crate::engine::{BoxFuture, Engine, McpGrant, TurnContext, TurnSink};
use crate::gateway::{OrgTools, TURN_TOKEN_GRACE};
use crate::{AgentSetup, RuntimeEvent, Shared, Work, memory, prompt};

pub(crate) struct Worker {
    shared: Arc<Shared>,
    agent: Agent,
    role: Role,
    toolbox: Option<Toolbox>,
    /// 组织、数据和外部 MCP 工具。
    org: OrgTools,
    /// 有引擎时每一轮交给它，不走内置循环。
    engine: Option<Arc<dyn Engine>>,
    /// 工具定义，和算它时 MCP 工具集的版本号。MCP 服务器在后台重连上之后版本号会变，
    /// 下一步重算；版本号不变时直接用缓存。
    tools: std::sync::Mutex<(u64, Vec<ToolSpec>)>,
    /// 与运行时共享的待办计数。
    pending: Arc<AtomicUsize>,
}

/// 一轮失败的原因，写进对话作为系统消息，并通过 `TurnFailed` 通知。
#[derive(Debug, thiserror::Error)]
enum TurnError {
    #[error("{0}")]
    Model(String),
    #[error("存储错误：{0}")]
    Store(#[from] ip_store::StoreError),
    #[error("超过 {0} 步仍未完成，已停止")]
    TooManySteps(u32),
    #[error("{0}")]
    Budget(String),
    #[error("{0}")]
    Runtime(#[from] crate::RuntimeError),
    #[error("{0}")]
    Engine(String),
}

impl Worker {
    pub fn new(shared: Arc<Shared>, setup: AgentSetup, pending: Arc<AtomicUsize>) -> Self {
        let org = OrgTools {
            data: setup.data,
            mcp: setup.mcp,
            memory: shared.config.memory.enabled_for(&setup.role.id),
        };
        let version = mcp_version(&org);
        let tools = all_specs(setup.toolbox.as_ref(), &org, &setup.role);
        Self {
            shared,
            agent: setup.agent,
            role: setup.role,
            toolbox: setup.toolbox,
            org,
            engine: setup.engine,
            tools: std::sync::Mutex::new((version, tools)),
            pending,
        }
    }

    /// 这一步发给模型的工具定义。MCP 工具集变了（后台重连上一个服务器）才重算。
    fn tool_specs(&self) -> Vec<ToolSpec> {
        let version = mcp_version(&self.org);
        let mut cached = self.tools.lock().unwrap_or_else(|p| p.into_inner());
        if cached.0 != version {
            *cached = (version, all_specs(self.toolbox.as_ref(), &self.org, &self.role));
        }
        cached.1.clone()
    }

    pub async fn run(self, mut inbox: mpsc::Receiver<Work>) {
        let mut batch: Vec<Work> = Vec::new();
        while let Some(first) = inbox.recv().await {
            // 忙的时候同一对话来了多条消息，合并成一轮处理（历史里已经包含它们）；任一条要求升级就升级。
            let mut received = 1;
            batch.push(first);
            while let Ok(more) = inbox.try_recv() {
                received += 1;
                match batch.iter_mut().find(|w| w.thread == more.thread) {
                    Some(w) => w.escalate |= more.escalate,
                    None => batch.push(more),
                }
            }
            for work in batch.drain(..) {
                self.turn(&work.thread, work.escalate).await;
            }
            self.pending.fetch_sub(received, Ordering::Relaxed);
        }
    }

    async fn turn(&self, thread: &ThreadId, escalate: bool) {
        let agent_id = self.agent.id.clone();
        self.shared.emit(RuntimeEvent::TurnStarted {
            agent_id: agent_id.clone(),
            thread_id: thread.clone(),
        });
        let mut usage = Usage::default();
        let mut cost_usd = 0.0;
        let began = std::time::Instant::now();
        let result = match &self.engine {
            Some(engine) => {
                self.engine_turn(engine.as_ref(), thread, &mut usage, &mut cost_usd)
                    .await
            }
            None => self.steps(thread, escalate, &mut usage, &mut cost_usd).await,
        };
        // 成功和失败的轮次都累计 token 用量；记不上只打日志，不影响这一轮的结果。
        if let Err(error) = self
            .shared
            .ledger
            .record_tokens(&self.shared.store, &self.role.id, usage)
            .await
        {
            tracing::warn!(agent = %agent_id, %error, "token 用量没记上");
        }
        match result {
            Ok(steps) => {
                let elapsed_ms = u64::try_from(began.elapsed().as_millis()).unwrap_or(u64::MAX);
                self.shared.emit(RuntimeEvent::TurnFinished {
                    agent_id,
                    thread_id: thread.clone(),
                    steps,
                    usage,
                    cost_usd,
                    elapsed_ms,
                });
            }
            Err(e) => {
                let error = e.to_string();
                tracing::warn!(agent = %agent_id, thread = %thread, %error, "一轮失败");
                let note = Message::notice(
                    thread.clone(),
                    Notice::TurnFailed,
                    format!("{} 的这轮工作失败：{error}", self.agent.name),
                );
                let _ = self.append(note).await;
                self.block_task(thread, &error).await;
                self.shared.emit(RuntimeEvent::TurnFailed {
                    agent_id,
                    thread_id: thread.clone(),
                    error,
                });
            }
        }
    }

    /// 这个对话是派给我的任务、而这一轮失败了：把任务标为受阻并写明原因，不让它一直挂着等结果。
    async fn block_task(&self, thread: &ThreadId, error: &str) {
        let task = match self.shared.store.task_by_thread(thread).await {
            Ok(Some(t)) => t,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(error = %e, "查任务失败");
                return;
            }
        };
        if task.assignee.as_ref() != Some(&self.agent.id) {
            return;
        }
        let id = task.id.clone();
        let reason = format!("{} 的工作中断：{error}", self.agent.name);
        if let Err(e) = self.shared.fail_task(task, reason).await {
            tracing::warn!(error = %e, task = %id, "没能把任务标为受阻");
        }
    }

    /// 返回调用模型的次数。
    async fn steps(
        &self,
        thread: &ThreadId,
        escalate: bool,
        usage: &mut Usage,
        cost: &mut f64,
    ) -> Result<u32, TurnError> {
        let slot = match (&self.role.model.escalate, escalate) {
            (Some(big), true) => big,
            _ => &self.role.model.default,
        };
        let (client, model) = self
            .shared
            .models
            .resolve(slot)
            .map_err(|e| TurnError::Model(e.to_string()))?;
        let roster = self.shared.roster(&self.agent.id);
        let extras = self.extras().await;
        let system = prompt::system_prompt(
            &self.shared.config.org_prompt,
            &self.role,
            &self.agent,
            &roster,
            &extras,
        );
        let max_steps = self.shared.config.max_steps.max(1);

        for step in 1..=max_steps {
            let mut history = self.load(thread).await?;
            let mut messages = self.build(&system, &history);
            self.check_budget().await?;
            let budget = self.shared.config.context_budget_chars;
            if budget > 0 && prompt::size_chars(&messages) > budget {
                let before = *usage;
                let compacted = self.compact(&history, client.as_ref(), model, thread, usage).await;
                self.charge(slot, before, *usage, cost).await?;
                match compacted {
                    Ok(true) => {
                        history = self.load(thread).await?;
                        messages = self.build(&system, &history);
                    }
                    Ok(false) => {}
                    // 压缩失败不影响这一轮，照原样发；端点若拒绝过长的请求，会作为模型错误报出来。
                    Err(e) => tracing::warn!(agent = %self.agent.id, thread = %thread, error = %e, "上下文压缩失败"),
                }
            }
            let mut request = ChatRequest::new(model, messages);
            request.tools = self.tool_specs();

            let before = *usage;
            let streamed = self.stream(client.stream(request), thread, usage).await;
            self.charge(slot, before, *usage, cost).await?;
            let (text, calls) = streamed?;
            if !text.is_empty() {
                self.append(self.message(thread, Body::Text { text })).await?;
            }
            if calls.is_empty() {
                return Ok(step);
            }
            for call in &calls {
                let input = prompt::arguments_value(&call.arguments);
                let body = Body::ToolCall {
                    call_id: call.id.clone(),
                    name: call.name.clone(),
                    input,
                };
                self.append(self.message(thread, body)).await?;
            }
            for call in calls {
                let out = self.call_tool(thread, &call).await;
                let body = Body::ToolResult {
                    call_id: call.id,
                    ok: out.ok,
                    output: out.output,
                };
                self.append(self.message(thread, body)).await?;
            }
        }
        Err(TurnError::TooManySteps(max_steps))
    }

    /// 依次交给组织工具、数据工具、MCP、工作目录工具；谁认得这个名字谁执行。
    async fn call_tool(&self, thread: &ThreadId, call: &ToolCall) -> ToolOutput {
        let (name, args) = (call.name.as_str(), call.arguments.as_str());
        if let Some(out) = self.org.call(&self.shared, &self.agent, thread, name, args).await {
            return out;
        }
        match &self.toolbox {
            Some(tb) => tb.call(name, args).await,
            None => ToolOutput::err(format!("没有工具 {name}")),
        }
    }

    async fn load(&self, thread: &ThreadId) -> Result<Vec<Message>, TurnError> {
        Ok(self
            .shared
            .store
            .thread_messages(thread, None, self.shared.config.history_limit)
            .await?)
    }

    fn build(&self, system: &str, history: &[Message]) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(history.len() + 1);
        messages.push(ChatMessage::System {
            content: system.to_string(),
        });
        messages.extend(prompt::history(
            &self.agent,
            history,
            self.shared.config.utc_offset_minutes,
        ));
        messages
    }

    /// 这个岗位的决策者笔记和岗位记忆摘要。读不出来时不带，这一轮照常进行。
    async fn extras(&self) -> memory::Extras {
        match memory::load(&self.shared.store, &self.shared.config.memory, &self.role.id).await {
            Ok(extras) => extras,
            Err(e) => {
                tracing::warn!(agent = %self.agent.id, error = %e, "读决策者笔记和岗位记忆失败，这一轮不带");
                memory::Extras::default()
            }
        }
    }

    /// 把较早的一段对话交给模型写成摘要，存成一条 `Summary` 消息。
    /// 保留最近约一半预算的原文；没有可压缩的内容时返回 `false`。
    async fn compact(
        &self,
        history: &[Message],
        client: &dyn ip_models::ModelClient,
        model: &str,
        thread: &ThreadId,
        usage: &mut Usage,
    ) -> Result<bool, TurnError> {
        let Some(range) = prompt::compaction_range(history, self.shared.config.context_budget_chars / 2) else {
            return Ok(false);
        };
        let covers_through = history[range.end - 1].id.clone();
        let mut request = ChatRequest::new(
            model,
            vec![
                ChatMessage::System {
                    content: prompt::SUMMARY_PROMPT.into(),
                },
                ChatMessage::User {
                    content: prompt::transcript(
                        prompt::latest_summary(history).map(|(t, _)| t),
                        &history[range],
                        self.shared.config.utc_offset_minutes,
                    ),
                },
            ],
        );
        request.tools = Vec::new();
        let (text, _) = self.collect(client.stream(request), usage).await?;
        let text = text.trim();
        if text.is_empty() {
            return Err(TurnError::Model("模型返回了空摘要".into()));
        }
        let summary = Message {
            id: MessageId::new(),
            thread_id: thread.clone(),
            at_ms: now_ms(),
            author: Author::System,
            body: Body::Summary {
                text: text.to_string(),
                covers_through,
            },
            notice: None,
        };
        self.append(summary).await?;
        Ok(true)
    }

    /// 岗位设了每日预算且今天已用满时，不再调用模型。
    async fn check_budget(&self) -> Result<(), TurnError> {
        let Some(limit) = self.role.budget_per_day_usd else {
            return Ok(());
        };
        let spent = self.shared.ledger.spent(&self.shared.store, &self.role.id).await?;
        if spent >= limit {
            return Err(TurnError::Budget(format!(
                "岗位 {} 今天的模型费用已达上限 ${limit:.2}（已用 ${spent:.4}），UTC 0 点后恢复；急用可调高 budget_per_day_usd",
                self.role.display_name()
            )));
        }
        Ok(())
    }

    /// 按价格把一次调用的用量记到岗位账上。
    async fn charge(&self, model: &str, before: Usage, after: Usage, cost: &mut f64) -> Result<(), TurnError> {
        let Some(price) = self.shared.models.price(model) else {
            return Ok(());
        };
        let delta = Usage {
            prompt_tokens: after.prompt_tokens - before.prompt_tokens,
            completion_tokens: after.completion_tokens - before.completion_tokens,
            cached_tokens: after.cached_tokens - before.cached_tokens,
            cache_write_tokens: after.cache_write_tokens - before.cache_write_tokens,
        };
        let usd = price.cost(&delta);
        *cost += usd;
        self.shared
            .ledger
            .charge(&self.shared.store, &self.role.id, usd)
            .await?;
        Ok(())
    }

    /// 读完一次模型输出，不广播（压缩用）。
    async fn collect(
        &self,
        mut rx: ip_models::EventStream,
        usage: &mut Usage,
    ) -> Result<(String, Vec<ToolCall>), TurnError> {
        let mut text = String::new();
        let mut calls = Vec::new();
        while let Some(event) = rx.recv().await {
            match event.map_err(|e| TurnError::Model(e.to_string()))? {
                StreamEvent::TextDelta(delta) => text.push_str(&delta),
                StreamEvent::ToolCall(call) => calls.push(call),
                StreamEvent::Done { usage: u, .. } => {
                    add_usage(usage, u);
                    return Ok((text, calls));
                }
            }
        }
        Err(TurnError::Model("模型输出意外中断".into()))
    }

    /// 读完一次模型输出，边读边广播文字片段。
    async fn stream(
        &self,
        mut rx: ip_models::EventStream,
        thread: &ThreadId,
        usage: &mut Usage,
    ) -> Result<(String, Vec<ToolCall>), TurnError> {
        let mut text = String::new();
        let mut calls = Vec::new();
        while let Some(event) = rx.recv().await {
            match event.map_err(|e| TurnError::Model(e.to_string()))? {
                StreamEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                    self.shared.emit(RuntimeEvent::Delta {
                        agent_id: self.agent.id.clone(),
                        thread_id: thread.clone(),
                        text: delta,
                    });
                }
                StreamEvent::ToolCall(call) => calls.push(call),
                StreamEvent::Done { usage: u, .. } => {
                    add_usage(usage, u);
                    return Ok((text, calls));
                }
            }
        }
        Err(TurnError::Model("模型输出意外中断".into()))
    }

    /// 把这一轮交给外部引擎（`docs/ip/m2-spec.md` D-20 到 D-25）。返回引擎报告的步数。
    ///
    /// 只把上一轮之后的新消息交给引擎（不含员工自己写的），引擎靠自己的会话记住之前的内容。
    /// 已交给引擎的位置记在引擎会话表里，只在这一轮成功后前移：失败的一轮下次会重新交一遍。
    async fn engine_turn(
        &self,
        engine: &dyn Engine,
        thread: &ThreadId,
        usage: &mut Usage,
        cost: &mut f64,
    ) -> Result<u32, TurnError> {
        self.check_budget().await?;
        let store = &self.shared.store;
        let session = store
            .engine_session(&self.agent.id, thread)
            .await?
            .filter(|s| s.engine == engine.kind());
        let history = self.load(thread).await?;
        let start = session
            .as_ref()
            .and_then(|s| history.iter().position(|m| m.id == s.last_message))
            .map_or(0, |i| i + 1);
        let me = Author::Agent {
            agent_id: self.agent.id.clone(),
        };
        let fresh: Vec<Message> = history[start..]
            .iter()
            .filter(|m| m.author != me && matches!(m.body, Body::Text { .. }))
            .cloned()
            .collect();
        let Some(last_seen) = history.last().map(|m| m.id.clone()) else {
            return Ok(0);
        };
        if fresh.is_empty() {
            return Ok(0);
        }
        let input = prompt::transcript(None, &fresh, self.shared.config.utc_offset_minutes);
        let roster = self.shared.roster(&self.agent.id);
        let extras = self.extras().await;
        let system = prompt::system_prompt(
            &self.shared.config.org_prompt,
            &self.role,
            &self.agent,
            &roster,
            &extras,
        );
        let url = self.shared.mcp_url.read().unwrap_or_else(|p| p.into_inner()).clone();
        let mcp = url.map(|url| McpGrant {
            url,
            token: self
                .shared
                .grants
                .issue(&self.agent.id, thread, engine.turn_timeout() + TURN_TOKEN_GRACE),
        });
        let sink = EngineSink { worker: self, thread };
        let report = engine
            .turn(TurnContext {
                agent: &self.agent,
                role: &self.role,
                thread,
                workspace: self.toolbox.as_ref().map(Toolbox::workspace),
                session: session.as_ref().map(|s| s.session_id.as_str()),
                session_usage: session.as_ref().map_or_else(Usage::default, |s| Usage {
                    prompt_tokens: s.used.prompt_tokens,
                    cached_tokens: s.used.cached_tokens,
                    cache_write_tokens: s.used.cache_write_tokens,
                    completion_tokens: s.used.completion_tokens,
                }),
                system_prompt: &system,
                input: &input,
                mcp: mcp.clone(),
                sink: &sink,
            })
            .await;
        if let Some(grant) = &mcp {
            self.shared.grants.end(&grant.token, TURN_TOKEN_GRACE);
        }
        add_usage(usage, Some(report.usage));
        *cost += report.cost_usd;
        self.shared.ledger.charge(store, &self.role.id, report.cost_usd).await?;
        // 会话的累计用量：引擎这一轮没报就沿用已存的。
        let used = report.session_usage.map_or_else(
            || session.as_ref().map(|s| s.used).unwrap_or_default(),
            |u| ip_store::SessionUsage {
                prompt_tokens: u.prompt_tokens,
                cached_tokens: u.cached_tokens,
                cache_write_tokens: u.cache_write_tokens,
                completion_tokens: u.completion_tokens,
            },
        );
        if let Some(error) = report.error {
            // 失败的一轮不前移 `last_message`（下次重新交一遍），但已经记过账的用量要存下来：
            // 续接时引擎报的是累计值，不存的话下一轮会把这一轮的用量再记一次。
            if let Some(s) = session.filter(|s| s.used != used) {
                store
                    .put_engine_session(ip_store::EngineSession {
                        used,
                        updated_at_ms: now_ms(),
                        ..s
                    })
                    .await?;
            }
            return Err(TurnError::Engine(error));
        }
        if let Some(session_id) = report.session_id.or(session.map(|s| s.session_id)) {
            store
                .put_engine_session(ip_store::EngineSession {
                    agent_id: self.agent.id.clone(),
                    thread_id: thread.clone(),
                    engine: engine.kind().to_string(),
                    session_id,
                    last_message: last_seen,
                    updated_at_ms: now_ms(),
                    used,
                })
                .await?;
        }
        Ok(report.steps.max(1))
    }

    fn message(&self, thread: &ThreadId, body: Body) -> Message {
        Message {
            id: MessageId::new(),
            thread_id: thread.clone(),
            at_ms: now_ms(),
            author: Author::Agent {
                agent_id: self.agent.id.clone(),
            },
            body,
            notice: None,
        }
    }

    async fn append(&self, message: Message) -> Result<(), TurnError> {
        self.shared.store.append_message(message.clone()).await?;
        self.shared.emit(RuntimeEvent::Message { message });
        Ok(())
    }
}

fn add_usage(total: &mut Usage, u: Option<Usage>) {
    if let Some(u) = u {
        total.prompt_tokens += u.prompt_tokens;
        total.completion_tokens += u.completion_tokens;
        total.cached_tokens += u.cached_tokens;
        total.cache_write_tokens += u.cache_write_tokens;
    }
}

/// 引擎写回对话的通道：以这位员工的名义写消息，和内置循环写的一样会被存储和广播。
struct EngineSink<'a> {
    worker: &'a Worker,
    thread: &'a ThreadId,
}

impl TurnSink for EngineSink<'_> {
    fn append(&self, body: Body) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            self.worker
                .append(self.worker.message(self.thread, body))
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// 内置工具（shell、文件）加组织、数据、MCP 工具。
fn all_specs(toolbox: Option<&Toolbox>, org: &OrgTools, role: &Role) -> Vec<ToolSpec> {
    let mut tools = toolbox.map(Toolbox::specs).unwrap_or_default();
    tools.extend(org.specs(&role.id));
    tools
}

fn mcp_version(org: &OrgTools) -> u64 {
    org.mcp.as_ref().map_or(0, |hub| hub.version())
}
