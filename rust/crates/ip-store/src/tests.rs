use ip_core::{Author, Message, ThreadId};

use super::*;

fn text(thread: &str, at_ms: i64, body: &str) -> Message {
    let mut message = Message::text(ThreadId::from(thread), Author::human("张三"), body);
    message.at_ms = at_ms;
    message
}

#[tokio::test]
async fn finds_chinese_and_english_text() {
    let store = crate::testing::store().await;
    store
        .append_message(text("t1", 1, "请帮我整理一下本周的销售报表"))
        .await
        .unwrap();
    store
        .append_message(text("t1", 2, "客户张三的合同已经签好了"))
        .await
        .unwrap();
    store
        .append_message(text("t2", 3, "Please send the quarterly report"))
        .await
        .unwrap();

    let hits = |q: &'static str| {
        let store = store.clone();
        async move {
            store
                .search(q, None, 10)
                .await
                .unwrap()
                .into_iter()
                .map(|hit| hit.text)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(hits("销售报表").await, ["请帮我整理一下本周的销售报表"]);
    assert_eq!(hits("合同").await, ["客户张三的合同已经签好了"]);
    assert_eq!(hits("REPORT").await, ["Please send the quarterly report"]);
    assert_eq!(hits("合同 张三").await, ["客户张三的合同已经签好了"]);
    assert_eq!(hits("售").await, ["请帮我整理一下本周的销售报表"]);
    assert!(hits("报销").await.is_empty(), "不相邻的字不应命中");
    assert!(hits("，").await.is_empty());
}

#[tokio::test]
async fn filters_by_thread() {
    let store = crate::testing::store().await;
    store.append_message(text("a", 1, "合同草稿")).await.unwrap();
    store.append_message(text("b", 2, "合同终稿")).await.unwrap();
    let hits = store.search("合同", Some(&ThreadId::from("b")), 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].thread_id.as_str(), "b");
}

#[tokio::test]
async fn duplicate_id_fails_alone() {
    let store = crate::testing::store().await;
    let first = text("t", 1, "第一条");
    let mut copy = text("t", 2, "复制");
    copy.id = first.id.clone();
    let other = text("t", 3, "第三条");

    let (a, b, c) = tokio::join!(
        store.append_message(first),
        store.append_message(copy),
        store.append_message(other)
    );
    assert!(a.is_ok());
    assert!(matches!(b, Err(StoreError::DuplicateMessage(_))));
    assert!(c.is_ok());
    let messages = store.thread_messages(&"t".into(), None, 10).await.unwrap();
    assert_eq!(messages.len(), 2);
    // PostgreSQL 的自增序号回滚后不收回，seq 可能跳号；要看的是事件条数。
    let events = store.events_after(0, 100).await.unwrap();
    assert_eq!(events.len(), 2, "失败的写入不应留下事件");
}

#[tokio::test]
async fn concurrent_appends_are_batched_and_ordered() {
    let store = crate::testing::store().await;
    let tasks: Vec<_> = (0..1000)
        .map(|i| {
            let store = store.clone();
            tokio::spawn(async move { store.append_message(text("bulk", i, &format!("第{i}条消息"))).await })
        })
        .collect();
    let mut seqs = Vec::new();
    for task in tasks {
        seqs.push(task.await.unwrap().unwrap());
    }
    seqs.sort_unstable();
    seqs.dedup();
    assert_eq!(seqs.len(), 1000);
    assert_eq!(store.last_seq().await.unwrap(), 1000);
}

#[tokio::test]
async fn pages_a_thread_and_survives_reopen() {
    let db = crate::testing::TestDb::new();
    {
        let store = db.open().await;
        for i in 1..=5 {
            store.append_message(text("t", i, &format!("消息{i}"))).await.unwrap();
        }
    }
    let store = db.open().await;
    let latest = store.thread_messages(&"t".into(), None, 2).await.unwrap();
    assert_eq!(latest.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [4, 5]);
    let earlier = store.thread_messages(&"t".into(), Some(4), 2).await.unwrap();
    assert_eq!(earlier.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [2, 3]);
}

#[tokio::test]
async fn lists_recent_threads_with_preview() {
    let store = crate::testing::store().await;
    store.append_message(text("旧", 1, "第一条")).await.unwrap();
    let hello = text("新", 2, "你好");
    store.append_message(hello.clone()).await.unwrap();
    let long = "长".repeat(200);
    store.append_message(text("旧", 3, &long)).await.unwrap();
    let dup = store.append_message(hello).await;
    assert!(dup.is_err(), "重复 id 不应计数");

    let threads = store.recent_threads(10).await.unwrap();
    let ids: Vec<&str> = threads.iter().map(|t| t.thread_id.as_str()).collect();
    assert_eq!(ids, ["旧", "新"]);
    assert_eq!(threads[0].message_count, 2);
    assert_eq!(threads[0].preview.as_deref(), Some("长".repeat(80).as_str()));
    assert_eq!(threads[1].message_count, 1);
    assert_eq!(store.recent_threads(1).await.unwrap().len(), 1);
}

