//! 通知规则（backlog F-9，`docs/ip/apps-plan.md` 第 5 节，`docs/ip/m2-spec.md` D-32）。
//!
//! 一个任务从运行时收事件，按第 5.1 节挑出要告诉人的几种，做成 [`Alert`] 广播出去；各 `/ws` 连接再按
//! 这台设备自己的设置（[`NotifySettings`]）决定发不发。以后的 Web Push（F-10）用同一份规则和设置。
//!
//! 流式输出的每一段文字都是一个事件，所以 [`Rules::observe`] 在热路径上：不相干的事件要立即放过，
//! 预览文字只在真要通知时才做。基准见 `ip bench notify`。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

use ip_core::{Agent, AgentId, Approval, ApprovalSubject, Author, Body, RoleId, Task, TaskId, TaskStatus, ThreadId};
use ip_runtime::RuntimeEvent;
use ip_store::Store;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use ts_rs::TS;

/// 共用令牌（或本机不设令牌）连上的控制台共用一份通知设置，存在这个名字下。
pub const SHARED: &str = "shared";
const SETTINGS_KEY: &str = "notify_settings";
/// 预览最多几个字。
const PREVIEW_CHARS: usize = 120;
/// 预览最多几句。
const PREVIEW_SENTENCES: usize = 2;
/// 预览只看正文开头这么多字节（去掉代码块、表格之后）。比 120 个字长得多，开头附近的密钥（私钥除外）都整段在里面。
const HEAD_BYTES: usize = 2048;
/// 记着的进行中的回复、任务状态最多几条；超过就清空重来（只会多通知或少通知一次）。
const MAX_TRACKED: usize = 4096;
/// 按岗位静音最多列几个岗位。
const MAX_MUTED_ROLES: usize = 64;

/// 一条通知有多要紧（`apps-plan.md` 5.1）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AlertLevel {
    /// 要人处理：审批卡、员工提问、一轮失败、任务受阻。
    NeedsMe,
    /// 知道一下就行：员工回复了、任务完成交回。
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AlertKind {
    /// 新的审批卡（路由拿不准的消息、一般问题）。
    Approval,
    /// 员工在工作中提的问题。
    Question,
    /// 一轮失败（定时任务触发的也算）。
    Failed,
    /// 人派的任务受阻。
    Blocked,
    /// 员工回复了。
    Reply,
    /// 人派的任务完成交回。
    TaskDone,
}

/// 推给设备的一条通知。
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Alert {
    pub kind: AlertKind,
    pub level: AlertLevel,
    pub title: String,
    /// 预览：去掉表格、代码块和 Markdown 记号，密钥打码，最多两句。设备关了预览时为空。
    pub body: String,
    /// 同一件事的通知用同一个标签，后来的盖掉先来的；处理过后按标签撤掉。
    pub tag: String,
    /// 点开去哪个对话；没有时去“待办”。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub thread_id: Option<ThreadId>,
    /// 哪个岗位的事，按岗位静音用。
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub role_id: Option<RoleId>,
    #[ts(type = "number")]
    pub at_ms: i64,
}

/// 广播给各连接的：一条通知，或者撤掉某个标签的通知（审批在别处处理过了）。
#[derive(Clone, Debug, PartialEq)]
pub enum Outgoing {
    Alert(Alert),
    Dismiss(String),
}

/// 一台设备要哪些通知。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum NotifyLevel {
    /// 不通知。
    Off,
    /// 只通知要人处理的。
    #[default]
    NeedsMe,
    /// 全部。
    All,
}

/// 一台设备自己的通知设置（`apps-plan.md` 5.1 最后一段）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NotifySettings {
    #[serde(default)]
    pub level: NotifyLevel,
    /// 这些岗位的事不通知。
    #[serde(default)]
    pub muted_roles: Vec<RoleId>,
    /// 这个时间之前都不通知。
    #[serde(default)]
    #[ts(type = "number | null")]
    pub mute_until_ms: Option<i64>,
    /// 通知里带不带预览文字。
    #[serde(default = "yes")]
    pub preview: bool,
}

fn yes() -> bool {
    true
}

