//! MCP 客户端，基于官方 rmcp。
//!
//! 一个 [`McpHub`] 管理组织配置里的全部 MCP 服务器：启动时连上、取一次工具列表，
//! 之后数字员工的工具调用直接转发过去。工具对模型的名字是 `mcp__<服务器>__<工具>`，
//! 和内置工具不会重名。服务器可以限定只给某些岗位用。
//!
//! 启动时某个服务器连不上（子进程起不来、握手失败或超时、密钥没设），只是这个服务器的工具用不了，
//! 其他服务器和整个服务照常启动；没连上的列在 [`McpHub::unavailable`]。
//! [`McpHub::spawn_reconnect`] 在后台重连它们，默认最多 5 次（[`Reconnect`]），连上后工具出现在
//! [`McpHub::specs_for`] 里，[`McpHub::version`] 加一。密钥没设的不重连。
//! 名字或字段写错属于配置错误，仍然直接报错。
//!
//! 服务器有两种接法：`command` 启动一个 stdio 子进程，`url` 连一个远程服务器（Streamable HTTP）。
//!
//! 注意：stdio 服务器是 harness 直接启动的子进程，**不在** bubblewrap 沙箱里，
//! 权限等同 harness 本身。只配置你信任的服务器。远程服务器返回的内容会原样交给模型，同样只连你信任的。
//!
//! stdio 子进程不继承 harness 的环境变量（模型、Slack、控制台的密钥都在里面），
//! 只带 [`BASE_ENV`] 里的几个基本变量，加上配置里写明的 `env`、`env_inherit`、`env_from`。
//!
//! 服务端（把 ip 的组织工具开放给外部引擎）在 [`server`] 模块。

pub mod server;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use ip_core::RoleId;
use ip_models::ToolSpec;
use ip_tools::{ToolOutput, truncate_output};
use rmcp::model::{CallToolRequestParams, ContentBlock};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{IntoTransport, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{Peer, ServiceExt};
use serde::Deserialize;
use tokio::sync::Mutex;

/// 工具名前缀。OpenAI 兼容端点要求函数名匹配 `^[a-zA-Z0-9_-]{1,64}$`。
const PREFIX: &str = "mcp__";
const MAX_NAME: usize = 64;
/// 返回给模型的输出上限（字节），与内置工具一致。
const MAX_OUTPUT: usize = 64 * 1024;
/// stdio 子进程默认从 harness 带过去的环境变量：找可执行文件、写缓存、编码和时区要用，都不是密钥。
/// 其余变量一律不带，要用的在配置里写 `env_inherit` 或 `env_from`。
pub const BASE_ENV: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "LANG", "LC_ALL", "LC_CTYPE", "TERM", "TMPDIR", "TZ",
];

/// `ip.yaml` 里 `mcp_servers` 的一项。`command`（stdio 子进程）和 `url`（远程服务器）二选一。
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    /// stdio：要启动的可执行文件。
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// 直接写明的环境变量（不要放密钥）。
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// 原样带给子进程的 harness 环境变量名，harness 里没设就不带。子进程默认只有 [`BASE_ENV`]，
    /// 其他要用的写在这里，例如 `SEC_USER_AGENT`、`HTTPS_PROXY`。必须设置的密钥用 `env_from`，没设时启动报错。
    #[serde(default)]
    pub env_inherit: Vec<String>,
    /// 从 harness 自己的环境变量取值：`子进程变量名: harness 变量名`。密钥走这里。
    #[serde(default)]
    pub env_from: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    /// 远程：服务器地址（Streamable HTTP），如 `https://example.com/mcp`。
    /// 必须是 https；只有本机地址（localhost、127.0.0.1、[::1]）可以用 http。
    pub url: Option<String>,
    /// 远程：访问令牌所在的环境变量名，以 `Authorization: Bearer <令牌>` 发送。
    pub bearer_token_env: Option<String>,
    /// 只给这些岗位用；空表示所有岗位。
    #[serde(default)]
    pub roles: Vec<RoleId>,
    /// 单次调用超时，也是启动时握手的超时，默认 120 秒。
    pub timeout_secs: Option<u64>,
}

