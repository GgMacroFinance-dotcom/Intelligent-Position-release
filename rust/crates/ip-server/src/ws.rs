//! WebSocket 接口 `/ws`。
//!
//! 客户端发请求：`{"id": 1, "type": "send", "agent_id": "macro-1", "text": "..."}`，
//! 服务回 `{"type": "reply", "id": 1, "ok": true, "data": ...}`。
//! 运行时事件（流式文字、新消息、一轮结束）以 `{"type": "event", "event": {...}}` 推送给所有连接。

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::App;
use crate::memory_admin;
use crate::protocol::{Envelope, Request, ServerFrame};
use crate::work;
use ip_router::RouteInput;

/// 列表类请求的条数上限。
const MAX_LIMIT: usize = 500;

#[derive(Deserialize)]
pub struct AuthQuery {
    token: Option<String>,
}

pub async fn handler(
    State(app): State<App>,
    Query(q): Query<AuthQuery>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Response {
    // 先查来源再查令牌：别的网页借浏览器发来的握手，不管带没带令牌都不升级。
    if let Err(reason) = crate::origin::check(&headers, app.token.is_some(), &app.allowed_origins) {
        tracing::warn!(
            origin = ?headers.get(header::ORIGIN),
            host = ?headers.get(header::HOST),
            "拒绝 /ws 连接：{reason}"
        );
        return (StatusCode::FORBIDDEN, reason).into_response();
    }
    if let Some(expected) = &app.token {
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        let given = bearer.or(q.token.as_deref()).unwrap_or("");
        if !constant_time_eq(given.as_bytes(), expected.as_bytes()) {
            return (StatusCode::UNAUTHORIZED, "需要访问令牌").into_response();
        }
    }
    upgrade.on_upgrade(move |socket| session(app, socket))
}

async fn session(app: App, mut socket: WebSocket) {
    let mut events = app.runtime.subscribe();
    let mut closing = app.closing.subscribe();
    if *closing.borrow() {
        return;
    }
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
    loop {
        tokio::select! {
            _ = closing.changed() => {
                let _ = socket.send(WsMessage::Close(None)).await;
                break;
            }
            incoming = socket.recv() => {
                let Some(Ok(msg)) = incoming else { break };
                match msg {
                    WsMessage::Text(text) => {
                        // 发消息可能要等收件箱，放到独立任务里，不挡事件推送。
                        let app = app.clone();
                        let out = out_tx.clone();
                        let text = text.to_string();
                        tokio::spawn(async move {
                            let reply = handle(&app, &text).await;
                            let _ = out.send(reply.to_json()).await;
                        });
                    }
                    WsMessage::Close(_) => break,
                    _ => {}
                }
            }
            Some(reply) = out_rx.recv() => {
                if socket.send(WsMessage::Text(reply.into())).await.is_err() {
                    break;
                }
            }
            event = events.recv() => {
                let frame = match event {
                    Ok(event) => ServerFrame::Event { event: Box::new(event) },
                    Err(broadcast::error::RecvError::Lagged(skipped)) => ServerFrame::Lagged { skipped },
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if socket.send(WsMessage::Text(frame.to_json().into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

async fn handle(app: &App, text: &str) -> ServerFrame {
    let envelope: Envelope = match serde_json::from_str(text) {
        Ok(e) => e,
        Err(e) => {
            let id = serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| v.get("id").cloned());
            return ServerFrame::reply(id, Err(format!("请求格式错误：{e}")));
        }
    };
    ServerFrame::reply(envelope.id, dispatch(app, envelope.request).await)
}

async fn dispatch(app: &App, request: Request) -> Result<Value, String> {
    let rt = &app.runtime;
    let limit = |l: Option<usize>, default: usize| l.unwrap_or(default).clamp(1, MAX_LIMIT);
    let to_json = |v: serde_json::Result<Value>| v.map_err(|e| e.to_string());
    match request {
        Request::ListRoles => to_json(serde_json::to_value(&app.org.roles)),
        Request::ListAgents => to_json(serde_json::to_value(rt.agents())),
        Request::SpendToday => to_json(serde_json::to_value(rt.spend_today().await.map_err(|e| e.to_string())?)),
        Request::ListThreads { limit: l } => {
            let threads = rt
                .store()
                .recent_threads(limit(l, 50))
                .await
                .map_err(|e| e.to_string())?;
            to_json(serde_json::to_value(threads))
        }
        Request::History {
            thread_id,
            before_ms,
            limit: l,
        } => {
            let messages = rt
                .store()
                .thread_messages(&thread_id, before_ms, limit(l, 100))
                .await
                .map_err(|e| e.to_string())?;
            to_json(serde_json::to_value(messages))
        }
        Request::Search {
            query,
            thread_id,
            limit: l,
        } => {
            let hits = rt
                .store()
                .search(&query, thread_id.as_ref(), limit(l, 20))
                .await
                .map_err(|e| e.to_string())?;
            to_json(serde_json::to_value(hits))
        }
        Request::Route { text, author } => to_json(serde_json::to_value(
            crate::routing::decide(
                app,
                RouteInput {
                    text: &text,
                    author: author.as_deref(),
                    via: None,
                },
            )
            .await,
        )),
        Request::ListRules => to_json(serde_json::to_value(crate::protocol::RulesView {
            spec: app.router().spec().clone(),
            from_console: app.rules_from_console.load(std::sync::atomic::Ordering::Relaxed),
        })),
        Request::SaveRules { spec, accepted_draft } => to_json(serde_json::to_value(
            crate::rules::save(app, spec, accepted_draft).await?,
        )),
        Request::ListRuleDrafts => to_json(serde_json::to_value(crate::rules::drafts(app).await?)),
        Request::ListSchedules => to_json(serde_json::to_value(app.schedules.views())),
        Request::RunSchedule { schedule_id } => {
            to_json(serde_json::to_value(crate::schedule::run(app, &schedule_id).await?))
        }
        Request::DismissRuleDraft { draft_id } => {
            to_json(serde_json::to_value(crate::rules::dismiss(app, &draft_id).await?))
        }
        Request::Send {
            agent_id,
            thread_id,
            text,
            author,
        } => {
            let author = author.unwrap_or_else(|| "用户".into());
            to_json(serde_json::to_value(
                work::send(app, agent_id, thread_id, text, author).await?,
            ))
        }
        Request::ListTasks { open_only, limit: l } => {
            let tasks = rt
                .store()
                .tasks(open_only.unwrap_or(false), limit(l, 200))
                .await
                .map_err(|e| e.to_string())?;
            to_json(serde_json::to_value(tasks))
        }
        Request::CreateTask {
            title,
            description,
            role,
            assignee,
            thread_id,
            due_ms,
            author,
        } => {
            let new = ip_runtime::NewTask {
                title,
                description: description.unwrap_or_default(),
                role,
                assignee,
                // 人建任务时给的对话是“从哪儿派的活”，结果回报到那里；负责人另开对话做事。
                thread_id: None,
                origin_thread_id: thread_id,
                due_ms,
            };
            to_json(serde_json::to_value(
                work::create_task(app, new, author.unwrap_or_else(|| "用户".into())).await?,
            ))
        }
        Request::UpdateTask {
            task_id,
            status,
            assignee,
            result,
            author,
        } => {
            let update = ip_runtime::TaskUpdate {
                status,
                assignee,
                result,
            };
            let by = author.unwrap_or_else(|| "用户".into());
            to_json(serde_json::to_value(
                work::update_task(app, &task_id, update, by).await?,
            ))
        }
        Request::ListApprovals { pending_only, limit: l } => {
            let cards = rt
                .store()
                .approvals(pending_only.unwrap_or(false), limit(l, 100))
                .await
                .map_err(|e| e.to_string())?;
            to_json(serde_json::to_value(cards))
        }
        Request::GetMemory { role_id } => to_json(serde_json::to_value(memory_admin::view(app, &role_id).await?)),
        Request::SaveMemoryText { role_id, layer, text } => to_json(serde_json::to_value(
            memory_admin::save_text(app, &role_id, layer, &text).await?,
        )),
        Request::ForgetThreadMemory { role_id, thread_id } => to_json(serde_json::to_value(
            memory_admin::forget_thread(app, &role_id, &thread_id).await?,
        )),
        Request::DeleteMemoryNote { role_id, note_id } => to_json(serde_json::to_value(
            memory_admin::delete_note(app, &role_id, note_id).await?,
        )),
        Request::ClearMemory { role_id } => to_json(serde_json::to_value(memory_admin::clear(app, &role_id).await?)),
        Request::ListMemoryOffThreads => to_json(serde_json::to_value(memory_admin::threads_off(app).await?)),
        Request::SetThreadMemory { thread_id, enabled } => to_json(serde_json::to_value(
            memory_admin::set_thread(app, &thread_id, enabled).await?,
        )),
        Request::ResolveApproval {
            approval_id,
            choice,
            author,
        } => {
            let by = author.unwrap_or_else(|| "用户".into());
            to_json(serde_json::to_value(
                work::resolve_approval(app, &approval_id, choice, by).await?,
            ))
        }
    }
}

/// 比较令牌时不因第一个不同字节提前返回，避免按耗时猜令牌。
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
