//! 由配置文件组装出运行中的服务：存储、模型端点、沙箱、数字员工。

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, bail};
use ip_core::{AgentId, OrgSpec, RoleId};
use ip_models::{ModelRegistry, OpenAiCompatible, OpenAiConfig};
use ip_router::{Router, RouterSpec};
use ip_runtime::{AgentSetup, Runtime, RuntimeConfig};
use ip_store::Store;
use ip_tools::{Limits, Sandbox, Toolbox};

use crate::config::{Config, SandboxMode};

/// 服务的共享状态，克隆开销很小。
#[derive(Clone)]
pub struct App {
    pub(crate) runtime: Runtime,
    pub(crate) org: Arc<OrgSpec>,
    pub(crate) token: Option<Arc<str>>,
    /// 除同源外允许连 `/ws` 的来源（`ip.yaml` 的 `allowed_origins`）。
    pub(crate) allowed_origins: Arc<[crate::origin::Origin]>,
    /// 当前生效的路由规则；控制台保存后整体替换。
    router: Arc<std::sync::RwLock<Arc<Router>>>,
    /// 规则是否来自控制台保存（否则来自 ip.yaml）。
    pub(crate) rules_from_console: Arc<std::sync::atomic::AtomicBool>,
    /// 串行化规则和草案的读改写。
    pub(crate) rules_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) mcp: Option<Arc<ip_mcp::McpHub>>,
    /// 服务关闭时置为 true，让 WebSocket 会话主动断开，否则优雅关闭会一直等长连接。
    pub(crate) closing: Arc<tokio::sync::watch::Sender<bool>>,
    /// 接上的外部渠道；关闭时先停掉它们。
    channels: Arc<tokio::sync::Mutex<Vec<crate::channels::ChannelHandle>>>,
    pub(crate) schedules: Arc<crate::schedule::Schedules>,
    /// 有工作目录的员工和他们的工作目录，控制台从这里读报告（backlog F-4）。
    pub(crate) workspaces: Arc<std::collections::HashMap<ip_core::AgentId, std::path::PathBuf>>,
    /// 编好号、留着备补发的事件（`docs/ip/m2-spec.md` D-29）。转发任务在第一个 `/ws` 连接进来时启动。
    pub(crate) events: Arc<crate::events::EventLog>,
    /// 配对过的设备和设备令牌（backlog F-7）。
    pub(crate) devices: Arc<crate::devices::Devices>,
    /// 通知规则和各设备的通知设置（backlog F-9）。服务启动时（或第一个 `/ws` 连接进来时）启动规则任务。
    pub(crate) notifier: Arc<crate::notify::Notifier>,
    /// Web Push 的密钥和订阅（backlog F-10）。
    pub(crate) push: Arc<crate::webpush::WebPush>,
}

impl App {
    /// 测试和嵌入使用：直接给出运行时。
    pub fn new(runtime: Runtime, org: OrgSpec, token: Option<String>) -> Self {
        let (closing, _) = tokio::sync::watch::channel(false);
        let store = runtime.store().clone();
        Self {
            runtime,
            org: Arc::new(org),
            token: token.map(Arc::from),
            allowed_origins: Arc::new([]),
            router: Arc::new(std::sync::RwLock::new(Arc::new(
                Router::new(RouterSpec::default(), &OrgSpec::default(), &[]).expect("空规则总是合法"),
            ))),
            rules_from_console: Arc::default(),
            rules_lock: Arc::default(),
            mcp: None,
            closing: Arc::new(closing),
            channels: Arc::default(),
            schedules: Arc::default(),
            workspaces: Arc::default(),
            events: Arc::default(),
            devices: Arc::new(crate::devices::Devices::new(store.clone())),
            notifier: Arc::new(crate::notify::Notifier::new(store.clone())),
            push: Arc::new(crate::webpush::WebPush::new(store, None)),
        }
    }

