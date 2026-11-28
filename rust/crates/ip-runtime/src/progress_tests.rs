//! 估算器的单元测试：直接喂构造好的数据，不碰存储。时间都用整数毫秒，方便心算。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use ip_core::{
    Agent, AgentId, Approval, ApprovalOption, ApprovalState, ApprovalSubject, Author, ModelSlot, Role, RoleId, Task,
    TaskStatus, ThreadId,
};
use ip_store::{TurnRecord, TurnTotals};

use super::*;

const NOW: i64 = 10_000_000;

fn role(id: &str, budget: Option<f64>) -> Role {
    Role {
        id: id.into(),
        name: Some(format!("{id}岗")),
        description: String::new(),
        model: ModelSlot {
            default: "fake/m".into(),
            escalate: None,
        },
        budget_per_day_usd: budget,
        coordinator: false,
        approve_completion: false,
    }
}

fn agent(id: &str, role: &str) -> Agent {
    Agent {
        id: id.into(),
        role: role.into(),
        name: format!("员工{id}"),
        celestial_style: None,
    }
}

fn task(id: &str, assignee: Option<&str>, status: TaskStatus) -> Task {
    Task {
        id: id.into(),
        title: format!("任务{id}"),
        description: String::new(),
        status,
        role: Some("r".into()),
        assignee: assignee.map(AgentId::from),
        thread_id: Some(format!("th-{id}").into()),
        origin_thread_id: None,
        result: None,
        depth: 1,
        created_by: Author::human("张三"),
        created_at_ms: NOW - 60_000,
        updated_at_ms: NOW - 60_000,
        due_ms: None,
        needs_approval: false,
    }
}

fn turn(agent: &str, thread: &str, started: i64, ms: i64, steps: u32, ok: bool) -> TurnRecord {
    TurnRecord {
        agent_id: agent.into(),
        role_id: "r".into(),
        thread_id: thread.into(),
        started_ms: started,
        finished_ms: started + ms,
        steps,
        ok,
        cost_usd: 0.0,
        error: (!ok).then(|| "模型超时".to_string()),
    }
}

/// 岗位 r 的历史：任务总步数 4、6、8、10、12，其中走 10 步和 12 步的两个在第 7 步问过人；
/// 轮次 10 个，每个 3 步、3 秒（每步 1 秒）。
fn history() -> Baseline {
    let tasks = [4.0, 6.0, 8.0, 10.0, 12.0]
        .into_iter()
        .map(|steps| TaskSample {
            steps,
            asked_at: (steps >= 10.0).then_some(7.0),
        })
        .collect();
    let turns: Vec<TurnRecord> = (0..10).map(|i| turn("x", "old", i * 10_000, 3_000, 3, true)).collect();
    Baseline::new(tasks, &turns)
}

struct World {
    agents: Vec<Agent>,
    roles: HashMap<RoleId, Role>,
    tasks: Vec<Task>,
    approvals: Vec<Approval>,
    asked: HashSet<ThreadId>,
    totals: HashMap<ThreadId, TurnTotals>,
    live: LiveSnapshot,
    last_turns: HashMap<AgentId, TurnRecord>,
    baselines: HashMap<RoleId, Arc<Baseline>>,
    spent_today: HashMap<RoleId, f64>,
    spent_recent: HashMap<RoleId, f64>,
    max_steps: u32,
}

impl World {
    fn new(baseline: Baseline) -> Self {
        Self {
            agents: vec![agent("a1", "r")],
            roles: HashMap::from([("r".into(), role("r", None))]),
            tasks: Vec::new(),
            approvals: Vec::new(),
            asked: HashSet::new(),
            totals: HashMap::new(),
            live: LiveSnapshot::default(),
            last_turns: HashMap::new(),
            baselines: HashMap::from([("r".into(), Arc::new(baseline))]),
            spent_today: HashMap::new(),
            spent_recent: HashMap::new(),
            max_steps: 16,
        }
    }

