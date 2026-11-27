//! 进度表基准（`docs/ip/progress-design.md`，backlog F-24，`docs/ip/m2-spec.md` D-42）：App 每收到员工的一步、
//! 一轮结束、任务或卡片变了就防抖重拉一次 `list_progress`，几台设备同时开着时每秒可能有好几次，算在热路径上。
//!
//! 量两样：
//! - 估算本身（纯函数）：500 位员工、2000 个没结束的任务、每个岗位 50 个做完的任务和 200 轮的历史；
//! - 从存储取数据再估（`Runtime::progress`）：50 位员工、200 个没结束的任务、2 万轮历史，基线第一次要现算，之后 60 秒内复用。
//!
//! 每一轮多写一行 `turns`、每一步更新一次内存里的状态，这部分算进 `ip bench agents` 的并发开销里。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use ip_core::{
    Agent, AgentId, Approval, ApprovalOption, ApprovalState, ApprovalSubject, Author, ModelSlot, Role, RoleId, Task,
    TaskStatus, ThreadId,
};
use ip_models::{ModelRegistry, ScriptedModel, ScriptedTurn};
use ip_runtime::progress::{Baseline, Inputs, LiveSnapshot, Running, TaskSample, estimate};
use ip_runtime::{AgentSetup, Runtime, RuntimeConfig};
use ip_store::{TurnRecord, TurnTotals};
use serde::Serialize;

use super::{median, round1};

const ROLES: usize = 10;
const AGENTS: usize = 500;
const OPEN_TASKS: usize = 2_000;
const ESTIMATE_ROUNDS: usize = 51;
/// 估算一张 500 位员工的表，中位数的上限。开发机上实测见 README。
const TARGET_ESTIMATE_MS: f64 = 10.0;

const DB_ROLES: usize = 5;
const DB_AGENTS: usize = 50;
const DB_OPEN_TASKS: usize = 200;
const DB_DONE_TASKS_PER_ROLE: usize = 50;
const DB_TURNS: usize = 20_000;
const DB_ROUNDS: usize = 40;
/// 基线已经算好时，从存储取数据再估一次的 p95 上限。
const TARGET_BOARD_P95_MS: f64 = 30.0;
/// 第一次（各岗位的基线都要现算）的上限。
const TARGET_COLD_MS: f64 = 200.0;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    benchmark: &'static str,
    agents: usize,
    open_tasks: usize,
    attention: usize,
    estimate_p50_ms: f64,
    estimate_max_ms: f64,
    target_estimate_ms: f64,
    db_agents: usize,
    db_open_tasks: usize,
    db_turns: usize,
    cold_ms: f64,
    board_p50_ms: f64,
    board_p95_ms: f64,
    target_board_p95_ms: f64,
    target_cold_ms: f64,
    pass: bool,
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let report = runtime.block_on(measure())?;
    println!("{}", serde_json::to_string(&report)?);
    if check && !report.pass {
        anyhow::bail!("progress benchmark missed its targets");
    }
    Ok(())
}

fn role(i: usize) -> Role {
    Role {
        id: format!("role{i}").into(),
        name: Some(format!("岗位{i}")),
        description: "基准用的岗位。".into(),
        model: ModelSlot {
            default: "fake/m".into(),
            escalate: None,
        },
        budget_per_day_usd: Some(50.0),
        coordinator: false,
    }
}

fn task(i: usize, agent: &Agent, status: TaskStatus, now: i64) -> Task {
    Task {
        id: format!("task{i}").into(),
        title: format!("整理第 {i} 份数据"),
        description: String::new(),
        status,
        role: Some(agent.role.clone()),
        assignee: Some(agent.id.clone()),
        thread_id: Some(format!("th{i}").into()),
        // 每 4 个任务里有一个是派活的员工在和人的对话里派出去的。
        origin_thread_id: (i % 4 == 0).then(|| format!("chat{}", i / 40).into()),
        result: None,
        depth: 1,
        created_by: if i % 4 == 0 {
            Author::Agent {
                agent_id: "agent0".into(),
            }
        } else {
            Author::human("张三")
        },
        created_at_ms: now - 600_000,
        updated_at_ms: now - 300_000,
        due_ms: (i % 5 == 0).then_some(now + 1_800_000),
    }
}

fn turn(agent: &Agent, thread: &str, started: i64, steps: u32) -> TurnRecord {
    TurnRecord {
        agent_id: agent.id.clone(),
        role_id: agent.role.clone(),
        thread_id: thread.into(),
        started_ms: started,
        finished_ms: started + 2_000 * i64::from(steps),
        steps,
        ok: steps % 7 != 0,
        cost_usd: 0.002 * f64::from(steps),
        error: None,
    }
}