impl ServerConfig {
    /// 检查 `command` 和 `url` 二选一，以及各自专属的字段没有混用。不联网、不读环境变量。
    pub fn validate(&self) -> Result<(), String> {
        match (&self.url, self.command.is_empty()) {
            (None, true) => Err("要写 command（stdio）或 url（远程）其中一个".into()),
            (Some(_), false) => Err("command 和 url 只能写一个".into()),
            (None, false) => {
                if self.bearer_token_env.is_some() {
                    return Err("bearer_token_env 只用于 url；stdio 服务器的密钥用 env_from".into());
                }
                Ok(())
            }
            (Some(url), true) => {
                if !self.args.is_empty()
                    || !self.env.is_empty()
                    || !self.env_inherit.is_empty()
                    || !self.env_from.is_empty()
                    || self.cwd.is_some()
                {
                    return Err(
                        "args、env、env_inherit、env_from、cwd 只用于 command；远程服务器的令牌用 bearer_token_env"
                            .into(),
                    );
                }
                check_url(url)
            }
        }
    }
}

/// 取一个必须设置的密钥；没设或为空时报错，免得服务器带着空令牌启动。
fn secret(name: &str, var: &str, env: &(dyn Fn(&str) -> Option<String> + Sync)) -> Result<String, McpError> {
    env(var).filter(|v| !v.is_empty()).ok_or_else(|| McpError::MissingEnv {
        name: name.to_string(),
        var: var.to_string(),
    })
}

/// 构造 stdio 服务器的启动命令。环境变量先清空，再按顺序放 [`BASE_ENV`]、`env_inherit`、`env`、`env_from`，
/// 后放的同名变量覆盖先放的。harness 自己的其他变量（模型和渠道的密钥）不会进子进程。
fn stdio_command(
    name: &str,
    config: &ServerConfig,
    env: &(dyn Fn(&str) -> Option<String> + Sync),
) -> Result<tokio::process::Command, McpError> {
    let mut command = tokio::process::Command::new(&config.command);
    command.args(&config.args).env_clear();
    for var in BASE_ENV
        .iter()
        .copied()
        .chain(config.env_inherit.iter().map(String::as_str))
    {
        if let Some(value) = env(var) {
            command.env(var, value);
        }
    }
    command.envs(&config.env);
    for (child_var, var) in &config.env_from {
        command.env(child_var, secret(name, var, env)?);
    }
    if let Some(dir) = &config.cwd {
        command.current_dir(dir);
    }
    Ok(command)
}

/// 远程地址必须是 https；http 只允许本机，免得令牌和数据明文出网。
fn check_url(url: &str) -> Result<(), String> {
    let uri: http::Uri = url.parse().map_err(|_| format!("url {url:?} 不是合法地址"))?;
    let host = uri.host().unwrap_or_default();
    match uri.scheme_str() {
        Some("https") if !host.is_empty() => Ok(()),
        Some("http") if matches!(host, "localhost" | "127.0.0.1" | "[::1]") => Ok(()),
        Some("http") => Err(format!("url {url:?} 要用 https；http 只能连本机地址")),
        _ => Err(format!("url {url:?} 要以 https:// 开头")),
    }
}

#[derive(Clone, Debug, thiserror::Error)]
pub enum McpError {
    #[error("MCP 服务器名 {0:?} 只能用字母、数字、- 和 _，且不能含 \"__\"")]
    BadName(String),
    #[error("MCP 服务器 {0} 重复")]
    Duplicate(String),
    #[error("MCP 服务器 {name} 的配置不对：{reason}")]
    BadConfig { name: String, reason: String },
    #[error("MCP 服务器 {name} 的环境变量 {var} 未设置")]
    MissingEnv { name: String, var: String },
    #[error("启动 MCP 服务器 {name} 失败：{error}")]
    Spawn { name: String, error: String },
    #[error("连接 MCP 服务器 {name} 失败：{error}")]
    Connect { name: String, error: String },
}

