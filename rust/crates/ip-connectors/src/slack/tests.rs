//! Slack 渠道对着假 Slack 服务器（`fake::slack`，需要 `fake` feature）。

#![cfg(feature = "fake")]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use ip_core::{Approval, ApprovalOption, ApprovalState, ApprovalSubject};
use serde_json::json;

use super::*;
use crate::fake::slack::{Failure, FakeSlack};

/// 记下收到的消息和点击；`U9` 的点击按规则拒绝。信封存在内存里，代替 `ip-server` 的数据库。
#[derive(Default)]
struct Recorder {
    messages: Mutex<Vec<Inbound>>,
    clicks: Mutex<Vec<CardClick>>,
    /// 接下来这么多次 `message` 返回内部错误。
    fail_messages: std::sync::atomic::AtomicUsize,
    /// `message` 被调用的次数，含失败的。
    message_calls: std::sync::atomic::AtomicUsize,
    /// 还没处理完的信封。
    pending: Mutex<Vec<Stashed>>,
    /// 存过的全部信封 id。
    known: Mutex<std::collections::HashSet<String>>,
    /// 每次 `settle` 的记录。
    settled: Mutex<Vec<(String, Settle)>>,
}

impl Recorder {
    fn settled(&self) -> Vec<(String, Settle)> {
        self.settled.lock().unwrap().clone()
    }

    fn pending_ids(&self) -> Vec<String> {
        self.pending.lock().unwrap().iter().map(|s| s.id.clone()).collect()
    }
}

impl Hub for Recorder {
    fn message(&self, inbound: Inbound) -> BoxFuture<'_, Result<(), HubError>> {
        use std::sync::atomic::Ordering;
        Box::pin(async move {
            self.message_calls.fetch_add(1, Ordering::SeqCst);
            // 还有要失败的次数就减一。不用 fetch_update：新版 Rust 把它标成弃用了，旧版又没有替代的 try_update。
            let mut left = self.fail_messages.load(Ordering::SeqCst);
            let failing = loop {
                if left == 0 {
                    break false;
                }
                match self
                    .fail_messages
                    .compare_exchange(left, left - 1, Ordering::SeqCst, Ordering::SeqCst)
                {
                    Ok(_) => break true,
                    Err(now) => left = now,
                }
            };
            if failing {
                return Err(HubError::Failed("数据库暂时不可用".into()));
            }
            self.messages.lock().unwrap().push(inbound);
            Ok(())
        })
    }

    fn stash(&self, id: &str, payload: &str) -> BoxFuture<'_, Result<bool, HubError>> {
        let (id, payload) = (id.to_string(), payload.to_string());
        Box::pin(async move {
            let mut known = self.known.lock().unwrap();
            let pending = self.pending.lock().unwrap();
            if !known.insert(id.clone()) || pending.iter().any(|s| s.id == id) {
                return Ok(false);
            }
            drop(pending);
            self.pending.lock().unwrap().push(Stashed {
                id,
                payload,
                attempts: 0,
            });
            Ok(true)
        })
    }

    fn stashed(&self) -> BoxFuture<'_, Result<Vec<Stashed>, HubError>> {
        Box::pin(async move { Ok(self.pending.lock().unwrap().clone()) })
    }

    fn settle(&self, id: &str, settle: Settle) -> BoxFuture<'_, Result<(), HubError>> {
        let id = id.to_string();
        Box::pin(async move {
            let mut pending = self.pending.lock().unwrap();
            match &settle {
                Settle::Failed(_) => pending.iter_mut().filter(|s| s.id == id).for_each(|s| s.attempts += 1),
                Settle::Done | Settle::GaveUp(_) => pending.retain(|s| s.id != id),
            }
            self.settled.lock().unwrap().push((id, settle));
            Ok(())
        })
    }

    fn click(&self, click: CardClick) -> BoxFuture<'_, Result<(), HubError>> {
        Box::pin(async move {
            if click.sender.external_id == "U9" {
                return Err(HubError::Rejected("只有提需求的人能选这张卡片".into()));
            }
            self.clicks.lock().unwrap().push(click);
            Ok(())
        })
    }
}

