//! `ip-relay`：手机 App 锁屏推送的中转（m2-spec D-35）。配置全部从环境变量读，凭据不写进任何文件。
//!
//! ```text
//! ip-relay keygen                     # 生成一把封条密钥，放进 IP_RELAY_SEAL_KEY
//! ip-relay                            # 按环境变量启动
//! ```
//!
//! | 环境变量 | 说明 |
//! | --- | --- |
//! | `IP_RELAY_LISTEN` | 监听地址，默认 `0.0.0.0:8080` |
//! | `IP_RELAY_PUBLIC_URL` | 对外的地址，比如 `https://push.example.com`（必填） |
//! | `IP_RELAY_SEAL_KEY` | 封条密钥，`ip-relay keygen` 生成（必填）。换了它，所有订阅地址失效，App 下次打开时重新登记 |
//! | `IP_RELAY_APNS_KEY_FILE` | APNs 的 `.p8` 签名密钥文件 |
//! | `IP_RELAY_APNS_KEY_ID`、`IP_RELAY_APNS_TEAM_ID` | 苹果开发者后台里这把密钥的 Key ID 和 Team ID |
//! | `IP_RELAY_APNS_TOPIC` | App 的 Bundle ID |
//! | `IP_RELAY_APNS_SANDBOX` | 设成 `1` 时发到苹果的测试环境（开发版 App 用） |
//! | `IP_RELAY_FCM_CREDENTIALS_FILE` | Firebase 服务账号的 JSON 文件 |
//! | `IP_RELAY_TRUST_PROXY` | 设成 `1` 时按 `X-Forwarded-For` 限流（只在反向代理后面开） |
//!
//! APNs 和 FCM 至少配一个。

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, bail};
use ip_relay::{Apns, ApnsConfig, Fcm, Relay, Sealer, ServiceAccount};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn required(name: &str) -> anyhow::Result<String> {
    env(name).with_context(|| format!("缺少环境变量 {name}"))
}

fn apns() -> anyhow::Result<Option<Apns>> {
    let Some(file) = env("IP_RELAY_APNS_KEY_FILE") else {
        return Ok(None);
    };
    let key_pem = std::fs::read_to_string(&file).with_context(|| format!("读不了 APNs 密钥文件 {file}"))?;
    let sandbox = env("IP_RELAY_APNS_SANDBOX").is_some_and(|v| v == "1");
    let cfg = ApnsConfig {
        key_pem,
        key_id: required("IP_RELAY_APNS_KEY_ID")?,
        team_id: required("IP_RELAY_APNS_TEAM_ID")?,
        topic: required("IP_RELAY_APNS_TOPIC")?,
        base_url: if sandbox {
            "https://api.sandbox.push.apple.com".into()
        } else {
            "https://api.push.apple.com".into()
        },
    };
    Ok(Some(Apns::new(cfg).map_err(anyhow::Error::msg)?))
}

fn fcm() -> anyhow::Result<Option<Fcm>> {
    let Some(file) = env("IP_RELAY_FCM_CREDENTIALS_FILE") else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(&file).with_context(|| format!("读不了服务账号文件 {file}"))?;
    let account: ServiceAccount = serde_json::from_str(&text).context("服务账号文件不是 Google 的服务账号 JSON")?;
    Ok(Some(
        Fcm::new(account, "https://fcm.googleapis.com".into()).map_err(anyhow::Error::msg)?,
    ))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    if std::env::args().nth(1).as_deref() == Some("keygen") {
        println!("{}", Sealer::generate_key().map_err(anyhow::Error::msg)?);
        return Ok(());
    }
    let public_url = required("IP_RELAY_PUBLIC_URL")?;
    if !public_url.starts_with("https://") {
        bail!("IP_RELAY_PUBLIC_URL 要以 https:// 开头，主机只往 https 的推送服务发");
    }
    let sealer = Sealer::from_b64(&required("IP_RELAY_SEAL_KEY")?).map_err(anyhow::Error::msg)?;
    let (apns, fcm) = (apns()?, fcm()?);
    if apns.is_none() && fcm.is_none() {
        bail!("APNs（IP_RELAY_APNS_KEY_FILE）和 FCM（IP_RELAY_FCM_CREDENTIALS_FILE）至少配一个");
    }
    let platforms = [apns.is_some().then_some("APNs"), fcm.is_some().then_some("FCM")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("、");
    let trust_proxy = env("IP_RELAY_TRUST_PROXY").is_some_and(|v| v == "1");
    let relay = Arc::new(Relay::new(sealer, &public_url, apns, fcm, trust_proxy));
    let listen = env("IP_RELAY_LISTEN").unwrap_or_else(|| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("监听不了 {listen}"))?;
    tracing::info!("ip-relay 在 {listen} 上，转发到 {platforms}");
    axum::serve(
        listener,
        ip_relay::router(relay).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
