//! 自动路由：把一条消息分派给岗位（以及可选的具体员工、是否升级模型）。
//!
//! 1. 规则：写在 `ip.yaml` 的 `router` 里或在控制台编辑，按顺序匹配，第一条命中的生效，
//!    人一眼能看懂为什么分给了谁。人写下的规则永远最先执行。
//! 2. 语义匹配（可选，`router.semantic`）：规则都没命中时请模型按岗位职责判断，见 [`semantic`]。
//!    把握够就直接分派；不够时把它的判断作为推荐项交给人。
//! 3. 兜底（`fallback`）：默认交给人决定（`ask`），也可以指定一个兜底岗位。
//!
//! 人在审批卡片上改派时会生成规则草案（[`draft_rule`]），人审核后才生效。

mod draft;
pub mod semantic;

use std::collections::{HashMap, HashSet};

pub use draft::{RuleDraft, draft_rule, suggest_keywords};
pub use semantic::SemanticSpec;

use ip_core::{Agent, AgentId, OrgSpec, RoleId};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// 正则编译后的大小上限，防止一条规则拖慢每条消息。
const REGEX_SIZE_LIMIT: usize = 1 << 20;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct RouterSpec {
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub fallback: Fallback,
    /// 规则都没命中时用模型做语义匹配；省略表示不用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub semantic: Option<SemanticSpec>,
}

/// 一条路由规则：满足 `when` 就交给 `to`。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct Rule {
    pub id: String,
    /// 写给人看的说明。
    #[serde(default)]
    pub description: String,
    pub when: Condition,
    pub to: Target,
}

/// 匹配条件。各项同时满足才算命中；关键词不区分英文大小写。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct Condition {
    /// 含其中任意一个。
    #[serde(default)]
    pub any: Vec<String>,
    /// 全部都含。
    #[serde(default)]
    pub all: Vec<String>,
    /// 一个都不含。
    #[serde(default)]
    pub none: Vec<String>,
    /// 正则表达式。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub regex: Option<String>,
    /// 发送人是其中之一。控制台用户写名字；外部渠道的用户写 `<渠道>:<用户 id>`（如 `slack:U012ABCDEF`），
    /// 渠道用户不按名字匹配，因为渠道里的显示名谁都能改。
    #[serde(default)]
    pub from: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export)]
pub struct Target {
    pub role: RoleId,
    /// 指定某个员工；省略时在岗位里挑当前最空闲的。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub agent: Option<AgentId>,
    /// 用岗位配置里的升级模型（`model.escalate`）处理。
    #[serde(default)]
    pub escalate: bool,
}

/// 没有规则命中时怎么办。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum Fallback {
    /// 交给人决定。
    #[default]
    Ask,
    /// 交给这个岗位。
    Role(RoleId),
}

/// 一次路由的输入。
#[derive(Clone, Copy, Debug)]
pub struct RouteInput<'a> {
    pub text: &'a str,
    /// 发送人的名字。
    pub author: Option<&'a str>,
    /// 发送人来自外部渠道时，`<渠道>:<用户 id>`（`Via::key`）。有它时 `from` 只按它匹配。
    pub via: Option<&'a str>,
}

/// 是怎么分派的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RouteBy {
    Rule,
    Semantic,
    Fallback,
}

