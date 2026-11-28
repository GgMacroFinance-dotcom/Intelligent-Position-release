//! 外部引擎端到端：用假的 `codex`、`claude`（同一个 bash 脚本）跑通 D-20 到 D-25 和 D-36。
//!
//! 假引擎把每次调用的参数、标准输入、环境变量记到文件里，再输出事先写好的 JSONL；
//! 需要时执行一段场景脚本（睡过头、往 stderr 写错误、用 curl 调 `/mcp`）。

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use ip_core::{Agent, Author, Body, ModelSlot, Notice, OrgSpec, Role, TaskStatus, ThreadId};
use ip_models::ModelRegistry;
use ip_runtime::{AgentSetup, NewTask, Runtime, RuntimeConfig, RuntimeEvent};
use ip_tools::{Limits, Sandbox, Toolbox};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

use crate::{App, router};

const FAKE_ENGINE: &str = r#"#!/usr/bin/env bash
dir="__DIR__"
n=$(( $(cat "$dir/calls" 2>/dev/null || echo 0) + 1 ))
echo "$n" > "$dir/calls"
printf '%s\n' "$@" > "$dir/args.$n"
cat > "$dir/stdin.$n"
env > "$dir/env.$n"
pwd > "$dir/pwd.$n"
if [ -f "$dir/script.$n.sh" ]; then source "$dir/script.$n.sh"; fi
if [ -f "$dir/out.$n.jsonl" ]; then cat "$dir/out.$n.jsonl"; fi
exit "$(cat "$dir/exit.$n" 2>/dev/null || echo 0)"
"#;

struct Fixture {
    _dir: tempfile::TempDir,
    /// 假引擎的记录和场景文件。
    fake: PathBuf,
    data: PathBuf,
    workspace: PathBuf,
    app: App,
}

impl Fixture {
    /// 第 `n` 次调用要输出的 JSONL。
    fn out(&self, n: u32, lines: &[serde_json::Value]) {
        let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(self.fake.join(format!("out.{n}.jsonl")), text).unwrap();
    }

    /// 第 `n` 次调用先执行的 bash 片段。
    fn script(&self, n: u32, body: &str) {
        std::fs::write(self.fake.join(format!("script.{n}.sh")), body).unwrap();
    }

    fn exit(&self, n: u32, code: i32) {
        std::fs::write(self.fake.join(format!("exit.{n}")), code.to_string()).unwrap();
    }

    fn recorded(&self, what: &str, n: u32) -> String {
        std::fs::read_to_string(self.fake.join(format!("{what}.{n}"))).unwrap_or_default()
    }

    fn runtime(&self) -> &Runtime {
        self.app.runtime()
    }
}

/// 一个运维测试员工 ops-1，岗位用 Codex 引擎；服务在本机随机端口上监听，`/mcp` 可用。
async fn fixture(codex: serde_json::Value) -> Fixture {
    fixture_with("codex", |bin, home_root| {
        let mut config = codex;
        config["command"] = serde_json::Value::String(bin.display().to_string());
        ip_engines::CodexEngine::new(serde_json::from_value(config).unwrap(), None, home_root)
    })
    .await
}

/// 同上，岗位用 Claude Code 引擎，API key 是 `sk-ant-test`。
async fn claude_fixture(claude: serde_json::Value) -> Fixture {
    fixture_with("claude", |bin, home_root| {
        let mut config = claude;
        config["command"] = serde_json::Value::String(bin.display().to_string());
        config["api_key_env"] = "ANTHROPIC_API_KEY".into();
        let config: ip_engines::ClaudeCodeConfig = serde_json::from_value(config).unwrap();
        config.validate().unwrap();
        ip_engines::ClaudeCodeEngine::new(config, Some("sk-ant-test".into()), home_root)
    })
    .await
}

/// `engine` 拿到假引擎可执行文件的路径和 `data_dir/engines`，造出这个岗位的引擎。
async fn fixture_with(
    program: &str,
    engine: impl FnOnce(&Path, PathBuf) -> Arc<dyn ip_runtime::engine::Engine>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("fake");
    let data = dir.path().join("data");
    let workspace = dir.path().join("project");
    for d in [&fake, &data, &workspace] {
        std::fs::create_dir_all(d).unwrap();
    }
    let bin = fake.join(program);
    std::fs::write(&bin, FAKE_ENGINE.replace("__DIR__", &fake.display().to_string())).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let engine = engine(&bin, data.join("engines"));

    let store = ip_store::testing::store_at(data.join("ip.db")).await;
    let rc = RuntimeConfig {
        org_prompt: "组织规则：结论先行。".into(),
        ..RuntimeConfig::default()
    };
    let runtime = Runtime::new(store, ModelRegistry::new(), rc);
    let role = Role {
        id: "ops".into(),
        name: Some("运维测试".into()),
        description: "维护服务器，修测试。".into(),
        model: ModelSlot {
            default: "unused/m".into(),
            escalate: None,
        },
        budget_per_day_usd: None,
        coordinator: false,
        approve_completion: false,
    };
    runtime
        .spawn_agent(AgentSetup {
            agent: Agent {
                id: "ops-1".into(),
                role: "ops".into(),
                name: "小维".into(),
                celestial_style: None,
            },
            role: role.clone(),
            toolbox: Some(Toolbox::new(&workspace, Sandbox::None, Limits::default()).unwrap()),
            mcp: None,
            data: None,
            engine: Some(engine),
        })
        .unwrap();
    let app = App::new(runtime, OrgSpec { roles: vec![role] }, None);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    app.runtime()
        .set_mcp_url(format!("http://127.0.0.1:{}/mcp", addr.port()));
    let r = router(Some(app.clone()));
    tokio::spawn(async move { axum::serve(listener, r).await.unwrap() });
    Fixture {
        _dir: dir,
        fake,
        data,
        workspace,
        app,
    }
}

