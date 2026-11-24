//! TLS 的测试。连的是测试里起的假服务器（`fake_server`），不是真的 PostgreSQL：它只走到
//! SSLRequest、TLS 握手、收到启动消息这一步，然后回一个错误。用真的 PostgreSQL 查过的见 PR 描述。

use std::io::Write;
use std::sync::Arc;

use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, IsCa, KeyPair};
use rustls::ServerConfig;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::*;

/// 一个 CA 和它签的、只对 `localhost` 有效的服务器证书。
struct Pki {
    ca_pem: String,
    server: Arc<ServerConfig>,
}

fn new_pki() -> Pki {
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = CertifiedIssuer::self_signed(ca_params, KeyPair::generate().unwrap()).unwrap();
    let server_key = KeyPair::generate().unwrap();
    let server_cert = CertificateParams::new(vec!["localhost".to_string()])
        .unwrap()
        .signed_by(&server_key, &ca)
        .unwrap();
    let server = ServerConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![server_cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
        )
        .unwrap();
    Pki {
        ca_pem: ca.pem(),
        server: Arc::new(server),
    }
}

fn pem_file(pem: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(pem.as_bytes()).unwrap();
    file
}

/// 假服务器看到了什么。
#[derive(Debug, Default, PartialEq, Eq)]
struct Seen {
    ssl_request: bool,
    tls: bool,
    startup: bool,
}

const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f];

/// 起一个只接一次连接的假服务器。`accept_tls` 为假时对 SSLRequest 回 `N`（服务器没开 SSL）。
async fn fake_server(server: Arc<ServerConfig>, accept_tls: bool) -> (u16, tokio::task::JoinHandle<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let mut seen = Seen::default();
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = [0u8; 8];
        if stream.read_exact(&mut head).await.is_err() {
            return seen;
        }
        if head != SSL_REQUEST {
            seen.startup = finish(&mut stream, &head).await;
            return seen;
        }
        seen.ssl_request = true;
        if !accept_tls {
            stream.write_all(b"N").await.unwrap();
            if stream.read_exact(&mut head).await.is_ok() {
                seen.startup = finish(&mut stream, &head).await;
            }
            return seen;
        }
        stream.write_all(b"S").await.unwrap();
        let Ok(mut tls) = TlsAcceptor::from(server).accept(stream).await else {
            return seen;
        };
        seen.tls = true;
        if tls.read_exact(&mut head).await.is_ok() {
            seen.startup = finish(&mut tls, &head).await;
        }
        seen
    });
    (port, task)
}

/// 读完启动消息（`head` 是已经读到的前 8 个字节），回一个 FATAL 错误。
async fn finish<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, head: &[u8; 8]) -> bool {
    let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
    // 协议版本 3.0。
    if head[4..8] != [0, 3, 0, 0] || len < 8 {
        return false;
    }
    let mut rest = vec![0u8; len - 8];
    if stream.read_exact(&mut rest).await.is_err() {
        return false;
    }
    let body = b"SFATAL\0VFATAL\0C28000\0Mfake server\0\0";
    let mut reply = vec![b'E'];
    reply.extend_from_slice(&(body.len() as u32 + 4).to_be_bytes());
    reply.extend_from_slice(body);
    let _ = stream.write_all(&reply).await;
    let _ = stream.flush().await;
    true
}

/// 连一次。返回假服务器看到的，和客户端是不是走到了服务器回的那个错误（握手、证书都过了）。
async fn connect(url: &str, server: Arc<ServerConfig>, accept_tls: bool) -> (Seen, bool) {
    let (port, task) = fake_server(server, accept_tls).await;
    let url = url.replace("PORT", &port.to_string());
    let (config, tls, _) = parse(&url).unwrap();
    let result = config.connect(tls).await;
    let reached = match &result {
        Ok(_) => panic!("假服务器不会让连接成功"),
        Err(e) => e.as_db_error().is_some_and(|db| db.message() == "fake server"),
    };
    (task.await.unwrap(), reached)
}

fn url(host: &str, params: &str) -> String {
    format!("postgres://u:secret-pw@{host}:PORT/db?hostaddr=127.0.0.1&connect_timeout=5&{params}")
}

