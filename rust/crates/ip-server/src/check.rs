//! `ip check`：逐项检查 `ip.yaml` 里配的东西能不能用。真实验收（`docs/ip/m2-acceptance.md`）的第一步。
//!
//! 只做读和最轻的调用：列端点的模型、调 Slack 的 `auth.test` 和 `apps.connections.open`（不连 WebSocket）、
//! 跑 `codex --version`、读数据源目录、连一下 MCP 服务器再断开。不发消息，不调模型，不花钱。

use std::fmt::Write as _;
use std::time::Duration;

use crate::config::{ChannelConfig, Config, SandboxMode};

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Ok,
    /// 能用，但有要注意的地方。
    Warn,
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    pub status: Status,
    /// 检查的是什么，比如“端点 deepseek”。
    pub item: String,
    pub detail: String,
}

#[derive(Debug, Default)]
pub struct Report {
    pub findings: Vec<Finding>,
}

impl Report {
    fn push(&mut self, status: Status, item: impl Into<String>, detail: impl Into<String>) {
        self.findings.push(Finding {
            status,
            item: item.into(),
            detail: detail.into(),
        });
    }

    pub fn failed(&self) -> bool {
        self.findings.iter().any(|f| f.status == Status::Fail)
    }

    /// 给人看的结果：每项一行，最后一行是汇总。
    pub fn render(&self) -> String {
        let mut out = String::new();
        for f in &self.findings {
            let mark = match f.status {
                Status::Ok => "✓",
                Status::Warn => "!",
                Status::Fail => "✗",
            };
            let _ = writeln!(out, "{mark} {}：{}", f.item, f.detail);
        }
        let count = |s| self.findings.iter().filter(|f| f.status == s).count();
        let _ = writeln!(
            out,
            "\n{} 项通过，{} 项要注意，{} 项失败",
            count(Status::Ok),
            count(Status::Warn),
            count(Status::Fail)
        );
        out
    }
}

/// 跑全部检查。`env` 读环境变量（测试里可以换掉）。
pub async fn check(config: &Config, env: &(dyn Fn(&str) -> Option<String> + Sync)) -> Report {
    let mut r = Report::default();
    let present = |var: &str| env(var).filter(|v| !v.is_empty());

    if let Some(var) = &config.auth_token_env {
        match present(var) {
            Some(_) => r.push(Status::Ok, "访问令牌", format!("{var} 已设置")),
            None => r.push(Status::Fail, "访问令牌", format!("环境变量 {var} 没有设置")),
        }
    }

    check_data_dir(config, &mut r);
    match config.sandbox.mode {
        SandboxMode::Bubblewrap => match ip_tools::Sandbox::bubblewrap(config.sandbox.network).await {
            Ok(_) => r.push(Status::Ok, "工具沙箱", "bubblewrap 可用"),
            Err(e) => r.push(
                Status::Fail,
                "工具沙箱",
                format!("{e}。开发机上可以在配置里写 sandbox: {{ mode: none }}"),
            ),
        },
        SandboxMode::None => r.push(Status::Warn, "工具沙箱", "已关闭：员工的 shell 命令直接在本机执行"),
    }

    let http = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .unwrap_or_default();
    for (name, ep) in &config.endpoints {
        check_endpoint(config, name, ep, &http, &present, &mut r).await;
    }

    if !config.data_sources.is_empty() {
        match ip_data::DataHub::open(&config.data_sources, std::path::Path::new(".")) {
            Ok(hub) => {
                for h in hub.health() {
                    let item = format!("数据源 {}", h.name);
                    match (&h.error, h.readable, h.series) {
                        (Some(e), ..) => r.push(Status::Fail, item, e.clone()),
                        (None, _, 0) => r.push(Status::Warn, item, "目录里没有登记任何序列"),
                        (None, 0, n) => {
                            r.push(Status::Warn, item, format!("{n} 个序列都还没有数据，先跑 ip data pull"))
                        }
                        (None, k, n) if k < n => r.push(Status::Warn, item, format!("{n} 个序列里 {k} 个有数据")),
                        (None, _, n) => r.push(Status::Ok, item, format!("{n} 个序列都有数据")),
                    }
                }
            }
            Err(e) => r.push(Status::Fail, "数据源", e.to_string()),
        }
    }

    if !config.mcp_servers.is_empty() {
        match ip_mcp::McpHub::connect_with_env(&config.mcp_servers, env).await {
            Ok(hub) => {
                // 连不上的服务器不挡住服务启动，但这里照样算失败，提醒先修好。
                for down in hub.unavailable() {
                    let roles = if down.roles.is_empty() {
                        "所有岗位".to_string()
                    } else {
                        down.roles.iter().map(|r| r.as_str()).collect::<Vec<_>>().join("、")
                    };
                    let after = if down.error.retryable() {
                        format!(
                            "服务仍会启动，并在后台重连 {} 次；连上之前 {roles} 用不了它的工具",
                            ip_mcp::Reconnect::default().attempts
                        )
                    } else {
                        format!("服务仍会启动，{roles} 用不了它的工具；密钥没设不会重连，设好后重启服务")
                    };
                    r.push(
                        Status::Fail,
                        format!("MCP 服务器 {}", down.name),
                        format!("{}；{after}", down.error),
                    );
                }
                let connected = config.mcp_servers.len() - hub.unavailable().len();
                if connected > 0 {
                    let tools: usize = config
                        .roles
                        .iter()
                        .map(|role| hub.specs_for(&role.id).len())
                        .max()
                        .unwrap_or_default();
                    let detail = if hub.unavailable().is_empty() {
                        format!("{connected} 个都连上了，共 {tools} 个工具")
                    } else {
                        format!("{connected} 个连上了，共 {tools} 个工具")
                    };
                    r.push(Status::Ok, "MCP 服务器", detail);
                }
                hub.shutdown().await;
            }
            Err(e) => r.push(Status::Fail, "MCP 服务器", e.to_string()),
        }
    }

    for (role, engine) in &config.engines {
        check_engine(role, engine, &present, &mut r).await;
    }

    for (name, channel) in &config.channels {
        let ChannelConfig::Slack(slack) = channel;
        let item = format!("渠道 {name}");
        let missing: Vec<&str> = [slack.bot_token_env.as_str(), slack.app_token_env.as_str()]
            .into_iter()
            .filter(|v| present(v).is_none())
            .collect();
        if !missing.is_empty() {
            r.push(Status::Fail, item, format!("环境变量 {} 没有设置", missing.join("、")));
            continue;
        }
        let env = |var: &str| present(var).ok_or_else(|| anyhow::anyhow!("环境变量 {var} 没有设置"));
        match crate::app::slack_channel(name, slack, env) {
            Ok((channel, _)) => match channel.verify().await {
                Ok(detail) => r.push(Status::Ok, item, detail),
                Err(e) => r.push(Status::Fail, item, e),
            },
            Err(e) => r.push(Status::Fail, item, e.to_string()),
        }
    }
    r
}

