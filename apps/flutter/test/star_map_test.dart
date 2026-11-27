// Bot 星图：谁坐太阳、离太阳多远（紧急度）、图标多大（重要度）、用哪颗星。

import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/protocol/models.dart';
import 'package:ip_app/src/star_map.dart';

const now = 1_000_000_000;
const hour = 3600 * 1000;

Agent agent(String id, String role, String name) => Agent(id: id, role: role, name: name);

Task task(
  String id, {
  String? assignee,
  String status = 'open',
  int? due,
  String? by,
  String? thread,
  String title = '任务',
}) => Task(
  id: id,
  title: title,
  description: '',
  status: status,
  assignee: assignee,
  threadId: thread,
  createdByAgent: by,
  dueMs: due,
  createdAtMs: 0,
  updatedAtMs: 0,
);

Approval question(String id, String agentId, {String thread = 'tq'}) => Approval(
  id: id,
  question: '要不要把口径改成同比？',
  options: const [ApprovalOption('改', ''), ApprovalOption('不改', '')],
  reason: '',
  subjectKind: 'agent_question',
  subjectAgentId: agentId,
  subjectThreadId: thread,
  createdAtMs: 0,
);

final agents = [
  agent('coord-1', 'coordinator', '小协'),
  agent('macro-1', 'macro', '小宏'),
  agent('ops-1', 'ops', '小维'),
  agent('data-1', 'data', '小数'),
  agent('idle-1', 'data', '小闲'),
];

final roles = [
  const Role(id: 'coordinator', name: '协调员', description: '', budgetPerDayUsd: 10),
  const Role(id: 'macro', name: '宏观研究员', description: '', budgetPerDayUsd: 2),
  const Role(id: 'ops', name: '运维测试', description: ''),
  const Role(id: 'data', name: '数据分析', description: '', budgetPerDayUsd: 4),
];

StarMap build({
  List<Task> tasks = const [],
  List<Approval> approvals = const [],
  Set<String> working = const {},
  Map<String, String> failures = const {},
  List<Agent>? withAgents,
}) => buildStarMap(
  agents: withAgents ?? agents,
  roles: roles,
  tasks: tasks,
  approvals: approvals,
  working: working,
  failures: failures,
  now: now,
);

