// Bot 星图的数据（backlog F-22，docs/ip/apps-plan.md 9.7）：幕僚长是中心的太阳，其他员工是行星和恒星。
// 离太阳越近越紧急，图标越大越重要。协议里没有“紧急度”“重要度”字段，这里用主机已有的数据算，
// 每一项都留下理由，界面上照原样给人看。

import 'dart:math' as math;

import 'protocol/models.dart';

/// 天体的种类：中心的太阳、八大行星、知名恒星。
enum CelestialKind { sun, planet, star }

/// 一个天体图标：名字和画它要的颜色。
class Celestial {
  const Celestial(this.id, this.name, this.english, this.kind, this.colors);

  final String id;
  final String name;
  final String english;
  final CelestialKind kind;

  /// 主色在前，画的时候按天体各取所需。ARGB。
  final List<int> colors;
}

const sun = Celestial('sun', '太阳', 'Sun', CelestialKind.sun, [0xFFFFF7CC, 0xFFFBBF24, 0xFFF97316]);

/// 八大行星，按离太阳的顺序。
const planets = [
  Celestial('mercury', '水星', 'Mercury', CelestialKind.planet, [0xFFB8B8BC, 0xFF6B6B70]),
  Celestial('venus', '金星', 'Venus', CelestialKind.planet, [0xFFF3E1B0, 0xFFC9A66B]),
  Celestial('earth', '地球', 'Earth', CelestialKind.planet, [0xFF2F6FE0, 0xFF22A055, 0xFFFFFFFF]),
  Celestial('mars', '火星', 'Mars', CelestialKind.planet, [0xFFE07A4A, 0xFF8F2F14]),
  Celestial('jupiter', '木星', 'Jupiter', CelestialKind.planet, [0xFFE8C9A2, 0xFFB07D56, 0xFFC2410C]),
  Celestial('saturn', '土星', 'Saturn', CelestialKind.planet, [0xFFEAD39A, 0xFFB8955A, 0xFFD9C79C]),
  Celestial('uranus', '天王星', 'Uranus', CelestialKind.planet, [0xFFA5EEF5, 0xFF3FB5C9]),
  Celestial('neptune', '海王星', 'Neptune', CelestialKind.planet, [0xFF4F86F7, 0xFF1E3A8A]),
];

/// 知名恒星，颜色照光谱型。员工多于八位时依次用。
const stars = [
  Celestial('sirius', '天狼星', 'Sirius', CelestialKind.star, [0xFFCFE3FF]),
  Celestial('vega', '织女星', 'Vega', CelestialKind.star, [0xFFDBE7FF]),
  Celestial('betelgeuse', '参宿四', 'Betelgeuse', CelestialKind.star, [0xFFFF8A50]),
  Celestial('rigel', '参宿七', 'Rigel', CelestialKind.star, [0xFF9CC4FF]),
  Celestial('arcturus', '大角星', 'Arcturus', CelestialKind.star, [0xFFFFB46B]),
  Celestial('antares', '心宿二', 'Antares', CelestialKind.star, [0xFFFF5A3C]),
  Celestial('polaris', '北极星', 'Polaris', CelestialKind.star, [0xFFFFF4D6]),
  Celestial('altair', '牛郎星', 'Altair', CelestialKind.star, [0xFFEEF4FF]),
  Celestial('aldebaran', '毕宿五', 'Aldebaran', CelestialKind.star, [0xFFFF9D5C]),
  Celestial('canopus', '老人星', 'Canopus', CelestialKind.star, [0xFFF5F3FF]),
  Celestial('deneb', '天津四', 'Deneb', CelestialKind.star, [0xFFE0ECFF]),
  Celestial('spica', '角宿一', 'Spica', CelestialKind.star, [0xFFA8C8FF]),
  Celestial('capella', '五车二', 'Capella', CelestialKind.star, [0xFFFFE7A3]),
  Celestial('alpha_centauri', '南门二', 'Alpha Centauri', CelestialKind.star, [0xFFFFF1C7]),
];

/// 一条理由：为什么在这个位置。`weight` 是这条理由给的紧急度（0 到 1）。
class Reason {
  const Reason(this.text, this.weight, {this.threadId});

  final String text;
  final double weight;

  /// 点这条理由去哪个对话。
  final String? threadId;
}

