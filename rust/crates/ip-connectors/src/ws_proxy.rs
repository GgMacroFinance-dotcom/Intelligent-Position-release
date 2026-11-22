//! WebSocket 客户端连接，按环境变量走 HTTP 代理（`docs/ip/m2-spec.md` D-37，backlog E-12）。
//!
//! - 代理规则和 Web API 用的 reqwest 一样：都用 hyper-util 的 `Matcher::from_env`。`wss://` 看
//!   `HTTPS_PROXY`/`https_proxy`，`ws://` 看 `HTTP_PROXY`/`http_proxy`，没设时退到 `ALL_PROXY`/`all_proxy`；
//!   `NO_PROXY`/`no_proxy` 里的主机直连（规则同 curl：域名后缀、IP、CIDR、`*`）。
//! - 要走代理时，先对代理发 `CONNECT 主机:端口` 建隧道，再在隧道上做 TLS 和 WebSocket 握手。
//!   TLS 是端到端的，代理只看到主机名和端口。
//! - 代理地址里的用户名密码（`http://用户:密码@主机:端口`，可以百分号编码）用 Basic 认证发给代理。
//!   日志和报错里只写代理的主机和端口，不写凭据。
//! - TLS 信任：tokio-tungstenite（`rustls-tls-native-roots`）和 reqwest（rustls-platform-verifier）在 Linux 上
//!   都用 rustls-native-certs 读根证书，`SSL_CERT_FILE`、`SSL_CERT_DIR` 两边都认，容器里的自定义 CA 两边一样生效。
//! - 只支持 `http://` 代理。`https://`（和代理之间也加密）、`socks5://` 代理报错说明。

use std::time::Duration;

use hyper_util::client::proxy::matcher::{Intercept, Matcher};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::http::Uri;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub(crate) type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 连代理加等它回 `CONNECT` 最多等多久。
const PROXY_TIMEOUT: Duration = Duration::from_secs(30);
/// 代理回的响应头最多读这么多字节，超过就当代理有问题。
const MAX_RESPONSE_HEAD: usize = 16 * 1024;

/// 按代理设置建 WebSocket 连接。
pub(crate) struct WsConnector {
    matcher: Matcher,
}

impl WsConnector {
    /// 从环境变量读代理设置，和 reqwest 读的是同一组变量。
    pub(crate) fn from_env() -> Self {
        Self {
            matcher: Matcher::from_env(),
        }
    }

    /// 测试用：直接给出代理设置，不读环境变量。
    #[cfg(test)]
    pub(crate) fn with_proxy(http: &str, https: &str, no: &str) -> Self {
        Self {
            matcher: Matcher::builder().http(http).https(https).no(no).build(),
        }
    }

    /// 连上 `url`（`ws://` 或 `wss://`）。返回连接，以及走了哪个代理（只有主机和端口）。
    pub(crate) async fn connect(&self, url: &str) -> Result<(Ws, Option<String>), String> {
        let target: Uri = url.parse().map_err(|_| "WebSocket 地址不合法".to_string())?;
        let default_port = match target.scheme_str() {
            Some("wss") => 443,
            Some("ws") => 80,
            _ => return Err("WebSocket 地址要以 ws:// 或 wss:// 开头".into()),
        };
        let host = target.host().ok_or("WebSocket 地址里没有主机名")?;
        let port = target.port_u16().unwrap_or(default_port);

        let Some(proxy) = self.proxy_for(&target) else {
            let (ws, _) = tokio_tungstenite::connect_async(url).await.map_err(|e| e.to_string())?;
            return Ok((ws, None));
        };
        let shown = label(proxy.uri());
        let stream = tokio::time::timeout(PROXY_TIMEOUT, tunnel(&proxy, &shown, host, port))
            .await
            .map_err(|_| format!("经代理 {shown} 建隧道超时（{} 秒）", PROXY_TIMEOUT.as_secs()))??;
        let (ws, _) = tokio_tungstenite::client_async_tls_with_config(url, stream, None, None)
            .await
            .map_err(|e| format!("经代理 {shown} 握手失败：{e}"))?;
        Ok((ws, Some(shown)))
    }

    /// 这个地址要走的代理；直连时为 `None`。Matcher 只认 http/https，ws/wss 换成对应的协议再问。
    fn proxy_for(&self, target: &Uri) -> Option<Intercept> {
        let scheme = if target.scheme_str() == Some("wss") {
            "https"
        } else {
            "http"
        };
        let probe = Uri::builder()
            .scheme(scheme)
            .authority(target.authority()?.as_str())
            .path_and_query("/")
            .build()
            .ok()?;
        self.matcher.intercept(&probe)
    }
}

