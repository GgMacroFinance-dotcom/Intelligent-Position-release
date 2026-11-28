//! 在线改组织（`docs/ip/m2-spec.md` D-42，桌面外壳后端适配清单第 3 项）。
//!
//! - 控制台和 App 能新建、改、删岗位（改、删是第 ⑦ 项，D-44），新建、改、删员工。改动存进数据库（设置表的 `org.console` 键），
//!   不写回 ip.yaml；启动时合并到 ip.yaml 的组织上，和路由规则一样（维护者 2026-11-27 定）。
//! - 每次改动都把“ip.yaml + 改动”拼成一份完整配置，整份过一遍 [`Config::validate`]：
//!   模型端点、预算和价格、外部引擎要工作目录、定时任务和渠道指定的员工、路由规则……都和 ip.yaml 一样校验。
//!   不过就拒绝，什么都不动。
//! - 在线建的员工只能用默认工作目录（`<data_dir>/workspaces/<id>`），不能指到主机上别的目录：
//!   那等于让一个设备决定员工的 shell 能碰哪里，只有改 ip.yaml 的人能决定。
//! - 改员工的名字、岗位、工具要重启这个员工，他正在干活时拒绝；只改星图形态不用重启。
//!   员工手里拿着岗位定义的副本（模型、预算、说明），所以改岗位也要重启这个岗位的员工。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, bail};
use ip_core::{Agent, AgentId, Role, RoleId};
use ip_router::Router;
use ip_runtime::{AgentSetup, RuntimeEvent};
use ip_store::Store;
use ip_tools::{Limits, Sandbox, Toolbox};
use serde::{Deserialize, Serialize};

use crate::App;
use crate::config::{AgentConfig, Config};

const KEY: &str = "org.console";

/// 控制台改过的组织，叠在 ip.yaml 上。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Overlay {
    /// 新建的岗位；和 ip.yaml 里同 id 的岗位整个替换它。
    #[serde(default)]
    pub roles: Vec<Role>,
    /// 新建或改过的员工；和 ip.yaml 里同 id 的员工替换它的名字、岗位、工具和形态（工作目录照旧）。
    #[serde(default)]
    pub agents: Vec<ConsoleAgent>,
    /// 删掉的 ip.yaml 里的员工。
    #[serde(default)]
    pub removed_agents: Vec<String>,
    /// 删掉的 ip.yaml 里的岗位（第 ⑦ 项，D-44）。
    #[serde(default)]
    pub removed_roles: Vec<String>,
}

/// 控制台建的或改过的一位员工。没有工作目录这一项：在线建的员工用默认工作目录。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConsoleAgent {
    pub id: String,
    pub role: String,
    pub name: String,
    pub tools: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub celestial_style: Option<String>,
}

impl Overlay {
    /// ip.yaml 加上这些改动。不校验，调用方接着调 [`Config::validate`]。
    pub fn apply(&self, base: &Config) -> Config {
        let mut c = base.clone();
        c.roles
            .retain(|r| !self.removed_roles.iter().any(|x| x == r.id.as_str()));
        for role in &self.roles {
            match c.roles.iter_mut().find(|r| r.id == role.id) {
                Some(r) => *r = role.clone(),
                None => c.roles.push(role.clone()),
            }
        }
        c.agents.retain(|a| !self.removed_agents.contains(&a.id));
        for a in &self.agents {
            match c.agents.iter_mut().find(|x| x.id == a.id) {
                Some(x) => {
                    x.role.clone_from(&a.role);
                    x.name.clone_from(&a.name);
                    x.tools = a.tools;
                    x.celestial_style.clone_from(&a.celestial_style);
                }
                None => c.agents.push(AgentConfig {
                    id: a.id.clone(),
                    role: a.role.clone(),
                    name: a.name.clone(),
                    tools: a.tools,
                    workspace: None,
                    celestial_style: a.celestial_style.clone(),
                }),
            }
        }
        c
    }
}

/// 读存下的改动；没有或读不出来时是空的（读不出来记一条警告）。
pub async fn load(store: &Store) -> Overlay {
    match store.setting(KEY).await {
        Ok(Some(v)) => serde_json::from_value(v).unwrap_or_else(|e| {
            tracing::warn!(error = %e, "控制台改过的组织读不出来，这次只用 ip.yaml");
            Overlay::default()
        }),
        Ok(None) => Overlay::default(),
        Err(e) => {
            tracing::warn!(error = %e, "读控制台改过的组织失败，这次只用 ip.yaml");
            Overlay::default()
        }
    }
}