/// 等 ops-1 的一轮结束，返回结束事件。
async fn turn_end(rx: &mut broadcast::Receiver<RuntimeEvent>) -> RuntimeEvent {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("等一轮结束超时")
            .unwrap();
        if matches!(&event, RuntimeEvent::TurnFinished { agent_id, .. } | RuntimeEvent::TurnFailed { agent_id, .. } if agent_id.as_str() == "ops-1")
        {
            return event;
        }
    }
}

fn human() -> Author {
    Author::human("张三")
}

/// 第一轮带系统提示词新开会话，第二轮用 resume 续接、只交新消息；Codex 的输出按 D-24 记进对话，用量按价格计费。
#[tokio::test]
async fn codex_turns_resume_with_only_new_messages() {
    let f = fixture(serde_json::json!({
        "model": "codex-test",
        "price": {"input": 1.0, "cached_input": 0.5, "output": 2.0}
    }))
    .await;
    f.out(
        1,
        &[
            serde_json::json!({"type": "thread.started", "thread_id": "sess-1"}),
            serde_json::json!({"type": "turn.started"}),
            serde_json::json!({"type": "item.completed", "item": {"id": "i1", "type": "reasoning", "text": "想一想"}}),
            serde_json::json!({"type": "item.completed", "item": {"id": "i2", "type": "command_execution", "command": "sh test.sh", "aggregated_output": "FAIL: growth", "exit_code": 1, "status": "failed"}}),
            serde_json::json!({"type": "item.completed", "item": {"id": "i3", "type": "file_change", "changes": [{"path": "growth.sh", "kind": "update"}], "status": "completed"}}),
            serde_json::json!({"type": "item.completed", "item": {"id": "i4", "type": "agent_message", "text": "修好了，测试通过。"}}),
            serde_json::json!({"type": "turn.completed", "usage": {"input_tokens": 1_000_000, "cached_input_tokens": 400_000, "output_tokens": 100_000}}),
        ],
    );
    f.out(
        2,
        &[
            serde_json::json!({"type": "item.completed", "item": {"id": "j1", "type": "agent_message", "text": "再跑一遍也通过。"}}),
            // 续接时 Codex 报的是整个会话的累计用量（backlog E-10）：这一轮自己只有 20 万输入（10 万命中缓存）、1 万输出。
            serde_json::json!({"type": "turn.completed", "usage": {"input_tokens": 1_200_000, "cached_input_tokens": 500_000, "output_tokens": 110_000}}),
        ],
    );
    // 第三轮报了用量之后又报失败：这一轮的用量照记，但不能在第四轮再记一次。
    f.out(
        3,
        &[
            serde_json::json!({"type": "turn.completed", "usage": {"input_tokens": 1_300_000, "cached_input_tokens": 500_000, "output_tokens": 110_000}}),
            serde_json::json!({"type": "turn.failed", "error": {"message": "额度不足"}}),
        ],
    );
    f.out(
        4,
        &[
            serde_json::json!({"type": "item.completed", "item": {"id": "k1", "type": "agent_message", "text": "好了。"}}),
            serde_json::json!({"type": "turn.completed", "usage": {"input_tokens": 1_300_000, "cached_input_tokens": 500_000, "output_tokens": 120_000}}),
        ],
    );
    let rt = f.runtime();
    let mut rx = rt.subscribe();
    let thread = ThreadId::new();
    rt.send(&"ops-1".into(), thread.clone(), human(), "测试挂了，看看")
        .await
        .unwrap();
    let RuntimeEvent::TurnFinished { cost_usd, steps, .. } = turn_end(&mut rx).await else {
        panic!(
            "第一轮应成功：{:?}",
            rt.store().thread_messages(&thread, None, 20).await
        )
    };
    // 60 万新输入 × 1 + 40 万缓存 × 0.5 + 10 万输出 × 2，每百万 token。
    assert!((cost_usd - 1.0).abs() < 1e-9, "{cost_usd}");
    assert_eq!(steps, 3, "两次工具调用加一条回复");

    let args = f.recorded("args", 1);
    assert!(args.starts_with("exec\n--json\n"), "{args}");
    assert!(
        args.contains("\n-m\ncodex-test\n") && !args.contains("resume"),
        "{args}"
    );
    assert!(args.contains("mcp_servers.ip.url=\"http://127.0.0.1:"), "{args}");
    let stdin = f.recorded("stdin", 1);
    assert!(
        stdin.contains("组织规则：结论先行。") && stdin.contains("张三（") && stdin.contains(" UTC）：测试挂了，看看"),
        "{stdin}"
    );
    let env = f.recorded("env", 1);
    let home = f.data.join("engines/ops-1");
    assert!(
        env.contains(&format!("CODEX_HOME={}", home.join("codex").display())),
        "{env}"
    );
    assert!(env.contains(&format!("HOME={}", home.display())), "{env}");
    assert!(env.contains("IP_MCP_TOKEN=ipt_"), "{env}");
    assert_eq!(f.recorded("pwd", 1).trim(), f.workspace.display().to_string());

    let messages = rt.store().thread_messages(&thread, None, 20).await.unwrap();
    let bodies: Vec<String> = messages
        .iter()
        .map(|m| match &m.body {
            Body::Text { text } => format!("text:{text}"),
            Body::ToolCall { name, .. } => format!("call:{name}"),
            Body::ToolResult { ok, .. } => format!("result:{ok}"),
            Body::Summary { .. } => "summary".into(),
        })
        .collect();
    assert_eq!(
        bodies,
        [
            "text:测试挂了，看看",
            "call:codex.shell",
            "result:false",
            "call:codex.edit",
            "result:true",
            "text:修好了，测试通过。"
        ]
    );
    assert!(messages[1..].iter().all(|m| m.author
        == Author::Agent {
            agent_id: "ops-1".into()
        }));

    rt.send(&"ops-1".into(), thread.clone(), human(), "再跑一遍")
        .await
        .unwrap();
    let RuntimeEvent::TurnFinished { usage, cost_usd, .. } = turn_end(&mut rx).await else {
        panic!("第二轮应当正常结束")
    };
    // 只记这一轮的差值：0.1（没命中的输入）+ 0.05（命中缓存的输入）+ 0.02（输出）。
    assert_eq!(
        (usage.prompt_tokens, usage.cached_tokens, usage.completion_tokens),
        (200_000, 100_000, 10_000)
    );
    assert!((cost_usd - 0.17).abs() < 1e-9, "{cost_usd}");
    let args = f.recorded("args", 2);
    assert!(args.contains("\nresume\nsess-1\n-\n"), "{args}");
    let stdin = f.recorded("stdin", 2);
    // 每条消息带发送时间：张三（2026-11-14 周六 13:22 UTC）：再跑一遍
    let stdin = stdin.trim();
    assert!(
        stdin.lines().count() == 1 && stdin.starts_with("张三（") && stdin.ends_with(" UTC）：再跑一遍"),
        "续接时只交新消息，不再带系统提示词和旧消息：{stdin}"
    );

    let spend = rt.spend_today().await.unwrap();
    assert!((spend.roles[&"ops".into()] - 1.17).abs() < 1e-9, "{spend:?}");

    // 第三轮失败：10 万输入照记（0.1），累计用量也存下来。
    rt.send(&"ops-1".into(), thread.clone(), human(), "还有一处")
        .await
        .unwrap();
    assert!(matches!(turn_end(&mut rx).await, RuntimeEvent::TurnFailed { .. }));
    let spend = rt.spend_today().await.unwrap();
    assert!((spend.roles[&"ops".into()] - 1.27).abs() < 1e-9, "{spend:?}");
    let session = rt
        .store()
        .engine_session(&"ops-1".into(), &thread)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.used.prompt_tokens, 1_300_000);

    // 第四轮：只多了 1 万输出（0.02），第三轮的 10 万输入没有被再记一次。
    rt.send(&"ops-1".into(), thread.clone(), human(), "再试一次")
        .await
        .unwrap();
    let RuntimeEvent::TurnFinished { usage, .. } = turn_end(&mut rx).await else {
        panic!("第四轮应当正常结束")
    };
    assert_eq!((usage.prompt_tokens, usage.completion_tokens), (0, 10_000));
    assert!(f.recorded("args", 4).contains("\nresume\nsess-1\n-\n"));
    let spend = rt.spend_today().await.unwrap();
    assert!((spend.roles[&"ops".into()] - 1.29).abs() < 1e-9, "{spend:?}");
}