/// 代理的主机和端口，日志和报错里用。hyper-util 解析时已经把用户名密码从 uri 里拿掉了。
fn label(proxy: &Uri) -> String {
    let host = proxy.host().unwrap_or("?");
    match proxy.port_u16() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    }
}

/// 连上代理，发 `CONNECT`，代理回 2xx 后返回这条连接（之后就是到目标的隧道）。
async fn tunnel(proxy: &Intercept, shown: &str, host: &str, port: u16) -> Result<TcpStream, String> {
    let uri = proxy.uri();
    match uri.scheme_str() {
        Some("http") => {}
        Some(other) => {
            return Err(format!(
                "代理 {shown} 用的是 {other}://，Socket Mode 只支持 http:// 代理"
            ));
        }
        None => return Err(format!("代理 {shown} 的地址没有协议")),
    }
    let proxy_host = uri.host().ok_or_else(|| format!("代理 {shown} 的地址没有主机名"))?;
    let proxy_host = proxy_host.trim_start_matches('[').trim_end_matches(']');
    let proxy_port = uri.port_u16().unwrap_or(80);
    let mut stream = TcpStream::connect((proxy_host, proxy_port))
        .await
        .map_err(|e| format!("连不上代理 {shown}：{e}"))?;
    let _ = stream.set_nodelay(true);

    // http 的 Uri 给出的 IPv6 主机名已经带方括号。
    let authority = format!("{host}:{port}");
    let mut request = format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n").into_bytes();
    let with_auth = if let Some(auth) = proxy.basic_auth() {
        request.extend_from_slice(b"Proxy-Authorization: ");
        request.extend_from_slice(auth.as_bytes());
        request.extend_from_slice(b"\r\n");
        true
    } else {
        false
    };
    request.extend_from_slice(b"\r\n");
    stream
        .write_all(&request)
        .await
        .map_err(|e| format!("向代理 {shown} 发 CONNECT 失败：{e}"))?;

    let head = read_head(&mut stream)
        .await
        .map_err(|e| format!("读代理 {shown} 的回应失败：{e}"))?;
    let (code, reason) = status_of(&head).ok_or_else(|| format!("代理 {shown} 的回应不是 HTTP"))?;
    match code {
        200..=299 => Ok(stream),
        407 if with_auth => Err(format!(
            "代理 {shown} 不接受代理地址里的用户名密码（407）：检查 HTTPS_PROXY 等环境变量里的凭据"
        )),
        407 => Err(format!(
            "代理 {shown} 要求认证（407）：在 HTTPS_PROXY 等环境变量里写上用户名密码，如 http://用户名:密码@主机:端口"
        )),
        403 => Err(format!("代理 {shown} 不允许连 {authority}（403）：检查代理的放行名单")),
        _ => Err(format!(
            "代理 {shown} 拒绝建到 {authority} 的隧道：HTTP {code} {reason}"
        )),
    }
}