fn check_data_dir(config: &Config, r: &mut Report) {
    let dir = &config.data_dir;
    if let Err(e) = std::fs::create_dir_all(dir) {
        r.push(Status::Fail, "数据目录", format!("创建 {} 失败：{e}", dir.display()));
        return;
    }
    let probe = dir.join(".ip-check");
    match std::fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
        }
        Err(e) => {
            r.push(Status::Fail, "数据目录", format!("{} 不可写：{e}", dir.display()));
            return;
        }
    }
    match ip_store::Store::open(dir.join("ip.db")) {
        Ok(_) => r.push(Status::Ok, "数据目录", format!("{} 可写，数据库能打开", dir.display())),
        Err(e) => r.push(Status::Fail, "数据目录", format!("打开数据库失败：{e}")),
    }
}

async fn check_endpoint(
    config: &Config,
    name: &str,
    ep: &crate::config::EndpointConfig,
    http: &reqwest::Client,
    present: &(dyn Fn(&str) -> Option<String> + Sync),
    r: &mut Report,
) {
    let item = format!("端点 {name}");
    let key = match &ep.api_key_env {
        Some(var) => match present(var) {
            Some(k) => Some(k),
            None => {
                r.push(Status::Fail, item, format!("环境变量 {var} 没有设置"));
                return;
            }
        },
        None => None,
    };
    // 这个端点上岗位和语义分派要用的模型（用外部引擎的岗位不调模型）。
    let wanted: Vec<String> = config
        .roles
        .iter()
        .filter(|role| !config.engines.contains_key(role.id.as_str()))
        .flat_map(|role| std::iter::once(&role.model.default).chain(role.model.escalate.as_ref()))
        .chain(config.router.semantic.as_ref().map(|s| &s.model))
        .filter_map(|m| m.split_once('/'))
        .filter(|(ep_name, _)| *ep_name == name)
        .map(|(_, model)| model.to_string())
        .collect();
    let url = format!("{}/models", ep.base_url.trim_end_matches('/'));
    let mut req = http.get(&url);
    if let Some(k) = &key {
        req = req.bearer_auth(k);
    }
    let resp = match req.send().await {
        Ok(resp) => resp,
        Err(e) => {
            r.push(Status::Fail, item, format!("连不上 {url}：{e}"));
            return;
        }
    };
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        r.push(Status::Fail, item, format!("key 无效（HTTP {status}）"));
        return;
    }
    if status != 200 {
        r.push(
            Status::Warn,
            item,
            format!("列不出模型（HTTP {status}），端点能连上，对话时再看"),
        );
        return;
    }
    let listed: Vec<String> = resp
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|v| v.get("data").and_then(|d| d.as_array()).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
        .collect();
    let missing: Vec<&String> = wanted.iter().filter(|m| !listed.contains(m)).collect();
    if missing.is_empty() {
        let what = if wanted.is_empty() {
            "能连上".to_string()
        } else {
            format!("能连上，要用的模型都在：{}", wanted.join("、"))
        };
        r.push(Status::Ok, item, what);
    } else {
        let names: Vec<&str> = missing.iter().map(|m| m.as_str()).collect();
        r.push(
            Status::Warn,
            item,
            format!("能连上，但模型列表里没有 {}，检查模型名是否写对", names.join("、")),
        );
    }
}