/// 超时整组终止，异常退出带上 stderr；派给它的任务标为受阻，原因写回派活的对话。
#[tokio::test]
async fn failed_codex_turns_block_the_task() {
    let f = fixture(serde_json::json!({"turn_timeout_secs": 1})).await;
    f.script(1, "sleep 30\n");
    let rt = f.runtime();
    let mut rx = rt.subscribe();
    let origin = ThreadId::new();
    let task = rt
        .create_task(
            NewTask {
                title: "修 growth 测试".into(),
                assignee: Some("ops-1".into()),
                origin_thread_id: Some(origin.clone()),
                ..Default::default()
            },
            human(),
        )
        .await
        .unwrap();
    let began = std::time::Instant::now();
    let RuntimeEvent::TurnFailed { error, .. } = turn_end(&mut rx).await else {
        panic!("应当超时失败")
    };
    assert!(error.contains("超过 1 秒"), "{error}");
    assert!(
        began.elapsed() < Duration::from_secs(10),
        "超时后应立即终止，不等 sleep 30"
    );
    let task = rt.store().task(&task.id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Blocked);
    let back = rt.store().thread_messages(&origin, None, 10).await.unwrap();
    assert!(back.iter().any(|m| m.notice == Some(Notice::TaskReport)), "{back:?}");
    let work = rt
        .store()
        .thread_messages(task.thread_id.as_ref().unwrap(), None, 10)
        .await
        .unwrap();
    assert!(work.iter().any(|m| m.notice == Some(Notice::TurnFailed)), "{work:?}");

    f.script(2, "echo 'boom: 没有登录' >&2\n");
    f.exit(2, 3);
    rt.send(&"ops-1".into(), ThreadId::new(), human(), "在吗")
        .await
        .unwrap();
    let RuntimeEvent::TurnFailed { error, .. } = turn_end(&mut rx).await else {
        panic!("应当失败")
    };
    assert!(
        error.contains("异常退出") && error.contains("boom: 没有登录"),
        "{error}"
    );
}

