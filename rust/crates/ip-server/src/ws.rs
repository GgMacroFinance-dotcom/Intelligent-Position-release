//! WebSocket 接口 `/ws`。
//!
//! 客户端发请求：`{"id": 1, "type": "send", "agent_id": "macro-1", "text": "..."}`，
//! 服务回 `{"type": "reply", "id": 1, "ok": true, "data": ...}`。
//! 运行时事件（流式文字、新消息、一轮结束）以 `{"type": "event", "seq": 1, "event": {...}}` 推送给所有连接。
//! 连上后服务先发 `hello`；重连时带 `?since=<序号>&epoch=<epoch>` 补发断开期间的事件（`docs/ip/m2-spec.md` D-29）。

use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};

use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};

use crate::App;
use crate::devices::{Device, Scope};
use crate::events::{EventLog, Stamped};
use crate::memory_admin;
use crate::protocol::{Envelope, Request, ServerFrame};
use crate::work;
use ip_router::RouteInput;

/// 列表类请求的条数上限。
const MAX_LIMIT: usize = 500;

#[derive(Deserialize)]
pub struct AuthQuery {
    token: Option<String>,
    /// 重连补发：上次收到的最后一个事件序号（`docs/ip/m2-spec.md` D-29）。
    since: Option<u64>,
    /// 重连补发：上次连接 `hello` 里的 `epoch`。
    epoch: Option<String>,
}

/// 客户端要求从哪里接着收事件。
struct Resume {
    since: u64,
    epoch: String,
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
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let given = bearer.or(q.token.as_deref()).unwrap_or("");
    // 共用令牌，或者某台配对过的设备的令牌（backlog F-7）。没配共用令牌时只接受本机地址（上面查过），
    // 不带令牌也放行；带了设备令牌就按那台设备算，署名用设备名。
    let device = app.devices.authenticate(given);
    if let Some(expected) = &app.token
        && device.is_none()
        && !constant_time_eq(given.as_bytes(), expected.as_bytes())
    {
        return (StatusCode::UNAUTHORIZED, "需要访问令牌").into_response();
    }
    let caller = Caller { device };
    let resume = match (q.since, q.epoch) {
        (Some(since), Some(epoch)) => Some(Resume { since, epoch }),
        _ => None,
    };
    upgrade.on_upgrade(move |socket| session(app, socket, resume, caller))
}

/// 这个连接是谁：共用令牌（或本机不设令牌）时 `device` 为空。
#[derive(Clone)]
pub(crate) struct Caller {
    device: Option<Device>,
}

/// “日常”权限能用的请求（`docs/ip/apps-plan.md` 4.3，backlog F-8）。没列在这里的只给“管理”，
/// 以后新加的请求类型也一样：要给日常设备用，得在这里明确加上。
fn daily_allows(request: &Request) -> bool {
    matches!(
        request,
        // 对话
        Request::ListRoles
            | Request::ListAgents
            | Request::SpendToday
            | Request::ListThreads { .. }
            | Request::History { .. }
            | Request::Search { .. }
            | Request::Route { .. }
            | Request::Send { .. }
            // 只读：新对话默认要不要自动分派，看有没有路由规则
            | Request::ListRules
            // 只读：对话页顶上显示这个对话进不进岗位记忆
            | Request::ListMemoryOffThreads
            // 任务、审批和员工提问、报告
            | Request::ListTasks { .. }
            | Request::CreateTask { .. }
            | Request::UpdateTask { .. }
            | Request::ListApprovals { .. }
            | Request::ResolveApproval { .. }
            | Request::ListReports { .. }
            | Request::ReadReport { .. }
            | Request::WhoAmI
            // 自己的通知设置和推送订阅
            | Request::GetNotifySettings
            | Request::SetNotifySettings { .. }
            | Request::PushKey
            | Request::SubscribePush { .. }
            | Request::UnsubscribePush { .. }
            | Request::Presence { .. }
            // 星图上拖员工换轨道：日常使用，各设备共用一张表（D-40）
            | Request::ListOrbitPins
            | Request::SetOrbitPins { .. }
    )
}

