//! `ip eval`：用真实模型跑固定的评测问题，按程序判定的检查项打分，输出通过率和花费。
//!
//! 每次运行用一个新的临时数据目录，员工的工作目录是空的，不受之前的对话和报告影响。
//! 评测集的格式见 `evals/macro-smoke.yaml`（看报告）和 `evals/team-context.yaml`（看回复）。

pub mod checks;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use clap::Args;
use ip_core::{AgentId, Author, Body, ThreadId};
use ip_models::Usage;
use ip_runtime::RuntimeEvent;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

#[derive(Args)]
pub struct EvalArgs {
    /// 组织配置文件。
    #[arg(long)]
    config: PathBuf,
    /// 评测集文件。
    #[arg(long, default_value = "evals/macro-smoke.yaml")]
    set: PathBuf,
    /// 换掉被测员工所在岗位的模型，例如 `openai/gpt-6-astra`，用来比较模型。
    #[arg(long)]
    model: Option<String>,
    /// 只跑这一题（id）。
    #[arg(long)]
    only: Option<String>,
    /// 把完整结果写成 JSON。
    #[arg(long)]
    out: Option<PathBuf>,
    /// 每题最多等多久（秒）。
    #[arg(long, default_value_t = 900)]
    timeout_secs: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EvalSet {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    /// 被测员工的 id。
    agent: String,
    question: String,
    /// 至少要有一次成功调用的工具（任一）。
    #[serde(default)]
    tools_any: Vec<String>,
    /// 报告标题里必须出现的小节名。
    #[serde(default)]
    sections: Vec<String>,
    /// 报告（或回复）里必须出现的文字，比如取不到的数据要写“未取得”。一项可以写成几种说法的列表，出现其一就算。
    #[serde(default)]
    must_contain: Vec<Needle>,
    /// 报告（或回复）里不能出现的文字，比如团队规定不用的叫法。
    #[serde(default)]
    must_not_contain: Vec<String>,
    /// 看员工在对话里的最后一条回复，不看报告文件：不查报告、数字追溯和观测日期，只查上面两项。
    #[serde(default)]
    reply: bool,
    /// 能追溯到工具结果的数字占比下限，默认 1（全部）。
    #[serde(default = "one")]
    min_traceable: f64,
}

fn one() -> f64 {
    1.0
}

/// `must_contain` 的一项：一种说法，或几种说法里出现其一。
#[derive(Deserialize)]
#[serde(untagged)]
enum Needle {
    One(String),
    Any(Vec<String>),
}

impl Needle {
    fn found_in(&self, text: &str) -> bool {
        match self {
            Needle::One(s) => text.contains(s.as_str()),
            Needle::Any(list) => list.iter().any(|s| text.contains(s.as_str())),
        }
    }

    fn label(&self) -> String {
        match self {
            Needle::One(s) => s.clone(),
            Needle::Any(list) => list.join(" / "),
        }
    }
}

/// 没出现的必含项。
fn missing_needles<'a>(needles: &'a [Needle], text: &str) -> Vec<&'a Needle> {
    needles.iter().filter(|n| !n.found_in(text)).collect()
}

/// 出现了的禁用写法。
fn forbidden_found<'a>(words: &'a [String], text: &str) -> Vec<&'a str> {
    words
        .iter()
        .filter(|w| text.contains(w.as_str()))
        .map(String::as_str)
        .collect()
}

#[derive(Serialize)]
struct Check {
    name: &'static str,
    pass: bool,
    detail: String,
}

#[derive(Serialize)]
struct CaseResult {
    id: String,
    model: String,
    pass: bool,
    cost_usd: f64,
    /// 这一题的 token 用量。缓存命中率 = `cached_tokens / prompt_tokens`。这一轮失败时拿不到，记 0。
    usage: Usage,
    steps: u32,
    elapsed_ms: u64,
    error: Option<String>,
    report_path: Option<String>,
    /// `reply` 题：员工最后一条回复的全文。
    reply: Option<String>,
    /// 成功调用过的工具（去重），看员工有没有用 `context_search`、`context_read` 这些。
    tools_used: Vec<String>,
    checks: Vec<Check>,
    untraceable: Vec<String>,
}

