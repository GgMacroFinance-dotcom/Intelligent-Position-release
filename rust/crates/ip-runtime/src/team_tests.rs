//! 团队上下文（`team.rs`）的测试：假模型，加一个在内存管道上跑的假 TCMS（MCP 服务器）。

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ip_core::{Agent, Author, ModelSlot, Role, ThreadId};
use ip_mcp::McpHub;
use ip_models::{ChatMessage, ModelRegistry, ScriptedModel, ScriptedTurn, ToolCall, Usage};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerConfig as ServerInfo, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::{ErrorData, ServerHandler, ServiceExt};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::engine::{BoxFuture, Engine, TurnContext, TurnReport};
use crate::team::{TeamContextConfig, TeamRole};
use crate::{AgentSetup, Runtime, RuntimeConfig, RuntimeEvent};

/// 假的 TCMS：记下每次 `context_get` 的参数；前缀可以改，`down` 时 `context_get` 报错。
#[derive(Clone, Default)]
struct FakeTcms {
    gets: Arc<Mutex<Vec<Value>>>,
    searches: Arc<Mutex<Vec<Value>>>,
    proposes: Arc<Mutex<Vec<Value>>>,
    prefix: Arc<Mutex<String>>,
    down: Arc<AtomicBool>,
}

fn schema() -> Arc<serde_json::Map<String, Value>> {
    match json!({"type": "object"}) {
        Value::Object(m) => Arc::new(m),
        _ => unreachable!(),
    }
}