    fn run(&mut self, agent: &str, thread: &str, started: i64, step: u32) {
        self.live.running.insert(
            agent.into(),
            Running {
                thread_id: thread.into(),
                started_ms: started,
                step,
            },
        );
    }

    fn board(&self) -> ProgressBoard {
        estimate(&Inputs {
            now_ms: NOW,
            max_steps: self.max_steps,
            day_end_ms: (NOW / 86_400_000 + 1) * 86_400_000,
            agents: &self.agents,
            roles: &self.roles,
            tasks: &self.tasks,
            approvals: &self.approvals,
            asked: &self.asked,
            totals: &self.totals,
            live: &self.live,
            last_turns: &self.last_turns,
            baselines: &self.baselines,
            spent_today: &self.spent_today,
            spent_recent: &self.spent_recent,
        })
    }
}

fn kinds(board: &ProgressBoard) -> Vec<AttentionKind> {
    board.attention.iter().map(|p| p.kind).collect()
}

fn point(board: &ProgressBoard, kind: AttentionKind) -> &AttentionPoint {
    board
        .attention
        .iter()
        .find(|p| p.kind == kind)
        .unwrap_or_else(|| panic!("没有 {kind:?}：{:?}", kinds(board)))
}

#[test]
fn remaining_uses_only_longer_history() {
    let sorted = [4.0, 6.0, 8.0, 10.0, 12.0];
    let fresh = remaining(&sorted, 0.0, 1.0);
    assert_eq!((fresh.p50, fresh.longer_than_all), (8.0, false));
    // 走到第 7 步：只看 8、10、12。
    let mid = remaining(&sorted, 7.0, 1.0);
    assert_eq!(mid.p50, 3.0);
    assert!((mid.p80 - 4.2).abs() < 1e-9);
    // 比以往都长：再走 1 步，80 分位按四分之一。
    let long = remaining(&sorted, 20.0, 1.0);
    assert_eq!((long.p50, long.p80, long.longer_than_all), (1.0, 5.0, true));
}

#[test]
fn task_sample_counts_turns_inside_the_task_and_the_first_question() {
    let turns = [
        turn("a", "t", 50, 10, 9, true), // 建任务以前，不算
        turn("a", "t", 100, 10, 3, true),
        turn("a", "t", 200, 10, 2, false),
        turn("a", "t", 300, 10, 4, true),
        turn("a", "t", 900, 10, 5, true), // 做完以后，不算
    ];
    let s = task_sample(100, 800, &turns, &[40, 250, 350]).unwrap();
    assert_eq!(s.steps, 9.0);
    assert_eq!(s.asked_at, Some(5.0), "250 那次落在第二轮里，整轮算进去");
    assert!(task_sample(100, 800, &[], &[]).is_none(), "没有轮次的不算样本");
}

#[test]
fn baseline_needs_enough_ok_turns_for_speed() {
    let few: Vec<TurnRecord> = (0..4).map(|i| turn("a", "t", i, 4_000, 2, true)).collect();
    assert_eq!(Baseline::new(Vec::new(), &few).ms_per_step, None);
    let mut many: Vec<TurnRecord> = (0..5).map(|i| turn("a", "t", i, 4_000, 2, true)).collect();
    many.push(turn("a", "t", 9, 1, 1, false));
    let b = Baseline::new(Vec::new(), &many);
    assert_eq!(b.ms_per_step, Some(2_000.0), "失败的轮次不算速度");
    assert_eq!(
        b.turn_steps.len(),
        6,
        "步数分布里有失败的轮次（走满上限的轮次是失败的）"
    );
}

#[test]
fn live_tracks_queue_and_running() {
    let live = Live::default();
    let (a, t1, t2) = (AgentId::from("a"), ThreadId::from("t1"), ThreadId::from("t2"));
    live.queue(&a, &t1);
    live.queue(&a, &t2);
    live.queue(&a, &t1);
    assert_eq!(live.snapshot().queued[&a], [t1.clone(), t2.clone()], "排着的不重复记");
    live.start(&a, &t1, 5);
    live.step(&a, 3);
    let snap = live.snapshot();
    assert_eq!(snap.queued[&a], std::slice::from_ref(&t2));
    assert_eq!(snap.running[&a].step, 3);
    assert_eq!(live.finish(&a).unwrap().thread_id, t1);
    live.unqueue(&a, &t2);
    let snap = live.snapshot();
    assert!(snap.running.is_empty() && snap.queued.is_empty());
}

