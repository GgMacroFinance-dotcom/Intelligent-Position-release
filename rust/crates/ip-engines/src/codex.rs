//! Codex 引擎（`docs/ip/m2-spec.md` D-21、D-22、D-24）。

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use ip_core::Body;
use ip_models::{Price, Usage};
use ip_runtime::engine::{BoxFuture, Engine, McpGrant, TurnContext, TurnReport};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Semaphore;

/// 写进对话的命令输出上限（字节）。完整输出在 Codex 自己的会话记录里。
const MAX_OUTPUT: usize = 16 * 1024;
/// 出错时附上的 stderr 末尾（字节）。
const MAX_STDERR: usize = 4 * 1024;
/// MCP 服务端在 Codex 配置里的名字；组织工具在 Codex 里叫 `mcp__ip__<工具>`。
const MCP_NAME: &str = "ip";

/// Codex 能动工作目录的程度，对应 `codex --sandbox`。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CodexSandbox {
    ReadOnly,
    #[default]
    WorkspaceWrite,
}

impl CodexSandbox {
    fn flag(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
        }
    }
}

/// `engines.<岗位>`，`kind: codex`。
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexConfig {
    /// 可执行文件，默认 `codex`（在 PATH 里找）。
    #[serde(default = "default_command")]
    pub command: String,
    /// 传给 `codex -m` 的模型名；省略时用 Codex 的默认模型。
    #[serde(default)]
    pub model: Option<String>,
    /// 存放 API key 的环境变量名，值以 `CODEX_API_KEY` 交给 Codex。
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub sandbox: CodexSandbox,
    /// Codex 里执行的命令能否联网。
    #[serde(default)]
    pub network: bool,
    /// 一轮最长多久，到时整组终止进程。
    #[serde(default = "default_timeout")]
    pub turn_timeout_secs: u64,
    /// 这个岗位同时最多跑几个 Codex 进程。
    #[serde(default = "default_concurrency")]
    pub max_concurrency: usize,
    /// 美元 / 百万 token，用来记账和执行岗位的 `budget_per_day_usd`。
    #[serde(default)]
    pub price: Option<Price>,
}

fn default_command() -> String {
    "codex".into()
}

fn default_timeout() -> u64 {
    1800
}

fn default_concurrency() -> usize {
    4
}

pub struct CodexEngine {
    config: CodexConfig,
    api_key: Option<String>,
    /// 每个员工的 `CODEX_HOME` 放在 `<home_root>/<员工 id>/codex`，不在工作目录里。
    home_root: PathBuf,
    slots: Semaphore,
}

impl CodexEngine {
    /// `api_key` 是从 `api_key_env` 读到的值，由调用方在启动时读取。
    pub fn new(config: CodexConfig, api_key: Option<String>, home_root: impl Into<PathBuf>) -> Arc<Self> {
        let slots = Semaphore::new(config.max_concurrency.max(1));
        Arc::new(Self {
            config,
            api_key,
            home_root: home_root.into(),
            slots,
        })
    }

