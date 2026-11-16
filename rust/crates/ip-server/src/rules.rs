//! 路由规则的在线编辑和规则草案。
//!
//! - 控制台保存的规则存进数据库，立即替换生效，重启后仍然沿用（优先于 ip.yaml 里的 `router`）。
//! - 人在卡片上给拿不准的消息选了岗位，就按消息关键词生成一条草案，等人修改后采纳或忽略。
//!   草案从不自动生效：规则永远是人写下或人点头的。

use ip_core::{Author, RoleId, now_ms};
use ip_router::{Router, RouterSpec, RuleDraft, draft_rule};

use crate::App;

const SPEC_KEY: &str = "router.spec";
const DRAFTS_KEY: &str = "router.drafts";
/// 最多保留的草案数，多了丢最旧的。
const MAX_DRAFTS: usize = 20;

/// 启动时：数据库里有控制台保存过的规则，就用它代替 ip.yaml 里的。
/// 保存的规则和现在的组织对不上（比如岗位被删了）时，记一条警告并沿用 ip.yaml。
pub async fn load_saved(app: &App) {
    let saved = match app.runtime.store().setting(SPEC_KEY).await {
        Ok(Some(v)) => v,
        Ok(None) => return,
        Err(e) => {
            tracing::warn!(error = %e, "读取已保存的路由规则失败，沿用 ip.yaml");
            return;
        }
    };
    let built = serde_json::from_value::<RouterSpec>(saved)
        .map_err(|e| e.to_string())
        .and_then(|spec| build(app, spec));
    match built {
        Ok(router) => {
            app.set_router(router);
            app.rules_from_console.store(true, std::sync::atomic::Ordering::Relaxed);
            tracing::info!("使用控制台保存的路由规则");
        }
        Err(e) => tracing::warn!(error = %e, "已保存的路由规则和当前组织不一致，沿用 ip.yaml"),
    }
}

/// 校验并保存一套规则，立即生效。`accepted_draft` 是这次采纳的草案，保存成功后从草案里移除。
pub async fn save(app: &App, spec: RouterSpec, accepted_draft: Option<String>) -> Result<RouterSpec, String> {
    let router = build(app, spec)?;
    let spec = router.spec().clone();
    let _guard = app.rules_lock.lock().await;
    let value = serde_json::to_value(&spec).map_err(|e| e.to_string())?;
    app.runtime
        .store()
        .put_setting(SPEC_KEY, value)
        .await
        .map_err(|e| e.to_string())?;
    app.set_router(router);
    app.rules_from_console.store(true, std::sync::atomic::Ordering::Relaxed);
    if let Some(id) = accepted_draft {
        let mut drafts = load_drafts(app).await?;
        drafts.retain(|d| d.id != id);
        store_drafts(app, &drafts).await?;
    }
    Ok(spec)
}

pub async fn drafts(app: &App) -> Result<Vec<RuleDraft>, String> {
    load_drafts(app).await
}

pub async fn dismiss(app: &App, id: &str) -> Result<Vec<RuleDraft>, String> {
    let _guard = app.rules_lock.lock().await;
    let mut drafts = load_drafts(app).await?;
    let before = drafts.len();
    drafts.retain(|d| d.id != id);
    if drafts.len() == before {
        return Err(format!("找不到草案 {id}"));
    }
    store_drafts(app, &drafts).await?;
    Ok(drafts)
}

/// 人把一条拿不准的消息交给了某个岗位：生成一条草案。提不出关键词，或已有规则会把它分到同一岗位时不生成。
pub async fn draft_from_override(
    app: &App,
    text: &str,
    role: &RoleId,
    by: &Author,
) -> Result<Option<RuleDraft>, String> {
    let _guard = app.rules_lock.lock().await;
    let router = app.router();
    let mut drafts = load_drafts(app).await?;
    let mut taken: Vec<&str> = router.spec().rules.iter().map(|r| r.id.as_str()).collect();
    taken.extend(drafts.iter().map(|d| d.rule.id.as_str()));
    let Some(rule) = draft_rule(text, role, &taken) else {
        return Ok(None);
    };
    // 同一岗位、同样关键词的草案已经在等审核，就不重复加。
    if drafts
        .iter()
        .any(|d| d.rule.to.role == rule.to.role && d.rule.when.any == rule.when.any)
    {
        return Ok(None);
    }
    let who = match by {
        Author::Human { name, .. } => name.clone(),
        Author::Agent { agent_id } => agent_id.to_string(),
        Author::System => "系统".into(),
    };
    let excerpt: String = text.chars().take(40).collect();
    let role_name = app.org.role(role).map_or(role.as_str(), |r| r.display_name());
    let draft = RuleDraft {
        id: format!("d{}", now_ms()),
        rule,
        source: format!("{who} 把「{excerpt}」交给了 {role_name}"),
        created_at_ms: now_ms(),
    };
    drafts.insert(0, draft.clone());
    drafts.truncate(MAX_DRAFTS);
    store_drafts(app, &drafts).await?;
    Ok(Some(draft))
}

fn build(app: &App, spec: RouterSpec) -> Result<Router, String> {
    if let Some(sem) = &spec.semantic {
        app.runtime
            .models()
            .resolve(&sem.model)
            .map_err(|e| format!("语义匹配的模型 {}：{e}", sem.model))?;
    }
    Router::new(spec, &app.org, &app.runtime.agents()).map_err(|e| e.to_string())
}

async fn load_drafts(app: &App) -> Result<Vec<RuleDraft>, String> {
    match app
        .runtime
        .store()
        .setting(DRAFTS_KEY)
        .await
        .map_err(|e| e.to_string())?
    {
        Some(v) => serde_json::from_value(v).map_err(|e| e.to_string()),
        None => Ok(Vec::new()),
    }
}

async fn store_drafts(app: &App, drafts: &[RuleDraft]) -> Result<(), String> {
    let value = serde_json::to_value(drafts).map_err(|e| e.to_string())?;
    app.runtime
        .store()
        .put_setting(DRAFTS_KEY, value)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}
