//! 事件转发基准（`docs/ip/m2-spec.md` D-29，backlog F-6）：所有运行时事件都要经过编号、序列化、
//! 留存、广播这一步，流式输出时每来一段文字就是一个事件，所以它在热路径上。另外量重连时补发
//! 留满的 4096 个事件要多久。

use std::time::Instant;

use ip_runtime::RuntimeEvent;
use ip_server::events::{CHANNEL_CAPACITY, EventLog, KEEP_BYTES, KEEP_EVENTS};
use serde::Serialize;

use super::{median, round1};

/// 推多少个事件。
const EVENTS: u32 = 200_000;
/// 模拟几个连接在收听。
const LISTENERS: usize = 8;
const REPLAY_ROUNDS: usize = 50;
/// 编号转发的吞吐下限（个/秒）。一位员工流式输出每秒几十段文字，500 位员工同时输出也就几万段。
/// 开发机上实测约 230 万个/秒（8 个连接收听），留了足够余量给 CI 的机器。
const TARGET_EVENTS_PER_SEC: f64 = 100_000.0;
/// 补发留满的 4096 个事件（取出要发的帧）中位数的上限。开发机上实测约 0.1 毫秒。
const TARGET_REPLAY_MS: f64 = 5.0;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    benchmark: &'static str,
    events: u32,
    listeners: usize,
    push_ms: f64,
    events_per_sec: f64,
    avg_frame_bytes: f64,
    kept_events: usize,
    replay_p50_ms: f64,
    replay_max_ms: f64,
    target_events_per_sec: f64,
    target_replay_ms: f64,
    pass: bool,
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let report = measure();
    println!("{}", serde_json::to_string(&report)?);
    if check && !report.pass {
        anyhow::bail!("events benchmark missed its targets");
    }
    Ok(())
}

fn measure() -> Report {
    let log = EventLog::new(KEEP_EVENTS, KEEP_BYTES, CHANNEL_CAPACITY);
    // 收听的连接不取，广播通道满了就覆盖最旧的，和跟不上的连接一样。
    let listeners: Vec<_> = (0..LISTENERS).map(|_| log.subscribe()).collect();

    let started = Instant::now();
    for i in 0..EVENTS {
        log.push(delta(i));
    }
    let push_ms = started.elapsed().as_secs_f64() * 1000.0;
    drop(listeners);

    let latest = log.latest();
    let since = latest.saturating_sub(KEEP_EVENTS as u64);
    let mut replay_ms = Vec::with_capacity(REPLAY_ROUNDS);
    let mut kept = Vec::new();
    for _ in 0..REPLAY_ROUNDS {
        let started = Instant::now();
        kept = log.since(since).unwrap_or_default();
        replay_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let avg_frame_bytes = if kept.is_empty() {
        0.0
    } else {
        kept.iter().map(|s| s.frame.len()).sum::<usize>() as f64 / kept.len() as f64
    };

    let events_per_sec = (f64::from(EVENTS) / (push_ms / 1000.0)).round();
    let replay_p50_ms = round3(median(&replay_ms));
    let replay_max_ms = round3(replay_ms.iter().copied().fold(0.0, f64::max));
    let pass =
        events_per_sec >= TARGET_EVENTS_PER_SEC && replay_p50_ms <= TARGET_REPLAY_MS && kept.len() == KEEP_EVENTS;
    Report {
        benchmark: "events",
        events: EVENTS,
        listeners: LISTENERS,
        push_ms: round1(push_ms),
        events_per_sec,
        avg_frame_bytes: round1(avg_frame_bytes),
        kept_events: kept.len(),
        replay_p50_ms,
        replay_max_ms,
        target_events_per_sec: TARGET_EVENTS_PER_SEC,
        target_replay_ms: TARGET_REPLAY_MS,
        pass,
    }
}

/// 和真实流式输出差不多大小的一段文字。
fn delta(i: u32) -> RuntimeEvent {
    RuntimeEvent::Delta {
        agent_id: "macro-1".into(),
        thread_id: "01JD8Z6Q4M3K2N1P0R9S8T7V6W".into(),
        text: format!("10 月 CPI 同比 2.9%，第 {i} 段"),
    }
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