/// codex-cli 0.169.0 中途会报 `error`（如 WebSocket 连不上时的 `Reconnecting... 5/5`），随后改走 HTTPS 照样跑完。
/// 跑完了就算成功；没有 `turn.completed` 才算失败，失败原因里带上最后一条错误（2026-11-14 真实验收时看到）。
#[tokio::test]
async fn transient_codex_errors_do_not_fail_the_turn() {
    let f = fixture(serde_json::json!({})).await;
    let retry =
        serde_json::json!({"type": "error", "message": "Reconnecting... 5/5 (stream disconnected before completion)"});
    f.out(
        1,
        &[
            serde_json::json!({"type": "thread.started", "thread_id": "sess-9"}),
            retry.clone(),
            serde_json::json!({"type": "item.completed", "item": {"id": "i1", "type": "agent_message", "text": "测试已修好。"}}),
            serde_json::json!({"type": "turn.completed", "usage": {"input_tokens": 100, "cached_input_tokens": 40, "cache_write_input_tokens": 50, "output_tokens": 10}}),
        ],
    );
    let rt = f.runtime();
    let mut rx = rt.subscribe();
    rt.send(&"ops-1".into(), ThreadId::new(), human(), "测试挂了")
        .await
        .unwrap();
    let RuntimeEvent::TurnFinished { usage, .. } = turn_end(&mut rx).await else {
        panic!("中途的 error 不该让这一轮失败")
    };
    assert_eq!((usage.cached_tokens, usage.cache_write_tokens), (40, 50));

    f.out(2, std::slice::from_ref(&retry));
    rt.send(&"ops-1".into(), ThreadId::new(), human(), "再试一次")
        .await
        .unwrap();
    let RuntimeEvent::TurnFailed { error, .. } = turn_end(&mut rx).await else {
        panic!("没有 turn.completed 应当失败")
    };
    assert!(
        error.contains("缺少 turn.completed") && error.contains("最后一条错误：Reconnecting... 5/5"),
        "{error}"
    );
}

/// Codex 通过 `/mcp` 调 `update_task` 交回任务：任务完成，结果回到派活的对话（D-19、M2-6 的流程骨架）。
#[tokio::test]
async fn codex_hands_back_a_task_through_mcp() {
    let f = fixture(serde_json::json!({})).await;
    f.script(
        1,
        r#"
for a in "$@"; do
  case "$a" in mcp_servers.ip.url=*) url="${a#mcp_servers.ip.url=}"; url="${url//\"/}";; esac
done
# 派活说明的格式：你被分派了任务「…」（id: <任务 id>），派活人：…
task=$(grep -o '（id: [A-Za-z0-9_-]*），派活人' "$dir/stdin.$n" | head -1 | sed -e 's/（id: //' -e 's/），派活人//')
hdr=(-H "Authorization: Bearer $IP_MCP_TOKEN" -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream")
curl -s "${hdr[@]}" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fake-codex","version":"0"}}}' "$url" > "$dir/init.$n"
curl -s "${hdr[@]}" -d "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"update_task\",\"arguments\":{\"task_id\":\"$task\",\"status\":\"done\",\"result\":\"growth 测试已修复\"}}}" "$url" > "$dir/call.$n"
"#,
    );
    f.out(
        1,
        &[
            serde_json::json!({"type": "thread.started", "thread_id": "sess-9"}),
            serde_json::json!({"type": "item.completed", "item": {"id": "k1", "type": "mcp_tool_call", "server": "ip", "tool": "update_task", "arguments": {"status": "done"}, "result": {"content": [{"type": "text", "text": "任务已更新"}]}, "status": "completed"}}),
            serde_json::json!({"type": "item.completed", "item": {"id": "k2", "type": "agent_message", "text": "已交回。"}}),
            serde_json::json!({"type": "turn.completed", "usage": {"input_tokens": 10, "output_tokens": 5}}),
        ],
    );
    let rt = f.runtime();
    let mut rx = rt.subscribe();
    let origin = ThreadId::new();
    let task = rt
        .create_task(
            NewTask {
                title: "修 growth 测试".into(),
                assignee: Some("ops-1".into()),
                origin_thread_id: Some(origin.clone()),
                ..Default::default()
            },
            human(),
        )
        .await
        .unwrap();
    let end = turn_end(&mut rx).await;
    assert!(
        matches!(end, RuntimeEvent::TurnFinished { .. }),
        "{end:?}\ninit: {}\ncall: {}",
        f.recorded("init", 1),
        f.recorded("call", 1)
    );
    let call = f.recorded("call", 1);
    let task = rt.store().task(&task.id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done, "{call}");
    assert_eq!(task.result.as_deref(), Some("growth 测试已修复"));
    let back = rt.store().thread_messages(&origin, None, 10).await.unwrap();
    assert!(
        back.iter()
            .any(|m| m.notice == Some(Notice::TaskReport) && format!("{:?}", m.body).contains("growth 测试已修复")),
        "{back:?}"
    );
    // 一轮结束后，令牌还有 60 秒宽限期，之后作废。
    let env = f.recorded("env", 1);
    let token = env.lines().find_map(|l| l.strip_prefix("IP_MCP_TOKEN=")).unwrap();
    assert!(rt.turn_token_valid(token));
}