struct Running {
    fake: FakeSlack,
    channel: Arc<SlackChannel>,
    hub: Arc<Recorder>,
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

async fn running(allow_channels: &[&str]) -> Running {
    running_with(Recorder::default(), allow_channels).await
}

/// 测试里重试间隔很短，最多处理 3 次。
fn quick_retry() -> RetryPolicy {
    RetryPolicy {
        first: Duration::from_millis(20),
        max: Duration::from_millis(100),
        attempts: 3,
    }
}

async fn running_with(hub: Recorder, allow_channels: &[&str]) -> Running {
    let fake = FakeSlack::start().await;
    fake.set_user("U1", "张三");
    let channel = SlackChannel::new(SlackConfig {
        name: "slack".into(),
        bot_token: "xoxb-test".into(),
        app_token: "xapp-test".into(),
        api_url: fake.api_url(),
        allow_channels: allow_channels.iter().map(|s| s.to_string()).collect(),
        retry: quick_retry(),
    });
    let hub = Arc::new(hub);
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(channel.clone().run(hub.clone(), shutdown.clone()));
    fake.wait_connected(1).await;
    Running {
        fake,
        channel,
        hub,
        shutdown,
        task,
    }
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "等不到：{what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn approval(id: &str, state: ApprovalState) -> Approval {
    Approval {
        id: id.into(),
        question: "先写 <CPI> 还是 PPI？".into(),
        options: vec![
            ApprovalOption {
                label: "CPI".into(),
                consequence: "先写 CPI".into(),
            },
            ApprovalOption {
                label: "PPI".into(),
                consequence: "先写 PPI & 对比".into(),
            },
        ],
        recommended: Some(1),
        reason: "PPI 刚公布".into(),
        subject: ApprovalSubject::Question,
        state,
        created_at_ms: 0,
    }
}

/// D-12、D-13：哪些事件交给 Hub、交过去是什么样子；每个信封都确认。
#[tokio::test]
async fn socket_mode_events_become_inbound_messages() {
    let r = running(&["C1", "D1"]).await;
    let f = &r.fake;
    let events = [
        // 1. @机器人：开启会话，去掉提及。
        json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> 点评一下 CPI", "ts": "1.0", "channel": "C1"}),
        // 2. 同一条消息的 message 事件：内容和去重 key 都一样。
        json!({"type": "message", "channel_type": "channel", "user": "U1", "text": "<@UBOT> 点评一下 CPI", "ts": "1.0", "channel": "C1"}),
        // 3. 线程里接话：不能开启会话，会话按线程根划分。
        json!({"type": "message", "channel_type": "channel", "user": "U1", "text": "再看看 PPI", "ts": "1.1", "thread_ts": "1.0", "channel": "C1"}),
        // 4–6. 别的机器人、自己、编辑：忽略。
        json!({"type": "message", "bot_id": "B5", "user": "U5", "text": "我是别的机器人", "ts": "1.2", "channel": "C1"}),
        json!({"type": "message", "user": "UBOT", "text": "我自己", "ts": "1.3", "channel": "C1"}),
        json!({"type": "message", "subtype": "message_changed", "channel": "C1", "ts": "1.4", "message": {"text": "改过"}}),
        // 7. 私信的顶层消息：开启会话。
        json!({"type": "message", "channel_type": "im", "user": "U1", "text": "私信你", "ts": "2.0", "channel": "D1"}),
        // 8. 不在允许名单里的频道：忽略。
        json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> 这里不行", "ts": "3.0", "channel": "C9"}),
    ];
    let ids: Vec<String> = events.into_iter().map(|e| f.event(e)).collect();
    wait_until("四条消息交给 Hub", || r.hub.messages.lock().unwrap().len() >= 4).await;
    wait_until("全部信封确认", || f.acks().len() >= ids.len()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(f.acks(), ids);
    // 每个信封都先存下、处理完再删掉（含按规则忽略的）。
    wait_until("全部信封处理完", || r.hub.settled().len() >= ids.len()).await;
    assert!(r.hub.pending_ids().is_empty());
    assert!(r.hub.settled().iter().all(|(_, s)| *s == Settle::Done));

    let got = r.hub.messages.lock().unwrap().clone();
    let summary: Vec<(String, String, bool, String)> = got
        .iter()
        .map(|m| {
            (
                m.conversation.0.clone(),
                m.text.clone(),
                m.may_start,
                m.dedupe_key.clone(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("T1/C1/1.0".into(), "点评一下 CPI".into(), true, "msg:C1:1.0".into()),
            ("T1/C1/1.0".into(), "点评一下 CPI".into(), true, "msg:C1:1.0".into()),
            ("T1/C1/1.0".into(), "再看看 PPI".into(), false, "msg:C1:1.1".into()),
            ("T1/D1/2.0".into(), "私信你".into(), true, "msg:D1:2.0".into()),
        ]
    );
    assert!(got.iter().all(|m| m.channel == "slack"
        && m.sender
            == Sender {
                external_id: "U1".into(),
                display_name: "张三".into()
            }));
    assert_eq!(f.calls("users.info").len(), 1, "显示名有缓存");
    assert_eq!(f.calls("auth.test").len(), 1, "机器人身份只查一次");

    r.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), r.task)
        .await
        .unwrap()
        .unwrap();
}

/// 点击交给 Hub；被拒绝的点击，只给点的人看原因（D-07、D-14）。
#[tokio::test]
async fn card_clicks_reach_the_hub_and_refusals_go_to_the_clicker() {
    let r = running(&[]).await;
    let f = &r.fake;
    f.click("U1", "C1", "1.0", "ip_choice_1", "a1:1", "5.0");
    f.click("U9", "C1", "1.0", "ip_choice_0", "a1:0", "5.1");
    f.click("U1", "C1", "1.0", "something_else", "x", "5.2");
    let ephemeral = f.wait_calls("chat.postEphemeral", 1).await;
    assert_eq!(
        ephemeral[0],
        json!({"channel": "C1", "user": "U9", "text": "只有提需求的人能选这张卡片", "thread_ts": "1.0"})
    );
    wait_until("点击交给 Hub", || !r.hub.clicks.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let clicks = r.hub.clicks.lock().unwrap().clone();
    assert_eq!(clicks.len(), 1, "不是我们按钮的点击不交给 Hub：{clicks:?}");
    assert_eq!(clicks[0].approval_id.as_str(), "a1");
    assert_eq!(clicks[0].choice, 1);
    assert_eq!(clicks[0].dedupe_key, "click:5.0:U1");
    assert_eq!(clicks[0].sender.display_name, "张三");
}

/// 文字用 markdown 块发到线程里，太长就切开，工作区不认 markdown 块就退回纯文字；
/// 卡片是 Block Kit 按钮，选定后用 chat.update 改原卡片（D-14）。
#[tokio::test]
async fn delivers_text_cards_and_card_updates() {
    let r = running(&[]).await;
    let (f, ch) = (&r.fake, &r.channel);
    let text = |t: &str| Outbound::Text {
        conversation: "T1/C1/1.0".into(),
        text: t.into(),
    };

    let sent = ch.deliver(&text("第一段\n\n第二段")).await.unwrap();
    let posts = f.calls("chat.postMessage");
    assert_eq!(sent.external_ref, "100.1");
    assert_eq!(
        (posts[0]["channel"].clone(), posts[0]["thread_ts"].clone()),
        (json!("C1"), json!("1.0"))
    );
    assert_eq!(
        posts[0]["blocks"],
        json!([{"type": "markdown", "text": "第一段\n\n第二段"}])
    );

    let long = format!("{}\n\n{}", "长".repeat(10_000), "段".repeat(10_000));
    ch.deliver(&text(&long)).await.unwrap();
    assert_eq!(f.calls("chat.postMessage").len(), 3, "超过上限按段落切成两条");

    f.fail_next("chat.postMessage", Failure::Error("invalid_blocks".into()));
    ch.deliver(&text("退回纯文字")).await.unwrap();
    let posts = f.calls("chat.postMessage");
    let last = posts.last().unwrap();
    assert_eq!(last["text"], json!("退回纯文字"));
    assert!(last.get("blocks").is_none(), "{last}");

    let card = Outbound::Card {
        conversation: "T1/C1/1.0".into(),
        approval: approval("a1", ApprovalState::Pending),
    };
    let card_ref = ch.deliver(&card).await.unwrap().external_ref;
    let blocks = f.calls("chat.postMessage").last().unwrap()["blocks"].clone();
    assert_eq!(
        blocks[0]["text"]["text"],
        json!("*先写 &lt;CPI&gt; 还是 PPI？*\nPPI 刚公布")
    );
    let buttons = blocks[1]["elements"].as_array().unwrap();
    assert_eq!(buttons.len(), 2);
    assert_eq!(buttons[1]["value"], json!("a1:1"));
    assert_eq!(buttons[1]["action_id"], json!("ip_choice_1"));
    assert_eq!(buttons[1]["style"], json!("primary"), "推荐项高亮");
    assert!(buttons[0].get("style").is_none());
    assert!(
        blocks[2]["elements"][0]["text"]
            .as_str()
            .unwrap()
            .contains("先写 PPI &amp; 对比")
    );

    let resolved = Outbound::CardResolved {
        conversation: "T1/C1/1.0".into(),
        approval: approval(
            "a1",
            ApprovalState::Resolved {
                choice: 1,
                by: "张三（slack:U1）".into(),
                at_ms: 1,
            },
        ),
        external_ref: card_ref.clone(),
    };
    ch.deliver(&resolved).await.unwrap();
    let update = &f.calls("chat.update")[0];
    assert_eq!(
        (update["channel"].clone(), update["ts"].clone()),
        (json!("C1"), json!(card_ref))
    );
    let blocks = update["blocks"].as_array().unwrap();
    assert_eq!(blocks.len(), 2, "按钮去掉了");
    assert_eq!(
        blocks[1]["elements"][0]["text"],
        json!("✅ 张三（slack:U1） 选了「PPI」")
    );
}

/// 发送失败分成可重试和永久失败（D-04、D-14）。
#[tokio::test]
async fn delivery_errors_are_retry_or_permanent() {
    let r = running(&[]).await;
    let (f, ch) = (&r.fake, &r.channel);
    let out = Outbound::Text {
        conversation: "T1/C1/1.0".into(),
        text: "你好".into(),
    };
    let cases = [
        (Failure::Status(429, Some(7)), Some(Some(Duration::from_secs(7)))),
        (Failure::Status(503, None), Some(None)),
        (Failure::Error("ratelimited".into()), Some(None)),
        (Failure::Error("channel_not_found".into()), None),
        (Failure::Status(404, None), None),
    ];
    for (failure, retry) in cases {
        f.fail_next("chat.postMessage", failure.clone());
        let err = ch.deliver(&out).await.unwrap_err();
        match (retry, err) {
            (Some(after), DeliverError::Retry { after: got, .. }) => assert_eq!(got, after, "{failure:?}"),
            (None, DeliverError::Permanent(_)) => {}
            (want, got) => panic!("{failure:?}：要 {want:?}，得到 {got:?}"),
        }
    }
    let bad = Outbound::Text {
        conversation: "nope".into(),
        text: "x".into(),
    };
    assert!(matches!(ch.deliver(&bad).await, Err(DeliverError::Permanent(_))));
}

/// Slack 要求换连接时马上重连，重连后照常收事件（D-11）。
#[tokio::test]
async fn reconnects_when_slack_asks() {
    let r = running(&[]).await;
    let f = &r.fake;
    f.disconnect();
    f.wait_connected(2).await;
    assert_eq!(f.opens(), 2);
    f.event(json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> 还在吗", "ts": "9.0", "channel": "C1"}));
    wait_until("重连后的消息交给 Hub", || {
        !r.hub.messages.lock().unwrap().is_empty()
    })
    .await;
    r.shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), r.task)
        .await
        .unwrap()
        .unwrap();
}

fn mention(text: &str, ts: &str) -> serde_json::Value {
    json!({"type": "app_mention", "user": "U1", "text": format!("<@UBOT> {text}"), "ts": ts, "channel": "C1"})
}

/// #69：确认之后 `auth.test` 暂时 500，事件不丢，退避后重试成功；之后的消息照常处理、排在它后面。
#[tokio::test]
async fn acknowledged_message_survives_a_temporary_auth_test_failure() {
    let r = running(&[]).await;
    let f = &r.fake;
    f.fail_next("auth.test", Failure::Status(500, None));
    let first = f.event(mention("第一条", "1.0"));
    wait_until("确认第一条", || f.acks().contains(&first)).await;
    let second = f.event(mention("第二条", "2.0"));
    wait_until("两条都交给 Hub", || r.hub.messages.lock().unwrap().len() >= 2).await;
    let texts: Vec<String> = r.hub.messages.lock().unwrap().iter().map(|m| m.text.clone()).collect();
    assert_eq!(texts, ["第一条", "第二条"], "按收到的先后处理");
    assert_eq!(f.calls("auth.test").len(), 2, "失败一次，重试一次");
    wait_until("都处理完", || r.hub.pending_ids().is_empty()).await;
    let settled = r.hub.settled();
    assert!(
        matches!(&settled[0], (id, Settle::Failed(e)) if *id == first && e.contains("auth.test")),
        "{settled:?}"
    );
    assert_eq!(&settled[1..], [(first, Settle::Done), (second, Settle::Done)]);
}

/// #69：Hub 内部出错的消息退避重试；一直失败的到次数上限放弃，不挡后面的消息。
#[tokio::test]
async fn hub_failures_are_retried_then_given_up() {
    use std::sync::atomic::Ordering;
    let hub = Recorder::default();
    hub.fail_messages.store(1, Ordering::SeqCst);
    let r = running_with(hub, &[]).await;
    let f = &r.fake;
    let once = f.event(mention("失败一次", "1.0"));
    wait_until("重试后交给 Hub", || r.hub.messages.lock().unwrap().len() == 1).await;
    assert_eq!(r.hub.message_calls.load(Ordering::SeqCst), 2);

    r.hub.fail_messages.store(100, Ordering::SeqCst);
    let broken = f.event(mention("一直失败", "2.0"));
    wait_until("放弃", || {
        r.hub.settled().iter().any(|(_, s)| matches!(s, Settle::GaveUp(_)))
    })
    .await;
    // 最多处理 3 次。
    assert_eq!(r.hub.message_calls.load(Ordering::SeqCst), 2 + 3);
    let settles: Vec<Settle> = r
        .hub
        .settled()
        .into_iter()
        .filter(|(id, _)| *id == broken)
        .map(|(_, s)| s)
        .collect();
    assert!(
        matches!(
            settles.as_slice(),
            [Settle::Failed(_), Settle::Failed(_), Settle::GaveUp(_)]
        ),
        "{settles:?}"
    );
    assert!(r.hub.settled().contains(&(once, Settle::Done)));

    r.hub.fail_messages.store(0, Ordering::SeqCst);
    f.event(mention("后面的", "3.0"));
    wait_until("后面的照常处理", || r.hub.messages.lock().unwrap().len() == 2).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(r.hub.message_calls.load(Ordering::SeqCst), 2 + 3 + 1, "放弃后不再重试");
}

/// #69：上次确认了但没处理完就退出的信封，下次启动先处理；Slack 再推同一个信封不处理第二遍。
#[tokio::test]
async fn envelopes_left_from_last_run_are_processed_first() {
    let envelope = |id: &str, text: &str, ts: &str| {
        json!({
            "envelope_id": id,
            "type": "events_api",
            "payload": {"type": "event_callback", "team_id": "T1", "event": mention(text, ts)},
        })
        .to_string()
    };
    let hub = Recorder::default();
    hub.pending.lock().unwrap().extend([
        Stashed {
            id: "old-1".into(),
            payload: envelope("old-1", "上次的一", "0.1"),
            attempts: 1,
        },
        Stashed {
            id: "old-2".into(),
            payload: envelope("old-2", "上次的二", "0.2"),
            attempts: 0,
        },
    ]);
    hub.known
        .lock()
        .unwrap()
        .extend(["old-1".to_string(), "old-2".to_string()]);
    let r = running_with(hub, &[]).await;
    let f = &r.fake;
    f.push(serde_json::from_str(&envelope("old-2", "上次的二", "0.2")).unwrap());
    f.event(mention("新的", "1.0"));
    wait_until("三条都交给 Hub", || r.hub.messages.lock().unwrap().len() >= 3).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let texts: Vec<String> = r.hub.messages.lock().unwrap().iter().map(|m| m.text.clone()).collect();
    assert_eq!(texts, ["上次的一", "上次的二", "新的"]);
    assert!(f.acks().contains(&"old-2".to_string()), "重推的照样确认");
    assert!(r.hub.pending_ids().is_empty());
}

/// #69：停止时正在退避的信封留着，下次启动再处理。
#[tokio::test]
async fn stopping_during_backoff_leaves_the_envelope_for_next_start() {
    let hub = Recorder::default();
    hub.fail_messages.store(1, std::sync::atomic::Ordering::SeqCst);
    let mut r = running_with(hub, &[]).await;
    r.channel = SlackChannel::new(SlackConfig {
        retry: RetryPolicy {
            first: Duration::from_secs(30),
            ..quick_retry()
        },
        ..r.channel.config.clone()
    });
    // 换成退避很长的渠道再跑一遍。
    r.shutdown.cancel();
    r.task.await.unwrap();
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(r.channel.clone().run(r.hub.clone(), shutdown.clone()));
    r.fake.wait_connected(2).await;
    let id = r.fake.event(mention("会失败一次", "1.0"));
    wait_until("第一次失败", || !r.hub.settled().is_empty()).await;
    shutdown.cancel();
    tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.hub.pending_ids(), std::slice::from_ref(&id), "没处理完的留着");
    assert!(r.hub.messages.lock().unwrap().is_empty());

    // 再启动：先处理它。
    let shutdown = CancellationToken::new();
    let task = tokio::spawn(r.channel.clone().run(r.hub.clone(), shutdown.clone()));
    wait_until("重启后交给 Hub", || r.hub.messages.lock().unwrap().len() == 1).await;
    assert!(r.hub.pending_ids().is_empty());
    shutdown.cancel();
    task.await.unwrap();
}

#[test]
fn splitting_and_parsing_helpers() {
    assert_eq!(split_text("短", 10), ["短"]);
    // 每段 5 个字（含换行），两段合起来 10 个字超过 7，所以一段一条；正好 8 个字不超过 8，就不切。
    let parts = split_text("一一一\n\n二二二\n\n三三三", 7);
    assert_eq!(parts, ["一一一\n\n", "二二二\n\n", "三三三"]);
    assert_eq!(split_text("二二二\n\n三三三", 8), ["二二二\n\n三三三"]);
    assert!(split_text(&"字".repeat(25), 10).iter().all(|p| p.chars().count() <= 10));
    assert_eq!(split_conversation("T1/C1/1.0"), Some(("T1", "C1", "1.0")));
    assert_eq!(split_conversation("T1/C1"), None);
    assert_eq!(
        parse_choice(&json!({"action_id": "ip_choice_2", "value": "a-b-c:2"})),
        Some(("a-b-c".into(), 2))
    );
    assert_eq!(parse_choice(&json!({"action_id": "other", "value": "a:1"})), None);
    let me = Me {
        user_id: "UBOT".into(),
        team_id: "T1".into(),
    };
    // 只 @ 了机器人、没有别的字：忽略。
    let empty = json!({"type": "app_mention", "user": "U1", "text": "<@UBOT>", "ts": "1.0", "channel": "C1"});
    assert_eq!(parse_message(&empty, &me, "T1", &[]), None);
    // 线程里 @机器人：可以开启会话，会话是这个线程。
    let in_thread = json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> 看下", "ts": "1.5", "thread_ts": "1.0", "channel": "C1"});
    let m = parse_message(&in_thread, &me, "T1", &[]).unwrap();
    assert_eq!((m.conversation.as_str(), m.may_start), ("T1/C1/1.0", true));
}
