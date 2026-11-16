//! Slack 端到端：假 Slack 服务器 → Slack 渠道 → Hub → 员工 → 投递器 → 假 Slack 服务器（D-11 到 D-16）。

use std::time::Duration;

use ip_connectors::fake::slack::{Failure, FakeSlack};
use ip_connectors::slack::{RetryPolicy, SlackChannel, SlackConfig};
use serde_json::{Value, json};

use crate::channel_tests::{build, text};

/// 一条 chat.postMessage 里 markdown 块的文字。
fn markdown(post: &Value) -> &str {
    post["blocks"][0]["text"].as_str().unwrap_or_default()
}

#[tokio::test]
async fn slack_thread_round_trip_with_a_routing_card() {
    let dir = tempfile::tempdir().unwrap();
    let app = build(
        &dir.path().join("ip.db"),
        vec![],
        vec![text("CPI 同比上涨 0.3%"), text("交给我了")],
        1,
    )
    .await;
    let fake = FakeSlack::start().await;
    fake.set_user("U1", "张三");
    let yaml = format!(
        "bot_token_env: BOT\napp_token_env: APP\napi_url: {}\nallow: {{ channels: [C1] }}\n",
        fake.api_url()
    );
    let config: crate::config::SlackChannelConfig = serde_saphyr::from_str(&yaml).unwrap();
    let env = |var: &str| Ok(format!("{var}-token"));
    let (slack, settings) = crate::app::slack_channel("slack", &config, env).unwrap();
    let handle = crate::channels::start(&app, slack, settings).await.unwrap();
    fake.wait_connected(1).await;

    // 在频道里 @机器人：按路由交给宏观研究员，回复发在同一个线程里。
    fake.event(
        json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> 点评一下本月 CPI", "ts": "1.0", "channel": "C1"}),
    );
    let posts = fake.wait_calls("chat.postMessage", 1).await;
    assert_eq!(
        (posts[0]["channel"].clone(), posts[0]["thread_ts"].clone()),
        (json!("C1"), json!("1.0"))
    );
    assert_eq!(markdown(&posts[0]), "CPI 同比上涨 0.3%");

    // 规则不命中：卡片发在线程里，按钮带着审批 id 和选项序号。
    fake.event(
        json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> 帮我看看这个", "ts": "2.0", "channel": "C1"}),
    );
    let posts = fake.wait_calls("chat.postMessage", 2).await;
    let card = &posts[1];
    assert_eq!(card["thread_ts"], json!("2.0"));
    let button = card["blocks"][1]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["text"]["text"] == json!("macro"))
        .expect("应当有交给 macro 岗位的按钮")
        .clone();

    // 别人点：不生效，只告诉他本人。提需求的人点：卡片更新，员工接手并回复。
    fake.click(
        "U2",
        "C1",
        "2.0",
        button["action_id"].as_str().unwrap(),
        button["value"].as_str().unwrap(),
        "7.0",
    );
    let ephemeral = fake.wait_calls("chat.postEphemeral", 1).await;
    assert_eq!(ephemeral[0]["user"], json!("U2"));
    fake.click(
        "U1",
        "C1",
        "2.0",
        button["action_id"].as_str().unwrap(),
        button["value"].as_str().unwrap(),
        "7.1",
    );
    let updates = fake.wait_calls("chat.update", 1).await;
    assert_eq!(updates[0]["ts"], json!("100.2"), "更新的是原来那张卡片");
    assert!(
        updates[0]["blocks"][1]["elements"][0]["text"]
            .as_str()
            .unwrap()
            .contains("张三（slack:U1） 选了「macro」")
    );
    let posts = fake.wait_calls("chat.postMessage", 3).await;
    assert_eq!(
        (posts[2]["thread_ts"].clone(), markdown(&posts[2])),
        (json!("2.0"), "交给我了")
    );

    // 名单外的频道：不理会。
    fake.event(json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> 点评 CPI", "ts": "3.0", "channel": "C9"}));
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(fake.calls("chat.postMessage").len(), 3);

    handle.stop().await;
}

