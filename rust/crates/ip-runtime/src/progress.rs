//! 员工的任务进度和“要人看”的时间点（`docs/ip/progress-design.md`，backlog F-24，`docs/ip/m2-spec.md` D-42）。
//!
//! 第一步只按历史推算：主机记每一轮的步数和用时（`turns` 表），按同岗位做完的任务估还剩几步，
//! 再按每步用时换算成时间。不改提示词，不为估算多调模型；样本不够就留空，不编数。
//!
//! [`estimate`] 是纯函数，单元测试和 `ip bench progress` 直接喂数据；
//! [`crate::Runtime::progress`] 从存储和内存里取齐数据后调它。

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use ip_core::{Agent, AgentId, Approval, ApprovalSubject, Author, Role, RoleId, Task, TaskId, TaskStatus, ThreadId};
use ip_store::{TurnRecord, TurnTotals};
use serde::Serialize;
use ts_rs::TS;

/// 做完的任务少于这么多个时不估任务进度。
pub const MIN_TASK_SAMPLES: usize = 3;
/// 记下的轮次少于这么多个时不估一轮的进度和每步用时。
pub const MIN_TURN_SAMPLES: usize = 5;
/// 基线看最近多少天做完的任务。
pub const HISTORY_DAYS: i64 = 30;
/// 基线最多看多少个做完的任务、多少轮。
pub const HISTORY_TASKS: usize = 50;
pub const HISTORY_TURNS: usize = 200;
/// 基线算好以后复用多久。
pub const BASELINE_TTL_MS: i64 = 60_000;
/// 没做完时进度最多显示到这里。
pub const MAX_FRACTION: f64 = 0.95;
/// 预测的时间点只列这么远以内的。
pub const HORIZON_MS: i64 = 24 * 3_600_000;
/// 可能性低于这个的预测不列。
pub const MIN_LIKELIHOOD: f64 = 0.2;
/// 任务停着这么久以后才列成“要人看”：一轮刚结束、马上又被叫回来的不算。
pub const STALL_GRACE_MS: i64 = 120_000;
/// 预算按最近这么久的花费速度外推。
pub const SPEND_WINDOW_MS: i64 = 3_600_000;

// ---------------------------------------------------------------------------
// 内存里的实时状态

/// 员工正在跑的一轮。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Running {
    pub thread_id: ThreadId,
    pub started_ms: i64,
    /// 内置循环走到第几步（从 1 起）；外部引擎看不到步数，一直是 0。
    pub step: u32,
}

/// 某一刻的实时状态：谁在跑哪一轮、谁的收件箱里排着哪些对话（先来的在前）。
#[derive(Clone, Debug, Default)]
pub struct LiveSnapshot {
    pub running: HashMap<AgentId, Running>,
    pub queued: HashMap<AgentId, Vec<ThreadId>>,
}

/// 员工的实时状态。只在内存里，重启后从空的开始（收件箱也是空的）。
#[derive(Default)]
pub(crate) struct Live {
    inner: Mutex<LiveInner>,
}

#[derive(Default)]
struct LiveInner {
    running: HashMap<AgentId, Running>,
    queued: HashMap<AgentId, VecDeque<ThreadId>>,
}

impl Live {
    fn lock(&self) -> std::sync::MutexGuard<'_, LiveInner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// 送进收件箱之前调。同一个对话排着时不重复记：员工会把它们合并成一轮。
    pub fn queue(&self, agent: &AgentId, thread: &ThreadId) {
        let mut inner = self.lock();
        let queue = inner.queued.entry(agent.clone()).or_default();
        if !queue.contains(thread) {
            queue.push_back(thread.clone());
        }
    }

    /// 没送进去（员工已停止）：撤掉刚记的。
    pub fn unqueue(&self, agent: &AgentId, thread: &ThreadId) {
        let mut inner = self.lock();
        if let Some(queue) = inner.queued.get_mut(agent) {
            queue.retain(|t| t != thread);
        }
    }

    pub fn start(&self, agent: &AgentId, thread: &ThreadId, now_ms: i64) {
        let mut inner = self.lock();
        if let Some(queue) = inner.queued.get_mut(agent) {
            queue.retain(|t| t != thread);
        }
        inner.running.insert(
            agent.clone(),
            Running {
                thread_id: thread.clone(),
                started_ms: now_ms,
                step: 0,
            },
        );
    }

    pub fn step(&self, agent: &AgentId, step: u32) {
        if let Some(running) = self.lock().running.get_mut(agent) {
            running.step = step;
        }
    }

    /// 一轮结束，返回它最后的样子（失败的轮次从这里取走了几步）。
    pub fn finish(&self, agent: &AgentId) -> Option<Running> {
        self.lock().running.remove(agent)
    }