impl Default for NotifySettings {
    fn default() -> Self {
        Self {
            level: NotifyLevel::NeedsMe,
            muted_roles: Vec::new(),
            mute_until_ms: None,
            preview: true,
        }
    }
}

impl NotifySettings {
    /// 这条通知要不要发给这台设备；要发时给出发出去的样子（关了预览就去掉正文）。
    pub fn filter(&self, alert: &Alert, now_ms: i64) -> Option<Alert> {
        let wanted = match self.level {
            NotifyLevel::Off => false,
            NotifyLevel::NeedsMe => alert.level == AlertLevel::NeedsMe,
            NotifyLevel::All => true,
        };
        if !wanted || self.mute_until_ms.is_some_and(|until| now_ms < until) {
            return None;
        }
        if alert.role_id.as_ref().is_some_and(|r| self.muted_roles.contains(r)) {
            return None;
        }
        let mut out = alert.clone();
        if !self.preview {
            out.body.clear();
        }
        Some(out)
    }

    fn clean(mut self) -> Result<Self, String> {
        self.muted_roles.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        self.muted_roles.dedup();
        if self.muted_roles.len() > MAX_MUTED_ROLES {
            return Err(format!("按岗位静音最多 {MAX_MUTED_ROLES} 个岗位"));
        }
        Ok(self)
    }
}

/// 从运行时事件里挑出要通知的（`apps-plan.md` 5.1）。只在转发任务里用，不加锁。
#[derive(Default)]
pub struct Rules {
    /// 正在进行的一轮里员工最后说的话。
    replies: HashMap<ThreadId, String>,
    /// 人派的任务上次看到的状态，免得任务每改一次就通知一次。
    tasks: HashMap<TaskId, TaskStatus>,
}

impl Rules {
    /// 看一个事件，要通知时返回通知。`agent` 按员工 id 查员工（名字和岗位），只在真要通知时才调用。
    pub fn observe(
        &mut self,
        event: &RuntimeEvent,
        now_ms: i64,
        agent: impl Fn(&AgentId) -> Option<Agent>,
    ) -> Option<Outgoing> {
        match event {
            RuntimeEvent::Delta { .. } => None,
            RuntimeEvent::TurnStarted { thread_id, .. } => {
                self.replies.remove(thread_id);
                None
            }
            RuntimeEvent::Message { message } => {
                if let (Author::Agent { .. }, Body::Text { text }, None) =
                    (&message.author, &message.body, message.notice)
                    && !text.trim().is_empty()
                {
                    if self.replies.len() >= MAX_TRACKED {
                        self.replies.clear();
                    }
                    self.replies.insert(message.thread_id.clone(), text.clone());
                }
                None
            }
            RuntimeEvent::TurnFinished {
                agent_id, thread_id, ..
            } => {
                let text = self.replies.remove(thread_id)?;
                let who = agent(agent_id);
                Some(Outgoing::Alert(Alert {
                    kind: AlertKind::Reply,
                    level: AlertLevel::All,
                    title: format!("{} 回复了", name_of(who.as_ref(), agent_id)),
                    body: preview(&text),
                    tag: format!("thread:{thread_id}"),
                    thread_id: Some(thread_id.clone()),
                    role_id: who.map(|a| a.role),
                    at_ms: now_ms,
                }))
            }
            RuntimeEvent::TurnFailed {
                agent_id,
                thread_id,
                error,
            } => {
                self.replies.remove(thread_id);
                let who = agent(agent_id);
                Some(Outgoing::Alert(Alert {
                    kind: AlertKind::Failed,
                    level: AlertLevel::NeedsMe,
                    title: format!("{} 这一轮失败了", name_of(who.as_ref(), agent_id)),
                    body: preview(error.lines().next().unwrap_or_default()),
                    tag: format!("thread:{thread_id}"),
                    thread_id: Some(thread_id.clone()),
                    role_id: who.map(|a| a.role),
                    at_ms: now_ms,
                }))
            }
            RuntimeEvent::ApprovalChanged { approval } => Some(approval_alert(approval, now_ms, agent)),
            RuntimeEvent::TaskChanged { task } => self.task_alert(task, now_ms),
            RuntimeEvent::OrbitPinsChanged { .. } => None,
        }
    }

