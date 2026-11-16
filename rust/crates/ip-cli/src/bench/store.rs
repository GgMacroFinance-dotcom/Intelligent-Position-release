//! 存储基准：写入 5 万条中英文消息，然后反复跑一组搜索；再量每轮对话都要做的“读岗位上下文”。

use std::time::Instant;

use ip_core::{Author, Message, ThreadId};
use ip_store::Store;
use serde::Serialize;

use super::{median, round1, rss_mb_of};

const MESSAGES: usize = 50_000;
const THREADS: usize = 300;
const PRODUCERS: usize = 64;
const SEARCH_ROUNDS: usize = 20;
/// 搜索 p95 的目标上限，来自方案文档的性能目标表。
const TARGET_SEARCH_P95_MS: f64 = 10.0;
const CONTEXT_READS: usize = 2_000;
/// 每轮对话开头读一次决策者笔记和岗位记忆摘要（各放满上限），p95 的目标上限。
/// 一轮对话至少有一次模型调用（秒级），5 毫秒以内可以忽略。开发机上实测 p50 约 0.3 毫秒，p95 约 1 毫秒。
const TARGET_CONTEXT_READ_P95_MS: f64 = 5.0;

const SENTENCES: &[&str] = &[
    "请帮我整理一下本周的销售报表，周五前发给财务",
    "客户反馈合同第三条的付款周期需要调整",
    "招聘岗位的简历已经筛选完毕，约了五位候选人面试",
    "供应商发票金额和采购订单对不上，需要核对",
    "Please send the quarterly report to the finance team",
    "The deployment failed because the database migration timed out",
    "会议纪要：确定下季度预算，市场部增加两名人员",
    "报销单据缺少发票原件，退回申请人补充",
    "Customer asked for an invoice copy and the signed contract",
    "库存盘点发现三个仓库的数据不一致",
];

const QUERIES: &[&str] = &[
    "销售报表",
    "合同",
    "发票 核对",
    "report",
    "invoice contract",
    "面试",
    "预算",
    "盘",
    "不存在的词组",
];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    benchmark: &'static str,
    messages: usize,
    insert_ms: f64,
    inserts_per_sec: f64,
    db_mb: f64,
    searches: usize,
    search_p50_ms: f64,
    search_p95_ms: f64,
    search_max_ms: f64,
    slowest_query: String,
    rss_mb: Option<f64>,
    target_search_p95_ms: f64,
    memory_scan_p95_ms: f64,
    /// `memory_search` 在一个岗位 2000 份对话记忆里找一次。
    memory_search_p95_ms: f64,
    context_reads: usize,
    context_read_p50_ms: f64,
    context_read_p95_ms: f64,
    target_context_read_p95_ms: f64,
    pass: bool,
}

pub fn run(check: bool) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let report = runtime.block_on(measure())?;
    println!("{}", serde_json::to_string(&report)?);
    if check && !report.pass {
        anyhow::bail!("store benchmark missed its targets");
    }
    Ok(())
}