/// 路由结果，附带写给人看的理由。
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum Decision {
    Routed {
        by: RouteBy,
        /// 命中的规则；不是按规则分派时为空。
        rule_id: Option<String>,
        role: RoleId,
        agent: Option<AgentId>,
        escalate: bool,
        reason: String,
    },
    NeedsHuman {
        reason: String,
        /// 语义匹配给出、但把握不够的岗位，作为卡片上的推荐项。
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        suggested: Option<RoleId>,
    },
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RouterError {
    #[error("规则 id `{0}` 重复")]
    DuplicateRule(String),
    #[error("规则 `{0}` 的 id 不能为空")]
    EmptyId(String),
    #[error("规则 `{0}` 没有任何匹配条件（any / all / regex / from 至少要有一项）")]
    NoCondition(String),
    #[error("规则 `{0}` 的关键词不能为空字符串")]
    EmptyKeyword(String),
    #[error("规则 `{0}` 的正则无效：{1}")]
    BadRegex(String, String),
    #[error("规则 `{0}` 指向不存在的岗位 `{1}`")]
    UnknownRole(String, String),
    #[error("规则 `{0}` 指向不存在的员工 `{1}`")]
    UnknownAgent(String, String),
    #[error("规则 `{0}` 指定的员工 `{1}` 不属于岗位 `{2}`")]
    AgentNotInRole(String, String, String),
    #[error("规则 `{0}` 要求升级模型，但岗位 `{1}` 没有配置 model.escalate")]
    NoEscalateModel(String, String),
    #[error("兜底岗位 `{0}` 不存在")]
    UnknownFallbackRole(String),
    #[error("语义匹配的模型名不能为空")]
    EmptySemanticModel,
    #[error("语义匹配的 min_confidence 必须在 0 到 1 之间，现在是 {0}")]
    BadConfidence(f64),
}

struct Compiled {
    any: Vec<String>,
    all: Vec<String>,
    none: Vec<String>,
    regex: Option<Regex>,
}

/// 校验并编译后的路由器。只读，可在多个任务间共享。
pub struct Router {
    spec: RouterSpec,
    compiled: Vec<Compiled>,
    /// 岗位显示名，写理由用。
    names: HashMap<RoleId, String>,
}

impl Router {
    /// 校验规则引用的岗位、员工都存在，编译正则。
    pub fn new(spec: RouterSpec, org: &OrgSpec, agents: &[Agent]) -> Result<Self, RouterError> {
        let mut seen = HashSet::new();
        let mut compiled = Vec::with_capacity(spec.rules.len());
        for rule in &spec.rules {
            let id = rule.id.trim();
            if id.is_empty() {
                return Err(RouterError::EmptyId(rule.description.clone()));
            }
            if !seen.insert(id) {
                return Err(RouterError::DuplicateRule(id.into()));
            }
            let w = &rule.when;
            if w.any.is_empty() && w.all.is_empty() && w.regex.is_none() && w.from.is_empty() {
                return Err(RouterError::NoCondition(id.into()));
            }
            if w.any
                .iter()
                .chain(&w.all)
                .chain(&w.none)
                .chain(&w.from)
                .any(|k| k.trim().is_empty())
            {
                return Err(RouterError::EmptyKeyword(id.into()));
            }
            let regex = match &w.regex {
                Some(r) => Some(
                    RegexBuilder::new(r)
                        .size_limit(REGEX_SIZE_LIMIT)
                        .build()
                        .map_err(|e| RouterError::BadRegex(id.into(), e.to_string()))?,
                ),
                None => None,
            };
            let Some(role) = org.role(&rule.to.role) else {
                return Err(RouterError::UnknownRole(id.into(), rule.to.role.to_string()));
            };
            if rule.to.escalate && role.model.escalate.is_none() {
                return Err(RouterError::NoEscalateModel(id.into(), role.id.to_string()));
            }
            if let Some(agent_id) = &rule.to.agent {
                let Some(agent) = agents.iter().find(|a| &a.id == agent_id) else {
                    return Err(RouterError::UnknownAgent(id.into(), agent_id.to_string()));
                };
                if agent.role != rule.to.role {
                    return Err(RouterError::AgentNotInRole(
                        id.into(),
                        agent_id.to_string(),
                        rule.to.role.to_string(),
                    ));
                }
            }
            let lower = |v: &[String]| v.iter().map(|k| k.to_lowercase()).collect::<Vec<_>>();
            compiled.push(Compiled {
                any: lower(&w.any),
                all: lower(&w.all),
                none: lower(&w.none),
                regex,
            });
        }
        if let Fallback::Role(role) = &spec.fallback
            && org.role(role).is_none()
        {
            return Err(RouterError::UnknownFallbackRole(role.to_string()));
        }
        if let Some(sem) = &spec.semantic {
            if sem.model.trim().is_empty() {
                return Err(RouterError::EmptySemanticModel);
            }
            if !(0.0..=1.0).contains(&sem.min_confidence) {
                return Err(RouterError::BadConfidence(sem.min_confidence));
            }
        }
        let names = org
            .roles
            .iter()
            .map(|r| (r.id.clone(), r.display_name().to_string()))
            .collect();
        Ok(Self { spec, compiled, names })
    }

    pub fn spec(&self) -> &RouterSpec {
        &self.spec
    }

    /// 只按规则匹配（不做语义匹配）：第一条命中的生效，都没命中时走兜底。
    pub fn route(&self, input: RouteInput<'_>) -> Decision {
        self.match_rules(input)
            .unwrap_or_else(|| self.fallback("没有规则命中", None))
    }

    /// 按顺序匹配规则，第一条命中的生效。
    pub fn match_rules(&self, input: RouteInput<'_>) -> Option<Decision> {
        let text = input.text.to_lowercase();
        self.spec.rules.iter().zip(&self.compiled).find_map(|(rule, c)| {
            let why = matches(rule, c, &text, input)?;
            Some(Decision::Routed {
                by: RouteBy::Rule,
                rule_id: Some(rule.id.clone()),
                role: rule.to.role.clone(),
                agent: rule.to.agent.clone(),
                escalate: rule.to.escalate,
                reason: format!("命中规则「{}」：{why}", rule.id),
            })
        })
    }

    /// 语义匹配的结论：把握够就分派，否则走兜底，并把判断作为推荐项留给人。
    pub fn after_guess(&self, guess: &semantic::Guess) -> Decision {
        let min = self.spec.semantic.as_ref().map_or(1.0, |s| s.min_confidence);
        let why = if guess.reason.is_empty() {
            String::new()
        } else {
            format!("：{}", guess.reason)
        };
        match &guess.role {
            Some(role) if guess.confidence >= min => Decision::Routed {
                by: RouteBy::Semantic,
                rule_id: None,
                role: role.clone(),
                agent: None,
                escalate: false,
                reason: format!(
                    "没有规则命中，语义匹配判断为{}（把握 {:.2}）{why}",
                    self.name(role),
                    guess.confidence
                ),
            },
            Some(role) => self.fallback(
                &format!(
                    "没有规则命中，语义匹配倾向{}但把握不够（{:.2}）{why}",
                    self.name(role),
                    guess.confidence
                ),
                Some(role.clone()),
            ),
            None => self.fallback(&format!("没有规则命中，语义匹配也拿不准{why}"), None),
        }
    }

    /// 兜底。`why` 说明为什么走到这里。
    pub fn fallback(&self, why: &str, suggested: Option<RoleId>) -> Decision {
        match &self.spec.fallback {
            Fallback::Ask => Decision::NeedsHuman {
                reason: format!("{why}，需要人来指定岗位"),
                suggested,
            },
            Fallback::Role(role) => Decision::Routed {
                by: RouteBy::Fallback,
                rule_id: None,
                role: role.clone(),
                agent: None,
                escalate: false,
                reason: format!("{why}，交给兜底岗位{}", self.name(role)),
            },
        }
    }

    fn name<'a>(&'a self, role: &'a RoleId) -> &'a str {
        self.names.get(role).map_or(role.as_str(), String::as_str)
    }
}

/// 命中时返回理由。
fn matches(rule: &Rule, c: &Compiled, text: &str, input: RouteInput<'_>) -> Option<String> {
    let mut why = Vec::new();
    if !rule.when.from.is_empty() {
        let who = match input.via {
            Some(key) => key,
            None => input.author?,
        };
        if !rule.when.from.iter().any(|f| f == who) {
            return None;
        }
        why.push(match (input.via, input.author) {
            (Some(key), Some(name)) => format!("发送人是 {name}（{key}）"),
            _ => format!("发送人是 {who}"),
        });
    }
    if !c.any.is_empty() {
        let (i, _) = c.any.iter().enumerate().find(|(_, k)| text.contains(k.as_str()))?;
        why.push(format!("含「{}」", rule.when.any[i]));
    }
    if !c.all.is_empty() {
        if !c.all.iter().all(|k| text.contains(k.as_str())) {
            return None;
        }
        why.push(format!("同时含「{}」", rule.when.all.join("」「")));
    }
    if c.none.iter().any(|k| text.contains(k.as_str())) {
        return None;
    }
    if let Some(re) = &c.regex {
        // 正则对原文匹配，大小写由正则自己决定（可写 `(?i)`）。
        let m = re.find(input.text)?;
        why.push(format!("匹配「{}」", m.as_str()));
    }
    Some(why.join("，"))
}

#[cfg(test)]
mod tests;
