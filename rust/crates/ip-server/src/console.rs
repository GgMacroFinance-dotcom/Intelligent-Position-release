//! Web 控制台：静态文件编译进可执行文件，部署时不需要额外的目录。

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

const INDEX: &str = include_str!("../../../web/index.html");

/// (文件名, 内容, Content-Type)。
const ASSETS: &[(&str, &str, &str)] = &[
    (
        "app.js",
        include_str!("../../../web/src/app.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "client.js",
        include_str!("../../../web/src/client.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "dom.js",
        include_str!("../../../web/src/dom.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "style.css",
        include_str!("../../../web/src/style.css"),
        "text/css; charset=utf-8",
    ),
];

/// 只允许加载本站资源和连本站 WebSocket。
const CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'";

pub async fn index() -> Response {
    respond(INDEX, "text/html; charset=utf-8")
}

pub async fn asset(Path(name): Path<String>) -> Response {
    match ASSETS.iter().find(|(n, _, _)| *n == name) {
        Some((_, body, ty)) => respond(body, ty),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn respond(body: &'static str, content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CONTENT_SECURITY_POLICY, CSP),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        body,
    )
        .into_response()
}
