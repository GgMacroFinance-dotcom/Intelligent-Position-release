//! 模型 HTTP 连接的取消、结束和超时。
//!
//! 用裸 TCP 写的假服务端，按连接顺序决定怎么回：可以只回一半正文后挂着、一直不回响应头、
//! 发完结束事件后不关连接。每条连接的“到达”“被客户端断开”都报给测试，
//! 这样断言的是连接真的释放了、下一个请求真的进来了，而不只是消费者函数返回了。

use std::time::Duration;

use ip_models::{
    ChatMessage, ChatRequest, ModelClient, ModelError, OpenAiCompatible, OpenAiConfig, StreamEvent, WireApi,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;

const CHAT_PARTIAL: &str = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n";
const CHAT_FULL: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"},\"finish_reason\":\"stop\"}]}\n\n",
    "data: [DONE]\n\n",
);
const RESPONSES_FULL: &str = concat!(
    "event: response.output_text.delta\n",
    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}\n\n",
    "event: response.completed\n",
    "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\n",
);

/// 第几条连接怎么回。
#[derive(Clone, Copy)]
enum Script {
    /// 回 200 和这段正文后关连接。
    Full(&'static str),
    /// 回 200 和这段正文后挂着不关，直到客户端断开。
    Hang(&'static str),
    /// 一直不回响应头。
    Silent,
    /// 回 401 和正文，但不关闭连接。
    ErrorHang(&'static str, usize),
    /// 先发文本，再等待超过响应头时限的时间后结束。
    SlowStream,
    /// 回 503，要求 30 秒后重试。
    Busy,
}

#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Arrived(usize),
    Closed(usize),
}

/// 起假服务端。`scripts` 用完后的连接都按最后一项回。
async fn serve(scripts: Vec<Script>) -> (String, mpsc::UnboundedReceiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        // 连接序号从 0 往上数；写成 loop 加计数器，新版 clippy 不许无上界的 `for n in 0..`。
        let mut n = 0;
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let script = scripts[n.min(scripts.len() - 1)];
            let tx = tx.clone();
            tokio::spawn(connection(n, socket, script, tx));
            n += 1;
        }
    });
    (format!("http://{addr}/v1"), rx)
}

async fn connection(n: usize, mut socket: TcpStream, script: Script, tx: mpsc::UnboundedSender<Seen>) {
    read_request(&mut socket).await;
    let _ = tx.send(Seen::Arrived(n));
    let head = |status: &str| {
        format!("HTTP/1.1 {status}\r\ncontent-type: text/event-stream\r\nconnection: close\r\nretry-after: 30\r\n\r\n")
    };
    let hang = match script {
        Script::Full(body) => {
            let _ = socket.write_all(format!("{}{body}", head("200 OK")).as_bytes()).await;
            false
        }
        Script::Hang(body) => {
            let _ = socket.write_all(format!("{}{body}", head("200 OK")).as_bytes()).await;
            true
        }
        Script::Silent => true,
        Script::ErrorHang(piece, count) => {
            let _ = socket
                .write_all(format!("{}{}", head("401 Unauthorized"), piece.repeat(count)).as_bytes())
                .await;
            true
        }
        Script::SlowStream => {
            let _ = socket
                .write_all(format!("{}{CHAT_PARTIAL}", head("200 OK")).as_bytes())
                .await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = socket.write_all(b"data: [DONE]\n\n").await;
            false
        }
        Script::Busy => {
            let body = "busy";
            let response = format!(
                "HTTP/1.1 503 Service Unavailable\r\ncontent-length: {}\r\nconnection: close\r\nretry-after: 30\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            false
        }
    };
    if hang {
        // 读到 EOF 或出错就是客户端断开了。
        let mut buf = [0u8; 1024];
        while matches!(socket.read(&mut buf).await, Ok(n) if n > 0) {}
        let _ = tx.send(Seen::Closed(n));
    }
}

/// 读完请求头和 `content-length` 指定的正文。
async fn read_request(socket: &mut TcpStream) {
    let mut data = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = socket.read(&mut buf).await.unwrap_or(0);
        if n == 0 {
            return;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&data[..end]).to_ascii_lowercase();
            let length = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            while data.len() < end + 4 + length {
                let n = socket.read(&mut buf).await.unwrap_or(0);
                if n == 0 {
                    return;
                }
                data.extend_from_slice(&buf[..n]);
            }
            return;
        }
    }
}