    pub fn snapshot(&self) -> LiveSnapshot {
        let inner = self.lock();
        LiveSnapshot {
            running: inner.running.clone(),
            queued: inner
                .queued
                .iter()
                .filter(|(_, q)| !q.is_empty())
                .map(|(a, q)| (a.clone(), q.iter().cloned().collect()))
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// 岗位基线

/// 一个做完的任务：总共走了几步，第一次问人时已经走了几步（没问过人为空）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskSample {
    pub steps: f64,
    pub asked_at: Option<f64>,
}

/// 从一个做完的任务的执行对话算出样本：只算任务建好以后、做完以前开始的轮次。没有轮次时为空（比如人直接改成做完的）。
/// `turns` 是这个对话的轮次，`questions` 是这个对话里员工提问卡片发出的时刻，都按时间排。
pub fn task_sample(created_ms: i64, done_ms: i64, turns: &[TurnRecord], questions: &[i64]) -> Option<TaskSample> {
    let inside: Vec<&TurnRecord> = turns
        .iter()
        .filter(|t| t.started_ms >= created_ms && t.started_ms <= done_ms)
        .collect();
    if inside.is_empty() {
        return None;
    }
    let steps: u32 = inside.iter().map(|t| t.steps).sum();
    // 问人的那一轮整轮算进去：卡片是这一轮里某一步发的，记成这一轮的最后一步，偏晚一点。
    let asked_at = questions.iter().find(|&&q| q >= created_ms && q <= done_ms).map(|&q| {
        inside
            .iter()
            .filter(|t| t.started_ms <= q)
            .map(|t| f64::from(t.steps))
            .sum()
    });
    Some(TaskSample {
        steps: f64::from(steps.max(1)),
        asked_at,
    })
}

/// 一个岗位的历史：做完的任务和最近的轮次。
#[derive(Clone, Debug, Default)]
pub struct Baseline {
    /// 按总步数从小到大。
    pub tasks: Vec<TaskSample>,
    /// 各轮的步数，从小到大。
    pub turn_steps: Vec<f64>,
    /// 各轮的用时（毫秒），从小到大。
    pub turn_ms: Vec<f64>,
    /// 每步用时（毫秒）：各轮“用时 ÷ 步数”的中位数；轮次不够时为空。
    pub ms_per_step: Option<f64>,
    /// `tasks` 的总步数，从小到大（和 `tasks` 同序）。
    task_steps: Vec<f64>,
}

impl Baseline {
    /// `turns` 是这个岗位最近的轮次（成功和失败的都要：走满步数上限的轮次是失败的）。
    pub fn new(mut tasks: Vec<TaskSample>, turns: &[TurnRecord]) -> Self {
        tasks.sort_by(|a, b| a.steps.total_cmp(&b.steps));
        let task_steps = tasks.iter().map(|t| t.steps).collect();
        let mut turn_steps: Vec<f64> = turns.iter().map(|t| f64::from(t.steps)).collect();
        turn_steps.sort_by(f64::total_cmp);
        // 用时和每步用时只看成功的轮次：失败的轮次常常是第一步就报错，用时不代表干活的速度。
        let ok: Vec<&TurnRecord> = turns.iter().filter(|t| t.ok && t.steps > 0).collect();
        let mut turn_ms: Vec<f64> = ok.iter().map(|t| t.active_ms() as f64).collect();
        turn_ms.sort_by(f64::total_cmp);
        let mut per_step: Vec<f64> = ok.iter().map(|t| t.active_ms() as f64 / f64::from(t.steps)).collect();
        per_step.sort_by(f64::total_cmp);
        let ms_per_step = (per_step.len() >= MIN_TURN_SAMPLES).then(|| quantile(&per_step, 0.5).max(1.0));
        Self {
            tasks,
            turn_steps,
            turn_ms,
            ms_per_step,
            task_steps,
        }
    }

    pub fn has_tasks(&self) -> bool {
        self.tasks.len() >= MIN_TASK_SAMPLES
    }

    pub fn has_turns(&self) -> bool {
        self.turn_steps.len() >= MIN_TURN_SAMPLES
    }

    /// 一轮的用时（中位数、80 分位）；成功的轮次不够时为空。
    pub fn turn_duration(&self) -> Option<(f64, f64)> {
        (self.turn_ms.len() >= MIN_TURN_SAMPLES).then(|| (quantile(&self.turn_ms, 0.5), quantile(&self.turn_ms, 0.8)))
    }
}

/// 已经走到 `x` 的情况下还剩多少。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Remaining {
    pub p50: f64,
    pub p80: f64,
    /// 历史上没有比 `x` 更长的：按“再走一点”估，依据里写明。
    pub longer_than_all: bool,
}

/// 只看历史上超过 `x` 的样本，用“样本 − x”的中位数和 80 分位。没有超过的时候，中位数按 `unit`、80 分位按 `x` 的四分之一（至少 `unit`）。
/// `sorted` 从小到大。
pub fn remaining(sorted: &[f64], x: f64, unit: f64) -> Remaining {
    let longer = &sorted[sorted.partition_point(|v| *v <= x)..];
    if longer.is_empty() {
        return Remaining {
            p50: unit,
            p80: (x * 0.25).max(unit),
            longer_than_all: true,
        };
    }
    Remaining {
        p50: quantile(longer, 0.5) - x,
        p80: quantile(longer, 0.8) - x,
        longer_than_all: false,
    }
}

/// 线性插值的分位数；`sorted` 从小到大、不能为空。
fn quantile(sorted: &[f64], q: f64) -> f64 {
    let last = sorted.len() - 1;
    let pos = q.clamp(0.0, 1.0) * last as f64;
    let lower = pos.floor() as usize;
    let upper = pos.ceil() as usize;
    sorted[lower] + (sorted[upper] - sorted[lower]) * (pos - lower as f64)
}

// ---------------------------------------------------------------------------
// 协议里的类型

/// `list_progress` 的回复。
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct ProgressBoard {
    /// 主机算这张表的时刻。App 拿它和自己的时钟对齐，两次拉取之间按预计做完的时刻把进度条往前推。
    #[ts(type = "number")]
    pub now_ms: i64,
    /// 每位员工一项，按员工 id 排。
    pub agents: Vec<AgentProgress>,
    /// 要人看的时间点，按时刻排，同一时刻可能性大的在前。
    pub attention: Vec<AttentionPoint>,
}

/// 一位员工在干什么、手上的事做到哪了。
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AgentProgress {
    pub agent_id: AgentId,
    pub phase: ProgressPhase,
    /// 头顶进度条显示的那一项：正在做的任务或这一轮；没在做时取排在最前的，或卡住的任务。没事时为空。
    pub headline: Option<Estimate>,
    /// 手上每个没结束的任务。
    pub tasks: Vec<Estimate>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ProgressPhase {
    Idle,
    /// 收件箱里有事，还没轮到。
    Queued,
    Working,
    /// 在等人回答提问。
    WaitingHuman,
    /// 派出去的子任务还没结束。
    WaitingColleague,
    Blocked,
    /// 没结束、有负责人，但不在做、不在排队、也不在等人或等同事。
    Stalled,
    /// 最近一轮失败了，之后没再成功过。
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum EstimateScope {
    /// 一个任务（可能跨好几轮）。
    Task,
    /// 没有挂任务的对话里的这一轮。
    Turn,
}

/// 一项估算。估不出的字段为空，`basis` 写为什么。
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Estimate {
    pub scope: EstimateScope,
    pub thread_id: ThreadId,
    pub task_id: Option<TaskId>,
    /// 任务标题；一轮的是“这一轮”。
    pub title: String,
    pub phase: ProgressPhase,
    /// 任务建好的时刻，或这一轮开始的时刻（还在排队的是现在）。
    #[ts(type = "number")]
    pub started_ms: i64,
    /// 已经走了几步。
    pub steps_done: u32,
    /// 按历史估的总步数（中位数）。
    pub steps_expected: Option<u32>,
    /// 0 到 1，没做完时最多 0.95；估不出时为空，界面画不定长的进度条。
    pub fraction: Option<f64>,
    /// 预计做完的时刻（中位数）。在等人、受阻、停着的不估。
    #[ts(type = "number | null")]
    pub eta_ms: Option<i64>,
    /// 最晚可能做完的时刻（80 分位）。
    #[ts(type = "number | null")]
    pub eta_late_ms: Option<i64>,
    /// 用了几个历史样本；0 表示没估。
    pub samples: u32,
    /// 估算依据，给人看的一句话。
    pub basis: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum AttentionKind {
    /// 员工在问你，或别的等你决定的卡片。
    Question,
    Blocked,
    Failed,
    Stalled,
    /// 任务没有负责人，或消息等你决定交给谁。
    Unassigned,
    Overdue,
    /// 截止前可能做不完。
    DueRisk,
    /// 快到截止、又估不出做完的时间。
    Due,
    /// 按历史，这类任务走到某一步时常会问人。
    LikelyQuestion,
    /// 人派的任务预计交回结果。
    Result,
    /// 员工在和人的对话里派出去的任务都回来后，汇总回复。
    Reply,
    /// 这一轮可能走满步数上限而中断。
    StepLimit,
    /// 按现在的花费速度，今天的预算会用完。
    Budget,
}

/// 一个要人看的时间点。
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AttentionPoint {
    pub kind: AttentionKind,
    pub agent_id: Option<AgentId>,
    pub role_id: Option<RoleId>,
    pub task_id: Option<TaskId>,
    /// 点了去哪个对话。
    pub thread_id: Option<ThreadId>,
    /// 预计什么时候要人看；已经发生的是发生的时刻。
    #[ts(type = "number")]
    pub at_ms: i64,
    /// 最晚什么时候（80 分位）；确定的时间点为空。
    #[ts(type = "number | null")]
    pub late_ms: Option<i64>,
    /// 有多大可能真的要人看，0 到 1；已经发生的是 1，按速度外推的为空。
    pub likelihood: Option<f64>,
    /// 给人看的一句话，不含时刻（时刻由界面按本地时区显示）。
    pub text: String,
}

// ---------------------------------------------------------------------------
// 估算

/// 估算要的全部数据。
pub struct Inputs<'a> {
    pub now_ms: i64,
    /// 一轮最多走几步。
    pub max_steps: u32,
    /// 今天（UTC）结束的时刻；预算到这时清零。
    pub day_end_ms: i64,
    pub agents: &'a [Agent],
    pub roles: &'a HashMap<RoleId, Role>,
    /// 没结束的任务。
    pub tasks: &'a [Task],
    /// 待决定的卡片。
    pub approvals: &'a [Approval],
    /// 没结束的任务里，问过人的执行对话（不论问题答没答）。
    pub asked: &'a HashSet<ThreadId>,
    /// 没结束的任务的执行对话里，已经结束的各轮加起来。
    pub totals: &'a HashMap<ThreadId, TurnTotals>,
    pub live: &'a LiveSnapshot,
    /// 每位员工最近结束的一轮。
    pub last_turns: &'a HashMap<AgentId, TurnRecord>,
    pub baselines: &'a HashMap<RoleId, Arc<Baseline>>,
    /// 各岗位今天已花的美元。
    pub spent_today: &'a HashMap<RoleId, f64>,
    /// 各岗位最近 [`SPEND_WINDOW_MS`] 里结束的轮次花的美元。
    pub spent_recent: &'a HashMap<RoleId, f64>,
}

/// 一个任务算到的结果，委派的父任务要用子任务的。
#[derive(Clone)]
struct TaskEst {
    estimate: Estimate,
    /// 还剩几步的历史样本，算“赶不上截止”的可能性用：开始的时刻、每步用时、剩下的步数。
    residual: Option<Residual>,
}

#[derive(Clone)]
struct Residual {
    start_ms: f64,
    ms_per_step: f64,
    /// 历史上比已走步数长的任务，各自还剩几步。
    left: Vec<f64>,
}

pub fn estimate(input: &Inputs<'_>) -> ProgressBoard {
    let now = input.now_ms;
    let empty = Arc::new(Baseline::default());
    let names: HashMap<&AgentId, &str> = input.agents.iter().map(|a| (&a.id, a.name.as_str())).collect();
    let role_of: HashMap<&AgentId, &RoleId> = input.agents.iter().map(|a| (&a.id, &a.role)).collect();
    let baseline = |role: Option<&RoleId>| -> Arc<Baseline> {
        role.and_then(|r| input.baselines.get(r))
            .cloned()
            .unwrap_or_else(|| empty.clone())
    };
    let role_name = |role: Option<&RoleId>| -> String {
        role.map(|r| input.roles.get(r).map_or(r.as_str(), |x| x.display_name()).to_string())
            .unwrap_or_else(|| "这个岗位".to_string())
    };
    let name_of = |a: &AgentId| names.get(a).copied().unwrap_or(a.as_str()).to_string();

    let task_by_thread: HashMap<&ThreadId, &Task> = input
        .tasks
        .iter()
        .filter_map(|t| t.thread_id.as_ref().map(|th| (th, t)))
        .collect();
    let mut children: HashMap<&ThreadId, Vec<&Task>> = HashMap::new();
    let mut tasks_of: HashMap<&AgentId, Vec<&Task>> = HashMap::new();
    for t in input.tasks {
        if let Some(origin) = &t.origin_thread_id {
            children.entry(origin).or_default().push(t);
        }
        if let Some(a) = &t.assignee {
            tasks_of.entry(a).or_default().push(t);
        }
    }
    let mut questions_on: HashSet<&ThreadId> = HashSet::new();
    let mut asking_agents: HashSet<&AgentId> = HashSet::new();
    for a in input.approvals {
        if let ApprovalSubject::AgentQuestion { agent_id, thread_id } = &a.subject {
            questions_on.insert(thread_id);
            asking_agents.insert(agent_id);
        }
    }

    let mut attention: Vec<AttentionPoint> = Vec::new();
    let mut by_task: HashMap<TaskId, TaskEst> = HashMap::new();
    // 一轮的估算（没挂任务的对话），按员工记：正在跑的和排着的。
    let mut turn_heads: HashMap<AgentId, Estimate> = HashMap::new();

    // 1. 每位员工按“正在跑的、收件箱里排着的”顺序往后排时间。
    for agent in input.agents {
        let role = &agent.role;
        let base = baseline(Some(role));
        let rname = role_name(Some(role));
        let running = input.live.running.get(&agent.id);
        let mut order: Vec<(&ThreadId, bool)> = Vec::new();
        if let Some(r) = running {
            order.push((&r.thread_id, true));
        }
        if let Some(queue) = input.live.queued.get(&agent.id) {
            for t in queue {
                if running.is_none_or(|r| &r.thread_id != t) {
                    order.push((t, false));
                }
            }
        }
        // 时间轴：中位数和 80 分位各推一条。估不出每步用时就到此为止，后面的都不估时间。
        let mut offset: Option<(f64, f64)> = Some((now as f64, now as f64));
        for (thread, is_running) in order {
            let task = task_by_thread
                .get(thread)
                .copied()
                .filter(|t| t.assignee.as_ref() == Some(&agent.id));
            match task {
                Some(task) => {
                    let live_steps = running.filter(|_| is_running).map(|r| live_steps(r, now, &base));
                    let est = task_estimate(
                        task,
                        if is_running {
                            ProgressPhase::Working
                        } else {
                            ProgressPhase::Queued
                        },
                        input.totals.get(thread).copied().unwrap_or_default(),
                        live_steps.unwrap_or(0),
                        &base,
                        &rname,
                        offset,
                    );
                    offset = match (offset, est.estimate.eta_ms, est.estimate.eta_late_ms) {
                        (Some(_), Some(e), Some(l)) => Some((e as f64, l as f64)),
                        _ => None,
                    };
                    by_task.insert(task.id.clone(), est);
                }
                None => {
                    let est = turn_estimate(thread, running.filter(|_| is_running), now, &base, &rname, offset);
                    offset = match (offset, est.eta_ms, est.eta_late_ms) {
                        (Some(_), Some(e), Some(l)) => Some((e as f64, l as f64)),
                        _ => None,
                    };
                    if is_running {
                        step_limit(input, agent, running, &base, &mut attention);
                    }
                    turn_heads.entry(agent.id.clone()).or_insert(est);
                }
            }
        }
    }

    // 2. 不在时间轴上的任务：受阻、在等人、等同事、停着。等同事的要先有子任务的结果，按层级从深到浅算。
    let mut rest: Vec<&Task> = input
        .tasks
        .iter()
        .filter(|t| t.assignee.is_some() && !by_task.contains_key(&t.id))
        .collect();
    rest.sort_by(|a, b| b.depth.cmp(&a.depth).then_with(|| a.id.as_str().cmp(b.id.as_str())));
    for task in rest {
        let role = task
            .role
            .as_ref()
            .or_else(|| task.assignee.as_ref().and_then(|a| role_of.get(a).copied()));
        let base = baseline(role);
        let rname = role_name(role);
        let thread = task.thread_id.as_ref();
        let open_children: Vec<&Task> = thread
            .and_then(|th| children.get(th))
            .map(|c| c.iter().copied().filter(|c| !c.status.is_closed()).collect())
            .unwrap_or_default();
        let phase = if task.status == TaskStatus::Blocked {
            ProgressPhase::Blocked
        } else if thread.is_some_and(|th| questions_on.contains(th)) {
            ProgressPhase::WaitingHuman
        } else if !open_children.is_empty() {
            ProgressPhase::WaitingColleague
        } else {
            ProgressPhase::Stalled
        };
        let totals = thread.and_then(|th| input.totals.get(th)).copied().unwrap_or_default();
        let mut est = task_estimate(task, phase, totals, 0, &base, &rname, None);
        if phase == ProgressPhase::WaitingColleague {
            // 子任务里最晚的做完，再加汇总的一轮。子任务有一个估不出，就不估。
            let mut latest: Option<(i64, i64)> = Some((now, now));
            for c in &open_children {
                latest = match (latest, by_task.get(&c.id)) {
                    (Some((e, l)), Some(ce)) => match (ce.estimate.eta_ms, ce.estimate.eta_late_ms) {
                        (Some(ce_e), Some(ce_l)) => Some((e.max(ce_e), l.max(ce_l))),
                        _ => None,
                    },
                    _ => None,
                };
            }
            if let (Some((e, l)), Some((t50, t80))) = (latest, base.turn_duration()) {
                est.estimate.eta_ms = Some(e + t50 as i64);
                est.estimate.eta_late_ms = Some(l + t80 as i64);
                est.estimate.basis = format!(
                    "等 {} 件子任务做完，再加汇总的一轮（{}一轮的中位用时约 {}）",
                    open_children.len(),
                    rname,
                    duration_text(t50)
                );
            } else {
                est.estimate.basis = format!("等 {} 件子任务做完；子任务有估不出的，先不估", open_children.len());
            }
        }
        by_task.insert(task.id.clone(), est);
    }

    // 3. 每位员工的状态和头顶那一项。
    let mut agents_out = Vec::with_capacity(input.agents.len());
    let mut sorted_agents: Vec<&Agent> = input.agents.iter().collect();
    sorted_agents.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    for agent in sorted_agents {
        let mut tasks: Vec<Estimate> = tasks_of
            .get(&agent.id)
            .map(|list| {
                list.iter()
                    .filter_map(|t| by_task.get(&t.id).map(|e| e.estimate.clone()))
                    .collect()
            })
            .unwrap_or_default();
        tasks.sort_by(|a, b| {
            phase_rank(a.phase)
                .cmp(&phase_rank(b.phase))
                .then(a.started_ms.cmp(&b.started_ms))
        });
        let running = input.live.running.get(&agent.id);
        let first_queued = input
            .live
            .queued
            .get(&agent.id)
            .and_then(|q| q.iter().find(|t| running.is_none_or(|r| &r.thread_id != *t)));
        let estimate_for = |thread: &ThreadId| -> Option<Estimate> {
            task_by_thread
                .get(thread)
                .filter(|t| t.assignee.as_ref() == Some(&agent.id))
                .and_then(|t| by_task.get(&t.id))
                .map(|e| e.estimate.clone())
                .or_else(|| turn_heads.get(&agent.id).filter(|e| &e.thread_id == thread).cloned())
        };
        let last_failed = input.last_turns.get(&agent.id).filter(|t| !t.ok && running.is_none());
        let (phase, headline) = if let Some(r) = running {
            (ProgressPhase::Working, estimate_for(&r.thread_id))
        } else if let Some(q) = first_queued {
            (ProgressPhase::Queued, estimate_for(q))
        } else if let Some(stuck) = tasks.iter().find(|t| t.phase != ProgressPhase::Stalled) {
            (stuck.phase, Some(stuck.clone()))
        } else if asking_agents.contains(&agent.id) {
            (ProgressPhase::WaitingHuman, None)
        } else if last_failed.is_some() {
            (ProgressPhase::Failed, None)
        } else if let Some(stalled) = tasks.first() {
            (ProgressPhase::Stalled, Some(stalled.clone()))
        } else {
            (ProgressPhase::Idle, None)
        };
        agents_out.push(AgentProgress {
            agent_id: agent.id.clone(),
            phase,
            headline,
            tasks,
        });

        // 上一轮失败：挂着任务的失败已经算成受阻（运行时把任务标成受阻），不重复。
        if let Some(t) = last_failed {
            let blocked_task = task_by_thread
                .get(&t.thread_id)
                .is_some_and(|task| task.status == TaskStatus::Blocked);
            if !blocked_task {
                attention.push(AttentionPoint {
                    kind: AttentionKind::Failed,
                    agent_id: Some(agent.id.clone()),
                    role_id: Some(agent.role.clone()),
                    task_id: None,
                    thread_id: Some(t.thread_id.clone()),
                    at_ms: t.finished_ms,
                    late_ms: None,
                    likelihood: Some(1.0),
                    text: format!(
                        "{} 的上一轮失败了{}",
                        agent.name,
                        t.error
                            .as_deref()
                            .map(|e| format!("：{}", short(e, 40)))
                            .unwrap_or_default()
                    ),
                });
            }
        }
    }

    // 4. 要人看的时间点。
    for a in input.approvals {
        let (kind, agent, thread, text) = match &a.subject {
            ApprovalSubject::AgentQuestion { agent_id, thread_id } => (
                AttentionKind::Question,
                Some(agent_id.clone()),
                Some(thread_id.clone()),
                format!("{} 在问你：{}", name_of(agent_id), short(&a.question, 40)),
            ),
            ApprovalSubject::RouteMessage { thread_id, .. } => (
                AttentionKind::Unassigned,
                None,
                Some(thread_id.clone()),
                format!("有一条消息等你决定交给谁：{}", short(&a.question, 40)),
            ),
            #[allow(unreachable_patterns)] // 以后加的卡片种类先按一般问题算
            _ => (
                AttentionKind::Question,
                None,
                None,
                format!("等你决定：{}", short(&a.question, 40)),
            ),
        };
        let task_id = thread
            .as_ref()
            .and_then(|th| task_by_thread.get(th))
            .map(|t| t.id.clone());
        attention.push(AttentionPoint {
            kind,
            role_id: agent.as_ref().and_then(|a| role_of.get(a)).map(|r| (*r).clone()),
            agent_id: agent,
            task_id,
            thread_id: thread,
            at_ms: a.created_at_ms,
            late_ms: None,
            likelihood: Some(1.0),
            text,
        });
    }

    for task in input.tasks {
        let title = short(&task.title, 24);
        let point = |kind, at_ms, late_ms, likelihood, text: String| AttentionPoint {
            kind,
            agent_id: task.assignee.clone(),
            role_id: task.role.clone(),
            task_id: Some(task.id.clone()),
            thread_id: task.thread_id.clone().or_else(|| task.origin_thread_id.clone()),
            at_ms,
            late_ms,
            likelihood,
            text,
        };
        let Some(assignee) = &task.assignee else {
            attention.push(point(
                AttentionKind::Unassigned,
                task.created_at_ms,
                None,
                Some(1.0),
                format!("任务等你分配：{title}"),
            ));
            continue;
        };
        let who = name_of(assignee);
        let Some(est) = by_task.get(&task.id) else { continue };
        let e = &est.estimate;
        match e.phase {
            ProgressPhase::Blocked => {
                let why = task
                    .result
                    .as_deref()
                    .map(|r| format!("（{}）", short(r, 40)))
                    .unwrap_or_default();
                attention.push(point(
                    AttentionKind::Blocked,
                    task.updated_at_ms,
                    None,
                    Some(1.0),
                    format!("{who} 的任务受阻：{title}{why}"),
                ));
                continue;
            }
            ProgressPhase::Stalled => {
                let last = task.updated_at_ms.max(
                    task.thread_id
                        .as_ref()
                        .and_then(|th| input.totals.get(th))
                        .map_or(0, |t| t.last_finished_ms),
                );
                if now - last >= STALL_GRACE_MS {
                    attention.push(point(
                        AttentionKind::Stalled,
                        last,
                        None,
                        Some(1.0),
                        format!("{who} 的任务停着：{title}（没在做，也没在排队）"),
                    ));
                }
            }
            _ => {}
        }
        if let Some(due) = task.due_ms {
            if due < now {
                attention.push(point(
                    AttentionKind::Overdue,
                    due,
                    None,
                    Some(1.0),
                    format!("{who} 的任务过了截止：{title}"),
                ));
            } else if let Some(eta) = e.eta_ms {
                let late = e.eta_late_ms.unwrap_or(eta);
                let risk = match &est.residual {
                    Some(r) if !r.left.is_empty() => {
                        let after = r
                            .left
                            .iter()
                            .filter(|left| r.start_ms + **left * r.ms_per_step > due as f64)
                            .count();
                        Some(after as f64 / r.left.len() as f64)
                    }
                    // 走得比以往都长，或者是按子任务推出来的：只能拿估计比一比。
                    _ => (late > due).then_some(if eta > due { 1.0 } else { 0.5 }),
                };
                if let Some(p) = risk.filter(|p| *p >= MIN_LIKELIHOOD) {
                    attention.push(point(
                        AttentionKind::DueRisk,
                        now,
                        None,
                        Some(round2(p)),
                        format!("{who} 的任务可能赶不上截止：{title}"),
                    ));
                }
            } else if due - now <= HORIZON_MS && e.phase != ProgressPhase::Blocked {
                attention.push(point(
                    AttentionKind::Due,
                    due,
                    None,
                    Some(1.0),
                    format!("{who} 的任务快到截止：{title}"),
                ));
            }
        }
        // 可能会问你：这个任务还没问过人，按历史上比它走得长的任务里、走过这一步以后还问过人的比例。
        if matches!(e.phase, ProgressPhase::Working | ProgressPhase::Queued)
            && !task.thread_id.as_ref().is_some_and(|th| input.asked.contains(th))
        {
            let role = task.role.as_ref().or_else(|| role_of.get(assignee).copied());
            let base = baseline(role);
            if let (true, Some(msps), Some(r)) = (base.has_tasks(), base.ms_per_step, &est.residual) {
                let s = f64::from(e.steps_done);
                let longer: Vec<&TaskSample> = base.tasks.iter().filter(|t| t.steps > s).collect();
                let mut ask_steps: Vec<f64> = longer.iter().filter_map(|t| t.asked_at.filter(|a| *a > s)).collect();
                if !longer.is_empty() && !ask_steps.is_empty() {
                    let p = ask_steps.len() as f64 / longer.len() as f64;
                    if p >= MIN_LIKELIHOOD {
                        ask_steps.sort_by(f64::total_cmp);
                        let at = r.start_ms + (quantile(&ask_steps, 0.5) - s) * msps;
                        let late = r.start_ms + (quantile(&ask_steps, 0.8) - s) * msps;
                        attention.push(point(
                            AttentionKind::LikelyQuestion,
                            at as i64,
                            Some(late as i64),
                            Some(round2(p)),
                            format!(
                                "{who} 可能会问你：{title}（{} 以往 {}/{} 次走到这一步之后问过人，多在第 {} 步前后）",
                                role_name(role),
                                ask_steps.len(),
                                longer.len(),
                                quantile(&ask_steps, 0.5).round()
                            ),
                        ));
                    }
                }
            }
        }
        // 交回结果：只算人派的任务。员工派的回到派活的员工那里，由下面的“汇总回复”算。
        if matches!(task.created_by, Author::Human { .. })
            && let (Some(eta), Some(late)) = (e.eta_ms, e.eta_late_ms)
        {
            attention.push(point(
                AttentionKind::Result,
                eta,
                Some(late),
                Some(1.0),
                format!("{who} 交回结果：{title}"),
            ));
        }
    }

    // 汇总回复：员工在不是任务的对话里（多半是和人聊）派出去的任务，都回来以后再汇总一轮。
    let mut origins: Vec<(&ThreadId, &Vec<&Task>)> = children
        .iter()
        .map(|(origin, list)| (*origin, list))
        .filter(|(origin, _)| !task_by_thread.contains_key(origin))
        .collect();
    origins.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    for (origin, list) in origins {
        let open: Vec<&&Task> = list.iter().filter(|t| !t.status.is_closed()).collect();
        let Some(Author::Agent { agent_id: boss }) = open.first().map(|t| &t.created_by) else {
            continue;
        };
        if open
            .iter()
            .any(|t| !matches!(&t.created_by, Author::Agent { agent_id } if agent_id == boss))
        {
            continue;
        }
        let mut latest = (now, now);
        let mut known = true;
        for t in &open {
            match by_task
                .get(&t.id)
                .and_then(|e| e.estimate.eta_ms.zip(e.estimate.eta_late_ms))
            {
                Some((e, l)) => latest = (latest.0.max(e), latest.1.max(l)),
                None => known = false,
            }
        }
        let boss_role = role_of.get(boss).copied();
        let Some((t50, t80)) = baseline(boss_role).turn_duration() else {
            continue;
        };
        if !known {
            continue;
        }
        attention.push(AttentionPoint {
            kind: AttentionKind::Reply,
            agent_id: Some(boss.clone()),
            role_id: boss_role.cloned(),
            task_id: None,
            thread_id: Some(origin.clone()),
            at_ms: latest.0 + t50 as i64,
            late_ms: Some(latest.1 + t80 as i64),
            likelihood: Some(1.0),
            text: format!("{} 收齐 {} 件子任务的结果后回复你", name_of(boss), open.len()),
        });
    }

    budget(input, &mut attention);

    attention.retain(|p| p.at_ms <= now + HORIZON_MS);
    attention.sort_by(|a, b| {
        a.at_ms
            .cmp(&b.at_ms)
            .then_with(|| b.likelihood.unwrap_or(0.0).total_cmp(&a.likelihood.unwrap_or(0.0)))
            .then_with(|| a.text.cmp(&b.text))
    });
    ProgressBoard {
        now_ms: now,
        agents: agents_out,
        attention,
    }
}

/// 正在跑的这一轮已经走完几步：内置循环是当前步之前的；外部引擎看不到步数，按用时和每步用时折算。
fn live_steps(r: &Running, now: i64, base: &Baseline) -> u32 {
    if r.step > 0 {
        return r.step - 1;
    }
    match base.ms_per_step {
        Some(msps) => ((now - r.started_ms).max(0) as f64 / msps).floor() as u32,
        None => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn task_estimate(
    task: &Task,
    phase: ProgressPhase,
    totals: TurnTotals,
    live: u32,
    base: &Baseline,
    role_name: &str,
    start: Option<(f64, f64)>,
) -> TaskEst {
    let steps_done = totals.steps.saturating_add(live);
    let s = f64::from(steps_done);
    let mut estimate = Estimate {
        scope: EstimateScope::Task,
        thread_id: task.thread_id.clone().unwrap_or_else(|| ThreadId::from("")),
        task_id: Some(task.id.clone()),
        title: task.title.clone(),
        phase,
        started_ms: task.created_at_ms,
        steps_done,
        steps_expected: None,
        fraction: None,
        eta_ms: None,
        eta_late_ms: None,
        samples: 0,
        basis: format!("{role_name}做完的任务还不到 {MIN_TASK_SAMPLES} 个，先不估"),
    };
    if !base.has_tasks() {
        return TaskEst {
            estimate,
            residual: None,
        };
    }
    let rem = remaining(&base.task_steps, s, 1.0);
    estimate.samples = base.tasks.len() as u32;
    estimate.steps_expected = Some((s + rem.p50).round() as u32);
    estimate.fraction = Some(round2((s / (s + rem.p50)).min(MAX_FRACTION)));
    let mut basis = format!(
        "按{role_name}最近 {} 个做完的任务：中位数 {} 步",
        base.tasks.len(),
        quantile(&base.task_steps, 0.5).round()
    );
    let mut residual = None;
    match (base.ms_per_step, start) {
        (Some(msps), Some((s50, s80))) => {
            basis.push_str(&format!("，每步约 {}", duration_text(msps)));
            estimate.eta_ms = Some((s50 + rem.p50 * msps) as i64);
            estimate.eta_late_ms = Some((s80 + rem.p80 * msps).max(s50 + rem.p50 * msps) as i64);
            residual = Some(Residual {
                start_ms: s50,
                ms_per_step: msps,
                left: base.task_steps.iter().filter(|v| **v > s).map(|v| v - s).collect(),
            });
        }
        (None, _) => basis.push_str(&format!("；成功的轮次还不到 {MIN_TURN_SAMPLES} 个，估不出时间")),
        // 不在时间轴上（在等人、受阻、停着）：时间取决于人什么时候回，不估。
        (Some(_), None) => {}
    }
    if rem.longer_than_all {
        basis.push_str("；已经比以往都长");
    }
    estimate.basis = basis;
    TaskEst { estimate, residual }
}

fn turn_estimate(
    thread: &ThreadId,
    running: Option<&Running>,
    now: i64,
    base: &Baseline,
    role_name: &str,
    start: Option<(f64, f64)>,
) -> Estimate {
    let started_ms = running.map_or(now, |r| r.started_ms);
    let mut estimate = Estimate {
        scope: EstimateScope::Turn,
        thread_id: thread.clone(),
        task_id: None,
        title: "这一轮".to_string(),
        phase: if running.is_some() {
            ProgressPhase::Working
        } else {
            ProgressPhase::Queued
        },
        started_ms,
        steps_done: 0,
        steps_expected: None,
        fraction: None,
        eta_ms: None,
        eta_late_ms: None,
        samples: 0,
        basis: format!("{role_name}记下的轮次还不到 {MIN_TURN_SAMPLES} 个，先不估"),
    };
    let (Some(msps), true) = (base.ms_per_step, base.has_turns()) else {
        return estimate;
    };
    estimate.samples = base.turn_steps.len() as u32;
    let s = running.map_or(0, |r| live_steps(r, now, base));
    estimate.steps_done = s;
    let x = f64::from(s);
    let rem = remaining(&base.turn_steps, x, 1.0);
    estimate.steps_expected = Some((x + rem.p50).round() as u32);
    estimate.fraction = Some(round2((x / (x + rem.p50)).min(MAX_FRACTION)));
    if let Some((s50, s80)) = start {
        estimate.eta_ms = Some((s50 + rem.p50 * msps) as i64);
        estimate.eta_late_ms = Some((s80 + rem.p80 * msps).max(s50 + rem.p50 * msps) as i64);
    }
    estimate.basis = format!(
        "按{role_name}最近 {} 轮：中位数 {} 步，每步约 {}{}",
        base.turn_steps.len(),
        quantile(&base.turn_steps, 0.5).round(),
        duration_text(msps),
        if rem.longer_than_all {
            "；这一轮已经比以往都长"
        } else {
            ""
        }
    );
    estimate
}

/// 正在跑的这一轮可能走满步数上限：按历史上比它长的轮次里、走满上限的比例。没有历史时，只在离上限 2 步以内时列。
fn step_limit(
    input: &Inputs<'_>,
    agent: &Agent,
    running: Option<&Running>,
    base: &Baseline,
    out: &mut Vec<AttentionPoint>,
) {
    let Some(r) = running.filter(|r| r.step > 0) else {
        return;
    };
    let max = f64::from(input.max_steps.max(1));
    let s = f64::from(r.step.saturating_sub(1));
    let left = (max - s).max(0.0);
    let likelihood = if base.has_turns() {
        let longer: Vec<&f64> = base.turn_steps.iter().filter(|v| **v > s).collect();
        if longer.is_empty() {
            return;
        }
        let hit = longer.iter().filter(|v| ***v >= max).count();
        Some(hit as f64 / longer.len() as f64)
    } else if left <= 2.0 {
        None
    } else {
        return;
    };
    if likelihood.is_some_and(|p| p < MIN_LIKELIHOOD) {
        return;
    }
    let at = input.now_ms + base.ms_per_step.map_or(0.0, |m| left * m) as i64;
    out.push(AttentionPoint {
        kind: AttentionKind::StepLimit,
        agent_id: Some(agent.id.clone()),
        role_id: Some(agent.role.clone()),
        task_id: None,
        thread_id: Some(r.thread_id.clone()),
        at_ms: at,
        late_ms: None,
        likelihood: likelihood.map(round2),
        text: format!(
            "{} 这一轮可能走满 {} 步上限而中断（已走 {} 步）",
            agent.name, input.max_steps, s
        ),
    });
}

/// 预算：已经用完的现在就要看；没用完的按最近一小时的花费速度外推，今天（UTC）之内会用完的才列。
fn budget(input: &Inputs<'_>, out: &mut Vec<AttentionPoint>) {
    let mut roles: Vec<&Role> = input.roles.values().collect();
    roles.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    for role in roles {
        let Some(limit) = role.budget_per_day_usd.filter(|b| *b > 0.0) else {
            continue;
        };
        let spent = input.spent_today.get(&role.id).copied().unwrap_or(0.0);
        let members: Vec<&Agent> = input.agents.iter().filter(|a| a.role == role.id).collect();
        let busy = members.iter().any(|a| {
            input.live.running.contains_key(&a.id) || input.live.queued.get(&a.id).is_some_and(|q| !q.is_empty())
        });
        let point = |at_ms, likelihood, text| AttentionPoint {
            kind: AttentionKind::Budget,
            agent_id: None,
            role_id: Some(role.id.clone()),
            task_id: None,
            thread_id: None,
            at_ms,
            late_ms: None,
            likelihood,
            text,
        };
        if spent >= limit {
            if !members.is_empty() {
                out.push(point(
                    input.now_ms,
                    Some(1.0),
                    format!(
                        "{}今天的预算 ${limit} 用完了，这个岗位的员工会停下",
                        role.display_name()
                    ),
                ));
            }
            continue;
        }
        let recent = input.spent_recent.get(&role.id).copied().unwrap_or(0.0);
        if !busy || recent <= 0.0 {
            continue;
        }
        let per_ms = recent / SPEND_WINDOW_MS as f64;
        let at = input.now_ms + ((limit - spent) / per_ms) as i64;
        if at < input.day_end_ms {
            out.push(point(
                at,
                None,
                format!(
                    "按最近一小时的花费速度，{}今天的预算 ${limit} 会用完（已花 ${:.2}），之后这个岗位的员工会停下",
                    role.display_name(),
                    spent
                ),
            ));
        }
    }
}

fn phase_rank(phase: ProgressPhase) -> u8 {
    match phase {
        ProgressPhase::Working => 0,
        ProgressPhase::Queued => 1,
        ProgressPhase::WaitingHuman => 2,
        ProgressPhase::Blocked => 3,
        ProgressPhase::Failed => 4,
        ProgressPhase::WaitingColleague => 5,
        ProgressPhase::Stalled => 6,
        ProgressPhase::Idle => 7,
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn short(s: &str, max: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= max {
        one
    } else {
        format!("{}…", one.chars().take(max).collect::<String>())
    }
}

/// “约 6 秒”“约 3 分钟”“约 1.5 小时”。
fn duration_text(ms: f64) -> String {
    let secs = ms / 1000.0;
    if secs < 90.0 {
        format!("{} 秒", secs.round().max(1.0))
    } else if secs < 5400.0 {
        format!("{} 分钟", (secs / 60.0).round())
    } else {
        format!("{:.1} 小时", secs / 3600.0)
    }
}

// ---------------------------------------------------------------------------
// 从存储和内存里取数据

/// 一次最多看多少个没结束的任务、多少张待决定的卡片。
const OPEN_TASKS: usize = 500;
const OPEN_APPROVALS: usize = 200;
const DAY_MS: i64 = 86_400_000;

impl crate::Runtime {
    /// 估一张进度表（`list_progress`）。`now_ms` 由调用方给，测试里可以拨快时间。
    pub async fn progress(&self, now_ms: i64) -> Result<ProgressBoard, crate::RuntimeError> {
        let shared = &self.shared;
        let store = &shared.store;
        let agents = self.agents();
        let roles: HashMap<RoleId, Role> = shared.directory.read().unwrap_or_else(|p| p.into_inner()).roles.clone();
        let tasks = store.tasks(true, OPEN_TASKS).await?;
        let approvals = store.approvals(true, OPEN_APPROVALS).await?;
        let threads: Vec<ThreadId> = tasks.iter().filter_map(|t| t.thread_id.clone()).collect();
        let totals = store.turn_totals(&threads).await?;
        let asked: HashSet<ThreadId> = store
            .question_times(&threads)
            .await?
            .into_iter()
            .map(|(thread, _)| thread)
            .collect();
        let live = shared.live.snapshot();
        let ids: Vec<AgentId> = agents.iter().map(|a| a.id.clone()).collect();
        let last_turns: HashMap<AgentId, TurnRecord> = store
            .last_turns(&ids)
            .await?
            .into_iter()
            .map(|t| (t.agent_id.clone(), t))
            .collect();
        let wanted: HashSet<&RoleId> = agents.iter().map(|a| &a.role).collect();
        let mut baselines = HashMap::with_capacity(wanted.len());
        for role in wanted {
            baselines.insert(role.clone(), self.baseline(role, now_ms).await?);
        }
        let spent_today = shared.ledger.today(store).await?.spent;
        let spent_recent = store.turn_cost_since(now_ms - SPEND_WINDOW_MS).await?;
        Ok(estimate(&Inputs {
            now_ms,
            max_steps: shared.config.max_steps.max(1),
            day_end_ms: (now_ms.div_euclid(DAY_MS) + 1) * DAY_MS,
            agents: &agents,
            roles: &roles,
            tasks: &tasks,
            approvals: &approvals,
            asked: &asked,
            totals: &totals,
            live: &live,
            last_turns: &last_turns,
            baselines: &baselines,
            spent_today: &spent_today,
            spent_recent: &spent_recent,
        }))
    }

    /// 这个岗位的历史基线；[`BASELINE_TTL_MS`] 内复用上次算的。
    async fn baseline(&self, role: &RoleId, now_ms: i64) -> Result<Arc<Baseline>, crate::RuntimeError> {
        let shared = &self.shared;
        {
            let cache = shared.baselines.lock().unwrap_or_else(|p| p.into_inner());
            if let Some((at, b)) = cache.get(role)
                && now_ms >= *at
                && now_ms - at < BASELINE_TTL_MS
            {
                return Ok(b.clone());
            }
        }
        let store = &shared.store;
        let done = store
            .done_tasks(role, now_ms - HISTORY_DAYS * DAY_MS, HISTORY_TASKS)
            .await?;
        let threads: Vec<ThreadId> = done.iter().filter_map(|t| t.thread_id.clone()).collect();
        let mut turns: HashMap<ThreadId, Vec<TurnRecord>> = HashMap::new();
        for t in store.turns_in_threads(&threads).await? {
            turns.entry(t.thread_id.clone()).or_default().push(t);
        }
        let mut questions: HashMap<ThreadId, Vec<i64>> = HashMap::new();
        for (thread, at) in store.question_times(&threads).await? {
            questions.entry(thread).or_default().push(at);
        }
        let samples: Vec<TaskSample> = done
            .iter()
            .filter_map(|t| {
                let thread = t.thread_id.as_ref()?;
                task_sample(
                    t.created_at_ms,
                    t.updated_at_ms,
                    turns.get(thread).map_or(&[][..], Vec::as_slice),
                    questions.get(thread).map_or(&[][..], Vec::as_slice),
                )
            })
            .collect();
        let recent = store.recent_turns(role, false, HISTORY_TURNS).await?;
        let baseline = Arc::new(Baseline::new(samples, &recent));
        shared
            .baselines
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(role.clone(), (now_ms, baseline.clone()));
        Ok(baseline)
    }
}

#[cfg(test)]
#[path = "progress_tests.rs"]
mod tests;