#[test]
fn without_history_nothing_is_made_up() {
    let mut w = World::new(Baseline::default());
    w.tasks.push(task("1", Some("a1"), TaskStatus::InProgress));
    w.run("a1", "th-1", NOW - 5_000, 2);
    let board = w.board();
    let a = &board.agents[0];
    assert_eq!(a.phase, ProgressPhase::Working);
    let head = a.headline.as_ref().unwrap();
    assert_eq!((head.fraction, head.eta_ms, head.samples), (None, None, 0));
    assert!(head.basis.contains("还不到 3 个"), "{}", head.basis);
    assert!(board.attention.is_empty(), "估不出就不预测：{:?}", kinds(&board));
}

#[test]
fn working_task_gets_fraction_eta_result_and_likely_question() {
    let mut w = World::new(history());
    w.tasks.push(task("1", Some("a1"), TaskStatus::InProgress));
    // 前面几轮走了 4 步，这一轮在第 3 步（已走完 2 步）：一共 6 步。
    w.totals.insert(
        "th-1".into(),
        TurnTotals {
            turns: 2,
            steps: 4,
            active_ms: 4_000,
            last_finished_ms: NOW - 10_000,
        },
    );
    w.run("a1", "th-1", NOW - 2_000, 3);
    let board = w.board();
    let head = board.agents[0].headline.clone().unwrap();
    assert_eq!(head.scope, EstimateScope::Task);
    assert_eq!(head.steps_done, 6);
    // 比 6 长的是 8、10、12，还剩中位数 4 步：6 / 10。每步 1 秒。
    assert_eq!(head.steps_expected, Some(10));
    assert_eq!(head.fraction, Some(0.6));
    assert_eq!(head.eta_ms, Some(NOW + 4_000));
    assert!(head.eta_late_ms.unwrap() > NOW + 4_000);
    assert_eq!(head.samples, 5);
    assert!(head.basis.contains("最近 5 个做完的任务"), "{}", head.basis);

    let result = point(&board, AttentionKind::Result);
    assert_eq!(result.at_ms, NOW + 4_000);
    assert_eq!(result.likelihood, Some(1.0));
    // 比 6 长的 3 个里有 2 个在第 7 步问过人。
    let ask = point(&board, AttentionKind::LikelyQuestion);
    assert_eq!(ask.likelihood, Some(0.67));
    assert_eq!(ask.at_ms, NOW + 1_000);
    assert!(ask.text.contains("2/3"), "{}", ask.text);

    // 已经问过人的任务不再预测“可能会问你”；员工派的任务不算“交回结果”。
    w.asked.insert("th-1".into());
    w.tasks[0].created_by = Author::Agent {
        agent_id: "boss".into(),
    };
    let board = w.board();
    assert!(board.attention.is_empty(), "{:?}", kinds(&board));
}

#[test]
fn fraction_never_reaches_one_before_done() {
    let mut w = World::new(history());
    w.tasks.push(task("1", Some("a1"), TaskStatus::InProgress));
    w.totals.insert(
        "th-1".into(),
        TurnTotals {
            steps: 40,
            ..TurnTotals::default()
        },
    );
    w.run("a1", "th-1", NOW, 1);
    let head = w.board().agents[0].headline.clone().unwrap();
    assert_eq!(head.fraction, Some(MAX_FRACTION));
    assert!(head.basis.contains("比以往都长"), "{}", head.basis);
}