/// 启动时：ip.yaml 加上存下的改动，整份校验通过就用合并后的，否则记警告、只用 ip.yaml（改动先不删，
/// 下一次在线改动成功时才覆盖它）。返回生效的配置和内存里的改动。
pub fn merge_at_startup(base: &Config, overlay: Overlay) -> (Config, Overlay) {
    if overlay == Overlay::default() {
        return (base.clone(), overlay);
    }
    let merged = overlay.apply(base);
    match merged.validate() {
        Ok(()) => {
            tracing::info!(
                roles = overlay.roles.len(),
                agents = overlay.agents.len(),
                removed = overlay.removed_agents.len() + overlay.removed_roles.len(),
                "使用控制台改过的组织"
            );
            (merged, overlay)
        }
        Err(e) => {
            tracing::warn!(error = %e, "控制台改过的组织和现在的 ip.yaml 对不上，这次只用 ip.yaml");
            (base.clone(), Overlay::default())
        }
    }
}

/// 组装员工要用的东西，启动时和在线新建时共用。
pub struct AgentFactory {
    pub data_dir: PathBuf,
    pub sandbox: Sandbox,
    pub mcp: Option<Arc<ip_mcp::McpHub>>,
    pub data: Option<Arc<ip_data::DataHub>>,
    pub engines: BTreeMap<String, Arc<dyn ip_runtime::engine::Engine>>,
}

impl AgentFactory {
    /// 按员工的配置组装 [`AgentSetup`]；有工具时建好工作目录，一起返回。
    pub fn setup(&self, a: &AgentConfig, role: &Role) -> anyhow::Result<(AgentSetup, Option<PathBuf>)> {
        let (toolbox, workspace) = if a.tools {
            let ws = match &a.workspace {
                Some(ws) => {
                    if !ws.is_dir() {
                        bail!("数字员工 {} 的工作目录 {} 不存在", a.id, ws.display());
                    }
                    ws.clone()
                }
                None => {
                    let ws = self.data_dir.join("workspaces").join(&a.id);
                    std::fs::create_dir_all(&ws).with_context(|| format!("创建工作目录 {}", ws.display()))?;
                    ws
                }
            };
            let toolbox = Toolbox::new(&ws, self.sandbox.clone(), Limits::default())?;
            (Some(toolbox), Some(ws))
        } else {
            (None, None)
        };
        let setup = AgentSetup {
            agent: a.agent(),
            role: role.clone(),
            toolbox,
            mcp: self.mcp.clone(),
            data: self.data.clone(),
            engine: self.engines.get(&a.role).cloned(),
        };
        Ok((setup, workspace))
    }
}

/// 在线改组织的状态。
pub struct OrgAdmin {
    /// ip.yaml 原样（相对路径已换算）。
    pub base: Config,
    pub factory: AgentFactory,
    /// 控制台的改动。这把锁也串行化所有组织改动。
    pub overlay: tokio::sync::Mutex<Overlay>,
}

/// `create_agent` 的参数。
pub struct NewAgent {
    pub id: AgentId,
    pub role: RoleId,
    pub name: String,
    pub tools: bool,
    pub celestial_style: Option<String>,
}

/// `update_agent` 的参数；没给的不改。`celestial_style` 给空字符串是清掉（回到 App 自己挑）。
#[derive(Default)]
pub struct AgentPatch {
    pub name: Option<String>,
    pub role: Option<RoleId>,
    pub tools: Option<bool>,
    pub celestial_style: Option<String>,
}

/// 新建一个岗位（没有员工也行），回复建好的岗位。
pub async fn create_role(app: &App, role: Role) -> Result<Role, String> {
    if !id_ok(role.id.as_str()) {
        return Err(format!("岗位 id `{}` 只能包含字母、数字、`-` 和 `_`", role.id));
    }
    let id = role.id.clone();
    change(app, |_base, current, overlay| {
        if current.roles.iter().any(|r| r.id == role.id) {
            return Err(format!("岗位 {} 已经有了", role.id));
        }
        overlay.removed_roles.retain(|r| r != role.id.as_str());
        overlay.roles.retain(|r| r.id != role.id);
        overlay.roles.push(role);
        Ok(())
    })
    .await?;
    app.org().role(&id).cloned().ok_or_else(|| format!("岗位 {id} 没建上"))
}

