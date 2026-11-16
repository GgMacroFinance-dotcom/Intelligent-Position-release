//! 性能基准。每个基准打印一行 JSON，`--check` 时未达标即以非零码退出，CI 用它拦性能回退。
//!
//! 目标值来自方案文档的"性能目标"表；新基准（并发 agent、搜索延迟等）按同样的格式加在这里。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use clap::{Args, ValueEnum};

mod agents;
mod events;
mod store;
use serde::Serialize;

/// 服务从启动到健康检查返回 200 的目标上限。
const TARGET_READY_MS: f64 = 100.0;
/// 服务空闲常驻内存的目标上限。
const TARGET_RSS_MB: f64 = 40.0;

#[derive(Args)]
pub struct BenchArgs {
    /// 要跑的基准。
    #[arg(value_enum, default_value = "startup")]
    which: Which,
    /// 启动测几次，取中位数。
    #[arg(long, default_value_t = 5)]
    runs: usize,
    /// 启动后等多少秒再读常驻内存。
    #[arg(long, default_value_t = 3)]
    settle_secs: u64,
    /// 未达标时以非零码退出。
    #[arg(long)]
    check: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum Which {
    /// 服务启动时间和空闲常驻内存。
    Startup,
    /// 5 万条消息的写入吞吐和全文搜索延迟。
    Store,
    /// 500 个常驻数字员工的内存和并发处理延迟。
    Agents,
    /// `/ws` 事件编号转发的吞吐和重连补发的耗时。
    Events,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupReport {
    benchmark: &'static str,
    ready_ms: Vec<f64>,
    median_ready_ms: f64,
    /// 只有 Linux 能读到（/proc），其他系统为 null。
    rss_mb: Option<f64>,
    target_ready_ms: f64,
    target_rss_mb: f64,
    pass: bool,
}

pub fn run(args: BenchArgs) -> anyhow::Result<()> {
    match args.which {
        Which::Startup => startup(&args),
        Which::Store => store::run(args.check),
        Which::Agents => agents::run(args.check),
        Which::Events => events::run(args.check),
    }
}

fn startup(args: &BenchArgs) -> anyhow::Result<()> {
    anyhow::ensure!(args.runs > 0, "--runs must be at least 1");
    let exe = std::env::current_exe()?;
    let mut ready_ms = Vec::with_capacity(args.runs);
    let mut rss_mb = None;
    for run in 0..args.runs {
        let started = Instant::now();
        let server = Server::spawn(&exe)?;
        server.wait_healthy(Duration::from_secs(10))?;
        ready_ms.push(round1(started.elapsed().as_secs_f64() * 1000.0));
        if run + 1 == args.runs {
            std::thread::sleep(Duration::from_secs(args.settle_secs));
            rss_mb = rss_mb_of(server.child.id());
        }
    }
    let median_ready_ms = median(&ready_ms);
    let pass = median_ready_ms <= TARGET_READY_MS && rss_mb.is_none_or(|rss| rss <= TARGET_RSS_MB);
    let report = StartupReport {
        benchmark: "startup",
        ready_ms,
        median_ready_ms,
        rss_mb,
        target_ready_ms: TARGET_READY_MS,
        target_rss_mb: TARGET_RSS_MB,
        pass,
    };
    println!("{}", serde_json::to_string(&report)?);
    if args.check && !pass {
        anyhow::bail!("startup benchmark missed its targets");
    }
    Ok(())
}

/// 一个用 `ip serve --addr 127.0.0.1:0` 起的子进程，离开作用域时结束。
struct Server {
    child: Child,
    addr: SocketAddr,
}

impl Server {
    fn spawn(exe: &std::path::Path) -> anyhow::Result<Self> {
        let mut child = Command::new(exe)
            .args(["serve", "--addr", "127.0.0.1:0"])
            .env("RUST_LOG", "warn")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("server stdout missing"))?;
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line)?;
        let addr = line
            .trim()
            .strip_prefix(ip_server::LISTENING_PREFIX)
            .ok_or_else(|| anyhow::anyhow!("unexpected first line from server: {line:?}"))?
            .parse()?;
        Ok(Self { child, addr })
    }

    fn wait_healthy(&self, timeout: Duration) -> anyhow::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if healthz_ok(self.addr) {
                return Ok(());
            }
            anyhow::ensure!(
                Instant::now() < deadline,
                "server at {} not healthy within {timeout:?}",
                self.addr
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn healthz_ok(addr: SocketAddr) -> bool {
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) else {
        return false;
    };
    let request = b"GET /healthz HTTP/1.1\r\nHost: bench\r\nConnection: close\r\n\r\n";
    let mut response = String::new();
    stream.write_all(request).is_ok()
        && stream.read_to_string(&mut response).is_ok()
        && response.starts_with("HTTP/1.1 200")
}

/// 读 /proc/<pid>/status 里的 VmRSS，单位 MB。
pub(crate) fn rss_mb_of(pid: u32) -> Option<f64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let kb: f64 = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()?;
    Some(round1(kb / 1024.0))
}

pub(crate) fn median(values: &[f64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

pub(crate) fn round1(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_picks_the_middle() {
        assert_eq!(median(&[5.0, 1.0, 3.0]), 3.0);
    }
}