    fn task_alert(&mut self, task: &Task, now_ms: i64) -> Option<Outgoing> {
        // 员工派给员工的任务，结果交回给派活的员工，不打扰人。
        if matches!(task.created_by, Author::Agent { .. }) {
            return None;
        }
        if self.tasks.len() >= MAX_TRACKED && !self.tasks.contains_key(&task.id) {
            self.tasks.clear();
        }
        let before = self.tasks.insert(task.id.clone(), task.status);
        if before == Some(task.status) {
            return None;
        }
        let tag = format!("task:{}", task.id);
        let (kind, level, title) = match task.status {
            TaskStatus::Done => (
                AlertKind::TaskDone,
                AlertLevel::All,
                format!("任务完成：{}", task.title),
            ),
            TaskStatus::Blocked => (
                AlertKind::Blocked,
                AlertLevel::NeedsMe,
                format!("任务受阻：{}", task.title),
            ),
            // 受阻的任务又动起来了（或取消了），撤掉受阻的通知。
            _ if before == Some(TaskStatus::Blocked) => return Some(Outgoing::Dismiss(tag)),
            _ => return None,
        };
        Some(Outgoing::Alert(Alert {
            kind,
            level,
            title,
            body: preview(task.result.as_deref().unwrap_or_default()),
            tag,
            thread_id: task.origin_thread_id.clone().or_else(|| task.thread_id.clone()),
            role_id: task.role.clone(),
            at_ms: now_ms,
        }))
    }
}

fn approval_alert(approval: &Approval, now_ms: i64, agent: impl Fn(&AgentId) -> Option<Agent>) -> Outgoing {
    let tag = format!("approval:{}", approval.id);
    if !approval.is_pending() {
        // 在一处处理过了，别的设备上的那条撤掉（`apps-plan.md` 5.2）。
        return Outgoing::Dismiss(tag);
    }
    let (kind, title, body, thread_id, role_id) = match &approval.subject {
        ApprovalSubject::AgentQuestion { agent_id, thread_id } => {
            let who = agent(agent_id);
            (
                AlertKind::Question,
                format!("{} 在问你", name_of(who.as_ref(), agent_id)),
                preview(&approval.question),
                Some(thread_id.clone()),
                who.map(|a| a.role),
            )
        }
        ApprovalSubject::RouteMessage { thread_id, text, .. } => (
            AlertKind::Approval,
            "这条消息该交给谁？".to_string(),
            preview(text),
            Some(thread_id.clone()),
            None,
        ),
        ApprovalSubject::Question => (
            AlertKind::Approval,
            "等你决定".to_string(),
            preview(&approval.question),
            None,
            None,
        ),
    };
    Outgoing::Alert(Alert {
        kind,
        level: AlertLevel::NeedsMe,
        title,
        body,
        tag,
        thread_id,
        role_id,
        at_ms: now_ms,
    })
}

fn name_of(agent: Option<&Agent>, id: &AgentId) -> String {
    agent.map_or_else(|| id.to_string(), |a| a.name.clone())
}