fn question(i: usize, agent: &AgentId, thread: &ThreadId, at: i64) -> Approval {
    Approval {
        id: format!("q{i}").into(),
        question: "要同比还是环比？".into(),
        options: vec![
            ApprovalOption {
                label: "同比".into(),
                consequence: String::new(),
            },
            ApprovalOption {
                label: "环比".into(),
                consequence: String::new(),
            },
        ],
        recommended: Some(0),
        reason: String::new(),
        subject: ApprovalSubject::AgentQuestion {
            agent_id: agent.clone(),
            thread_id: thread.clone(),
        },
        state: ApprovalState::Pending,
        created_at_ms: at,
    }
}

fn baseline(seed: usize) -> Baseline {
    let tasks = (0..50)
        .map(|i| TaskSample {
            steps: 4.0 + ((i * 7 + seed) % 20) as f64,
            asked_at: (i % 3 == 0).then_some(3.0 + (i % 5) as f64),
        })
        .collect();
    let agent = Agent {
        id: "x".into(),
        role: "r".into(),
        name: "x".into(),
    };
    let turns: Vec<TurnRecord> = (0..200)
        .map(|i| turn(&agent, "old", i as i64 * 100_000, 1 + (i % 9) as u32))
        .collect();
    Baseline::new(tasks, &turns)
}

