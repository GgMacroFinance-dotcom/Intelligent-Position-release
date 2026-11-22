//! Claude Code 引擎（`docs/ip/m2-spec.md` D-36）。
//!
//! 每轮跑一次 `claude -p --output-format stream-json`，跑完进程就退出；同一对话的后续几轮用 `--resume` 续接。
//! Claude Code 没有 Codex 那样的系统沙箱（它自己的沙箱要 bubblewrap，第一版不用），靠它的权限规则限定能做的事：
//! `--bare --restricted` 不读工作目录和用户目录里的设置、钩子、插件，文件工具只能动工作目录；
//! `--permission-mode acceptEdits` 自动接受工作目录里的改动；要人批准的一律拒绝（`--permission-prompts none`）；
//! 配置里 `allowed_tools` 的规则（`--allowedTools`）由部署的人预先放行。
//!
//! 参数和输出格式按 2026-11 的 Claude Code 2.1.330 写成，并用它连假的模型接口核对过；换版本时要重新核对。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ip_core::Body;
use ip_models::{Price, Usage};
use ip_runtime::engine::{BoxFuture, Engine, McpGrant, TurnContext, TurnReport};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

use crate::process::{self, Action, clip};

const KIND: process::Kind = process::Kind {
    name: "Claude Code",
    end_event: "result",
    parse: parse_line,
};

/// MCP 服务端在 Claude Code 配置里的名字；组织工具在 Claude Code 里叫 `mcp__ip__<工具>`。
const MCP_NAME: &str = "ip";

/// Claude Code 能动工作目录的程度，对应交给它的内置工具。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClaudeCodeAccess {
    /// 只能读工作目录（`Read`），不能改、不能跑命令。
    ReadOnly,
    /// 能读、能改工作目录（`Read`、`Edit`），能跑 Claude Code 认为安全的命令和 `allowed_tools` 放行的命令（`Bash`）。
    #[default]
    WorkspaceWrite,
}

impl ClaudeCodeAccess {
    /// `--tools` 的值。`--bare` 下 Claude Code 只有这三个内置工具。
    fn tools(self) -> &'static str {
        match self {
            Self::ReadOnly => "Read",
            Self::WorkspaceWrite => "Bash,Edit,Read",
        }
    }
}

/// `engines.<岗位>`，`kind: claude_code`。
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeCodeConfig {
    /// 可执行文件，默认 `claude`（在 PATH 里找）。
    #[serde(default = "default_command")]
    pub command: String,
    /// 传给 `claude --model` 的模型名；省略时用 Claude Code 的默认模型。
    #[serde(default)]
    pub model: Option<String>,
    /// 存放 Anthropic API key 的环境变量名，值以 `ANTHROPIC_API_KEY` 交给 Claude Code。
    /// 必须写：`--bare` 只认 API key，不读登录状态和钥匙串。
    pub api_key_env: String,
    /// 换一个兼容 Anthropic 接口的地址（以 `ANTHROPIC_BASE_URL` 交给 Claude Code）；省略时连 Anthropic。
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub access: ClaudeCodeAccess,
    /// 预先放行的权限规则，原样交给 `--allowedTools`，如 `Bash(cargo test:*)`。
    /// 放行的命令不受工作目录限制，能执行工作目录里的代码，只在可信的项目上放行。
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// 一轮最长多久，到时整组终止进程。
    #[serde(default = "default_timeout")]
    pub turn_timeout_secs: u64,
    /// 这个岗位同时最多跑几个 Claude Code 进程。
    #[serde(default = "default_concurrency")]
    pub max_concurrency: usize,
    /// 美元 / 百万 token，用来记账和执行岗位的 `budget_per_day_usd`。
    /// 不用 Claude Code 自己报的 `total_cost_usd`：续接时它是整个会话的累计（2.1.330 核对过），每轮记会重复。
    #[serde(default)]
    pub price: Option<Price>,
}

fn default_command() -> String {
    "claude".into()
}

fn default_timeout() -> u64 {
    1800
}

fn default_concurrency() -> usize {
    4
}