pub fn run(args: EvalArgs) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run_async(args))
}

async fn run_async(args: EvalArgs) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(&args.set).with_context(|| format!("读取评测集 {}", args.set.display()))?;
    let set: EvalSet = serde_saphyr::from_str(&text).map_err(|e| anyhow::anyhow!("评测集格式不对：{e}"))?;
    let cases: Vec<&Case> = set
        .cases
        .iter()
        .filter(|c| args.only.as_deref().is_none_or(|id| c.id == id))
        .collect();
    if cases.is_empty() {
        bail!("没有要跑的题");
    }

    let mut config = ip_server::Config::load(&args.config)?;
    let dir = std::env::temp_dir().join(format!("ip-eval-{}", ThreadId::new()));
    config.data_dir = dir.clone();
    config.channels.clear();
    config.schedules.clear();
    let mut models = std::collections::BTreeMap::new();
    for case in &cases {
        let Some(agent) = config.agents.iter().find(|a| a.id == case.agent) else {
            bail!("题目 {} 的员工 {} 不在配置里", case.id, case.agent);
        };
        let role_id = agent.role.clone();
        let workspace = agent.workspace.clone();
        let Some(role) = config.roles.iter_mut().find(|r| r.id.as_str() == role_id) else {
            bail!("员工 {} 的岗位 {role_id} 不存在", case.agent);
        };
        if let Some(model) = &args.model {
            role.model.default = model.clone();
        }
        models.insert(case.agent.clone(), (role.model.default.clone(), workspace));
    }
    println!("临时数据目录：{}", dir.display());
    let app = ip_server::App::from_config(&config).await?;

    let mut results = Vec::new();
    for case in cases {
        let (model, workspace) = &models[&case.agent];
        let workspace = workspace
            .clone()
            .unwrap_or_else(|| dir.join("workspaces").join(&case.agent));
        println!("▶ {}（{}，{model}）", case.id, case.agent);
        let result = run_case(&app, case, model, &workspace, Duration::from_secs(args.timeout_secs)).await?;
        print_case(&result);
        results.push(result);
    }
    app.shutdown().await;

    let passed = results.iter().filter(|r| r.pass).count();
    let cost: f64 = results.iter().map(|r| r.cost_usd).sum();
    let mut usage = Usage::default();
    for r in &results {
        usage.prompt_tokens += r.usage.prompt_tokens;
        usage.completion_tokens += r.usage.completion_tokens;
        usage.cached_tokens += r.usage.cached_tokens;
        usage.cache_write_tokens += r.usage.cache_write_tokens;
    }
    println!(
        "\n通过 {passed}/{}，花费 {cost:.4} 美元，{}",
        results.len(),
        tokens_line(&usage)
    );
    if let Some(out) = &args.out {
        let json = serde_json::json!({ "passed": passed, "total": results.len(), "cost_usd": cost, "usage": usage, "cases": results });
        std::fs::write(out, serde_json::to_string_pretty(&json)?).with_context(|| format!("写入 {}", out.display()))?;
        println!("结果已写入 {}", out.display());
    }
    if passed < results.len() {
        std::process::exit(1);
    }
    Ok(())
}

