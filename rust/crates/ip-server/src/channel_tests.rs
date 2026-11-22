//! 渠道测试 T1–T14（`docs/ip/m2-spec.md` D-10、D-28），用假渠道加脚本模型，验收 Hub（D-03、D-05、D-07）
//! 和投递器（D-04、D-06）。测试只通过 [`attach`]（T14 用 `hub_for_tests`）接触 `crate::channels`，
//! 其余用的都是已有的接口，不读 D-08 的表。

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use ip_connectors::fake::{FakeChannel, FakeHandle, Sent};
use ip_connectors::{DeliverError, HubError, Inbound, Outbound, Sender};
use ip_core::{
    Agent, Approval, ApprovalState, ApprovalSubject, Author, Body, ModelSlot, Notice, OrgSpec, Role, ThreadId, Via,
};
use ip_models::{ModelRegistry, ScriptedModel, ScriptedTurn, ToolCall};
use ip_runtime::{AgentSetup, Runtime, RuntimeConfig};
use serde_json::json;

use crate::App;

/// 等该发生的事。
const WAIT: Duration = Duration::from_secs(10);
/// 确认不该发生的事没有发生。
const QUIET: Duration = Duration::from_millis(800);

// ---------------------------------------------------------------------------------------------
// 接入点
// ---------------------------------------------------------------------------------------------

use crate::channels::{Approvers, ChannelHandle, ChannelSettings as Settings};

/// 把 `channel` 接到 `app` 上：建 Hub，启动渠道的 `run` 和这个渠道的投递器。
async fn attach(app: &App, channel: Arc<FakeChannel>, settings: Settings) -> ChannelHandle {
    crate::channels::start(app, channel, settings).await.unwrap()
}

// ---------------------------------------------------------------------------------------------
// 测试用的组织
// ---------------------------------------------------------------------------------------------

/// 路由：含“研报”的交给协调员，含 CPI 的交给宏观研究员，其余出卡片让人选（fallback 默认是 ask）。
const RULES: &str = r#"
rules:
  - id: 研报
    when: { any: [研报] }
    to: { role: coordinator }
  - id: 数据
    when: { any: [CPI] }
    to: { role: macro }
"#;

struct Org {
    _dir: tempfile::TempDir,
    app: App,
}

fn role(id: &str, model: &str) -> Role {
    Role {
        id: id.into(),
        name: None,
        description: format!("{id} 的职责"),
        model: ModelSlot {
            default: model.into(),
            escalate: None,
        },
        budget_per_day_usd: None,
    }
}

/// 协调员 coord-1（模型脚本 `coord`）和 `macros` 个宏观研究员 macro-1、macro-2……（共用脚本 `research`）。
/// 脚本用完后会重复最后一轮，所以最后一轮要是文字。
pub(crate) async fn build(db: &Path, coord: Vec<ScriptedTurn>, research: Vec<ScriptedTurn>, macros: usize) -> App {
    let store = ip_store::testing::store_at(db).await;
    let mut models = ModelRegistry::new();
    models.insert("coord", Arc::new(ScriptedModel::new(coord)));
    models.insert("research", Arc::new(ScriptedModel::new(research)));
    let runtime = Runtime::new(store, models, RuntimeConfig::default());
    let coordinator = role("coordinator", "coord/m");
    let research_role = role("macro", "research/m");
    let mut agents = vec![Agent {
        id: "coord-1".into(),
        role: "coordinator".into(),
        name: "小协".into(),
    }];
    for i in 1..=macros {
        agents.push(Agent {
            id: format!("macro-{i}").as_str().into(),
            role: "macro".into(),
            name: format!("小宏{i}"),
        });
    }
    for a in &agents {
        let r = if a.role.as_str() == "macro" {
            &research_role
        } else {
            &coordinator
        };
        runtime
            .spawn_agent(AgentSetup {
                agent: a.clone(),
                role: r.clone(),
                toolbox: None,
                mcp: None,
                data: None,
                engine: None,
            })
            .unwrap();
    }
    let org = OrgSpec {
        roles: vec![coordinator, research_role],
    };
    let router = ip_router::Router::new(serde_saphyr::from_str(RULES).unwrap(), &org, &agents).unwrap();
    App::new(runtime, org, None).with_router(router)
}

