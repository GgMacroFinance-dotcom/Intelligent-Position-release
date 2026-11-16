//! 任务与审批卡片：人通过服务接口、员工通过工具，走的是同一套校验和事件。

use std::collections::{BTreeMap, HashMap};

use ip_core::{
    Agent, AgentId, Approval, Author, Message, Notice, Role, RoleId, Task, TaskId, TaskStatus, ThreadId, now_ms,
};

use crate::{RuntimeError, RuntimeEvent, Shared};

/// 运行时知道的岗位和员工，只用来校验指派。不含收件箱，员工任务可以放心持有。
#[derive(Default)]
pub(crate) struct Directory {
    pub roles: HashMap<RoleId, Role>,
    pub agents: BTreeMap<AgentId, Agent>,
}

/// 新建任务的参数。
#[derive(Clone, Debug, Default)]
pub struct NewTask {
    pub title: String,
    pub description: String,
    pub role: Option<RoleId>,
    pub assignee: Option<AgentId>,
    /// 负责人做事的对话；不给时有负责人就新开一个。
    pub thread_id: Option<ThreadId>,
    /// 派活的对话，任务结束时结果写回这里。
    pub origin_thread_id: Option<ThreadId>,
    pub due_ms: Option<i64>,
}

/// 修改任务的参数；都不给时只刷新更新时间。
#[derive(Clone, Debug, Default)]
pub struct TaskUpdate {
    pub status: Option<TaskStatus>,
    pub assignee: Option<AgentId>,
    /// 交回的结果或受阻原因。
    pub result: Option<String>,
}

impl Shared {
    /// 同事名单，写进系统提示词，让员工知道任务可以派给谁。按 id 排序，名单不变时提示词前缀不变。
    pub(crate) fn roster(&self, me: &AgentId) -> String {
        let dir = self.directory();
        let mut s = String::new();
        for a in dir.agents.values().filter(|a| &a.id != me) {
            let role = dir.roles.get(&a.role).map_or(a.role.as_str(), |r| r.display_name());
            s.push_str(&format!("- {}（id: {}，岗位：{}）\n", a.name, a.id, role));
        }
        s
    }

