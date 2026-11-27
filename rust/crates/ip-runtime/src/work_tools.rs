//! 组织层面的工具：建任务、改任务、看任务、请人拍板。每个员工都有，不需要工作目录。

use ip_core::{
    Agent, AgentId, Approval, ApprovalId, ApprovalOption, ApprovalState, ApprovalSubject, Author, RoleId, TaskId,
    TaskStatus, ThreadId, now_ms,
};
use ip_models::ToolSpec;
use ip_tools::ToolOutput;
use serde::Deserialize;
use serde_json::json;

use crate::{NewTask, Shared, TaskUpdate};

/// 列任务时最多返回的条数。
const LIST_LIMIT: usize = 50;

pub fn specs() -> Vec<ToolSpec> {
    let status = json!({"type": "string", "enum": ["open", "in_progress", "blocked", "done", "cancelled"]});
    vec![
        ToolSpec {
            name: "create_task".into(),
            description: "在组织的任务板上建一个任务。指定负责人（员工 id）时，系统会另开一个对话把任务交给他；\
                          他做完、受阻或取消时，结果会以系统消息写回当前对话，你会被叫回来汇总。\
                          只指定岗位或都不指定时，任务留给人来分配。"
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "description": {"type": "string"},
                    "assignee": {"type": "string", "description": "负责的员工 id"},
                    "role": {"type": "string", "description": "负责的岗位 id"},
                    "due_ms": {"type": "integer", "description": "截止时间，Unix 毫秒"}
                },
                "required": ["title"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "update_task".into(),
            description: "改任务状态、负责人或结果。开始做时设 in_progress；做完设 done 并在 result 里写结论；\
                          卡住设 blocked 并在 result 里写缺什么。结论会自动回报给派活的人。"
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "status": status,
                    "assignee": {"type": "string", "description": "新的负责员工 id"},
                    "result": {"type": "string", "description": "交回的结论或受阻原因"}
                },
                "required": ["task_id"],
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "list_tasks".into(),
            description: "列出未完成的任务，最近更新的在前。默认只列分给你或你所在岗位的。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "all": {"type": "boolean", "description": "列出所有人的任务"},
                    "include_closed": {"type": "boolean", "description": "包括已完成和已取消的"}
                },
                "additionalProperties": false
            }),
        },
        ToolSpec {
            name: "ask_human".into(),
            description: "需要人拍板时，发一张选择卡片给人：一个问题、2 到 4 个选项、一个推荐项。\
                          卡片是异步的：调用后立即返回，人选定后结果会以系统消息写进这个对话，你会被叫回来接着做。\
                          发完卡片就结束这一轮，或先做不依赖这个决定的事；不要等待或反复询问。"
                .into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "question": {"type": "string"},
                    "options": {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 4,
                        "items": {
                            "type": "object",
                            "properties": {
                                "label": {"type": "string"},
                                "consequence": {"type": "string", "description": "选它会发生什么，一句话"}
                            },
                            "required": ["label", "consequence"],
                            "additionalProperties": false
                        }
                    },
                    "recommended": {"type": "integer", "minimum": 0, "description": "推荐项的序号，从 0 开始"},
                    "reason": {"type": "string", "description": "推荐理由"}
                },
                "required": ["question", "options", "recommended"],
                "additionalProperties": false
            }),
        },
    ]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateArgs {
    title: String,
    #[serde(default)]
    description: String,
    assignee: Option<AgentId>,
    role: Option<RoleId>,
    due_ms: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateArgs {
    task_id: TaskId,
    status: Option<TaskStatus>,
    assignee: Option<AgentId>,
    result: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    #[serde(default)]
    all: bool,
    #[serde(default)]
    include_closed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskArgs {
    question: String,
    options: Vec<ApprovalOption>,
    recommended: u32,
    #[serde(default)]
    reason: String,
}

/// 执行一个组织工具；不是组织工具时返回 `None`。
pub async fn call(shared: &Shared, me: &Agent, thread: &ThreadId, name: &str, arguments: &str) -> Option<ToolOutput> {
    let args = if arguments.trim().is_empty() { "{}" } else { arguments };
    let out = match name {
        "create_task" => match serde_json::from_str::<CreateArgs>(args) {
            Ok(a) => {
                let new = NewTask {
                    title: a.title,
                    description: a.description,
                    role: a.role,
                    assignee: a.assignee,
                    thread_id: None,
                    origin_thread_id: Some(thread.clone()),
                    due_ms: a.due_ms,
                    needs_approval: false,
                };
                let by = Author::Agent {
                    agent_id: me.id.clone(),
                };
                result(shared.create_task(new, by).await.map(|t| {
                    format!(
                        "已建任务 {}「{}」，负责人：{}",
                        t.id,
                        t.title,
                        t.assignee.map_or_else(|| "待分配".to_string(), |a| a.to_string())
                    )
                }))
            }
            Err(e) => bad_args(name, e),
        },
        "update_task" => match serde_json::from_str::<UpdateArgs>(args) {
            Ok(a) => {
                let update = TaskUpdate {
                    status: a.status,
                    assignee: a.assignee,
                    result: a.result,
                };
                let by = Author::Agent {
                    agent_id: me.id.clone(),
                };
                result(
                    shared
                        .update_task(&a.task_id, update, &by)
                        .await
                        .map(|t| match t.status {
                            TaskStatus::PendingApproval => format!(
                                "任务 {} 已交给人验收。人通过后结论会回报给派活的人，打回时会叫你接着改；现在结束这一轮。",
                                t.id
                            ),
                            s => format!("任务 {} 现在是 {}", t.id, status_label(s)),
                        }),
                )
            }
            Err(e) => bad_args(name, e),
        },
        "list_tasks" => match serde_json::from_str::<ListArgs>(args) {
            Ok(a) => list(shared, me, a).await,
            Err(e) => bad_args(name, e),
        },
        "ask_human" => match serde_json::from_str::<AskArgs>(args) {
            Ok(a) if a.question.trim().is_empty() => ToolOutput::err("问题不能为空"),
            Ok(a) if !(2..=4).contains(&a.options.len()) => ToolOutput::err("要给 2 到 4 个选项"),
            Ok(a) => {
                let approval = Approval {
                    id: ApprovalId::new(),
                    question: a.question.trim().to_string(),
                    options: a.options,
                    recommended: Some(a.recommended),
                    reason: a.reason,
                    subject: ApprovalSubject::AgentQuestion {
                        agent_id: me.id.clone(),
                        thread_id: thread.clone(),
                    },
                    state: ApprovalState::Pending,
                    created_at_ms: now_ms(),
                };
                result(shared.open_approval(approval).await.map(|a| {
                    format!(
                        "卡片 {} 已发出。人选定后结果会写进这个对话，你会被叫回来；现在先结束这一轮，或做不依赖它的事。",
                        a.id
                    )
                }))
            }
            Err(e) => bad_args(name, e),
        },
        _ => return None,
    };
    Some(out)
}

async fn list(shared: &Shared, me: &Agent, a: ListArgs) -> ToolOutput {
    // 先多取一些再按“我的”筛，避免别人的任务占满窗口。
    let tasks = match shared.store.tasks(!a.include_closed, LIST_LIMIT * 10).await {
        Ok(t) => t,
        Err(e) => return ToolOutput::err(e.to_string()),
    };
    let mine = |t: &ip_core::Task| match &t.assignee {
        Some(x) => *x == me.id,
        None => t.role.as_ref() == Some(&me.role),
    };
    let lines: Vec<String> = tasks
        .iter()
        .filter(|t| a.all || mine(t))
        .take(LIST_LIMIT)
        .map(|t| {
            format!(
                "{} [{}] {}（负责人：{}）",
                t.id,
                status_label(t.status),
                t.title,
                t.assignee.as_ref().map_or("待分配", |x| x.as_str())
            )
        })
        .collect();
    if lines.is_empty() {
        ToolOutput::ok("没有任务")
    } else {
        ToolOutput::ok(lines.join("\n"))
    }
}

fn status_label(s: TaskStatus) -> &'static str {
    match s {
        TaskStatus::Open => "待开始",
        TaskStatus::InProgress => "进行中",
        TaskStatus::Blocked => "受阻",
        TaskStatus::PendingApproval => "已完成、等人验收",
        TaskStatus::Done => "已完成",
        TaskStatus::Cancelled => "已取消",
    }
}

fn result(r: Result<String, crate::RuntimeError>) -> ToolOutput {
    match r {
        Ok(s) => ToolOutput::ok(s),
        Err(e) => ToolOutput::err(e.to_string()),
    }
}

fn bad_args(name: &str, e: serde_json::Error) -> ToolOutput {
    ToolOutput::err(format!("{name} 的参数不对：{e}"))
}