async fn org(coord: Vec<ScriptedTurn>, research: Vec<ScriptedTurn>, macros: usize) -> Org {
    let dir = tempfile::tempdir().unwrap();
    let app = build(&dir.path().join("ip.db"), coord, research, macros).await;
    Org { _dir: dir, app }
}

pub(crate) fn text(s: &str) -> ScriptedTurn {
    ScriptedTurn::Text(s.into())
}

fn tool(name: &str, args: serde_json::Value) -> ScriptedTurn {
    ScriptedTurn::ToolCalls(vec![ToolCall {
        id: format!("call-{name}"),
        name: name.into(),
        arguments: args.to_string(),
    }])
}

fn sent_text(s: &Sent) -> Option<(&str, &str)> {
    match &s.out {
        Outbound::Text { conversation, text } => Some((conversation.0.as_str(), text.as_str())),
        _ => None,
    }
}

fn sent_card(s: &Sent) -> Option<&Approval> {
    match &s.out {
        Outbound::Card { approval, .. } => Some(approval),
        _ => None,
    }
}

/// 第一条文字是 `first` 的对话。
async fn thread_starting(app: &App, first: &str) -> Option<ThreadId> {
    let store = app.runtime().store();
    for t in store.recent_threads(50).await.unwrap() {
        let all = store.thread_messages(&t.thread_id, None, 500).await.unwrap();
        if matches!(all.first().map(|m| &m.body), Some(Body::Text { text }) if text == first) {
            return Some(t.thread_id);
        }
    }
    None
}

