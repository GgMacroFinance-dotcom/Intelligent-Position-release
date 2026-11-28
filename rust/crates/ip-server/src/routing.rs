//! 一条消息的完整路由：先规则，再（配置了的话）语义匹配，最后兜底。

use std::time::Duration;

use ip_models::{ChatMessage, ChatRequest, StreamEvent};
use ip_router::semantic::{self, Guess};
use ip_router::{Decision, RouteInput, SemanticSpec};

use crate::App;

/// 语义匹配最多等这么久，超时就走兜底，不让一条消息卡住。
const SEMANTIC_TIMEOUT: Duration = Duration::from_secs(20);

pub async fn decide(app: &App, input: RouteInput<'_>) -> Decision {
    let router = app.router();
    if let Some(d) = router.match_rules(input) {
        return d;
    }
    let Some(spec) = &router.spec().semantic else {
        return router.fallback("没有规则命中", None);
    };
    match guess(app, spec, input.text).await {
        Ok(g) => router.after_guess(&g),
        Err(e) => {
            tracing::warn!(error = %e, "语义匹配失败，走兜底");
            router.fallback(&format!("没有规则命中，语义匹配失败（{e}）"), None)
        }
    }
}

async fn guess(app: &App, spec: &SemanticSpec, text: &str) -> Result<Guess, String> {
    let (client, model) = app.runtime.models().resolve(&spec.model).map_err(|e| e.to_string())?;
    let (system, user) = semantic::prompt(&app.org(), text);
    let mut request = ChatRequest::new(
        model,
        vec![
            ChatMessage::System { content: system },
            ChatMessage::User { content: user },
        ],
    );
    request.max_tokens = Some(200);
    request.temperature = Some(0.0);
    let mut events = client.stream(request);
    let collect = async {
        let mut reply = String::new();
        let mut usage = None;
        while let Some(event) = events.recv().await {
            match event.map_err(|e| e.to_string())? {
                StreamEvent::TextDelta(t) => reply.push_str(&t),
                StreamEvent::Done { usage: u, .. } => {
                    usage = u;
                    break;
                }
                StreamEvent::ToolCall(_) => {}
            }
        }
        Ok::<_, String>((reply, usage))
    };
    let (reply, usage) = tokio::time::timeout(SEMANTIC_TIMEOUT, collect)
        .await
        .map_err(|_| format!("{} 秒内没有回答", SEMANTIC_TIMEOUT.as_secs()))??;
    if let Some(usage) = usage
        && let Err(e) = app.runtime.charge_routing(&spec.model, &usage).await
    {
        tracing::warn!(error = %e, "语义分派的费用没记上");
    }
    semantic::parse(&reply, &app.org())
}