/// 假 claude 的场景脚本：把 `--append-system-prompt-file` 指的文件抄一份（一轮结束后引擎会删掉它）。
const COPY_PROMPT: &str = r#"
prev=""
for a in "$@"; do
  if [ "$prev" = "--append-system-prompt-file" ]; then cp "$a" "$dir/prompt.$n"; fi
  prev="$a"
done
"#;

/// Claude Code：系统提示词放文件、新消息走标准输入；第二轮 `--resume` 续接、只交新消息。
/// 输出按 stream-json 记进对话；用量按价格计费，不用它自己报的累计 `total_cost_usd`。
#[tokio::test]
async fn claude_code_turns_resume_with_only_new_messages() {
    let f = claude_fixture(serde_json::json!({
        "model": "claude-test",
        "allowed_tools": ["Bash(sh test.sh)"],
        "price": {"input": 1.0, "cached_input": 0.5, "output": 2.0}
    }))
    .await;
    f.script(1, COPY_PROMPT);
    f.script(2, COPY_PROMPT);
    f.out(
        1,
        &[
            serde_json::json!({"type": "system", "subtype": "init", "cwd": "/w", "session_id": "sess-c1", "tools": ["Bash", "Edit", "Read"], "mcp_servers": [{"name": "ip", "status": "connected"}], "model": "claude-test", "permissionMode": "acceptEdits"}),
            serde_json::json!({"type": "assistant", "message": {"id": "msg_1", "role": "assistant", "content": [{"type": "text", "text": "我先跑一下测试。"}]}, "parent_tool_use_id": null, "session_id": "sess-c1"}),
            serde_json::json!({"type": "assistant", "message": {"id": "msg_1", "role": "assistant", "content": [{"type": "tool_use", "id": "toolu_1", "name": "Bash", "input": {"command": "sh test.sh", "description": "跑测试"}}]}, "parent_tool_use_id": null, "session_id": "sess-c1"}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": [{"tool_use_id": "toolu_1", "type": "tool_result", "content": "FAIL: growth", "is_error": true}]}, "parent_tool_use_id": null, "session_id": "sess-c1"}),
            serde_json::json!({"type": "assistant", "message": {"id": "msg_2", "role": "assistant", "content": [{"type": "thinking", "thinking": "改一行"}, {"type": "tool_use", "id": "toolu_2", "name": "Edit", "input": {"file_path": "growth.sh", "old_string": "a", "new_string": "b"}}]}, "parent_tool_use_id": null, "session_id": "sess-c1"}),
            serde_json::json!({"type": "user", "message": {"role": "user", "content": [{"tool_use_id": "toolu_2", "type": "tool_result", "content": "The file growth.sh has been updated.", "is_error": false}]}, "parent_tool_use_id": null, "session_id": "sess-c1"}),
            serde_json::json!({"type": "assistant", "message": {"id": "msg_3", "role": "assistant", "content": [{"type": "text", "text": "修好了，测试通过。"}]}, "parent_tool_use_id": null, "session_id": "sess-c1"}),
            serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "num_turns": 3, "result": "修好了，测试通过。", "session_id": "sess-c1", "total_cost_usd": 42.0, "usage": {"input_tokens": 600_000, "cache_read_input_tokens": 400_000, "cache_creation_input_tokens": 0, "output_tokens": 100_000}}),
        ],
    );
    f.out(
        2,
        &[
            serde_json::json!({"type": "system", "subtype": "init", "session_id": "sess-c1", "mcp_servers": []}),
            serde_json::json!({"type": "assistant", "message": {"id": "msg_4", "content": [{"type": "text", "text": "再跑一遍也通过。"}]}, "parent_tool_use_id": null}),
            // 续接时 total_cost_usd 是整个会话的累计，usage 只算这一次。
            serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "session_id": "sess-c1", "total_cost_usd": 84.0, "usage": {"input_tokens": 0, "output_tokens": 0}}),
        ],
    );
    let rt = f.runtime();
    let mut rx = rt.subscribe();
    let thread = ThreadId::new();
    rt.send(&"ops-1".into(), thread.clone(), human(), "测试挂了，看看")
        .await
        .unwrap();
    let RuntimeEvent::TurnFinished { cost_usd, steps, .. } = turn_end(&mut rx).await else {
        panic!(
            "第一轮应成功：{:?}",
            rt.store().thread_messages(&thread, None, 20).await
        )
    };
    // 60 万新输入 × 1 + 40 万缓存 × 0.5 + 10 万输出 × 2，每百万 token。
    assert!((cost_usd - 1.0).abs() < 1e-9, "{cost_usd}");
    assert_eq!(steps, 4, "两次工具调用加两条回复");

    let args = f.recorded("args", 1);
    assert!(
        args.starts_with("-p\n--bare\n--restricted\n--output-format\nstream-json\n--verbose\n"),
        "{args}"
    );
    assert!(
        args.contains("\n--permission-mode\nacceptEdits\n--permission-prompts\nnone\n--tools\nBash,Edit,Read\n--strict-mcp-config\n"),
        "{args}"
    );
    assert!(
        args.contains("\n--model\nclaude-test\n") && !args.contains("--resume"),
        "{args}"
    );
    assert!(
        args.contains("\"url\":\"http://127.0.0.1:") && args.contains("Bearer ${IP_MCP_TOKEN}"),
        "{args}"
    );
    assert!(
        args.ends_with("--allowedTools\nmcp__ip\n--allowedTools\nBash(sh test.sh)\n"),
        "{args}"
    );
    assert!(!args.contains("ipt_"), "本轮令牌不进参数：{args}");
    assert!(f.recorded("prompt", 1).contains("组织规则：结论先行。"));
    let prompts = f.data.join("engines/ops-1/prompts");
    assert_eq!(
        std::fs::read_dir(&prompts).unwrap().count(),
        0,
        "一轮结束后删掉提示词文件"
    );
    let stdin = f.recorded("stdin", 1);
    assert!(
        stdin.contains("张三（") && stdin.contains(" UTC）：测试挂了，看看") && !stdin.contains("组织规则"),
        "系统提示词不走标准输入：{stdin}"
    );
    let env = f.recorded("env", 1);
    let home = f.data.join("engines/ops-1");
    for want in [
        format!("HOME={}", home.display()),
        format!("CLAUDE_CONFIG_DIR={}", home.join("claude").display()),
        "ANTHROPIC_API_KEY=sk-ant-test".into(),
        "IP_MCP_TOKEN=ipt_".into(),
        "DISABLE_AUTOUPDATER=1".into(),
        "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1".into(),
    ] {
        assert!(env.contains(&want), "缺 {want}：{env}");
    }
    assert!(!env.contains("CARGO"), "harness 自己的环境变量不交给引擎：{env}");
    assert_eq!(f.recorded("pwd", 1).trim(), f.workspace.display().to_string());

    let messages = rt.store().thread_messages(&thread, None, 20).await.unwrap();
    let bodies: Vec<String> = messages
        .iter()
        .map(|m| match &m.body {
            Body::Text { text } => format!("text:{text}"),
            Body::ToolCall { name, .. } => format!("call:{name}"),
            Body::ToolResult { ok, .. } => format!("result:{ok}"),
            Body::Summary { .. } => "summary".into(),
        })
        .collect();
    assert_eq!(
        bodies,
        [
            "text:测试挂了，看看",
            "text:我先跑一下测试。",
            "call:claude.Bash",
            "result:false",
            "call:claude.Edit",
            "result:true",
            "text:修好了，测试通过。"
        ]
    );
    assert!(messages[1..].iter().all(|m| m.author
        == Author::Agent {
            agent_id: "ops-1".into()
        }));

    rt.send(&"ops-1".into(), thread.clone(), human(), "再跑一遍")
        .await
        .unwrap();
    assert!(matches!(turn_end(&mut rx).await, RuntimeEvent::TurnFinished { .. }));
    let args = f.recorded("args", 2);
    assert!(args.ends_with("\n--resume\nsess-c1\n"), "{args}");
    assert!(
        f.recorded("prompt", 2).contains("组织规则：结论先行。"),
        "续接时也给系统提示词（压缩上下文后要用）"
    );
    let stdin = f.recorded("stdin", 2);
    let stdin = stdin.trim();
    assert!(
        stdin.lines().count() == 1 && stdin.starts_with("张三（") && stdin.ends_with(" UTC）：再跑一遍"),
        "续接时只交新消息：{stdin}"
    );
    let spend = rt.spend_today().await.unwrap();
    assert!(
        (spend.roles[&"ops".into()] - 1.0).abs() < 1e-9,
        "第二轮用量是 0，不能按累计的 total_cost_usd 记：{spend:?}"
    );
    // 引擎会话表里的累计用量：第一轮的加上第二轮的（第二轮是 0）。
    let session = rt
        .store()
        .engine_session(&"ops-1".into(), &thread)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(session.engine, "claude_code");
    assert_eq!(
        (
            session.used.prompt_tokens,
            session.used.cached_tokens,
            session.used.completion_tokens
        ),
        (1_000_000, 400_000, 100_000)
    );
}

