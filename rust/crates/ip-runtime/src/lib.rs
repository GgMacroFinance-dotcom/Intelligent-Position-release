//! 数字员工运行时：每个数字员工是一个 tokio 任务，不是一个 CLI 进程。
//!
//! 一轮工作：从存储读对话历史 → 拼提示词 → 流式调模型 → 执行工具 → 把每一步写回存储，
//! 直到模型不再调用工具或达到步数上限。过程中的事件（流式文字、新消息、结束）广播给订阅者，
//! WebSocket 服务就是订阅者之一。
//!
//! 空闲的数字员工只占一个挂起的任务和一个通道，几百个员工常驻一台机器没有压力。

mod agent;
mod budget;
pub mod engine;
mod gateway;
pub mod memory;
mod memory_tools;
mod prompt;
#[cfg(test)]
mod tests;
mod work;
mod work_tools;

pub use gateway::TURN_TOKEN_GRACE;
pub use work::{NewTask, TaskUpdate};

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock, Weak};

use ip_core::{Agent, AgentId, Approval, Author, Message, Notice, Role, RoleId, Task, TaskId, ThreadId};
use ip_models::{ModelRegistry, Usage};
use ip_store::Store;
use ip_tools::Toolbox;
use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// 运行时参数。
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    /// 所有岗位共用的组织规则，放在系统提示词最前面，让不同岗位的请求也能共享前缀缓存。
    pub org_prompt: String,
    /// 一轮里最多调用模型几次（每次可能带若干工具调用）。
    pub max_steps: u32,
    /// 每轮读入的最近消息数。
    pub history_limit: usize,
    /// 发给模型的上下文超过这么多字时，把较早的对话压缩成摘要。0 表示不压缩。
    pub context_budget_chars: usize,
    /// 岗位记忆：开关和各层的大小上限。
    pub memory: memory::MemoryConfig,
    /// 任务最多能委派几层（员工在执行对话里继续往下派活时加一层）。
    pub max_delegation_depth: u32,
    /// 每个员工收件箱的容量；满了以后 `send` 会等待。
    pub inbox_capacity: usize,
    /// 事件广播的缓冲；订阅者跟不上时会丢最旧的事件（收到 `Lagged`）。
    pub event_capacity: usize,
    /// 员工看到的消息时间按哪个时区显示，相对 UTC 的分钟数（北京时间是 480）。默认 0，即 UTC。
    pub utc_offset_minutes: i64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            org_prompt: String::new(),
            max_steps: 16,
            history_limit: 200,
            context_budget_chars: 60_000,
            memory: memory::MemoryConfig::default(),
            max_delegation_depth: 3,
            inbox_capacity: 64,
            event_capacity: 4096,
            utc_offset_minutes: 0,
        }
    }
}