/// #69：确认之后 `auth.test` 暂时 500，进程又在重试之前退出。重启后接着处理，回复照常发出；
/// 之后 Slack 再推同一条消息（新的信封 id），按消息去重，不再回一遍。
#[tokio::test]
async fn slack_event_acknowledged_before_exit_is_handled_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ip.db");
    let fake = FakeSlack::start().await;
    fake.set_user("U1", "张三");
    let slack = |wait: Duration| {
        SlackChannel::new(SlackConfig {
            name: "slack".into(),
            bot_token: "xoxb-test".into(),
            app_token: "xapp-test".into(),
            api_url: fake.api_url(),
            allow_channels: vec!["C1".into()],
            retry: RetryPolicy {
                first: wait,
                max: wait,
                attempts: 6,
            },
        })
    };
    let mention =
        json!({"type": "app_mention", "user": "U1", "text": "<@UBOT> 点评一下本月 CPI", "ts": "1.0", "channel": "C1"});

    // 第一次运行：重试要等 30 秒，等不到就退出了。
    let app = build(&db, vec![], vec![text("CPI 同比上涨 0.3%")], 1).await;
    let handle = crate::channels::start(&app, slack(Duration::from_secs(30)), Default::default())
        .await
        .unwrap();
    fake.wait_connected(1).await;
    fake.fail_next("auth.test", Failure::Status(500, None));
    let id = fake.event(mention.clone());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let pending = app.runtime().store().pending_envelopes("slack", 10).await.unwrap();
        if pending.first().is_some_and(|e| e.attempts == 1) {
            assert_eq!(pending.len(), 1);
            assert_eq!(pending[0].id, id);
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "等不到第一次处理失败：{pending:?}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(fake.acks().contains(&id), "已经向 Slack 确认");
    handle.stop().await;
    app.shutdown().await;
    drop(app);
    assert!(fake.calls("chat.postMessage").is_empty());

    // 重启：同一个库，新的渠道。没处理完的那条先处理。
    let app = build(&db, vec![], vec![text("CPI 同比上涨 0.3%")], 1).await;
    let handle = crate::channels::start(&app, slack(Duration::from_millis(20)), Default::default())
        .await
        .unwrap();
    let posts = fake.wait_calls("chat.postMessage", 1).await;
    assert_eq!(posts[0]["thread_ts"], json!("1.0"));
    assert_eq!(markdown(&posts[0]), "CPI 同比上涨 0.3%");
    assert!(
        app.runtime()
            .store()
            .pending_envelopes("slack", 10)
            .await
            .unwrap()
            .is_empty()
    );

    fake.wait_connected(2).await;
    fake.event(mention);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(fake.calls("chat.postMessage").len(), 1, "重推的消息不再回");
    handle.stop().await;
}

const CHANNELS: &str = r#"
roles:
  - id: macro
    description: 宏观研报。
    model: { default: ep/m }
endpoints:
  ep: { base_url: http://127.0.0.1:1 }
agents:
  - { id: macro-1, role: macro, name: 小宏 }
channels:
  slack:
    kind: slack
    bot_token_env: SLACK_BOT_TOKEN
    app_token_env: SLACK_APP_TOKEN
    allow: { users: [U1], channels: [C1] }
    approvers: [U1, U2]
    agent: macro-1
"#;

#[test]
fn channels_config_parses_and_is_checked() {
    let c = crate::Config::from_yaml(CHANNELS).unwrap();
    let crate::config::ChannelConfig::Slack(slack) = &c.channels["slack"];
    assert_eq!(slack.bot_token_env, "SLACK_BOT_TOKEN");
    let env = |var: &str| match var {
        "SLACK_BOT_TOKEN" => Ok("xoxb-1".to_string()),
        "SLACK_APP_TOKEN" => Ok("xapp-1".to_string()),
        _ => anyhow::bail!("没有 {var}"),
    };
    let (_, settings) = crate::app::slack_channel("slack", slack, env).unwrap();
    assert_eq!(settings.allow_users, ["U1"]);
    assert_eq!(
        settings.approvers,
        crate::channels::Approvers::Only(vec!["U1".into(), "U2".into()])
    );
    assert_eq!(settings.agent.as_ref().map(|a| a.as_str()), Some("macro-1"));
    let missing = |_: &str| -> anyhow::Result<String> { anyhow::bail!("没设置") };
    assert!(
        crate::app::slack_channel("slack", slack, missing).is_err(),
        "缺令牌时拒绝启动"
    );

    let anyone = CHANNELS.replace("approvers: [U1, U2]", "approvers: anyone");
    let c = crate::Config::from_yaml(&anyone).unwrap();
    let crate::config::ChannelConfig::Slack(slack) = &c.channels["slack"];
    assert_eq!(slack.approvers.to_rule(), Some(crate::channels::Approvers::Anyone));

    for (from, to, why) in [
        ("approvers: [U1, U2]", "approvers: everybody", "不认识的 approvers"),
        ("agent: macro-1", "agent: macro-9", "不存在的员工"),
        (
            "  slack:\n    kind: slack",
            "  \"slack:x\":\n    kind: slack",
            "渠道名带冒号",
        ),
        ("kind: slack", "kind: feishu", "还没实现的渠道"),
        (
            "bot_token_env: SLACK_BOT_TOKEN",
            "bot_token: xoxb-明文",
            "令牌不能写进配置文件",
        ),
    ] {
        let text = CHANNELS.replacen(from, to, 1);
        assert!(crate::Config::from_yaml(&text).is_err(), "{why} 应被拒绝");
    }
}