#[test]
fn queued_work_starts_after_the_running_turn() {
    let mut w = World::new(history());
    w.tasks.push(task("1", Some("a1"), TaskStatus::InProgress));
    w.tasks.push(task("2", Some("a1"), TaskStatus::Open));
    w.run("a1", "th-1", NOW, 1);
    w.live.queued.insert("a1".into(), vec!["th-1".into(), "th-2".into()]);
    let board = w.board();
    let a = &board.agents[0];
    assert_eq!(a.phase, ProgressPhase::Working);
    let by_id = |id: &str| {
        a.tasks
            .iter()
            .find(|e| e.task_id.as_ref().unwrap().as_str() == id)
            .unwrap()
    };
    // 第一个从 0 步起，还剩中位数 8 步；第二个排在它后面，再 8 步。
    assert_eq!(by_id("1").eta_ms, Some(NOW + 8_000));
    assert_eq!(by_id("2").phase, ProgressPhase::Queued);
    assert_eq!(by_id("2").eta_ms, Some(NOW + 16_000));
    assert_eq!(by_id("2").fraction, Some(0.0));
}

#[test]
fn plain_turn_and_step_limit() {
    let mut w = World::new(history());
    w.max_steps = 3;
    // 没挂任务的对话；历史上的轮次都是 3 步，已经走完 1 步，还剩 2 步，正好走满上限。
    w.run("a1", "chat", NOW - 1_000, 2);
    let board = w.board();
    let head = board.agents[0].headline.clone().unwrap();
    assert_eq!(head.scope, EstimateScope::Turn);
    assert_eq!((head.steps_done, head.steps_expected), (1, Some(3)));
    assert_eq!(head.eta_ms, Some(NOW + 2_000));
    let limit = point(&board, AttentionKind::StepLimit);
    assert_eq!((limit.likelihood, limit.at_ms), (Some(1.0), NOW + 2_000));

    // 上限够用时不列。
    w.max_steps = 16;
    assert!(w.board().attention.is_empty());
}

#[test]
fn waiting_states_and_what_needs_a_human_now() {
    let mut w = World::new(history());
    w.agents.push(agent("a2", "r"));
    w.agents.push(agent("a3", "r"));
    let mut blocked = task("b", Some("a1"), TaskStatus::Blocked);
    blocked.result = Some("拿不到 CPI 数据".into());
    w.tasks.push(blocked);
    w.tasks.push(task("q", Some("a2"), TaskStatus::InProgress));
    w.approvals.push(Approval {
        id: "c1".into(),
        question: "要同比还是环比？".into(),
        options: vec![ApprovalOption {
            label: "同比".into(),
            consequence: String::new(),
        }],
        recommended: None,
        reason: String::new(),
        subject: ApprovalSubject::AgentQuestion {
            agent_id: "a2".into(),
            thread_id: "th-q".into(),
        },
        state: ApprovalState::Pending,
        created_at_ms: NOW - 30_000,
    });
    let mut stalled = task("s", Some("a3"), TaskStatus::InProgress);
    stalled.updated_at_ms = NOW - STALL_GRACE_MS - 1;
    w.tasks.push(stalled);
    w.tasks.push(task("u", None, TaskStatus::Open));
    let board = w.board();
    let phase = |id: &str| board.agents.iter().find(|a| a.agent_id.as_str() == id).unwrap().phase;
    assert_eq!(phase("a1"), ProgressPhase::Blocked);
    assert_eq!(phase("a2"), ProgressPhase::WaitingHuman);
    assert_eq!(phase("a3"), ProgressPhase::Stalled);
    let mut got = kinds(&board);
    got.sort_by_key(|k| format!("{k:?}"));
    assert_eq!(
        got,
        [
            AttentionKind::Blocked,
            AttentionKind::Question,
            AttentionKind::Stalled,
            AttentionKind::Unassigned
        ]
    );
    assert!(point(&board, AttentionKind::Blocked).text.contains("拿不到 CPI 数据"));
    assert_eq!(
        point(&board, AttentionKind::Question)
            .task_id
            .as_ref()
            .unwrap()
            .as_str(),
        "q"
    );
    // 在等人、受阻、停着的不估做完时间。
    assert!(
        board
            .agents
            .iter()
            .all(|a| a.headline.as_ref().unwrap().eta_ms.is_none())
    );
    // 清单按时刻排。
    assert!(board.attention.windows(2).all(|p| p[0].at_ms <= p[1].at_ms));

    // 刚停下的不算“停着要人看”。
    w.tasks[2].updated_at_ms = NOW - 1_000;
    assert!(!kinds(&w.board()).contains(&AttentionKind::Stalled));
}