async fn measure() -> anyhow::Result<Report> {
    let dir = std::env::temp_dir().join(format!("ip-bench-store-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("store.db");
    let store = Store::open(&path)?;

    let started = Instant::now();
    let producers: Vec<_> = (0..PRODUCERS)
        .map(|producer| {
            let store = store.clone();
            tokio::spawn(async move {
                let mut i = producer;
                while i < MESSAGES {
                    store.append_message(synthetic(i)).await?;
                    i += PRODUCERS;
                }
                anyhow::Ok(())
            })
        })
        .collect();
    for producer in producers {
        producer.await??;
    }
    let insert_ms = started.elapsed().as_secs_f64() * 1000.0;

    let mut samples = Vec::new();
    let mut slowest = (0.0, String::new());
    for _ in 0..SEARCH_ROUNDS {
        for query in QUERIES {
            let started = Instant::now();
            store.search(query, None, 20).await?;
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            if ms > slowest.0 {
                slowest = (ms, (*query).to_owned());
            }
            samples.push(ms);
        }
    }
    let p95 = p95_of(&samples);
    // 记忆提炼每轮先找“凉下来”的对话：在这个库上取最多 1000 个。
    let mut scan_samples = Vec::new();
    for _ in 0..200 {
        let started = Instant::now();
        store.threads_last_active_between(0, i64::MAX, 1_000).await?;
        scan_samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let memory_search_samples = memory_searches(&store).await?;
    let context_samples = context_reads(&store).await?;
    let context_p95 = p95_of(&context_samples);
    let db_bytes: u64 = std::fs::read_dir(&dir)?
        .filter_map(|entry| entry.ok()?.metadata().ok())
        .map(|meta| meta.len())
        .sum();
    let rss_mb = rss_mb_of(std::process::id());
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);

    Ok(Report {
        benchmark: "store",
        messages: MESSAGES,
        insert_ms: round1(insert_ms),
        inserts_per_sec: (MESSAGES as f64 / (insert_ms / 1000.0)).round(),
        db_mb: round1(db_bytes as f64 / 1_048_576.0),
        searches: samples.len(),
        search_p50_ms: round2(median(&samples)),
        search_p95_ms: round2(p95),
        search_max_ms: round2(slowest.0),
        slowest_query: slowest.1,
        rss_mb,
        target_search_p95_ms: TARGET_SEARCH_P95_MS,
        memory_scan_p95_ms: round3(p95_of(&scan_samples)),
        memory_search_p95_ms: round3(p95_of(&memory_search_samples)),
        context_reads: context_samples.len(),
        context_read_p50_ms: round3(median(&context_samples)),
        context_read_p95_ms: round3(context_p95),
        target_context_read_p95_ms: TARGET_CONTEXT_READ_P95_MS,
        pass: p95 <= TARGET_SEARCH_P95_MS && context_p95 <= TARGET_CONTEXT_READ_P95_MS,
    })
}

fn p95_of(samples: &[f64]) -> f64 {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[(sorted.len() * 95).div_ceil(100).saturating_sub(1)]
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// 一个岗位存 2000 份对话记忆（每份约 500 字），照 `memory_search` 的做法找一次。
async fn memory_searches(store: &Store) -> anyhow::Result<Vec<f64>> {
    use ip_store::{MemoryStatus, ThreadMemory};

    let role: ip_core::RoleId = "bench-memory".into();
    for i in 0..2_000_i64 {
        let sentence = SENTENCES[i as usize % SENTENCES.len()];
        store
            .put_thread_memory(ThreadMemory {
                role_id: role.clone(),
                thread_id: format!("bench-memory-{i}").into(),
                source_updated_ms: i,
                generated_ms: i,
                status: MemoryStatus::Ok,
                slug: Some(format!("第 {i} 次")),
                summary: Some(format!("{sentence}。").repeat(500 / (sentence.chars().count() + 1) + 1)),
                attempts: 0,
                retry_after_ms: None,
            })
            .await?;
    }
    let mut samples = Vec::new();
    for _ in 0..SEARCH_ROUNDS {
        for query in QUERIES {
            let started = Instant::now();
            let terms = ip_runtime::memory::search::terms(query);
            let memories = store.search_thread_memories(&role, &terms, 5).await?;
            let hits = ip_runtime::memory::search::search(&memories, &terms, 5);
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
            std::hint::black_box(hits);
        }
    }
    Ok(samples)
}

/// 在装了 5 万条消息的库上，反复读一个岗位的决策者笔记和记忆摘要，两层都写满默认上限。
async fn context_reads(store: &Store) -> anyhow::Result<Vec<f64>> {
    use ip_runtime::memory::{self, Layer, MemoryConfig};

    let config = MemoryConfig {
        enabled: true,
        ..MemoryConfig::default()
    };
    let role = "macro-researcher".into();
    memory::put(store, Layer::Notes, &role, &"笔".repeat(config.notes_max_chars), 1).await?;
    memory::put(store, Layer::Summary, &role, &"忆".repeat(config.summary_max_chars), 2).await?;
    let mut samples = Vec::with_capacity(CONTEXT_READS);
    for _ in 0..CONTEXT_READS {
        let started = Instant::now();
        let extras = memory::load(store, &config, &role).await?;
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
        anyhow::ensure!(extras.notes.is_some() && extras.summary.is_some(), "没读到岗位上下文");
    }
    Ok(samples)
}

fn synthetic(i: usize) -> Message {
    let a = SENTENCES[i % SENTENCES.len()];
    let b = SENTENCES[(i * 7 + 3) % SENTENCES.len()];
    let thread = ThreadId::from(format!("thread-{}", i % THREADS));
    let mut message = Message::text(thread, Author::human("bench"), format!("{a}。{b}（编号 {i}）"));
    message.at_ms = i64::try_from(i).unwrap_or(i64::MAX);
    message
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