impl ServerHandler for FakeTcms {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let names = [
            "context_get",
            "context_search",
            "context_read",
            "context_changes",
            "context_propose",
        ];
        Ok(ListToolsResult::with_all_items(
            names.iter().map(|n| Tool::new(*n, "团队上下文", schema())).collect(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let args = Value::Object(request.arguments.clone().unwrap_or_default());
        match request.name.as_ref() {
            "context_get" => {
                self.gets.lock().unwrap().push(args.clone());
                if self.down.load(Ordering::SeqCst) {
                    return Ok(CallToolResult::error(vec![ContentBlock::text("索引还没建好")]).into());
                }
                let task = args["task"].as_str().unwrap_or_default();
                let prefix = self.prefix.lock().unwrap().clone();
                Ok(CallToolResult::structured(json!({
                    "prefix": prefix,
                    "task": format!("## 和这次任务相关的条目\n[ctx_A r1] 任务是：{task}\n[/ctx_A]"),
                    "cursor": 7,
                    "warnings": [],
                }))
                .into())
            }
            "context_search" => {
                self.searches.lock().unwrap().push(args.clone());
                Ok(CallToolResult::structured(json!({"hits": [{"id": "ctx_A", "title": "CPI 口径"}]})).into())
            }
            "context_propose" => {
                let mut proposes = self.proposes.lock().unwrap();
                proposes.push(args.clone());
                if self.down.load(Ordering::SeqCst) {
                    return Ok(CallToolResult::error(vec![ContentBlock::text("没处理的提议已经有 20 条")]).into());
                }
                Ok(CallToolResult::structured(json!({
                    "proposal_id": format!("prop_{}", proposes.len()),
                    "status": "open",
                    "duplicate": false,
                    "conflicts": [],
                }))
                .into())
            }
            _ => Ok(CallToolResult::error(vec![ContentBlock::text("测试里没用到")]).into()),
        }
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    runtime: Runtime,
    model: Arc<ScriptedModel>,
    tcms: FakeTcms,
    hub: Arc<McpHub>,
}

fn role() -> Role {
    Role {
        id: "researcher".into(),
        name: Some("宏观研究员".into()),
        description: "撰写宏观经济研究报告。".into(),
        model: ModelSlot {
            default: "fake/m".into(),
            escalate: None,
        },
        budget_per_day_usd: None,
    }
}

fn agent() -> Agent {
    Agent {
        id: "a1".into(),
        role: "researcher".into(),
        name: "小宏".into(),
    }
}

fn config() -> RuntimeConfig {
    let mut roles = BTreeMap::new();
    roles.insert(
        "researcher".into(),
        TeamRole {
            server: "tcms".into(),
            role: Some("macro".into()),
            team: None,
            share_memory: false,
        },
    );
    RuntimeConfig {
        org_prompt: "公司规则：结论先行。".into(),
        memory: crate::memory::MemoryConfig {
            enabled: false,
            ..crate::memory::MemoryConfig::default()
        },
        team_context: TeamContextConfig {
            budget_tokens: 3000,
            roles,
        },
        ..RuntimeConfig::default()
    }
}

/// 起假 TCMS、接进 MCP，再起一个员工。`engine` 给了时这个员工的每一轮交给它。
async fn fixture(turns: Vec<ScriptedTurn>, engine: Option<Arc<dyn Engine>>) -> Fixture {
    fixture_with(turns, engine, config()).await
}

async fn fixture_with(turns: Vec<ScriptedTurn>, engine: Option<Arc<dyn Engine>>, config: RuntimeConfig) -> Fixture {
    let tcms = FakeTcms::default();
    *tcms.prefix.lock().unwrap() = "# 团队上下文\n## 常驻条目\n[ctx_R r1] 数字注明来源。\n[/ctx_R]".into();
    let (client, server) = tokio::io::duplex(64 * 1024);
    let handler = tcms.clone();
    tokio::spawn(async move {
        let running = handler.serve(server).await.unwrap();
        let _ = running.waiting().await;
    });
    let hub = Arc::new(McpHub::default());
    hub.add("tcms", client, vec!["researcher".into()], Duration::from_secs(5))
        .await
        .unwrap();

    let dir = tempfile::tempdir().unwrap();
    let store = ip_store::testing::store_at(dir.path().join("ip.db")).await;
    let model = Arc::new(ScriptedModel::new(turns));
    let mut models = ModelRegistry::new();
    models.insert("fake", model.clone());
    let runtime = Runtime::new(store, models, config);
    runtime
        .spawn_agent(AgentSetup {
            agent: agent(),
            role: role(),
            toolbox: None,
            mcp: Some(hub.clone()),
            data: None,
            engine,
        })
        .unwrap();
    Fixture {
        _dir: dir,
        runtime,
        model,
        tcms,
        hub,
    }
}

async fn until_turn_end(rx: &mut broadcast::Receiver<RuntimeEvent>) -> Vec<RuntimeEvent> {
    let mut seen = Vec::new();
    loop {
        let e = tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("等待事件超时")
            .unwrap();
        let end = matches!(e, RuntimeEvent::TurnFinished { .. } | RuntimeEvent::TurnFailed { .. });
        seen.push(e);
        if end {
            return seen;
        }
    }
}

async fn say(f: &Fixture, thread: &ThreadId, text: &str) {
    let mut rx = f.runtime.subscribe();
    f.runtime
        .send(&"a1".into(), thread.clone(), Author::human("张三"), text)
        .await
        .unwrap();
    let events = until_turn_end(&mut rx).await;
    assert!(
        matches!(events.last(), Some(RuntimeEvent::TurnFinished { .. })),
        "{events:?}"
    );
}

fn system_of(messages: &[ChatMessage]) -> &str {
    match &messages[0] {
        ChatMessage::System { content } => content,
        other => panic!("第一条应该是系统提示词：{other:?}"),
    }
}

fn user_texts(messages: &[ChatMessage]) -> Vec<&str> {
    messages
        .iter()
        .filter_map(|m| match m {
            ChatMessage::User { content } => Some(content.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn team_context_goes_into_the_prompt_and_the_opening_stays_put() {
    let f = fixture(vec![ScriptedTurn::Text("好".into())], None).await;
    let thread = ThreadId::new();
    say(&f, &thread, "写一份 CPI 点评").await;

    // 用配置里的岗位名和预算取包，任务描述是人的请求。
    let gets = f.tcms.gets.lock().unwrap().clone();
    assert_eq!(gets.len(), 1);
    assert_eq!(gets[0]["role"], "macro");
    assert_eq!(gets[0]["budget_tokens"], 3000);
    assert_eq!(gets[0]["task"], "写一份 CPI 点评");
    assert!(gets[0].get("team").is_none());

    // 固定前缀在组织规则之后、岗位说明之前；工具名按 MCP 的写法告诉员工。
    let first = &f.model.requests()[0];
    let system = system_of(&first.messages);
    let org = system.find("公司规则：结论先行。").unwrap();
    let team = system.find("数字注明来源。").expect("带上固定前缀");
    let role = system.find("# 岗位：宏观研究员").unwrap();
    assert!(org < team && team < role, "{system}");
    assert!(system.contains("mcp__tcms__context_search") && system.contains("mcp__tcms__context_propose"));

    // 按任务挑的部分是对话的第一条，在人的消息前面。
    let users = user_texts(&first.messages);
    assert!(users[0].starts_with("[团队上下文"), "{users:?}");
    assert!(users[0].contains("任务是：写一份 CPI 点评"));
    assert!(users[1].ends_with("写一份 CPI 点评"));

    // context_get、context_changes 不给模型；查、读、提议给。
    let tools: Vec<&str> = first.tools.iter().map(|t| t.name.as_str()).collect();
    assert!(tools.contains(&"mcp__tcms__context_search") && tools.contains(&"mcp__tcms__context_propose"));
    assert!(!tools.contains(&"mcp__tcms__context_get") && !tools.contains(&"mcp__tcms__context_changes"));

    // 第二轮：规则改了，前缀跟着换；对话开头不换（前面的内容不变，缓存能命中）。
    *f.tcms.prefix.lock().unwrap() = "## 规则\n[ctx_R r2] 数字注明来源和日期。\n[/ctx_R]".into();
    say(&f, &thread, "再加上 PPI").await;
    let gets = f.tcms.gets.lock().unwrap().clone();
    assert_eq!(gets.len(), 2);
    assert_eq!(gets[1]["task"], "写一份 CPI 点评", "任务描述还是对话的第一条");
    let second = &f.model.requests()[1];
    assert!(system_of(&second.messages).contains("数字注明来源和日期。"));
    assert_eq!(user_texts(&second.messages)[0], users[0]);

    // 别的对话按自己的任务取。
    say(&f, &ThreadId::new(), "看一下 PMI").await;
    let third = &f.model.requests()[2];
    assert!(user_texts(&third.messages)[0].contains("任务是：看一下 PMI"));
}

#[tokio::test]
async fn employees_use_search_but_not_the_hidden_tools() {
    let call = |id: &str, name: &str| ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: r#"{"query":"CPI 口径","role":"macro","task":"x"}"#.into(),
    };
    let f = fixture(
        vec![
            ScriptedTurn::ToolCalls(vec![
                call("c1", "mcp__tcms__context_search"),
                call("c2", "mcp__tcms__context_get"),
            ]),
            ScriptedTurn::Text("好".into()),
        ],
        None,
    )
    .await;
    let thread = ThreadId::new();
    say(&f, &thread, "CPI 用哪个口径").await;
    assert_eq!(f.tcms.searches.lock().unwrap().len(), 1);
    // 员工调不到 context_get：只有每轮开头系统调的那一次。
    assert_eq!(f.tcms.gets.lock().unwrap().len(), 1);
    let results: Vec<(bool, String)> = f
        .runtime
        .store()
        .thread_messages(&thread, None, 20)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|m| match m.body {
            ip_core::Body::ToolResult { ok, output, .. } => Some((ok, output)),
            _ => None,
        })
        .collect();
    assert!(results[0].0 && results[0].1.contains("CPI 口径"), "{results:?}");
    assert!(
        !results[1].0 && results[1].1.contains("由系统在每轮开头调用"),
        "{results:?}"
    );
}

#[tokio::test]
async fn turns_go_on_without_team_context_when_tcms_fails() {
    let f = fixture(vec![ScriptedTurn::Text("好".into())], None).await;
    f.tcms.down.store(true, Ordering::SeqCst);
    let thread = ThreadId::new();
    say(&f, &thread, "写一份 CPI 点评").await;
    let first = &f.model.requests()[0];
    assert!(!system_of(&first.messages).contains("团队上下文"));
    assert!(!user_texts(&first.messages)[0].contains("团队上下文"));

    // 修好以后下一轮就带上；失败的那次没有记下“这个对话没有开头”。
    f.tcms.down.store(false, Ordering::SeqCst);
    say(&f, &thread, "继续").await;
    let second = &f.model.requests()[1];
    assert!(system_of(&second.messages).contains("数字注明来源。"));
    assert!(user_texts(&second.messages)[0].starts_with("[团队上下文"));
}

/// 记下每轮拿到的系统提示词和输入的假引擎。
#[derive(Default)]
struct RecordingEngine {
    seen: Mutex<Vec<(Option<String>, String, String)>>,
}

impl Engine for RecordingEngine {
    fn kind(&self) -> &str {
        "fake"
    }

    fn turn_timeout(&self) -> Duration {
        Duration::from_secs(10)
    }

    fn turn<'a>(&'a self, ctx: TurnContext<'a>) -> BoxFuture<'a, TurnReport> {
        Box::pin(async move {
            self.seen.lock().unwrap().push((
                ctx.session.map(str::to_string),
                ctx.system_prompt.to_string(),
                ctx.input.to_string(),
            ));
            let _ = ctx.sink.append(ip_core::Body::Text { text: "好".into() }).await;
            TurnReport {
                steps: 1,
                usage: Usage::default(),
                session_id: Some("s1".into()),
                ..TurnReport::default()
            }
        })
    }
}

#[tokio::test]
async fn engines_get_the_prefix_and_the_opening_on_their_first_turn() {
    let engine = Arc::new(RecordingEngine::default());
    let f = fixture(Vec::new(), Some(engine.clone())).await;
    let thread = ThreadId::new();
    say(&f, &thread, "修一下 CPI 图表").await;
    say(&f, &thread, "再检查一遍").await;
    let seen = engine.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2);
    let (session, system, input) = &seen[0];
    assert!(session.is_none());
    assert!(system.contains("数字注明来源。"));
    assert!(
        input.starts_with("[团队上下文") && input.contains("任务是：修一下 CPI 图表"),
        "{input}"
    );
    assert!(input.trim_end().ends_with("修一下 CPI 图表"), "{input}");
    // 续接的那一轮只交新消息，引擎自己的会话里已经有开头了。
    let (session, _, input) = &seen[1];
    assert_eq!(session.as_deref(), Some("s1"));
    assert!(!input.contains("团队上下文"), "{input}");
}

/// 连真的 `tcms serve`（只在设了 `IP_TEST_TCMS_URL`、`IP_TEST_TCMS_TOKEN` 时跑，CI 里没有 tcms，跳过）。
/// 令牌是 `tcms token add --role macro` 发的；上下文仓库里要有 `role:macro` 能看到的规则或常驻条目。
#[tokio::test]
async fn real_tcms_over_http_when_configured() {
    let (Ok(url), Ok(token)) = (std::env::var("IP_TEST_TCMS_URL"), std::env::var("IP_TEST_TCMS_TOKEN")) else {
        eprintln!("没设 IP_TEST_TCMS_URL、IP_TEST_TCMS_TOKEN，跳过");
        return;
    };
    let mut servers = BTreeMap::new();
    servers.insert(
        "tcms".to_string(),
        ip_mcp::ServerConfig {
            url: Some(url),
            bearer_token_env: Some("TOKEN".into()),
            roles: vec!["researcher".into()],
            ..ip_mcp::ServerConfig::default()
        },
    );
    let hub = McpHub::connect_with_env(&servers, &|var| (var == "TOKEN").then(|| token.clone()))
        .await
        .unwrap();
    assert!(hub.unavailable().is_empty(), "{:?}", hub.unavailable());

    let dir = tempfile::tempdir().unwrap();
    let store = ip_store::testing::store_at(dir.path().join("ip.db")).await;
    let search = ToolCall {
        id: "c1".into(),
        name: "mcp__tcms__context_search".into(),
        arguments: r#"{"query":"CPI"}"#.into(),
    };
    let propose = ToolCall {
        id: "c2".into(),
        name: "mcp__tcms__context_propose".into(),
        arguments: json!({
            "action": "create",
            "draft": {"title": "PPI 也用统计局的数", "body": "PPI 同比用国家统计局公布的数。"},
            "reason": "这次写 CPI 点评时顺便核对过"
        })
        .to_string(),
    };
    let model = Arc::new(ScriptedModel::new(vec![
        ScriptedTurn::ToolCalls(vec![search, propose]),
        ScriptedTurn::Text("好".into()),
    ]));
    let mut models = ModelRegistry::new();
    models.insert("fake", model.clone());
    let runtime = Runtime::new(store, models, config());
    runtime
        .spawn_agent(AgentSetup {
            agent: agent(),
            role: role(),
            toolbox: None,
            mcp: Some(Arc::new(hub)),
            data: None,
            engine: None,
        })
        .unwrap();
    let mut rx = runtime.subscribe();
    let thread = ThreadId::new();
    runtime
        .send(
            &"a1".into(),
            thread.clone(),
            Author::human("张三"),
            "写一份 CPI 点评，注意口径",
        )
        .await
        .unwrap();
    until_turn_end(&mut rx).await;

    let requests = model.requests();
    let system = system_of(&requests[0].messages);
    eprintln!("---- 系统提示词 ----\n{system}");
    let users = user_texts(&requests[0].messages);
    eprintln!("---- 对话开头 ----\n{}", users[0]);
    assert!(system.contains("# 团队上下文"), "{system}");
    let tools: Vec<&str> = requests[0].tools.iter().map(|t| t.name.as_str()).collect();
    eprintln!("---- 工具 ----\n{tools:?}");
    assert!(tools.contains(&"mcp__tcms__context_search") && !tools.contains(&"mcp__tcms__context_get"));
    let results: Vec<&str> = requests[1]
        .messages
        .iter()
        .filter_map(|m| match m {
            ChatMessage::Tool { content, .. } => Some(content.as_str()),
            _ => None,
        })
        .collect();
    eprintln!(
        "---- context_search、context_propose 的结果 ----\n{}",
        results.join("\n")
    );
    assert!(results[0].contains("hits"), "{results:?}");
    assert!(results[1].contains("proposal_id"), "{results:?}");
}

/// 开了 `share_memory` 的配置：岗位记忆开着，团队是 research。
fn share_config() -> RuntimeConfig {
    let mut c = config();
    c.memory.enabled = true;
    let team = c.team_context.roles.get_mut(&"researcher".into()).unwrap();
    team.team = Some("research".into());
    team.share_memory = true;
    c
}

#[tokio::test]
async fn new_role_memory_is_shared_with_the_team_as_proposals() {
    use crate::memory::{self, Layer};

    let first = r#"```json
{"items": [
  {"kind": "procedure", "title": "图表下面注明数据来源", "body": "每张图下面写数据来源和截至日期。\n## 怎么写\n来源写全称。令牌 sk-abcdefghijklmnop123456",
   "why": "团队里写报告的岗位都要这样。", "threads": ["thr_1", "thr_bogus"]},
  {"kind": "rule", "title": "美国数据用 FRED", "body": "美国的宏观序列从 FRED 取。", "why": "", "threads": []},
  {"kind": "fact", "title": "  ", "body": "没有标题的不提。"}
]}
```"#;
    let second = r#"{"items": [
  {"kind": "procedure", "title": "图表下面 注明数据来源", "body": "换个说法又提一次。"},
  {"kind": "fact", "title": "PMI 先看新订单", "body": "看 PMI 先看新订单分项。"}
]}"#;
    let f = fixture_with(
        vec![
            ScriptedTurn::Text(first.into()),
            ScriptedTurn::Text(second.into()),
            ScriptedTurn::Text("这次没有合适的。".into()),
        ],
        None,
        share_config(),
    )
    .await;
    let store = f.runtime.store();
    let role: ip_core::RoleId = "researcher".into();
    let hub = Some(f.hub.as_ref());
    let v1 = "## 怎么和这个团队配合\n图表下面注明数据来源（对话 thr_1、thr_2 都这样要求）。\n## 干活的提示\n美国数据用 FRED。\n## 记忆里有什么\n- 2026-11-20 对话 thr_1 图表";

    // 还没有岗位记忆：不调模型。
    assert_eq!(f.runtime.memory_share(hub, 1_000).await.unwrap(), Default::default());
    assert!(f.model.requests().is_empty());

    // 有了第一版：调一次模型，挑出的两条提到 team:research。
    memory::put(store, Layer::Summary, &role, v1, 1_000).await.unwrap();
    let report = f.runtime.memory_share(hub, 2_000).await.unwrap();
    assert_eq!((report.checked, report.proposed, report.failed), (1, 2, 0));
    let requests = f.model.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].tools.is_empty());
    assert!(system_of(&requests[0].messages).contains("团队经验整理员"));
    let input = user_texts(&requests[0].messages)[0];
    assert!(input.contains("团队：research") && input.contains(v1), "{input}");
    assert!(input.contains("# 已经提过的\n（没有）"), "{input}");
    let proposes = f.tcms.proposes.lock().unwrap().clone();
    assert_eq!(proposes.len(), 2);
    let p = &proposes[0];
    assert_eq!(p["action"], "create");
    assert_eq!(p["draft"]["scope"], "team:research");
    assert_eq!(p["draft"]["kind"], "procedure");
    assert_eq!(p["draft"]["title"], "图表下面注明数据来源");
    let body = p["draft"]["body"].as_str().unwrap();
    assert!(body.contains("### 怎么写") && !body.contains("\n## "), "{body}");
    assert!(!body.contains("abcdefghijklmnop"), "{body}");
    let reason = p["reason"].as_str().unwrap();
    assert!(reason.contains("宏观研究员") && reason.contains("research"), "{reason}");
    let evidence: Vec<&str> = p["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(evidence[0].contains("岗位记忆"), "{evidence:?}");
    // 岗位记忆里写到的对话才当证据。
    assert!(
        evidence.contains(&"I.P. 对话 thr_1") && !evidence.iter().any(|e| e.contains("thr_bogus")),
        "{evidence:?}"
    );
    // 规则只能人写，换成经验。
    assert_eq!(proposes[1]["draft"]["kind"], "memory");

    // 同一版不再看。
    assert_eq!(f.runtime.memory_share(hub, 3_000).await.unwrap(), Default::default());
    assert_eq!(f.model.requests().len(), 1);

    // 第二版：告诉模型提过什么；标题一样（不算空白）的不再提。
    memory::put(
        store,
        Layer::Summary,
        &role,
        &format!("{v1}\n看 PMI 先看新订单。"),
        4_000,
    )
    .await
    .unwrap();
    let report = f.runtime.memory_share(hub, 5_000).await.unwrap();
    assert_eq!((report.checked, report.proposed), (1, 1));
    let input = user_texts(&f.model.requests()[1].messages)[0].to_string();
    assert!(
        input.contains("- 图表下面注明数据来源") && input.contains("- 美国数据用 FRED"),
        "{input}"
    );
    let proposes = f.tcms.proposes.lock().unwrap().clone();
    assert_eq!(proposes.len(), 3);
    assert_eq!(proposes[2]["draft"]["title"], "PMI 先看新订单");

    // 模型输出看不懂：这一版算看过了，不再为它调模型。
    memory::put(store, Layer::Summary, &role, "## 怎么和这个团队配合\n换了一版。", 6_000)
        .await
        .unwrap();
    let report = f.runtime.memory_share(hub, 7_000).await.unwrap();
    assert_eq!((report.checked, report.proposed), (1, 0));
    assert_eq!(f.runtime.memory_share(hub, 8_000).await.unwrap(), Default::default());
    assert_eq!(f.model.requests().len(), 3);
    assert_eq!(f.tcms.proposes.lock().unwrap().len(), 3);

    // 没有 MCP 服务器时不提，也不调模型。
    memory::put(store, Layer::Summary, &role, "## 怎么和这个团队配合\n又一版。", 9_000)
        .await
        .unwrap();
    assert_eq!(f.runtime.memory_share(None, 9_500).await.unwrap(), Default::default());
    assert_eq!(f.model.requests().len(), 3);
}

#[tokio::test]
async fn sharing_role_memory_is_off_unless_configured_and_survives_tcms_errors() {
    use crate::memory::{self, Layer};

    let pick = r#"{"items": [{"kind": "fact", "title": "CPI 用统计局的数", "body": "国内 CPI 用国家统计局的数。"}]}"#;
    // 没开 share_memory：什么都不做。
    let off = fixture_with(vec![ScriptedTurn::Text(pick.into())], None, {
        let mut c = config();
        c.memory.enabled = true;
        c
    })
    .await;
    let role: ip_core::RoleId = "researcher".into();
    memory::put(
        off.runtime.store(),
        Layer::Summary,
        &role,
        "## 干活的提示\nCPI 用统计局的数。",
        1_000,
    )
    .await
    .unwrap();
    assert_eq!(
        off.runtime.memory_share(Some(off.hub.as_ref()), 2_000).await.unwrap(),
        Default::default()
    );
    assert!(off.model.requests().is_empty());

    // TCMS 拒了提议（比如攒满了）：记一次失败，这一版算看过了，下一版再说。
    let f = fixture_with(vec![ScriptedTurn::Text(pick.into())], None, share_config()).await;
    f.tcms.down.store(true, Ordering::SeqCst);
    memory::put(
        f.runtime.store(),
        Layer::Summary,
        &role,
        "## 干活的提示\nCPI 用统计局的数。",
        1_000,
    )
    .await
    .unwrap();
    let hub = Some(f.hub.as_ref());
    let report = f.runtime.memory_share(hub, 2_000).await.unwrap();
    assert_eq!((report.checked, report.proposed, report.failed), (1, 0, 1));
    assert_eq!(f.tcms.proposes.lock().unwrap().len(), 1);
    assert_eq!(f.runtime.memory_share(hub, 3_000).await.unwrap(), Default::default());
    assert_eq!(f.model.requests().len(), 1);
}

/// 把岗位记忆提给真的 `tcms serve`（和上面一样只在设了 `IP_TEST_TCMS_URL`、`IP_TEST_TCMS_TOKEN` 时跑）。
/// 令牌的团队写在 `IP_TEST_TCMS_TEAM`，不写时是 research。提完以后用 `tcms proposals` 看得到这一条。
#[tokio::test]
async fn real_tcms_receives_shared_role_memory_when_configured() {
    use crate::memory::{self, Layer};

    let (Ok(url), Ok(token)) = (std::env::var("IP_TEST_TCMS_URL"), std::env::var("IP_TEST_TCMS_TOKEN")) else {
        eprintln!("没设 IP_TEST_TCMS_URL、IP_TEST_TCMS_TOKEN，跳过");
        return;
    };
    let team = std::env::var("IP_TEST_TCMS_TEAM").unwrap_or_else(|_| "research".into());
    let mut servers = BTreeMap::new();
    servers.insert(
        "tcms".to_string(),
        ip_mcp::ServerConfig {
            url: Some(url),
            bearer_token_env: Some("TOKEN".into()),
            roles: vec!["researcher".into()],
            ..ip_mcp::ServerConfig::default()
        },
    );
    let hub = McpHub::connect_with_env(&servers, &|var| (var == "TOKEN").then(|| token.clone()))
        .await
        .unwrap();
    assert!(hub.unavailable().is_empty(), "{:?}", hub.unavailable());

    let dir = tempfile::tempdir().unwrap();
    let store = ip_store::testing::store_at(dir.path().join("ip.db")).await;
    let pick = json!({"items": [{
        "kind": "procedure",
        "title": "图表下面注明数据来源和截至日期",
        "body": "每张图下面写数据来源（全称）和数据截至日期。",
        "why": "团队里写报告的岗位都这样做。",
        "threads": ["thr_1"],
    }]});
    let model = Arc::new(ScriptedModel::new(vec![ScriptedTurn::Text(pick.to_string())]));
    let mut models = ModelRegistry::new();
    models.insert("fake", model.clone());
    let mut config = share_config();
    config.team_context.roles.get_mut(&"researcher".into()).unwrap().team = Some(team.clone());
    let runtime = Runtime::new(store, models, config);
    runtime
        .spawn_agent(AgentSetup {
            agent: agent(),
            role: role(),
            toolbox: None,
            mcp: None,
            data: None,
            engine: None,
        })
        .unwrap();
    let role_id: ip_core::RoleId = "researcher".into();
    memory::put(
        runtime.store(),
        Layer::Summary,
        &role_id,
        "## 怎么和这个团队配合\n图表下面注明数据来源和截至日期（对话 thr_1、thr_2 都这样要求）。",
        1_000,
    )
    .await
    .unwrap();
    let report = runtime.memory_share(Some(&hub), 2_000).await.unwrap();
    eprintln!("---- 一轮的结果 ----\n{report:?}");
    assert_eq!((report.checked, report.failed), (1, 0), "{report:?}");
    hub.shutdown().await;
}
