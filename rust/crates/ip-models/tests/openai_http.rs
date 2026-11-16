//! 用本地假服务验证 HTTP 层：请求格式、流式解析、429 重试、错误状态。

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::{Json, Router, extract::State, http::StatusCode, response::IntoResponse, routing::post};
use ip_models::{
    ChatMessage, ChatRequest, FinishReason, ModelClient, ModelError, OpenAiCompatible, OpenAiConfig, StreamEvent,
    ToolSpec,
};

#[derive(Clone, Default)]
struct Mock {
    calls: Arc<AtomicUsize>,
    fail_first: usize,
    status: u16,
    seen: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
}

async fn completions(State(mock): State<Mock>, Json(body): Json<serde_json::Value>) -> impl IntoResponse {
    let n = mock.calls.fetch_add(1, Ordering::SeqCst);
    mock.seen.lock().unwrap().push(body);
    if n < mock.fail_first {
        return (
            StatusCode::from_u16(mock.status).unwrap(),
            [("retry-after", "0")],
            "busy".to_owned(),
        )
            .into_response();
    }
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"！\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    ([("content-type", "text/event-stream")], sse).into_response()
}

async fn serve(mock: Mock) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/v1/chat/completions", post(completions))
        .with_state(mock);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}/v1")
}

fn client(base: String) -> OpenAiCompatible {
    let mut config = OpenAiConfig::new(base);
    config.api_key = Some("test-key".into());
    config.connect_timeout = Duration::from_secs(5);
    OpenAiCompatible::new(config).unwrap()
}

async fn collect(client: &OpenAiCompatible, request: ChatRequest) -> Vec<Result<StreamEvent, ModelError>> {
    let mut stream = client.stream(request);
    let mut events = Vec::new();
    while let Some(event) = stream.recv().await {
        events.push(event);
    }
    events
}

fn request() -> ChatRequest {
    let mut request = ChatRequest::new("deepseek-chat", vec![ChatMessage::User { content: "hi".into() }]);
    request.tools.push(ToolSpec {
        name: "shell".into(),
        description: "run a command".into(),
        parameters: serde_json::json!({"type": "object"}),
    });
    request
}

#[tokio::test]
async fn streams_text_and_sends_openai_shaped_body() {
    let mock = Mock::default();
    let events = collect(&client(serve(mock.clone()).await), request()).await;
    let events: Vec<_> = events.into_iter().map(Result::unwrap).collect();
    assert_eq!(
        events,
        vec![
            StreamEvent::TextDelta("你好".into()),
            StreamEvent::TextDelta("！".into()),
            StreamEvent::Done {
                finish: FinishReason::Stop,
                usage: None
            },
        ]
    );
    let body = mock.seen.lock().unwrap()[0].clone();
    assert_eq!(body["model"], "deepseek-chat");
    assert_eq!(body["stream"], true);
    assert_eq!(
        body["messages"][0],
        serde_json::json!({"role": "user", "content": "hi"})
    );
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "shell");
}

#[tokio::test]
async fn retries_rate_limits_then_succeeds() {
    let mock = Mock {
        fail_first: 2,
        status: 429,
        ..Mock::default()
    };
    let events = collect(&client(serve(mock.clone()).await), request()).await;
    assert!(events.iter().all(Result::is_ok));
    assert_eq!(mock.calls.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn does_not_retry_client_errors() {
    let mock = Mock {
        fail_first: 10,
        status: 401,
        ..Mock::default()
    };
    let events = collect(&client(serve(mock.clone()).await), request()).await;
    assert!(matches!(
        events.as_slice(),
        [Err(ModelError::Status { status: 401, .. })]
    ));
    assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
}