/// Claude Code 报错、没报结束、续接时找不到会话：这一轮失败，原因写清楚。
#[tokio::test]
async fn failed_claude_code_turns_explain_why() {
    let f = claude_fixture(serde_json::json!({})).await;
    let rt = f.runtime();
    let mut rx = rt.subscribe();

    // 模型接口报错：subtype 还是 success，is_error 为 true，退出码 1。
    f.out(
        1,
        &[
            serde_json::json!({"type": "system", "subtype": "init", "session_id": "s1", "mcp_servers": [{"name": "ip", "status": "connected"}]}),
            serde_json::json!({"type": "result", "subtype": "success", "is_error": true, "api_error_status": 400, "result": "API Error: 400 请求不对", "session_id": "s1", "usage": {"input_tokens": 0, "output_tokens": 0}}),
        ],
    );
    f.exit(1, 1);
    rt.send(&"ops-1".into(), ThreadId::new(), human(), "在吗")
        .await
        .unwrap();
    let RuntimeEvent::TurnFailed { error, .. } = turn_end(&mut rx).await else {
        panic!("应当失败")
    };
    assert!(error.contains("Claude Code 报错：API Error: 400 请求不对"), "{error}");

    // 正常退出却没有 result：失败，附上组织工具没连上的提示。
    f.out(
        2,
        &[
            serde_json::json!({"type": "system", "subtype": "init", "session_id": "s2", "mcp_servers": [{"name": "ip", "status": "failed"}]}),
            serde_json::json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "做了一半"}]}, "parent_tool_use_id": null}),
        ],
    );
    rt.send(&"ops-1".into(), ThreadId::new(), human(), "再试")
        .await
        .unwrap();
    let RuntimeEvent::TurnFailed { error, .. } = turn_end(&mut rx).await else {
        panic!("没有 result 应当失败")
    };
    assert!(
        error.contains("缺少 result") && error.contains("MCP 服务器 ip 没连上（failed）"),
        "{error}"
    );

    // 续接时找不到会话：result 里没有文字，原因只在 stderr 里（Claude Code 2.1.330 的行为）。
    f.out(
        3,
        &[serde_json::json!({"type": "result", "subtype": "error_during_execution", "is_error": true, "result": null, "session_id": "s3", "usage": {"input_tokens": 0, "output_tokens": 0}})],
    );
    f.script(3, "echo 'No conversation found with session ID: s0' >&2\n");
    f.exit(3, 1);
    rt.send(&"ops-1".into(), ThreadId::new(), human(), "还在吗")
        .await
        .unwrap();
    let RuntimeEvent::TurnFailed { error, .. } = turn_end(&mut rx).await else {
        panic!("应当失败")
    };
    assert!(
        error.contains("error_during_execution") && error.contains("No conversation found with session ID: s0"),
        "{error}"
    );
}

