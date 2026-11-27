use ip_core::{Agent, ModelSlot, OrgSpec, Role};

use crate::{Decision, RouteBy, RouteInput, Router, RouterError, RouterSpec, draft_rule, suggest_keywords};

fn org() -> (OrgSpec, Vec<Agent>) {
    let role = |id: &str, escalate: Option<&str>| Role {
        id: id.into(),
        name: None,
        description: format!("{id} 的职责"),
        model: ModelSlot {
            default: "ep/small".into(),
            escalate: escalate.map(String::from),
        },
        budget_per_day_usd: None,
        coordinator: false,
    };
    let agent = |id: &str, role: &str| Agent {
        id: id.into(),
        role: role.into(),
        name: id.into(),
    };
    (
        OrgSpec {
            roles: vec![role("macro", Some("ep/big")), role("ops", None)],
        },
        vec![
            agent("macro-1", "macro"),
            agent("macro-2", "macro"),
            agent("ops-1", "ops"),
        ],
    )
}

const SPEC: &str = r#"
rules:
  - id: 线上故障
    description: 服务器、部署相关的问题交给运维
    when: { any: [报错, 宕机, Deploy, "502"], none: [演练] }
    to: { role: ops }
  - id: 深度研报
    when: { all: [研报, 深度] }
    to: { role: macro, escalate: true }
  - id: 数据点评
    when: { regex: "(CPI|PPI|PMI)\\s*(同比|环比)?" }
    to: { role: macro }
  - id: 张三的专属
    when: { from: [张三] }
    to: { role: macro, agent: macro-2 }
"#;

fn router(spec: &str) -> Result<Router, RouterError> {
    let (org, agents) = org();
    Router::new(serde_saphyr::from_str::<RouterSpec>(spec).unwrap(), &org, &agents)
}

fn route(r: &Router, text: &str, author: Option<&str>) -> Decision {
    r.route(RouteInput {
        text,
        author,
        via: None,
    })
}

fn rule_of(d: &Decision) -> Option<&str> {
    match d {
        Decision::Routed { rule_id, .. } => rule_id.as_deref(),
        Decision::NeedsHuman { .. } => None,
    }
}

#[test]
fn first_matching_rule_wins_with_reason() {
    let r = router(SPEC).unwrap();
    let d = route(&r, "生产环境又报错了，顺便看下 CPI 同比", None);
    assert_eq!(rule_of(&d), Some("线上故障"));
    let Decision::Routed {
        reason, role, escalate, ..
    } = d
    else {
        panic!()
    };
    assert_eq!(role.as_str(), "ops");
    assert!(!escalate);
    assert_eq!(reason, "命中规则「线上故障」：含「报错」");

    let d = route(&r, "the deploy failed", None);
    assert_eq!(rule_of(&d), Some("线上故障"), "英文关键词不区分大小写");
    let d = route(&r, "宕机演练安排在周五", None);
    assert_ne!(rule_of(&d), Some("线上故障"), "含排除词不命中");
}

#[test]
fn all_regex_and_from_conditions() {
    let r = router(SPEC).unwrap();
    let d = route(&r, "写一篇深度研报", None);
    assert!(matches!(&d, Decision::Routed { escalate: true, .. }), "{d:?}");
    assert_eq!(rule_of(&route(&r, "写一篇研报", None)), None);

    let Decision::Routed { reason, .. } = route(&r, "点评一下本月 PPI 环比", None) else {
        panic!()
    };
    assert_eq!(reason, "命中规则「数据点评」：匹配「PPI 环比」");

    let d = route(&r, "帮我看看", Some("张三"));
    assert!(
        matches!(&d, Decision::Routed { agent: Some(a), .. } if a.as_str() == "macro-2"),
        "{d:?}"
    );
    assert!(matches!(
        route(&r, "帮我看看", Some("李四")),
        Decision::NeedsHuman { .. }
    ));
    assert!(matches!(route(&r, "帮我看看", None), Decision::NeedsHuman { .. }));
}

#[test]
fn channel_users_match_from_by_id_not_by_name() {
    let spec =
        format!("{SPEC}  - id: 老板的 Slack\n    when: {{ from: [\"slack:U0BOSS\"] }}\n    to: {{ role: ops }}\n");
    let r = router(&spec).unwrap();
    let via = |name: &'static str, key: &'static str| {
        r.route(RouteInput {
            text: "帮我看看",
            author: Some(name),
            via: Some(key),
        })
    };
    // 渠道用户把显示名改成“张三”，也命中不了只写了名字的规则。
    assert!(matches!(via("张三", "slack:U0MALLORY"), Decision::NeedsHuman { .. }));
    // 按用户 id 写的规则照常命中，改了显示名也一样。
    let Decision::Routed { role, reason, .. } = via("随便什么名字", "slack:U0BOSS") else {
        panic!()
    };
    assert_eq!(role.as_str(), "ops");
    assert_eq!(
        reason,
        "命中规则「老板的 Slack」：发送人是 随便什么名字（slack:U0BOSS）"
    );
    // 控制台用户仍按名字匹配。
    assert!(matches!(
        route(&r, "帮我看看", Some("张三")),
        Decision::Routed { agent: Some(a), .. } if a.as_str() == "macro-2"
    ));
}