const OVER_TLS: Seen = Seen {
    ssl_request: true,
    tls: true,
    startup: true,
};

const TLS_REFUSED: Seen = Seen {
    ssl_request: true,
    tls: false,
    startup: false,
};

#[tokio::test]
async fn verify_full_with_ca_file_connects_over_tls() {
    let pki = new_pki();
    let ca = pem_file(&pki.ca_pem);
    let params = format!("sslmode=verify-full&sslrootcert={}", ca.path().display());
    assert_eq!(
        connect(&url("localhost", &params), pki.server, true).await,
        (OVER_TLS, true)
    );
}

#[tokio::test]
async fn verify_full_rejects_wrong_hostname() {
    let pki = new_pki();
    let ca = pem_file(&pki.ca_pem);
    // 证书只对 localhost 有效。
    let params = format!("sslmode=verify-full&sslrootcert={}", ca.path().display());
    assert_eq!(
        connect(&url("127.0.0.1", &params), pki.server, true).await,
        (TLS_REFUSED, false)
    );
}

#[tokio::test]
async fn verify_ca_ignores_hostname_but_checks_chain() {
    let pki = new_pki();
    let ca = pem_file(&pki.ca_pem);
    let params = format!("sslmode=verify-ca&sslrootcert={}", ca.path().display());
    assert_eq!(
        connect(&url("127.0.0.1", &params), pki.server.clone(), true).await,
        (OVER_TLS, true)
    );

    let other = new_pki();
    let wrong_ca = pem_file(&other.ca_pem);
    let params = format!("sslmode=verify-ca&sslrootcert={}", wrong_ca.path().display());
    assert_eq!(
        connect(&url("127.0.0.1", &params), pki.server, true).await,
        (TLS_REFUSED, false)
    );
}

#[tokio::test]
async fn verify_full_rejects_unknown_ca() {
    let pki = new_pki();
    let other = new_pki();
    let wrong_ca = pem_file(&other.ca_pem);
    let params = format!("sslmode=verify-full&sslrootcert={}", wrong_ca.path().display());
    assert_eq!(
        connect(&url("localhost", &params), pki.server.clone(), true).await,
        (TLS_REFUSED, false)
    );
    // 不给 sslrootcert 时用系统根证书，测试 CA 不在里面。机器上读不到系统根证书时直接报配置错误。
    match parse(&url("localhost", "sslmode=verify-full").replace("PORT", "5432")) {
        Ok(_) => assert_eq!(
            connect(&url("localhost", "sslmode=verify-full"), pki.server, true).await,
            (TLS_REFUSED, false)
        ),
        Err(StoreError::Config(message)) => assert!(message.contains("系统根证书"), "{message}"),
        Err(other) => panic!("{other}"),
    }
}

#[tokio::test]
async fn require_encrypts_without_checking_cert() {
    let pki = new_pki();
    assert_eq!(
        connect(&url("127.0.0.1", "sslmode=require"), pki.server, true).await,
        (OVER_TLS, true)
    );
}

#[tokio::test]
async fn require_with_root_cert_checks_chain() {
    let pki = new_pki();
    let other = new_pki();
    let wrong_ca = pem_file(&other.ca_pem);
    let params = format!("sslmode=require&sslrootcert={}", wrong_ca.path().display());
    assert_eq!(
        connect(&url("localhost", &params), pki.server, true).await,
        (TLS_REFUSED, false)
    );
}

#[tokio::test]
async fn prefer_uses_tls_when_offered_and_plain_otherwise() {
    let pki = new_pki();
    // 不写 sslmode 就是 prefer。
    assert_eq!(
        connect(&url("localhost", ""), pki.server.clone(), true).await,
        (OVER_TLS, true)
    );
    let plain = Seen {
        ssl_request: true,
        tls: false,
        startup: true,
    };
    assert_eq!(
        connect(&url("localhost", "sslmode=prefer"), pki.server, false).await,
        (plain, true)
    );
}

