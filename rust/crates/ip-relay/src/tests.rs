//! 用假的 APNs、FCM 和 Google 令牌服务走一遍：登记、转发的字段、令牌失效、各种拒收。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{
    ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, RSA_PKCS1_2048_8192_SHA256,
    UnparsedPublicKey,
};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD as B64};
use serde_json::{Value, json};

use super::*;

/// 假上游收到的一个请求。
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: HeaderMap,
    body: String,
}

#[derive(Clone, Default)]
struct Fake {
    seen: Arc<Mutex<Vec<Seen>>>,
    /// 下一次推送回什么状态码，默认 200。
    status: Arc<Mutex<Option<(u16, Value)>>>,
}

impl Fake {
    fn record(&self, path: String, headers: HeaderMap, body: String) -> (StatusCode, axum::Json<Value>) {
        self.seen.lock().unwrap().push(Seen { path, headers, body });
        let (status, reply) = self.status.lock().unwrap().take().unwrap_or((200, json!({})));
        (StatusCode::from_u16(status).unwrap(), axum::Json(reply))
    }
    fn last(&self) -> Seen {
        self.seen.lock().unwrap().last().cloned().unwrap()
    }
    fn fail_next(&self, status: u16, reply: Value) {
        *self.status.lock().unwrap() = Some((status, reply));
    }
}

async fn serve(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap();
    });
    addr
}

fn pem(der: &[u8]) -> String {
    format!(
        "-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n",
        STANDARD.encode(der)
    )
}

struct Setup {
    relay: SocketAddr,
    apns: Fake,
    google: Fake,
    apns_public: Vec<u8>,
    rsa_public: Vec<u8>,
}