#[test]
fn failed_turn_is_reported_once() {
    let mut w = World::new(history());
    w.last_turns
        .insert("a1".into(), turn("a1", "chat", NOW - 9_000, 1_000, 1, false));
    let board = w.board();
    assert_eq!(board.agents[0].phase, ProgressPhase::Failed);
    assert!(point(&board, AttentionKind::Failed).text.contains("模型超时"));

    // 挂着任务的失败已经变成受阻，不再另报失败。
    w.last_turns
        .insert("a1".into(), turn("a1", "th-1", NOW - 9_000, 1_000, 1, false));
    w.tasks.push(task("1", Some("a1"), TaskStatus::Blocked));
    assert_eq!(kinds(&w.board()), [AttentionKind::Blocked]);
}

#[test]
fn delegated_tasks_roll_up_to_the_parent_and_the_reply() {
    let mut w = World::new(history());
    w.agents.push(agent("boss", "r"));
    // 人和幕僚长在对话 chat 里聊，幕僚长派了任务 c 给 a1；另有一个人派给幕僚长的任务 p，它又派了子任务 k。
    let mut child = task("c", Some("a1"), TaskStatus::InProgress);
    child.created_by = Author::Agent {
        agent_id: "boss".into(),
    };
    child.origin_thread_id = Some("chat".into());
    w.tasks.push(child);
    let parent = task("p", Some("boss"), TaskStatus::InProgress);
    w.tasks.push(parent);
    let mut grandchild = task("k", Some("a1"), TaskStatus::Open);
    grandchild.created_by = Author::Agent {
        agent_id: "boss".into(),
    };
    grandchild.origin_thread_id = Some("th-p".into());
    grandchild.depth = 2;
    w.tasks.push(grandchild);
    w.run("a1", "th-c", NOW, 1);
    w.live.queued.insert("a1".into(), vec!["th-k".into()]);
    let board = w.board();
    let boss = board.agents.iter().find(|a| a.agent_id.as_str() == "boss").unwrap();
    let p = boss.headline.as_ref().unwrap();
    assert_eq!(p.phase, ProgressPhase::WaitingColleague);
    // 子任务 c 8 秒、k 排在它后面再 8 秒；汇总一轮 3 秒。
    assert_eq!(p.eta_ms, Some(NOW + 16_000 + 3_000));
    assert_eq!(
        point(&board, AttentionKind::Result).task_id.as_ref().unwrap().as_str(),
        "p"
    );
    let reply = point(&board, AttentionKind::Reply);
    assert_eq!(reply.thread_id.as_ref().unwrap().as_str(), "chat");
    assert_eq!(reply.at_ms, NOW + 8_000 + 3_000);
    assert_eq!(reply.agent_id.as_ref().unwrap().as_str(), "boss");
}

#[test]
fn deadlines() {
    let mut w = World::new(history());
    let mut late = task("1", Some("a1"), TaskStatus::InProgress);
    late.due_ms = Some(NOW - 1);
    w.tasks.push(late);
    assert!(kinds(&w.board()).contains(&AttentionKind::Overdue));

    // 从 0 步起，剩 4、6、8、10、12 步；截止在 7 秒以后：8、10、12 三个赶不上。
    w.tasks[0].due_ms = Some(NOW + 7_000);
    w.run("a1", "th-1", NOW, 1);
    let board = w.board();
    assert_eq!(point(&board, AttentionKind::DueRisk).likelihood, Some(0.6));

    // 估不出时间的，24 小时内到期就提醒截止时刻。
    let mut w = World::new(Baseline::default());
    let mut soon = task("1", Some("a1"), TaskStatus::InProgress);
    soon.due_ms = Some(NOW + 3_600_000);
    w.tasks.push(soon);
    w.run("a1", "th-1", NOW, 1);
    assert_eq!(point(&w.board(), AttentionKind::Due).at_ms, NOW + 3_600_000);
}