/// 等对话里出现一条文字是 `want` 的消息。
async fn wait_for_text(app: &App, thread: &ThreadId, want: &str) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let all = app.runtime().store().thread_messages(thread, None, 500).await.unwrap();
        if all
            .iter()
            .any(|m| matches!(&m.body, Body::Text { text } if text == want))
        {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "等不到「{want}」：{all:?}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// 卡片上“交给 macro 岗位”是第几个选项。
fn macro_choice(card: &Approval) -> u32 {
    let ApprovalSubject::RouteMessage { roles, .. } = &card.subject else {
        panic!("应当是路由卡片：{card:?}")
    };
    roles.iter().position(|r| r.as_str() == "macro").unwrap() as u32
}

async fn assert_quiet(h: &FakeHandle) {
    if let Some(s) = h.next(QUIET).await {
        panic!("不该发出：{s:?}");
    }
}

// ---------------------------------------------------------------------------------------------
// T1–T11
// ---------------------------------------------------------------------------------------------

/// T1：新会话按路由分派，回复回到同一会话；人的消息带上渠道来源（D-03、D-27）。
#[tokio::test]
async fn t1_new_conversation_is_routed_and_the_reply_comes_back() {
    let o = org(vec![], vec![text("CPI 同比上涨 0.3%")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;

    h.say("c1", "u1", "点评一下本月 CPI").await.unwrap();
    let sent = h.next(WAIT).await.expect("应当回复");
    assert_eq!(sent_text(&sent), Some(("c1", "CPI 同比上涨 0.3%")));
    assert_quiet(&h).await;

    let thread = thread_starting(&o.app, "点评一下本月 CPI").await.unwrap();
    let messages = o
        .app
        .runtime()
        .store()
        .thread_messages(&thread, None, 10)
        .await
        .unwrap();
    assert_eq!(
        messages[0].author,
        Author::Human {
            name: "用户u1".into(),
            via: Some(Via {
                channel: "fake".into(),
                user_id: "u1".into()
            })
        }
    );
    assert_eq!(
        messages[1].author,
        Author::Agent {
            agent_id: "macro-1".into()
        }
    );
}

/// T1 补充：渠道配了固定员工时不走路由（D-16 的 `agent`）。
#[tokio::test]
async fn t1_fixed_agent_skips_routing() {
    let o = org(vec![], vec![text("我是小宏2")], 2).await;
    let (channel, h) = FakeChannel::new("fake");
    let settings = Settings {
        agent: Some("macro-2".into()),
        ..Settings::default()
    };
    let _a = attach(&o.app, channel, settings).await;

    // 这句话规则不命中，本来会出卡片。
    h.say("c1", "u1", "帮我看看这个").await.unwrap();
    let sent = h.next(WAIT).await.expect("应当直接回复");
    assert_eq!(sent_text(&sent), Some(("c1", "我是小宏2")));
    let thread = thread_starting(&o.app, "帮我看看这个").await.unwrap();
    let messages = o
        .app
        .runtime()
        .store()
        .thread_messages(&thread, None, 10)
        .await
        .unwrap();
    assert_eq!(
        messages[1].author,
        Author::Agent {
            agent_id: "macro-2".into()
        }
    );
}

/// T2：同一会话追问，交给同一个员工，不重新路由；没有绑定、也不能开启会话的消息被忽略（D-03）。
#[tokio::test]
async fn t2_follow_ups_stick_to_the_same_agent() {
    let o = org(vec![], vec![text("CPI 的回复"), text("PPI 的回复")], 2).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;

    h.say("c1", "u1", "点评一下本月 CPI").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "CPI 的回复")));

    // “再看看 PPI”规则不命中；要是重新路由会出卡片，粘住员工才会直接回复。
    h.reply("c1", "u1", "再看看 PPI").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "PPI 的回复")));
    let thread = thread_starting(&o.app, "点评一下本月 CPI").await.unwrap();
    let messages = o
        .app
        .runtime()
        .store()
        .thread_messages(&thread, None, 10)
        .await
        .unwrap();
    let first = Author::Agent {
        agent_id: "macro-1".into(),
    };
    let agents: Vec<&Author> = messages
        .iter()
        .map(|m| &m.author)
        .filter(|a| matches!(a, Author::Agent { .. }))
        .collect();
    assert_eq!(agents, [&first, &first]);

    h.reply("c9", "u1", "没有绑定的会话里接话").await.unwrap();
    assert_quiet(&h).await;
    assert!(thread_starting(&o.app, "没有绑定的会话里接话").await.is_none());
}

/// T3：路由拿不准 → 卡片发到会话 → 选定前的追问只记下 → 点击 → 交给选定岗位，之后的消息都粘住他（D-03、D-05）。
#[tokio::test]
async fn t3_uncertain_route_card_then_click_binds_the_agent() {
    let o = org(vec![], vec![text("交给我了"), text("继续处理")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;

    h.say("c1", "u1", "帮我看看这个").await.unwrap();
    let card_sent = h.next(WAIT).await.expect("应当发卡片");
    let card = sent_card(&card_sent).expect("应当是卡片").clone();
    assert_eq!(card_sent.out.conversation().0, "c1");

    // 卡片还没人选：消息只写进对话，不叫任何员工，也不发东西。
    h.reply("c1", "u1", "补充：要看同比").await.unwrap();
    assert_quiet(&h).await;

    h.click(&card.id, macro_choice(&card), "u1").await.unwrap();
    let sent = h.take(2, WAIT).await;
    let resolved = sent
        .iter()
        .find_map(|s| match &s.out {
            Outbound::CardResolved {
                conversation,
                approval,
                external_ref,
            } => Some((conversation.0.as_str(), approval, external_ref.as_str())),
            _ => None,
        })
        .expect("卡片应当更新");
    assert_eq!((resolved.0, resolved.2), ("c1", card_sent.external_ref.as_str()));
    assert!(matches!(resolved.1.state, ApprovalState::Resolved { .. }));
    assert!(
        sent.iter().any(|s| sent_text(s) == Some(("c1", "交给我了"))),
        "{sent:?}"
    );
    let thread = thread_starting(&o.app, "帮我看看这个").await.unwrap();
    wait_for_text(&o.app, &thread, "补充：要看同比").await;

    h.reply("c1", "u1", "还有一件事").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "继续处理")));
}