impl McpError {
    /// 重连有没有可能成功。密钥没设的不重连：进程的环境变量运行中不会变；配置错误在连接前就报了。
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Spawn { .. } | Self::Connect { .. })
    }
}

/// 后台重连启动时没连上的服务器：最多几次、隔多久。
#[derive(Clone, Copy, Debug)]
pub struct Reconnect {
    /// 最多重连几次。
    pub attempts: u32,
    /// 第一次重连前等多久，之后每次翻倍。
    pub first_delay: Duration,
}

impl Default for Reconnect {
    /// 维护者 2026-11-14 定：重连 5 次。间隔从 30 秒起翻倍（30 秒，1、2、4、8 分钟），
    /// 最后一次大约在启动后 15 分钟，不算每次握手本身的耗时。
    fn default() -> Self {
        Self {
            attempts: 5,
            first_delay: Duration::from_secs(30),
        }
    }
}

/// 读环境变量的函数。后台重连要在启动之后继续用它，所以是能跨任务共享的版本。
pub type EnvFn = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

struct Server {
    name: String,
    peer: Peer<RoleClient>,
    roles: Vec<RoleId>,
    timeout: Duration,
    /// 持有连接；`shutdown` 时取出并关闭。
    running: Mutex<Option<RunningService<RoleClient, ()>>>,
}

impl Server {
    fn allowed(&self, role: &RoleId) -> bool {
        self.roles.is_empty() || self.roles.contains(role)
    }
}

/// 没连上的服务器。
#[derive(Clone, Debug)]
pub struct Unavailable {
    pub name: String,
    /// 原本能用它的岗位；空表示所有岗位。
    pub roles: Vec<RoleId>,
    /// 最近一次失败的原因。
    pub error: McpError,
    /// 后台已经重连了几次。
    pub attempts: u32,
    /// 后台还会再重连。为 false 时这次运行不会再连它，修好后重启服务。
    pub reconnecting: bool,
}

struct ExposedTool {
    server: Arc<Server>,
    /// 服务器上的原名。
    name: String,
    spec: ToolSpec,
}

#[derive(Default)]
struct State {
    servers: Vec<Arc<Server>>,
    tools: BTreeMap<String, ExposedTool>,
    unavailable: Vec<Unavailable>,
    /// `shutdown` 之后不再接入新连接。
    closed: bool,
}

/// 所有 MCP 服务器和它们的工具，放进 `Arc` 在员工之间共享。
///
/// 工具集只在接入服务器时变（启动时，或后台重连上一个）。锁只在读写工具表时短暂持有，
/// 不跨 `await`；调用工具时先取出连接再放锁。
#[derive(Default)]
pub struct McpHub {
    state: std::sync::RwLock<State>,
    /// 工具集每变一次加一。员工每步比一下，变了才重算工具列表。
    version: AtomicU64,
    /// 后台重连任务；`shutdown` 时停掉。
    reconnects: std::sync::Mutex<Vec<tokio::task::AbortHandle>>,
}

impl McpHub {
    /// 按配置逐个启动并连接。
    ///
    /// 先检查全部配置，名字或字段写错时直接返回错误，一个都不连。之后某个服务器连不上，
    /// 记进 [`unavailable`](Self::unavailable) 并接着连下一个，不影响其他服务器和用不到它的岗位。
    /// 这里不重连；要重连的再调 [`spawn_reconnect`](Self::spawn_reconnect)。
    pub async fn connect(configs: &BTreeMap<String, ServerConfig>) -> Result<Self, McpError> {
        Self::connect_with_env(configs, &|var| std::env::var(var).ok()).await
    }

