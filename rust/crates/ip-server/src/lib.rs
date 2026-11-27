//! 对外接口：`/healthz` 探活，`/ws` 收发消息和推送运行时事件，`/` 是 Web 控制台，
//! `/mcp` 是给外部引擎用的 MCP 服务端（只认本轮令牌，见 `docs/ip/m2-spec.md` D-17）。
//!
//! 不带配置启动时只有 `/healthz`，供部署探活和 `ip bench startup` 测启动时间。
//! 登录与多租户在 M1。

mod app;
pub mod channels;
pub mod check;
pub mod config;
mod console;
pub mod devices;
pub mod events;
mod mcp;
mod memory_admin;
pub mod notify;
mod origin;
pub mod protocol;
mod reports;
mod routing;
mod rules;
pub mod schedule;
mod star_map;
pub mod webpush;
mod work;
mod ws;

use std::net::SocketAddr;

use anyhow::bail;
use axum::{Json, Router, routing::get};
use serde::Serialize;
use tokio::net::TcpListener;

pub use app::App;
pub use config::Config;

/// 启动完成后打印到标准输出的那一行的前缀，`ip bench` 靠它拿到端口。
pub const LISTENING_PREFIX: &str = "listening ";

#[derive(Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
}

/// 服务的全部路由。没有 `App` 时只有健康检查。
pub fn router(app: Option<App>) -> Router {
    let base = Router::new().route("/healthz", get(healthz));
    match app {
        Some(app) => base
            .route("/", get(console::index))
            .route("/assets/{name}", get(console::asset))
            .route("/manifest.webmanifest", get(console::manifest))
            .route("/sw.js", get(console::service_worker))
            .merge(mcp::routes(&app))
            .merge(
                Router::new()
                    .route("/ws", get(ws::handler))
                    .route("/pair", axum::routing::post(ws::pair))
                    .with_state(app),
            ),
        None => base,
    }
}

async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// 绑定地址并一直服务到收到 Ctrl-C 或 SIGTERM。
///
/// 端口给 0 时由系统分配；实际地址以 `listening <addr>` 打印到标准输出。
/// 监听非本机地址时必须配置访问令牌，否则拒绝启动。
pub async fn serve(addr: SocketAddr, app: Option<App>) -> anyhow::Result<()> {
    if let Some(app) = &app
        && !addr.ip().is_loopback()
        && !app.requires_token()
    {
        bail!("监听 {addr} 会暴露给外部网络，请在配置里设置 auth_token_env");
    }
    let app_for_shutdown = app.clone();
    let app_after = app.clone();
    let listener = TcpListener::bind(addr).await?;
    let local = listener.local_addr()?;
    if let Some(app) = &app {
        app.runtime().set_mcp_url(mcp_url(local));
    }
    println!("{LISTENING_PREFIX}{local}");
    tracing::info!(%local, "ip-server listening");
    if let Some(app) = &app {
        app.start_schedules();
        app.start_memory();
        app.start_notifications();
    }
    axum::serve(listener, router(app))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            if let Some(app) = &app_for_shutdown {
                app.close();
            }
        })
        .await?;
    if let Some(app) = app_after {
        // 等正在进行的一轮做完再退出，避免留下没有结果的工具调用。
        app.shutdown().await;
    }
    Ok(())
}

/// 外部引擎回调组织工具用的地址。引擎和本服务在同一台机器上，总是走本机回环地址
/// （`/mcp` 也只接受本机的 Host）。
fn mcp_url(local: SocketAddr) -> String {
    match local {
        SocketAddr::V4(a) => format!("http://127.0.0.1:{}/mcp", a.port()),
        SocketAddr::V6(a) => format!("http://[::1]:{}/mcp", a.port()),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

#[cfg(test)]
mod channel_tests;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod slack_tests;
#[cfg(test)]
mod ws_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn healthz_answers_ok() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router(None)).await.unwrap() });

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(b"GET /healthz HTTP/1.1\r\nHost: test\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();

        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains(r#""status":"ok""#), "{response}");
    }
}