fn client(base: String, api: WireApi) -> OpenAiCompatible {
    let mut config = OpenAiConfig::new(base);
    config.max_concurrency = 1;
    config.api = api;
    OpenAiCompatible::new(config).unwrap()
}

fn request() -> ChatRequest {
    ChatRequest::new("m", vec![ChatMessage::User { content: "hi".into() }])
}

async fn next_seen(rx: &mut mpsc::UnboundedReceiver<Seen>) -> Seen {
    timeout(Duration::from_secs(5), rx.recv())
        .await
        .expect("假服务端 5 秒内没有动静")
        .unwrap()
}

/// 名额只有 1 个：再发一个请求，它能到达服务端并正常结束，说明前一个已经交还了名额。
async fn next_request_gets_through(model: &OpenAiCompatible, rx: &mut mpsc::UnboundedReceiver<Seen>, n: usize) {
    let mut stream = model.stream(request());
    assert_eq!(next_seen(rx).await, Seen::Arrived(n));
    let mut done = false;
    while let Some(event) = timeout(Duration::from_secs(5), stream.recv()).await.unwrap() {
        done |= matches!(event.unwrap(), StreamEvent::Done { .. });
    }
    assert!(done);
}

#[tokio::test]
async fn dropping_stream_cancels_stalled_body_and_releases_permit() {
    let (base, mut rx) = serve(vec![Script::Hang(CHAT_PARTIAL), Script::Full(CHAT_FULL)]).await;
    let model = client(base, WireApi::ChatCompletions);
    let mut stream = model.stream(request());
    assert_eq!(next_seen(&mut rx).await, Seen::Arrived(0));
    let first = timeout(Duration::from_secs(5), stream.recv()).await.unwrap().unwrap();
    assert_eq!(first.unwrap(), StreamEvent::TextDelta("你好".into()));
    // 服务端此后一直不说话；调用方不要了。
    drop(stream);
    assert_eq!(next_seen(&mut rx).await, Seen::Closed(0));
    next_request_gets_through(&model, &mut rx, 1).await;
}

#[tokio::test]
async fn dropping_stream_cancels_wait_for_headers() {
    let (base, mut rx) = serve(vec![Script::Silent, Script::Full(CHAT_FULL)]).await;
    let model = client(base, WireApi::ChatCompletions);
    let stream = model.stream(request());
    assert_eq!(next_seen(&mut rx).await, Seen::Arrived(0));
    drop(stream);
    assert_eq!(next_seen(&mut rx).await, Seen::Closed(0));
    next_request_gets_through(&model, &mut rx, 1).await;
}

#[tokio::test]
async fn dropping_unpolled_stream_does_not_send_request() {
    let (base, mut rx) = serve(vec![Script::Full(CHAT_FULL)]).await;
    let model = client(base, WireApi::ChatCompletions);
    drop(model.stream(request()));
    // 后台任务看到接收端已关，不发请求：服务端收到的第一条连接是下面这个请求。
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(rx.try_recv().is_err(), "丢掉的请求不该发出去");
    next_request_gets_through(&model, &mut rx, 0).await;
}

