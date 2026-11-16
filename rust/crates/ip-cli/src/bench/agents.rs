//! 并发数字员工基准：常驻 500 个员工，测每个空闲员工占多少内存，
//! 再同时给每个员工发几条消息，测运行时自身带来的延迟（模型用按固定速度吐字的假模型）。

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ip_core::{Agent, Author, ModelSlot, Role, ThreadId};
use ip_models::{ModelRegistry, ScriptedModel, ScriptedTurn};
use ip_runtime::{AgentSetup, Runtime, RuntimeConfig, RuntimeEvent};
use ip_store::Store;
use serde::Serialize;
use tokio::sync::broadcast::error::RecvError;

use super::{median, round1, rss_mb_of};

const AGENTS: usize = 500;
/// 每个员工收到的消息数（各在一个新对话里）。
const ROUNDS: usize = 3;
/// 假模型每 4 个字符一片，片间隔 2 毫秒；200 字的回复约 100 毫秒，接近真实模型的吐字速度。
const REPLY_CHARS: usize = 200;
const CHUNK_DELAY: Duration = Duration::from_millis(2);
/// 每个空闲员工的内存目标上限（KB）。方案文档的目标是 < 1 MB，这里收紧到 64 KB 防回退。
const TARGET_IDLE_KB_PER_AGENT: f64 = 64.0;
/// 500 个员工同时工作时，一轮比单独一轮多花时间的 p95 上限。
const TARGET_OVERHEAD_P95_MS: f64 = 80.0;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    benchmark: &'static str,
    agents: usize,
    turns: usize,
    /// 单独一轮的耗时（基线）。
    solo_turn_ms: f64,
    idle_kb_per_agent: Option<f64>,
    wall_ms: f64,
    turns_per_sec: f64,
    turn_p50_ms: f64,
    turn_p95_ms: f64,
    turn_max_ms: f64,
    /// 并发时一轮比单独一轮多花的时间（p95），即运行时和存储在 500 个员工同时工作时的额外开销。
    overhead_p95_ms: f64,
    /// 从发出消息到这一轮完成，包含排队（每个员工依次处理自己的 3 条消息）。
    end_to_end_p95_ms: f64,
    rss_mb: Option<f64>,
    target_idle_kb_per_agent: f64,
    target_overhead_p95_ms: f64,
    pass: bool,
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let report = runtime.block_on(measure())?;
    println!("{}", serde_json::to_string(&report)?);
    if check && !report.pass {
        anyhow::bail!("agents benchmark missed its targets");
    }
    Ok(())
}