/// Claude Code 按 `--mcp-config` 连 `/mcp`（令牌从环境变量展开）、调 `update_task` 交回任务。
#[tokio::test]
async fn claude_code_hands_back_a_task_through_mcp() {
    let f = claude_fixture(serde_json::json!({})).await;
    f.script(
        1,
        r##"
prev=""
for a in "$@"; do
  if [ "$prev" = "--mcp-config" ]; then
    url=$(printf '%s' "$a" | grep -o '"url":"[^"]*"' | sed -e 's/^"url":"//' -e 's/"$//')
    auth=$(printf '%s' "$a" | grep -o 'Bearer [^"]*')
  fi
  prev="$a"
done
# 和 Claude Code 一样，把 ${IP_MCP_TOKEN} 换成环境变量的值。
pat='${IP_MCP_TOKEN}'
auth="${auth/"$pat"/$IP_MCP_TOKEN}"
task=$(grep -o '（id: [A-Za-z0-9_-]*），派活人' "$dir/stdin.$n" | head -1 | sed -e 's/（id: //' -e 's/），派活人//')
hdr=(-H "Authorization: $auth" -H "Content-Type: application/json" -H "Accept: application/json, text/event-stream")
curl -s "${hdr[@]}" -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fake-claude","version":"0"}}}' "$url" > "$dir/init.$n"
curl -s "${hdr[@]}" -d "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"update_task\",\"arguments\":{\"task_id\":\"$task\",\"status\":\"done\",\"result\":\"growth 测试已修复\"}}}" "$url" > "$dir/call.$n"
"##,
    );
    f.out(
        1,
        &[
            serde_json::json!({"type": "system", "subtype": "init", "session_id": "sess-m", "mcp_servers": [{"name": "ip", "status": "connected"}]}),
            serde_json::json!({"type": "assistant", "message": {"content": [{"type": "tool_use", "id": "toolu_m", "name": "mcp__ip__update_task", "input": {"status": "done"}}]}, "parent_tool_use_id": null}),
            serde_json::json!({"type": "user", "message": {"content": [{"type": "tool_result", "tool_use_id": "toolu_m", "content": [{"type": "text", "text": "任务已更新"}]}]}, "parent_tool_use_id": null}),
            serde_json::json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "已交回。"}]}, "parent_tool_use_id": null}),
            serde_json::json!({"type": "result", "subtype": "success", "is_error": false, "session_id": "sess-m", "usage": {"input_tokens": 10, "output_tokens": 5}}),
        ],
    );
    let rt = f.runtime();
    let mut rx = rt.subscribe();
    let origin = ThreadId::new();
    let task = rt
        .create_task(
            NewTask {
                title: "修 growth 测试".into(),
                assignee: Some("ops-1".into()),
                origin_thread_id: Some(origin.clone()),
                ..Default::default()
            },
            human(),
        )
        .await
        .unwrap();
    let end = turn_end(&mut rx).await;
    assert!(
        matches!(end, RuntimeEvent::TurnFinished { .. }),
        "{end:?}\ninit: {}\ncall: {}",
        f.recorded("init", 1),
        f.recorded("call", 1)
    );
    let call = f.recorded("call", 1);
    let task = rt.store().task(&task.id).await.unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Done, "{call}");
    assert_eq!(task.result.as_deref(), Some("growth 测试已修复"));
    let back = rt.store().thread_messages(&origin, None, 10).await.unwrap();
    assert!(
        back.iter()
            .any(|m| m.notice == Some(Notice::TaskReport) && format!("{:?}", m.body).contains("growth 测试已修复")),
        "{back:?}"
    );
    let work = rt
        .store()
        .thread_messages(task.thread_id.as_ref().unwrap(), None, 10)
        .await
        .unwrap();
    assert!(
        work.iter()
            .any(|m| matches!(&m.body, Body::ToolCall { name, .. } if name == "mcp__ip__update_task")),
        "{work:?}"
    );
}