/// 通知里的预览：密钥打码，去掉代码块、表格和 Markdown 记号，取头两句，最多 120 个字。
pub fn preview(text: &str) -> String {
    // 1. 去掉代码块和表格，只留开头一段：脱敏是一串正则，报告可能很长。
    let mut head = String::with_capacity(HEAD_BYTES + 256);
    let mut in_code = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("```") || line.starts_with("~~~") {
            in_code = !in_code;
            continue;
        }
        if in_code || line.is_empty() || line.starts_with('|') {
            continue;
        }
        head.push_str(line);
        head.push('\n');
        if head.len() >= HEAD_BYTES {
            break;
        }
    }
    if head.len() > HEAD_BYTES {
        let mut cut = HEAD_BYTES;
        while !head.is_char_boundary(cut) {
            cut -= 1;
        }
        head.truncate(cut);
    }
    // 2. 脱敏要在去掉 Markdown 记号之前做，不然 `ghp_…` 这类密钥少了下划线就认不出来。
    //    截到的这段里还有私钥开头，说明私钥比这段还长、没被整段换掉，从那里起都不要。
    let mut head = ip_runtime::memory::redact::redact(&head);
    if let Some(i) = head.find("-----BEGIN") {
        head.truncate(i);
    }
    // 3. 去掉行首的标题、引用、列表记号和行内记号，连成一行。
    let mut plain = String::with_capacity(head.len());
    for line in head.lines() {
        let line = line
            .trim_start_matches(['#', '>'])
            .trim_start()
            .trim_start_matches(['-', '*', '+'])
            .trim_start();
        if line.is_empty() {
            continue;
        }
        if !plain.is_empty() {
            plain.push(' ');
        }
        plain.push_str(line);
    }
    let plain = strip_inline(&plain);
    // 4. 头两句，最多 120 个字。
    let mut out = String::new();
    let mut sentences = 0;
    let mut chars = plain.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        let ends = matches!(c, '。' | '！' | '？' | '!' | '?' | '；')
            || (c == '.' && chars.peek().is_none_or(|n| n.is_whitespace()));
        if ends {
            sentences += 1;
            if sentences >= PREVIEW_SENTENCES {
                break;
            }
        }
        if out.len() > PREVIEW_CHARS * 4 {
            break;
        }
    }
    let out = out.trim();
    if out.chars().count() > PREVIEW_CHARS {
        let cut: String = out.chars().take(PREVIEW_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        out.to_string()
    }
}

/// 去掉行内的 Markdown 记号：`**粗体**`、`` `代码` ``、`[文字](链接)` 只留文字。
/// 下划线不动：`MACRO_API_KEY`、`cpi_10.md` 这类名字里常有，模型也很少用下划线标粗体。
fn strip_inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if c == '['
            && let Some(close) = rest.find("](")
            && let Some(end) = rest[close..].find(')')
            && !rest[1..close].contains('[')
        {
            out.push_str(&rest[1..close]);
            rest = &rest[close + end + 1..];
            continue;
        }
        if !matches!(c, '*' | '`') {
            out.push(c);
        }
        rest = &rest[c.len_utf8()..];
    }
    // 连续空白并成一个。
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 通知规则的转发任务和各设备的设置。
pub struct Notifier {
    store: Store,
    settings: RwLock<HashMap<String, NotifySettings>>,
    /// 串行化设置的读改写。
    write: tokio::sync::Mutex<()>,
    tx: broadcast::Sender<Arc<Outgoing>>,
    started: AtomicBool,
}

