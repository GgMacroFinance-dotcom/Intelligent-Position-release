//! Web 控制台：静态文件编译进可执行文件，部署时不需要额外的目录。
//!
//! 带 `/manifest.webmanifest` 和 `/sw.js`，浏览器能把控制台“安装”成桌面和手机上的应用（PWA，backlog F-2）。
//! Service Worker 必须从根路径提供，才能管到整个站点。

use axum::extract::Path;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

const INDEX: &str = include_str!("../../../web/index.html");
const MANIFEST: &str = include_str!("../../../web/manifest.webmanifest");
const SERVICE_WORKER: &str = include_str!("../../../web/src/sw.js");

/// (文件名, 内容, Content-Type)。
const ASSETS: &[(&str, &[u8], &str)] = &[
    (
        "app.js",
        include_bytes!("../../../web/src/app.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "client.js",
        include_bytes!("../../../web/src/client.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "dom.js",
        include_bytes!("../../../web/src/dom.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "markdown.js",
        include_bytes!("../../../web/src/markdown.js"),
        "text/javascript; charset=utf-8",
    ),
    (
        "style.css",
        include_bytes!("../../../web/src/style.css"),
        "text/css; charset=utf-8",
    ),
    (
        "icon.svg",
        include_bytes!("../../../web/icons/icon.svg"),
        "image/svg+xml",
    ),
    (
        "icon-192.png",
        include_bytes!("../../../web/icons/icon-192.png"),
        "image/png",
    ),
    (
        "icon-512.png",
        include_bytes!("../../../web/icons/icon-512.png"),
        "image/png",
    ),
    (
        "apple-touch-icon.png",
        include_bytes!("../../../web/icons/apple-touch-icon.png"),
        "image/png",
    ),
];

/// 只允许加载本站资源和连本站 WebSocket。
const CSP: &str = "default-src 'self'; connect-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; frame-ancestors 'none'";

pub async fn index() -> Response {
    respond(INDEX.as_bytes(), "text/html; charset=utf-8")
}

pub async fn manifest() -> Response {
    respond(MANIFEST.as_bytes(), "application/manifest+json")
}

pub async fn service_worker() -> Response {
    respond(SERVICE_WORKER.as_bytes(), "text/javascript; charset=utf-8")
}

pub async fn asset(Path(name): Path<String>) -> Response {
    match ASSETS.iter().find(|(n, _, _)| *n == name) {
        Some((_, body, ty)) => respond(body, ty),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn respond(body: &'static [u8], content_type: &'static str) -> Response {
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