/// 逐字节读到响应头结束（空行）。不能多读：多读的字节属于隧道里的 TLS 或 WebSocket。
/// 一次连接只读一百来个字节，逐字节读的开销可以不计。
async fn read_head(stream: &mut TcpStream) -> Result<String, String> {
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_RESPONSE_HEAD {
            return Err(format!("响应头超过 {MAX_RESPONSE_HEAD} 字节"));
        }
        match stream.read(&mut byte).await {
            Ok(0) => return Err("代理没回完就断开了".into()),
            Ok(_) => head.push(byte[0]),
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// 状态行里的状态码和原因短语（短语只留可打印字符，最多 80 个）。
fn status_of(head: &str) -> Option<(u16, String)> {
    let line = head.lines().next()?;
    let mut parts = line.splitn(3, ' ');
    if !parts.next()?.starts_with("HTTP/") {
        return None;
    }
    let code = parts.next()?.parse().ok()?;
    let reason = parts
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(80)
        .collect();
    Some((code, reason))
}

/// 测试用的假代理：接受 `CONNECT`，按设定放行（转发到目标）、要求认证或拒绝。
#[cfg(test)]
pub(crate) mod fake_proxy {
    use std::sync::{Arc, Mutex};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    #[derive(Clone)]
    pub(crate) enum Mode {
        /// 放行所有 `CONNECT`。
        Allow,
        /// 只放行带这个 `Proxy-Authorization` 的，别的回 407。
        RequireAuth(String),
        /// 一律回 403。
        Forbid,
    }

    pub(crate) struct FakeProxy {
        pub(crate) port: u16,
        /// 收到的每个请求头，原样记下。
        requests: Arc<Mutex<Vec<String>>>,
    }

    impl FakeProxy {
        pub(crate) async fn start(mode: Mode) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let requests = Arc::new(Mutex::new(Vec::new()));
            let seen = requests.clone();
            tokio::spawn(async move {
                while let Ok((client, _)) = listener.accept().await {
                    tokio::spawn(serve(client, mode.clone(), seen.clone()));
                }
            });
            Self { port, requests }
        }

        pub(crate) fn url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        pub(crate) fn requests(&self) -> Vec<String> {
            self.requests.lock().unwrap().clone()
        }
    }

    async fn serve(mut client: TcpStream, mode: Mode, seen: Arc<Mutex<Vec<String>>>) {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match client.read(&mut byte).await {
                Ok(1) => head.push(byte[0]),
                _ => return,
            }
        }
        let head = String::from_utf8_lossy(&head).into_owned();
        seen.lock().unwrap().push(head.clone());
        let target = head
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("CONNECT "))
            .and_then(|l| l.split(' ').next())
            .unwrap_or("")
            .to_string();
        let auth = head
            .lines()
            .find_map(|l| l.strip_prefix("Proxy-Authorization: "))
            .map(str::to_string);
        let refuse = match &mode {
            Mode::Allow => None,
            Mode::RequireAuth(want) if auth.as_deref() == Some(want.as_str()) => None,
            Mode::RequireAuth(_) => Some(
                "HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"fake\"\r\nContent-Length: 0\r\n\r\n",
            ),
            Mode::Forbid => Some("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"),
        };
        if let Some(response) = refuse {
            let _ = client.write_all(response.as_bytes()).await;
            return;
        }
        let Ok(mut upstream) = TcpStream::connect(target.as_str()).await else {
            let _ = client.write_all(b"HTTP/1.1 502 Bad Gateway\r\n\r\n").await;
            return;
        };
        if client
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .await
            .is_err()
        {
            return;
        }
        let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
    }
}

#[cfg(test)]
mod tests {
    use futures_util::{SinkExt, StreamExt};
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::Message;

    use super::fake_proxy::{FakeProxy, Mode};
    use super::*;