    /// 一轮的命令行参数。提示词从标准输入传（最后的 `-`）。
    fn args(&self, session: Option<&str>, workspace: &Path, mcp: Option<&McpGrant>) -> Vec<String> {
        let mut a: Vec<String> = vec!["exec".into(), "--json".into(), "--skip-git-repo-check".into()];
        a.push("--cd".into());
        a.push(workspace.display().to_string());
        a.push("--sandbox".into());
        a.push(self.config.sandbox.flag().into());
        a.push("-c".into());
        a.push(r#"approval_policy="never""#.into());
        if let Some(m) = &self.config.model {
            a.push("-m".into());
            a.push(m.clone());
        }
        if self.config.network {
            a.push("-c".into());
            a.push("sandbox_workspace_write.network_access=true".into());
        }
        if let Some(grant) = mcp {
            a.push("-c".into());
            a.push(format!("mcp_servers.{MCP_NAME}.url={}", toml_string(&grant.url)));
            a.push("-c".into());
            a.push(format!(r#"mcp_servers.{MCP_NAME}.bearer_token_env_var="IP_MCP_TOKEN""#));
            // codex-cli 0.169.0 默认要人批准 MCP 工具调用；approval_policy 是 never 时就直接拒掉，
            // 员工没法用 update_task 交回任务。组织工具已经由本轮令牌限定范围，这里一律放行。
            a.push("-c".into());
            a.push(format!(
                r#"mcp_servers.{MCP_NAME}.default_tools_approval_mode="approve""#
            ));
        }
        if let Some(id) = session {
            a.push("resume".into());
            a.push(id.into());
        }
        a.push("-".into());
        a
    }

    async fn run(&self, ctx: TurnContext<'_>) -> TurnReport {
        let mut report = TurnReport::default();
        // 一轮的时限从排队等空位就开始算：本轮令牌的期限按 turn_timeout 定，不能被排队时间吃掉。
        let started = std::time::Instant::now();
        let timeout = self.turn_timeout();
        let _slot = match tokio::time::timeout(timeout, self.slots.acquire()).await {
            Ok(Ok(slot)) => slot,
            Ok(Err(_)) => {
                report.error = Some("Codex 引擎已关闭".into());
                return report;
            }
            Err(_) => {
                report.error = Some(format!("排队等 Codex 空位超过 {} 秒", timeout.as_secs()));
                return report;
            }
        };
        let Some(workspace) = ctx.workspace else {
            report.error = Some("Codex 引擎需要工作目录：这个员工的 tools 不能设为 false".into());
            return report;
        };
        let home = self.home_root.join(ctx.agent.id.as_str());
        let codex_home = home.join("codex");
        if let Err(e) = std::fs::create_dir_all(&codex_home) {
            report.error = Some(format!("创建 {} 失败：{e}", codex_home.display()));
            return report;
        }
        let prompt = match ctx.session {
            None => format!("{}\n\n---\n\n{}", ctx.system_prompt, ctx.input),
            Some(_) => ctx.input.to_string(),
        };

        let mut cmd = tokio::process::Command::new(&self.config.command);
        cmd.args(self.args(ctx.session, workspace, ctx.mcp.as_ref()))
            .current_dir(workspace)
            .env_clear()
            .env("HOME", &home)
            .env("CODEX_HOME", &codex_home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        if let Some(path) = std::env::var_os("PATH") {
            cmd.env("PATH", path);
        }
        if let Some(key) = &self.api_key {
            cmd.env("CODEX_API_KEY", key);
        }
        if let Some(grant) = &ctx.mcp {
            cmd.env("IP_MCP_TOKEN", &grant.token);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                report.error = Some(format!("启动 {} 失败：{e}", self.config.command));
                return report;
            }
        };
        let pid = child.id();
        if let Some(mut stdin) = child.stdin.take() {
            // 单独的任务里写：提示词超过管道缓冲、而进程又不读时，不能卡在超时保护之外。
            // 写失败多半是进程已经退出，原因会出现在 stderr 和退出码里。
            tokio::spawn(async move {
                let _ = stdin.write_all(prompt.as_bytes()).await;
            });
        }
        let stderr = child.stderr.take().map(|s| tokio::spawn(tail(s)));
        let stdout = child.stdout.take().expect("stdout 已设为 piped");

        let mut state = Turn::default();
        let read = async {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                for action in parse_line(&line) {
                    state.apply(action, &ctx).await;
                }
            }
            child.wait().await
        };
        let status = tokio::time::timeout(timeout.saturating_sub(started.elapsed()), read).await;
        let stderr = match stderr {
            Some(task) if status.is_ok() => task.await.unwrap_or_default(),
            Some(task) => {
                task.abort();
                String::new()
            }
            None => String::new(),
        };

        report.steps = state.steps;
        // 续接的会话里，`turn.completed` 报的是整个会话的累计用量（2026-11-14 用 codex-cli 0.169.0 核对过），
        // 只有减掉以前几轮的才是这一轮的。没续接、或者 Codex 另开了会话时，基线是 0。
        let resumed = ctx.session.is_some() && state.session.as_deref().is_none_or(|id| Some(id) == ctx.session);
        let baseline = if resumed { ctx.session_usage } else { Usage::default() };
        let (turn_usage, session_usage) = split_usage(baseline, state.usage);
        report.usage = turn_usage;
        report.cost_usd = self.config.price.map_or(0.0, |p| p.cost(&turn_usage));
        report.session_usage = Some(session_usage);
        report.session_id = state.session;
        report.error = match status {
            Err(_) => {
                if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
                    let _ =
                        nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGKILL);
                }
                Some(format!("Codex 超过 {} 秒没有完成，已终止", timeout.as_secs()))
            }
            Ok(Err(e)) => Some(format!("等待 Codex 退出失败：{e}")),
            Ok(Ok(exit)) => {
                let last = state
                    .warning
                    .map(|w| format!("；最后一条错误：{w}"))
                    .unwrap_or_default();
                match (state.failure, state.completed) {
                    (Some(e), _) => Some(format!("Codex 报错：{e}")),
                    (None, true) => None,
                    (None, false) if exit.success() => {
                        Some(format!("Codex 没有报告这一轮结束（缺少 turn.completed）{last}"))
                    }
                    (None, false) => Some(format!("Codex 异常退出（{exit}）：{}{last}", stderr.trim())),
                }
            }
        };
        report
    }
}

