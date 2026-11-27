//! 组织定义：岗位和数字员工。由人写 YAML，方案文档"自动路由机制"一节有示例。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{AgentId, RoleId};

/// 一个组织的全部岗位定义。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct OrgSpec {
    #[serde(default)]
    pub roles: Vec<Role>,
}

/// 一个岗位。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct Role {
    pub id: RoleId,
    /// 显示名；省略时用 id。
    #[serde(default)]
    pub name: Option<String>,
    /// 职责说明，也是语义路由比对的文本。
    pub description: String,
    pub model: ModelSlot,
    /// 每日模型费用上限（美元）。
    #[serde(default)]
    pub budget_per_day_usd: Option<f64>,
    /// 协调层岗位：接没有明确归属的需求，拆成任务派给执行岗位（App 星图中心的太阳）。最多一个岗位标这一项。
    /// 只是标记，不改这个岗位的工具和提示词。是 false 时不出现在协议里，旧主机也一样没有。
    #[serde(default, skip_serializing_if = "is_false")]
    #[ts(as = "Option<bool>", optional)]
    pub coordinator: bool,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// 岗位使用的模型。名字的格式是 `<端点>/<模型>`，本地岗位模型写成 `local/<基座>+<LoRA>`。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct ModelSlot {
    pub default: String,
    /// 高难度或高风险任务升级到的模型。
    #[serde(default)]
    pub escalate: Option<String>,
}

/// 一个数字员工：某个岗位的一个实例。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Agent {
    pub id: AgentId,
    pub role: RoleId,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OrgError {
    #[error("组织定义不是合法的 YAML：{0}")]
    Parse(String),
    #[error("岗位 id 重复：{0}")]
    DuplicateRole(RoleId),
    #[error("岗位 {0} 的 {1} 为空")]
    Empty(RoleId, &'static str),
    #[error("岗位 {0} 的每日预算必须大于 0")]
    BadBudget(RoleId),
    #[error("岗位 {0} 和 {1} 都标了 coordinator，最多只能有一个协调岗位")]
    TwoCoordinators(RoleId, RoleId),
}

impl OrgSpec {
    /// 解析并校验 YAML。
    pub fn from_yaml(text: &str) -> Result<Self, OrgError> {
        let spec: Self = serde_saphyr::from_str(text).map_err(|error| OrgError::Parse(error.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn validate(&self) -> Result<(), OrgError> {
        let mut seen = HashSet::new();
        let mut coordinator: Option<&RoleId> = None;
        for role in &self.roles {
            if role.id.as_str().trim().is_empty() {
                return Err(OrgError::Empty(role.id.clone(), "id"));
            }
            if !seen.insert(&role.id) {
                return Err(OrgError::DuplicateRole(role.id.clone()));
            }
            if role.description.trim().is_empty() {
                return Err(OrgError::Empty(role.id.clone(), "description"));
            }
            if role.model.default.trim().is_empty() {
                return Err(OrgError::Empty(role.id.clone(), "model.default"));
            }
            if role
                .budget_per_day_usd
                .is_some_and(|budget| budget.is_nan() || budget <= 0.0)
            {
                return Err(OrgError::BadBudget(role.id.clone()));
            }
            if role.coordinator {
                if let Some(first) = coordinator {
                    return Err(OrgError::TwoCoordinators(first.clone(), role.id.clone()));
                }
                coordinator = Some(&role.id);
            }
        }
        Ok(())
    }

    pub fn role(&self, id: &RoleId) -> Option<&Role> {
        self.roles.iter().find(|role| &role.id == id)
    }

    /// 标了 `coordinator` 的岗位。
    pub fn coordinator(&self) -> Option<&Role> {
        self.roles.iter().find(|role| role.coordinator)
    }
}

impl Role {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(self.id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
roles:
  - id: chief
    name: 幕僚长
    description: 接需求、拆任务、派活、汇总
    model: { default: anthropic/claude }
    coordinator: true
  - id: finance-ap
    name: 应付会计
    description: 应付账款：发票核对、付款申请
    model: { default: local/qwen-1b+lora-finance-ap, escalate: anthropic/claude }
    budget_per_day_usd: 5
  - id: hr-recruiting
    description: 招聘：筛简历、约面试
    model: { default: deepseek/deepseek-chat }
"#;

    #[test]
    fn parses_the_sample() {
        let spec = OrgSpec::from_yaml(SAMPLE).unwrap();
        assert_eq!(spec.roles.len(), 3);
        assert_eq!(spec.coordinator().map(|r| r.id.as_str()), Some("chief"));
        assert!(!spec.role(&"finance-ap".into()).unwrap().coordinator);
        let ap = spec.role(&"finance-ap".into()).unwrap();
        assert_eq!(ap.display_name(), "应付会计");
        assert_eq!(ap.model.escalate.as_deref(), Some("anthropic/claude"));
        assert_eq!(
            spec.role(&"hr-recruiting".into()).unwrap().display_name(),
            "hr-recruiting"
        );
    }

    #[test]
    fn rejects_duplicates_unknown_fields_and_bad_budgets() {
        let duplicate = "roles:\n  - {id: a, description: x, model: {default: m}}\n  - {id: a, description: y, model: {default: m}}\n";
        assert!(matches!(OrgSpec::from_yaml(duplicate), Err(OrgError::DuplicateRole(_))));
        let unknown = "roles:\n  - {id: a, description: x, model: {default: m}, salary: 1}\n";
        assert!(matches!(OrgSpec::from_yaml(unknown), Err(OrgError::Parse(_))));
        let budget = "roles:\n  - {id: a, description: x, model: {default: m}, budget_per_day_usd: 0}\n";
        assert!(matches!(OrgSpec::from_yaml(budget), Err(OrgError::BadBudget(_))));
        let two = "roles:\n  - {id: a, description: x, model: {default: m}, coordinator: true}\n  - {id: b, description: y, model: {default: m}, coordinator: true}\n";
        assert!(matches!(OrgSpec::from_yaml(two), Err(OrgError::TwoCoordinators(_, _))));
    }

    #[test]
    fn coordinator_is_left_out_of_json_when_false() {
        let spec = OrgSpec::from_yaml(SAMPLE).unwrap();
        let json = serde_json::to_value(&spec.roles).unwrap();
        assert_eq!(json[0]["coordinator"], serde_json::json!(true));
        assert!(json[1].get("coordinator").is_none(), "{json}");
        // 旧主机、旧客户端存下的 JSON 没有这一项：按 false 读。
        let old: Role = serde_json::from_value(serde_json::json!({
            "id": "a", "description": "x", "model": {"default": "m"}
        }))
        .unwrap();
        assert!(!old.coordinator);
    }
}