/// T4：协调员派活 → 执行员工 `ask_human` → 卡片出现在最初的会话 → 点击 → 执行员工继续；
/// 执行对话里的回复不发到渠道（D-06、T10）。
#[tokio::test]
async fn t4_card_from_delegated_work_reaches_the_original_conversation() {
    let coord = vec![
        tool("create_task", json!({"title": "写本周研报", "assignee": "macro-1"})),
        text("已派给小宏1"),
    ];
    let research = vec![
        tool(
            "ask_human",
            json!({
                "question": "先写 CPI 还是 PPI？",
                "options": [{"label": "CPI", "consequence": "先写 CPI"}, {"label": "PPI", "consequence": "先写 PPI"}],
                "recommended": 1
            }),
        ),
        text("等你决定"),
        text("开始写 PPI"),
    ];
    let o = org(coord, research, 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;

    h.say("c1", "u1", "写本周研报").await.unwrap();
    let sent = h.take(2, WAIT).await;
    assert!(
        sent.iter().any(|s| sent_text(s) == Some(("c1", "已派给小宏1"))),
        "{sent:?}"
    );
    let card_sent = sent
        .iter()
        .find(|s| sent_card(s).is_some())
        .expect("执行员工的卡片应当发到 c1");
    assert_eq!(card_sent.out.conversation().0, "c1");
    let card = sent_card(card_sent).unwrap().clone();
    let origin = thread_starting(&o.app, "写本周研报").await.unwrap();
    let ApprovalSubject::AgentQuestion { agent_id, thread_id } = &card.subject else {
        panic!("应当是员工提的问题：{card:?}")
    };
    assert_eq!(agent_id.as_str(), "macro-1");
    assert_ne!(thread_id, &origin, "卡片是在执行对话里提的，靠派活关系找回 c1");
    let work = thread_id.clone();

    h.click(&card.id, 1, "u1").await.unwrap();
    let resolved = h.next(WAIT).await.expect("卡片应当更新");
    assert!(
        matches!(&resolved.out, Outbound::CardResolved { external_ref, .. } if *external_ref == card_sent.external_ref)
    );
    wait_for_text(&o.app, &work, "开始写 PPI").await;
    // “等你决定”“开始写 PPI”都在执行对话里，不发到渠道。
    assert_quiet(&h).await;
}

/// T5：在控制台选定卡片后，渠道里的卡片同步更新（D-04）。
#[tokio::test]
async fn t5_console_choice_updates_the_channel_card() {
    let o = org(vec![], vec![text("好的")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;

    h.say("c1", "u1", "帮我看看这个").await.unwrap();
    let card_sent = h.next(WAIT).await.unwrap();
    let card = sent_card(&card_sent).expect("应当是卡片").clone();
    crate::work::resolve_approval(&o.app, &card.id, macro_choice(&card), "李四".into())
        .await
        .unwrap();
    let sent = h.take(2, WAIT).await;
    assert!(
        sent.iter().any(
            |s| matches!(&s.out, Outbound::CardResolved { conversation, external_ref, .. }
            if conversation.0 == "c1" && *external_ref == card_sent.external_ref)
        ),
        "{sent:?}"
    );
    assert!(sent.iter().any(|s| sent_text(s) == Some(("c1", "好的"))), "{sent:?}");
}

/// T6：同一条消息推送两次，只处理一次（D-03 第 1 步）。
#[tokio::test]
async fn t6_duplicate_delivery_is_processed_once() {
    let o = org(vec![], vec![text("只回一次")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;

    let first = h.say("c1", "u1", "点评一下本月 CPI").await.unwrap();
    h.send(first).await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "只回一次")));
    assert_quiet(&h).await;
    let thread = thread_starting(&o.app, "点评一下本月 CPI").await.unwrap();
    let messages = o
        .app
        .runtime()
        .store()
        .thread_messages(&thread, None, 10)
        .await
        .unwrap();
    let humans = messages
        .iter()
        .filter(|m| matches!(m.author, Author::Human { .. }))
        .count();
    assert_eq!(humans, 1, "{messages:?}");
}

/// T7：重新打开存储、重建投递器后，不重发也不漏发；绑定还在（D-04、D-08）。
#[tokio::test]
async fn t7_restart_neither_resends_nor_drops() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ip.db");
    let app = build(&db, vec![], vec![text("第一条回复")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let a = attach(&app, channel, Settings::default()).await;
    h.say("c1", "u1", "点评一下本月 CPI").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "第一条回复")));
    let thread = thread_starting(&app, "点评一下本月 CPI").await.unwrap();

    // 关掉渠道和投递器，期间对话里多了一条员工的消息。
    a.stop().await;
    app.runtime()
        .post(
            thread.clone(),
            Author::Agent {
                agent_id: "macro-1".into(),
            },
            "停机期间的补充",
        )
        .await
        .unwrap();
    app.shutdown().await;
    drop(app);

    // 重新打开同一个库，接上一个新的渠道。
    let app = build(&db, vec![], vec![text("重启后的回复")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&app, channel, Settings::default()).await;
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "停机期间的补充")));
    assert_quiet(&h).await;

    // 绑定存在库里，重启后同一会话照样交给原来的员工。
    h.reply("c1", "u1", "我回来了").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "重启后的回复")));
}