async fn setup() -> Setup {
    let apns = Fake::default();
    let apns_addr =
        serve(
            Router::new()
                .route(
                    "/3/device/{token}",
                    post(
                        |State(f): State<Fake>, Path(t): Path<String>, h: HeaderMap, b: String| async move {
                            f.record(t, h, b)
                        },
                    ),
                )
                .with_state(apns.clone()),
        )
        .await;
    let google = Fake::default();
    let google_addr =
        serve(
            Router::new()
                .route(
                    "/token",
                    post(|State(f): State<Fake>, h: HeaderMap, b: String| async move {
                        f.seen.lock().unwrap().push(Seen {
                            path: "/token".into(),
                            headers: h,
                            body: b,
                        });
                        axum::Json(json!({"access_token": "at-1", "expires_in": 3600, "token_type": "Bearer"}))
                    }),
                )
                .route(
                    "/v1/projects/{project}/messages:send",
                    post(
                        |State(f): State<Fake>, Path(p): Path<String>, h: HeaderMap, b: String| async move {
                            f.record(p, h, b)
                        },
                    ),
                )
                .with_state(google.clone()),
        )
        .await;

    let rng = SystemRandom::new();
    let ec = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let apns_public = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, ec.as_ref())
        .unwrap()
        .public_key()
        .as_ref()
        .to_vec();
    let rsa = aws_lc_rs::rsa::KeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048).unwrap();
    let rsa_public = rsa.public_key().as_ref().to_vec();
    let rsa_der = rsa.as_der().unwrap();

    let apns_client = Apns::new(ApnsConfig {
        key_pem: pem(ec.as_ref()),
        key_id: "KEY123".into(),
        team_id: "TEAM456".into(),
        topic: "org.example.ip".into(),
        base_url: format!("http://{apns_addr}"),
    })
    .unwrap();
    let fcm_client = Fcm::new(
        ServiceAccount {
            project_id: "ip-demo".into(),
            client_email: "relay@ip-demo.iam.gserviceaccount.com".into(),
            private_key: pem(rsa_der.as_ref()),
            token_uri: format!("http://{google_addr}/token"),
        },
        format!("http://{google_addr}"),
    )
    .unwrap();
    let sealer = Sealer::new(&[9u8; 32]).unwrap();
    let relay = Arc::new(Relay::new(
        sealer,
        "http://placeholder/",
        Some(apns_client),
        Some(fcm_client),
        false,
    ));
    let relay_addr = serve(router(relay)).await;
    Setup {
        relay: relay_addr,
        apns,
        google,
        apns_public,
        rsa_public,
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

async fn register(s: &Setup, platform: &str, token: &str) -> reqwest::Response {
    http()
        .post(format!("http://{}/register", s.relay))
        .json(&json!({"platform": platform, "token": token}))
        .send()
        .await
        .unwrap()
}

/// 登记后拿到的订阅地址，换成假中转的真实地址。
async fn endpoint(s: &Setup, platform: &str, token: &str) -> String {
    let r = register(s, platform, token).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let ep = v["endpoint"].as_str().unwrap().to_string();
    assert!(ep.starts_with("http://placeholder/w/"), "{ep}");
    ep.replace("http://placeholder", &format!("http://{}", s.relay))
}

async fn push_to(ep: &str, body: &[u8], extra: &[(&str, &str)]) -> reqwest::Response {
    let mut req = http()
        .post(ep)
        .header("content-encoding", "aes128gcm")
        .header("ttl", "3600")
        .body(body.to_vec());
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    req.send().await.unwrap()
}

fn verify_jwt(
    token: &str,
    alg: &'static dyn aws_lc_rs::signature::VerificationAlgorithm,
    public: &[u8],
) -> (Value, Value) {
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    let msg = format!("{}.{}", parts[0], parts[1]);
    UnparsedPublicKey::new(alg, public)
        .verify(msg.as_bytes(), &B64.decode(parts[2]).unwrap())
        .expect("签名要能用公钥验过");
    let decode = |p: &str| serde_json::from_slice::<Value>(&B64.decode(p).unwrap()).unwrap();
    (decode(parts[0]), decode(parts[1]))
}

#[tokio::test]
async fn apns_push_is_forwarded_as_is() {
    let s = setup().await;
    let token = "0a".repeat(32);
    let ep = endpoint(&s, "apns", &token).await;
    let body = b"\x01\x02ciphertext\xff";
    let r = push_to(&ep, body, &[("urgency", "high"), ("topic", "abcDEF_123-x")]).await;
    assert_eq!(r.status(), 201);

    let seen = s.apns.last();
    assert_eq!(seen.path, token);
    let h = |k: &str| seen.headers.get(k).map(|v| v.to_str().unwrap().to_string());
    assert_eq!(h("apns-topic").as_deref(), Some("org.example.ip"));
    assert_eq!(h("apns-push-type").as_deref(), Some("alert"));
    assert_eq!(h("apns-priority").as_deref(), Some("10"));
    assert_eq!(h("apns-collapse-id").as_deref(), Some("abcDEF_123-x"));
    let exp: i64 = h("apns-expiration").unwrap().parse().unwrap();
    assert!((exp - upstream::now_s() - 3600).abs() < 10, "{exp}");
    let jwt = h("authorization").unwrap();
    let (header, claims) = verify_jwt(
        jwt.strip_prefix("Bearer ").unwrap(),
        &ECDSA_P256_SHA256_FIXED,
        &s.apns_public,
    );
    assert_eq!(header["kid"], "KEY123");
    assert_eq!(claims["iss"], "TEAM456");
    let v: Value = serde_json::from_str(&seen.body).unwrap();
    assert_eq!(v["ip"], STANDARD.encode(body));
    assert_eq!(v["aps"]["mutable-content"], 1);

    // 普通的：低优先级，没有 collapse id；JWT 缓存着，第二次用同一个
    assert_eq!(push_to(&ep, body, &[]).await.status(), 201);
    let again = s.apns.last();
    assert_eq!(again.headers["apns-priority"], "5");
    assert!(again.headers.get("apns-collapse-id").is_none());
    assert_eq!(again.headers["authorization"], seen.headers["authorization"]);
}

#[tokio::test]
async fn fcm_push_is_forwarded_as_a_data_message() {
    let s = setup().await;
    let token = "dGVzdA:APA91bH-abc_DEF0123456789";
    let ep = endpoint(&s, "fcm", token).await;
    let body = b"opaque";
    assert_eq!(
        push_to(&ep, body, &[("urgency", "high"), ("topic", "t1")])
            .await
            .status(),
        201
    );

    let seen = s.google.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "先换访问令牌，再发");
    let form = &seen[0].body;
    assert!(form.starts_with("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion="));
    let assertion = form.split("assertion=").nth(1).unwrap();
    let (_, claims) = verify_jwt(assertion, &RSA_PKCS1_2048_8192_SHA256, &s.rsa_public);
    assert_eq!(claims["iss"], "relay@ip-demo.iam.gserviceaccount.com");
    assert_eq!(claims["scope"], "https://www.googleapis.com/auth/firebase.messaging");

    assert_eq!(seen[1].path, "ip-demo");
    assert_eq!(seen[1].headers["authorization"], "Bearer at-1");
    let v: Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(v["message"]["token"], token);
    assert_eq!(v["message"]["data"]["ip"], STANDARD.encode(body));
    assert_eq!(v["message"]["android"]["priority"], "HIGH");
    assert_eq!(v["message"]["data"]["urgency"], "high");
    assert_eq!(v["message"]["android"]["ttl"], "3600s");
    assert_eq!(v["message"]["android"]["collapse_key"], "t1");
    assert!(v["message"].get("notification").is_none(), "不让系统直接显示");

    // 访问令牌缓存着，第二次不再换
    assert_eq!(push_to(&ep, body, &[]).await.status(), 201);
    assert_eq!(s.google.seen.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn dead_tokens_answer_410() {
    let s = setup().await;
    let apns = endpoint(&s, "apns", &"0b".repeat(32)).await;
    s.apns.fail_next(410, json!({"reason": "Unregistered"}));
    assert_eq!(push_to(&apns, b"x", &[]).await.status(), 410);
    s.apns.fail_next(400, json!({"reason": "BadDeviceToken"}));
    assert_eq!(push_to(&apns, b"x", &[]).await.status(), 410);
    s.apns.fail_next(503, json!({"reason": "ServiceUnavailable"}));
    assert_eq!(push_to(&apns, b"x", &[]).await.status(), 502);

    let fcm = endpoint(&s, "fcm", "fcm-token-0123456789abcdef").await;
    s.google.fail_next(404, json!({"error": {"status": "NOT_FOUND"}}));
    assert_eq!(push_to(&fcm, b"x", &[]).await.status(), 410);
}

#[tokio::test]
async fn bad_requests_are_refused() {
    let s = setup().await;
    // 令牌格式不对、平台不认识
    assert_eq!(register(&s, "apns", "not-hex-not-hex-not-hex").await.status(), 400);
    assert_eq!(register(&s, "fcm", "short").await.status(), 400);
    assert_eq!(register(&s, "wns", &"0c".repeat(32)).await.status(), 422);

    let ep = endpoint(&s, "apns", &"0c".repeat(32)).await;
    // 不是 aes128gcm
    let r = http().post(&ep).header("ttl", "60").body("plain").send().await.unwrap();
    assert_eq!(r.status(), 400);
    // 太大、空的
    assert_eq!(push_to(&ep, &vec![0u8; MAX_BODY + 1], &[]).await.status(), 413);
    assert_eq!(push_to(&ep, b"", &[]).await.status(), 413);
    // 封条被改过、瞎编的
    let base = ep.rsplit_once('/').unwrap().0;
    assert_eq!(
        push_to(&format!("{base}/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"), b"x", &[])
            .await
            .status(),
        404
    );
    // 不合格的 Topic 不转
    assert_eq!(push_to(&ep, b"x", &[("topic", "has space")]).await.status(), 201);
    assert!(s.apns.last().headers.get("apns-collapse-id").is_none());
    // 都没送到上游
    assert_eq!(s.apns.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn pushes_and_registrations_are_rate_limited() {
    let s = setup().await;
    let ep = endpoint(&s, "apns", &"0d".repeat(32)).await;
    for _ in 0..PUSH_PER_WINDOW {
        assert_eq!(push_to(&ep, b"x", &[]).await.status(), 201);
    }
    assert_eq!(push_to(&ep, b"x", &[]).await.status(), 429);
    // 已经登记过一次
    for _ in 1..REGISTER_PER_WINDOW {
        assert_eq!(register(&s, "apns", &"0e".repeat(32)).await.status(), 200);
    }
    assert_eq!(register(&s, "apns", &"0e".repeat(32)).await.status(), 429);
}

#[test]
fn limiter_resets_each_window() {
    let l = Limiter::new(2, 1000);
    assert!(l.allow("a", 0));
    assert!(l.allow("a", 10));
    assert!(!l.allow("a", 20));
    assert!(l.allow("b", 20));
    assert!(l.allow("a", 1000));
}

#[test]
fn forwarding_is_timed() {
    let t = time_forward(10).unwrap();
    assert_eq!(t.len(), 10);
}