async fn run_case(
    app: &ip_server::App,
    case: &Case,
    model: &str,
    workspace: &Path,
    timeout: Duration,
) -> anyhow::Result<CaseResult> {
    let runtime = app.runtime();
    let agent = AgentId::from(case.agent.as_str());
    let thread = ThreadId::new();
    let mut events = runtime.subscribe();
    let author = Author::Human {
        name: "eval".into(),
        via: None,
    };
    runtime.send(&agent, thread.clone(), author, &case.question).await?;

    let started = Instant::now();
    let (mut cost, mut steps, mut error) = (0.0, 0, None);
    let mut usage = Usage::default();
    loop {
        let left = timeout.saturating_sub(started.elapsed());
        let event = match tokio::time::timeout(left, events.recv()).await {
            Err(_) => {
                error = Some(format!("{} 秒内没有结束", timeout.as_secs()));
                break;
            }
            Ok(Err(RecvError::Lagged(_))) => continue,
            Ok(Err(RecvError::Closed)) => bail!("运行时已关闭"),
            Ok(Ok(event)) => event,
        };
        match event {
            RuntimeEvent::TurnFinished {
                agent_id,
                thread_id,
                steps: s,
                cost_usd,
                usage: u,
                ..
            } if agent_id == agent && thread_id == thread => {
                cost += cost_usd;
                usage = u;
                steps += s;
                break;
            }
            RuntimeEvent::TurnFailed {
                agent_id,
                thread_id,
                error: e,
            } if agent_id == agent && thread_id == thread => {
                error = Some(e);
                break;
            }
            _ => {}
        }
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;

    let messages = runtime.store().thread_messages(&thread, None, 10_000).await?;
    let mut outputs = Vec::new();
    let mut calls = std::collections::HashMap::new();
    let mut ok_tools = Vec::new();
    let mut report_rel = None;
    let mut reply = None;
    let me = Author::Agent {
        agent_id: agent.clone(),
    };
    for message in &messages {
        match &message.body {
            Body::Text { text } if message.author == me => reply = Some(text.clone()),
            Body::ToolCall { call_id, name, input } => {
                calls.insert(call_id.clone(), name.clone());
                if name == "write_file"
                    && let Some(path) = input["path"].as_str()
                    && path.starts_with("reports/")
                    && path.ends_with(".md")
                {
                    report_rel = Some(path.to_owned());
                }
            }
            Body::ToolResult { call_id, ok, output } if *ok => {
                outputs.push(output.clone());
                if let Some(name) = calls.get(call_id) {
                    ok_tools.push(name.clone());
                }
            }
            _ => {}
        }
    }
    // 数字来源只认取数工具，不认员工自己写文件、跑 shell 的回显。
    let data_outputs: Vec<&str> = messages
        .iter()
        .filter_map(|m| match &m.body {
            Body::ToolResult {
                call_id,
                ok: true,
                output,
            } => {
                let name = calls.get(call_id)?;
                (name.starts_with("mcp__") || name.starts_with("data_")).then_some(output.as_str())
            }
            _ => None,
        })
        .collect();

    let report = report_rel
        .as_ref()
        .and_then(|rel| std::fs::read_to_string(workspace.join(rel)).ok());
    let mut checks = Vec::new();
    checks.push(Check {
        name: "本轮正常结束",
        pass: error.is_none(),
        detail: error.clone().unwrap_or_default(),
    });
    if !case.tools_any.is_empty() {
        let used: Vec<&String> = case.tools_any.iter().filter(|t| ok_tools.contains(t)).collect();
        checks.push(Check {
            name: "调了取数工具",
            pass: !used.is_empty(),
            detail: format!(
                "要求其一：{}；实际成功调用：{}",
                case.tools_any.join("、"),
                unique(&ok_tools).join("、")
            ),
        });
    }
    let mut untraceable = Vec::new();
    if case.reply {
        let text = reply.clone().unwrap_or_default();
        checks.push(Check {
            name: "回复了",
            pass: !text.trim().is_empty(),
            detail: String::new(),
        });
        if !case.must_contain.is_empty() {
            let missing = missing_needles(&case.must_contain, &text);
            checks.push(Check {
                name: "回复里有要求的内容",
                pass: missing.is_empty(),
                detail: labels("没出现", &missing),
            });
        }
        if !case.must_not_contain.is_empty() {
            let found = forbidden_found(&case.must_not_contain, &text);
            checks.push(Check {
                name: "没有不该用的写法",
                pass: found.is_empty(),
                detail: if found.is_empty() {
                    String::new()
                } else {
                    format!("出现了：{}", found.join("、"))
                },
            });
        }
    } else {
        checks.push(Check {
            name: "写了报告",
            pass: report.is_some(),
            detail: report_rel.clone().unwrap_or_else(|| "没有写 reports/*.md".into()),
        });
    }
    if !case.reply
        && let Some(report) = &report
    {
        let numbers = checks::numbers(report);
        let tools = checks::ToolNumbers::new(data_outputs.iter().copied());
        untraceable = numbers
            .iter()
            .filter(|n| !tools.traces(n))
            .map(|n| n.raw.clone())
            .collect();
        let ratio = if numbers.is_empty() {
            0.0
        } else {
            1.0 - untraceable.len() as f64 / numbers.len() as f64
        };
        checks.push(Check {
            name: "数字能在工具结果里找到",
            pass: !numbers.is_empty() && ratio + 1e-9 >= case.min_traceable,
            detail: format!(
                "{}/{} 个能追溯（要求 ≥{:.0}%）",
                numbers.len() - untraceable.len(),
                numbers.len(),
                case.min_traceable * 100.0
            ),
        });
        let rows = checks::table_rows_without_date(report);
        let dates = checks::shared_dates(report, data_outputs.iter().copied());
        checks.push(Check {
            name: "数字带观测日期",
            pass: rows.is_empty() && !dates.is_empty(),
            detail: if rows.is_empty() {
                format!("和工具结果一致的日期：{}", dates.join("、"))
            } else {
                format!("表格里没写日期的行：{}", rows.join(" / "))
            },
        });
        if !case.must_contain.is_empty() {
            let missing = missing_needles(&case.must_contain, report);
            checks.push(Check {
                name: "取不到的写明",
                pass: missing.is_empty(),
                detail: labels("没出现", &missing),
            });
        }
        if !case.must_not_contain.is_empty() {
            let found = forbidden_found(&case.must_not_contain, report);
            checks.push(Check {
                name: "没有不该用的写法",
                pass: found.is_empty(),
                detail: if found.is_empty() {
                    String::new()
                } else {
                    format!("出现了：{}", found.join("、"))
                },
            });
        }
        if !case.sections.is_empty() {
            let missing = checks::missing_sections(report, &case.sections);
            checks.push(Check {
                name: "报告结构齐全",
                pass: missing.is_empty(),
                detail: if missing.is_empty() {
                    String::new()
                } else {
                    format!("缺小节：{}", missing.join("、"))
                },
            });
        }
    }
    Ok(CaseResult {
        id: case.id.clone(),
        model: model.to_owned(),
        pass: checks.iter().all(|c| c.pass),
        cost_usd: cost,
        usage,
        steps,
        elapsed_ms,
        error,
        report_path: report_rel.map(|rel| workspace.join(rel).display().to_string()),
        reply: case.reply.then_some(reply).flatten(),
        tools_used: unique(&ok_tools),
        checks,
        untraceable,
    })
}

fn labels(prefix: &str, needles: &[&Needle]) -> String {
    if needles.is_empty() {
        String::new()
    } else {
        let names: Vec<String> = needles.iter().map(|n| n.label()).collect();
        format!("{prefix}：{}", names.join("、"))
    }
}

fn unique(names: &[String]) -> Vec<String> {
    let mut names = names.to_vec();
    names.sort();
    names.dedup();
    names
}

fn print_case(result: &CaseResult) {
    for check in &result.checks {
        let mark = if check.pass { "✓" } else { "✗" };
        if check.detail.is_empty() {
            println!("  {mark} {}", check.name);
        } else {
            println!("  {mark} {}：{}", check.name, check.detail);
        }
    }
    if !result.untraceable.is_empty() {
        println!("  追溯不到的数字：{}", result.untraceable.join("、"));
    }
    if let Some(path) = &result.report_path {
        println!("  报告：{path}");
    }
    if !result.tools_used.is_empty() {
        println!("  用过的工具：{}", result.tools_used.join("、"));
    }
    println!(
        "  {}，{} 步，{:.1} 秒，{:.4} 美元，{}",
        if result.pass { "通过" } else { "未通过" },
        result.steps,
        result.elapsed_ms as f64 / 1000.0,
        result.cost_usd,
        tokens_line(&result.usage)
    );
}

/// “输入 12345 token（缓存命中 62%），输出 678 token”。
fn tokens_line(usage: &Usage) -> String {
    let hit = if usage.prompt_tokens == 0 {
        0
    } else {
        (100 * u64::from(usage.cached_tokens.min(usage.prompt_tokens)) + u64::from(usage.prompt_tokens) / 2)
            / u64::from(usage.prompt_tokens)
    };
    format!(
        "输入 {} token（缓存命中 {hit}%），输出 {} token",
        usage.prompt_tokens, usage.completion_tokens
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needles_match_any_of_their_wordings() {
        let set: EvalSet = serde_saphyr::from_str(
            "cases:\n  - id: a\n    agent: x\n    question: q\n    reply: true\n    must_contain: [林雯, [2026-12-12, 12 月 12 日]]\n    must_not_contain: [DXY]\n",
        )
        .unwrap();
        let case = &set.cases[0];
        assert!(case.reply);
        assert!(missing_needles(&case.must_contain, "林雯负责，12 月 12 日交付").is_empty());
        let missing = missing_needles(&case.must_contain, "林雯负责");
        assert_eq!(missing.len(), 1);
        assert_eq!(labels("没出现", &missing), "没出现：2026-12-12 / 12 月 12 日");
        assert_eq!(
            forbidden_found(&case.must_not_contain, "广义美元指数（不是 DXY）"),
            ["DXY"]
        );
        assert!(forbidden_found(&case.must_not_contain, "广义美元指数").is_empty());
    }

    /// 仓库里的评测集都要能解析（字段写错会被拒）。
    #[test]
    fn shipped_eval_sets_parse() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals");
        for name in ["macro-smoke.yaml", "macro.yaml", "team-context.yaml"] {
            let text = std::fs::read_to_string(dir.join(name)).unwrap();
            let set: EvalSet = serde_saphyr::from_str(&text).unwrap_or_else(|e| panic!("{name}：{e}"));
            assert!(!set.cases.is_empty(), "{name} 没有题");
            let mut ids: Vec<&str> = set.cases.iter().map(|c| c.id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), set.cases.len(), "{name} 里有重复的 id");
        }
    }

    /// 比较模型用的评测集 10 到 20 题（backlog B-2），冒烟的两题原样包含在里面。
    #[test]
    fn macro_set_contains_the_smoke_cases() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../evals");
        let load = |name: &str| -> EvalSet {
            serde_saphyr::from_str(&std::fs::read_to_string(dir.join(name)).unwrap()).unwrap()
        };
        let (smoke, full) = (load("macro-smoke.yaml"), load("macro.yaml"));
        assert!((10..=20).contains(&full.cases.len()), "macro.yaml 有 {} 题", full.cases.len());
        for case in &smoke.cases {
            let same = full.cases.iter().find(|c| c.id == case.id).expect("冒烟题不在 macro.yaml 里");
            assert_eq!(same.question, case.question, "{} 的题面两边不一样", case.id);
            assert_eq!(same.tools_any, case.tools_any, "{} 的工具要求两边不一样", case.id);
        }
        // 每题都看报告，并且有题要写“未取得”。
        assert!(full.cases.iter().all(|c| !c.reply && !c.sections.is_empty()));
        let missing = full
            .cases
            .iter()
            .filter(|c| c.must_contain.iter().any(|n| n.found_in("未取得")))
            .count();
        assert!(missing >= 3, "要写“未取得”的题只有 {missing} 道");
    }
}