const ENGINE_CONFIG: &str = r#"
endpoints:
  deepseek: { base_url: https://api.deepseek.com/v1 }
roles:
  - id: ops
    description: 修测试。
    model: { default: deepseek/deepseek-chat }
engines:
  ops:
    kind: codex
    command: /opt/codex/bin/codex
    model: codex-test
    api_key_env: CODEX_API_KEY
    sandbox: read-only
    turn_timeout_secs: 600
agents:
  - { id: ops-1, role: ops, name: 小维 }
"#;

#[test]
fn engines_config_parses_and_is_checked() {
    let c = crate::Config::from_yaml(ENGINE_CONFIG).unwrap();
    let ip_engines::EngineConfig::Codex(codex) = &c.engines["ops"] else {
        panic!("应是 Codex")
    };
    assert_eq!(codex.command, "/opt/codex/bin/codex");
    assert_eq!(codex.sandbox, ip_engines::CodexSandbox::ReadOnly);
    assert_eq!((codex.turn_timeout_secs, codex.network), (600, false));

    // 用引擎的岗位不调 `model` 里的模型，端点写错也不拦。
    assert!(crate::Config::from_yaml(&ENGINE_CONFIG.replace("deepseek/deepseek-chat", "nowhere/m")).is_ok());
    for (from, to, why) in [
        ("engines:\n  ops:", "engines:\n  nobody:", "不存在的岗位"),
        ("kind: codex", "kind: claude", "不认识的引擎"),
        ("sandbox: read-only", "sandbox: danger-full-access", "不允许的沙箱级别"),
        ("turn_timeout_secs: 600", "turn_timeout: 600", "拼错的字段"),
        ("name: 小维 }", "name: 小维, tools: false }", "没有工作目录的员工"),
        (
            "description: 修测试。",
            "description: 修测试。\n    budget_per_day_usd: 5",
            "设了预算却没给引擎价格",
        ),
    ] {
        let text = ENGINE_CONFIG.replacen(from, to, 1);
        assert!(crate::Config::from_yaml(&text).is_err(), "{why} 应被拒绝");
    }
    let priced = ENGINE_CONFIG
        .replace(
            "description: 修测试。",
            "description: 修测试。\n    budget_per_day_usd: 5",
        )
        .replace(
            "turn_timeout_secs: 600",
            "turn_timeout_secs: 600\n    price: { input: 1.25, output: 10 }",
        );
    assert!(crate::Config::from_yaml(&priced).is_ok());
    assert!(crate::Config::from_yaml(&priced.replace("input: 1.25", "input: -1")).is_err());
}

const CLAUDE_CONFIG: &str = r#"
endpoints:
  deepseek: { base_url: https://api.deepseek.com/v1 }
roles:
  - id: ops
    description: 修测试。
    model: { default: deepseek/deepseek-chat }
engines:
  ops:
    kind: claude_code
    model: claude-test
    api_key_env: ANTHROPIC_API_KEY
    access: read-only
    allowed_tools: ["Bash(cargo test:*)"]
    turn_timeout_secs: 600
agents:
  - { id: ops-1, role: ops, name: 小维 }
"#;

#[test]
fn claude_code_config_parses_and_is_checked() {
    let c = crate::Config::from_yaml(CLAUDE_CONFIG).unwrap();
    let ip_engines::EngineConfig::ClaudeCode(claude) = &c.engines["ops"] else {
        panic!("应是 Claude Code")
    };
    assert_eq!(claude.command, "claude");
    assert_eq!(claude.access, ip_engines::ClaudeCodeAccess::ReadOnly);
    assert_eq!(claude.allowed_tools, ["Bash(cargo test:*)"]);
    assert_eq!(c.engines["ops"].label(), "Claude Code");
    assert_eq!(c.engines["ops"].api_key_env(), Some("ANTHROPIC_API_KEY"));
    for (from, to, why) in [
        ("    api_key_env: ANTHROPIC_API_KEY\n", "", "没有 api_key_env"),
        ("access: read-only", "access: full", "不认识的权限"),
        ("access: read-only", "sandbox: read-only", "Codex 的字段"),
        (
            "\"Bash(cargo test:*)\"",
            "\"--dangerously-skip-permissions\"",
            "会被当成参数的规则",
        ),
        ("model: claude-test", "model: --bare", "会被当成参数的模型名"),
        (
            "description: 修测试。",
            "description: 修测试。\n    budget_per_day_usd: 5",
            "设了预算却没给引擎价格",
        ),
    ] {
        let text = CLAUDE_CONFIG.replacen(from, to, 1);
        assert_ne!(text, CLAUDE_CONFIG, "{why}：替换没生效");
        assert!(crate::Config::from_yaml(&text).is_err(), "{why} 应被拒绝");
    }
}