    /// 同 [`connect`](Self::connect)，环境变量从 `env` 取（测试里不用改进程的环境变量）。
    pub async fn connect_with_env(
        configs: &BTreeMap<String, ServerConfig>,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Result<Self, McpError> {
        for (name, config) in configs {
            check_name(name)?;
            config.validate().map_err(|reason| McpError::BadConfig {
                name: name.clone(),
                reason,
            })?;
        }
        let hub = Self::default();
        for (name, config) in configs {
            if let Err(error) = hub.connect_one(name, config, env).await {
                tracing::error!(
                    server = %name,
                    roles = %roles_text(&config.roles),
                    %error,
                    "MCP 服务器没连上，它的工具暂时用不了；其他部分照常启动"
                );
                hub.write().unavailable.push(Unavailable {
                    name: name.clone(),
                    roles: config.roles.clone(),
                    error,
                    attempts: 0,
                    reconnecting: false,
                });
            }
        }
        Ok(hub)
    }

    /// 在后台重连 [`unavailable`](Self::unavailable) 里的服务器，每个一个任务，按 `policy` 重试，
    /// 连上就停。密钥没设的不重连。任务只持有弱引用，hub 释放或 [`shutdown`](Self::shutdown) 后自己结束。
    pub fn spawn_reconnect(self: &Arc<Self>, configs: &BTreeMap<String, ServerConfig>, env: EnvFn, policy: Reconnect) {
        if policy.attempts == 0 {
            return;
        }
        let targets: Vec<(String, ServerConfig)> = {
            let mut state = self.write();
            if state.closed {
                return;
            }
            state
                .unavailable
                .iter_mut()
                .filter(|u| u.error.retryable())
                .filter_map(|u| {
                    let config = configs.get(&u.name)?.clone();
                    u.reconnecting = true;
                    Some((u.name.clone(), config))
                })
                .collect()
        };
        let mut tasks = self.reconnects.lock().unwrap_or_else(|p| p.into_inner());
        for (name, config) in targets {
            let task = tokio::spawn(reconnect(Arc::downgrade(self), name, config, env.clone(), policy));
            tasks.push(task.abort_handle());
        }
    }

    /// 启动或连上一个服务器。配置已经检查过。
    async fn connect_one(
        &self,
        name: &str,
        config: &ServerConfig,
        env: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Result<(), McpError> {
        let timeout = Duration::from_secs(config.timeout_secs.unwrap_or(120).max(1));
        if let Some(url) = &config.url {
            let mut transport = StreamableHttpClientTransportConfig::with_uri(url.as_str());
            if let Some(var) = &config.bearer_token_env {
                transport = transport.auth_header(secret(name, var, env)?);
            }
            let transport = StreamableHttpClientTransport::from_config(transport);
            return self.add(name, transport, config.roles.clone(), timeout).await;
        }
        let transport = TokioChildProcess::builder(stdio_command(name, config, env)?)
            .stderr(std::process::Stdio::inherit())
            .spawn()
            .map_err(|e| McpError::Spawn {
                name: name.to_string(),
                error: e.to_string(),
            })?
            .0;
        self.add(name, transport, config.roles.clone(), timeout).await
    }

    /// 接入一个已经建好的传输通道（测试里用内存管道）。连上后这个名字从 `unavailable` 里去掉。
    pub async fn add<T, E, A>(
        &self,
        name: &str,
        transport: T,
        roles: Vec<RoleId>,
        timeout: Duration,
    ) -> Result<(), McpError>
    where
        T: IntoTransport<RoleClient, E, A>,
        E: std::error::Error + Send + Sync + 'static,
    {
        check_name(name)?;
        if self.read().servers.iter().any(|s| s.name == name) {
            return Err(McpError::Duplicate(name.to_string()));
        }
        let connect_error = |e: String| McpError::Connect {
            name: name.to_string(),
            error: e,
        };
        let running = tokio::time::timeout(timeout, ().serve(transport))
            .await
            .map_err(|_| connect_error(format!("握手超过 {} 秒没有完成", timeout.as_secs())))?
            .map_err(|e| connect_error(e.to_string()))?;
        let peer = running.peer().clone();
        let listed = tokio::time::timeout(timeout, peer.list_all_tools())
            .await
            .map_err(|_| connect_error("获取工具列表超时".into()))?
            .map_err(|e| connect_error(e.to_string()))?;
        let server = Arc::new(Server {
            name: name.to_string(),
            peer,
            roles,
            timeout,
            running: Mutex::new(Some(running)),
        });
        // 从这里到放锁没有 await：`shutdown` 置了 closed 之后，不会再有连接混进来。
        let mut state = self.write();
        if state.closed {
            // 丢掉连接即断开；stdio 子进程随传输通道一起被结束。
            return Err(connect_error("服务正在关闭".into()));
        }
        if state.servers.iter().any(|s| s.name == name) {
            return Err(McpError::Duplicate(name.to_string()));
        }
        let mut count = 0;
        for tool in listed {
            let exposed = exposed_name(name, &tool.name);
            if state.tools.contains_key(&exposed) {
                tracing::warn!(server = name, tool = %tool.name, "MCP 工具名截断后重名，已跳过");
                continue;
            }
            let description = tool.description.as_deref().unwrap_or_default();
            let spec = ToolSpec {
                name: exposed.clone(),
                description: format!("[{name}] {description}"),
                parameters: serde_json::Value::Object((*tool.input_schema).clone()),
            };
            state.tools.insert(
                exposed,
                ExposedTool {
                    server: server.clone(),
                    name: tool.name.to_string(),
                    spec,
                },
            );
            count += 1;
        }
        state.servers.push(server);
        state.unavailable.retain(|u| u.name != name);
        self.version.fetch_add(1, Ordering::Release);
        drop(state);
        tracing::info!(server = name, tools = count, "MCP 服务器已连接");
        Ok(())
    }

    /// 某个岗位能用的工具定义，按名字排序（顺序稳定，利于前缀缓存）。
    pub fn specs_for(&self, role: &RoleId) -> Vec<ToolSpec> {
        self.read()
            .tools
            .values()
            .filter(|t| t.server.allowed(role))
            .map(|t| t.spec.clone())
            .collect()
    }

    /// 工具集的版本号：每接入一个服务器加一。不变时 [`specs_for`](Self::specs_for) 的结果也不变。
    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    /// 执行一个 MCP 工具；名字不是 MCP 工具时返回 `None`。
    pub async fn call(&self, role: &RoleId, name: &str, arguments: &str) -> Option<ToolOutput> {
        if !name.starts_with(PREFIX) {
            return None;
        }
        let found = {
            let state = self.read();
            match state.tools.get(name).filter(|t| t.server.allowed(role)) {
                Some(tool) => Ok((tool.server.clone(), tool.name.clone())),
                None => Err(not_found(&state, role, name)),
            }
        };
        let (server, tool) = match found {
            Ok(found) => found,
            Err(message) => return Some(ToolOutput::err(message)),
        };
        let args = match parse_object(arguments) {
            Ok(a) => a,
            Err(e) => return Some(ToolOutput::err(e)),
        };
        let mut params = CallToolRequestParams::new(tool);
        if let Some(a) = args {
            params = params.with_arguments(a);
        }
        let out = match tokio::time::timeout(server.timeout, server.peer.call_tool(params)).await {
            Err(_) => ToolOutput::err(format!("{} 超过 {} 秒没有返回", name, server.timeout.as_secs())),
            Ok(Err(e)) => ToolOutput::err(format!("MCP 服务器 {} 出错：{e}", server.name)),
            Ok(Ok(result)) => {
                let text = render(&result.content, result.structured_content.as_ref());
                let text = truncate_output(text, MAX_OUTPUT, 0);
                if result.is_error.unwrap_or(false) {
                    ToolOutput::err(text)
                } else {
                    ToolOutput::ok(text)
                }
            }
        };
        Some(out)
    }

    /// 由 harness 自己调某个服务器上的一个工具（不经过模型），返回结构化结果。
    ///
    /// 和 [`call`](Self::call) 不同：按服务器名找，不看岗位（调用方自己保证这个服务器是给谁用的）；
    /// 结果不截断，优先取 `structuredContent`，没有时把文字内容当 JSON 解析。
    /// 用于团队上下文：每轮开头取上下文包（`docs/ip/team-context.md`）。
    pub async fn call_json(
        &self,
        server: &str,
        tool: &str,
        arguments: serde_json::Map<String, serde_json::Value>,
        timeout: Duration,
    ) -> Result<serde_json::Value, String> {
        let found = {
            let state = self.read();
            match state.servers.iter().find(|s| s.name == server) {
                Some(s) => Ok(s.clone()),
                None => Err(match state.unavailable.iter().find(|u| u.name == server) {
                    Some(u) => format!("MCP 服务器 {server} 没连上：{}", u.error),
                    None => format!("没有 MCP 服务器 {server}"),
                }),
            }
        };
        let server = found?;
        let params = CallToolRequestParams::new(tool.to_string()).with_arguments(arguments);
        let wait = timeout.min(server.timeout);
        let result = tokio::time::timeout(wait, server.peer.call_tool(params))
            .await
            .map_err(|_| format!("{} 的 {tool} 超过 {} 毫秒没有返回", server.name, wait.as_millis()))?
            .map_err(|e| format!("MCP 服务器 {} 出错：{e}", server.name))?;
        if result.is_error.unwrap_or(false) {
            return Err(render(&result.content, None));
        }
        if let Some(v) = result.structured_content {
            return Ok(v);
        }
        let text = render(&result.content, None);
        serde_json::from_str(&text).map_err(|_| format!("{} 的 {tool} 返回的不是 JSON", server.name))
    }

    /// 停掉后台重连，关闭所有连接；stdio 服务器的子进程随之退出。远程服务器只是断开。
    pub async fn shutdown(&self) {
        let servers = {
            let mut state = self.write();
            state.closed = true;
            for u in &mut state.unavailable {
                u.reconnecting = false;
            }
            state.servers.clone()
        };
        for task in self.reconnects.lock().unwrap_or_else(|p| p.into_inner()).drain(..) {
            task.abort();
        }
        for s in servers {
            if let Some(mut running) = s.running.lock().await.take() {
                let _ = running.close().await;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.read().servers.is_empty()
    }

    /// 现在没连上的服务器，按名字排序。
    pub fn unavailable(&self) -> Vec<Unavailable> {
        self.read().unavailable.clone()
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.state.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.state.write().unwrap_or_else(|p| p.into_inner())
    }
}

/// 一个服务器的后台重连：等一会儿、试一次，失败就把等待时间翻倍，试满 `policy.attempts` 次为止。
async fn reconnect(hub: Weak<McpHub>, name: String, config: ServerConfig, env: EnvFn, policy: Reconnect) {
    let mut delay = policy.first_delay;
    for attempt in 1..=policy.attempts {
        tokio::time::sleep(delay).await;
        delay = delay.saturating_mul(2);
        let Some(hub) = hub.upgrade() else { return };
        let Err(error) = hub.connect_one(&name, &config, &*env).await else {
            tracing::info!(server = %name, attempt, "MCP 服务器重连上了，它的工具从员工的下一步起可以用");
            return;
        };
        let more = attempt < policy.attempts && error.retryable();
        {
            let mut state = hub.write();
            if state.closed {
                return;
            }
            if let Some(u) = state.unavailable.iter_mut().find(|u| u.name == name) {
                u.error = error.clone();
                u.attempts = attempt;
                u.reconnecting = more;
            }
        }
        if !more {
            tracing::error!(
                server = %name,
                roles = %roles_text(&config.roles),
                attempts = attempt,
                %error,
                "MCP 服务器重连也没连上，这次运行不再重连；修好后重启服务"
            );
            return;
        }
        tracing::warn!(server = %name, attempt, %error, "MCP 服务器重连失败，稍后再试");
    }
}

/// 工具找不到时给模型的说明。服务器没连上（对话里可能留着上次运行调过的工具名）时说清楚原因；
/// 岗位本来就不能用的服务器，不说它的状态。
fn not_found(state: &State, role: &RoleId, name: &str) -> String {
    let down = state
        .unavailable
        .iter()
        .find(|u| name.starts_with(&format!("{PREFIX}{}__", u.name)))
        .filter(|u| u.roles.is_empty() || u.roles.contains(role));
    match down {
        Some(u) if u.reconnecting => format!(
            "MCP 服务器 {} 现在没连上，后台正在重连；这一轮先不要再调它的工具",
            u.name
        ),
        Some(u) if u.attempts > 0 => format!(
            "MCP 服务器 {} 没连上，后台重连 {} 次也没成功，这次运行用不了它的工具",
            u.name, u.attempts
        ),
        Some(u) => format!("MCP 服务器 {} 在服务启动时没连上，这次运行用不了它的工具", u.name),
        None => format!("没有工具 {name}，或你的岗位不能用它"),
    }
}

fn roles_text(roles: &[RoleId]) -> String {
    if roles.is_empty() {
        "所有岗位".to_string()
    } else {
        roles.iter().map(RoleId::as_str).collect::<Vec<_>>().join(",")
    }
}

impl std::fmt::Debug for McpHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.read();
        f.debug_struct("McpHub")
            .field("servers", &state.servers.iter().map(|s| &s.name).collect::<Vec<_>>())
            .field("tools", &state.tools.keys().collect::<Vec<_>>())
            .field(
                "unavailable",
                &state.unavailable.iter().map(|u| &u.name).collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// 共享句柄。
pub type SharedHub = Arc<McpHub>;

fn check_name(name: &str) -> Result<(), McpError> {
    let ok = !name.is_empty()
        && name.len() <= 24
        && !name.contains("__")
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(McpError::BadName(name.to_string()))
    }
}

/// `mcp__<服务器>__<工具>`，工具名里不允许的字符换成 `_`，总长不超过 64。
fn exposed_name(server: &str, tool: &str) -> String {
    let mut s = format!("{PREFIX}{server}__");
    for c in tool.chars() {
        if s.len() >= MAX_NAME {
            break;
        }
        s.push(if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            c
        } else {
            '_'
        });
    }
    s
}

fn parse_object(arguments: &str) -> Result<Option<serde_json::Map<String, serde_json::Value>>, String> {
    if arguments.trim().is_empty() {
        return Ok(None);
    }
    match serde_json::from_str::<serde_json::Value>(arguments) {
        Ok(serde_json::Value::Object(map)) => Ok(Some(map)),
        Ok(serde_json::Value::Null) => Ok(None),
        Ok(_) => Err("参数必须是 JSON 对象".into()),
        Err(e) => Err(format!("参数不是合法的 JSON：{e}")),
    }
}

/// 把结果内容拼成给模型看的文字。非文字内容只留一个占位说明。
fn render(content: &[ContentBlock], structured: Option<&serde_json::Value>) -> String {
    let mut parts: Vec<String> = Vec::new();
    for block in content {
        match block.as_text() {
            Some(t) => parts.push(t.text.clone()),
            None => {
                let kind = serde_json::to_value(block)
                    .ok()
                    .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_string))
                    .unwrap_or_else(|| "未知".into());
                parts.push(format!("[{kind} 内容，未显示]"));
            }
        }
    }
    if parts.is_empty()
        && let Some(v) = structured
    {
        parts.push(v.to_string());
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests;
