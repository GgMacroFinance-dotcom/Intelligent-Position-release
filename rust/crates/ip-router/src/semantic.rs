//! 语义匹配：规则都没命中时，请一个模型按各岗位的职责说明判断这条消息该交给谁。
//!
//! 这里只负责写提示词和解析回答，调用模型由服务端完成（它持有模型端点）。
//! 模型要回答一个 JSON 对象：`{"role": "<岗位 id>", "confidence": 0~1, "reason": "..."}`，
//! 拿不准时 `role` 写 `null`。把握不低于阈值才直接分派，否则把它的判断作为推荐项交给人决定。

use ip_core::{OrgSpec, RoleId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 送给模型的消息最多保留这么多字，避免一条超长消息拖慢路由。
const MAX_TEXT_CHARS: usize = 2000;

/// `router.semantic` 配置。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct SemanticSpec {
    /// 用来判断的模型，写法同岗位模型：`<端点>/<模型>`。
    pub model: String,
    /// 把握不低于这个值才直接分派，否则交给人并给出推荐；默认 0.7。
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f64,
}

fn default_min_confidence() -> f64 {
    0.7
}

/// 模型的判断。
#[derive(Clone, Debug, PartialEq)]
pub struct Guess {
    /// 拿不准时为空。
    pub role: Option<RoleId>,
    pub confidence: f64,
    pub reason: String,
}

/// 系统提示词和用户消息。岗位列表放在系统提示词里，同一组织的请求前缀相同，利于缓存。
pub fn prompt(org: &OrgSpec, text: &str) -> (String, String) {
    let mut system = String::from(
        "你是一个消息分派器。根据下面各岗位的职责说明，判断用户这条消息应该交给哪个岗位处理。\n\
         只回答一个 JSON 对象，不要输出其他内容：\n\
         {\"role\": \"<岗位 id>\", \"confidence\": <0 到 1 之间的数>, \"reason\": \"<一句话理由>\"}\n\
         confidence 表示你有多大把握。没有合适的岗位或拿不准时，role 写 null。\n\n# 岗位\n",
    );
    for role in &org.roles {
        let description = role.description.split_whitespace().collect::<Vec<_>>().join(" ");
        system.push_str(&format!(
            "- id: {}；名称：{}；职责：{description}\n",
            role.id,
            role.display_name()
        ));
    }
    let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
    (system, format!("消息：\n{text}"))
}

/// 解析模型的回答。容忍回答前后带说明文字或代码块；岗位也可以写显示名。
pub fn parse(reply: &str, org: &OrgSpec) -> Result<Guess, String> {
    #[derive(Deserialize)]
    struct Raw {
        role: Option<String>,
        #[serde(default)]
        confidence: Option<f64>,
        #[serde(default)]
        reason: Option<String>,
    }
    let (Some(start), Some(end)) = (reply.find('{'), reply.rfind('}')) else {
        return Err(format!("回答里没有 JSON 对象：{}", excerpt(reply)));
    };
    if end < start {
        return Err(format!("回答里没有 JSON 对象：{}", excerpt(reply)));
    }
    let raw: Raw = serde_json::from_str(&reply[start..=end]).map_err(|e| format!("回答不是合法的 JSON：{e}"))?;
    let confidence = raw.confidence.unwrap_or(0.0).clamp(0.0, 1.0);
    let reason = raw.reason.unwrap_or_default().trim().to_string();
    let role = match raw.role.as_deref().map(str::trim) {
        None | Some("" | "null" | "none") => None,
        Some(r) => Some(
            org.roles
                .iter()
                .find(|x| x.id.as_str() == r || x.display_name() == r)
                .map(|x| x.id.clone())
                .ok_or_else(|| format!("模型给出了不存在的岗位 {r}"))?,
        ),
    };
    Ok(Guess {
        role,
        confidence,
        reason,
    })
}

fn excerpt(s: &str) -> String {
    let mut out: String = s.chars().take(80).collect();
    if s.chars().count() > 80 {
        out.push('…');
    }
    out
}