async fn session(app: App, mut socket: WebSocket, resume: Option<Resume>, caller: Caller) {
    // 设备连接：记在线，吊销时立即断开。
    let _online = match &caller.device {
        Some(d) => Some(app.devices.connected(d, ip_core::now_ms()).await),
        None => None,
    };
    let mut revoked = app.devices.subscribe_revoked();
    app.start_notifications();
    let mut alerts = app.notifier.subscribe();
    let log = app.events.clone();
    log.start_pump(app.runtime.subscribe());
    // 先收听再取留着的事件，两步之间来的事件不会漏；重复的按序号跳过。
    let mut events = log.subscribe();
    let mut closing = app.closing.subscribe();
    if *closing.borrow() {
        return;
    }
    let replay = resume
        .filter(|r| r.epoch == log.epoch())
        .and_then(|r| log.since(r.since).ok().map(|frames| (r.since, frames)));
    let resumed = replay.is_some();
    let (start, frames) = replay.unwrap_or_else(|| (log.latest(), Vec::new()));
    let hello = ServerFrame::Hello {
        protocol: crate::events::PROTOCOL_VERSION,
        epoch: log.epoch().to_string(),
        seq: start,
        resumed,
    };
    if socket.send(WsMessage::Text(hello.to_json().into())).await.is_err() {
        return;
    }
    // 已经发给这个连接的最后一个序号。
    let mut last = start;
    if !send_frames(&mut socket, &frames, &mut last).await {
        return;
    }
    let (out_tx, mut out_rx) = mpsc::channel::<String>(256);
    loop {
        tokio::select! {
            _ = closing.changed() => {
                let _ = socket.send(WsMessage::Close(None)).await;
                break;
            }
            notice = revoked.recv(), if caller.device.is_some() => {
                let mine = caller.device.as_ref().map_or("", |d| d.id.as_str());
                let gone = match notice {
                    Ok(id) => id == mine,
                    // 吊销通知积压丢了：直接看这台设备还在不在。
                    Err(broadcast::error::RecvError::Lagged(_)) => !app.devices.list().iter().any(|i| i.id == mine),
                    // 发送端跟着 App 一直在，不会关；万一关了就断开，免得空转。
                    Err(broadcast::error::RecvError::Closed) => true,
                };
                if gone {
                    let _ = socket.send(WsMessage::Close(None)).await;
                    break;
                }
            }
            // 丢几条通知不要紧（Lagged）；发送端跟着 App 一直在，不会关。
            Ok(out) = alerts.recv() => {
                if let Some(frame) = alert_frame(&app, &caller, &out)
                    && socket.send(WsMessage::Text(frame.to_json().into())).await.is_err()
                {
                    break;
                }
            }
            incoming = socket.recv() => {
                let Some(Ok(msg)) = incoming else { break };
                match msg {
                    WsMessage::Text(text) => {
                        // 发消息可能要等收件箱，放到独立任务里，不挡事件推送。
                        let app = app.clone();
                        let out = out_tx.clone();
                        let text = text.to_string();
                        let caller = caller.clone();
                        tokio::spawn(async move {
                            let reply = handle(&app, &caller, &text).await;
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
                let sent = match event {
                    Ok(stamped) if stamped.seq <= last => true,
                    Ok(stamped) if stamped.seq == last + 1 => send_frames(&mut socket, &[stamped], &mut last).await,
                    // 序号不连续（转发任务丢过事件）或这个连接跟不上：先从留着的事件里补。
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => catch_up(&mut socket, &log, &mut last).await,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                if !sent {
                    break;
                }
            }
        }
    }
}

/// 按这个连接的通知设置筛一条通知；撤通知的一律发。
fn alert_frame(app: &App, caller: &Caller, out: &crate::notify::Outgoing) -> Option<ServerFrame> {
    match out {
        crate::notify::Outgoing::Dismiss(tag) => Some(ServerFrame::Dismiss { tag: tag.clone() }),
        crate::notify::Outgoing::Alert(alert) => {
            let who = caller.device.as_ref().map_or(crate::notify::SHARED, |d| d.id.as_str());
            let alert = app.notifier.settings(who).filter(alert, ip_core::now_ms())?;
            Some(ServerFrame::Alert { alert })
        }
    }
}

/// 设备连上的请求，署名换成设备名，忽略自填的 `author`（`docs/ip/apps-plan.md` 4.4）。
fn sign(mut request: Request, device: Option<&Device>) -> Request {
    let Some(device) = device else { return request };
    match &mut request {
        Request::Route { author, .. }
        | Request::Send { author, .. }
        | Request::CreateTask { author, .. }
        | Request::UpdateTask { author, .. }
        | Request::ResolveApproval { author, .. } => *author = Some(device.name.clone()),
        _ => {}
    }
    request
}

#[derive(Deserialize)]
pub struct PairBody {
    code: String,
}

/// `POST /pair`：用 6 位配对码换这台设备专属的令牌（backlog F-7）。来源检查和 `/ws` 一样。
pub async fn pair(State(app): State<App>, headers: HeaderMap, axum::Json(body): axum::Json<PairBody>) -> Response {
    if let Err(reason) = crate::origin::check(&headers, app.token.is_some(), &app.allowed_origins) {
        return (StatusCode::FORBIDDEN, reason).into_response();
    }
    match app.devices.claim(&body.code, ip_core::now_ms()).await {
        Ok(claimed) => axum::Json(claimed).into_response(),
        Err(e) => {
            let status = match e {
                crate::devices::ClaimError::Invalid => StatusCode::BAD_REQUEST,
                crate::devices::ClaimError::TooManyAttempts => StatusCode::TOO_MANY_REQUESTS,
                crate::devices::ClaimError::Store(_) => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, axum::Json(serde_json::json!({ "error": e.to_string() }))).into_response()
        }
    }
}

/// 按顺序发出序号比 `last` 新的帧，更新 `last`。连接断了返回 false。
async fn send_frames(socket: &mut WebSocket, frames: &[Stamped], last: &mut u64) -> bool {
    for stamped in frames {
        if stamped.seq <= *last {
            continue;
        }
        if socket.send(WsMessage::Text(stamped.frame.clone())).await.is_err() {
            return false;
        }
        *last = stamped.seq;
    }
    true
}

/// 从留着的事件里补上 `last` 之后的；补不上就发 `lagged`，从最新的序号接着推。连接断了返回 false。
async fn catch_up(socket: &mut WebSocket, log: &EventLog, last: &mut u64) -> bool {
    match log.since(*last) {
        Ok(frames) => send_frames(socket, &frames, last).await,
        Err(gap) => {
            *last = log.latest();
            let frame = ServerFrame::Lagged { skipped: gap.skipped };
            socket.send(WsMessage::Text(frame.to_json().into())).await.is_ok()
        }
    }
}

async fn handle(app: &App, caller: &Caller, text: &str) -> ServerFrame {
    let envelope: Envelope = match serde_json::from_str(text) {
        Ok(e) => e,
        Err(e) => {
            let id = serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| v.get("id").cloned());
            return ServerFrame::reply(id, Err(format!("请求格式错误：{e}")));
        }
    };
    ServerFrame::reply(envelope.id, dispatch(app, caller, envelope.request).await)
}

async fn dispatch(app: &App, caller: &Caller, request: Request) -> Result<Value, String> {
    // 设备的名字和权限按现在的算，连着时改过的下一个请求就生效。吊销和断开之间的请求一律拒绝。
    let device = match &caller.device {
        Some(d) => Some(app.devices.current(&d.id).ok_or("这台设备已吊销")?),
        None => None,
    };
    let scope = device.as_ref().map_or(Scope::Admin, |d| d.scope);
    if scope != Scope::Admin && !daily_allows(&request) {
        return Err("这台设备是“日常”权限，做不了这件事；要在“设备”页给它“管理”权限".into());
    }
    let request = sign(request, device.as_ref());
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
        Request::ListReports { agent_id, limit: l } => to_json(serde_json::to_value(
            crate::reports::list(app, agent_id.as_ref(), limit(l, 100)).await?,
        )),
        Request::WhoAmI => {
            let device = device.and_then(|d| app.devices.list().into_iter().find(|i| i.id == d.id));
            to_json(serde_json::to_value(crate::protocol::WhoAmI { device, scope }))
        }
        Request::ListDevices => to_json(serde_json::to_value(app.devices.list())),
        Request::CreatePairing { name, link_base } => {
            let pairing = app
                .devices
                .create_pairing(&name, link_base.as_deref(), ip_core::now_ms())
                .await?;
            to_json(serde_json::to_value(pairing))
        }
        Request::RenameDevice { device_id, name } => {
            to_json(serde_json::to_value(app.devices.rename(&device_id, &name).await?))
        }
        Request::SetDeviceScope { device_id, scope } => {
            to_json(serde_json::to_value(app.devices.set_scope(&device_id, scope).await?))
        }
        Request::RevokeDevice { device_id } => {
            let list = app.devices.revoke(&device_id).await?;
            if let Err(e) = app.notifier.forget(&device_id).await {
                tracing::warn!(device = %device_id, "删掉吊销设备的通知设置失败：{e}");
            }
            if let Err(e) = app.push.forget(&device_id).await {
                tracing::warn!(device = %device_id, "删掉吊销设备的推送订阅失败：{e}");
            }
            to_json(serde_json::to_value(list))
        }
        Request::GetNotifySettings => {
            let who = device.as_ref().map_or(crate::notify::SHARED, |d| d.id.as_str());
            to_json(serde_json::to_value(app.notifier.settings(who)))
        }
        Request::SetNotifySettings { settings } => {
            let who = device.as_ref().map_or(crate::notify::SHARED, |d| d.id.as_str());
            to_json(serde_json::to_value(app.notifier.save(who, settings).await?))
        }
        Request::PushKey => to_json(serde_json::to_value(app.push.public_key().await?)),
        Request::SubscribePush { endpoint, p256dh, auth } => {
            let who = device.as_ref().map_or(crate::notify::SHARED, |d| d.id.as_str());
            app.push
                .subscribe(who, &endpoint, &p256dh, &auth, ip_core::now_ms())
                .await?;
            Ok(Value::Null)
        }
        Request::UnsubscribePush { endpoint } => {
            let who = device.as_ref().map_or(crate::notify::SHARED, |d| d.id.as_str());
            app.push.unsubscribe(who, &endpoint).await?;
            Ok(Value::Null)
        }
        Request::Presence { active } => {
            let who = device.as_ref().map_or(crate::notify::SHARED, |d| d.id.as_str());
            app.push.presence(who, active, ip_core::now_ms());
            Ok(Value::Null)
        }
        Request::ListOrbitPins => to_json(serde_json::to_value(crate::star_map::list(app).await?)),
        Request::SetOrbitPins { changes } => to_json(serde_json::to_value(crate::star_map::set(app, changes).await?)),
        Request::ReadReport { agent_id, path } => {
            to_json(serde_json::to_value(crate::reports::read(app, &agent_id, &path).await?))
        }
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