/// 改一个岗位（D-44），ip.yaml 里的也行：给出整个岗位，按 id 整个替换。这个岗位的员工要重启才用上
/// 新的模型、预算和说明，他们有人在干活时回错误。回复改完的岗位。
pub async fn update_role(app: &App, role: Role) -> Result<Role, String> {
    let id = role.id.clone();
    change(app, |_base, current, overlay| {
        if !current.roles.iter().any(|r| r.id == role.id) {
            return Err(format!("找不到岗位 {}", role.id));
        }
        overlay.roles.retain(|r| r.id != role.id);
        overlay.roles.push(role);
        Ok(())
    })
    .await?;
    app.org().role(&id).cloned().ok_or_else(|| format!("岗位 {id} 不见了"))
}

/// 删掉一个岗位（D-44），ip.yaml 里的也行。还有员工在这个岗位、或者引擎、MCP 服务器、数据源、记忆、
/// 团队上下文、路由规则、定时任务还指着它时回错误（和 ip.yaml 一样整份校验）。它的记忆和花费记录留着。
pub async fn delete_role(app: &App, id: &RoleId) -> Result<(), String> {
    change(app, |base, current, overlay| {
        if !current.roles.iter().any(|r| &r.id == id) {
            return Err(format!("找不到岗位 {id}"));
        }
        let staff: Vec<&str> = current
            .agents
            .iter()
            .filter(|a| a.role == id.as_str())
            .map(|a| a.id.as_str())
            .collect();
        if !staff.is_empty() {
            return Err(format!(
                "岗位 {id} 还有员工：{}，先把他们删掉或换到别的岗位",
                staff.join("、")
            ));
        }
        overlay.roles.retain(|r| &r.id != id);
        let in_yaml = base.roles.iter().any(|r| &r.id == id);
        if in_yaml && !overlay.removed_roles.iter().any(|r| r == id.as_str()) {
            overlay.removed_roles.push(id.to_string());
        }
        Ok(())
    })
    .await
}

/// 新建一位员工并启动他，回复他的样子。
pub async fn create_agent(app: &App, new: NewAgent) -> Result<Agent, String> {
    if new.name.trim().is_empty() {
        return Err("员工的名字不能为空".into());
    }
    let id = new.id.clone();
    change(app, |_base, current, overlay| {
        if current.agents.iter().any(|a| a.id == new.id.as_str()) {
            return Err(format!("员工 {} 已经有了", new.id));
        }
        let style = new.celestial_style.filter(|s| !s.is_empty());
        overlay.removed_agents.retain(|r| r != new.id.as_str());
        overlay.agents.retain(|a| a.id != new.id.as_str());
        overlay.agents.push(ConsoleAgent {
            id: new.id.to_string(),
            role: new.role.to_string(),
            name: new.name.trim().to_string(),
            tools: new.tools,
            celestial_style: style,
        });
        Ok(())
    })
    .await?;
    find_agent(app, &id)
}

/// 改一位员工（ip.yaml 里的也行），回复改完的样子。
pub async fn update_agent(app: &App, id: &AgentId, patch: AgentPatch) -> Result<Agent, String> {
    if patch.name.as_ref().is_some_and(|n| n.trim().is_empty()) {
        return Err("员工的名字不能为空".into());
    }
    change(app, |_base, current, overlay| {
        let Some(now) = current.agents.iter().find(|a| a.id == id.as_str()) else {
            return Err(format!("找不到员工 {id}"));
        };
        let mut next = ConsoleAgent {
            id: now.id.clone(),
            role: now.role.clone(),
            name: now.name.clone(),
            tools: now.tools,
            celestial_style: now.celestial_style.clone(),
        };
        if let Some(name) = patch.name {
            next.name = name.trim().to_string();
        }
        if let Some(role) = patch.role {
            next.role = role.to_string();
        }
        if let Some(tools) = patch.tools {
            next.tools = tools;
        }
        if let Some(style) = patch.celestial_style {
            next.celestial_style = (!style.is_empty()).then_some(style);
        }
        overlay.agents.retain(|a| a.id != id.as_str());
        overlay.agents.push(next);
        Ok(())
    })
    .await?;
    find_agent(app, id)
}

/// 删掉一位员工（ip.yaml 里的也行）。他的对话、任务、记忆和工作目录都留着。
pub async fn delete_agent(app: &App, id: &AgentId) -> Result<(), String> {
    change(app, |base, current, overlay| {
        if !current.agents.iter().any(|a| a.id == id.as_str()) {
            return Err(format!("找不到员工 {id}"));
        }
        overlay.agents.retain(|a| a.id != id.as_str());
        let in_yaml = base.agents.iter().any(|a| a.id == id.as_str());
        if in_yaml && !overlay.removed_agents.iter().any(|r| r == id.as_str()) {
            overlay.removed_agents.push(id.to_string());
        }
        Ok(())
    })
    .await
}