#[tokio::test]
async fn require_fails_when_server_has_no_tls() {
    let pki = new_pki();
    let (seen, reached) = connect(&url("localhost", "sslmode=require"), pki.server, false).await;
    assert!(seen.ssl_request && !seen.tls && !reached, "{seen:?}");
}

#[tokio::test]
async fn disable_skips_ssl_request() {
    let pki = new_pki();
    let plain = Seen {
        ssl_request: false,
        tls: false,
        startup: true,
    };
    assert_eq!(
        connect(&url("localhost", "sslmode=disable"), pki.server, true).await,
        (plain, true)
    );
}

#[test]
fn tls_params_are_taken_out_of_url() {
    let (rest, mode, root) = split_tls_params(
        "postgres://u:p%40ss@db.example.com:5432/ip?sslmode=verify-full&application_name=x&sslrootcert=%2Fetc%2Fca%20dir%2Fca.pem",
    )
    .unwrap();
    assert_eq!(rest, "postgres://u:p%40ss@db.example.com:5432/ip?application_name=x");
    assert_eq!(mode, Some(TlsMode::VerifyFull));
    assert_eq!(root, Some(RootCert::File("/etc/ca dir/ca.pem".into())));

    let (rest, mode, root) = split_tls_params("postgresql://h/ip?sslrootcert=system").unwrap();
    assert_eq!(
        (rest.as_str(), mode, root),
        ("postgresql://h/ip", None, Some(RootCert::System))
    );

    // 另一种形式原样交给 tokio-postgres。
    let kv = "host=h dbname=ip sslmode=require";
    assert_eq!(split_tls_params(kv).unwrap(), (kv.to_string(), None, None));
}

#[test]
fn modes_follow_libpq() {
    let mode = |url: &str| parse(url).map(|(_, _, mode)| mode).unwrap();
    assert_eq!(mode("postgres://h/ip"), TlsMode::Prefer);
    assert_eq!(mode("postgres://h/ip?sslmode=disable"), TlsMode::Disable);
    assert_eq!(mode("postgres://h/ip?sslmode=require"), TlsMode::Require);
    assert_eq!(mode("host=h dbname=ip sslmode=require"), TlsMode::Require);
    assert_eq!(mode("host=h dbname=ip sslmode=disable"), TlsMode::Disable);
    let pki = new_pki();
    let ca = pem_file(&pki.ca_pem);
    let with_ca = |m: &str| format!("postgres://h/ip?sslmode={m}&sslrootcert={}", ca.path().display());
    assert_eq!(mode(&with_ca("require")), TlsMode::VerifyCa);
    assert_eq!(mode(&with_ca("prefer")), TlsMode::VerifyCa);
    assert_eq!(mode(&with_ca("verify-full")), TlsMode::VerifyFull);
    assert_eq!(mode(&with_ca("disable")), TlsMode::Disable);

    let (config, _, _) = parse(&with_ca("verify-ca")).unwrap();
    assert_eq!(config.get_ssl_mode(), SslMode::Require);
}

#[test]
fn bad_tls_params_are_config_errors_without_password() {
    let error = |url: &str| match parse(url) {
        Err(StoreError::Config(message)) => message,
        Err(other) => panic!("{other}"),
        Ok(_) => panic!("{url} 应该报错"),
    };
    let message = error("postgres://u:secret-pw@h/ip?sslmode=verify-everything");
    assert!(
        message.contains("verify-everything") && !message.contains("secret-pw"),
        "{message}"
    );

    let message = error("postgres://u:secret-pw@h/ip?sslmode=verify-full&sslrootcert=/no/such/ca.pem");
    assert!(
        message.contains("/no/such/ca.pem") && !message.contains("secret-pw"),
        "{message}"
    );

    let not_pem = pem_file("not a certificate");
    let message = error(&format!(
        "postgres://h/ip?sslmode=verify-full&sslrootcert={}",
        not_pem.path().display()
    ));
    assert!(message.contains("没有能用的证书"), "{message}");

    let message = error("host=h password=secret-pw sslmode=verify-full");
    assert!(
        message.contains("postgres://") && !message.contains("secret-pw"),
        "{message}"
    );
}
