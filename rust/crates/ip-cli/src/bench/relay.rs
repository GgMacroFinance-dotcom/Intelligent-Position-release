//! 推送中转基准（`docs/ip/m2-spec.md` D-35，backlog F-21）：中转每收一条推送，要打开封条、读请求头、
//! 拼好给 APNs 或 FCM 的请求体。官方中转替所有用户转发，这一步在它的热路径上。不含网络往返。

use serde::Serialize;

use super::median;

const ROUNDS: usize = 2000;
/// 一条中位数的上限（毫秒）。网络往返是几十毫秒，这一步要小到可以忽略。
const TARGET_FORWARD_MS: f64 = 0.2;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    benchmark: &'static str,
    rounds: usize,
    forward_p50_ms: f64,
    forward_max_ms: f64,
    target_forward_ms: f64,
    pass: bool,
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let ms = ip_relay::time_forward(ROUNDS).map_err(anyhow::Error::msg)?;
    let round4 = |v: f64| (v * 10_000.0).round() / 10_000.0;
    let forward_p50_ms = round4(median(&ms));
    let report = Report {
        benchmark: "relay",
        rounds: ROUNDS,
        forward_p50_ms,
        forward_max_ms: round4(ms.iter().copied().fold(0.0, f64::max)),
        target_forward_ms: TARGET_FORWARD_MS,
        pass: forward_p50_ms <= TARGET_FORWARD_MS,
    };
    println!("{}", serde_json::to_string(&report)?);
    if check && !report.pass {
        anyhow::bail!("relay benchmark missed its target");
    }
    Ok(())
}
