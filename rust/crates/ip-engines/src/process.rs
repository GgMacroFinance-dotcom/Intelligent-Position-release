//! 各种引擎共用的进程管理（`docs/ip/m2-spec.md` D-22、D-36）：排队等空位、起进程、在单独的任务里写标准输入、
//! 逐行解析输出并写回对话、超时整组终止、收 stderr 末尾，最后按解析出的动作拼出这一轮的结果。
//! 各引擎只管命令行参数、环境变量和怎么解析一行输出。

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use ip_core::Body;
use ip_models::Usage;
use ip_runtime::engine::{TurnContext, TurnReport};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{Semaphore, SemaphorePermit};

/// 写进对话的工具输出上限（字节）。完整输出在引擎自己的会话记录里。
pub(crate) const MAX_OUTPUT: usize = 16 * 1024;
/// 出错时附上的 stderr 末尾（字节）。
const MAX_STDERR: usize = 4 * 1024;

/// 解析一行输出得到的动作。
#[derive(Debug, PartialEq)]
pub(crate) enum Action {
    /// 引擎的会话 id，下一轮用它续接。
    Session(String),
    /// 以员工的名义写进对话。
    Append(Body),
    /// 这一轮的用量，可以出现多次，累加。
    Usage(Usage),
    /// 这一轮正常结束。
    Completed,
    /// 这一轮失败。
    Failed(String),
    /// 中途报的错，之后常常照样跑完，只记下来（如 codex-cli 0.169.0 的 `Reconnecting... 2/5`）。
    /// 这一轮有没有失败看 [`Action::Failed`] 和 [`Action::Completed`]。
    Warning(String),
}

/// 一种引擎在进程这一层的差别。
pub(crate) struct Kind {
    /// 出错信息里的名字，如 `Codex`。
    pub name: &'static str,
    /// 表示一轮结束的输出事件，缺了它时写进失败原因。
    pub end_event: &'static str,
    /// 把一行输出变成动作；认不出的行返回空。
    pub parse: fn(&str) -> Vec<Action>,
}

/// 等一个空位。一轮的时限从这里开始算：本轮令牌的期限按 `timeout` 定，不能被排队时间吃掉。
pub(crate) async fn acquire<'s>(
    slots: &'s Semaphore,
    timeout: Duration,
    name: &str,
) -> Result<SemaphorePermit<'s>, String> {
    match tokio::time::timeout(timeout, slots.acquire()).await {
        Ok(Ok(slot)) => Ok(slot),
        Ok(Err(_)) => Err(format!("{name} 引擎已关闭")),
        Err(_) => Err(format!("排队等 {name} 空位超过 {} 秒", timeout.as_secs())),
    }
}

/// 起一个在工作目录里运行、环境变量清空（只留 `PATH`）、单独成一个进程组的命令。调用方再加参数和环境变量。
pub(crate) fn command(program: &str, workspace: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.current_dir(workspace)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .process_group(0);
    if let Some(path) = std::env::var_os("PATH") {
        cmd.env("PATH", path);
    }
    cmd
}

/// 跑这一轮的进程：`prompt` 写进标准输入，输出逐行交给 `kind.parse`，到 `started + timeout` 还没结束就整组终止。
///
/// 结果里的 `usage` 是进程报的原样累加，`cost_usd` 和 `session_usage` 留给引擎填：
/// Codex 续接时报的是会话累计（要减掉以前几轮，backlog E-10），Claude Code 报的只是这一次进程。
pub(crate) async fn drive(
    kind: &Kind,
    mut cmd: Command,
    prompt: String,
    started: Instant,
    timeout: Duration,
    ctx: &TurnContext<'_>,
) -> TurnReport {
    let name = kind.name;
    let mut report = TurnReport::default();
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            report.error = Some(format!(
                "启动 {} 失败：{e}",
                cmd.as_std().get_program().to_string_lossy()
            ));
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
            for action in (kind.parse)(&line) {
                state.apply(action, ctx, name).await;
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
    report.usage = state.usage;
    report.session_id = state.session;
    report.error = match status {
        Err(_) => {
            if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
                let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGKILL);
            }
            Some(format!("{name} 超过 {} 秒没有完成，已终止", timeout.as_secs()))
        }
        Ok(Err(e)) => Some(format!("等待 {name} 退出失败：{e}")),
        Ok(Ok(exit)) => {
            let last = state
                .warning
                .map(|w| format!("；最后一条错误：{w}"))
                .unwrap_or_default();
            match (state.failure, state.completed) {
                (Some(e), _) => {
                    // 报错的原因有时只在 stderr 里（如 Claude Code 续接时找不到会话），非零退出时附上最后一行。
                    let hint = stderr
                        .lines()
                        .rev()
                        .map(str::trim)
                        .find(|l| !l.is_empty())
                        .filter(|l| !exit.success() && !e.contains(l))
                        .map(|l| format!("（{l}）"))
                        .unwrap_or_default();
                    Some(format!("{name} 报错：{e}{hint}"))
                }
                (None, true) => None,
                (None, false) if exit.success() => {
                    Some(format!("{name} 没有报告这一轮结束（缺少 {}）{last}", kind.end_event))
                }
                (None, false) => Some(format!("{name} 异常退出（{exit}）：{}{last}", stderr.trim())),
            }
        }
    };
    report
}

/// 读完 stderr，只留末尾。
async fn tail(mut stderr: tokio::process::ChildStderr) -> String {
    let mut buf = Vec::new();
    let _ = stderr.read_to_end(&mut buf).await;
    let start = buf.len().saturating_sub(MAX_STDERR);
    String::from_utf8_lossy(&buf[start..]).into_owned()
}

/// 两份用量相加。
pub(crate) fn add_usage(a: Usage, b: Usage) -> Usage {
    Usage {
        prompt_tokens: a.prompt_tokens.saturating_add(b.prompt_tokens),
        cached_tokens: a.cached_tokens.saturating_add(b.cached_tokens),
        cache_write_tokens: a.cache_write_tokens.saturating_add(b.cache_write_tokens),
        completion_tokens: a.completion_tokens.saturating_add(b.completion_tokens),
    }
}

/// 写进对话前截断工具输出。
pub(crate) fn clip(text: &str) -> String {
    ip_tools::truncate_output(text.to_string(), MAX_OUTPUT, 0)
}

#[derive(Default)]
struct Turn {
    session: Option<String>,
    usage: Usage,
    steps: u32,
    completed: bool,
    failure: Option<String>,
    /// 最近一条中途报的错；这一轮没正常结束时附在失败原因里。
    warning: Option<String>,
}

impl Turn {
    async fn apply(&mut self, action: Action, ctx: &TurnContext<'_>, engine: &str) {
        match action {
            Action::Session(id) => self.session = Some(id),
            Action::Append(body) => {
                // 步数等于工具调用数加回复条数。
                if !matches!(body, Body::ToolResult { .. }) {
                    self.steps += 1;
                }
                if let Err(e) = ctx.sink.append(body).await {
                    tracing::warn!(engine, error = %e, "写回引擎的输出失败");
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
                tracing::warn!(engine, warning = %w, "引擎报了一条错误，这一轮继续");
                self.warning = Some(w);
            }
        }
    }
}
