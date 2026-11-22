// 路由规则的类型（F-18）。规则页要整套改完再存回去，所以这几个类型是可改的。
// JSON 进出都经过从 Rust 生成的类型（`wire.dart`，F-20）：RouterSpec、Rule、Condition、Target、Fallback、
// SemanticSpec、RulesView、RuleDraft、Decision。

import 'models.dart' show Json;
import 'wire.dart' as w;

/// 匹配条件：各项同时满足才算命中；关键词不区分英文大小写。
class Condition {
  Condition({List<String>? any, List<String>? all, List<String>? none, this.regex, List<String>? from})
    : any = any ?? [],
      all = all ?? [],
      none = none ?? [],
      from = from ?? [];

  factory Condition.fromWire(w.Condition c) =>
      Condition(any: [...c.any], all: [...c.all], none: [...c.none], regex: c.regex, from: [...c.from]);

  /// 含其中任意一个。
  List<String> any;

  /// 全部都含。
  List<String> all;

  /// 一个都不含。
  List<String> none;

  /// 正则表达式；为空时不用。
  String? regex;

  /// 发送人是其中之一（控制台用户写名字，渠道用户写 `<渠道>:<用户 id>`）。
  List<String> from;

  w.Condition toWire() =>
      w.Condition(any: any, all: all, none: none, regex: regex == null || regex!.isEmpty ? null : regex, from: from);

  /// 写给人看的一行，和控制台的 describeCondition 一样。
  String describe() => [
    if (any.isNotEmpty) '含任一：${any.join('、')}',
    if (all.isNotEmpty) '全部含：${all.join('、')}',
    if (none.isNotEmpty) '不含：${none.join('、')}',
    if (regex != null && regex!.isNotEmpty) '正则：$regex',
    if (from.isNotEmpty) '发送人：${from.join('、')}',
  ].join('；');
}

/// 交给谁。
class RouteTarget {
  RouteTarget({required this.role, this.agent, this.escalate = false});

  factory RouteTarget.fromWire(w.Target t) => RouteTarget(role: t.role, agent: t.agent, escalate: t.escalate);

  String role;

  /// 指定某个员工；为空时在岗位里挑当前最空闲的。
  String? agent;

  /// 用岗位配置里的升级模型处理。
  bool escalate;

  w.Target toWire() => w.Target(role: role, agent: agent, escalate: escalate);
}

/// 一条路由规则：满足 `when` 就交给 `to`。
class Rule {
  Rule({required this.id, this.description = '', Condition? when, required this.to}) : when = when ?? Condition();

  factory Rule.fromJson(Json j) => Rule.fromWire(w.Rule.fromJson(j));

  factory Rule.fromWire(w.Rule r) =>
      Rule(id: r.id, description: r.description, when: Condition.fromWire(r.when), to: RouteTarget.fromWire(r.to));

  String id;
  String description;
  Condition when;
  RouteTarget to;

  w.Rule toWire() => w.Rule(id: id, description: description, when: when.toWire(), to: to.toWire());

  /// 深拷贝：编辑时改的是副本，取消就丢掉。
  Rule copy() => Rule.fromWire(toWire());
}

/// 规则都没命中时用模型做语义匹配。
class SemanticSpec {
  SemanticSpec({required this.model, this.minConfidence = 0.7});

  factory SemanticSpec.fromWire(w.SemanticSpec s) =>
      SemanticSpec(model: s.model, minConfidence: s.minConfidence.toDouble());

  /// `<端点>/<模型>`。
  String model;

  /// 把握不低于这个值才直接分派，否则交给人并给出推荐。
  double minConfidence;

  w.SemanticSpec toWire() => w.SemanticSpec(model: model, minConfidence: minConfidence);
}

/// 一整套路由规则。`fallbackRole` 为空时兜底是“交给人决定”（`"ask"`）。
class RouterSpec {
  RouterSpec({List<Rule>? rules, this.fallbackRole, this.semantic}) : rules = rules ?? [];

  factory RouterSpec.fromJson(Json j) => RouterSpec.fromWire(w.RouterSpec.fromJson(j));

  factory RouterSpec.fromWire(w.RouterSpec s) => RouterSpec(
    rules: [for (final r in s.rules) Rule.fromWire(r)],
    fallbackRole: switch (s.fallback) {
      w.FallbackRole f => f.role,
      _ => null,
    },
    semantic: s.semantic == null ? null : SemanticSpec.fromWire(s.semantic!),
  );

  List<Rule> rules;
  String? fallbackRole;
  SemanticSpec? semantic;

  w.RouterSpec toWire() => w.RouterSpec(
    rules: [for (final r in rules) r.toWire()],
    fallback: fallbackRole == null ? const w.FallbackAsk() : w.FallbackRole(role: fallbackRole!),
    semantic: semantic?.toWire(),
  );

  Json toJson() => toWire().toJson();

  RouterSpec copy() => RouterSpec.fromWire(toWire());
}

/// `list_rules` 的回复。
class RulesView {
  const RulesView({required this.spec, required this.fromConsole});

  factory RulesView.fromJson(Json j) {
    final v = w.RulesView.fromJson(j);
    return RulesView(spec: RouterSpec.fromWire(v.spec), fromConsole: v.fromConsole);
  }

  final RouterSpec spec;

  /// 规则来自控制台保存（true）还是 ip.yaml（false）。
  final bool fromConsole;
}

/// 一条待人审核的规则草案。
class RuleDraft {
  const RuleDraft({required this.id, required this.rule, required this.source, required this.createdAtMs});

  factory RuleDraft.fromJson(Json j) {
    final d = w.RuleDraft.fromJson(j);
    return RuleDraft(id: d.id, rule: Rule.fromWire(d.rule), source: d.source, createdAtMs: d.createdAtMs.toInt());
  }

  final String id;
  final Rule rule;

  /// 草案从哪来，例如“李四 把「服务器又挂了」交给了 运维测试”。
  final String source;
  final int createdAtMs;
}

/// “试一试”的路由结果（`Decision`）。
class RouteDecision {
  const RouteDecision({
    required this.routed,
    required this.reason,
    this.role,
    this.agent,
    this.escalate = false,
    this.suggested,
  });

  factory RouteDecision.fromJson(Json j) => switch (w.Decision.fromJson(j)) {
    w.DecisionRouted d => RouteDecision(
      routed: true,
      reason: d.reason,
      role: d.role,
      agent: d.agent,
      escalate: d.escalate,
    ),
    w.DecisionNeedsHuman d => RouteDecision(routed: false, reason: d.reason, suggested: d.suggested),
    w.DecisionUnknown() => const RouteDecision(routed: false, reason: '这个版本不认识主机给的路由结果'),
  };

  /// false 时是“交给人决定”。
  final bool routed;
  final String reason;
  final String? role;
  final String? agent;
  final bool escalate;

  /// 交给人时，语义匹配给出、但把握不够的岗位。
  final String? suggested;
}