/// T8：发送失败后重试，顺序不乱；永久失败的跳过，后面的照常发（D-04）。
#[tokio::test]
async fn t8_retries_keep_order_and_permanent_failures_are_skipped() {
    let o = org(vec![], vec![text("开始")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;
    h.say("c1", "u1", "点评一下本月 CPI").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "开始")));
    let thread = thread_starting(&o.app, "点评一下本月 CPI").await.unwrap();
    let rt = o.app.runtime();
    let me = || Author::Agent {
        agent_id: "macro-1".into(),
    };

    let before = h.attempts();
    h.fail_next(
        2,
        DeliverError::Retry {
            after: Some(Duration::from_millis(50)),
            reason: "限流".into(),
        },
    );
    rt.post(thread.clone(), me(), "一").await.unwrap();
    rt.post(thread.clone(), me(), "二").await.unwrap();
    let sent = h.take(2, WAIT).await;
    let texts: Vec<_> = sent.iter().filter_map(sent_text).collect();
    assert_eq!(texts, [("c1", "一"), ("c1", "二")], "重试中的那条不能被后面的越过");
    assert_eq!(h.attempts() - before, 4, "两次失败加两次成功");

    h.fail_next(1, DeliverError::Permanent("机器人被移出频道".into()));
    rt.post(thread.clone(), me(), "三").await.unwrap();
    rt.post(thread.clone(), me(), "四").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "四")));
    assert_quiet(&h).await;
}

/// T9（一）：名单外的人发消息被忽略，不回复（D-03 第 2 步）。
#[tokio::test]
async fn t9_messages_from_outside_the_allow_list_are_ignored() {
    let o = org(vec![], vec![text("给 u1 的回复")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let settings = Settings {
        allow_users: vec!["u1".into()],
        ..Settings::default()
    };
    let _a = attach(&o.app, channel, settings).await;

    h.say("c2", "u2", "点评一下本月 CPI").await.unwrap();
    assert_quiet(&h).await;
    assert!(thread_starting(&o.app, "点评一下本月 CPI").await.is_none());

    h.say("c1", "u1", "点评一下本月 CPI").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "给 u1 的回复")));
}