    /// 除同源外还允许这些来源连 `/ws`。
    #[must_use]
    pub fn with_allowed_origins(mut self, origins: Vec<crate::origin::Origin>) -> Self {
        self.allowed_origins = origins.into();
        self
    }

    /// 告诉服务各员工的工作目录，控制台才能读他们写的报告。
    #[must_use]
    pub fn with_workspaces(
        mut self,
        workspaces: std::collections::HashMap<ip_core::AgentId, std::path::PathBuf>,
    ) -> Self {
        self.workspaces = Arc::new(workspaces);
        self
    }

    /// 换上定时任务（还要调用 [`App::start_schedules`] 才会到点运行）。
    #[must_use]
    pub fn with_schedules(mut self, schedules: crate::schedule::Schedules) -> Self {
        self.schedules = Arc::new(schedules);
        self
    }

    /// 启动通知规则和 Web Push：页面都关着时也要推（backlog F-10）。
    pub fn start_notifications(&self) {
        self.notifier.start(self.runtime.clone());
        self.push.start(&self.notifier);
    }

    /// 启动定时任务的后台调度，服务关闭时停止。
    pub fn start_schedules(&self) {
        crate::schedule::spawn(self.clone());
    }

    /// 启动岗位记忆的后台提炼和合并（`docs/ip/context-memory-design.md` 5.3、5.4）。没开记忆时什么都不做，服务关闭时停止。
    /// 启动后先跑一轮，之后每隔 `memory.interval_minutes` 跑一轮；每轮先提炼再合并。
    pub fn start_memory(&self) {
        let Some(interval) = self.runtime.memory_interval() else {
            return;
        };
        let runtime = self.runtime.clone();
        let mut closing = self.closing.subscribe();
        tokio::spawn(async move {
            loop {
                match runtime.memory_pass(ip_core::now_ms()).await {
                    Ok(report) if report != ip_runtime::memory::extract::PassReport::default() => {
                        tracing::info!(
                            extracted = report.extracted,
                            empty = report.empty,
                            failed = report.failed,
                            skipped_budget = report.skipped_budget,
                            "记忆提炼跑了一轮"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "记忆提炼这一轮出错，下一轮再试"),
                }
                match runtime.memory_consolidate(ip_core::now_ms()).await {
                    Ok(report) if report != ip_runtime::memory::consolidate::ConsolidateReport::default() => {
                        tracing::info!(
                            consolidated = report.consolidated,
                            failed = report.failed,
                            skipped_budget = report.skipped_budget,
                            "记忆合并跑了一轮"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(error = %e, "记忆合并这一轮出错，下一轮再试"),
                }
                tokio::select! {
                    () = tokio::time::sleep(interval) => {}
                    _ = closing.wait_for(|c| *c) => return,
                }
            }
        });
    }

    /// 换上路由规则。
    #[must_use]
    pub fn with_router(self, router: Router) -> Self {
        self.set_router(router);
        self
    }

    /// 当前生效的路由规则。
    pub fn router(&self) -> Arc<Router> {
        self.router.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub(crate) fn set_router(&self, router: Router) {
        *self.router.write().unwrap_or_else(|p| p.into_inner()) = Arc::new(router);
    }

    /// 停掉外部渠道和员工，再关闭 MCP 连接。
    pub async fn shutdown(&self) {
        for handle in self.channels.lock().await.drain(..) {
            handle.stop().await;
        }
        self.runtime.shutdown().await;
        if let Some(hub) = &self.mcp {
            hub.shutdown().await;
        }
    }

    /// 通知所有 WebSocket 会话断开。
    pub fn close(&self) {
        self.closing.send_replace(true);
    }

    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub fn requires_token(&self) -> bool {
        self.token.is_some()
    }

    /// 按配置启动全部数字员工。密钥从环境变量读取，缺失时报错退出。
    pub async fn from_config(config: &Config) -> anyhow::Result<Self> {
        let token = match &config.auth_token_env {
            Some(var) => Some(env(var).with_context(|| "auth_token_env 指定的访问令牌")?),
            None => None,
        };
        let allowed_origins = config.allowed_origins()?;

        let mut models = ModelRegistry::new();
        for (name, ep) in &config.endpoints {
            let mut oc = OpenAiConfig::new(&ep.base_url);
            if let Some(var) = &ep.api_key_env {
                oc.api_key = Some(env(var).with_context(|| format!("端点 {name} 的 API key"))?);
            }
            if let Some(n) = ep.max_concurrency {
                oc.max_concurrency = n.max(1);
            }
            oc.api = ep.api;
            oc.reasoning_effort = ep.reasoning_effort.clone();
            let client = OpenAiCompatible::new(oc).with_context(|| format!("创建端点 {name}"))?;
            models.insert(name.clone(), Arc::new(client));
            for (model, price) in &ep.prices {
                models.set_price(format!("{name}/{model}"), *price);
            }
        }

        std::fs::create_dir_all(&config.data_dir)
            .with_context(|| format!("创建数据目录 {}", config.data_dir.display()))?;
        let store = Store::open(config.data_dir.join("ip.db")).context("打开数据库")?;

        let sandbox = match config.sandbox.mode {
            SandboxMode::Bubblewrap => Sandbox::bubblewrap(config.sandbox.network)
                .await
                .context("工具沙箱不可用；开发机上可在配置里写 sandbox: { mode: none }")?,
            SandboxMode::None => {
                tracing::warn!("工具沙箱已关闭：数字员工的 shell 命令直接在本机执行");
                Sandbox::None
            }
        };

        let mut rc = RuntimeConfig {
            org_prompt: config.org_prompt.clone(),
            memory: config.memory.clone(),
            utc_offset_minutes: config.utc_offset_minutes()?,
            ..RuntimeConfig::default()
        };
        if let Some(n) = config.max_steps {
            rc.max_steps = n;
        }
        if let Some(n) = config.context_budget_chars {
            rc.context_budget_chars = n;
        }
        let mcp = if config.mcp_servers.is_empty() {
            None
        } else {
            tracing::warn!("MCP 服务器以子进程运行，不在工具沙箱里，权限等同本服务");
            let hub = Arc::new(ip_mcp::McpHub::connect(&config.mcp_servers).await?);
            // 没连上的在后台重连（默认 5 次）；连上后员工的下一步就能用它的工具。
            hub.spawn_reconnect(
                &config.mcp_servers,
                Arc::new(|var: &str| std::env::var(var).ok()),
                ip_mcp::Reconnect::default(),
            );
            Some(hub)
        };
        let data = if config.data_sources.is_empty() {
            None
        } else {
            Some(Arc::new(ip_data::DataHub::open(
                &config.data_sources,
                std::path::Path::new("."),
            )?))
        };
        let mut engines: BTreeMap<String, Arc<dyn ip_runtime::engine::Engine>> = BTreeMap::new();
        for (role, ec) in &config.engines {
            let api_key = match ec.api_key_env() {
                Some(var) => Some(env(var).with_context(|| format!("岗位 {role} 的引擎 API key"))?),
                None => None,
            };
            let home_root = config.data_dir.join("engines");
            let engine: Arc<dyn ip_runtime::engine::Engine> = match ec {
                ip_engines::EngineConfig::Codex(c) => ip_engines::CodexEngine::new(c.clone(), api_key, home_root),
            };
            tracing::info!(role = %role, "岗位使用外部引擎");
            engines.insert(role.clone(), engine);
        }
        let org = config.org();
        let runtime = Runtime::new(store, models, rc);
        for role in &org.roles {
            runtime.add_role(role.clone());
        }
        let mut workspaces = std::collections::HashMap::new();
        for a in &config.agents {
            let Some(role) = org.role(&RoleId::from(a.role.as_str())) else {
                bail!("数字员工 {} 的岗位 {} 不存在", a.id, a.role);
            };
            let toolbox = if a.tools {
                let ws = match &a.workspace {
                    Some(ws) => {
                        if !ws.is_dir() {
                            bail!("数字员工 {} 的工作目录 {} 不存在", a.id, ws.display());
                        }
                        ws.clone()
                    }
                    None => {
                        let ws = config.data_dir.join("workspaces").join(&a.id);
                        std::fs::create_dir_all(&ws).with_context(|| format!("创建工作目录 {}", ws.display()))?;
                        ws
                    }
                };
                let toolbox = Toolbox::new(&ws, sandbox.clone(), Limits::default())?;
                workspaces.insert(ip_core::AgentId::from(a.id.as_str()), ws);
                Some(toolbox)
            } else {
                None
            };
            runtime.spawn_agent(AgentSetup {
                agent: a.agent(),
                role: role.clone(),
                toolbox,
                mcp: mcp.clone(),
                data: data.clone(),
                engine: engines.get(&a.role).cloned(),
            })?;
        }
        tracing::info!(agents = config.agents.len(), roles = config.roles.len(), "组织已启动");
        let router = config.router()?;
        let mut app = Self::new(runtime, org, token)
            .with_allowed_origins(allowed_origins)
            .with_router(router)
            .with_schedules(config.schedules()?)
            .with_workspaces(workspaces);
        app.mcp = mcp;
        app.devices
            .load()
            .await
            .map_err(anyhow::Error::msg)
            .context("读设备列表")?;
        app.notifier
            .load()
            .await
            .map_err(anyhow::Error::msg)
            .context("读通知设置")?;
        app.push = Arc::new(crate::webpush::WebPush::new(
            app.runtime.store().clone(),
            config.push.contact.clone(),
        ));
        app.push
            .load()
            .await
            .map_err(anyhow::Error::msg)
            .context("读推送订阅和 VAPID 密钥")?;
        crate::rules::load_saved(&app).await;
        for (name, channel) in &config.channels {
            let crate::config::ChannelConfig::Slack(slack) = channel;
            let (channel, settings) = slack_channel(name, slack, env)?;
            let handle = crate::channels::start(&app, channel, settings)
                .await
                .map_err(|e| anyhow::anyhow!("接入渠道 {name} 失败：{e}"))?;
            tracing::info!(channel = %name, "Slack 渠道已接入");
            app.channels.lock().await.push(handle);
        }
        crate::schedule::load_last(&app).await;
        Ok(app)
    }
}

/// 按配置建 Slack 渠道和它的接入设置。`env` 读环境变量（测试里可以换掉）。
pub(crate) fn slack_channel(
    name: &str,
    config: &crate::config::SlackChannelConfig,
    env: impl Fn(&str) -> anyhow::Result<String>,
) -> anyhow::Result<(
    Arc<ip_connectors::slack::SlackChannel>,
    crate::channels::ChannelSettings,
)> {
    let bot_token = env(&config.bot_token_env).with_context(|| format!("渠道 {name} 的机器人令牌"))?;
    let app_token = env(&config.app_token_env).with_context(|| format!("渠道 {name} 的应用级令牌"))?;
    let channel = ip_connectors::slack::SlackChannel::new(ip_connectors::slack::SlackConfig {
        name: name.to_string(),
        bot_token,
        app_token,
        api_url: config
            .api_url
            .clone()
            .unwrap_or_else(|| ip_connectors::slack::DEFAULT_API_URL.to_string()),
        allow_channels: config.allow.channels.clone(),
        retry: ip_connectors::slack::RetryPolicy::default(),
    });
    let settings = crate::channels::ChannelSettings {
        allow_users: config.allow.users.clone(),
        approvers: config.approvers.to_rule().unwrap_or_default(),
        agent: config.agent.as_deref().map(AgentId::from),
    };
    Ok((channel, settings))
}

fn env(var: &str) -> anyhow::Result<String> {
    match std::env::var(var) {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => bail!("环境变量 {var} 未设置"),
    }
}