impl Notifier {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            settings: RwLock::new(HashMap::new()),
            write: tokio::sync::Mutex::new(()),
            tx: broadcast::channel(256).0,
            started: AtomicBool::new(false),
        }
    }

    /// 启动时从库里读各设备的设置。
    pub async fn load(&self) -> Result<(), String> {
        let all = self.stored().await?;
        *self.settings.write().unwrap_or_else(std::sync::PoisonError::into_inner) = all;
        Ok(())
    }

    /// `who` 的设置：设备 id，共用令牌时是 [`SHARED`]。没设过的是默认（只推需要我的、带预览）。
    pub fn settings(&self, who: &str) -> NotifySettings {
        self.settings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(who)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn save(&self, who: &str, settings: NotifySettings) -> Result<NotifySettings, String> {
        let settings = settings.clean()?;
        let _w = self.write.lock().await;
        let mut all = self.stored().await?;
        all.insert(who.to_string(), settings.clone());
        let value = serde_json::to_value(&all).map_err(|e| e.to_string())?;
        self.store
            .put_setting(SETTINGS_KEY, value)
            .await
            .map_err(|e| e.to_string())?;
        *self.settings.write().unwrap_or_else(std::sync::PoisonError::into_inner) = all;
        Ok(settings)
    }

    /// 设备吊销后删掉它的设置。
    pub async fn forget(&self, who: &str) -> Result<(), String> {
        let _w = self.write.lock().await;
        let mut all = self.stored().await?;
        if all.remove(who).is_none() {
            return Ok(());
        }
        let value = serde_json::to_value(&all).map_err(|e| e.to_string())?;
        self.store
            .put_setting(SETTINGS_KEY, value)
            .await
            .map_err(|e| e.to_string())?;
        *self.settings.write().unwrap_or_else(std::sync::PoisonError::into_inner) = all;
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Outgoing>> {
        self.tx.subscribe()
    }

    /// 第一次调用时启动转发任务：从运行时收事件，按 [`Rules`] 挑出通知广播出去。之后再调用什么都不做。
    pub fn start(&self, runtime: ip_runtime::Runtime) {
        if self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        let tx = self.tx.clone();
        let mut events = runtime.subscribe();
        tokio::spawn(async move {
            let mut rules = Rules::default();
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let found = rules.observe(&event, ip_core::now_ms(), |id| {
                            runtime.agents().into_iter().find(|a| &a.id == id)
                        });
                        if let Some(out) = found {
                            // 没有连接时发送失败，不要紧。
                            let _ = tx.send(Arc::new(out));
                        }
                    }
                    // 通知丢几条不要紧，界面上的列表照样会更新。
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "通知规则跟不上运行时，跳过了一些事件");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    async fn stored(&self) -> Result<HashMap<String, NotifySettings>, String> {
        match self.store.setting(SETTINGS_KEY).await.map_err(|e| e.to_string())? {
            Some(v) => serde_json::from_value(v).map_err(|e| format!("设置 {SETTINGS_KEY} 格式不对：{e}")),
            None => Ok(HashMap::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use ip_core::{ApprovalOption, ApprovalState, Message};
    use ip_models::Usage;

    use super::*;

    fn agents(id: &AgentId) -> Option<Agent> {
        (id.as_str() == "macro-1").then(|| Agent {
            id: id.clone(),
            role: RoleId::from("macro"),
            name: "宏观研究员".into(),
        })
    }

    fn agent_says(thread: &str, text: &str) -> RuntimeEvent {
        RuntimeEvent::Message {
            message: Message {
                id: "m1".into(),
                thread_id: thread.into(),
                at_ms: 0,
                author: Author::Agent {
                    agent_id: "macro-1".into(),
                },
                body: Body::Text { text: text.into() },
                notice: None,
            },
        }
    }

    fn finished(thread: &str) -> RuntimeEvent {
        RuntimeEvent::TurnFinished {
            agent_id: "macro-1".into(),
            thread_id: thread.into(),
            steps: 1,
            usage: Usage::default(),
            cost_usd: 0.0,
            elapsed_ms: 1,
        }
    }

    fn alert(out: Option<Outgoing>) -> Alert {
        match out {
            Some(Outgoing::Alert(a)) => a,
            other => panic!("应该是通知：{other:?}"),
        }
    }

    #[test]
    fn a_finished_turn_with_a_reply_is_an_all_level_alert() {
        let mut r = Rules::default();
        assert!(
            r.observe(
                &RuntimeEvent::TurnStarted {
                    agent_id: "macro-1".into(),
                    thread_id: "t".into()
                },
                1,
                agents
            )
            .is_none()
        );
        assert!(
            r.observe(
                &RuntimeEvent::Delta {
                    agent_id: "macro-1".into(),
                    thread_id: "t".into(),
                    text: "10 月".into()
                },
                1,
                agents
            )
            .is_none()
        );
        assert!(
            r.observe(
                &agent_says("t", "## 结论\n10 月 CPI 同比 2.9%。核心通胀回落！后面还有很多。"),
                2,
                agents
            )
            .is_none()
        );
        let a = alert(r.observe(&finished("t"), 3, agents));
        assert_eq!(a.kind, AlertKind::Reply);
        assert_eq!(a.level, AlertLevel::All);
        assert_eq!(a.title, "宏观研究员 回复了");
        assert_eq!(a.body, "结论 10 月 CPI 同比 2.9%。核心通胀回落！");
        assert_eq!(a.tag, "thread:t");
        assert_eq!(a.role_id, Some(RoleId::from("macro")));
        // 没有回复文字的一轮（只调了工具）不通知
        assert!(r.observe(&finished("t"), 4, agents).is_none());
    }

    #[test]
    fn failures_questions_and_resolutions() {
        let mut r = Rules::default();
        let failed = RuntimeEvent::TurnFailed {
            agent_id: "macro-1".into(),
            thread_id: "t".into(),
            error: "模型返回 401：key sk-abcdefghijklmnopqrstuvwxyz 无效\n详细信息".into(),
        };
        let a = alert(r.observe(&failed, 1, agents));
        assert_eq!(a.level, AlertLevel::NeedsMe);
        assert_eq!(a.title, "宏观研究员 这一轮失败了");
        assert!(!a.body.contains("sk-abc") && !a.body.contains("详细信息"), "{}", a.body);

        let mut approval = Approval {
            id: "ap1".into(),
            question: "要不要把 9 月数据也加进来？".into(),
            options: vec![ApprovalOption {
                label: "要".into(),
                consequence: String::new(),
            }],
            recommended: None,
            reason: String::new(),
            subject: ApprovalSubject::AgentQuestion {
                agent_id: "macro-1".into(),
                thread_id: "t".into(),
            },
            state: ApprovalState::Pending,
            created_at_ms: 0,
        };
        let a = alert(r.observe(
            &RuntimeEvent::ApprovalChanged {
                approval: approval.clone(),
            },
            2,
            agents,
        ));
        assert_eq!(a.kind, AlertKind::Question);
        assert_eq!(a.title, "宏观研究员 在问你");
        assert_eq!(a.tag, "approval:ap1");
        approval.state = ApprovalState::Resolved {
            choice: 0,
            by: "张三".into(),
            at_ms: 3,
        };
        assert_eq!(
            r.observe(&RuntimeEvent::ApprovalChanged { approval }, 3, agents),
            Some(Outgoing::Dismiss("approval:ap1".into()))
        );
    }

    #[test]
    fn human_tasks_alert_once_per_status_change() {
        let mut r = Rules::default();
        let mut task: Task = serde_json::from_value(serde_json::json!({
            "id": "task1", "title": "写 10 月 CPI 点评", "status": "in_progress", "role": "macro",
            "assignee": "macro-1", "thread_id": "t2", "origin_thread_id": "t1",
            "created_by": {"kind": "human", "name": "张三"}, "created_at_ms": 0, "updated_at_ms": 0, "due_ms": null
        }))
        .unwrap();
        let changed = |t: &Task| RuntimeEvent::TaskChanged { task: t.clone() };
        assert!(r.observe(&changed(&task), 1, agents).is_none());
        task.status = TaskStatus::Blocked;
        task.result = Some("缺 9 月的数据。".into());
        let a = alert(r.observe(&changed(&task), 2, agents));
        assert_eq!((a.kind, a.level), (AlertKind::Blocked, AlertLevel::NeedsMe));
        assert!(r.observe(&changed(&task), 3, agents).is_none(), "状态没变不再通知");
        task.status = TaskStatus::InProgress;
        assert_eq!(
            r.observe(&changed(&task), 4, agents),
            Some(Outgoing::Dismiss("task:task1".into()))
        );
        task.status = TaskStatus::Done;
        task.result = Some("写好了，见 reports/cpi-10.md。".into());
        let a = alert(r.observe(&changed(&task), 5, agents));
        assert_eq!((a.kind, a.level), (AlertKind::TaskDone, AlertLevel::All));
        assert_eq!(
            a.thread_id.as_ref().map(ThreadId::as_str),
            Some("t1"),
            "点开去派活的对话"
        );

        // 员工派给员工的不通知
        task.id = "task2".into();
        task.created_by = Author::Agent {
            agent_id: "macro-1".into(),
        };
        assert!(r.observe(&changed(&task), 6, agents).is_none());
    }

    #[test]
    fn settings_filter_by_level_role_mute_and_preview() {
        let a = Alert {
            kind: AlertKind::Reply,
            level: AlertLevel::All,
            title: "宏观研究员 回复了".into(),
            body: "10 月 CPI 同比 2.9%。".into(),
            tag: "thread:t".into(),
            thread_id: Some("t".into()),
            role_id: Some(RoleId::from("macro")),
            at_ms: 0,
        };
        let needs_me = Alert {
            level: AlertLevel::NeedsMe,
            ..a.clone()
        };
        let default = NotifySettings::default();
        assert!(default.filter(&a, 0).is_none(), "默认只推需要我的");
        assert!(default.filter(&needs_me, 0).is_some());
        let all = NotifySettings {
            level: NotifyLevel::All,
            ..NotifySettings::default()
        };
        assert!(all.filter(&a, 0).is_some());
        let off = NotifySettings {
            level: NotifyLevel::Off,
            ..NotifySettings::default()
        };
        assert!(off.filter(&needs_me, 0).is_none());
        let muted = NotifySettings {
            muted_roles: vec![RoleId::from("macro")],
            ..NotifySettings::default()
        };
        assert!(muted.filter(&needs_me, 0).is_none());
        let quiet = NotifySettings {
            mute_until_ms: Some(100),
            ..NotifySettings::default()
        };
        assert!(quiet.filter(&needs_me, 99).is_none());
        assert!(quiet.filter(&needs_me, 100).is_some());
        let private = NotifySettings {
            preview: false,
            ..NotifySettings::default()
        };
        assert_eq!(private.filter(&needs_me, 0).unwrap().body, "");
    }

    #[test]
    fn previews_drop_code_tables_and_markup() {
        let text = "# 10 月 CPI\n\n| 指标 | 值 |\n| --- | --- |\n| CPI | 2.9% |\n\n```python\nprint(1)\n```\n\n- **同比 2.9%**，见 [报告](reports/cpi.md)。`核心` 回落。第三句不要。";
        assert_eq!(preview(text), "10 月 CPI 同比 2.9%，见 报告。核心 回落。");
        let long = "很长".repeat(200);
        let p = preview(&long);
        assert_eq!(p.chars().count(), PREVIEW_CHARS);
        assert!(p.ends_with('…'));
        assert_eq!(preview("Done. Next step is here. Third."), "Done. Next step is here.");
        assert_eq!(preview("版本 v1.2 发布了"), "版本 v1.2 发布了");
    }

    #[test]
    fn previews_never_show_secrets() {
        // 下划线是密钥的一部分：先脱敏再去掉 Markdown 记号
        assert_eq!(
            preview("新的令牌 ghp_abcdefghijklmnopqrstuvwxyz0123 已生效。"),
            "新的令牌 [已脱敏] 已生效。"
        );
        // 值后面紧跟的中文标点也算进值里一起抹掉（`redact` 的规则）
        assert_eq!(
            preview("配好了 MACRO_API_KEY=abcdef123456。"),
            "配好了 MACRO_API_KEY=[已脱敏]"
        );
        // 整段私钥在开头一段里：换掉
        let pem = format!(
            "私钥如下\n-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----",
            "MIIEow".repeat(10)
        );
        assert_eq!(preview(&pem), "私钥如下 [已脱敏]");
        // 私钥比开头一段还长：从开头起都不要
        let long = format!(
            "私钥如下\n-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----",
            "MIIEowIBAAKCAQEA\n".repeat(300)
        );
        assert_eq!(preview(&long), "私钥如下");
        // 一行很长的回复也只看开头
        let huge = format!("结论在前。{}", "数据".repeat(100_000));
        assert!(preview(&huge).starts_with("结论在前。"));
    }

    #[tokio::test]
    async fn settings_are_saved_per_device() {
        let dir = tempfile::tempdir().unwrap();
        let store = ip_store::testing::store_at(dir.path().join("ip.db")).await;
        let n = Notifier::new(store.clone());
        assert_eq!(n.settings("dev_1"), NotifySettings::default());
        let s = NotifySettings {
            level: NotifyLevel::All,
            muted_roles: vec![RoleId::from("ops"), RoleId::from("macro"), RoleId::from("ops")],
            mute_until_ms: None,
            preview: false,
        };
        let saved = n.save("dev_1", s).await.unwrap();
        assert_eq!(saved.muted_roles, [RoleId::from("macro"), RoleId::from("ops")]);
        assert_eq!(n.settings(SHARED), NotifySettings::default());
        let again = Notifier::new(store);
        again.load().await.unwrap();
        assert_eq!(again.settings("dev_1"), saved);
        again.forget("dev_1").await.unwrap();
        assert_eq!(again.settings("dev_1"), NotifySettings::default());
    }
}