fn find_agent(app: &App, id: &AgentId) -> Result<Agent, String> {
    app.runtime
        .agents()
        .into_iter()
        .find(|a| &a.id == id)
        .ok_or_else(|| format!("员工 {id} 没启动起来"))
}

/// 员工和岗位 id 的字符集，和 ip.yaml 的员工 id 一样。
fn id_ok(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// 改名字、岗位、工具、工作目录要重启员工；只改形态不用。
fn needs_respawn(a: &AgentConfig, b: &AgentConfig) -> bool {
    a.role != b.role || a.name != b.name || a.tools != b.tools || a.workspace != b.workspace
}

/// 一次组织改动：在改动的副本上做 `edit`，拼出完整配置整份校验，再让运行时、路由、存储跟上，推 `org_changed`。
/// `edit` 拿到 ip.yaml 原样、现在生效的配置和改动的副本。
async fn change(
    app: &App,
    edit: impl FnOnce(&Config, &Config, &mut Overlay) -> Result<(), String>,
) -> Result<(), String> {
    let admin = app
        .org_admin
        .as_ref()
        .ok_or("这台主机不是按 ip.yaml 启动的，不能在线改组织")?;
    let mut overlay = admin.overlay.lock().await;
    // 和保存路由规则互斥：两边都会换掉路由。
    let _rules = app.rules_lock.lock().await;
    let current = overlay.apply(&admin.base);
    let mut next_overlay = overlay.clone();
    edit(&admin.base, &current, &mut next_overlay)?;
    let next = next_overlay.apply(&admin.base);
    next.validate().map_err(|e| e.to_string())?;
    let org = next.org();
    let agents: Vec<Agent> = next.agents.iter().map(AgentConfig::agent).collect();
    let router = Router::new(app.router().spec().clone(), &org, &agents)
        .map_err(|e| format!("现在的路由规则和改后的组织对不上：{e}"))?;

    let rt = &app.runtime;
    let old_org = current.org();
    let old: HashMap<&str, &AgentConfig> = current.agents.iter().map(|a| (a.id.as_str(), a)).collect();
    let new: HashMap<&str, &AgentConfig> = next.agents.iter().map(|a| (a.id.as_str(), a)).collect();
    // 员工手里拿着岗位定义的副本（模型、预算、说明）：岗位改了也要重启。
    let respawn = |a: &AgentConfig, b: &AgentConfig| {
        needs_respawn(a, b) || old_org.role(&RoleId::from(a.role.as_str())) != org.role(&RoleId::from(b.role.as_str()))
    };
    let mut workspaces = (*app.workspaces()).clone();
    // 先停掉要删和要重启的：他在干活时这里就报错返回，什么都还没动。
    for (id, a) in &old {
        if new.get(id).is_none_or(|b| respawn(a, b)) {
            let id = AgentId::from(*id);
            rt.remove_agent(&id).await.map_err(|e| e.to_string())?;
            workspaces.remove(&id);
        }
    }
    for role in &old_org.roles {
        if org.role(&role.id).is_none() {
            rt.remove_role(&role.id);
        }
    }
    for role in &org.roles {
        rt.add_role(role.clone());
    }
    for (id, b) in &new {
        if old.get(id).is_none_or(|a| respawn(a, b)) {
            let role = org.role(&RoleId::from(b.role.as_str())).ok_or("岗位不存在")?;
            let (setup, workspace) = admin.factory.setup(b, role).map_err(|e| format!("{e:#}"))?;
            rt.spawn_agent(setup).map_err(|e| e.to_string())?;
            if let Some(ws) = workspace {
                workspaces.insert(AgentId::from(*id), ws);
            }
        } else if old.get(id).is_some_and(|a| a.celestial_style != b.celestial_style) {
            rt.set_agent_style(&AgentId::from(*id), b.celestial_style.clone())
                .map_err(|e| e.to_string())?;
        }
    }
    app.set_org(org.clone());
    app.set_router(router);
    app.set_workspaces(workspaces);
    let value = serde_json::to_value(&next_overlay).map_err(|e| e.to_string())?;
    let saved = rt.store().put_setting(KEY, value).await;
    *overlay = next_overlay;
    rt.publish(RuntimeEvent::OrgChanged {
        roles: org.roles.clone(),
        agents: rt.agents(),
    });
    saved
        .map(|_| ())
        .map_err(|e| format!("已经生效，但没能存下，重启后会回到原来的样子：{e}"))
}
