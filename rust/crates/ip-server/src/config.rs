//! `ip.yaml`：服务的全部配置，一个文件描述整个组织。
//!
//! ```yaml
//! data_dir: ./data                 # 相对路径以配置文件所在目录为准
//! auth_token_env: IP_TOKEN         # 访问令牌所在的环境变量；监听非本机地址时必填
//! allowed_origins: [https://ip.example.com]  # 反向代理改写了 Host 时，控制台对外的地址
//! push: { contact: mailto:ops@example.com }   # Web Push 的联系方式，推送服务有问题时找谁；Safari 要有
//! utc_offset: "+08:00"             # 员工看到的消息时间按这个时区显示；省略时是 UTC
//! org_prompt: |
//!   你在一家 AI 原生的研究机构工作。结论先行，引用数据注明来源。
//! sandbox: { mode: bubblewrap, network: false }
//! endpoints:
//!   deepseek: { base_url: https://api.deepseek.com/v1, api_key_env: DEEPSEEK_API_KEY, max_concurrency: 16 }
//!   local:    { base_url: http://gpu-box:8000/v1 }
//! roles:
//!   - id: macro-researcher
//!     name: 宏观研究员
//!     description: 跟踪宏观数据，撰写研究报告。
//!     model: { default: deepseek/deepseek-chat }
//! agents:
//!   - { id: macro-1, role: macro-researcher, name: 小宏 }
//! ```

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use ip_core::{Agent, OrgSpec, Role, RoleId};
use ip_router::{Router, RouterSpec};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
    #[serde(default)]
    pub auth_token_env: Option<String>,
    /// 除同源外，还允许从哪些地址打开的控制台连 `/ws`（`scheme://主机[:端口]`）。
    /// 反向代理把 `Host` 改成上游地址时要写上对外的地址，否则浏览器的连接会被拒绝。
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub org_prompt: String,
    /// 员工看到的消息时间按哪个时区显示，写成相对 UTC 的偏移，如 `+08:00`。默认 `+00:00`（UTC）。
    /// 只管显示；每日预算仍按 UTC 0 点重置，定时任务用各自的 `utc_offset`。
    #[serde(default)]
    pub utc_offset: Option<String>,
    #[serde(default)]
    pub sandbox: SandboxConfig,
    #[serde(default)]
    pub max_steps: Option<u32>,
    /// 上下文超过这么多字时压缩较早的对话；0 表示不压缩。
    #[serde(default)]
    pub context_budget_chars: Option<usize>,
    /// 岗位记忆：默认关（`docs/ip/context-memory-design.md`）。
    #[serde(default)]
    pub memory: ip_runtime::memory::MemoryConfig,
    #[serde(default)]
    pub endpoints: BTreeMap<String, EndpointConfig>,
    /// 外部 MCP 服务器：`command`（stdio 子进程，不在沙箱里）或 `url`（远程，Streamable HTTP）。
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, ip_mcp::ServerConfig>,
    /// 数据源：员工用 `data_catalog`、`data_series` 查数据。
    #[serde(default)]
    pub data_sources: BTreeMap<String, ip_data::SourceConfig>,
    #[serde(default)]
    pub roles: Vec<Role>,
    /// 外部渠道：键是渠道名，比如 `slack`（`docs/ip/m2-spec.md` D-16）。
    #[serde(default)]
    pub channels: BTreeMap<String, ChannelConfig>,
    /// 外部引擎：键是岗位 id，这个岗位的每一轮交给引擎，不走内置循环（`docs/ip/m2-spec.md` D-21）。
    #[serde(default)]
    pub engines: BTreeMap<String, ip_engines::EngineConfig>,
    #[serde(default)]
    pub agents: Vec<AgentConfig>,
    /// 路由规则：没指定员工的消息按它分派。
    #[serde(default)]
    pub router: RouterSpec,
    /// 定时任务：到点给岗位或员工发一条消息。
    #[serde(default)]
    pub schedules: Vec<crate::schedule::ScheduleSpec>,
    /// Web Push（`docs/ip/m2-spec.md` D-33）。
    #[serde(default)]
    pub push: PushConfig,
}

