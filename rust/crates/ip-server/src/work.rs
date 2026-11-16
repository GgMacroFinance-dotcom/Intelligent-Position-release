//! 发消息、任务、审批这些需要跨运行时、存储、路由协作的操作。

use ip_core::{
    AgentId, Approval, ApprovalId, ApprovalOption, ApprovalState, ApprovalSubject, Author, Notice, RoleId, Task,
    TaskId, ThreadId, now_ms,
};
use ip_router::{Decision, RouteInput};
use ip_runtime::NewTask;
use ip_runtime::RuntimeEvent;

use crate::App;
use crate::protocol::SendResult;

/// 发一条消息。指定了员工就直接交给他；否则按路由规则分派，拿不准时建一张审批卡片等人选岗位。
pub async fn send(
    app: &App,
    agent_id: Option<AgentId>,
    thread_id: Option<ThreadId>,
    text: String,
    author: String,
) -> Result<SendResult, String> {
    send_as(app, agent_id, thread_id, text, Author::human(author)).await
}

/// 同 [`send`]，作者由调用方给出：外部渠道来的人带着 `via`，路由规则 `from` 按它匹配（D-27）。
pub async fn send_as(
    app: &App,
    agent_id: Option<AgentId>,
    thread_id: Option<ThreadId>,
    text: String,
    human: Author,
) -> Result<SendResult, String> {
    if text.trim().is_empty() {
        return Err("消息不能为空".into());
    }
    let (author, via) = match &human {
        Author::Human { name, via } => (name.clone(), via.as_ref().map(ip_core::Via::key)),
        _ => return Err("只有人能发消息".into()),
    };
    let rt = &app.runtime;
    let thread_id = thread_id.unwrap_or_default(); // 新对话：Default 生成新 id
    if let Some(agent) = agent_id {
        let message = rt
            .send(&agent, thread_id, human, text)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(SendResult {
            message,
            agent_id: Some(agent),
            decision: None,
            approval_id: None,
        });
    }

    let decision = crate::routing::decide(
        app,
        RouteInput {
            text: &text,
            author: Some(&author),
            via: via.as_deref(),
        },
    )
    .await;
    match &decision {
        Decision::Routed {
            role, agent, escalate, ..
        } => {
            let agent = match agent {
                Some(a) => a.clone(),
                None => rt.least_busy(role).ok_or_else(|| format!("岗位 {role} 没有员工"))?,
            };
            let message = rt
                .send_with(&agent, thread_id, human, text, *escalate)
                .await
                .map_err(|e| e.to_string())?;
            Ok(SendResult {
                message,
                agent_id: Some(agent),
                decision: Some(decision),
                approval_id: None,
            })
        }
        Decision::NeedsHuman { reason, suggested } => {
            let message = rt
                .post(thread_id.clone(), human, text.clone())
                .await
                .map_err(|e| e.to_string())?;
            let roles: Vec<RoleId> = app.org.roles.iter().map(|r| r.id.clone()).collect();
            if roles.is_empty() {
                return Err("还没有任何岗位".into());
            }
            let options = app
                .org
                .roles
                .iter()
                .map(|r| ApprovalOption {
                    label: r.display_name().to_string(),
                    consequence: format!("交给{}岗位当前最空闲的员工处理", r.display_name()),
                })
                .collect();
            let approval = Approval {
                id: ApprovalId::new(),
                question: format!("这条消息交给哪个岗位？{}", excerpt(&text)),
                options,
                recommended: suggested
                    .as_ref()
                    .and_then(|s| app.org.roles.iter().position(|r| &r.id == s))
                    .map(|i| i as u32),
                reason: reason.clone(),
                subject: ApprovalSubject::RouteMessage {
                    message_id: message.id.clone(),
                    thread_id,
                    text,
                    roles,
                },
                state: ApprovalState::Pending,
                created_at_ms: now_ms(),
            };
            let approval_id = approval.id.clone();
            rt.open_approval(approval).await.map_err(|e| e.to_string())?;
            Ok(SendResult {
                message,
                agent_id: None,
                decision: Some(decision),
                approval_id: Some(approval_id),
            })
        }
    }
}

/// 卡片标题里引用的消息摘要。
fn excerpt(text: &str) -> String {
    let short: String = text.chars().take(40).collect();
    if short.len() < text.len() {
        format!("「{short}…」")
    } else {
        format!("「{short}」")
    }
}

pub async fn create_task(app: &App, new: NewTask, author: String) -> Result<Task, String> {
    app.runtime
        .create_task(new, Author::human(author))
        .await
        .map_err(|e| e.to_string())
}

pub async fn update_task(app: &App, id: &TaskId, update: ip_runtime::TaskUpdate, by: String) -> Result<Task, String> {
    app.runtime
        .update_task(id, update, &Author::human(by))
        .await
        .map_err(|e| e.to_string())
}

/// 选定审批卡片。路由类卡片选定后把那条消息交给所选岗位最空闲的员工；
/// 员工提的问题选定后把结果写回对话，叫这位员工接着做。
pub async fn resolve_approval(app: &App, id: &ApprovalId, choice: u32, by: String) -> Result<Approval, String> {
    let rt = &app.runtime;
    let approval = rt
        .store()
        .resolve_approval(id, choice, by.clone())
        .await
        .map_err(|e| e.to_string())?;
    rt.publish(RuntimeEvent::ApprovalChanged {
        approval: approval.clone(),
    });
    let (thread_id, agent, note) = match &approval.subject {
        ApprovalSubject::RouteMessage {
            thread_id, roles, text, ..
        } => {
            let role = roles.get(choice as usize).ok_or("选项和岗位对不上")?;
            let agent = rt.least_busy(role).ok_or_else(|| format!("岗位 {role} 没有员工"))?;
            let note = format!("{by} 把这条消息交给了 {}（{}）", agent_name(app, &agent), role);
            // 这个对话绑定了外部会话、还没记员工的话，补上：之后会话里的消息都交给他（D-05）。
            if let Ok(Some(b)) = rt.store().binding_by_thread(thread_id).await
                && let Err(e) = rt.store().set_binding_agent(&b.channel, &b.conversation, &agent).await
            {
                tracing::warn!(error = %e, "没能把员工补进渠道绑定");
            }
            // 人的选择是最好的路由样本：生成一条规则草案，等人审核。失败不影响分派。
            let human = Author::human(by.clone());
            if let Err(e) = crate::rules::draft_from_override(app, text, role, &human).await {
                tracing::warn!(error = %e, "生成规则草案失败");
            }
            (thread_id, agent, note)
        }
        ApprovalSubject::AgentQuestion { agent_id, thread_id } => {
            let label = approval.options.get(choice as usize).map_or("", |o| o.label.as_str());
            let note = format!("{by} 回答了「{}」：选「{label}」", approval.question);
            (thread_id, agent_id.clone(), note)
        }
        ApprovalSubject::Question => return Ok(approval),
    };
    rt.post_notice(thread_id.clone(), Notice::ApprovalAnswer, note)
        .await
        .map_err(|e| e.to_string())?;
    rt.dispatch(&agent, thread_id.clone(), false)
        .await
        .map_err(|e| e.to_string())?;
    Ok(approval)
}

fn agent_name(app: &App, id: &AgentId) -> String {
    app.runtime
        .agents()
        .into_iter()
        .find(|a| &a.id == id)
        .map_or_else(|| id.to_string(), |a| a.name)
}
