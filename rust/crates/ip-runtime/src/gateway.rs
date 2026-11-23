//! 本轮令牌和经 MCP 服务端开放的工具（`docs/ip/m2-spec.md` D-18、D-19）。
//!
//! 外部引擎每开始一轮，运行时签发一个令牌，绑定“哪个员工、在哪个对话”。引擎带着令牌调
//! `/mcp` 上的工具，就以这个员工的身份、在这个对话里执行，和员工自己调工具效果一样。
//! 令牌只存在内存里，一轮结束后留一小段宽限期就作废；进程重启后全部失效。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ip_core::{Agent, AgentId, RoleId, ThreadId};
use ip_models::ToolSpec;
use ip_tools::ToolOutput;

use crate::{Runtime, RuntimeError, Shared, memory_tools, work_tools};

/// 一轮结束后令牌还能用多久：引擎收尾时可能还有正在路上的调用。
pub const TURN_TOKEN_GRACE: Duration = Duration::from_secs(60);

/// 不依赖工作目录的工具：组织工具、数据工具、外部 MCP 工具。
/// 员工自己的循环用它，经 MCP 服务端开放给外部引擎的也正是这一套（shell 和文件工具不在内）。
#[derive(Clone, Default)]
pub(crate) struct OrgTools {
    pub data: Option<Arc<ip_data::DataHub>>,
    pub mcp: Option<Arc<ip_mcp::McpHub>>,
    /// 这个岗位开了记忆：多三个记忆工具（查、读、记更正）。
    pub memory: bool,
    /// 不给员工的 MCP 工具（完整的 `mcp__<服务器>__<工具>` 名）：团队上下文的 `context_get` 等由 harness 调。
    pub hidden: Vec<String>,
}

impl OrgTools {
    pub fn new(config: &crate::RuntimeConfig, setup: &crate::AgentSetup) -> Self {
        let hidden = config
            .team_context
            .roles
            .get(&setup.role.id)
            .map(|t| {
                crate::team::HIDDEN_TOOLS
                    .iter()
                    .map(|tool| crate::team::tool_name(&t.server, tool))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            data: setup.data.clone(),
            mcp: setup.mcp.clone(),
            memory: config.memory.enabled_for(&setup.role.id),
            hidden,
        }
    }

    /// 某个岗位能用的工具定义。顺序固定，利于前缀缓存。
    pub fn specs(&self, role: &RoleId) -> Vec<ToolSpec> {
        let mut tools = work_tools::specs();
        if self.memory {
            tools.extend(memory_tools::specs());
        }
        if let Some(hub) = &self.data {
            tools.extend(hub.specs_for(role));
        }
        if let Some(hub) = &self.mcp {
            tools.extend(
                hub.specs_for(role)
                    .into_iter()
                    .filter(|t| !self.hidden.contains(&t.name)),
            );
        }
        tools
    }

    /// 依次交给组织工具、数据工具、MCP；都不认得这个名字时返回 `None`。
    pub async fn call(
        &self,
        shared: &Shared,
        me: &Agent,
        thread: &ThreadId,
        name: &str,
        arguments: &str,
    ) -> Option<ToolOutput> {
        if let Some(out) = work_tools::call(shared, me, thread, name, arguments).await {
            return Some(out);
        }
        if self.memory
            && let Some(out) = memory_tools::call(shared, me, thread, name, arguments).await
        {
            return Some(out);
        }
        if let Some(hub) = &self.data
            && let Some(out) = hub.call(&me.role, name, arguments).await
        {
            return Some(out);
        }
        if self.hidden.iter().any(|h| h == name) {
            return Some(ToolOutput::err(format!(
                "{name} 由系统在每轮开头调用，结果已经在提示词里；要找条目用 context_search"
            )));
        }
        if let Some(hub) = &self.mcp
            && let Some(out) = hub.call(&me.role, name, arguments).await
        {
            return Some(out);
        }
        None
    }
}

struct Grant {
    agent: AgentId,
    thread: ThreadId,
    expires_at: Instant,
}

/// 已签发的本轮令牌。
#[derive(Default)]
pub(crate) struct Grants(Mutex<HashMap<String, Grant>>);

impl Grants {
    /// 签发一个令牌，`ttl` 后自动作废。顺手清掉已过期的。
    pub fn issue(&self, agent: &AgentId, thread: &ThreadId, ttl: Duration) -> String {
        let token = new_token();
        let now = Instant::now();
        let mut grants = self.0.lock().unwrap_or_else(|p| p.into_inner());
        grants.retain(|_, g| now < g.expires_at);
        grants.insert(
            token.clone(),
            Grant {
                agent: agent.clone(),
                thread: thread.clone(),
                expires_at: now + ttl,
            },
        );
        token
    }

