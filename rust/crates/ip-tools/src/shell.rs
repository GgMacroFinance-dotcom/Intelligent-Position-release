//! `shell` 工具：执行命令，合并 stdout/stderr，限时、限输出。

use std::process::Stdio;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::{Sandbox, ToolOutput, Toolbox, truncate_output};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    command: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
}

/// 子进程退出后，等残留输出读完的最长时间。
const DRAIN_GRACE: Duration = Duration::from_secs(2);

pub async fn run(tb: &Toolbox, a: Args) -> ToolOutput {
    let timeout = a
        .timeout_secs
        .map(|s| Duration::from_secs(s.max(1)))
        .unwrap_or(tb.limits.timeout)
        .min(tb.limits.max_timeout);

    let mut cmd = tb.sandbox.command(tb.workspace(), &a.command);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolOutput::err(format!("启动命令失败：{e}")),
    };
    let pid = child.id();

    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(32);
    let mut readers: Vec<JoinHandle<()>> = Vec::with_capacity(2);
    if let Some(out) = child.stdout.take() {
        readers.push(tokio::spawn(pump(out, tx.clone())));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(tokio::spawn(pump(err, tx.clone())));
    }
    drop(tx);

    // 一边等进程一边收输出，超过上限的部分继续读走但丢弃，避免子进程卡在写管道上。
    let max = tb.limits.max_output;
    let collector = tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut dropped = 0usize;
        while let Some(chunk) = rx.recv().await {
            let room = max.saturating_sub(buf.len());
            let take = room.min(chunk.len());
            buf.extend_from_slice(&chunk[..take]);
            dropped += chunk.len() - take;
        }
        (buf, dropped)
    });

    let (status, timed_out) = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(s)) => (Some(s), false),
        Ok(Err(e)) => return ToolOutput::err(format!("等待命令失败：{e}")),
        Err(_) => {
            kill_tree(&tb.sandbox, pid);
            let _ = child.start_kill();
            (child.wait().await.ok(), true)
        }
    };
    // 不隔离时，后台子进程（`cmd &`）可能还握着管道，整组清理。bwrap 的 pid 命名空间会随主进程一起结束。
    kill_tree(&tb.sandbox, pid);

    let drain = async {
        for r in &mut readers {
            let _ = r.await;
        }
    };
    if tokio::time::timeout(DRAIN_GRACE, drain).await.is_err() {
        for r in &readers {
            r.abort();
        }
    }
    let (buf, dropped) = collector.await.unwrap_or_default();
    let mut text = truncate_output(String::from_utf8_lossy(&buf).into_owned(), max, dropped);

    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if timed_out {
        text.push_str(&format!("[超时 {} 秒，已终止]", timeout.as_secs()));
        return ToolOutput::err(text);
    }
    match status.and_then(|s| s.code()) {
        Some(0) => {
            text.push_str("[退出码 0]");
            ToolOutput::ok(text)
        }
        Some(code) => {
            text.push_str(&format!("[退出码 {code}]"));
            ToolOutput::err(text)
        }
        None => {
            text.push_str("[被信号终止]");
            ToolOutput::err(text)
        }
    }
}

async fn pump(mut r: impl AsyncRead + Unpin, tx: mpsc::Sender<Vec<u8>>) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
        }
    }
}

fn kill_tree(sandbox: &Sandbox, pid: Option<u32>) {
    if let (Sandbox::None, Some(pid)) = (sandbox, pid)
        && let Ok(pid) = i32::try_from(pid)
    {
        // 进程组号等于 sh 的 pid（process_group(0)）。组已不存在时返回 ESRCH，忽略。
        let _ = nix::sys::signal::killpg(nix::unistd::Pid::from_raw(pid), nix::sys::signal::Signal::SIGKILL);
    }
}