impl ClaudeCodeConfig {
    /// 配置里交给命令行的值不能被当成别的参数，也不能跨行。
    pub fn validate(&self) -> Result<(), String> {
        let arg_ok = |v: &str| !v.trim().is_empty() && !v.starts_with('-') && !v.chars().any(char::is_control);
        if let Some(m) = &self.model
            && !arg_ok(m)
        {
            return Err(format!("model `{m}` 不能为空、不能以 - 开头、不能含控制字符"));
        }
        if let Some(rule) = self.allowed_tools.iter().find(|r| !arg_ok(r)) {
            return Err(format!(
                "allowed_tools 里的 `{rule}` 不能为空、不能以 - 开头、不能含控制字符"
            ));
        }
        if let Some(url) = &self.base_url
            && !(url.starts_with("https://") || url.starts_with("http://"))
        {
            return Err(format!("base_url `{url}` 要以 https:// 或 http:// 开头"));
        }
        Ok(())
    }
}

pub struct ClaudeCodeEngine {
    config: ClaudeCodeConfig,
    api_key: Option<String>,
    /// 每个员工的 `HOME` 放在 `<home_root>/<员工 id>`，`CLAUDE_CONFIG_DIR`（会话记录也在里面）放在它下面的 `claude`，
    /// 都不在工作目录里。
    home_root: PathBuf,
    slots: Semaphore,
}

impl ClaudeCodeEngine {
    /// `api_key` 是从 `api_key_env` 读到的值，由调用方在启动时读取。
    pub fn new(config: ClaudeCodeConfig, api_key: Option<String>, home_root: impl Into<PathBuf>) -> Arc<Self> {
        let slots = Semaphore::new(config.max_concurrency.max(1));
        Arc::new(Self {
            config,
            api_key,
            home_root: home_root.into(),
            slots,
        })
    }

    /// 一轮的命令行参数。新消息从标准输入传，系统提示词放在 `prompt_file` 里。
    fn args(&self, session: Option<&str>, prompt_file: &Path, mcp: Option<&McpGrant>) -> Vec<String> {
        let mut a: Vec<String> = [
            "-p",
            "--bare",
            "--restricted",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-mode",
            "acceptEdits",
            "--permission-prompts",
            "none",
            "--tools",
            self.config.access.tools(),
            // 只用这里给的 MCP 服务器，不读工作目录里的 .mcp.json。
            "--strict-mcp-config",
        ]
        .map(String::from)
        .into();
        // 续接时也给：Claude Code 记下第一轮的系统提示词后沿用，只在压缩上下文后重新生成，那时要用到。
        a.push("--append-system-prompt-file".into());
        a.push(prompt_file.display().to_string());
        if let Some(m) = &self.config.model {
            a.push("--model".into());
            a.push(m.clone());
        }
        if let Some(grant) = mcp {
            // 令牌不写进参数（`ps` 看得到），用 `${IP_MCP_TOKEN}` 让 Claude Code 从环境变量展开。
            let servers = json!({
                "mcpServers": {
                    MCP_NAME: {
                        "type": "http",
                        "url": grant.url,
                        "headers": { "Authorization": "Bearer ${IP_MCP_TOKEN}" },
                    }
                }
            });
            a.push("--mcp-config".into());
            a.push(servers.to_string());
            // 组织工具已经由本轮令牌限定范围，一律放行。
            a.push("--allowedTools".into());
            a.push(format!("mcp__{MCP_NAME}"));
        }
        for rule in &self.config.allowed_tools {
            a.push("--allowedTools".into());
            a.push(rule.clone());
        }
        if let Some(id) = session {
            a.push("--resume".into());
            a.push(id.into());
        }
        a
    }