fn task(id: &str, status: ip_core::TaskStatus, updated: i64) -> ip_core::Task {
    ip_core::Task {
        id: id.into(),
        title: format!("任务{id}"),
        description: String::new(),
        status,
        role: None,
        assignee: None,
        thread_id: None,
        origin_thread_id: None,
        result: None,
        depth: 1,
        created_by: Author::System,
        created_at_ms: 1,
        updated_at_ms: updated,
        due_ms: None,
    }
}

#[tokio::test]
async fn tasks_upsert_and_filter() {
    use ip_core::TaskStatus::*;
    let store = crate::testing::store().await;
    store.put_task(task("a", Open, 1)).await.unwrap();
    store.put_task(task("b", InProgress, 2)).await.unwrap();
    store.put_task(task("a", Done, 3)).await.unwrap();
    let all = store.tasks(false, 10).await.unwrap();
    assert_eq!(all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    let open = store.tasks(true, 10).await.unwrap();
    assert_eq!(open.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["b"]);
    assert_eq!(store.task(&"a".into()).await.unwrap().unwrap().status, Done);
    assert!(store.task(&"zz".into()).await.unwrap().is_none());
    assert_eq!(store.last_seq().await.unwrap(), 3, "每次变更一条事件");
}

#[tokio::test]
async fn approval_resolves_exactly_once() {
    use ip_core::{Approval, ApprovalOption, ApprovalState, ApprovalSubject};
    let store = crate::testing::store().await;
    let card = Approval {
        id: "c1".into(),
        question: "这条消息交给谁？".into(),
        options: vec![
            ApprovalOption {
                label: "宏观".into(),
                consequence: "交给宏观研究员".into(),
            },
            ApprovalOption {
                label: "运维".into(),
                consequence: "交给运维".into(),
            },
        ],
        recommended: Some(0),
        reason: String::new(),
        subject: ApprovalSubject::Question,
        state: ApprovalState::Pending,
        created_at_ms: 1,
    };
    store.put_approval(card.clone()).await.unwrap();
    assert!(store.put_approval(card).await.is_err(), "同 id 不能建两次");
    assert_eq!(store.approvals(true, 10).await.unwrap().len(), 1);

    assert!(matches!(
        store.resolve_approval(&"c1".into(), 5, "张三").await,
        Err(StoreError::Conflict(_))
    ));
    let tasks: Vec<_> = ["张三", "李四"]
        .into_iter()
        .map(|who| {
            let store = store.clone();
            tokio::spawn(async move { store.resolve_approval(&"c1".into(), 1, who).await })
        })
        .collect();
    let mut ok = 0;
    for t in tasks {
        if t.await.unwrap().is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, 1, "同时点两次只生效一次");
    assert!(store.approvals(true, 10).await.unwrap().is_empty());
    let all = store.approvals(false, 10).await.unwrap();
    assert!(matches!(all[0].state, ApprovalState::Resolved { choice: 1, .. }));
    assert!(matches!(
        store.resolve_approval(&"nope".into(), 0, "x").await,
        Err(StoreError::NotFound(_))
    ));
}

#[tokio::test]
async fn settings_overwrite_and_survive_reopen() {
    let db = crate::testing::TestDb::new();
    {
        let store = db.open().await;
        assert_eq!(store.setting("router.spec").await.unwrap(), None);
        store
            .put_setting("router.spec", serde_json::json!({"rules": []}))
            .await
            .unwrap();
        store
            .put_setting("router.spec", serde_json::json!({"rules": [1]}))
            .await
            .unwrap();
    }
    let store = db.open().await;
    assert_eq!(
        store.setting("router.spec").await.unwrap(),
        Some(serde_json::json!({"rules": [1]}))
    );
}

#[tokio::test]
async fn engine_sessions_upsert_without_events_and_survive_reopen() {
    let db = crate::testing::TestDb::new();
    let (agent, thread) = (ip_core::AgentId::from("ops-1"), ThreadId::from("t1"));
    {
        let store = db.open().await;
        assert_eq!(store.engine_session(&agent, &thread).await.unwrap(), None);
        let before = store.last_seq().await.unwrap();
        let mut s = crate::EngineSession {
            agent_id: agent.clone(),
            thread_id: thread.clone(),
            engine: "codex".into(),
            session_id: "sess-1".into(),
            last_message: "m1".into(),
            updated_at_ms: 1,
            used: crate::SessionUsage::default(),
        };
        store.put_engine_session(s.clone()).await.unwrap();
        s.last_message = "m2".into();
        s.updated_at_ms = 2;
        s.used = crate::SessionUsage {
            prompt_tokens: 25_006,
            cached_tokens: 12_489,
            cache_write_tokens: 12_511,
            completion_tokens: 23,
        };
        store.put_engine_session(s).await.unwrap();
        // 每轮都会更新，不写事件日志。
        assert_eq!(store.last_seq().await.unwrap(), before);
    }
    let store = db.open().await;
    let s = store.engine_session(&agent, &thread).await.unwrap().unwrap();
    assert_eq!(
        (s.session_id.as_str(), s.last_message.as_str(), s.updated_at_ms),
        ("sess-1", "m2", 2)
    );
    // 累计用量跟着会话存下来，重开还在。
    assert_eq!(
        (
            s.used.prompt_tokens,
            s.used.cached_tokens,
            s.used.cache_write_tokens,
            s.used.completion_tokens
        ),
        (25_006, 12_489, 12_511, 23)
    );
    assert_eq!(store.engine_session(&agent, &"t2".into()).await.unwrap(), None);
}

#[tokio::test]
async fn migrations_run_once_and_reopening_is_harmless() {
    let db = crate::testing::TestDb::new();
    // 两个连接同时打开：迁移在 advisory lock 下只跑一次，谁都不报错。
    let (a, b) = tokio::join!(db.open(), db.open());
    a.append_message(text("t1", 1, "第一次")).await.unwrap();
    drop((a, b));
    let store = db.open().await;
    let client = store.reader().await.unwrap();
    let versions: i64 = client
        .query_one("SELECT COUNT(*) FROM schema_migrations", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(versions, crate::schema::MIGRATIONS.len() as i64);
    assert_eq!(store.last_seq().await.unwrap(), 1);
}

fn by_agent(thread: &str, at_ms: i64, agent: &str, body: &str) -> Message {
    let mut message = Message::text(ThreadId::from(thread), Author::Agent { agent_id: agent.into() }, body);
    message.at_ms = at_ms;
    message
}

#[tokio::test]
async fn threads_remember_the_last_agent_who_replied() {
    let store = crate::testing::store().await;
    store.append_message(text("t", 1, "帮我看看 CPI")).await.unwrap();
    let last = |store: &Store| {
        let store = store.clone();
        async move { store.recent_threads(10).await.unwrap()[0].last_agent_id.clone() }
    };
    assert_eq!(last(&store).await, None, "员工还没回");
    store.append_message(by_agent("t", 2, "macro-1", "好的")).await.unwrap();
    store.append_message(text("t", 3, "再看看 PPI")).await.unwrap();
    assert_eq!(last(&store).await, Some("macro-1".into()), "人的消息不改归属");
    store
        .append_message(by_agent("t", 4, "coord-1", "我来派活"))
        .await
        .unwrap();
    assert_eq!(last(&store).await, Some("coord-1".into()));
    let between = store.threads_last_active_between(0, 10, 10).await.unwrap();
    assert_eq!(between[0].last_agent_id, Some("coord-1".into()));
}

/// 迁移 2 之前就有的对话：升级时按消息补上最后回复的员工。
#[tokio::test]
async fn migration_backfills_the_last_agent_of_old_threads() {
    let db = crate::testing::TestDb::new();
    let store = db.open().await;
    store
        .append_message(by_agent("t1", 1, "macro-1", "第一版"))
        .await
        .unwrap();
    store
        .append_message(by_agent("t1", 2, "ops-1", "第二版"))
        .await
        .unwrap();
    store.append_message(text("t1", 3, "人说的")).await.unwrap();
    store.append_message(text("t2", 4, "没人回")).await.unwrap();
    // 退回迁移 1 的样子，再打开一次让迁移 2 重新跑。
    store
        .reader()
        .await
        .unwrap()
        .batch_execute("ALTER TABLE threads DROP COLUMN last_agent_id; DELETE FROM schema_migrations WHERE version = 2")
        .await
        .unwrap();
    drop(store);
    let store = db.open().await;
    let threads = store.recent_threads(10).await.unwrap();
    let agent = |id: &str| {
        threads
            .iter()
            .find(|t| t.thread_id.as_str() == id)
            .and_then(|t| t.last_agent_id.clone())
    };
    assert_eq!(agent("t1"), Some("ops-1".into()));
    assert_eq!(agent("t2"), None);
}

#[tokio::test]
async fn nul_characters_are_stored_as_replacement_characters() {
    let store = crate::testing::store().await;
    store.append_message(text("t1", 1, "工具输出\0二进制")).await.unwrap();
    let messages = store.thread_messages(&"t1".into(), None, 10).await.unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(store.search("二进制", None, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn rejects_bad_schema_names() {
    let mut config = crate::DbConfig::new("postgres://localhost/ip");
    config.schema = "ip; DROP TABLE x".into();
    assert!(matches!(Store::connect(&config).await, Err(StoreError::Config(_))));
}

#[tokio::test]
async fn channel_bindings_claim_once_and_fill_in_the_agent() {
    let db = crate::testing::TestDb::new();
    let b = crate::Binding {
        channel: "slack".into(),
        conversation: "T1/C1/1.0".into(),
        thread_id: "t1".into(),
        agent_id: None,
        requester: "U1".into(),
        created_at_ms: 1,
    };
    {
        let store = db.open().await;
        let before = store.last_seq().await.unwrap();
        assert_eq!(store.claim_binding(b.clone()).await.unwrap(), b);
        // 同一会话再来一次：返回已有的，不另开对话，不写事件。
        let other = crate::Binding {
            thread_id: "t2".into(),
            requester: "U2".into(),
            ..b.clone()
        };
        assert_eq!(store.claim_binding(other).await.unwrap(), b);
        assert_eq!(store.last_seq().await.unwrap(), before + 1);
        let events = store.events_after(before, 10).await.unwrap();
        assert_eq!(events[0].kind, "binding.put");

        let filled = store
            .set_binding_agent("slack", "T1/C1/1.0", &"macro-1".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(filled.agent_id.as_ref().map(|a| a.as_str()), Some("macro-1"));
        // 已经有员工就不再改。
        let kept = store
            .set_binding_agent("slack", "T1/C1/1.0", &"macro-2".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(kept.agent_id.as_ref().map(|a| a.as_str()), Some("macro-1"));
        assert_eq!(
            store
                .set_binding_agent("slack", "none", &"macro-2".into())
                .await
                .unwrap(),
            None
        );
    }
    let store = db.open().await;
    let by_thread = store.binding_by_thread(&"t1".into()).await.unwrap().unwrap();
    assert_eq!(
        (by_thread.conversation.as_str(), by_thread.requester.as_str()),
        ("T1/C1/1.0", "U1")
    );
    assert_eq!(by_thread.agent_id.as_ref().map(|a| a.as_str()), Some("macro-1"));
    assert_eq!(store.binding_by_thread(&"t2".into()).await.unwrap(), None);
    assert_eq!(store.binding("slack", "other").await.unwrap(), None);
}

#[tokio::test]
async fn stashed_envelopes_survive_reopen_until_settled() {
    use crate::{Envelope, Settle};
    let db = crate::testing::TestDb::new();
    let store = db.open().await;
    assert!(store.stash_envelope("slack", "env-1", "{\"a\":1}").await.unwrap());
    assert!(store.stash_envelope("slack", "env-2", "{\"a\":2}").await.unwrap());
    assert!(store.stash_envelope("slack", "env-3", "{\"a\":3}").await.unwrap());
    assert!(store.stash_envelope("email", "env-1", "x").await.unwrap(), "按渠道分开");
    // 同一个 id 重推：不覆盖。
    assert!(!store.stash_envelope("slack", "env-1", "changed").await.unwrap());
    store.settle_envelope("slack", "env-2", Settle::Done).await.unwrap();
    store
        .settle_envelope("slack", "env-3", Settle::Failed("auth.test 500".into()))
        .await
        .unwrap();
    assert!(!store.inbound_seen("slack", "msg:C1:1.0").await.unwrap());
    store.remember_inbound("slack", "msg:C1:1.0").await.unwrap();
    assert!(store.inbound_seen("slack", "msg:C1:1.0").await.unwrap());
    drop(store);

    // 进程退出再打开：没处理完的还在，按收到的先后，带着失败次数。
    let store = db.open().await;
    let envelope = |id: &str, payload: &str, attempts| Envelope {
        id: id.into(),
        payload: payload.into(),
        attempts,
    };
    assert_eq!(
        store.pending_envelopes("slack", 10).await.unwrap(),
        [envelope("env-1", "{\"a\":1}", 0), envelope("env-3", "{\"a\":3}", 1)]
    );
    assert_eq!(store.pending_envelopes("slack", 1).await.unwrap().len(), 1);
    // 放弃的不再列出，过了保留期删掉。
    store
        .settle_envelope("slack", "env-3", Settle::GaveUp("还是 500".into()))
        .await
        .unwrap();
    store.settle_envelope("slack", "env-1", Settle::Done).await.unwrap();
    assert!(store.pending_envelopes("slack", 10).await.unwrap().is_empty());
    // 处理完的还认得：Slack 重推同一个信封时不再存一遍。
    assert!(!store.stash_envelope("slack", "env-1", "{\"a\":1}").await.unwrap());
    assert!(!store.stash_envelope("slack", "env-2", "{\"a\":2}").await.unwrap());
    assert!(store.pending_envelopes("slack", 10).await.unwrap().is_empty());
    // 去重记录 1 条，slack 的信封 3 个，email 的那个还没处理完，留着。
    assert_eq!(store.prune_inbound(ip_core::now_ms() + 1).await.unwrap(), 4);
    assert_eq!(store.pending_envelopes("email", 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn channel_inbox_deliveries_and_cursors() {
    let store = crate::testing::store().await;
    let before = store.last_seq().await.unwrap();
    assert!(store.remember_inbound("slack", "msg:C1:1.0").await.unwrap());
    assert!(!store.remember_inbound("slack", "msg:C1:1.0").await.unwrap());
    assert!(
        store.remember_inbound("email", "msg:C1:1.0").await.unwrap(),
        "按渠道分开"
    );
    assert_eq!(store.prune_inbound(ip_core::now_ms() + 1).await.unwrap(), 2);
    assert!(
        store.remember_inbound("slack", "msg:C1:1.0").await.unwrap(),
        "清理后重新算新的"
    );

    assert_eq!(store.delivery("slack", "card:a1").await.unwrap(), None);
    let d = crate::Delivery {
        channel: "slack".into(),
        item: "card:a1".into(),
        conversation: "T1/C1/1.0".into(),
        external_ref: Some("1.5".into()),
        sent: true,
        error: None,
        at_ms: 2,
    };
    store.put_delivery(d.clone()).await.unwrap();
    let report = crate::Delivery {
        item: "report:room:x.md:1234".into(),
        ..d.clone()
    };
    store.put_delivery(report.clone()).await.unwrap();
    store.prune_inbound(2).await.unwrap();
    assert_eq!(
        store.delivery("slack", &report.item).await.unwrap(),
        Some(report.clone())
    );
    store.prune_inbound(3).await.unwrap();
    assert!(store.delivery("slack", &report.item).await.unwrap().is_none());
    assert_eq!(store.delivery("slack", "card:a1").await.unwrap(), Some(d));

    assert_eq!(store.cursor("slack").await.unwrap(), None);
    store.put_cursor("slack", 41).await.unwrap();
    store.put_cursor("slack", 42).await.unwrap();
    assert_eq!(store.cursor("slack").await.unwrap(), Some(42));
    // 这些都是运行状态，不写事件日志。
    assert_eq!(store.last_seq().await.unwrap(), before);
}

#[tokio::test]
async fn thread_memories_upsert_per_role_and_survive_reopen() {
    use crate::{MemoryStatus, ThreadMemory};

    let db = crate::testing::TestDb::new();
    let store = db.open().await;
    for (thread, at) in [("t-old", 100), ("t-mid", 200), ("t-new", 300)] {
        store.append_message(text(thread, at, "你好")).await.unwrap();
    }
    // 找凉下来的对话：按最后一条消息的时间取一段，新的在前。
    let ids = |rows: Vec<crate::ThreadSummary>| rows.into_iter().map(|t| t.thread_id.to_string()).collect::<Vec<_>>();
    assert_eq!(
        ids(store.threads_last_active_between(100, 200, 10).await.unwrap()),
        ["t-mid", "t-old"]
    );
    assert_eq!(
        ids(store.threads_last_active_between(0, 1000, 1).await.unwrap()),
        ["t-new"]
    );

    let before = store.last_seq().await.unwrap();
    let memory = |role: &str, thread: &str, generated_ms: i64| ThreadMemory {
        role_id: role.into(),
        thread_id: thread.into(),
        source_updated_ms: 100,
        generated_ms,
        status: MemoryStatus::Ok,
        slug: Some("收益率口径".into()),
        summary: Some("用 DGS10。".into()),
        attempts: 0,
        retry_after_ms: None,
    };
    assert_eq!(
        store.thread_memory(&"macro".into(), &"t-old".into()).await.unwrap(),
        None
    );
    store.put_thread_memory(memory("macro", "t-old", 1)).await.unwrap();
    store.put_thread_memory(memory("macro", "t-mid", 2)).await.unwrap();
    // 同一个对话，另一个岗位各存各的。
    store.put_thread_memory(memory("ops", "t-old", 3)).await.unwrap();
    let failed = ThreadMemory {
        status: MemoryStatus::Failed,
        attempts: 2,
        retry_after_ms: Some(999),
        ..memory("macro", "t-new", 4)
    };
    store.put_thread_memory(failed.clone()).await.unwrap();
    // 覆盖同一条。
    let updated = ThreadMemory {
        source_updated_ms: 150,
        summary: Some("用 DGS10，注意日期。".into()),
        ..memory("macro", "t-old", 5)
    };
    store.put_thread_memory(updated.clone()).await.unwrap();
    // 提炼结果可以重新生成，不写事件日志。
    assert_eq!(store.last_seq().await.unwrap(), before);
    drop(store);

    let store = db.open().await;
    assert_eq!(
        store.thread_memory(&"macro".into(), &"t-old".into()).await.unwrap(),
        Some(updated.clone())
    );
    assert_eq!(
        store.thread_memory(&"macro".into(), &"t-new".into()).await.unwrap(),
        Some(failed)
    );
    // 只列有摘要的，最近生成的在前；别的岗位的不混进来。
    assert_eq!(
        store.thread_memories(&"macro".into(), 10).await.unwrap(),
        vec![updated, memory("macro", "t-mid", 2)]
    );
    assert_eq!(store.thread_memories(&"ops".into(), 10).await.unwrap().len(), 1);
    assert_eq!(store.thread_memories(&"macro".into(), 1).await.unwrap().len(), 1);

    // 合并时的选择：只要还留着摘要的（失败但留着旧摘要的也算），太久没动的不选，有上限。
    let picked = |since: i64, limit: usize| {
        let store = store.clone();
        async move {
            let rows = store
                .thread_memories_for_consolidation(&"macro".into(), since, limit)
                .await
                .unwrap();
            rows.into_iter().map(|m| m.thread_id.to_string()).collect::<Vec<_>>()
        }
    };
    assert_eq!(picked(0, 10).await, ["t-old", "t-new", "t-mid"]);
    assert_eq!(picked(4, 10).await, ["t-old", "t-new"]);
    assert_eq!(picked(0, 1).await, ["t-old"]);
    store
        .put_thread_memory(ThreadMemory {
            status: MemoryStatus::Empty,
            summary: None,
            ..memory("macro", "t-mid", 9)
        })
        .await
        .unwrap();
    assert_eq!(picked(0, 10).await, ["t-old", "t-new"]);

    // 搜索：每个词都要中（标题或正文），英文不分大小写，只看本岗位有摘要的。
    let found = |terms: &[&str]| {
        let store = store.clone();
        let terms: Vec<String> = terms.iter().map(|t| (*t).to_owned()).collect();
        async move {
            let rows = store.search_thread_memories(&"macro".into(), &terms, 10).await.unwrap();
            rows.into_iter().map(|m| m.thread_id.to_string()).collect::<Vec<_>>()
        }
    };
    assert_eq!(found(&["dgs10"]).await, ["t-old"]);
    assert_eq!(found(&["口径", "日期"]).await, ["t-old"]);
    assert_eq!(found(&["口径", "没有的词"]).await, Vec::<String>::new());
    assert_eq!(found(&[]).await, Vec::<String>::new());
    assert_eq!(
        found(&["%"]).await,
        Vec::<String>::new(),
        "不是 LIKE，通配符没有特殊含义"
    );

    // 读过的记使用次数：用得多的排前面，重新提炼不清零，别的岗位和不存在的不受影响。
    let macro_role: ip_core::RoleId = "macro".into();
    assert!(
        store
            .touch_thread_memory(&macro_role, &"t-new".into(), 50)
            .await
            .unwrap()
    );
    assert!(
        store
            .touch_thread_memory(&macro_role, &"t-new".into(), 60)
            .await
            .unwrap()
    );
    assert!(
        !store
            .touch_thread_memory(&macro_role, &"nope".into(), 60)
            .await
            .unwrap()
    );
    assert_eq!(picked(0, 10).await, ["t-new", "t-old"]);
    store.put_thread_memory(memory("macro", "t-new", 7)).await.unwrap();
    assert_eq!(
        store.thread_memory_usage(&macro_role, &"t-new".into()).await.unwrap(),
        Some((2, Some(60)))
    );
    assert_eq!(
        store.thread_memory_usage(&"ops".into(), &"t-old".into()).await.unwrap(),
        Some((0, None))
    );
    // 最近用过的不算“太久没动”，哪怕生成得早。
    assert_eq!(picked(55, 10).await, ["t-new"]);
}

#[tokio::test]
async fn memory_notes_are_kept_per_role_until_applied() {
    let db = crate::testing::TestDb::new();
    let store = db.open().await;
    let (macro_role, ops): (ip_core::RoleId, ip_core::RoleId) = ("macro".into(), "ops".into());
    let a = store
        .add_memory_note(&macro_role, Some(&"t1".into()), "收益率写两位小数", 100)
        .await
        .unwrap();
    let b = store
        .add_memory_note(&macro_role, None, "忘掉上周的口径", 200)
        .await
        .unwrap();
    let c = store.add_memory_note(&ops, None, "发布前先冒烟", 300).await.unwrap();
    assert!(a < b && b < c);
    assert_eq!(store.pending_memory_note_count(&macro_role).await.unwrap(), 2);

    // 用别的岗位的名义标不掉；标过的不重复标。
    store.apply_memory_notes(&ops, vec![a], 400).await.unwrap();
    store.apply_memory_notes(&macro_role, vec![a], 500).await.unwrap();
    store.apply_memory_notes(&macro_role, vec![a], 600).await.unwrap();
    drop(store);

    let store = db.open().await;
    let notes = store.memory_notes(&macro_role, 0, 10).await.unwrap();
    let seen: Vec<_> = notes.iter().map(|n| (n.id, n.text.as_str(), n.applied_ms)).collect();
    assert_eq!(seen, [(a, "收益率写两位小数", Some(500)), (b, "忘掉上周的口径", None)]);
    assert_eq!(notes[0].thread_id.as_ref().map(|t| t.as_str()), Some("t1"));
    assert_eq!(store.pending_memory_note_count(&macro_role).await.unwrap(), 1);
    // 太早的不取；条数有上限时留最近的。
    assert_eq!(store.memory_notes(&macro_role, 150, 10).await.unwrap().len(), 1);
    assert_eq!(store.memory_notes(&macro_role, 0, 1).await.unwrap()[0].id, b);
    assert_eq!(store.memory_notes(&ops, 0, 10).await.unwrap().len(), 1);

    // 人删更正：一条一条删，或整个岗位清掉；别的岗位的删不到。
    assert_eq!(store.delete_memory_notes(&ops, Some(b)).await.unwrap(), 0);
    assert_eq!(store.delete_memory_notes(&macro_role, Some(b)).await.unwrap(), 1);
    assert_eq!(store.delete_memory_notes(&macro_role, None).await.unwrap(), 1);
    assert!(store.memory_notes(&macro_role, 0, 10).await.unwrap().is_empty());
    assert_eq!(store.memory_notes(&ops, 0, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn forgotten_thread_memories_keep_their_row_but_lose_their_content() {
    use crate::{MemoryStatus, ThreadMemory};

    let store = crate::testing::store().await;
    let memory = |role: &str, thread: &str| ThreadMemory {
        role_id: role.into(),
        thread_id: thread.into(),
        source_updated_ms: 100,
        generated_ms: 1,
        status: MemoryStatus::Ok,
        slug: Some("口径".into()),
        summary: Some("用 DGS10。".into()),
        attempts: 0,
        retry_after_ms: None,
    };
    for (role, thread) in [
        ("macro", "t1"),
        ("macro", "t2"),
        ("macro", "t3"),
        ("ops", "t1"),
        ("ops", "t4"),
    ] {
        store.put_thread_memory(memory(role, thread)).await.unwrap();
    }
    let (macro_role, ops): (ip_core::RoleId, ip_core::RoleId) = ("macro".into(), "ops".into());
    store.touch_thread_memory(&macro_role, &"t2".into(), 50).await.unwrap();
    let listed = |role: &ip_core::RoleId| {
        let (store, role) = (store.clone(), role.clone());
        async move {
            let rows = store.thread_memories_with_usage(&role, 10).await.unwrap();
            rows.into_iter()
                .map(|(m, used, at)| (m.thread_id.to_string(), used, at))
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(listed(&macro_role).await.len(), 3);
    assert!(listed(&macro_role).await.contains(&("t2".into(), 1, Some(50))));

    // 抹一条：行还在，记成没什么可记的，提炼时的对话时间不变（没有新消息就不会再提炼）。
    assert_eq!(
        store
            .forget_thread_memories(Some(&macro_role), Some(&"t2".into()))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        store
            .forget_thread_memories(Some(&macro_role), Some(&"t2".into()))
            .await
            .unwrap(),
        0
    );
    let row = store.thread_memory(&macro_role, &"t2".into()).await.unwrap().unwrap();
    assert_eq!(
        (row.status, row.summary, row.source_updated_ms),
        (MemoryStatus::Empty, None, 100)
    );
    // 抹一个对话在各岗位下的全部；抹一个岗位的全部；什么都不指定时不动。
    assert_eq!(store.forget_thread_memories(None, Some(&"t1".into())).await.unwrap(), 2);
    assert_eq!(store.forget_thread_memories(None, None).await.unwrap(), 0);
    assert_eq!(listed(&ops).await.len(), 1);
    assert_eq!(store.forget_thread_memories(Some(&macro_role), None).await.unwrap(), 1);
    assert!(listed(&macro_role).await.is_empty());
    assert_eq!(listed(&ops).await.len(), 1);
}

fn turn(agent: &str, role: &str, thread: &str, started: i64, finished: i64, steps: u32, ok: bool) -> TurnRecord {
    TurnRecord {
        agent_id: agent.into(),
        role_id: role.into(),
        thread_id: thread.into(),
        started_ms: started,
        finished_ms: finished,
        steps,
        ok,
        cost_usd: 0.01 * f64::from(steps),
        error: (!ok).then(|| format!("模型报错{}", "很长".repeat(400))),
    }
}

#[tokio::test]
async fn turns_record_and_summarize() {
    use ip_core::{Approval, ApprovalOption, ApprovalState, ApprovalSubject, TaskStatus::*};
    let store = crate::testing::store().await;
    let before = store.last_seq().await.unwrap();
    store
        .record_turn(turn("r1", "macro", "t1", 100, 200, 3, true))
        .await
        .unwrap();
    store
        .record_turn(turn("r1", "macro", "t1", 300, 450, 4, false))
        .await
        .unwrap();
    store
        .record_turn(turn("r2", "macro", "t2", 120, 180, 2, true))
        .await
        .unwrap();
    store
        .record_turn(turn("o1", "ops", "t3", 500, 900, 9, true))
        .await
        .unwrap();
    assert_eq!(store.last_seq().await.unwrap(), before, "轮次不进事件日志");

    let totals = store.turn_totals(&["t1".into(), "t9".into()]).await.unwrap();
    assert_eq!(totals.len(), 1, "没有记录的对话不在结果里");
    assert_eq!(
        totals[&ThreadId::from("t1")],
        TurnTotals {
            turns: 2,
            steps: 7,
            active_ms: 250,
            last_finished_ms: 450
        }
    );
    assert!(store.turn_totals(&[]).await.unwrap().is_empty());

    let in_threads = store.turns_in_threads(&["t1".into()]).await.unwrap();
    assert_eq!(in_threads.iter().map(|t| t.started_ms).collect::<Vec<_>>(), [100, 300]);
    let failed = &in_threads[1];
    assert!(!failed.ok);
    assert_eq!(
        failed.error.as_ref().unwrap().chars().count(),
        TURN_ERROR_CHARS,
        "失败原因截短"
    );

    let ok_only = store.recent_turns(&"macro".into(), true, 10).await.unwrap();
    assert_eq!(ok_only.iter().map(|t| t.finished_ms).collect::<Vec<_>>(), [200, 180]);
    let all = store.recent_turns(&"macro".into(), false, 1).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].finished_ms, 450);

    let last = store
        .last_turns(&["r1".into(), "o1".into(), "nobody".into()])
        .await
        .unwrap();
    let mut last: Vec<_> = last.iter().map(|t| (t.agent_id.to_string(), t.finished_ms)).collect();
    last.sort();
    assert_eq!(last, [("o1".to_string(), 900), ("r1".to_string(), 450)]);

    let cost = store.turn_cost_since(210).await.unwrap();
    assert!(
        (cost[&RoleId::from("macro")] - 0.04).abs() < 1e-9,
        "只算 210 之后结束的"
    );
    assert!((cost[&RoleId::from("ops")] - 0.09).abs() < 1e-9);

    // 做完的任务按岗位找。
    let mut done = task("d1", Done, 50);
    done.role = Some("macro".into());
    store.put_task(done).await.unwrap();
    let mut old = task("d0", Done, 5);
    old.role = Some("macro".into());
    store.put_task(old).await.unwrap();
    let mut open = task("d2", InProgress, 60);
    open.role = Some("macro".into());
    store.put_task(open).await.unwrap();
    let found = store.done_tasks(&"macro".into(), 10, 10).await.unwrap();
    assert_eq!(found.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["d1"]);
    assert!(store.done_tasks(&"ops".into(), 0, 10).await.unwrap().is_empty());

    // 员工提问卡片按对话找，别的卡片不算。
    let card = |id: &str, subject: ApprovalSubject, at: i64| Approval {
        id: id.into(),
        question: "要哪个口径？".into(),
        options: vec![ApprovalOption {
            label: "同比".into(),
            consequence: "按同比算".into(),
        }],
        recommended: None,
        reason: String::new(),
        subject,
        state: ApprovalState::Pending,
        created_at_ms: at,
    };
    let asked = |thread: &str| ApprovalSubject::AgentQuestion {
        agent_id: "r1".into(),
        thread_id: thread.into(),
    };
    store.put_approval(card("q2", asked("t1"), 320)).await.unwrap();
    store.put_approval(card("q1", asked("t1"), 150)).await.unwrap();
    store.put_approval(card("q3", asked("t2"), 160)).await.unwrap();
    store
        .put_approval(card("q4", ApprovalSubject::Question, 170))
        .await
        .unwrap();
    let times = store.question_times(&["t1".into(), "t3".into()]).await.unwrap();
    assert_eq!(times, [(ThreadId::from("t1"), 150), (ThreadId::from("t1"), 320)]);
}