    /// 假 WebSocket 服务器：连上先发 `hello`，之后原样回显。返回端口。
    async fn echo_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                        return;
                    };
                    let _ = ws.send(Message::Text("hello".into())).await;
                    while let Some(Ok(m)) = ws.next().await {
                        if m.is_text() && ws.send(m).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        port
    }

    async fn talk(ws: &mut Ws) {
        let first = ws.next().await.unwrap().unwrap();
        assert_eq!(first.into_text().unwrap().as_str(), "hello");
        ws.send(Message::Text("ping".into())).await.unwrap();
        let echo = ws.next().await.unwrap().unwrap();
        assert_eq!(echo.into_text().unwrap().as_str(), "ping");
    }

    #[tokio::test]
    async fn connects_through_proxy_tunnel() {
        let port = echo_server().await;
        let proxy = FakeProxy::start(Mode::Allow).await;
        let connector = WsConnector::with_proxy(&proxy.url(), &proxy.url(), "");
        let (mut ws, via) = connector.connect(&format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
        talk(&mut ws).await;
        assert_eq!(via.as_deref(), Some(format!("127.0.0.1:{}", proxy.port).as_str()));
        let requests = proxy.requests();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].starts_with(&format!(
                "CONNECT 127.0.0.1:{port} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
            )),
            "{}",
            requests[0]
        );
        assert!(!requests[0].contains("Proxy-Authorization"));
    }

    #[tokio::test]
    async fn sends_basic_auth_from_proxy_url() {
        let port = echo_server().await;
        // `alice:p@ss` 的 Base64；代理地址里的 `@` 写成 `%40`。
        let proxy = FakeProxy::start(Mode::RequireAuth("Basic YWxpY2U6cEBzcw==".into())).await;
        let url = format!("http://alice:p%40ss@127.0.0.1:{}", proxy.port);
        let connector = WsConnector::with_proxy(&url, &url, "");
        let (mut ws, via) = connector.connect(&format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
        talk(&mut ws).await;
        // 返回的代理说明里没有凭据。
        let via = via.unwrap();
        assert!(!via.contains("alice") && !via.contains("p%40ss"), "{via}");
    }

    #[tokio::test]
    async fn no_proxy_hosts_connect_directly() {
        let port = echo_server().await;
        let proxy = FakeProxy::start(Mode::Allow).await;
        let connector = WsConnector::with_proxy(&proxy.url(), &proxy.url(), "example.com, 127.0.0.1");
        let (mut ws, via) = connector.connect(&format!("ws://127.0.0.1:{port}/ws")).await.unwrap();
        talk(&mut ws).await;
        assert_eq!(via, None);
        assert!(proxy.requests().is_empty());
    }

    #[test]
    fn picks_proxy_by_scheme_and_no_proxy() {
        let c = WsConnector::with_proxy("http://plain:1", "http://secure:2", ".internal.example, 10.0.0.0/8");
        let via = |url: &str| c.proxy_for(&url.parse().unwrap()).map(|p| label(p.uri()));
        // wss 看 HTTPS_PROXY，ws 看 HTTP_PROXY。
        assert_eq!(
            via("wss://wss-primary.slack.com/link/?ticket=t").as_deref(),
            Some("secure:2")
        );
        assert_eq!(via("ws://wss-primary.slack.com/").as_deref(), Some("plain:1"));
        // NO_PROXY：域名后缀、CIDR。
        assert_eq!(via("wss://a.internal.example/"), None);
        assert_eq!(via("wss://10.1.2.3:8443/"), None);
        assert_eq!(via("wss://internal.example.com/").as_deref(), Some("secure:2"));
        // 只设了 HTTPS_PROXY 时 ws 直连。
        let only_https = WsConnector::with_proxy("", "http://secure:2", "");
        assert!(only_https.proxy_for(&"ws://slack.com/".parse().unwrap()).is_none());
        // `*` 表示全部直连。
        let star = WsConnector::with_proxy("http://plain:1", "http://secure:2", "*");
        assert!(star.proxy_for(&"wss://slack.com/".parse().unwrap()).is_none());
    }

    #[tokio::test]
    async fn proxy_requiring_auth_is_reported() {
        let port = echo_server().await;
        let proxy = FakeProxy::start(Mode::RequireAuth("Basic YWxpY2U6cEBzcw==".into())).await;
        let target = format!("ws://127.0.0.1:{port}/ws");

        let connector = WsConnector::with_proxy(&proxy.url(), &proxy.url(), "");
        let err = connector.connect(&target).await.err().unwrap();
        assert!(err.contains("407") && err.contains("要求认证"), "{err}");

        // 密码不对：报错里不出现用户名和密码。
        let url = format!("http://alice:wrong-secret@127.0.0.1:{}", proxy.port);
        let connector = WsConnector::with_proxy(&url, &url, "");
        let err = connector.connect(&target).await.err().unwrap();
        assert!(err.contains("407") && err.contains("不接受"), "{err}");
        assert!(!err.contains("alice") && !err.contains("wrong-secret"), "{err}");
    }

    #[tokio::test]
    async fn proxy_forbidding_target_is_reported() {
        let port = echo_server().await;
        let proxy = FakeProxy::start(Mode::Forbid).await;
        let connector = WsConnector::with_proxy(&proxy.url(), &proxy.url(), "");
        let err = connector
            .connect(&format!("ws://127.0.0.1:{port}/ws"))
            .await
            .err()
            .unwrap();
        assert!(
            err.contains("403") && err.contains(&format!("127.0.0.1:{port}")),
            "{err}"
        );
    }

    #[tokio::test]
    async fn unsupported_proxy_scheme_is_reported() {
        let connector = WsConnector::with_proxy("socks5://user:pw@127.0.0.1:1080", "", "");
        let err = connector.connect("ws://127.0.0.1:9/ws").await.err().unwrap();
        assert!(err.contains("socks5://") && err.contains("http://"), "{err}");
        assert!(!err.contains("pw"), "{err}");
    }

    #[tokio::test]
    async fn unreachable_proxy_is_reported() {
        // 先占一个端口再放掉，大概率没人在听。
        let port = TcpListener::bind("127.0.0.1:0")
            .await
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let url = format!("http://127.0.0.1:{port}");
        let connector = WsConnector::with_proxy(&url, &url, "");
        let err = connector.connect("ws://127.0.0.1:9/ws").await.err().unwrap();
        assert!(err.contains("连不上代理"), "{err}");
    }

    #[test]
    fn parses_status_line() {
        assert_eq!(
            status_of("HTTP/1.1 200 Connection established\r\n\r\n"),
            Some((200, "Connection established".into()))
        );
        assert_eq!(status_of("HTTP/1.0 407\r\n\r\n"), Some((407, String::new())));
        assert_eq!(status_of("SSH-2.0-OpenSSH\r\n\r\n"), None);
    }
}
