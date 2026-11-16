use ip_core::{Author, Message, ThreadId};

use super::*;

fn temp_db(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ip-store-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("store.db")
}

fn text(thread: &str, at_ms: i64, body: &str) -> Message {
    let mut message = Message::text(ThreadId::from(thread), Author::human("张三"), body);
    message.at_ms = at_ms;
    message
}

#[tokio::test]
async fn finds_chinese_and_english_text() {
    let store = Store::open(temp_db("search")).unwrap();
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
    let store = Store::open(temp_db("thread-filter")).unwrap();
    store.append_message(text("a", 1, "合同草稿")).await.unwrap();
    store.append_message(text("b", 2, "合同终稿")).await.unwrap();
    let hits = store.search("合同", Some(&ThreadId::from("b")), 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].thread_id.as_str(), "b");
}

#[tokio::test]
async fn duplicate_id_fails_alone() {
    let store = Store::open(temp_db("duplicate")).unwrap();
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
    assert_eq!(store.last_seq().await.unwrap(), 2, "失败的写入不应留下事件");
}

#[tokio::test]
async fn concurrent_appends_are_batched_and_ordered() {
    let store = Store::open(temp_db("concurrent")).unwrap();
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
    let path = temp_db("reopen");
    {
        let store = Store::open(&path).unwrap();
        for i in 1..=5 {
            store.append_message(text("t", i, &format!("消息{i}"))).await.unwrap();
        }
    }
    let store = Store::open(&path).unwrap();
    let latest = store.thread_messages(&"t".into(), None, 2).await.unwrap();
    assert_eq!(latest.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [4, 5]);
    let earlier = store.thread_messages(&"t".into(), Some(4), 2).await.unwrap();
    assert_eq!(earlier.iter().map(|m| m.at_ms).collect::<Vec<_>>(), [2, 3]);
}

#[tokio::test]
async fn lists_recent_threads_with_preview() {
    let store = Store::open(temp_db("threads")).unwrap();
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
    let store = Store::open(temp_db("tasks")).unwrap();
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
    let store = Store::open(temp_db("approvals")).unwrap();
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
    let path = temp_db("settings");
    {
        let store = Store::open(&path).unwrap();
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
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.setting("router.spec").await.unwrap(),
        Some(serde_json::json!({"rules": [1]}))
    );
}

#[tokio::test]
async fn engine_sessions_upsert_without_events_and_survive_reopen() {
    let path = temp_db("engine");
    let (agent, thread) = (ip_core::AgentId::from("ops-1"), ThreadId::from("t1"));
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.engine_session(&agent, &thread).await.unwrap(), None);
        let before = store.last_seq().await.unwrap();
        let mut s = crate::EngineSession {
            agent_id: agent.clone(),
            thread_id: thread.clone(),
            engine: "codex".into(),
            session_id: "sess-1".into(),
            last_message: "m1".into(),
            updated_at_ms: 1,
        };
        store.put_engine_session(s.clone()).await.unwrap();
        s.last_message = "m2".into();
        s.updated_at_ms = 2;
        store.put_engine_session(s).await.unwrap();
        // 每轮都会更新，不写事件日志。
        assert_eq!(store.last_seq().await.unwrap(), before);
    }
    let store = Store::open(&path).unwrap();
    let s = store.engine_session(&agent, &thread).await.unwrap().unwrap();
    assert_eq!(
        (s.session_id.as_str(), s.last_message.as_str(), s.updated_at_ms),
        ("sess-1", "m2", 2)
    );
    assert_eq!(store.engine_session(&agent, &"t2".into()).await.unwrap(), None);
}

#[tokio::test]
async fn channel_bindings_claim_once_and_fill_in_the_agent() {
    let path = temp_db("bindings");
    let b = crate::Binding {
        channel: "slack".into(),
        conversation: "T1/C1/1.0".into(),
        thread_id: "t1".into(),
        agent_id: None,
        requester: "U1".into(),
        created_at_ms: 1,
    };
    {
        let store = Store::open(&path).unwrap();
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
    let store = Store::open(&path).unwrap();
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
    let path = temp_db("channel-envelopes");
    let store = Store::open(&path).unwrap();
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
    let store = Store::open(&path).unwrap();
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
    let store = Store::open(temp_db("channel-state")).unwrap();
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

    let path = temp_db("thread-memories");
    let store = Store::open(&path).unwrap();
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

    let store = Store::open(&path).unwrap();
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
    let path = temp_db("memory-notes");
    let store = Store::open(&path).unwrap();
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

    let store = Store::open(&path).unwrap();
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

    let store = Store::open(temp_db("forget-memories")).unwrap();
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
