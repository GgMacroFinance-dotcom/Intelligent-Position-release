//! 通知规则基准（`docs/ip/m2-spec.md` D-32，backlog F-9）：运行时的每个事件都要过一遍通知规则，
//! 流式输出时每来一段文字就是一个事件，所以它在热路径上。另外量一次预览（脱敏、去掉 Markdown、取头两句）的耗时，
//! 和加密、签名一条 Web Push（F-10）的耗时。

use std::time::Instant;

use ip_core::{Agent, AgentId, Author, Body, Message, RoleId};
use ip_models::Usage;
use ip_runtime::RuntimeEvent;
use ip_server::notify::{Rules, preview};
use serde::Serialize;

use super::{median, round1};

/// 推多少个事件。
const EVENTS: u32 = 200_000;
/// 每一轮流式输出多少段文字；一轮结束时有一条回复要做预览。
const DELTAS_PER_TURN: u32 = 50;
const PREVIEW_ROUNDS: usize = 200;
/// 过规则的吞吐下限（个/秒）。和 `ip bench events` 同一个量级要求：500 位员工同时输出也就每秒几万段。
/// 开发机上实测见 README。
const TARGET_EVENTS_PER_SEC: f64 = 100_000.0;
/// 一次预览（约 4 KB 的 Markdown 报告）中位数的上限。
const TARGET_PREVIEW_MS: f64 = 1.0;
/// 加密并签名一条推送中位数的上限。一条通知要推给每个订阅各一次。
const TARGET_PUSH_MS: f64 = 5.0;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    benchmark: &'static str,
    events: u32,
    alerts: u32,
    observe_ms: f64,
    events_per_sec: f64,
    preview_bytes: usize,
    preview_p50_ms: f64,
    preview_max_ms: f64,
    push_p50_ms: f64,
    push_max_ms: f64,
    target_events_per_sec: f64,
    target_preview_ms: f64,
    target_push_ms: f64,
    pass: bool,
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let report = measure()?;
    println!("{}", serde_json::to_string(&report)?);
    if check && !report.pass {
        anyhow::bail!("notify benchmark missed its targets");
    }
    Ok(())
}

fn measure() -> anyhow::Result<Report> {
    let reply = report_text();
    let agent = |id: &AgentId| {
        Some(Agent {
            id: id.clone(),
            role: RoleId::from("macro"),
            name: "宏观研究员".into(),
        })
    };
    let mut rules = Rules::default();
    let mut alerts = 0;
    let started = Instant::now();
    for i in 0..EVENTS {
        let event = event(i, &reply);
        if rules.observe(&event, i64::from(i), agent).is_some() {
            alerts += 1;
        }
    }
    let observe_ms = started.elapsed().as_secs_f64() * 1000.0;

    let mut preview_ms = Vec::with_capacity(PREVIEW_ROUNDS);
    for _ in 0..PREVIEW_ROUNDS {
        let started = Instant::now();
        std::hint::black_box(preview(std::hint::black_box(&reply)));
        preview_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }

    let payload = br#"{"title":"\u5b8f\u89c2\u7814\u7a76\u5458 \u56de\u590d\u4e86","body":"10 \u6708 CPI \u540c\u6bd4 2.9%","tag":"thread:01JD8Z6Q4M3K2N1P0R9S8T7V6W","thread_id":"01JD8Z6Q4M3K2N1P0R9S8T7V6W"}"#;
    let push_ms = ip_server::webpush::time_pushes(payload, PREVIEW_ROUNDS).map_err(anyhow::Error::msg)?;

    let events_per_sec = (f64::from(EVENTS) / (observe_ms / 1000.0)).round();
    let preview_p50_ms = round3(median(&preview_ms));
    let preview_max_ms = round3(preview_ms.iter().copied().fold(0.0, f64::max));
    let turns = EVENTS / (DELTAS_PER_TURN + 3);
    let push_p50_ms = round3(median(&push_ms));
    let push_max_ms = round3(push_ms.iter().copied().fold(0.0, f64::max));
    let pass = events_per_sec >= TARGET_EVENTS_PER_SEC
        && preview_p50_ms <= TARGET_PREVIEW_MS
        && alerts >= turns
        && push_p50_ms <= TARGET_PUSH_MS;
    Ok(Report {
        benchmark: "notify",
        events: EVENTS,
        alerts,
        observe_ms: round1(observe_ms),
        events_per_sec,
        preview_bytes: reply.len(),
        preview_p50_ms,
        preview_max_ms,
        push_p50_ms,
        push_max_ms,
        target_events_per_sec: TARGET_EVENTS_PER_SEC,
        target_preview_ms: TARGET_PREVIEW_MS,
        target_push_ms: TARGET_PUSH_MS,
        pass,
    })
}

/// 一轮：开始、`DELTAS_PER_TURN` 段文字、整条回复、结束。事件本身在计时里构造，和真实转发时一样要克隆文字。
fn event(i: u32, reply: &str) -> RuntimeEvent {
    let thread = format!("thread-{}", i / (DELTAS_PER_TURN + 3) % 64);
    let agent_id = AgentId::from("macro-1");
    match i % (DELTAS_PER_TURN + 3) {
        0 => RuntimeEvent::TurnStarted {
            agent_id,
            thread_id: thread.into(),
        },
        n if n == DELTAS_PER_TURN + 1 => RuntimeEvent::Message {
            message: Message {
                id: format!("m{i}").into(),
                thread_id: thread.into(),
                at_ms: i64::from(i),
                author: Author::Agent { agent_id },
                body: Body::Text {
                    text: reply.to_string(),
                },
                notice: None,
            },
        },
        n if n == DELTAS_PER_TURN + 2 => RuntimeEvent::TurnFinished {
            agent_id,
            thread_id: thread.into(),
            steps: 3,
            usage: Usage::default(),
            cost_usd: 0.0,
            elapsed_ms: 1,
        },
        _ => RuntimeEvent::Delta {
            agent_id,
            thread_id: thread.into(),
            text: format!("10 月 CPI 同比 2.9%，第 {i} 段"),
        },
    }
}

/// 约 4 KB、带表格和代码块的报告，像员工交回的研报摘要。
fn report_text() -> String {
    let mut text = String::from(
        "## 10 月物价点评\n\n**结论**：CPI 同比 2.9%，比上月回落 0.2 个百分点。核心通胀继续放缓，api_key=sk-abcdefghijklmnopqrstuvwxyz 不该出现。\n\n",
    );
    text.push_str("| 指标 | 10 月 | 9 月 |\n| --- | --- | --- |\n");
    for n in 0..40 {
        text.push_str(&format!("| 分项 {n} | 2.{n}% | 3.{n}% |\n"));
    }
    text.push_str("\n```python\nimport pandas as pd\nprint(pd.read_csv('cpi.csv').tail())\n```\n\n");
    while text.len() < 4096 {
        text.push_str("- 食品价格同比下降，能源价格受国际油价影响小幅上行，见 [报告](reports/cpi-10.md)。\n");
    }
    text
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}