/// T9（二）：不符合 `approvers` 的点击不生效，并把原因告诉点的人（D-07）。
#[tokio::test]
async fn t9_clicks_follow_the_approvers_rule() {
    for (approvers, outsider_allowed, insider) in [
        (Approvers::Requester, false, "u1"),
        (Approvers::Anyone, true, "u1"),
        (Approvers::Only(vec!["u3".into()]), false, "u3"),
    ] {
        let o = org(vec![], vec![text("好的")], 1).await;
        let (channel, h) = FakeChannel::new("fake");
        let settings = Settings {
            approvers: approvers.clone(),
            ..Settings::default()
        };
        let _a = attach(&o.app, channel, settings).await;
        h.say("c1", "u1", "帮我看看这个").await.unwrap();
        let card = sent_card(&h.next(WAIT).await.unwrap()).expect("应当是卡片").clone();
        let choice = macro_choice(&card);

        let outsider = if matches!(approvers, Approvers::Only(_)) {
            "u1"
        } else {
            "u2"
        };
        let r = h.click(&card.id, choice, outsider).await;
        if outsider_allowed {
            assert_eq!(r, Ok(()), "{approvers:?}");
            continue;
        }
        assert!(matches!(r, Err(HubError::Rejected(_))), "{approvers:?}：{r:?}");
        let pending = o.app.runtime().store().approvals(true, 10).await.unwrap();
        assert!(pending.iter().any(|a| a.id == card.id), "被拒绝的点击不能选定卡片");

        h.click(&card.id, choice, insider).await.unwrap();
        let pending = o.app.runtime().store().approvals(true, 10).await.unwrap();
        assert!(pending.is_empty(), "{approvers:?}");
    }
}

/// T10：绑定对话里员工的工具调用和工具结果不发到渠道，只发文字回复（D-04）。
#[tokio::test]
async fn t10_tool_traffic_stays_internal() {
    let o = org(vec![], vec![tool("list_tasks", json!({})), text("目前没有任务")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;

    h.say("c1", "u1", "点评一下本月 CPI 和任务").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "目前没有任务")));
    assert_quiet(&h).await;
    let thread = thread_starting(&o.app, "点评一下本月 CPI 和任务").await.unwrap();
    let messages = o
        .app
        .runtime()
        .store()
        .thread_messages(&thread, None, 10)
        .await
        .unwrap();
    assert!(
        messages.iter().any(|m| matches!(m.body, Body::ToolCall { .. })),
        "工具调用确实发生了"
    );
}

/// T11：绑定对话里只有“这轮失败”的系统消息发到渠道，任务说明、任务回报、选定说明都不发（D-26）。
#[tokio::test]
async fn t11_only_turn_failed_notices_reach_the_channel() {
    let o = org(vec![], vec![text("你好")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;
    h.say("c1", "u1", "点评一下本月 CPI").await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "你好")));
    let thread = thread_starting(&o.app, "点评一下本月 CPI").await.unwrap();

    let rt = o.app.runtime();
    rt.post_notice(thread.clone(), Notice::TaskBrief, "你被分派了任务「x」")
        .await
        .unwrap();
    rt.post_notice(thread.clone(), Notice::TaskReport, "任务「x」已完成")
        .await
        .unwrap();
    rt.post_notice(thread.clone(), Notice::ApprovalAnswer, "李四 回答了「…」：选「PPI」")
        .await
        .unwrap();
    rt.post_notice(thread.clone(), Notice::TurnFailed, "小宏1 的这轮工作失败：超时")
        .await
        .unwrap();
    assert_eq!(
        sent_text(&h.next(WAIT).await.unwrap()),
        Some(("c1", "小宏1 的这轮工作失败：超时"))
    );
    assert_quiet(&h).await;
}

// ---------------------------------------------------------------------------------------------
// T12–T14：处理失败后重试（D-28，#69）
// ---------------------------------------------------------------------------------------------

fn inbound(key: &str, conversation: &str, text: &str) -> Inbound {
    Inbound {
        channel: "fake".into(),
        dedupe_key: key.into(),
        conversation: conversation.into(),
        sender: Sender {
            external_id: "u1".into(),
            display_name: "用户u1".into(),
        },
        text: text.into(),
        may_start: true,
    }
}