async fn check_engine(
    role: &str,
    engine: &ip_engines::EngineConfig,
    present: &(dyn Fn(&str) -> Option<String> + Sync),
    r: &mut Report,
) {
    let item = format!("岗位 {role} 的 {} 引擎", engine.label());
    let command = engine.command();
    if let Some(var) = engine.api_key_env()
        && present(var).is_none()
    {
        r.push(Status::Fail, item, format!("环境变量 {var} 没有设置"));
        return;
    }
    // 和真正派活时一样不带 harness 的环境变量（里面有模型和渠道的密钥），只给 PATH 和 HOME。
    let mut run = tokio::process::Command::new(command);
    run.arg("--version")
        .env_clear()
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    for var in ["PATH", "HOME"] {
        if let Some(value) = std::env::var_os(var) {
            run.env(var, value);
        }
    }
    let run = run.output();
    match tokio::time::timeout(Duration::from_secs(20), run).await {
        Ok(Ok(out)) if out.status.success() => {
            let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
            r.push(Status::Ok, item, format!("{} 可以运行：{version}", command));
        }
        Ok(Ok(out)) => r.push(
            Status::Fail,
            item,
            format!(
                "{} --version 失败（{}）：{}",
                command,
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        ),
        Ok(Err(e)) => r.push(Status::Fail, item, format!("找不到或运行不了 {}：{e}", command)),
        Err(_) => r.push(Status::Fail, item, format!("{} --version 超过 20 秒没有返回", command)),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use ip_connectors::fake::slack::{Failure, FakeSlack};
    use serde_json::json;

    use super::*;

    /// 假的 OpenAI 兼容端点：只认 `Bearer good`，模型列表里只有 m1。
    async fn fake_endpoint() -> String {
        async fn models(headers: HeaderMap) -> axum::response::Response {
            let auth = headers.get("authorization").and_then(|v| v.to_str().ok());
            if auth != Some("Bearer good") {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            axum::Json(json!({"object": "list", "data": [{"id": "m1"}]})).into_response()
        }
        let app = axum::Router::new().route("/v1/models", axum::routing::get(models));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}/v1")
    }

    fn yaml(dir: &std::path::Path, endpoint: &str, slack: &str, codex: &str) -> String {
        let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        // 登记了序列、还没拉过数据的目录。
        let public = dir.join("public");
        std::fs::create_dir_all(&public).unwrap();
        std::fs::write(
            public.join("catalog.yaml"),
            "source: 测试\nseries:\n  - { id: us_cpi, name: 美国 CPI, frequency: monthly, fetch: { fred: { id: CPIAUCSL } } }\n",
        )
        .unwrap();
        format!(
            r#"
data_dir: {data}
auth_token_env: IP_TOKEN
sandbox: {{ mode: none }}
endpoints:
  ep: {{ base_url: "{endpoint}", api_key_env: EP_KEY }}
roles:
  - {{ id: macro, description: 研报, model: {{ default: ep/m1, escalate: ep/m2 }} }}
  - {{ id: ops, description: 运维, model: {{ default: ep/unused }} }}
engines:
  ops: {{ kind: codex, command: "{codex}", api_key_env: CODEX_KEY }}
agents:
  - {{ id: macro-1, role: macro, name: 小宏 }}
  - {{ id: ops-1, role: ops, name: 小维 }}
data_sources:
  demo: {{ kind: csv_dir, path: "{demo}" }}
  public: {{ kind: csv_dir, path: "{public}" }}
channels:
  slack: {{ kind: slack, bot_token_env: SLACK_BOT, app_token_env: SLACK_APP, api_url: "{slack}" }}
"#,
            data = dir.join("data").display(),
            demo = examples.join("macro-data").display(),
            public = public.display(),
        )
    }

    fn status_of<'a>(r: &'a Report, item: &str) -> &'a Finding {
        r.findings
            .iter()
            .find(|f| f.item == item)
            .unwrap_or_else(|| panic!("没有检查 {item}：{}", r.render()))
    }

    #[tokio::test]
    async fn check_reports_each_part_of_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let codex = dir.path().join("codex");
        std::fs::write(&codex, "#!/bin/sh\necho codex-cli 0.99.0\n").unwrap();
        std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
        let endpoint = fake_endpoint().await;
        let slack = FakeSlack::start().await;
        let config = Config::from_yaml(&yaml(
            dir.path(),
            &endpoint,
            &slack.api_url(),
            &codex.display().to_string(),
        ))
        .unwrap();

        let all = |var: &str| match var {
            "EP_KEY" => Some("good".to_string()),
            _ => Some(format!("{var}-value")),
        };
        let r = check(&config, &all).await;
        assert!(!r.failed(), "{}", r.render());
        assert_eq!(status_of(&r, "访问令牌").status, Status::Ok);
        assert_eq!(status_of(&r, "数据目录").status, Status::Ok);
        assert_eq!(status_of(&r, "工具沙箱").status, Status::Warn);
        // m1 在列表里，m2 不在；用引擎的 ops 岗位的模型不检查。
        let ep = status_of(&r, "端点 ep");
        assert_eq!(ep.status, Status::Warn);
        assert!(
            ep.detail.contains("m2") && !ep.detail.contains("unused"),
            "{}",
            ep.detail
        );
        assert_eq!(status_of(&r, "数据源 demo").status, Status::Ok);
        let public = status_of(&r, "数据源 public");
        assert!(
            public.status == Status::Warn && public.detail.contains("ip data pull"),
            "{}",
            public.detail
        );
        let engine = status_of(&r, "岗位 ops 的 Codex 引擎");
        assert!(
            engine.status == Status::Ok && engine.detail.contains("codex-cli 0.99.0"),
            "{}",
            engine.detail
        );
        let channel = status_of(&r, "渠道 slack");
        assert!(
            channel.status == Status::Ok && channel.detail.contains("工作区 T1"),
            "{}",
            channel.detail
        );
        assert_eq!(slack.calls("apps.connections.open").len(), 1);
        assert!(r.render().ends_with("项失败\n"));

        // 密钥没设置、key 不对、令牌失效、codex 不存在：都算失败。
        let none = |var: &str| match var {
            "EP_KEY" => Some("wrong".to_string()),
            "SLACK_BOT" | "SLACK_APP" => Some("x".to_string()),
            _ => None,
        };
        slack.fail_next("auth.test", Failure::Error("invalid_auth".into()));
        let broken = yaml(dir.path(), &endpoint, &slack.api_url(), "/nonexistent/codex");
        let r = check(&Config::from_yaml(&broken).unwrap(), &none).await;
        assert!(r.failed());
        for item in ["访问令牌", "端点 ep", "岗位 ops 的 Codex 引擎", "渠道 slack"] {
            assert_eq!(status_of(&r, item).status, Status::Fail, "{item}：{}", r.render());
        }
        assert!(status_of(&r, "端点 ep").detail.contains("key 无效"));
        assert!(status_of(&r, "岗位 ops 的 Codex 引擎").detail.contains("CODEX_KEY"));
        assert!(status_of(&r, "渠道 slack").detail.contains("invalid_auth"));
    }

    /// 连不上的 MCP 服务器逐个列为失败，写明服务仍会启动、哪些岗位用不了它。
    #[tokio::test]
    async fn check_lists_each_unreachable_mcp_server() {
        let dir = tempfile::tempdir().unwrap();
        let text = format!(
            r#"
data_dir: {data}
sandbox: {{ mode: none }}
endpoints:
  ep: {{ base_url: "http://127.0.0.1:1" }}
roles:
  - {{ id: macro, description: 研报, model: {{ default: ep/m1 }} }}
agents:
  - {{ id: macro-1, role: macro, name: 小宏 }}
mcp_servers:
  broken: {{ command: /nonexistent/mcp-server, roles: [macro] }}
  nokey: {{ command: "true", env_from: {{ TOKEN: IP_TEST_SURELY_UNSET_VAR }} }}
"#,
            data = dir.path().join("data").display(),
        );
        let r = check(&Config::from_yaml(&text).unwrap(), &|_| None).await;
        assert!(r.failed());
        let broken = status_of(&r, "MCP 服务器 broken");
        assert_eq!(broken.status, Status::Fail);
        assert!(
            broken.detail.contains("服务仍会启动") && broken.detail.contains("macro 用不了"),
            "{}",
            broken.detail
        );
        let nokey = status_of(&r, "MCP 服务器 nokey");
        assert!(
            nokey.detail.contains("IP_TEST_SURELY_UNSET_VAR") && nokey.detail.contains("所有岗位"),
            "{}",
            nokey.detail
        );
        assert!(
            !r.findings.iter().any(|f| f.item == "MCP 服务器"),
            "一个都没连上时不报“连上了”：{}",
            r.render()
        );
    }
}