/// 广播给订阅者的事件。按 `type` 字段区分，直接序列化给前端。
#[derive(Clone, Debug, PartialEq, Serialize, ts_rs::TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(export)]
pub enum RuntimeEvent {
    TurnStarted {
        agent_id: AgentId,
        thread_id: ThreadId,
    },
    /// 模型刚生成的一段文字。完整文字随后以 `Message` 事件给出。
    Delta {
        agent_id: AgentId,
        thread_id: ThreadId,
        text: String,
    },
    /// 一条消息已经写入存储。
    Message {
        message: Message,
    },
    TurnFinished {
        agent_id: AgentId,
        thread_id: ThreadId,
        steps: u32,
        usage: Usage,
        /// 这一轮按配置价格折算的美元费用；模型没配价格时为 0。
        cost_usd: f64,
        /// 从开始处理到结束，不含在收件箱里排队的时间。
        #[ts(type = "number")]
        elapsed_ms: u64,
    },
    TurnFailed {
        agent_id: AgentId,
        thread_id: ThreadId,
        error: String,
    },
    /// 任务新建或变更。
    TaskChanged {
        task: Task,
    },
    /// 审批卡片新建或被选定。
    ApprovalChanged {
        approval: Approval,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("数字员工 {0} 已存在")]
    DuplicateAgent(AgentId),
    #[error("找不到数字员工 {0}")]
    UnknownAgent(AgentId),
    #[error("数字员工 {0} 的岗位是 {1}，但传入的岗位是 {2}")]
    RoleMismatch(AgentId, String, String),
    #[error("数字员工 {0} 已停止")]
    AgentStopped(AgentId),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Store(#[from] ip_store::StoreError),
}

/// 创建一个数字员工所需的全部信息。
pub struct AgentSetup {
    pub agent: Agent,
    pub role: Role,
    /// 没有工具箱的员工没有工作目录（仍有组织工具和 MCP 工具）。
    pub toolbox: Option<Toolbox>,
    /// 组织配置的 MCP 服务器；按岗位过滤后给员工用。
    pub mcp: Option<Arc<ip_mcp::McpHub>>,
    /// 组织配置的数据源（`data_catalog`、`data_series`）；按岗位过滤。
    pub data: Option<Arc<ip_data::DataHub>>,
    /// 把每一轮交给外部引擎（如 Codex）；`None` 时用内置循环。
    pub engine: Option<Arc<dyn engine::Engine>>,
}

type AgentTable = RwLock<HashMap<AgentId, AgentEntry>>;

/// 员工任务共享的部分。员工表只持弱引用：收件箱归运行时所有，员工任务不会让自己的收件箱一直不关。
pub(crate) struct Shared {
    pub store: Store,
    pub models: ModelRegistry,
    pub events: broadcast::Sender<RuntimeEvent>,
    pub config: RuntimeConfig,
    pub directory: RwLock<work::Directory>,
    /// 岗位每日模型费用。
    pub ledger: budget::Ledger,
    /// 同一时间只跑一轮记忆提炼。
    pub memory_pass: tokio::sync::Mutex<()>,
    /// 已经处理完的对话和当时它最后一条消息的时间。没有新消息的对话下一轮直接跳过，不用再读一遍。
    /// 只在内存里，重启后重新看一遍。
    pub memory_seen: std::sync::Mutex<HashMap<ThreadId, i64>>,
    /// 外部引擎的本轮令牌。
    pub grants: gateway::Grants,
    /// `/mcp` 的地址；服务开始监听后才有。
    pub mcp_url: RwLock<Option<String>>,
    agents: Weak<AgentTable>,
}

impl Shared {
    pub fn emit(&self, event: RuntimeEvent) {
        // 没有订阅者时 send 返回错误，属于正常情况。
        let _ = self.events.send(event);
    }

    /// 只把消息写进对话，不通知任何员工。
    pub async fn post(&self, message: Message) -> Result<Message, RuntimeError> {
        self.store.append_message(message.clone()).await?;
        self.emit(RuntimeEvent::Message {
            message: message.clone(),
        });
        Ok(message)
    }