/// 星图上的一位员工。
class BotBody {
  BotBody({
    required this.agent,
    required this.celestial,
    required this.urgency,
    required this.importance,
    required this.reasons,
    required this.importanceNotes,
    required this.focusThreadId,
    required this.working,
  });

  final Agent agent;
  final Celestial celestial;

  /// 0（空闲）到 1（要你马上处理），决定离太阳多远。
  final double urgency;

  /// 0 到 1，决定图标多大。
  final double importance;

  /// 紧急度的理由，最要紧的在前。空闲时只有一条“空闲”。
  final List<Reason> reasons;

  /// 重要度的构成，给人看的。
  final List<String> importanceNotes;

  /// 点开它时打开哪个对话：在问你的那个、受阻或在做的任务的执行对话；没有时为空（开新对话）。
  final String? focusThreadId;

  /// 正在处理一轮。
  final bool working;
}

/// 两位员工之间的一条连线：派活的员工 → 负责人，来自还没结束的任务。
class BotLink {
  const BotLink(this.from, this.to, this.label, {required this.active});

  final String from;
  final String to;
  final String label;

  /// 负责人正在处理：线上跑数据流光点。
  final bool active;
}

/// 整张星图。
class StarMap {
  const StarMap({required this.chief, required this.bodies, required this.links, required this.routingWaiting});

  /// 坐在太阳位置的幕僚长；没配协调员岗位时为空，太阳显示成空位。
  final BotBody? chief;

  /// 其他员工，按紧急度从高到低。
  final List<BotBody> bodies;
  final List<BotLink> links;

  /// 路由拿不准、等人分派的消息数，算在太阳上。
  final int routingWaiting;

  BotBody? byId(String id) {
    if (chief?.agent.id == id) return chief;
    for (final b in bodies) {
      if (b.agent.id == id) return b;
    }
    return null;
  }

  /// 和这位员工有连线的员工（含幕僚长）。
  Set<String> neighbors(String id) {
    final out = <String>{};
    final c = chief?.agent.id;
    if (c != null && c != id) out.add(c);
    if (c == id) out.addAll(bodies.map((b) => b.agent.id));
    for (final l in links) {
      if (l.from == id) out.add(l.to);
      if (l.to == id) out.add(l.from);
    }
    return out;
  }
}

/// 紧急度分档，画成轨道。`at` 是这一档的下限。
const urgencyBands = [
  (at: 0.8, label: '要你处理'),
  (at: 0.6, label: '今天'),
  (at: 0.3, label: '进行中'),
  (at: 0.0, label: '空闲'),
];

/// 轨道半径：紧急度 1 在最里圈，0 在最外圈。单位是星图的世界坐标。
double orbitRadius(double urgency, {double inner = 130, double outer = 390}) =>
    inner + (1 - urgency.clamp(0.0, 1.0)) * (outer - inner);

/// 图标半径：重要度 0 到 1 对应 13 到 34。
double bodyRadius(double importance) => 13 + importance.clamp(0.0, 1.0) * 21;

/// 谁是幕僚长：主机标了协调岗位（`Role.coordinator`）时就是那个岗位的员工；
/// 一个都没标（旧主机，或 ip.yaml 没写）时退回猜：岗位 id 含 coordinator 或 chief，或者岗位名、员工名里有“协调”“幕僚”。
/// [explicit] 是“主机标过协调岗位”，由调用方对整个岗位表算一次。
bool isChiefOfStaff(Agent a, Role? role, {bool explicit = false}) {
  if (explicit) return role?.coordinator ?? false;
  final id = a.role.toLowerCase();
  if (id.contains('coordinator') || id.contains('chief')) return true;
  final names = '${role?.name ?? ''} ${a.name}';
  return names.contains('协调') || names.contains('幕僚');
}

/// 按员工 id 给天体：先用八大行星，用完了用恒星。按 id 排序后依次挑，id 的哈希决定从哪颗开始找，
/// 加了新员工时多数人的图标不变。
Map<String, Celestial> assignCelestials(Iterable<String> agentIds) {
  final ids = agentIds.toList()..sort();
  final taken = <String>{};
  final out = <String, Celestial>{};
  for (final id in ids) {
    final pool = taken.length < planets.length ? planets : stars;
    var i = _stableHash(id) % pool.length;
    var tries = 0;
    while (taken.contains(pool[i].id) && tries < pool.length) {
      i = (i + 1) % pool.length;
      tries++;
    }
    // 恒星也用完了：再从头轮一遍。
    final c = tries >= pool.length ? stars[out.length % stars.length] : pool[i];
    taken.add(c.id);
    out[id] = c;
  }
  return out;
}