    async fn run(&self, ctx: TurnContext<'_>) -> TurnReport {
        let started = Instant::now();
        let timeout = self.turn_timeout();
        let fail = |e: String| TurnReport {
            error: Some(e),
            ..TurnReport::default()
        };
        let _slot = match process::acquire(&self.slots, timeout, KIND.name).await {
            Ok(slot) => slot,
            Err(e) => return fail(e),
        };
        let Some(workspace) = ctx.workspace else {
            return fail("Claude Code 引擎需要工作目录：这个员工的 tools 不能设为 false".into());
        };
        let home = self.home_root.join(ctx.agent.id.as_str());
        let config_dir = home.join("claude");
        let prompts = home.join("prompts");
        for dir in [&config_dir, &prompts] {
            if let Err(e) = std::fs::create_dir_all(dir) {
                return fail(format!("创建 {} 失败：{e}", dir.display()));
            }
        }
        // 每个对话一个文件：同一员工的几个对话可能同时在跑。
        let prompt_file = prompts.join(format!("{}.md", file_name(ctx.thread.as_str())));
        if let Err(e) = std::fs::write(&prompt_file, ctx.system_prompt) {
            return fail(format!("写 {} 失败：{e}", prompt_file.display()));
        }

        let mut cmd = process::command(&self.config.command, workspace);
        cmd.args(self.args(ctx.session, &prompt_file, ctx.mcp.as_ref()))
            .env("HOME", &home)
            .env("CLAUDE_CONFIG_DIR", &config_dir)
            // 不自动升级，不发遥测和错误报告。
            .env("DISABLE_AUTOUPDATER", "1")
            .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1");
        if let Some(key) = &self.api_key {
            cmd.env("ANTHROPIC_API_KEY", key);
        }
        if let Some(url) = &self.config.base_url {
            cmd.env("ANTHROPIC_BASE_URL", url);
        }
        if let Some(grant) = &ctx.mcp {
            cmd.env("IP_MCP_TOKEN", &grant.token);
        }
        let mut report = process::drive(&KIND, cmd, ctx.input.to_string(), started, timeout, &ctx).await;
        // `usage` 只算这一次进程，直接计费；会话累计值（存进引擎会话表）在上一轮的基础上往上加。
        report.cost_usd = self.config.price.map_or(0.0, |p| p.cost(&report.usage));
        let resumed = ctx.session.is_some() && report.session_id.as_deref().is_none_or(|id| Some(id) == ctx.session);
        let baseline = if resumed { ctx.session_usage } else { Usage::default() };
        report.session_usage = Some(process::add_usage(baseline, report.usage));
        let _ = std::fs::remove_file(&prompt_file);
        report
    }
}

impl Engine for ClaudeCodeEngine {
    fn kind(&self) -> &str {
        "claude_code"
    }

    fn turn_timeout(&self) -> Duration {
        Duration::from_secs(self.config.turn_timeout_secs.max(1))
    }

    fn turn<'a>(&'a self, ctx: TurnContext<'a>) -> BoxFuture<'a, TurnReport> {
        Box::pin(self.run(ctx))
    }
}