/// Web Push 的设置。
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PushConfig {
    /// VAPID 的 `sub`：`mailto:` 或 `https://` 开头。推送服务有问题时联系谁；Safari 没有它会拒收。
    #[serde(default)]
    pub contact: Option<String>,
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("data")
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxConfig {
    #[serde(default)]
    pub mode: SandboxMode,
    /// 工具进程能否联网。
    #[serde(default)]
    pub network: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// 默认：bwrap 不可用时拒绝启动，而不是悄悄降级。
    #[default]
    Bubblewrap,
    /// 不隔离，仅限开发机。
    None,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndpointConfig {
    pub base_url: String,
    /// 存放 API key 的环境变量名。key 本身不写进配置文件。
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub max_concurrency: Option<usize>,
    /// 协议：`chat_completions`（默认）或 `responses`。OpenAI 的推理模型带工具时只能用 `responses`。
    #[serde(default)]
    pub api: ip_models::WireApi,
    /// 只对 `responses` 生效：推理强度（`low`、`medium`、`high` 等），省略时用模型默认值。
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    /// 各模型的价格（美元 / 百万 token），用于岗位每日预算和费用显示。键是端点内的模型名。
    #[serde(default)]
    pub prices: BTreeMap<String, ip_models::Price>,
}

/// `channels` 下的一项（D-16）。
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelConfig {
    Slack(SlackChannelConfig),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlackChannelConfig {
    /// 存放机器人令牌（`xoxb-`）的环境变量名。
    pub bot_token_env: String,
    /// 存放应用级令牌（`xapp-`，Socket Mode 用）的环境变量名。
    pub app_token_env: String,
    #[serde(default)]
    pub allow: AllowConfig,
    /// `requester`（默认）、`anyone`，或用户 id 列表。
    #[serde(default)]
    pub approvers: ApproversConfig,
    /// 这个渠道的消息都交给固定员工，不走路由。
    #[serde(default)]
    pub agent: Option<String>,
    /// Slack Web API 地址，默认 `https://slack.com/api`。一般不用改。
    #[serde(default)]
    pub api_url: Option<String>,
}

/// 只接名单里的用户和频道；都省略表示不限。
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowConfig {
    #[serde(default)]
    pub users: Vec<String>,
    #[serde(default)]
    pub channels: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum ApproversConfig {
    /// `requester` 或 `anyone`。
    Rule(String),
    /// 只有这些用户 id 能点。
    Users(Vec<String>),
}

impl Default for ApproversConfig {
    fn default() -> Self {
        Self::Rule("requester".into())
    }
}

impl ApproversConfig {
    pub fn to_rule(&self) -> Option<crate::channels::Approvers> {
        use crate::channels::Approvers;
        match self {
            Self::Rule(r) if r == "requester" => Some(Approvers::Requester),
            Self::Rule(r) if r == "anyone" => Some(Approvers::Anyone),
            Self::Rule(_) => None,
            Self::Users(ids) => Some(Approvers::Only(ids.clone())),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub id: String,
    pub role: String,
    pub name: String,
    /// 是否给这个员工 shell 和文件工具。
    #[serde(default = "yes")]
    pub tools: bool,
    /// 工作目录：员工的 shell 和文件工具只能碰这个目录。不写时用 `<data_dir>/workspaces/<id>`；
    /// 运维测试这类岗位指向要维护的项目。相对路径以配置文件所在目录为准，目录必须已存在。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
}

fn yes() -> bool {
    true
}

impl AgentConfig {
    pub fn agent(&self) -> Agent {
        Agent {
            id: self.id.as_str().into(),
            role: self.role.as_str().into(),
            name: self.name.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("读取配置文件 {0} 失败：{1}")]
    Read(PathBuf, std::io::Error),
    #[error("配置文件格式错误：{0}")]
    Parse(String),
    #[error("岗位定义错误：{0}")]
    Org(#[from] ip_core::OrgError),
    #[error("数字员工 id `{0}` 只能包含字母、数字、`-` 和 `_`")]
    BadAgentId(String),
    #[error("utc_offset `{0}` 应写成 +08:00 这样的格式")]
    BadUtcOffset(String),
    #[error("allowed_origins：{0}")]
    BadOrigin(String),
    #[error("push.contact `{0}` 要以 mailto: 或 https:// 开头")]
    BadPushContact(String),
    #[error("数字员工 id `{0}` 重复")]
    DuplicateAgent(String),
    #[error("数字员工 `{0}` 的岗位 `{1}` 不存在")]
    UnknownRole(String, String),
    #[error("路由规则错误：{0}")]
    Router(#[from] ip_router::RouterError),
    #[error("MCP 服务器 `{0}` 限定的岗位 `{1}` 不存在")]
    McpRole(String, String),
    #[error("MCP 服务器 `{0}` 的配置不对：{1}")]
    McpServer(String, String),
    #[error("岗位 `{0}` 的模型 `{1}` 应写成 `<端点>/<模型>`，且端点要在 endpoints 里定义")]
    UnknownEndpoint(String, String),
    #[error("语义匹配的模型 `{0}` 应写成 `<端点>/<模型>`，且端点要在 endpoints 里定义")]
    UnknownSemanticEndpoint(String),
    #[error("数据源 `{0}` 限定的岗位 `{1}` 不存在")]
    DataRole(String, String),
    #[error("模型 `{0}` 的价格无效（要是非负数）")]
    BadPrice(String),
    #[error("岗位 `{0}` 设了 budget_per_day_usd，但模型 `{1}` 没有在端点的 prices 里配置价格")]
    BudgetWithoutPrice(String, String),
    #[error("engines 里的岗位 `{0}` 不存在")]
    EngineRole(String),
    #[error("memory.roles 里的岗位 `{0}` 不存在")]
    MemoryRole(String),
    #[error("memory 里的模型 `{0}` 的端点没有定义，或缺少 `<端点>/` 前缀")]
    UnknownMemoryEndpoint(String),
    #[error("岗位 `{0}` 用外部引擎且设了 budget_per_day_usd，引擎配置里要写 price")]
    BudgetWithoutEnginePrice(String),
    #[error("岗位 `{0}` 的引擎价格无效（要是非负数）")]
    BadEnginePrice(String),
    #[error("数字员工 `{0}` 的岗位用外部引擎，需要工作目录，tools 不能设为 false")]
    EngineNeedsWorkspace(String),
    #[error("渠道名 `{0}` 只能包含字母、数字、`-` 和 `_`")]
    BadChannelName(String),
    #[error("渠道 `{0}` 的 approvers 只能是 requester、anyone 或用户 id 列表")]
    BadApprovers(String),
    #[error("渠道 `{0}` 指定的员工 `{1}` 不存在")]
    ChannelAgent(String, String),
    #[error(transparent)]
    Schedule(#[from] crate::schedule::ScheduleError),
}

impl Config {
    pub fn from_yaml(text: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_saphyr::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// 读取配置文件；`data_dir` 和数据源的 `path` 是相对路径时换算到配置文件所在目录。
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Read(path.to_path_buf(), e))?;
        let mut config = Self::from_yaml(&text)?;
        let base = path.parent().unwrap_or(Path::new("."));
        for agent in &mut config.agents {
            if let Some(ws) = &mut agent.workspace
                && ws.is_relative()
            {
                *ws = base.join(&*ws);
            }
        }
        if config.data_dir.is_relative() {
            config.data_dir = base.join(&config.data_dir);
        }
        for source in config.data_sources.values_mut() {
            if source.path.is_relative() {
                source.path = base.join(&source.path);
            }
        }
        Ok(config)
    }

    /// 校验并编译路由规则。
    pub fn router(&self) -> Result<Router, ConfigError> {
        let agents: Vec<Agent> = self.agents.iter().map(AgentConfig::agent).collect();
        Ok(Router::new(self.router.clone(), &self.org(), &agents)?)
    }

    /// 校验并编译定时任务。
    /// `utc_offset` 换成分钟数；没写时是 0（UTC）。
    pub fn utc_offset_minutes(&self) -> Result<i64, ConfigError> {
        match &self.utc_offset {
            None => Ok(0),
            Some(text) => crate::schedule::parse_offset(text).ok_or_else(|| ConfigError::BadUtcOffset(text.clone())),
        }
    }

    pub fn schedules(&self) -> Result<crate::schedule::Schedules, ConfigError> {
        let agents: Vec<(String, String)> = self.agents.iter().map(|a| (a.id.clone(), a.role.clone())).collect();
        Ok(crate::schedule::Schedules::new(
            self.schedules.clone(),
            &self.org(),
            &agents,
        )?)
    }

    /// 完整模型名 `<端点>/<模型>` 的价格。
    pub fn price(&self, model: &str) -> Option<&ip_models::Price> {
        let (ep, m) = model.split_once('/')?;
        self.endpoints.get(ep)?.prices.get(m)
    }

    pub fn org(&self) -> OrgSpec {
        OrgSpec {
            roles: self.roles.clone(),
        }
    }

    /// 解析 `allowed_origins`。
    pub(crate) fn allowed_origins(&self) -> Result<Vec<crate::origin::Origin>, ConfigError> {
        self.allowed_origins
            .iter()
            .map(|o| crate::origin::Origin::parse(o).map_err(ConfigError::BadOrigin))
            .collect()
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let org = self.org();
        org.validate()?;
        let known_model = |model: &str| {
            model
                .split_once('/')
                .is_some_and(|(ep, m)| self.endpoints.contains_key(ep) && !m.is_empty())
        };
        for (name, ep) in &self.endpoints {
            if let Some(model) = ep.prices.iter().find(|(_, p)| !p.valid()).map(|(m, _)| m) {
                return Err(ConfigError::BadPrice(format!("{name}/{model}")));
            }
        }
        for (role, engine) in &self.engines {
            let Some(r) = org.role(&RoleId::from(role.as_str())) else {
                return Err(ConfigError::EngineRole(role.clone()));
            };
            match engine.price() {
                Some(p) if !p.valid() => return Err(ConfigError::BadEnginePrice(role.clone())),
                None if r.budget_per_day_usd.is_some() => {
                    return Err(ConfigError::BudgetWithoutEnginePrice(role.clone()));
                }
                _ => {}
            }
        }
        // 用外部引擎的岗位不调 `model` 里的模型，不检查它。
        for role in self.roles.iter().filter(|r| !self.engines.contains_key(r.id.as_str())) {
            for model in std::iter::once(&role.model.default).chain(role.model.escalate.as_ref()) {
                if !known_model(model) {
                    return Err(ConfigError::UnknownEndpoint(role.id.to_string(), model.clone()));
                }
                // 设了预算就必须能算钱，否则预算形同虚设。
                if role.budget_per_day_usd.is_some() && self.price(model).is_none() {
                    return Err(ConfigError::BudgetWithoutPrice(role.id.to_string(), model.clone()));
                }
            }
        }
        if let Some(role) = self.memory.roles.iter().find(|r| org.role(r).is_none()) {
            return Err(ConfigError::MemoryRole(role.to_string()));
        }
        let memory_models = [&self.memory.extract_model, &self.memory.consolidation_model];
        if let Some(model) = memory_models.into_iter().flatten().find(|m| !known_model(m)) {
            return Err(ConfigError::UnknownMemoryEndpoint(model.clone()));
        }
        self.schedules()?;
        self.utc_offset_minutes()?;
        self.allowed_origins()?;
        if let Some(contact) = &self.push.contact
            && !(contact.starts_with("mailto:") || contact.starts_with("https://"))
        {
            return Err(ConfigError::BadPushContact(contact.clone()));
        }
        if let Some(sem) = &self.router.semantic
            && !known_model(&sem.model)
        {
            return Err(ConfigError::UnknownSemanticEndpoint(sem.model.clone()));
        }
        let mut seen = HashSet::new();
        for a in &self.agents {
            let ok = !a.id.is_empty() && a.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !ok {
                return Err(ConfigError::BadAgentId(a.id.clone()));
            }
            if !seen.insert(a.id.as_str()) {
                return Err(ConfigError::DuplicateAgent(a.id.clone()));
            }
            if org.role(&RoleId::from(a.role.as_str())).is_none() {
                return Err(ConfigError::UnknownRole(a.id.clone(), a.role.clone()));
            }
            if !a.tools && self.engines.contains_key(&a.role) {
                return Err(ConfigError::EngineNeedsWorkspace(a.id.clone()));
            }
        }
        for (name, channel) in &self.channels {
            // 渠道名会出现在 `<渠道>:<用户 id>` 里（路由规则 from、审批记录），不能含冒号。
            let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
            if !ok {
                return Err(ConfigError::BadChannelName(name.clone()));
            }
            let ChannelConfig::Slack(slack) = channel;
            if slack.approvers.to_rule().is_none() {
                return Err(ConfigError::BadApprovers(name.clone()));
            }
            if let Some(agent) = &slack.agent
                && !self.agents.iter().any(|a| &a.id == agent)
            {
                return Err(ConfigError::ChannelAgent(name.clone(), agent.clone()));
            }
        }
        for (name, server) in &self.mcp_servers {
            server
                .validate()
                .map_err(|reason| ConfigError::McpServer(name.clone(), reason))?;
            if let Some(r) = server.roles.iter().find(|r| org.role(r).is_none()) {
                return Err(ConfigError::McpRole(name.clone(), r.to_string()));
            }
        }
        for (name, source) in &self.data_sources {
            if let Some(r) = source.roles.iter().find(|r| org.role(r).is_none()) {
                return Err(ConfigError::DataRole(name.clone(), r.to_string()));
            }
        }
        self.router()?;
        Ok(())
    }
}
