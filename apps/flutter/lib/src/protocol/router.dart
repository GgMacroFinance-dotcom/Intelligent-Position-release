// 路由规则的类型（F-18），字段名和 rust/web/src/types/ 的 RouterSpec、Rule、Condition、Target、Fallback、
// SemanticSpec、RulesView、RuleDraft、Decision 一致。规则页要整套改完再存回去，所以这几个类型是可改的，
// 并且能原样转回 JSON（`toJson`）。

import 'models.dart' show Json;

List<String> _strings(Object? v) => [for (final e in (v as List? ?? const [])) e as String];

/// 匹配条件：各项同时满足才算命中；关键词不区分英文大小写。
class Condition {
  Condition({List<String>? any, List<String>? all, List<String>? none, this.regex, List<String>? from})
    : any = any ?? [],
      all = all ?? [],
      none = none ?? [],
      from = from ?? [];

  factory Condition.fromJson(Json j) => Condition(
    any: _strings(j['any']),
    all: _strings(j['all']),
    none: _strings(j['none']),
    regex: j['regex'] as String?,
    from: _strings(j['from']),
  );

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

  Json toJson() => {
    'any': any,
    'all': all,
    'none': none,
    if (regex != null && regex!.isNotEmpty) 'regex': regex,
    'from': from,
  };

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

  factory RouteTarget.fromJson(Json j) =>
      RouteTarget(role: j['role'] as String, agent: j['agent'] as String?, escalate: j['escalate'] as bool? ?? false);

  String role;

  /// 指定某个员工；为空时在岗位里挑当前最空闲的。
  String? agent;

  /// 用岗位配置里的升级模型处理。
  bool escalate;

  Json toJson() => {'role': role, 'agent': ?agent, 'escalate': escalate};
}

/// 一条路由规则：满足 `when` 就交给 `to`。
class Rule {
  Rule({required this.id, this.description = '', Condition? when, required this.to}) : when = when ?? Condition();

  factory Rule.fromJson(Json j) => Rule(
    id: j['id'] as String,
    description: j['description'] as String? ?? '',
    when: Condition.fromJson(j['when'] as Json? ?? const {}),
    to: RouteTarget.fromJson(j['to'] as Json),
  );

  String id;
  String description;
  Condition when;
  RouteTarget to;

  Json toJson() => {'id': id, 'description': description, 'when': when.toJson(), 'to': to.toJson()};

  /// 深拷贝：编辑时改的是副本，取消就丢掉。
  Rule copy() => Rule.fromJson(toJson());
}

/// 规则都没命中时用模型做语义匹配。
class SemanticSpec {
  SemanticSpec({required this.model, this.minConfidence = 0.7});

  factory SemanticSpec.fromJson(Json j) =>
      SemanticSpec(model: j['model'] as String, minConfidence: (j['min_confidence'] as num? ?? 0.7).toDouble());

  /// `<端点>/<模型>`。
  String model;

  /// 把握不低于这个值才直接分派，否则交给人并给出推荐。
  double minConfidence;

  Json toJson() => {'model': model, 'min_confidence': minConfidence};
}

/// 一整套路由规则。`fallbackRole` 为空时兜底是“交给人决定”（`"ask"`）。
class RouterSpec {
  RouterSpec({List<Rule>? rules, this.fallbackRole, this.semantic}) : rules = rules ?? [];

  factory RouterSpec.fromJson(Json j) {
    final fb = j['fallback'];
    return RouterSpec(
      rules: [for (final r in (j['rules'] as List? ?? const [])) Rule.fromJson(r as Json)],
      fallbackRole: fb is Map ? fb['role'] as String? : null,
      semantic: j['semantic'] == null ? null : SemanticSpec.fromJson(j['semantic'] as Json),
    );
  }

  List<Rule> rules;
  String? fallbackRole;
  SemanticSpec? semantic;

  Json toJson() => {
    'rules': [for (final r in rules) r.toJson()],
    'fallback': fallbackRole == null ? 'ask' : {'role': fallbackRole},
    if (semantic != null) 'semantic': semantic!.toJson(),
  };

  RouterSpec copy() => RouterSpec.fromJson(toJson());
}

/// `list_rules` 的回复。
class RulesView {
  const RulesView({required this.spec, required this.fromConsole});

  factory RulesView.fromJson(Json j) =>
      RulesView(spec: RouterSpec.fromJson(j['spec'] as Json), fromConsole: j['from_console'] as bool? ?? false);

  final RouterSpec spec;

  /// 规则来自控制台保存（true）还是 ip.yaml（false）。
  final bool fromConsole;
}

/// 一条待人审核的规则草案。
class RuleDraft {
  const RuleDraft({required this.id, required this.rule, required this.source, required this.createdAtMs});

  factory RuleDraft.fromJson(Json j) => RuleDraft(
    id: j['id'] as String,
    rule: Rule.fromJson(j['rule'] as Json),
    source: j['source'] as String? ?? '',
    createdAtMs: (j['created_at_ms'] as num? ?? 0).toInt(),
  );

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

  factory RouteDecision.fromJson(Json j) => RouteDecision(
    routed: j['kind'] == 'routed',
    reason: j['reason'] as String? ?? '',
    role: j['role'] as String?,
    agent: j['agent'] as String?,
    escalate: j['escalate'] as bool? ?? false,
    suggested: j['suggested'] as String?,
  );

  /// false 时是“交给人决定”。
  final bool routed;
  final String reason;
  final String? role;
  final String? agent;
  final bool escalate;

  /// 交给人时，语义匹配给出、但把握不够的岗位。
  final String? suggested;
}