/// 对话 id 用作文件名时只留字母、数字、`-`、`_`。
fn file_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn tokens(v: &Value, key: &str) -> u32 {
    v.get(key)
        .and_then(Value::as_u64)
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// 把 `claude -p --output-format stream-json --verbose` 的一行变成动作（D-36）。认不出的行返回空。
fn parse_line(line: &str) -> Vec<Action> {
    let Ok(event) = serde_json::from_str::<Value>(line.trim()) else {
        if !line.trim().is_empty() {
            tracing::debug!(line, "Claude Code 输出了非 JSON 的一行");
        }
        return vec![];
    };
    // 子智能体内部的消息带着发起它的工具调用 id；对话里只记顶层的调用和结果。
    if event.get("parent_tool_use_id").is_some_and(|p| !p.is_null()) {
        return vec![];
    }
    match str_of(&event, "type") {
        "system" if str_of(&event, "subtype") == "init" => {
            let mut out: Vec<Action> = event
                .get("session_id")
                .and_then(Value::as_str)
                .map(|id| Action::Session(id.to_string()))
                .into_iter()
                .collect();
            // 组织工具连不上时这一轮照样跑，但员工交不回任务：记下来，没正常结束时附在失败原因里。
            for s in event.get("mcp_servers").and_then(Value::as_array).into_iter().flatten() {
                let status = str_of(s, "status");
                if status != "connected" {
                    out.push(Action::Warning(format!(
                        "MCP 服务器 {} 没连上（{status}）",
                        str_of(s, "name")
                    )));
                }
            }
            out
        }
        "assistant" => blocks(&event)
            .filter_map(|b| match str_of(b, "type") {
                "text" => {
                    let text = str_of(b, "text");
                    (!text.trim().is_empty()).then(|| Action::Append(Body::Text { text: text.to_string() }))
                }
                "tool_use" => Some(Action::Append(Body::ToolCall {
                    call_id: str_of(b, "id").to_string(),
                    name: tool_name(str_of(b, "name")),
                    input: b.get("input").cloned().unwrap_or_else(|| json!({})),
                })),
                // 思考过程不记。
                _ => None,
            })
            .collect(),
        "user" => blocks(&event)
            .filter(|b| str_of(b, "type") == "tool_result")
            .map(|b| {
                Action::Append(Body::ToolResult {
                    call_id: str_of(b, "tool_use_id").to_string(),
                    ok: !b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
                    output: clip(&content_text(b.get("content"))),
                })
            })
            .collect(),
        "result" => {
            // `usage` 只算这一次进程（续接时不含之前几轮，2.1.330 核对过）。Anthropic 的 input_tokens 不含缓存的部分。
            let u = event.get("usage").cloned().unwrap_or_default();
            let (fresh, read, written) = (
                tokens(&u, "input_tokens"),
                tokens(&u, "cache_read_input_tokens"),
                tokens(&u, "cache_creation_input_tokens"),
            );
            let mut out = vec![Action::Usage(Usage {
                prompt_tokens: fresh.saturating_add(read).saturating_add(written),
                cached_tokens: read,
                cache_write_tokens: written,
                completion_tokens: tokens(&u, "output_tokens"),
            })];
            if let Some(id) = event.get("session_id").and_then(Value::as_str) {
                out.push(Action::Session(id.to_string()));
            }
            // 模型接口报错时 subtype 仍是 success，要看 is_error（2.1.330 核对过）。
            let failed = event.get("is_error").and_then(Value::as_bool).unwrap_or(false)
                || str_of(&event, "subtype").starts_with("error");
            out.push(if failed {
                let reason = str_of(&event, "result").trim();
                Action::Failed(if reason.is_empty() {
                    str_of(&event, "subtype").to_string()
                } else {
                    reason.to_string()
                })
            } else {
                Action::Completed
            });
            out
        }
        other => {
            tracing::debug!(event = other, "忽略 Claude Code 事件");
            vec![]
        }
    }
}

/// 一条消息的内容块。
fn blocks(event: &Value) -> impl Iterator<Item = &Value> {
    event
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

/// 内置工具记成 `claude.<工具>`（和 Codex 的 `codex.shell` 对应）；MCP 工具本来就叫 `mcp__<服务器>__<工具>`，不改。
fn tool_name(name: &str) -> String {
    if name.starts_with("mcp__") {
        name.to_string()
    } else {
        format!("claude.{name}")
    }
}

/// 工具结果的文字：可能是一个字符串，也可能是几块内容。
fn content_text(content: Option<&Value>) -> String {
    match content {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .map(|p| match str_of(p, "type") {
                "text" => str_of(p, "text").to_string(),
                "image" => "[图片]".to_string(),
                _ => p.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> ClaudeCodeConfig {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn config_defaults_and_checks() {
        let c = config(r#"{"api_key_env": "ANTHROPIC_API_KEY"}"#);
        assert_eq!(c.command, "claude");
        assert_eq!(c.access, ClaudeCodeAccess::WorkspaceWrite);
        assert_eq!((c.turn_timeout_secs, c.max_concurrency), (1800, 4));
        assert!(c.allowed_tools.is_empty() && c.base_url.is_none() && c.validate().is_ok());
        assert!(serde_json::from_str::<ClaudeCodeConfig>(r#"{"api_key_env": "K", "access": "read-only"}"#).is_ok());
        assert!(serde_json::from_str::<ClaudeCodeConfig>(r#"{"api_key_env": "K", "access": "full"}"#).is_err());
        assert!(serde_json::from_str::<ClaudeCodeConfig>(r#"{"api_key_env": "K", "sandbox": "read-only"}"#).is_err());
        assert!(
            serde_json::from_str::<ClaudeCodeConfig>("{}").is_err(),
            "没有 api_key_env"
        );
        for bad in [
            r#"{"api_key_env": "K", "allowed_tools": ["--dangerously-skip-permissions"]}"#,
            r#"{"api_key_env": "K", "allowed_tools": [""]}"#,
            r#"{"api_key_env": "K", "allowed_tools": ["Bash(ls)\nRead"]}"#,
            r#"{"api_key_env": "K", "model": "-x"}"#,
            r#"{"api_key_env": "K", "base_url": "ftp://x"}"#,
        ] {
            assert!(config(bad).validate().is_err(), "{bad}");
        }
        assert!(
            config(
                r#"{"api_key_env": "K", "allowed_tools": ["Bash(cargo test:*)"], "base_url": "https://gw.example.com"}"#
            )
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn first_turn_and_resume_args() {
        let e = ClaudeCodeEngine::new(
            config(r#"{"api_key_env": "K", "model": "claude-test", "allowed_tools": ["Bash(cargo test:*)"]}"#),
            None,
            "/tmp/ip-engines",
        );
        let grant = McpGrant {
            url: "http://127.0.0.1:7878/mcp".into(),
            token: "ipt_secret".into(),
        };
        let first = e.args(None, Path::new("/h/prompts/t1.md"), Some(&grant));
        let line = first.join(" ");
        assert!(
            line.starts_with(
                "-p --bare --restricted --output-format stream-json --verbose --permission-mode acceptEdits --permission-prompts none --tools Bash,Edit,Read --strict-mcp-config --append-system-prompt-file /h/prompts/t1.md --model claude-test --mcp-config "
            ),
            "{line}"
        );
        assert!(
            line.ends_with("--allowedTools mcp__ip --allowedTools Bash(cargo test:*)"),
            "{line}"
        );
        assert!(
            !line.contains("--resume") && !line.contains("ipt_secret"),
            "令牌不进参数：{line}"
        );
        let mcp: Value =
            serde_json::from_str(&first[first.iter().position(|a| a == "--mcp-config").unwrap() + 1]).unwrap();
        assert_eq!(mcp["mcpServers"]["ip"]["url"], "http://127.0.0.1:7878/mcp");
        assert_eq!(
            mcp["mcpServers"]["ip"]["headers"]["Authorization"],
            "Bearer ${IP_MCP_TOKEN}"
        );

        let again = e.args(Some("sess-1"), Path::new("/h/prompts/t1.md"), None).join(" ");
        assert!(
            again.ends_with("--allowedTools Bash(cargo test:*) --resume sess-1"),
            "{again}"
        );
        assert!(
            again.contains("--append-system-prompt-file") && !again.contains("--mcp-config"),
            "{again}"
        );

        let read_only = ClaudeCodeEngine::new(config(r#"{"api_key_env": "K", "access": "read-only"}"#), None, "/tmp");
        assert!(
            read_only
                .args(None, Path::new("/p"), None)
                .join(" ")
                .contains("--tools Read --strict-mcp-config"),
            "只读时只给 Read"
        );
    }

    /// 下面的行取自 Claude Code 2.1.330 连假模型接口时的真实输出（2026-11-21），删掉了无关的字段。
    #[test]
    fn parses_stream_json() {
        let init = parse_line(
            r#"{"type":"system","subtype":"init","cwd":"/w","session_id":"85f75368-442d-4d06-aa33-cdfef8abce14","tools":["Bash","Edit","Read"],"mcp_servers":[{"name":"ip","status":"failed","source":"dynamic"}],"model":"claude-test","permissionMode":"acceptEdits","apiKeySource":"ANTHROPIC_API_KEY","claude_code_version":"2.1.330"}"#,
        );
        assert_eq!(
            init,
            [
                Action::Session("85f75368-442d-4d06-aa33-cdfef8abce14".into()),
                Action::Warning("MCP 服务器 ip 没连上（failed）".into())
            ]
        );
        assert!(parse_line(r#"{"type":"system","subtype":"ui_invalidate","session_id":"x"}"#).is_empty());
        assert_eq!(
            parse_line(
                r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"text","text":"我先跑一下。"}],"usage":{"input_tokens":1000,"output_tokens":1}},"parent_tool_use_id":null,"session_id":"s"}"#
            ),
            [Action::Append(Body::Text {
                text: "我先跑一下。".into()
            })]
        );
        assert_eq!(
            parse_line(
                r#"{"type":"assistant","message":{"id":"msg_1","role":"assistant","content":[{"type":"tool_use","id":"toolu_1_1","name":"Bash","input":{"command":"echo hi > a.txt && cat a.txt","description":"写一个文件"}}]},"parent_tool_use_id":null,"session_id":"s"}"#
            ),
            [Action::Append(Body::ToolCall {
                call_id: "toolu_1_1".into(),
                name: "claude.Bash".into(),
                input: json!({"command": "echo hi > a.txt && cat a.txt", "description": "写一个文件"})
            })]
        );
        assert_eq!(
            parse_line(
                r#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"toolu_1_1","type":"tool_result","content":"hi","is_error":false}]},"parent_tool_use_id":null,"session_id":"s","tool_use_result":{"stdout":"hi","stderr":""}}"#
            ),
            [Action::Append(Body::ToolResult {
                call_id: "toolu_1_1".into(),
                ok: true,
                output: "hi".into()
            })]
        );
        // 被拒绝的工具调用记成失败的工具结果。
        let denied = parse_line(
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"touch in '/tmp/x' needs approval. The path is outside the working directories for this session ('/w').","is_error":true,"tool_use_id":"toolu_3_0"}]},"parent_tool_use_id":null,"session_id":"s"}"#,
        );
        assert!(
            matches!(&denied[..], [Action::Append(Body::ToolResult { ok: false, output, .. })] if output.contains("needs approval")),
            "{denied:?}"
        );
        // MCP 工具名不改；内容是几块时拼起来。
        let mcp = parse_line(
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"..."},{"type":"tool_use","id":"t2","name":"mcp__ip__update_task","input":{"task_id":"t1"}}]},"parent_tool_use_id":null}"#,
        );
        assert!(matches!(&mcp[..], [Action::Append(Body::ToolCall { name, .. })] if name == "mcp__ip__update_task"));
        let parts = parse_line(
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"任务已更新"},{"type":"image","source":{}}]}]},"parent_tool_use_id":null}"#,
        );
        assert!(
            matches!(&parts[..], [Action::Append(Body::ToolResult { ok: true, output, .. })] if output == "任务已更新\n[图片]"),
            "{parts:?}"
        );
        // 子智能体里的消息不记。
        assert!(
            parse_line(r#"{"type":"assistant","message":{"content":[{"type":"text","text":"子任务"}]},"parent_tool_use_id":"toolu_9"}"#)
                .is_empty()
        );

        // 结束：用量只算这一次进程，缓存的部分要加回输入里；不看 total_cost_usd。
        let done = parse_line(
            r#"{"type":"result","subtype":"success","is_error":false,"num_turns":2,"result":"完成。","session_id":"s","total_cost_usd":0.02616,"usage":{"input_tokens":2000,"cache_creation_input_tokens":600,"cache_read_input_tokens":400,"output_tokens":100},"permission_denials":[]}"#,
        );
        assert_eq!(
            done,
            [
                Action::Usage(Usage {
                    prompt_tokens: 3000,
                    cached_tokens: 400,
                    cache_write_tokens: 600,
                    completion_tokens: 100
                }),
                Action::Session("s".into()),
                Action::Completed
            ]
        );
        // 模型接口报错：subtype 还是 success，is_error 为 true。
        let api_error = parse_line(
            r#"{"type":"result","subtype":"success","is_error":true,"api_error_status":400,"result":"API Error: 400 请求不对","session_id":"s","usage":{"input_tokens":0,"output_tokens":0}}"#,
        );
        assert_eq!(
            api_error.last(),
            Some(&Action::Failed("API Error: 400 请求不对".into()))
        );
        // 续接时找不到会话：没有 result 文字，原因在 stderr 里（由 process::drive 附上）。
        let lost = parse_line(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":null,"session_id":"s","usage":{"input_tokens":0,"output_tokens":0}}"#,
        );
        assert_eq!(lost.last(), Some(&Action::Failed("error_during_execution".into())));

        assert!(parse_line("not json").is_empty());
        assert!(parse_line(r#"{"type":"rate_limit_event"}"#).is_empty());
    }

    #[test]
    fn thread_ids_become_safe_file_names() {
        assert_eq!(file_name("th_01JX-abc"), "th_01JX-abc");
        assert_eq!(file_name("../x/y"), "___x_y");
    }
}