#[test]
fn fallback_role() {
    let r = router(&format!("{SPEC}fallback: {{ role: ops }}\n")).unwrap();
    let d = route(&r, "随便聊聊", None);
    assert!(
        matches!(&d, Decision::Routed { rule_id: None, role, .. } if role.as_str() == "ops"),
        "{d:?}"
    );
}

#[test]
fn rejects_bad_rules() {
    let cases = [
        (
            "rules: [{ id: a, when: { any: [x] }, to: { role: nope } }]",
            "不存在的岗位",
        ),
        (
            "rules: [{ id: a, when: { any: [x] }, to: { role: ops, agent: macro-1 } }]",
            "员工不属于岗位",
        ),
        (
            "rules: [{ id: a, when: { any: [x] }, to: { role: ops, agent: ghost } }]",
            "不存在的员工",
        ),
        (
            "rules: [{ id: a, when: { any: [x] }, to: { role: ops, escalate: true } }]",
            "没有升级模型",
        ),
        (
            "rules: [{ id: a, when: { none: [x] }, to: { role: ops } }]",
            "只有排除条件",
        ),
        (
            "rules: [{ id: a, when: { any: [\"\"] }, to: { role: ops } }]",
            "空关键词",
        ),
        (
            "rules: [{ id: a, when: { regex: \"(\" }, to: { role: ops } }]",
            "坏正则",
        ),
        (
            "rules: [{ id: a, when: { any: [x] }, to: { role: ops } }, { id: a, when: { any: [y] }, to: { role: ops } }]",
            "重复 id",
        ),
        ("fallback: { role: nope }", "不存在的兜底岗位"),
    ];
    for (spec, why) in cases {
        assert!(router(spec).is_err(), "{why} 应被拒绝");
    }
    assert!(
        serde_saphyr::from_str::<RouterSpec>("rules: [{ id: a, when: { anyy: [x] }, to: { role: ops } }]").is_err()
    );
}

#[test]
fn empty_spec_asks_human() {
    let r = router("{}").unwrap();
    assert!(matches!(route(&r, "任何话", None), Decision::NeedsHuman { .. }));
}

#[test]
fn keywords_for_drafts() {
    assert_eq!(suggest_keywords("帮我看看 CPI 和 PPI 的走势"), ["CPI", "PPI", "走势"]);
    assert_eq!(suggest_keywords("服务器又挂了，nginx 报 502"), ["nginx", "服务器"]);
    assert!(suggest_keywords("好的，谢谢").len() <= 1);
    assert!(suggest_keywords("？！").is_empty());

    let rule = draft_rule("服务器又挂了", &"ops".into(), &["草案-服务器"]).unwrap();
    assert_eq!(rule.id, "草案-服务器-2");
    assert_eq!(rule.when.any, ["服务器"]);
    assert_eq!(rule.to.role, "ops".into());
    assert!(draft_rule("？", &"ops".into(), &[]).is_none());
}

#[test]
fn semantic_guesses() {
    use crate::semantic::{Guess, parse, prompt};
    let (org, _) = org();
    let (system, user) = prompt(&org, "美联储议息");
    assert!(
        system.contains("id: macro") && system.contains("macro 的职责"),
        "{system}"
    );
    assert!(user.contains("美联储议息"));

    let g = parse(r#"好的 {"role":"ops","confidence":1.5,"reason":"故障"} 以上"#, &org).unwrap();
    assert_eq!(g.role.as_ref().map(|r| r.as_str()), Some("ops"));
    assert_eq!(g.confidence, 1.0);
    assert_eq!(parse(r#"{"role":null,"confidence":0.9}"#, &org).unwrap().role, None);
    assert!(parse(r#"{"role":"hr","confidence":0.9}"#, &org).is_err());
    assert!(parse("没有 JSON", &org).is_err());

    let r = router("semantic: { model: ep/small }\nfallback: { role: ops }").unwrap();
    let sure = Guess {
        role: Some("macro".into()),
        confidence: 0.8,
        reason: String::new(),
    };
    assert!(
        matches!(r.after_guess(&sure), Decision::Routed { by: RouteBy::Semantic, ref role, .. } if role.as_str() == "macro")
    );
    let unsure = Guess {
        confidence: 0.5,
        ..sure
    };
    // 兜底是岗位时，把握不够就交给兜底岗位。
    assert!(
        matches!(r.after_guess(&unsure), Decision::Routed { by: RouteBy::Fallback, ref role, .. } if role.as_str() == "ops")
    );
    let r = router("semantic: { model: ep/small }").unwrap();
    assert!(
        matches!(r.after_guess(&unsure), Decision::NeedsHuman { suggested: Some(ref s), .. } if s.as_str() == "macro")
    );

    assert_eq!(
        router("semantic: { model: ep/small, min_confidence: 2 }").err(),
        Some(RouterError::BadConfidence(2.0))
    );
    assert_eq!(
        router("semantic: { model: ' ' }").err(),
        Some(RouterError::EmptySemanticModel)
    );
}