    fn directory(&self) -> std::sync::RwLockReadGuard<'_, Directory> {
        self.directory.read().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) async fn create_task(&self, new: NewTask, created_by: Author) -> Result<Task, RuntimeError> {
        let title = new.title.trim();
        if title.is_empty() {
            return Err(RuntimeError::Invalid("任务标题不能为空".into()));
        }
        let (role, assignee) = self.check_assignment(new.role, new.assignee)?;
        if let (Author::Agent { agent_id }, Some(a)) = (&created_by, &assignee)
            && agent_id == a
        {
            return Err(RuntimeError::Invalid("不能把任务派给自己，直接做就行".into()));
        }
        let depth = match &new.origin_thread_id {
            Some(origin) => self
                .store
                .task_by_thread(origin)
                .await?
                .map_or(1, |parent| parent.depth + 1),
            None => 1,
        };
        let max = self.config.max_delegation_depth.max(1);
        if depth > max {
            return Err(RuntimeError::Invalid(format!(
                "委派已经到第 {max} 层，不能再往下派；请自己完成，或把任务设为 blocked 说明原因"
            )));
        }
        let thread_id = new.thread_id.or_else(|| assignee.as_ref().map(|_| ThreadId::new()));
        let now = now_ms();
        let task = Task {
            id: TaskId::new(),
            title: title.to_string(),
            description: new.description,
            status: TaskStatus::Open,
            role,
            assignee,
            thread_id,
            origin_thread_id: new.origin_thread_id,
            result: None,
            depth,
            created_by,
            created_at_ms: now,
            updated_at_ms: now,
            due_ms: new.due_ms,
        };
        let task = self.save_task(task).await?;
        self.hand_over(&task).await?;
        Ok(task)
    }

    /// `by` 是改任务的人或员工，用来避免把消息回报给改的人自己。
    pub(crate) async fn update_task(&self, id: &TaskId, update: TaskUpdate, by: &Author) -> Result<Task, RuntimeError> {
        let mut task = self
            .store
            .task(id)
            .await?
            .ok_or_else(|| RuntimeError::Invalid(format!("找不到任务 {id}")))?;
        let was_closed = finished(task.status);
        let reassigned = update.assignee.is_some() && update.assignee != task.assignee;
        if let Some(s) = update.status {
            task.status = s;
        }
        if let Some(r) = update.result.filter(|r| !r.trim().is_empty()) {
            task.result = Some(r.trim().to_string());
        }
        if reassigned {
            // 换负责人时岗位跟着负责人走。
            let (role, assignee) = self.check_assignment(None, update.assignee)?;
            task.role = role;
            task.assignee = assignee;
            if task.thread_id.is_none() {
                task.thread_id = Some(ThreadId::new());
            }
        }
        task.updated_at_ms = now_ms();
        let task = self.save_task(task).await?;
        if reassigned {
            self.hand_over(&task).await?;
        }
        if !was_closed && finished(task.status) {
            self.report_back(&task, by, true).await?;
        }
        Ok(task)
    }

    /// 把任务说明写进负责人的对话，并通知负责人开工。
    async fn hand_over(&self, task: &Task) -> Result<(), RuntimeError> {
        let (Some(assignee), Some(thread)) = (&task.assignee, &task.thread_id) else {
            return Ok(());
        };
        let mut text = format!(
            "你被分派了任务「{}」（id: {}），派活人：{}。",
            task.title,
            task.id,
            self.author_name(&task.created_by)
        );
        if !task.description.trim().is_empty() {
            text.push_str(&format!("\n说明：{}", task.description.trim()));
        }
        text.push_str(
            "\n开始时用 update_task 设为 in_progress；做完设为 done，并把结论写进 result；\
             做不下去设为 blocked，在 result 里写明缺什么。",
        );
        self.post(Message::notice(thread.clone(), Notice::TaskBrief, text))
            .await?;
        self.dispatch(assignee, thread.clone(), false).await
    }

    /// 负责人这一轮失败了（费用用满、模型出错、步数超限）：任务标为受阻，原因写回派活的对话。
    /// 不叫回派活的员工：重新派活多半还会失败，自动重试会一圈圈地花钱，交给人处理。
    pub(crate) async fn fail_task(&self, mut task: Task, reason: String) -> Result<(), RuntimeError> {
        if finished(task.status) {
            return Ok(());
        }
        task.status = TaskStatus::Blocked;
        task.result = Some(reason);
        task.updated_at_ms = now_ms();
        let task = self.save_task(task).await?;
        self.report_back(&task, &Author::System, false).await
    }

    /// 任务结束（完成、受阻、取消）时把结果写回派活的对话；派活的是员工、且 `wake` 时叫他回来接着做。
    async fn report_back(&self, task: &Task, by: &Author, wake: bool) -> Result<(), RuntimeError> {
        let Some(origin) = &task.origin_thread_id else {
            return Ok(());
        };
        let who = task
            .assignee
            .as_ref()
            .map_or_else(|| "负责人".to_string(), |a| self.agent_name(a));
        let verb = match task.status {
            TaskStatus::Done => "已完成",
            TaskStatus::Blocked => "受阻",
            _ => "已取消",
        };
        let mut text = format!("任务「{}」（id: {}）{verb}，负责人：{who}。", task.title, task.id);
        if let Some(r) = &task.result {
            text.push_str(&format!("\n结果：{r}"));
        }
        // 同一个对话里同一人派出的其他任务还在做时，先只记下结果，等最后一个有结果再叫回派活人，
        // 免得协调员每收到一个结果就调一次模型。受阻要人处理，立即叫回。
        let pending = self
            .store
            .tasks_from_thread(origin)
            .await?
            .into_iter()
            .filter(|t| t.id != task.id && t.created_by == task.created_by && !finished(t.status))
            .count();
        let wait = pending > 0 && task.status != TaskStatus::Blocked;
        if wait {
            text.push_str(&format!("\n还有 {pending} 个任务在进行，全部有结果后再一起处理。"));
        }
        if !wake {
            text.push_str("\n负责人这轮工作中断，需要人处理：解决原因后在任务页改派或重新派活。");
        }
        self.post(Message::notice(origin.clone(), Notice::TaskReport, text))
            .await?;
        if let Author::Agent { agent_id } = &task.created_by
            && by != &task.created_by
            && wake
            && !wait
        {
            self.dispatch(agent_id, origin.clone(), false).await?;
        }
        Ok(())
    }

    fn agent_name(&self, id: &AgentId) -> String {
        self.directory()
            .agents
            .get(id)
            .map_or_else(|| id.to_string(), |a| a.name.clone())
    }

    fn author_name(&self, a: &Author) -> String {
        match a {
            Author::Human { name, .. } => name.clone(),
            Author::Agent { agent_id } => self.agent_name(agent_id),
            Author::System => "系统".into(),
        }
    }

    /// 存一张新的审批卡片并广播。
    pub(crate) async fn open_approval(&self, approval: Approval) -> Result<Approval, RuntimeError> {
        if approval.options.is_empty() {
            return Err(RuntimeError::Invalid("审批卡片至少要有一个选项".into()));
        }
        if approval
            .recommended
            .is_some_and(|r| r as usize >= approval.options.len())
        {
            return Err(RuntimeError::Invalid("推荐项超出了选项范围".into()));
        }
        self.store.put_approval(approval.clone()).await?;
        self.emit(RuntimeEvent::ApprovalChanged {
            approval: approval.clone(),
        });
        Ok(approval)
    }

    /// 岗位和负责人都必须存在；只给了负责人时，岗位取他的岗位。
    fn check_assignment(
        &self,
        role: Option<RoleId>,
        assignee: Option<AgentId>,
    ) -> Result<(Option<RoleId>, Option<AgentId>), RuntimeError> {
        let dir = self.directory();
        if let Some(r) = &role
            && !dir.roles.contains_key(r)
        {
            return Err(RuntimeError::Invalid(format!("岗位 {r} 不存在")));
        }
        let Some(a) = assignee else { return Ok((role, None)) };
        let agent = dir
            .agents
            .get(&a)
            .ok_or_else(|| RuntimeError::Invalid(format!("员工 {a} 不存在")))?;
        if let Some(r) = &role
            && *r != agent.role
        {
            return Err(RuntimeError::Invalid(format!("员工 {a} 不属于岗位 {r}")));
        }
        Ok((Some(agent.role.clone()), Some(a)))
    }

    async fn save_task(&self, task: Task) -> Result<Task, RuntimeError> {
        self.store.put_task(task.clone()).await?;
        self.emit(RuntimeEvent::TaskChanged { task: task.clone() });
        Ok(task)
    }
}

/// 已经有结论的状态：完成、受阻、取消。受阻也要回报，派活人才能决定怎么办。
fn finished(s: TaskStatus) -> bool {
    matches!(s, TaskStatus::Done | TaskStatus::Blocked | TaskStatus::Cancelled)
}
