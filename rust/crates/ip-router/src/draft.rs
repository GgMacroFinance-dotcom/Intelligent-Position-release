//! 从人工改派生成规则草案。
//!
//! 路由拿不准、人在卡片上选了岗位时，把这条消息里的关键词和所选岗位拼成一条草案，
//! 放进控制台等人修改、采纳或忽略。草案永远不会自动生效。

use ip_core::RoleId;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Condition, Rule, Target};

/// 一条待人审核的规则草案。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct RuleDraft {
    pub id: String,
    pub rule: Rule,
    /// 草案从哪来，例如“李四 把「服务器又挂了」交给了 运维测试”。
    pub source: String,
    #[ts(type = "number")]
    pub created_at_ms: i64,
}

/// 关键词最多取几个。
const MAX_KEYWORDS: usize = 3;

/// 中文里不适合当关键词的虚词和客套话，按它们切开句子。
const SPLIT_CHARS: &str =
    "的了吗呢吧啊呀是在和与及或把被给帮我你他她它们这那个一下请看看什么怎么为啥就都也还又要会能想让";

/// 用一条消息和人选的岗位拼一条草案。找不到像样的关键词时返回 `None`。
pub fn draft_rule(text: &str, role: &RoleId, taken_ids: &[&str]) -> Option<Rule> {
    let keywords = suggest_keywords(text);
    if keywords.is_empty() {
        return None;
    }
    let base = format!("草案-{}", keywords[0]);
    let mut id = base.clone();
    let mut n = 2;
    while taken_ids.contains(&id.as_str()) {
        id = format!("{base}-{n}");
        n += 1;
    }
    Some(Rule {
        id,
        description: String::new(),
        when: Condition {
            any: keywords,
            ..Condition::default()
        },
        to: Target {
            role: role.clone(),
            agent: None,
            escalate: false,
        },
    })
}

/// 粗略地挑关键词：英文和数字组成的词（如 CPI、K8s）优先，再取被虚词切开后 2–6 个字的中文片段，
/// 长的在前。只是给人改的起点，不追求准确分词。
pub fn suggest_keywords(text: &str) -> Vec<String> {
    let mut ascii: Vec<String> = Vec::new();
    let mut cjk: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut run = String::new();
    let flush_word = |w: &mut String, out: &mut Vec<String>| {
        if w.chars().count() >= 2 && !w.chars().all(|c| c.is_ascii_digit()) && !out.contains(w) {
            out.push(w.clone());
        }
        w.clear();
    };
    let flush_run = |r: &mut String, out: &mut Vec<String>| {
        let n = r.chars().count();
        if (2..=6).contains(&n) && !out.contains(r) {
            out.push(r.clone());
        }
        r.clear();
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            flush_run(&mut run, &mut cjk);
            word.push(c);
        } else if is_cjk(c) && !SPLIT_CHARS.contains(c) {
            flush_word(&mut word, &mut ascii);
            run.push(c);
        } else {
            flush_word(&mut word, &mut ascii);
            flush_run(&mut run, &mut cjk);
        }
    }
    flush_word(&mut word, &mut ascii);
    flush_run(&mut run, &mut cjk);
    cjk.sort_by_key(|s| std::cmp::Reverse(s.chars().count()));
    ascii.into_iter().chain(cjk).take(MAX_KEYWORDS).collect()
}

fn is_cjk(c: char) -> bool {
    matches!(c, '\u{4E00}'..='\u{9FFF}' | '\u{3400}'..='\u{4DBF}')
}