int _stableHash(String s) {
  // FNV-1a，跨平台、跨启动都一样（String.hashCode 不保证）。
  var h = 0x811c9dc5;
  for (final c in s.codeUnits) {
    h ^= c;
    h = (h * 0x01000193) & 0xffffffff;
  }
  return h;
}

/// 从主机的数据算出星图。`now` 是 Unix 毫秒。
///
/// [reworkAgents]：最近刚被“打回修改”的员工集合（当前用于本地演示态；真实后端补齐
/// `pending_approval -> in_progress` 协议后会由任务状态自动驱动回活跃轨）。
/// [celestialOverrides]：用户手动指定的星球形态映射（`agentId -> Celestial`，严格排除太阳；
/// 当前由前端本地新建 Bot 维护，后续后端 `Agent` 补齐 `celestial_style` 字段后直接从此注入）。
/// [importanceOverrides]：用户通过滚轮或员工大窗手动调节的重要度权重（`agentId -> 0.0..1.0`，
/// 严格不作用于中心太阳；未手动覆盖时仍按任务负荷与岗位预算自动计算）。
/// 说明（5A）：`agent_question`（员工中途提问）仍按紧急度 1.0 留在内环；
/// 独立的「任务完成待审批」状态由 `completion_approval.dart` 提取并在宽窗口外壳中离轨停泊到左中下底栏。
StarMap buildStarMap({
  required List<Agent> agents,
  required List<Role> roles,
  required List<Task> tasks,
  required List<Approval> approvals,
  required Set<String> working,
  required Map<String, String> failures,
  required int now,
  Set<String> reworkAgents = const {},
  Map<String, Celestial> celestialOverrides = const {},
  Map<String, double> importanceOverrides = const {},
}) {
  final roleById = {for (final r in roles) r.id: r};
  final explicit = roles.any((r) => r.coordinator);
  Agent? chiefAgent;
  for (final a in agents) {
    // 明确手选了非太阳行星形态的员工不抢占中心太阳（幕僚长）席位。
    if (celestialOverrides.containsKey(a.id)) continue;
    if (isChiefOfStaff(a, roleById[a.role], explicit: explicit)) {
      chiefAgent = a;
      break;
    }
  }
  final others = agents.where((a) => a != chiefAgent).toList();
  final icons = assignCelestials(others.map((a) => a.id));
  final budgets = roles.map((r) => r.budgetPerDayUsd).whereType<double>().where((b) => b > 0).toList();
  final maxBudget = budgets.isEmpty ? 0.0 : budgets.reduce(math.max);
  final pending = approvals.where((a) => a.pending).toList();

  BotBody body(Agent a, Celestial c) {
    final mine = tasks
        .where((t) => t.isOpen && (t.assignee == a.id || (t.assignee == null && t.role == a.role)))
        .toList();
    final questions = pending.where((p) => p.isAgentQuestion && p.subjectAgentId == a.id).toList();
    final reasons = <Reason>[];
    for (final q in questions) {
      reasons.add(Reason('在问你：${_short(q.question)}', 1.0, threadId: q.subjectThreadId));
    }
    for (final t in mine) {
      final due = t.dueMs;
      if (due != null && due < now) {
        reasons.add(Reason('任务已过期：${_short(t.title)}', 0.9, threadId: t.threadId));
      } else if (t.status == 'blocked') {
        reasons.add(Reason('任务受阻：${_short(t.title)}', 0.8, threadId: t.threadId));
      } else if (due != null && due - now <= const Duration(hours: 24).inMilliseconds) {
        reasons.add(Reason('24 小时内到期：${_short(t.title)}', 0.65, threadId: t.threadId));
      }
    }
    final failure = failures[a.id];
    if (failure != null) reasons.add(Reason('上一轮失败了：${_short(failure)}', 0.85));
    final isWorking = working.contains(a.id);
    if (isWorking) reasons.add(const Reason('正在处理', 0.45));
    if (reworkAgents.contains(a.id)) reasons.add(const Reason('审批打回 · 修改中', 0.52));
    final ongoing = mine.where((t) => t.status == 'open' || t.status == 'in_progress').toList();
    if (ongoing.isNotEmpty) {
      reasons.add(Reason('有 ${ongoing.length} 件任务在做', 0.3, threadId: ongoing.first.threadId));
    }
    reasons.sort((x, y) => y.weight.compareTo(x.weight));
    // 最要紧的一条定档；事情多的在档内再靠近一点，但不跨进上一档：几件“快到期”加起来也不算“要你处理”。
    final urgency = reasons.isEmpty ? 0.0 : _withinBand(reasons.first.weight, 0.04 * (reasons.length - 1));
    if (reasons.isEmpty) reasons.add(const Reason('空闲', 0));

    // 重要度：手上的事（人派的任务 1、员工派的 0.5、在问你 1、受阻再加 0.5），按饱和曲线压到 0 到 1；
    // 加上岗位预算占最大预算的比例。手上没事的员工只剩岗位那一份，图标最小。
    var load = questions.length.toDouble();
    for (final t in mine) {
      load += t.createdByAgent == null ? 1.0 : 0.5;
      if (t.status == 'blocked') load += 0.5;
    }
    final work = 1 - math.exp(-load / 2);
    final budget = roleById[a.role]?.budgetPerDayUsd;
    final roleWeight = maxBudget > 0 && budget != null ? budget / maxBudget : 0.5;
    final autoImportance = (0.65 * work + 0.35 * roleWeight).clamp(0.0, 1.0);
    final manualImp = c.kind == CelestialKind.sun ? null : importanceOverrides[a.id];
    final importance = (manualImp ?? autoImportance).clamp(0.0, 1.0);
    final notes = [
      if (manualImp != null) '已手动调节重要度：${(importance * 100).round()}%（自动参考 ${(autoImportance * 100).round()}%）',
      if (load > 0)
        '手上的事：${_count(mine.length, '件任务')}${questions.isEmpty ? '' : '、${questions.length} 个提问'}（人派的任务权重更高）',
      if (load == 0) '手上没有任务',
      budget == null
          ? '岗位没设每日预算，按中等算'
          : '岗位每日预算 \$${_money(budget)}${maxBudget > 0 ? '（最高的是 \$${_money(maxBudget)}）' : ''}',
    ];

    final focus = reasons.firstWhere((r) => r.threadId != null, orElse: () => const Reason('', 0)).threadId;
    return BotBody(
      agent: a,
      celestial: c,
      urgency: urgency,
      importance: importance,
      reasons: reasons,
      importanceNotes: notes,
      focusThreadId: focus,
      working: isWorking,
    );
  }

  Celestial resolveCelestial(String id) {
    final override = celestialOverrides[id];
    if (override != null && override.kind != CelestialKind.sun) return override;
    return icons[id]!;
  }

  final bodies = [for (final a in others) body(a, resolveCelestial(a.id))]
    ..sort(
      (x, y) => y.urgency.compareTo(x.urgency) != 0 ? y.urgency.compareTo(x.urgency) : x.agent.id.compareTo(y.agent.id),
    );

  final ids = {for (final a in agents) a.id};
  final links = <BotLink>[
    for (final t in tasks)
      if (t.isOpen && t.createdByAgent != null && t.assignee != null && t.createdByAgent != t.assignee)
        if (ids.contains(t.createdByAgent) && ids.contains(t.assignee))
          BotLink(t.createdByAgent!, t.assignee!, _short(t.title, 14), active: working.contains(t.assignee)),
  ];

  return StarMap(
    chief: chiefAgent == null ? null : body(chiefAgent, sun),
    bodies: bodies,
    links: links,
    routingWaiting: pending.where((p) => p.subjectKind == 'route_message').length,
  );
}

/// 在 `weight` 所在的紧急度档里加上 `bonus`，最多到这一档的上沿（最高一档到 1）。
double _withinBand(double weight, double bonus) {
  var ceiling = 1.0;
  for (final b in urgencyBands) {
    if (weight >= b.at) break;
    ceiling = b.at - 0.01;
  }
  return math.min(weight + bonus, ceiling);
}

String _short(String s, [int max = 24]) {
  final one = s.replaceAll(RegExp(r'\s+'), ' ').trim();
  return one.length <= max ? one : '${one.substring(0, max)}…';
}

String _count(int n, String unit) => '$n $unit';

String _money(double v) => v == v.roundToDouble() ? v.toStringAsFixed(0) : v.toStringAsFixed(2);
