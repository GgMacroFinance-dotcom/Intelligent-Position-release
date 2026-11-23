//! 团队上下文基准（`docs/ip/team-context.md`）：接了团队上下文的岗位每轮开头经 MCP 取一次上下文包，
//! 这一步在每一轮的热路径上。这里量 I.P. 这一侧的开销：MCP 客户端发请求、收一个约 1 万字的包、解析 JSON。
//! 服务端用内存管道上的假服务，不含 TCMS 编包的时间（那部分在 TCMS 自己的 `tcms bench serve` 里量）。

use std::sync::Arc;
use std::time::{Duration, Instant};

use ip_mcp::McpHub;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig as ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};
use serde::Serialize;
use serde_json::{Value, json};

use super::median;

const WARMUP: usize = 50;
const ROUNDS: usize = 1000;
/// p95 的上限（毫秒）。一轮里调模型要几秒，这一步要小到可以忽略。
const TARGET_P95_MS: f64 = 5.0;

/// 假的团队上下文服务：`context_get` 每次返回同一个包（前缀约 1700 字，按任务挑的约 8000 字）。
#[derive(Clone)]
struct FakeTcms(Arc<Value>);

impl ServerHandler for FakeTcms {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let Value::Object(schema) = json!({"type": "object"}) else {
            unreachable!()
        };
        Ok(ListToolsResult::with_all_items(vec![Tool::new(
            "context_get",
            "取上下文包",
            Arc::new(schema),
        )]))
    }

    async fn call_tool(
        &self,
        _request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        Ok(CallToolResult::structured((*self.0).clone()).into())
    }
}

fn pack() -> Value {
    let item = |i: usize, len: usize| {
        format!(
            "[ctx_01JDX4M6Q8{i:016} r3 · fact · role:macro]\n### 第 {i} 条\n\n{}\n[/ctx_01JDX4M6Q8{i:016}]\n",
            "宏观数据口径说明，引用时写明来源和日期。".repeat(len)
        )
    };
    let prefix: String = (0..5).map(|i| item(i, 12)).collect();
    let task: String = (5..25).map(|i| item(i, 15)).collect();
    json!({"prefix": prefix, "task": task, "cursor": 42, "warnings": []})
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    benchmark: &'static str,
    rounds: usize,
    pack_chars: usize,
    fetch_p50_ms: f64,
    fetch_p95_ms: f64,
    target_p95_ms: f64,
    pass: bool,
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let report = rt.block_on(measure())?;
    println!("{}", serde_json::to_string(&report)?);
    if check && !report.pass {
        anyhow::bail!("team benchmark missed its target");
    }
    Ok(())
}

async fn measure() -> anyhow::Result<Report> {
    let pack = Arc::new(pack());
    let pack_chars = ["prefix", "task"]
        .iter()
        .map(|k| pack[*k].as_str().unwrap_or_default().chars().count())
        .sum();
    let (client, server) = tokio::io::duplex(1 << 20);
    let handler = FakeTcms(pack);
    tokio::spawn(async move {
        if let Ok(running) = handler.serve(server).await {
            let _ = running.waiting().await;
        }
    });
    let hub = McpHub::default();
    hub.add("tcms", client, vec!["macro".into()], Duration::from_secs(10))
        .await
        .map_err(anyhow::Error::msg)?;
    let args = || {
        let Value::Object(m) = json!({"role": "macro", "task": "写一份 CPI 点评", "budget_tokens": 4000}) else {
            unreachable!()
        };
        m
    };
    let mut ms = Vec::with_capacity(ROUNDS);
    for i in 0..WARMUP + ROUNDS {
        let started = Instant::now();
        let got = hub
            .call_json("tcms", "context_get", args(), Duration::from_secs(10))
            .await
            .map_err(anyhow::Error::msg)?;
        anyhow::ensure!(got["prefix"].is_string(), "包里没有 prefix");
        if i >= WARMUP {
            ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
    }
    ms.sort_by(f64::total_cmp);
    let p95 = ms[(ms.len() * 95 / 100).min(ms.len() - 1)];
    let round3 = |v: f64| (v * 1000.0).round() / 1000.0;
    hub.shutdown().await;
    Ok(Report {
        benchmark: "team",
        rounds: ROUNDS,
        pack_chars,
        fetch_p50_ms: round3(median(&ms)),
        fetch_p95_ms: round3(p95),
        target_p95_ms: TARGET_P95_MS,
        pass: p95 <= TARGET_P95_MS,
    })
}
