//! 控制台“记忆”页背后的操作（`docs/ip/context-memory-design.md` 5.6）：看一个岗位的记忆，
//! 改决策者笔记和岗位记忆摘要，抹掉某个对话的记忆，删更正，整个清空，按对话关掉记忆。

use ip_core::{RoleId, ThreadId, now_ms};
use ip_runtime::memory::{self, Layer};

use crate::App;
use crate::protocol::{CorrectionView, MemoryLayer, MemoryText, MemoryView, ThreadMemoryView};

/// 一页最多列多少份对话记忆和多少条更正。
const THREADS_LIMIT: usize = 200;
const CORRECTIONS_LIMIT: usize = 200;
/// 人直接写的文本最多多少字。放进提示词时还会按 `memory` 配置里的上限截。
const TEXT_MAX_CHARS: usize = 20_000;

fn known(app: &App, role: &RoleId) -> Result<(), String> {
    if app.org().roles.iter().any(|r| &r.id == role) {
        Ok(())
    } else {
        Err(format!("没有岗位 {role}"))
    }
}

pub async fn view(app: &App, role: &RoleId) -> Result<MemoryView, String> {
    known(app, role)?;
    let store = app.runtime.store();
    let e = |e: ip_store::StoreError| e.to_string();
    let text = |entry: memory::Entry| MemoryText {
        text: entry.text,
        updated_ms: entry.updated_ms,
    };
    let threads = store.thread_memories_with_usage(role, THREADS_LIMIT).await.map_err(e)?;
    let corrections = store.memory_notes(role, 0, CORRECTIONS_LIMIT).await.map_err(e)?;
    Ok(MemoryView {
        role_id: role.clone(),
        enabled: app.runtime.memory_enabled_for(role),
        notes: memory::get(store, Layer::Notes, role).await.map_err(e)?.map(text),
        summary: memory::get(store, Layer::Summary, role).await.map_err(e)?.map(text),
        summary_from_threads: memory::summary_inputs(store, role).await.map_err(e)?,
        threads: threads
            .into_iter()
            .map(|(m, usage_count, last_usage_ms)| ThreadMemoryView {
                thread_id: m.thread_id,
                title: m.slug,
                summary: m.summary.unwrap_or_default(),
                generated_ms: m.generated_ms,
                usage_count,
                last_usage_ms,
            })
            .collect(),
        corrections: corrections
            .into_iter()
            .map(|n| CorrectionView {
                note_id: n.id,
                text: n.text,
                created_ms: n.created_ms,
                thread_id: n.thread_id,
                applied_ms: n.applied_ms,
            })
            .collect(),
    })
}

pub async fn save_text(app: &App, role: &RoleId, layer: MemoryLayer, text: &str) -> Result<MemoryView, String> {
    known(app, role)?;
    if text.chars().count() > TEXT_MAX_CHARS {
        return Err(format!("太长了，最多 {TEXT_MAX_CHARS} 字"));
    }
    let store = app.runtime.store();
    let guard = app.runtime.memory_lock().await;
    match layer {
        MemoryLayer::Notes => memory::put(store, Layer::Notes, role, text, now_ms()).await,
        MemoryLayer::Summary => memory::edit_summary(store, role, text, now_ms()).await,
    }
    .map_err(|e| e.to_string())?;
    drop(guard);
    view(app, role).await
}

pub async fn forget_thread(app: &App, role: &RoleId, thread: &ThreadId) -> Result<MemoryView, String> {
    known(app, role)?;
    let _guard = app.runtime.memory_lock().await;
    app.runtime
        .store()
        .forget_thread_memories(Some(role), Some(thread))
        .await
        .map_err(|e| e.to_string())?;
    view(app, role).await
}

pub async fn delete_note(app: &App, role: &RoleId, note_id: i64) -> Result<MemoryView, String> {
    known(app, role)?;
    let _guard = app.runtime.memory_lock().await;
    app.runtime
        .store()
        .delete_memory_notes(role, Some(note_id))
        .await
        .map_err(|e| e.to_string())?;
    view(app, role).await
}

pub async fn clear(app: &App, role: &RoleId) -> Result<MemoryView, String> {
    known(app, role)?;
    let _guard = app.runtime.memory_lock().await;
    memory::clear_role(app.runtime.store(), role, now_ms())
        .await
        .map_err(|e| e.to_string())?;
    view(app, role).await
}

pub async fn threads_off(app: &App) -> Result<Vec<ThreadId>, String> {
    memory::threads_off(app.runtime.store())
        .await
        .map_err(|e| e.to_string())
}

pub async fn set_thread(app: &App, thread: &ThreadId, enabled: bool) -> Result<Vec<ThreadId>, String> {
    let guard = app.runtime.memory_lock().await;
    memory::set_thread_off(app.runtime.store(), thread, !enabled)
        .await
        .map_err(|e| e.to_string())?;
    drop(guard);
    threads_off(app).await
}