#[tokio::test]
async fn dropping_queued_stream_never_sends_request() {
    let (base, mut rx) = serve(vec![Script::Hang(CHAT_PARTIAL), Script::Full(CHAT_FULL)]).await;
    let model = client(base, WireApi::ChatCompletions);
    let first = model.stream(request());
    assert_eq!(next_seen(&mut rx).await, Seen::Arrived(0));
    // 第二个请求在排队等名额时被丢掉。
    let queued = model.stream(request());
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(queued);
    drop(first);
    assert_eq!(next_seen(&mut rx).await, Seen::Closed(0));
    // 排队的那个没有发出去：下一条连接就是第三个请求。
    next_request_gets_through(&model, &mut rx, 1).await;
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn dropping_stream_cancels_retry_backoff() {
    let (base, mut rx) = serve(vec![Script::Busy, Script::Full(CHAT_FULL)]).await;
    let model = client(base, WireApi::ChatCompletions);
    let stream = model.stream(request());
    assert_eq!(next_seen(&mut rx).await, Seen::Arrived(0));
    // 503 之后要等 30 秒重试；这时丢掉，不该再重试，也不该占着名额等满 30 秒。
    tokio::time::sleep(Duration::from_millis(200)).await;
    drop(stream);
    next_request_gets_through(&model, &mut rx, 1).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(rx.try_recv().is_err(), "取消后不该再重试");
}

#[tokio::test]
async fn done_closes_stream_without_waiting_for_http_eof() {
    for (api, body) in [
        (WireApi::ChatCompletions, CHAT_FULL),
        (WireApi::Responses, RESPONSES_FULL),
    ] {
        let (base, mut rx) = serve(vec![Script::Hang(body), Script::Full(body)]).await;
        let model = client(base, api);
        let mut stream = model.stream(request());
        assert_eq!(next_seen(&mut rx).await, Seen::Arrived(0));
        // 发完结束事件后服务端挂着连接：流照样在 Done 之后结束，连接被断开，名额交还。
        let mut events = Vec::new();
        while let Some(event) = timeout(Duration::from_secs(5), stream.recv())
            .await
            .unwrap_or_else(|_| panic!("{api:?}：收到 Done 后流没有结束"))
        {
            events.push(event.unwrap());
        }
        assert!(
            matches!(events.last(), Some(StreamEvent::Done { .. })),
            "{api:?}: {events:?}"
        );
        assert_eq!(next_seen(&mut rx).await, Seen::Closed(0), "{api:?}");
        next_request_gets_through(&model, &mut rx, 1).await;
    }
}

fn timeout_client(base: String, connect_timeout: Duration) -> OpenAiCompatible {
    let mut config = OpenAiConfig::new(base);
    config.connect_timeout = connect_timeout;
    config.max_retries = 0;
    OpenAiCompatible::new(config).unwrap()
}

#[tokio::test]
async fn times_out_when_server_never_sends_headers() {
    let (base, mut rx) = serve(vec![Script::Silent]).await;
    let model = timeout_client(base, Duration::from_millis(250));
    let mut stream = model.stream(request());
    assert_eq!(next_seen(&mut rx).await, Seen::Arrived(0));
    let event = timeout(Duration::from_secs(3), stream.recv())
        .await
        .expect("等待响应头没有超时");
    assert!(matches!(event, Some(Err(ModelError::Http(reason))) if reason.contains("响应头超时")));
    assert_eq!(next_seen(&mut rx).await, Seen::Closed(0));
}

#[tokio::test]
async fn stalled_error_body_preserves_http_status_and_partial_text() {
    let (base, mut rx) = serve(vec![Script::ErrorHang("denied", 1)]).await;
    let model = timeout_client(base, Duration::from_millis(250));
    let mut stream = model.stream(request());
    assert_eq!(next_seen(&mut rx).await, Seen::Arrived(0));
    let event = timeout(Duration::from_secs(3), stream.recv())
        .await
        .expect("读取错误正文没有超时");
    assert!(matches!(event, Some(Err(ModelError::Status { status: 401, body })) if body == "denied"));
    assert_eq!(next_seen(&mut rx).await, Seen::Closed(0));
}

#[tokio::test]
async fn large_error_body_is_returned_without_waiting_for_eof() {
    let (base, mut rx) = serve(vec![Script::ErrorHang("错", 5_000)]).await;
    let model = timeout_client(base, Duration::from_secs(10));
    let mut stream = model.stream(request());
    assert_eq!(next_seen(&mut rx).await, Seen::Arrived(0));
    let event = timeout(Duration::from_secs(3), stream.recv())
        .await
        .expect("错误正文超过读取上限后仍在等待 EOF");
    assert!(matches!(event, Some(Err(ModelError::Status { status: 401, body })) if body == "错".repeat(2_000)));
    assert_eq!(next_seen(&mut rx).await, Seen::Closed(0));
}

#[tokio::test]
async fn header_timeout_does_not_limit_normal_stream_duration() {
    let (base, _rx) = serve(vec![Script::SlowStream]).await;
    let model = timeout_client(base, Duration::from_millis(250));
    let mut stream = model.stream(request());
    assert_eq!(
        timeout(Duration::from_secs(3), stream.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        StreamEvent::TextDelta("你好".into())
    );
    assert!(matches!(
        timeout(Duration::from_secs(3), stream.recv()).await.unwrap(),
        Some(Ok(StreamEvent::Done { .. }))
    ));
}