impl Engine for CodexEngine {
    fn kind(&self) -> &str {
        "codex"
    }

    fn turn_timeout(&self) -> Duration {
        Duration::from_secs(self.config.turn_timeout_secs.max(1))
    }

    fn turn<'a>(&'a self, ctx: TurnContext<'a>) -> BoxFuture<'a, TurnReport> {
        Box::pin(self.run(ctx))
    }
}

/// TOML 字符串字面量（`-c key=value` 的值按 TOML 解析）。
fn toml_string(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

/// 读完 stderr，只留末尾。
async fn tail(mut stderr: tokio::process::ChildStderr) -> String {
    let mut buf = Vec::new();
    let _ = stderr.read_to_end(&mut buf).await;
    let start = buf.len().saturating_sub(MAX_STDERR);
    String::from_utf8_lossy(&buf[start..]).into_owned()
}

/// 解析一行输出得到的动作。
#[derive(Debug, PartialEq)]
enum Action {
    Session(String),
    Append(Body),
    Usage(Usage),
    Completed,
    Failed(String),
    /// 顶层 `error` 事件。codex-cli 0.169.0 用它报中途重试（如 `Reconnecting... 2/5`），
    /// 之后常常照样跑完，所以只记下来；这一轮有没有失败看 `turn.failed` 和 `turn.completed`。
    Warning(String),
}

/// 把 Codex 这次报的用量拆成（这一轮的用量，会话累计用量）。`baseline` 是会话到上一轮为止的累计值。
///
/// 报的数每一项都不小于基线时，按累计值处理：这一轮是两者之差。有一项比基线小，说明报的不是累计值
/// （没续接上，或者以后的版本改成按轮报），就按这一轮的用量全额记，宁可多记也不漏记。
fn split_usage(baseline: Usage, reported: Usage) -> (Usage, Usage) {
    let cumulative = reported.prompt_tokens >= baseline.prompt_tokens
        && reported.cached_tokens >= baseline.cached_tokens
        && reported.cache_write_tokens >= baseline.cache_write_tokens
        && reported.completion_tokens >= baseline.completion_tokens;
    if cumulative {
        let turn = Usage {
            prompt_tokens: reported.prompt_tokens - baseline.prompt_tokens,
            cached_tokens: reported.cached_tokens - baseline.cached_tokens,
            cache_write_tokens: reported.cache_write_tokens - baseline.cache_write_tokens,
            completion_tokens: reported.completion_tokens - baseline.completion_tokens,
        };
        (turn, reported)
    } else {
        let total = Usage {
            prompt_tokens: baseline.prompt_tokens.saturating_add(reported.prompt_tokens),
            cached_tokens: baseline.cached_tokens.saturating_add(reported.cached_tokens),
            cache_write_tokens: baseline.cache_write_tokens.saturating_add(reported.cache_write_tokens),
            completion_tokens: baseline.completion_tokens.saturating_add(reported.completion_tokens),
        };
        (reported, total)
    }
}

#[derive(Default)]
struct Turn {
    session: Option<String>,
    usage: Usage,
    steps: u32,
    completed: bool,
    failure: Option<String>,
    /// 最近一条 `error` 事件；这一轮没正常结束时附在失败原因里。
    warning: Option<String>,
}

impl Turn {
    async fn apply(&mut self, action: Action, ctx: &TurnContext<'_>) {
        match action {
            Action::Session(id) => self.session = Some(id),
            Action::Append(body) => {
                if !matches!(body, Body::ToolResult { .. }) {
                    self.steps += 1;
                }
                if let Err(e) = ctx.sink.append(body).await {
                    tracing::warn!(error = %e, "写回 Codex 的输出失败");
                }
            }
            Action::Usage(u) => {
                self.usage.prompt_tokens += u.prompt_tokens;
                self.usage.cached_tokens += u.cached_tokens;
                self.usage.cache_write_tokens += u.cache_write_tokens;
                self.usage.completion_tokens += u.completion_tokens;
            }
            Action::Completed => self.completed = true,
            Action::Failed(e) => self.failure = Some(e),
            Action::Warning(w) => {
                tracing::warn!(warning = %w, "Codex 报了一条错误，这一轮继续");
                self.warning = Some(w);
            }
        }
    }
}

fn tokens(v: &Value, key: &str) -> u32 {
    v.get(key)
        .and_then(Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

fn clip(text: &str) -> String {
    ip_tools::truncate_output(text.to_string(), MAX_OUTPUT, 0)
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// 把 `codex exec --json` 的一行变成动作（D-24）。认不出的行返回空。
fn parse_line(line: &str) -> Vec<Action> {
    let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
        if !line.trim().is_empty() {
            tracing::debug!(line, "Codex 输出了非 JSON 的一行");
        }
        return vec![];
    };
    match str_of(&event, "type") {
        "thread.started" => match event.get("thread_id").and_then(Value::as_str) {
            Some(id) => vec![Action::Session(id.to_string())],
            None => vec![],
        },
        "turn.completed" => {
            let u = event.get("usage").cloned().unwrap_or_default();
            vec![
                Action::Usage(Usage {
                    prompt_tokens: tokens(&u, "input_tokens"),
                    cached_tokens: tokens(&u, "cached_input_tokens"),
                    completion_tokens: tokens(&u, "output_tokens"),
                    // 真实的 codex-cli 0.169.0 会报这一项（2026-11-14 验收时看到）。
                    cache_write_tokens: tokens(&u, "cache_write_input_tokens"),
                }),
                Action::Completed,
            ]
        }
        "turn.failed" => {
            let message = event
                .get("error")
                .map(|e| str_of(e, "message").to_string())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "这一轮失败".into());
            vec![Action::Failed(message)]
        }
        "error" => vec![Action::Warning(str_of(&event, "message").to_string())],
        "item.completed" => event.get("item").map(item).unwrap_or_default(),
        other => {
            tracing::debug!(event = other, "忽略 Codex 事件");
            vec![]
        }
    }
}

/// 一个完成的条目写成对话里的消息。
fn item(item: &Value) -> Vec<Action> {
    let id = str_of(item, "id").to_string();
    let call = |name: String, input: Value| {
        Action::Append(Body::ToolCall {
            call_id: id.clone(),
            name,
            input,
        })
    };
    let result = |ok: bool, output: String| {
        Action::Append(Body::ToolResult {
            call_id: id.clone(),
            ok,
            output,
        })
    };
    match str_of(item, "type") {
        "agent_message" => {
            let text = str_of(item, "text");
            if text.trim().is_empty() {
                vec![]
            } else {
                vec![Action::Append(Body::Text { text: text.to_string() })]
            }
        }
        "command_execution" => {
            let exit = item.get("exit_code").and_then(Value::as_i64);
            let ok = exit == Some(0) && str_of(item, "status") != "failed";
            let mut output = clip(str_of(item, "aggregated_output"));
            if let Some(code) = exit.filter(|c| *c != 0) {
                output.push_str(&format!("\n（退出码 {code}）"));
            }
            vec![
                call("codex.shell".into(), json!({ "command": str_of(item, "command") })),
                result(ok, output),
            ]
        }
        "file_change" => {
            let changes = item.get("changes").cloned().unwrap_or_else(|| json!([]));
            let list: Vec<String> = changes
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|c| format!("{} {}", str_of(c, "kind"), str_of(c, "path")))
                        .collect()
                })
                .unwrap_or_default();
            let ok = str_of(item, "status") != "failed";
            vec![
                call("codex.edit".into(), json!({ "changes": changes })),
                result(ok, list.join("\n")),
            ]
        }
        "mcp_tool_call" => {
            let name = format!("mcp__{}__{}", str_of(item, "server"), str_of(item, "tool"));
            let input = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // codex-cli 0.169.0 成功时也带 `"error": null`，空的不算出错。
            let error = item
                .get("error")
                .filter(|e| !e.is_null())
                .map(|e| match e.get("message") {
                    Some(m) => m.as_str().unwrap_or_default().to_string(),
                    None => e.to_string(),
                });
            let ok = error.is_none() && str_of(item, "status") != "failed";
            let output = match error {
                Some(e) => e,
                None => mcp_text(item.get("result")),
            };
            vec![call(name, input), result(ok, clip(&output))]
        }
        "reasoning" => vec![],
        other => {
            tracing::debug!(item = other, "忽略 Codex 条目");
            vec![]
        }
    }
}