void main() {
  test('协调员坐在太阳的位置，其他人是行星', () {
    final map = build();
    expect(map.chief?.agent.id, 'coord-1');
    expect(map.chief?.celestial.id, 'sun');
    expect(map.bodies.map((b) => b.agent.id), isNot(contains('coord-1')));
    expect(map.bodies.every((b) => b.celestial.kind == CelestialKind.planet), isTrue);
  });

  test('岗位名或员工名里有“幕僚”也算；都没有时太阳空着', () {
    expect(isChiefOfStaff(agent('a', 'staff', '幕僚长'), null), isTrue);
    expect(isChiefOfStaff(agent('a', 'x', 'A'), const Role(id: 'x', name: '幕僚', description: '')), isTrue);
    expect(isChiefOfStaff(agent('a', 'chief-of-staff', 'A'), null), isTrue);
    expect(isChiefOfStaff(agent('a', 'macro', '小宏'), null), isFalse);
    final map = build(withAgents: [agent('macro-1', 'macro', '小宏')]);
    expect(map.chief, isNull);
    expect(map.bodies.single.agent.id, 'macro-1');
  });

  test('主机标了协调岗位（coordinator: true）就以它为准，不再按名字猜', () {
    StarMap mapOf(List<Role> roles, List<Agent> agents) => buildStarMap(
      agents: agents,
      roles: roles,
      tasks: const [],
      approvals: const [],
      working: const {},
      failures: const {},
      now: now,
    );
    const coordinator = Role(id: 'coordinator', name: '协调员', description: '');
    const macro = Role(id: 'macro', name: '宏观研究员', description: '');
    final staff = [
      agent('coord-1', 'coordinator', '小协'),
      agent('lead-1', 'lead', '老王'),
      agent('macro-1', 'macro', '小宏'),
    ];
    // 名字像协调员的岗位没标，标了的岗位名字不像：按标记来。
    final marked = mapOf([
      coordinator,
      const Role(id: 'lead', name: '牵头人', description: '', coordinator: true),
      macro,
    ], staff);
    expect(marked.chief?.agent.id, 'lead-1');
    expect(marked.bodies.map((b) => b.agent.id), containsAll(['coord-1', 'macro-1']));
    // 标了的岗位没有员工：太阳空着，不退回按名字猜。
    final empty = mapOf([
      coordinator,
      const Role(id: 'nobody', name: '第一组', description: '', coordinator: true),
    ], staff);
    expect(empty.chief, isNull);
    // 旧主机一个都没标：退回按名字猜。
    final old = mapOf([coordinator, const Role(id: 'lead', name: '牵头人', description: ''), macro], staff);
    expect(old.chief?.agent.id, 'coord-1');
  });

  test('越紧急离太阳越近：在问你 > 过期 > 受阻 > 正在处理 > 有任务 > 空闲', () {
    final map = build(
      approvals: [question('q1', 'macro-1')],
      tasks: [
        task('t1', assignee: 'ops-1', status: 'blocked', title: '修数据管道', thread: 'tops'),
        task('t2', assignee: 'data-1', due: now - hour, title: 'CPI 周报'),
      ],
      working: {'idle-1'},
    );
    final order = map.bodies.map((b) => b.agent.id).toList();
    expect(order, ['macro-1', 'data-1', 'ops-1', 'idle-1']);
    final byId = {for (final b in map.bodies) b.agent.id: b};
    expect(byId['macro-1']!.urgency, 1.0);
    expect(byId['macro-1']!.reasons.first.text, startsWith('在问你'));
    expect(byId['macro-1']!.focusThreadId, 'tq');
    expect(byId['data-1']!.reasons.first.text, '任务已过期：CPI 周报');
    expect(byId['ops-1']!.reasons.first.text, '任务受阻：修数据管道');
    expect(byId['ops-1']!.focusThreadId, 'tops');
    expect(byId['idle-1']!.reasons.single.text, '正在处理');
    // 紧急度越高，轨道越靠里。
    final radii = order.map((id) => orbitRadius(byId[id]!.urgency)).toList();
    expect(radii, orderedEquals([...radii]..sort()));
  });

  test('事情多的在档内靠近一点，但不跨档：几件快到期的任务不算“要你处理”', () {
    final map = build(
      tasks: [for (var i = 0; i < 6; i++) task('t$i', assignee: 'tax-1', due: now + hour)],
      withAgents: [...agents, agent('tax-1', 'data', '小税')],
    );
    final tax = map.bodies.firstWhere((b) => b.agent.id == 'tax-1');
    expect(tax.reasons.first.text, startsWith('24 小时内到期'));
    expect(tax.urgency, greaterThan(0.65));
    expect(tax.urgency, lessThan(urgencyBands.first.at));
  });

  test('空闲的员工在最外圈，理由写“空闲”', () {
    final idle = build().bodies.firstWhere((b) => b.agent.id == 'idle-1');
    expect(idle.urgency, 0);
    expect(idle.reasons.single.text, '空闲');
    expect(orbitRadius(idle.urgency), orbitRadius(0));
    expect(idle.focusThreadId, isNull);
  });

  test('上一轮失败、24 小时内到期也会拉近；做完的任务不算', () {
    final map = build(
      failures: {'macro-1': '模型超时'},
      tasks: [
        task('t1', assignee: 'ops-1', due: now + 2 * hour),
        task('t2', assignee: 'data-1', status: 'done', due: now - hour),
      ],
    );
    final byId = {for (final b in map.bodies) b.agent.id: b};
    expect(byId['macro-1']!.reasons.first.text, '上一轮失败了：模型超时');
    expect(byId['macro-1']!.urgency, closeTo(0.85, 1e-9));
    expect(byId['ops-1']!.reasons.first.text, startsWith('24 小时内到期'));
    expect(byId['data-1']!.urgency, 0);
  });

  test('越重要图标越大：人派的任务比员工派的重，预算高的岗位重', () {
    final map = build(
      tasks: [
        task('t1', assignee: 'macro-1'),
        task('t2', assignee: 'macro-1'),
        task('t3', assignee: 'ops-1', by: 'coord-1'),
      ],
    );
    final byId = {for (final b in map.bodies) b.agent.id: b};
    expect(byId['macro-1']!.importance, greaterThan(byId['ops-1']!.importance));
    expect(byId['ops-1']!.importance, greaterThan(byId['idle-1']!.importance));
    expect(bodyRadius(byId['macro-1']!.importance), greaterThan(bodyRadius(byId['idle-1']!.importance)));
    // 都没事时，预算高的岗位（数据分析 4 美元）比没设预算的（运维测试，按中等）大一点点。
    final quiet = {for (final b in build().bodies) b.agent.id: b};
    expect(quiet['data-1']!.importance, greaterThan(quiet['macro-1']!.importance));
    expect(quiet['idle-1']!.importanceNotes, contains('手上没有任务'));
  });

  test('员工派给员工、还没结束的任务连一条线；负责人在忙时线上有光点', () {
    final map = build(
      tasks: [
        task('t1', assignee: 'macro-1', by: 'coord-1', title: '写一份 CPI 点评'),
        task('t2', assignee: 'data-1', by: 'macro-1', status: 'done'),
        task('t3', assignee: 'ops-1'),
      ],
      working: {'macro-1'},
    );
    expect(map.links.single.from, 'coord-1');
    expect(map.links.single.to, 'macro-1');
    expect(map.links.single.active, isTrue);
    expect(map.neighbors('macro-1'), {'coord-1'});
    expect(map.neighbors('coord-1'), {'macro-1', 'ops-1', 'data-1', 'idle-1'});
  });

  test('路由拿不准的消息算在太阳上', () {
    final route = Approval(
      id: 'r1',
      question: '这条消息交给哪个岗位？',
      options: const [],
      reason: '',
      subjectKind: 'route_message',
      createdAtMs: 0,
    );
    expect(build(approvals: [route]).routingWaiting, 1);
  });

  group('天体', () {
    test('先用八大行星，用完了用恒星，不重复', () {
      final ids = [for (var i = 0; i < 12; i++) 'agent-$i'];
      final icons = assignCelestials(ids);
      expect(icons.values.map((c) => c.id).toSet(), hasLength(12));
      expect(icons.values.where((c) => c.kind == CelestialKind.planet), hasLength(8));
      expect(icons.values.where((c) => c.kind == CelestialKind.star), hasLength(4));
    });

    test('同一批员工每次拿到的都一样；加一位员工，大多数人不变', () {
      final ids = ['coord-1', 'macro-1', 'ops-1', 'data-1'];
      expect(
        assignCelestials(ids).map((k, v) => MapEntry(k, v.id)),
        assignCelestials(ids.reversed).map((k, v) => MapEntry(k, v.id)),
      );
      final before = assignCelestials(ids);
      final after = assignCelestials([...ids, 'seo-1']);
      final same = ids.where((id) => before[id]!.id == after[id]!.id).length;
      expect(same, greaterThanOrEqualTo(3));
    });

    test('八大行星和恒星的名字', () {
      expect(planets.map((p) => p.name), ['水星', '金星', '地球', '火星', '木星', '土星', '天王星', '海王星']);
      expect(stars.map((s) => s.name), containsAll(['天狼星', '织女星', '参宿四', '北极星']));
    });
  });
}