async fn human_count(app: &App, thread: &ThreadId) -> usize {
    let messages = app.runtime().store().thread_messages(thread, None, 50).await.unwrap();
    messages
        .iter()
        .filter(|m| matches!(m.author, Author::Human { .. }))
        .count()
}

/// T12：Hub 内部出错的消息不记去重，渠道再交一次时照常处理，不被当成重复丢掉；处理成功后再来才算重复。
#[tokio::test]
async fn t12_failed_message_is_retried_not_swallowed_as_duplicate() {
    let o = org(vec![], vec![text("终于回了")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let settings = Settings {
        agent: Some("macro-9".into()),
        ..Settings::default()
    };
    let _a = attach(&o.app, channel, settings).await;
    let message = inbound("msg:t12", "c1", "点评一下本月 CPI");

    // 渠道指定的员工还没上岗：内部错误。
    let err = h.send(message.clone()).await.unwrap_err();
    assert!(matches!(err, HubError::Failed(_)), "{err:?}");
    assert_quiet(&h).await;

    // 员工上岗后渠道重试同一条消息：照常处理，回复回到会话。
    o.app
        .runtime()
        .spawn_agent(AgentSetup {
            agent: Agent {
                id: "macro-9".into(),
                role: "macro".into(),
                name: "小宏9".into(),
            },
            role: role("macro", "research/m"),
            toolbox: None,
            mcp: None,
            data: None,
            engine: None,
        })
        .unwrap();
    h.send(message.clone()).await.unwrap();
    assert_eq!(sent_text(&h.next(WAIT).await.unwrap()), Some(("c1", "终于回了")));

    // 处理成功之后再推：重复，不处理。
    h.send(message).await.unwrap();
    assert_quiet(&h).await;
    let thread = thread_starting(&o.app, "点评一下本月 CPI").await.unwrap();
    assert_eq!(human_count(&o.app, &thread).await, 1);
}

/// T13：上次开会话时绑定写好了、第一条消息没写进去就出错了。重试时接着开会话（按路由分派），
/// 不当成“等路由卡片”只记进对话。
#[tokio::test]
async fn t13_half_started_conversation_is_started_again() {
    let o = org(vec![], vec![text("CPI 同比上涨 0.3%")], 1).await;
    let (channel, h) = FakeChannel::new("fake");
    let _a = attach(&o.app, channel, Settings::default()).await;
    let thread = ThreadId::new();
    o.app
        .runtime()
        .store()
        .claim_binding(ip_store::Binding {
            channel: "fake".into(),
            conversation: "c1".into(),
            thread_id: thread.clone(),
            agent_id: None,
            requester: "u1".into(),
            created_at_ms: ip_core::now_ms(),
        })
        .await
        .unwrap();
    h.send(inbound("msg:t13", "c1", "点评一下本月 CPI")).await.unwrap();
    assert_eq!(
        sent_text(&h.next(WAIT).await.unwrap()),
        Some(("c1", "CPI 同比上涨 0.3%"))
    );
    assert_eq!(human_count(&o.app, &thread).await, 1);
}

/// T14：同一条消息同时交两次（比如渠道重试和外部系统重推撞在一起），只派一次活。
#[tokio::test]
async fn t14_concurrent_redeliveries_dispatch_once() {
    let o = org(vec![], vec![text("只回一次")], 1).await;
    let hub = crate::channels::hub_for_tests(&o.app, "fake", Settings::default());
    let message = inbound("msg:t14", "c1", "点评一下本月 CPI");
    let (a, b, c) = tokio::join!(
        hub.message(message.clone()),
        hub.message(message.clone()),
        hub.message(message.clone())
    );
    assert_eq!((a, b, c), (Ok(()), Ok(()), Ok(())));
    let thread = thread_starting(&o.app, "点评一下本月 CPI").await.unwrap();
    wait_for_text(&o.app, &thread, "只回一次").await;
    assert_eq!(human_count(&o.app, &thread).await, 1);
    let threads = o.app.runtime().store().recent_threads(10).await.unwrap();
    assert_eq!(threads.len(), 1, "只开了一个对话");
}
