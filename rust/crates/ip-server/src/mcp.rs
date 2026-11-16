//! `/mcp`：外部引擎调组织工具的入口。
//!
//! 每个请求都要带运行时签发的本轮令牌（`Authorization: Bearer <令牌>`），这里先在 HTTP 层
//! 挡掉没有令牌或令牌已过期的请求（401），再交给 rmcp 处理协议。控制台的访问令牌在这里无效。

use std::sync::Arc;

use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};

use crate::App;

pub fn routes(app: &App) -> Router {
    let tools: Arc<dyn ip_mcp::server::TurnTools> = Arc::new(app.runtime().clone());
    Router::new()
        .route_service("/mcp", ip_mcp::server::http_service(tools))
        .layer(middleware::from_fn_with_state(app.clone(), require_turn_token))
}

async fn require_turn_token(State(app): State<App>, request: Request, next: Next) -> Response {
    match ip_mcp::server::bearer(request.headers()) {
        Some(token) if app.runtime().turn_token_valid(token) => next.run(request).await,
        _ => (StatusCode::UNAUTHORIZED, "需要有效的本轮令牌").into_response(),
    }
}