async fn measure() -> anyhow::Result<Report> {
    let now = ip_core::now_ms();

    // 1. 只量估算。
    let roles: HashMap<RoleId, Role> = (0..ROLES).map(|i| (role(i).id.clone(), role(i))).collect();
    let agents: Vec<Agent> = (0..AGENTS)
        .map(|i| Agent {
            id: format!("agent{i}").into(),
            role: format!("role{}", i % ROLES).into(),
            name: format!("员工{i}"),
        })
        .collect();
    let statuses = [
        TaskStatus::InProgress,
        TaskStatus::Open,
        TaskStatus::Blocked,
        TaskStatus::InProgress,
    ];
    let tasks: Vec<Task> = (0..OPEN_TASKS)
        .map(|i| task(i, &agents[i % AGENTS], statuses[(i / AGENTS) % statuses.len()], now))
        .collect();
    let mut live = LiveSnapshot::default();
    let mut totals = HashMap::new();
    for (i, t) in tasks.iter().enumerate() {
        let thread = t.thread_id.clone().unwrap_or_default();
        totals.insert(
            thread.clone(),
            TurnTotals {
                turns: 2,
                steps: (i % 11) as u32,
                active_ms: 20_000,
                last_finished_ms: now - 60_000,
            },
        );
        let agent = t.assignee.clone().unwrap_or_else(|| "agent0".into());
        match i / AGENTS {
            0 if i % 3 != 0 => {
                live.running.insert(
                    agent,
                    Running {
                        thread_id: thread,
                        started_ms: now - 5_000,
                        step: 1 + (i % 6) as u32,
                    },
                );
            }
            1 => live.queued.entry(agent).or_default().push(thread),
            _ => {}
        }
    }
    let approvals: Vec<Approval> = tasks
        .iter()
        .step_by(20)
        .enumerate()
        .map(|(i, t)| {
            question(
                i,
                t.assignee.as_ref().unwrap_or(&"agent0".into()),
                t.thread_id.as_ref().unwrap_or(&ThreadId::new()),
                now - 10_000,
            )
        })
        .collect();
    let asked: HashSet<ThreadId> = approvals
        .iter()
        .filter_map(|a| match &a.subject {
            ApprovalSubject::AgentQuestion { thread_id, .. } => Some(thread_id.clone()),
            _ => None,
        })
        .collect();
    let baselines: HashMap<RoleId, Arc<Baseline>> = roles
        .keys()
        .enumerate()
        .map(|(i, r)| (r.clone(), Arc::new(baseline(i))))
        .collect();
    let spent_today: HashMap<RoleId, f64> = roles.keys().map(|r| (r.clone(), 20.0)).collect();
    let spent_recent: HashMap<RoleId, f64> = roles.keys().map(|r| (r.clone(), 3.0)).collect();
    let last_turns = HashMap::new();
    let inputs = Inputs {
        now_ms: now,
        max_steps: 16,
        day_end_ms: (now.div_euclid(86_400_000) + 1) * 86_400_000,
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
    };
    let mut samples = Vec::with_capacity(ESTIMATE_ROUNDS);
    let mut attention = 0;
    for _ in 0..ESTIMATE_ROUNDS {
        let started = Instant::now();
        let board = std::hint::black_box(estimate(&inputs));
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        attention = board.attention.len();
    }
    let estimate_p50_ms = median(&samples);
    let estimate_max_ms = samples.iter().copied().fold(0.0, f64::max);

    // 2. 从存储取数据再估。
    let store = ip_store::testing::TestDb::new().open().await;
    let mut models = ModelRegistry::new();
    models.insert("fake", Arc::new(ScriptedModel::new([ScriptedTurn::Text("好".into())])));
    let runtime = Runtime::new(store.clone(), models, RuntimeConfig::default());
    let db_agents: Vec<Agent> = (0..DB_AGENTS)
        .map(|i| Agent {
            id: format!("agent{i}").into(),
            role: format!("role{}", i % DB_ROLES).into(),
            name: format!("员工{i}"),
        })
        .collect();
    for a in &db_agents {
        let i: usize = a.role.as_str().trim_start_matches("role").parse()?;
        runtime.spawn_agent(AgentSetup {
            agent: a.clone(),
            role: role(i),
            toolbox: None,
            mcp: None,
            data: None,
            engine: None,
        })?;
    }
    // 做完的任务和它们的轮次（基线用），再加大量别的轮次（表里的历史量）。
    let mut writes = Vec::new();
    let mut n = 0;
    for (r, a) in db_agents.iter().take(DB_ROLES).enumerate() {
        for k in 0..DB_DONE_TASKS_PER_ROLE {
            n += 1;
            let mut t = task(100_000 + n, a, TaskStatus::Done, now);
            t.origin_thread_id = None;
            let thread = t.thread_id.clone().unwrap_or_default();
            let start = now - 86_400_000 + (k as i64) * 60_000;
            t.created_at_ms = start;
            t.updated_at_ms = start + 50_000;
            for j in 0..4 {
                writes.push(turn(
                    a,
                    thread.as_str(),
                    start + j * 10_000,
                    1 + ((k + j as usize + r) % 4) as u32,
                ));
            }
            store.put_task(t).await?;
        }
    }
    for i in 0..DB_TURNS {
        let a = &db_agents[i % DB_AGENTS];
        writes.push(turn(
            a,
            &format!("hist{}", i / 5),
            now - 3 * 86_400_000 + i as i64 * 1_000,
            1 + (i % 8) as u32,
        ));
    }
    for (i, a) in db_agents.iter().cycle().take(DB_OPEN_TASKS).enumerate() {
        let t = task(i, a, statuses[i % statuses.len()], now);
        let thread = t.thread_id.clone().unwrap_or_default();
        writes.push(turn(a, thread.as_str(), now - 200_000, 3));
        if i % 10 == 0 {
            store.put_approval(question(i, &a.id, &thread, now - 20_000)).await?;
        }
        store.put_task(t).await?;
    }
    let db_turns = writes.len();
    let handles: Vec<_> = writes
        .into_iter()
        .map(|t| {
            let store = store.clone();
            tokio::spawn(async move { store.record_turn(t).await })
        })
        .collect();
    for h in handles {
        h.await??;
    }
    store.analyze().await?;

    let started = Instant::now();
    std::hint::black_box(runtime.progress(now).await?);
    let cold_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut board = Vec::with_capacity(DB_ROUNDS);
    for _ in 0..DB_ROUNDS {
        let started = Instant::now();
        std::hint::black_box(runtime.progress(now).await?);
        board.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    board.sort_by(f64::total_cmp);
    let board_p50_ms = median(&board);
    let board_p95_ms = board[(board.len() * 95 / 100).min(board.len() - 1)];
    runtime.shutdown().await;

    let pass =
        estimate_p50_ms <= TARGET_ESTIMATE_MS && board_p95_ms <= TARGET_BOARD_P95_MS && cold_ms <= TARGET_COLD_MS;
    Ok(Report {
        benchmark: "progress",
        agents: AGENTS,
        open_tasks: OPEN_TASKS,
        attention,
        estimate_p50_ms: round1(estimate_p50_ms),
        estimate_max_ms: round1(estimate_max_ms),
        target_estimate_ms: TARGET_ESTIMATE_MS,
        db_agents: DB_AGENTS,
        db_open_tasks: DB_OPEN_TASKS,
        db_turns,
        cold_ms: round1(cold_ms),
        board_p50_ms: round1(board_p50_ms),
        board_p95_ms: round1(board_p95_ms),
        target_board_p95_ms: TARGET_BOARD_P95_MS,
        target_cold_ms: TARGET_COLD_MS,
        pass,
    })
}