#[test]
fn budget_runs_out() {
    let mut w = World::new(history());
    w.roles.insert("r".into(), role("r", Some(10.0)));
    w.spent_today.insert("r".into(), 9.0);
    w.spent_recent.insert("r".into(), 2.0);
    // 没在做事：不外推。
    assert!(w.board().attention.is_empty());
    // 在做事：每小时 2 美元，剩 1 美元，半小时用完。
    w.run("a1", "chat", NOW, 1);
    let board = w.board();
    let b = point(&board, AttentionKind::Budget);
    assert_eq!(b.at_ms, NOW + 1_800_000);
    assert_eq!(b.likelihood, None, "按速度外推的不给可能性");
    // 已经用完：现在就要看。
    w.spent_today.insert("r".into(), 10.0);
    let board = w.board();
    let b = point(&board, AttentionKind::Budget);
    assert_eq!((b.at_ms, b.likelihood), (NOW, Some(1.0)));
}

#[test]
fn far_predictions_are_dropped() {
    let mut slow = history();
    slow.ms_per_step = Some(HORIZON_MS as f64);
    let mut w = World::new(slow);
    w.tasks.push(task("1", Some("a1"), TaskStatus::InProgress));
    w.run("a1", "th-1", NOW, 1);
    let board = w.board();
    assert!(board.agents[0].headline.as_ref().unwrap().eta_ms.is_some(), "估算照给");
    assert!(board.attention.is_empty(), "24 小时以外的预测不列：{:?}", kinds(&board));
}

#[test]
fn pending_review_waits_for_a_human_and_is_not_stalled() {
    let mut w = World::new(history());
    let mut done = task("v", Some("a1"), TaskStatus::PendingApproval);
    done.needs_approval = true;
    done.updated_at_ms = NOW - STALL_GRACE_MS * 10;
    w.tasks.push(done);
    w.totals.insert(
        "th-v".into(),
        TurnTotals {
            steps: 9,
            ..TurnTotals::default()
        },
    );
    w.approvals.push(Approval {
        id: "c9".into(),
        question: "研究员交回了“任务v”".into(),
        options: vec![ApprovalOption {
            label: "通过".into(),
            consequence: String::new(),
        }],
        recommended: Some(0),
        reason: String::new(),
        subject: ApprovalSubject::TaskCompletion {
            agent_id: "a1".into(),
            task_id: "v".into(),
            thread_id: Some("th-v".into()),
        },
        state: ApprovalState::Pending,
        created_at_ms: NOW - 5_000,
    });
    let board = w.board();
    let a = &board.agents[0];
    assert_eq!(a.phase, ProgressPhase::WaitingHuman);
    let head = a.headline.as_ref().unwrap();
    assert_eq!((head.fraction, head.eta_ms), (Some(1.0), None));
    assert_eq!(kinds(&board), [AttentionKind::Review], "等验收的不算停着");
    let review = point(&board, AttentionKind::Review);
    assert_eq!(review.task_id.as_ref().unwrap().as_str(), "v");
    assert!(review.text.contains("等你验收：任务v"), "{}", review.text);

    // 做完要验收的任务，员工派的也要人看：预计交回时提醒。
    let mut w = World::new(history());
    let mut t = task("1", Some("a1"), TaskStatus::InProgress);
    t.created_by = Author::Agent {
        agent_id: "boss".into(),
    };
    t.needs_approval = true;
    w.tasks.push(t);
    w.asked.insert("th-1".into());
    w.run("a1", "th-1", NOW, 1);
    let board = w.board();
    assert!(point(&board, AttentionKind::Result).text.contains("等你验收"));
}
