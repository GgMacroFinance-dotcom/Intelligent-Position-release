//! `ip-relay`：手机 App 锁屏推送的中转（m2-spec D-35）。配置全部从环境变量读，凭据不写进任何文件。
//!
//! ```text
//! ip-relay keygen                     # 生成一把封条密钥，放进 IP_RELAY_SEAL_KEY
//! ip-relay check                      # 只检查配置：环境变量、封条密钥、APNs 和 FCM 的密钥能不能签名，不联网
//! ip-relay check --online             # 再向 Google 换一次 FCM 的访问令牌（不发推送）
//! ip-relay                            # 按环境变量启动
//! ```
//!
//! `check` 全部通过时退出码为 0，有一项不过为 1。升级脚本在换上新版本前先跑它。
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
//! | `IP_RELAY_STATS_TOKEN` | 设了才打开 `GET /stats`（计数、版本、最近一次失败的原因），请求带 `Authorization: Bearer <它>`；至少 16 个字符 |
//! | `IP_RELAY_APNS_URL`、`IP_RELAY_FCM_URL` | 只给演练和测试用：把 APNs、FCM 换成假的服务。设了会在日志里警告 |
//!
//! APNs 和 FCM 至少配一个。

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, bail};
use ip_relay::{Apns, ApnsConfig, Fcm, Platform, Relay, Sealer, ServiceAccount};

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
        base_url: match env("IP_RELAY_APNS_URL") {
            Some(url) => {
                tracing::warn!("APNs 换成了 {url}（IP_RELAY_APNS_URL），只该在演练和测试时这样");
                url
            }
            None if sandbox => "https://api.sandbox.push.apple.com".into(),
            None => "https://api.push.apple.com".into(),
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
    let base_url = match env("IP_RELAY_FCM_URL") {
        Some(url) => {
            tracing::warn!("FCM 换成了 {url}（IP_RELAY_FCM_URL），只该在演练和测试时这样");
            url
        }
        None => "https://fcm.googleapis.com".into(),
    };
    Ok(Some(Fcm::new(account, base_url).map_err(anyhow::Error::msg)?))
}

/// 从环境变量读出的全部配置。
struct Loaded {
    public_url: String,
    sealer: Sealer,
    apns: Option<Apns>,
    fcm: Option<Fcm>,
    trust_proxy: bool,
    stats_token: Option<String>,
}

fn load() -> anyhow::Result<Loaded> {
    let public_url = required("IP_RELAY_PUBLIC_URL")?;
    if !public_url.starts_with("https://") {
        bail!("IP_RELAY_PUBLIC_URL 要以 https:// 开头，主机只往 https 的推送服务发");
    }
    let sealer = Sealer::from_b64(&required("IP_RELAY_SEAL_KEY")?).map_err(anyhow::Error::msg)?;
    let (apns, fcm) = (apns()?, fcm()?);
    if apns.is_none() && fcm.is_none() {
        bail!("APNs（IP_RELAY_APNS_KEY_FILE）和 FCM（IP_RELAY_FCM_CREDENTIALS_FILE）至少配一个");
    }
    Ok(Loaded {
        public_url,
        sealer,
        apns,
        fcm,
        trust_proxy: env("IP_RELAY_TRUST_PROXY").is_some_and(|v| v == "1"),
        stats_token: env("IP_RELAY_STATS_TOKEN"),
    })
}

/// `ip-relay check`：逐项检查，打印结果，不打印任何密钥。全部通过时返回 true。
async fn check(online: bool) -> bool {
    let mut ok = true;
    let mut line = |pass: bool, what: &str, detail: String| {
        ok &= pass;
        println!(
            "{} {what}{}",
            if pass { "✓" } else { "✗" },
            if detail.is_empty() {
                detail
            } else {
                format!("：{detail}")
            }
        );
    };
    let cfg = match load() {
        Ok(c) => c,
        Err(e) => {
            line(false, "配置", format!("{e:#}"));
            return false;
        }
    };
    line(true, "配置", format!("对外地址 {}", cfg.public_url));
    let probe = ip_relay::Target {
        platform: if cfg.apns.is_some() {
            Platform::Apns
        } else {
            Platform::Fcm
        },
        token: "ab".repeat(32),
    };
    let sealed = cfg.sealer.seal(&probe);
    line(
        sealed.as_ref().ok().and_then(|s| cfg.sealer.open(s)).as_ref() == Some(&probe),
        "封条密钥",
        String::new(),
    );
    match &cfg.apns {
        Some(a) => line(
            a.self_check().is_ok(),
            "APNs 签名",
            a.self_check().err().unwrap_or_default(),
        ),
        None => println!("- APNs 没配"),
    }
    match &cfg.fcm {
        Some(f) => {
            line(
                f.self_check().is_ok(),
                "FCM 签名",
                f.self_check().err().unwrap_or_default(),
            );
            if online {
                let r = f.check_online().await;
                line(r.is_ok(), "FCM 换访问令牌", r.err().unwrap_or_default());
            }
        }
        None => println!("- FCM 没配"),
    }
    match &cfg.stats_token {
        Some(t) if t.len() < 16 => line(false, "统计令牌", "IP_RELAY_STATS_TOKEN 至少要 16 个字符".into()),
        Some(_) => line(true, "统计令牌", "GET /stats 已打开".into()),
        None => println!("- 没设 IP_RELAY_STATS_TOKEN，GET /stats 关着"),
    }
    ok
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            println!("{}", Sealer::generate_key().map_err(anyhow::Error::msg)?);
            return Ok(());
        }
        Some("check") => {
            let online = args.iter().any(|a| a == "--online");
            std::process::exit(if check(online).await { 0 } else { 1 });
        }
        Some(other) => bail!("不认识的子命令 {other}（只有 keygen、check）"),
        None => {}
    }
    let cfg = load()?;
    let platforms = [cfg.apns.is_some().then_some("APNs"), cfg.fcm.is_some().then_some("FCM")]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("、");
    let relay = Relay::new(cfg.sealer, &cfg.public_url, cfg.apns, cfg.fcm, cfg.trust_proxy)
        .with_stats_token(cfg.stats_token)
        .map_err(anyhow::Error::msg)?;
    let relay = Arc::new(relay);
    let listen = env("IP_RELAY_LISTEN").unwrap_or_else(|| "0.0.0.0:8080".into());
    let listener = tokio::net::TcpListener::bind(&listen)
        .await
        .with_context(|| format!("监听不了 {listen}"))?;
    tracing::info!(
        "ip-relay {} 在 {listen} 上，转发到 {platforms}",
        env!("CARGO_PKG_VERSION")
    );
    axum::serve(
        listener,
        ip_relay::router(relay).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;
    tracing::info!("ip-relay 已停止");
    Ok(())
}

/// Ctrl-C 或 SIGTERM（`docker stop`、`systemctl stop`）时停止收新请求，等正在转发的推送发完。
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