    /// 通知员工处理某个对话（对话里的新消息已经写好）。
    pub async fn dispatch(&self, to: &AgentId, thread_id: ThreadId, escalate: bool) -> Result<(), RuntimeError> {
        let (inbox, pending) = {
            let table = self
                .agents
                .upgrade()
                .ok_or_else(|| RuntimeError::AgentStopped(to.clone()))?;
            let agents = table.read().unwrap_or_else(|p| p.into_inner());
            agents
                .get(to)
                .map(|e| (e.inbox.clone(), e.pending.clone()))
                .ok_or_else(|| RuntimeError::UnknownAgent(to.clone()))?
        };
        pending.fetch_add(1, Ordering::Relaxed);
        if inbox
            .send(Work {
                thread: thread_id,
                escalate,
            })
            .await
            .is_err()
        {
            pending.fetch_sub(1, Ordering::Relaxed);
            return Err(RuntimeError::AgentStopped(to.clone()));
        }
        Ok(())
    }
}

/// 交给员工的一件工作：处理某个对话。
pub(crate) struct Work {
    pub thread: ThreadId,
    /// 这一轮用岗位的升级模型。
    pub escalate: bool,
}

struct AgentEntry {
    agent: Agent,
    inbox: mpsc::Sender<Work>,
    /// 已送达但还没处理完的工作数，用来挑最空闲的员工。
    pending: Arc<AtomicUsize>,
    /// 经 MCP 服务端开放给外部引擎的工具。
    tools: gateway::OrgTools,
    task: JoinHandle<()>,
}

/// 今天（UTC 自然日）的模型费用。
#[derive(Clone, Debug, PartialEq, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct SpendToday {
    /// `YYYY-MM-DD`。
    pub date: String,
    /// 岗位 id → 美元。今天还没花钱的岗位不列出。
    pub roles: std::collections::BTreeMap<RoleId, f64>,
    /// 语义分派调用模型花的美元，不占岗位预算。
    pub routing_usd: f64,
    /// 岗位 id → 今天的 token 用量，每轮结束时累计。提示词缓存命中率 = `cached_tokens / prompt_tokens`
    /// （`prompt_tokens` 已包含命中缓存的部分）。今天还没调过模型的岗位不列出。
    pub tokens: std::collections::BTreeMap<RoleId, Usage>,
    /// 语义分派的 token 用量。
    pub routing_tokens: Usage,
}

/// 运行时句柄，克隆开销很小。
#[derive(Clone)]
pub struct Runtime {
    shared: Arc<Shared>,
    agents: Arc<AgentTable>,
}

impl Runtime {
    pub fn new(store: Store, models: ModelRegistry, config: RuntimeConfig) -> Self {
        let (events, _) = broadcast::channel(config.event_capacity.max(1));
        let agents = Arc::new(RwLock::new(HashMap::new()));
        Self {
            shared: Arc::new(Shared {
                store,
                models,
                events,
                config,
                directory: RwLock::default(),
                ledger: budget::Ledger::default(),
                memory_pass: tokio::sync::Mutex::new(()),
                memory_seen: std::sync::Mutex::default(),
                grants: gateway::Grants::default(),
                mcp_url: RwLock::default(),
                agents: Arc::downgrade(&agents),
            }),
            agents,
        }
    }

    pub fn store(&self) -> &Store {
        &self.shared.store
    }

    /// 跑一轮记忆提炼：从闲置够久的对话里按岗位提炼摘要。没开记忆时什么都不做。
    /// `now_ms` 由调用方给，测试里可以拨快时间。
    pub async fn memory_pass(&self, now_ms: i64) -> Result<memory::extract::PassReport, RuntimeError> {
        memory::extract::run_pass(&self.shared, now_ms).await
    }