/// MCP 调用结果里的文字内容。
fn mcp_text(result: Option<&Value>) -> String {
    let Some(r) = result else { return String::new() };
    let texts: Vec<&str> = r
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if texts.is_empty() {
        r.to_string()
    } else {
        texts.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine(config: &str) -> Arc<CodexEngine> {
        CodexEngine::new(serde_json::from_str(config).unwrap(), None, "/tmp/ip-engines")
    }

    #[test]
    fn config_defaults_and_unknown_fields() {
        let c: CodexConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(c.command, "codex");
        assert_eq!(c.sandbox, CodexSandbox::WorkspaceWrite);
        assert_eq!((c.turn_timeout_secs, c.max_concurrency, c.network), (1800, 4, false));
        assert!(serde_json::from_str::<CodexConfig>(r#"{"sandbox": "read-only"}"#).is_ok());
        assert!(serde_json::from_str::<CodexConfig>(r#"{"sandbox": "danger-full-access"}"#).is_err());
        assert!(serde_json::from_str::<CodexConfig>(r#"{"typo": 1}"#).is_err());
    }

    #[test]
    fn resumed_sessions_are_charged_only_the_difference() {
        let usage = |prompt_tokens, cached_tokens, cache_write_tokens, completion_tokens| Usage {
            prompt_tokens,
            cached_tokens,
            cache_write_tokens,
            completion_tokens,
        };
        // 第一轮：没有基线，报多少记多少。
        let first = usage(12_492, 0, 12_489, 5);
        assert_eq!(split_usage(Usage::default(), first), (first, first));
        // 续接的第二轮报的是累计值（2026-11-14 用真实 Codex 核对的数）：只记差值。
        let second = usage(25_006, 12_489, 12_511, 23);
        assert_eq!(split_usage(first, second), (usage(12_514, 12_489, 22, 18), second));
        // 这一轮没报用量（没有 turn.completed）：不记，累计值不变。
        assert_eq!(split_usage(second, Usage::default()), (Usage::default(), second));
        // 报的数比基线小，说明不是累计值：全额记，累计值往上加。
        let per_turn = usage(9_000, 100, 0, 40);
        assert_eq!(
            split_usage(second, per_turn),
            (per_turn, usage(34_006, 12_589, 12_511, 63))
        );
    }

    #[test]
    fn first_turn_and_resume_args() {
        let e = engine(r#"{"model": "m1", "network": true}"#);
        let grant = McpGrant {
            url: "http://127.0.0.1:7878/mcp".into(),
            token: "t".into(),
        };
        let first = e.args(None, Path::new("/w"), Some(&grant)).join(" ");
        assert_eq!(
            first,
            r#"exec --json --skip-git-repo-check --cd /w --sandbox workspace-write -c approval_policy="never" -m m1 -c sandbox_workspace_write.network_access=true -c mcp_servers.ip.url="http://127.0.0.1:7878/mcp" -c mcp_servers.ip.bearer_token_env_var="IP_MCP_TOKEN" -c mcp_servers.ip.default_tools_approval_mode="approve" -"#
        );
        let again = e.args(Some("sess-1"), Path::new("/w"), None).join(" ");
        assert!(
            again.ends_with("resume sess-1 -") && !again.contains("mcp_servers"),
            "{again}"
        );
    }

    #[test]
    fn parses_events_and_items() {
        assert_eq!(
            parse_line(r#"{"type":"thread.started","thread_id":"abc"}"#),
            [Action::Session("abc".into())]
        );
        assert_eq!(
            parse_line(
                r#"{"type":"turn.completed","usage":{"input_tokens":1200,"cached_input_tokens":200,"output_tokens":300}}"#
            ),
            [
                Action::Usage(Usage {
                    prompt_tokens: 1200,
                    cached_tokens: 200,
                    completion_tokens: 300,
                    cache_write_tokens: 0,
                }),
                Action::Completed
            ]
        );
        // codex-cli 0.169.0 的真实输出（2026-11-14）：多了缓存写入和推理 token。
        assert_eq!(
            parse_line(
                r#"{"type":"turn.completed","usage":{"input_tokens":26535,"cached_input_tokens":13175,"cache_write_input_tokens":13354,"output_tokens":134,"reasoning_output_tokens":0}}"#
            ),
            [
                Action::Usage(Usage {
                    prompt_tokens: 26535,
                    cached_tokens: 13175,
                    completion_tokens: 134,
                    cache_write_tokens: 13354,
                }),
                Action::Completed
            ]
        );
        assert_eq!(
            parse_line(r#"{"type":"turn.failed","error":{"message":"额度不足"}}"#),
            [Action::Failed("额度不足".into())]
        );
        assert_eq!(
            parse_line(r#"{"type":"error","message":"Reconnecting... 2/5"}"#),
            [Action::Warning("Reconnecting... 2/5".into())]
        );
        assert_eq!(
            parse_line(r#"{"type":"item.completed","item":{"id":"i1","type":"agent_message","text":"修好了"}}"#),
            [Action::Append(Body::Text {
                text: "修好了".into()
            })]
        );
        let shell = parse_line(
            r#"{"type":"item.completed","item":{"id":"i2","type":"command_execution","command":"sh test.sh","aggregated_output":"FAIL","exit_code":1,"status":"failed"}}"#,
        );
        assert_eq!(
            shell[0],
            Action::Append(Body::ToolCall {
                call_id: "i2".into(),
                name: "codex.shell".into(),
                input: json!({"command": "sh test.sh"})
            })
        );
        assert!(
            matches!(&shell[1], Action::Append(Body::ToolResult { ok: false, output, .. }) if output.contains("FAIL") && output.contains("退出码 1"))
        );
        let edit = parse_line(
            r#"{"type":"item.completed","item":{"id":"i3","type":"file_change","changes":[{"path":"growth.sh","kind":"update"}],"status":"completed"}}"#,
        );
        assert!(
            matches!(&edit[1], Action::Append(Body::ToolResult { ok: true, output, .. }) if output == "update growth.sh")
        );
        let mcp = parse_line(
            r#"{"type":"item.completed","item":{"id":"i4","type":"mcp_tool_call","server":"ip","tool":"update_task","arguments":{"task_id":"t1"},"result":{"content":[{"type":"text","text":"已更新"}]},"status":"completed"}}"#,
        );
        assert!(matches!(&mcp[0], Action::Append(Body::ToolCall { name, .. }) if name == "mcp__ip__update_task"));
        assert!(matches!(&mcp[1], Action::Append(Body::ToolResult { ok: true, output, .. }) if output == "已更新"));
        // 推理过程、不认识的事件、不是 JSON 的行都忽略。
        assert!(
            parse_line(r#"{"type":"item.completed","item":{"id":"i5","type":"reasoning","text":"..."}}"#).is_empty()
        );
        assert!(parse_line(r#"{"type":"turn.started"}"#).is_empty());
        assert!(parse_line("not json").is_empty());
        // codex-cli 0.169.0 的 MCP 调用：成功时 `error` 是 null，不能当成失败（2026-11-14 真实验收时看到）。
        let ok_call = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_4","type":"mcp_tool_call","server":"ip","tool":"list_tasks","arguments":{},"result":{"content":[{"type":"text","text":"[]"}]},"error":null,"status":"completed"}}"#,
        );
        assert!(
            matches!(ok_call.last(), Some(Action::Append(Body::ToolResult { ok: true, output, .. })) if output == "[]"),
            "{ok_call:?}"
        );
        let bad_call = parse_line(
            r#"{"type":"item.completed","item":{"id":"item_5","type":"mcp_tool_call","server":"ip","tool":"update_task","arguments":{},"result":null,"error":{"message":"任务不存在"},"status":"failed"}}"#,
        );
        assert!(
            matches!(bad_call.last(), Some(Action::Append(Body::ToolResult { ok: false, output, .. })) if output == "任务不存在"),
            "{bad_call:?}"
        );
    }
}