async fn measure() -> anyhow::Result<Report> {
    let dir = std::env::temp_dir().join(format!("ip-bench-agents-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let store = Store::open(dir.join("store.db"))?;

    let reply: String = "宏观数据点评：本月通胀温和回升，"
        .chars()
        .cycle()
        .take(REPLY_CHARS)
        .collect();
    let model = ScriptedModel::new([ScriptedTurn::Text(reply)]).with_delay(CHUNK_DELAY);
    let mut models = ModelRegistry::new();
    models.insert("fake", Arc::new(model));
    let config = RuntimeConfig {
        event_capacity: 1 << 17,
        ..RuntimeConfig::default()
    };
    let runtime = Runtime::new(store, models, config);
    let role = Role {
        id: "macro".into(),
        name: Some("宏观研究员".into()),
        description: "撰写宏观研究报告。".into(),
        model: ModelSlot {
            default: "fake/m".into(),
            escalate: None,
        },
        budget_per_day_usd: None,
    };

    // 先热身一个员工，让分配器、存储连接池、tokio 线程都就绪，避免把一次性开销算到每个员工头上。
    let mut events = runtime.subscribe();
    spawn(&runtime, &role, "warmup")?;
    // 单独跑一轮作为基线：没有其他员工竞争时一轮要多久（主要是假模型的吐字时间，含定时器精度）。
    let mut solo_ms = f64::MAX;
    for _ in 0..3 {
        runtime.send(&"warmup".into(), ThreadId::new(), human(), "热身").await?;
        solo_ms = solo_ms.min(wait_finished(&mut events, 1).await?[0].1);
    }

    tokio::time::sleep(Duration::from_millis(200)).await;
    let rss_before = rss_mb_of(std::process::id());
    for i in 0..AGENTS {
        spawn(&runtime, &role, &format!("a{i}"))?;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let rss_after = rss_mb_of(std::process::id());
    let idle_kb_per_agent = match (rss_before, rss_after) {
        (Some(b), Some(a)) => Some(round1((a - b).max(0.0) * 1024.0 / AGENTS as f64)),
        _ => None,
    };

    let started = Instant::now();
    let mut sent_at: HashMap<ThreadId, Instant> = HashMap::with_capacity(AGENTS * ROUNDS);
    for _ in 0..ROUNDS {
        for i in 0..AGENTS {
            let thread = ThreadId::new();
            sent_at.insert(thread.clone(), Instant::now());
            runtime
                .send(&format!("a{i}").into(), thread, human(), "点评本月 CPI")
                .await?;
        }
    }
    let finished = wait_finished(&mut events, AGENTS * ROUNDS).await?;
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;

    // 一轮耗时：员工开始处理到处理完（不含在收件箱里排队的时间）；排队另算，含在“发出到完成”里。
    let mut turn_ms: Vec<f64> = finished.iter().map(|(_, ms, _)| *ms).collect();
    let mut e2e_ms: Vec<f64> = finished
        .iter()
        .filter_map(|(thread, _, end)| {
            sent_at
                .get(thread)
                .map(|s| end.duration_since(*s).as_secs_f64() * 1000.0)
        })
        .collect();
    anyhow::ensure!(
        e2e_ms.len() == AGENTS * ROUNDS,
        "有 {} 轮没有完成",
        AGENTS * ROUNDS - e2e_ms.len()
    );
    turn_ms.sort_by(f64::total_cmp);
    e2e_ms.sort_by(f64::total_cmp);
    let p95 = percentile95(&turn_ms);
    let max = *turn_ms.last().unwrap_or(&0.0);
    let rss_mb = rss_mb_of(std::process::id());
    runtime.shutdown().await;
    let _ = std::fs::remove_dir_all(&dir);

    let overhead_p95 = (p95 - solo_ms).max(0.0);
    let pass =
        idle_kb_per_agent.is_none_or(|kb| kb <= TARGET_IDLE_KB_PER_AGENT) && overhead_p95 <= TARGET_OVERHEAD_P95_MS;
    Ok(Report {
        benchmark: "agents",
        agents: AGENTS,
        turns: AGENTS * ROUNDS,
        solo_turn_ms: round1(solo_ms),
        idle_kb_per_agent,
        wall_ms: round1(wall_ms),
        turns_per_sec: ((AGENTS * ROUNDS) as f64 / (wall_ms / 1000.0)).round(),
        turn_p50_ms: round1(median(&turn_ms)),
        turn_p95_ms: round1(p95),
        turn_max_ms: round1(max),
        overhead_p95_ms: round1(overhead_p95),
        end_to_end_p95_ms: round1(percentile95(&e2e_ms)),
        rss_mb,
        target_idle_kb_per_agent: TARGET_IDLE_KB_PER_AGENT,
        target_overhead_p95_ms: TARGET_OVERHEAD_P95_MS,
        pass,
    })
}

fn percentile95(sorted: &[f64]) -> f64 {
    sorted[(sorted.len() * 95).div_ceil(100).saturating_sub(1)]
}

fn spawn(runtime: &Runtime, role: &Role, id: &str) -> anyhow::Result<()> {
    let agent = Agent {
        id: id.into(),
        role: role.id.clone(),
        name: id.into(),
    };
    runtime.spawn_agent(AgentSetup {
        agent,
        role: role.clone(),
        toolbox: None,
        mcp: None,
        data: None,
        engine: None,
    })?;
    Ok(())
}

fn human() -> Author {
    Author::human("bench")
}

/// 等 `n` 轮结束，返回每轮的对话、处理耗时（毫秒）和收到结束事件的时刻。事件丢失（订阅跟不上）视为失败。
async fn wait_finished(
    events: &mut tokio::sync::broadcast::Receiver<RuntimeEvent>,
    n: usize,
) -> anyhow::Result<Vec<(ThreadId, f64, Instant)>> {
    let mut done = Vec::with_capacity(n);
    while done.len() < n {
        match tokio::time::timeout(Duration::from_secs(60), events.recv()).await {
            Err(_) => anyhow::bail!("60 秒内只完成了 {}/{n} 轮", done.len()),
            Ok(Err(RecvError::Lagged(k))) => anyhow::bail!("事件订阅落后，丢了 {k} 条"),
            Ok(Err(RecvError::Closed)) => anyhow::bail!("运行时已关闭"),
            Ok(Ok(RuntimeEvent::TurnFinished {
                thread_id, elapsed_ms, ..
            })) => {
                done.push((thread_id, elapsed_ms as f64, Instant::now()));
            }
            Ok(Ok(RuntimeEvent::TurnFailed { error, .. })) => anyhow::bail!("一轮失败：{error}"),
            Ok(Ok(_)) => {}
        }
    }
    Ok(done)
}