    /// 提炼和合并用的那把锁。人在控制台改记忆时先拿它，免得正在跑的一轮合并把刚删掉的内容又写回去。
    pub async fn memory_lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.shared.memory_pass.lock().await
    }

    /// 这个岗位开没开记忆。
    pub fn memory_enabled_for(&self, role: &RoleId) -> bool {
        self.shared.config.memory.enabled_for(role)
    }

    /// 跑一轮岗位记忆合并：选中的对话摘要有变化的岗位，各合并出新的一版记忆摘要。
    pub async fn memory_consolidate(
        &self,
        now_ms: i64,
    ) -> Result<memory::consolidate::ConsolidateReport, RuntimeError> {
        memory::consolidate::run_pass(&self.shared, now_ms).await
    }

    /// 后台多久跑一轮记忆提炼；没开记忆时是 `None`。
    pub fn memory_interval(&self) -> Option<std::time::Duration> {
        let memory = &self.shared.config.memory;
        memory
            .enabled
            .then(|| std::time::Duration::from_secs(memory.interval_minutes.max(1).saturating_mul(60)))
    }

    /// 今天（UTC）各岗位和语义分派已花的模型费用。
    pub async fn spend_today(&self) -> Result<SpendToday, RuntimeError> {
        let budget::Today {
            date,
            mut spent,
            mut tokens,
        } = self.shared.ledger.today(&self.shared.store).await?;
        let routing = RoleId::from(budget::ROUTING);
        let routing_usd = spent.remove(&routing).unwrap_or(0.0);
        let routing_tokens = tokens.remove(&routing).unwrap_or_default();
        Ok(SpendToday {
            date,
            roles: spent.into_iter().collect(),
            routing_usd,
            tokens: tokens.into_iter().collect(),
            routing_tokens,
        })
    }

    /// 服务端语义分派调用了一次模型：按价格记到“自动分派”名下，返回这次的美元数。
    /// 模型没登记价格时不计费，返回 0；token 用量照记。
    pub async fn charge_routing(&self, model: &str, usage: &Usage) -> Result<f64, RuntimeError> {
        let routing = RoleId::from(budget::ROUTING);
        self.shared
            .ledger
            .record_tokens(&self.shared.store, &routing, *usage)
            .await?;
        let Some(price) = self.shared.models.price(model) else {
            return Ok(0.0);
        };
        let usd = price.cost(usage);
        self.shared.ledger.charge(&self.shared.store, &routing, usd).await?;
        Ok(usd)
    }

    /// 配置的模型端点（服务端做语义路由时用）。
    pub fn models(&self) -> &ModelRegistry {
        &self.shared.models
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.shared.events.subscribe()
    }

    /// 启动一个数字员工。
    pub fn spawn_agent(&self, setup: AgentSetup) -> Result<(), RuntimeError> {
        if setup.agent.role != setup.role.id {
            return Err(RuntimeError::RoleMismatch(
                setup.agent.id.clone(),
                setup.agent.role.to_string(),
                setup.role.id.to_string(),
            ));
        }
        let mut agents = self.agents.write().unwrap_or_else(|p| p.into_inner());
        if agents.contains_key(&setup.agent.id) {
            return Err(RuntimeError::DuplicateAgent(setup.agent.id));
        }
        let (inbox, rx) = mpsc::channel(self.shared.config.inbox_capacity.max(1));
        let agent = setup.agent.clone();
        {
            let mut dir = self.shared.directory.write().unwrap_or_else(|p| p.into_inner());
            dir.roles
                .entry(setup.role.id.clone())
                .or_insert_with(|| setup.role.clone());
            dir.agents.insert(agent.id.clone(), agent.clone());
        }
        let pending = Arc::new(AtomicUsize::new(0));
        let tools = gateway::OrgTools {
            data: setup.data.clone(),
            mcp: setup.mcp.clone(),
            memory: self.shared.config.memory.enabled_for(&setup.role.id),
        };
        let worker = agent::Worker::new(self.shared.clone(), setup, pending.clone());
        let task = tokio::spawn(worker.run(rx));
        agents.insert(
            agent.id.clone(),
            AgentEntry {
                agent,
                inbox,
                pending,
                tools,
                task,
            },
        );
        Ok(())
    }

    /// 当前所有数字员工。
    pub fn agents(&self) -> Vec<Agent> {
        let agents = self.agents.read().unwrap_or_else(|p| p.into_inner());
        let mut list: Vec<Agent> = agents.values().map(|e| e.agent.clone()).collect();
        list.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        list
    }

    /// 岗位里手上待办最少的员工；一样多时取 id 最小的。岗位没有员工时返回 None。
    pub fn least_busy(&self, role: &RoleId) -> Option<AgentId> {
        let agents = self.agents.read().unwrap_or_else(|p| p.into_inner());
        agents
            .values()
            .filter(|e| &e.agent.role == role)
            .min_by(|a, b| {
                let (pa, pb) = (a.pending.load(Ordering::Relaxed), b.pending.load(Ordering::Relaxed));
                pa.cmp(&pb).then_with(|| a.agent.id.as_str().cmp(b.agent.id.as_str()))
            })
            .map(|e| e.agent.id.clone())
    }

    /// 把一条文字消息发给某个数字员工：先写入存储，再通知员工处理该对话。
    /// 返回写入的消息；员工的回复通过事件异步送达。
    pub async fn send(
        &self,
        to: &AgentId,
        thread_id: ThreadId,
        author: Author,
        text: impl Into<String>,
    ) -> Result<Message, RuntimeError> {
        self.send_with(to, thread_id, author, text, false).await
    }

    /// 同 `send`；`escalate` 为 true 时这一轮用岗位的升级模型（路由规则决定）。
    pub async fn send_with(
        &self,
        to: &AgentId,
        thread_id: ThreadId,
        author: Author,
        text: impl Into<String>,
        escalate: bool,
    ) -> Result<Message, RuntimeError> {
        if !self.has_agent(to) {
            return Err(RuntimeError::UnknownAgent(to.clone()));
        }
        let message = self.post(thread_id.clone(), author, text).await?;
        self.dispatch(to, thread_id, escalate).await?;
        Ok(message)
    }

    /// 只把消息写进对话，不通知任何员工（例如等人决定交给谁）。
    pub async fn post(
        &self,
        thread_id: ThreadId,
        author: Author,
        text: impl Into<String>,
    ) -> Result<Message, RuntimeError> {
        self.shared.post(Message::text(thread_id, author, text)).await
    }

    /// 写一条带类型的系统消息，不通知任何员工。
    pub async fn post_notice(
        &self,
        thread_id: ThreadId,
        notice: Notice,
        text: impl Into<String>,
    ) -> Result<Message, RuntimeError> {
        self.shared.post(Message::notice(thread_id, notice, text)).await
    }

    /// 通知员工处理某个对话（对话里的新消息已经写好）。
    pub async fn dispatch(&self, to: &AgentId, thread_id: ThreadId, escalate: bool) -> Result<(), RuntimeError> {
        self.shared.dispatch(to, thread_id, escalate).await
    }

    pub fn has_agent(&self, id: &AgentId) -> bool {
        self.agents.read().unwrap_or_else(|p| p.into_inner()).contains_key(id)
    }

    /// 广播一个事件（任务、审批等由服务层产生的变更）。
    pub fn publish(&self, event: RuntimeEvent) {
        self.shared.emit(event);
    }

    /// 登记一个岗位。没有员工的岗位也可以挂任务。启动员工时会自动登记它的岗位。
    pub fn add_role(&self, role: Role) {
        let mut dir = self.shared.directory.write().unwrap_or_else(|p| p.into_inner());
        dir.roles.insert(role.id.clone(), role);
    }

    /// 新建任务。指定了负责人时岗位取负责人的岗位；岗位或员工不存在时返回 `Invalid`。
    pub async fn create_task(&self, new: NewTask, created_by: Author) -> Result<Task, RuntimeError> {
        self.shared.create_task(new, created_by).await
    }

    /// 改任务状态、负责人或结果。换了负责人会通知新负责人；任务结束会回报派活的对话。
    pub async fn update_task(&self, id: &TaskId, update: TaskUpdate, by: &Author) -> Result<Task, RuntimeError> {
        self.shared.update_task(id, update, by).await
    }

    /// 发一张审批卡片。
    pub async fn open_approval(&self, approval: Approval) -> Result<Approval, RuntimeError> {
        self.shared.open_approval(approval).await
    }

    /// 停止所有员工：关闭收件箱，等正在进行的一轮做完。
    pub async fn shutdown(&self) {
        let tasks: Vec<JoinHandle<()>> = {
            let mut agents = self.agents.write().unwrap_or_else(|p| p.into_inner());
            agents.drain().map(|(_, e)| e.task).collect()
        };
        for t in tasks {
            let _ = t.await;
        }
    }
}