    /// 令牌在 `grace` 之后作废；只会缩短原有期限。
    pub fn end(&self, token: &str, grace: Duration) {
        let mut grants = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(g) = grants.get_mut(token) {
            g.expires_at = g.expires_at.min(Instant::now() + grace);
        }
    }

    fn lookup(&self, token: &str) -> Option<(AgentId, ThreadId)> {
        let grants = self.0.lock().unwrap_or_else(|p| p.into_inner());
        grants
            .get(token)
            .filter(|g| Instant::now() < g.expires_at)
            .map(|g| (g.agent.clone(), g.thread.clone()))
    }
}

/// 256 位随机数，十六进制。
fn new_token() -> String {
    format!("ipt_{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

impl Runtime {
    /// 给员工的一轮工作签发令牌。`ttl` 是这一轮最长能跑多久，到期自动作废，
    /// 引擎异常退出没来得及调 [`Runtime::end_turn`] 时也不会留下长期有效的令牌。
    pub fn grant_turn(&self, agent: &AgentId, thread: &ThreadId, ttl: Duration) -> Result<String, RuntimeError> {
        if !self.has_agent(agent) {
            return Err(RuntimeError::UnknownAgent(agent.clone()));
        }
        Ok(self.shared.grants.issue(agent, thread, ttl))
    }

    /// 这一轮结束：令牌在 `grace` 之后作废（通常用 [`TURN_TOKEN_GRACE`]），不会因此延长原有期限。
    pub fn end_turn(&self, token: &str, grace: Duration) {
        self.shared.grants.end(token, grace);
    }

    /// `/mcp` 的地址，服务开始监听后设置；外部引擎据此回调组织工具。
    pub fn set_mcp_url(&self, url: impl Into<String>) {
        *self.shared.mcp_url.write().unwrap_or_else(|p| p.into_inner()) = Some(url.into());
    }

    /// 令牌是否还有效。
    pub fn turn_token_valid(&self, token: &str) -> bool {
        self.shared.grants.lookup(token).is_some()
    }

    /// 令牌对应的员工和他的工具。员工已不存在时视同令牌无效。
    fn granted(&self, token: &str) -> Option<(Agent, ThreadId, OrgTools)> {
        let (agent_id, thread) = self.shared.grants.lookup(token)?;
        let agents = self.agents.read().unwrap_or_else(|p| p.into_inner());
        let entry = agents.get(&agent_id)?;
        Some((entry.agent.clone(), thread, entry.tools.clone()))
    }
}

impl ip_mcp::server::TurnTools for Runtime {
    fn specs(&self, token: &str) -> Option<Vec<ToolSpec>> {
        let (agent, _, tools) = self.granted(token)?;
        Some(tools.specs(&agent.role))
    }

    fn call<'a>(
        &'a self,
        token: &'a str,
        name: &'a str,
        arguments: String,
    ) -> ip_mcp::server::BoxFuture<'a, Option<ToolOutput>> {
        Box::pin(async move {
            let (agent, thread, tools) = self.granted(token)?;
            let out = tools.call(&self.shared, &agent, &thread, name, &arguments).await;
            Some(out.unwrap_or_else(|| ToolOutput::err(format!("没有工具 {name}，或你的岗位不能用它"))))
        })
    }
}
