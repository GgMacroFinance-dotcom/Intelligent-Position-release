// 在线改主机的组织（m2-spec D-42、D-44，backlog F-23 ⑦）：岗位页改岗位、删岗位，改员工、删员工；
// 只有“管理”权限的设备看得到入口。跑在假主机上。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import 'harness.dart';

Map<String, Object?> _role(String id, String name, {String model = 'fake/m'}) => {
  'id': id,
  'name': name,
  'description': '$name的职责',
  'model': {'default': model, 'escalate': null},
  'budget_per_day_usd': null,
};

/// “管理”权限的设备，主机有两个岗位：macro（有员工）和 spare（没有员工）。
Harness _admin() {
  final h = Harness();
  h.host.handlers.addAll({
    'who_am_i': (_) => {
      'device': {'id': 'd1', 'name': '张三的电脑', 'scope': 'admin', 'paired_at_ms': 1, 'last_seen_ms': 1, 'online': true},
      'scope': 'admin',
    },
    'list_roles': (_) => [_role('macro', '宏观研究'), _role('spare', '备用')],
    'update_role': (req) => req['role'],
    'delete_role': (_) => null,
    'update_agent': (req) => {
      'id': req['agent_id'],
      'role': req['role'] ?? 'macro',
      'name': req['name'] ?? '宏观研究员',
      if ((req['celestial_style'] as String?)?.isNotEmpty ?? false) 'celestial_style': req['celestial_style'],
    },
    'delete_agent': (_) => null,
  });
  return h;
}

Future<void> _openRoles(Harness h, WidgetTester tester) async {
  await h.savePaired();
  await h.start(tester);
  await tester.tap(find.text('岗位'));
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('改岗位：整份发给主机，填错的预算不发；改完列表换上新的', (tester) async {
    final h = _admin();
    await _openRoles(h, tester);
    await tester.tap(find.byKey(const ValueKey('edit-role-macro')));
    await tester.pumpAndSettle();
    expect(find.text('编辑岗位 macro'), findsOneWidget);
    await tester.enterText(find.byKey(const ValueKey('role-budget')), '0');
    await tester.tap(find.byKey(const ValueKey('role-save')));
    await tester.pumpAndSettle();
    expect(find.textContaining('大于 0'), findsOneWidget);
    expect(h.host.requests.where((r) => r['type'] == 'update_role'), isEmpty);

    await tester.enterText(find.byKey(const ValueKey('role-model')), 'fake/big');
    await tester.enterText(find.byKey(const ValueKey('role-escalate')), 'fake/huge');
    await tester.enterText(find.byKey(const ValueKey('role-budget')), '2.5');
    await tester.tap(find.byKey(const ValueKey('role-approve')));
    await tester.tap(find.byKey(const ValueKey('role-save')));
    await tester.pumpAndSettle();
    final sent = h.host.requests.lastWhere((r) => r['type'] == 'update_role')['role'] as Map;
    expect(sent['id'], 'macro');
    expect(sent['name'], '宏观研究');
    expect(sent['description'], '宏观研究的职责');
    expect(sent['model'], {'default': 'fake/big', 'escalate': 'fake/huge'});
    expect(sent['budget_per_day_usd'], 2.5);
    expect(sent['approve_completion'], true);
    expect(sent.containsKey('coordinator'), isFalse, reason: '没勾协调岗位不带这一项');
    expect(find.text('编辑岗位 macro'), findsNothing, reason: '保存成功关框');
    final macro = h.state.roles.firstWhere((r) => r.id == 'macro');
    expect((macro.model, macro.budgetPerDayUsd, macro.approveCompletion), ('fake/big', 2.5, true));
    await h.stop(tester);
  });

  testWidgets('主机拒了：框不关，显示原因', (tester) async {
    final h = _admin();
    h.host.handlers['update_role'] = (_) => throw '数字员工 macro-1 正在处理工作，等这一轮做完再改';
    await _openRoles(h, tester);
    await tester.tap(find.byKey(const ValueKey('edit-role-macro')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('role-save')));
    await tester.pumpAndSettle();
    expect(find.textContaining('正在处理工作'), findsOneWidget);
    expect(find.text('编辑岗位 macro'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('改员工只发改了的字段；删员工、删没有员工的岗位都要确认', (tester) async {
    final h = _admin();
    await _openRoles(h, tester);
    expect(find.byKey(const ValueKey('delete-role-macro')), findsNothing, reason: '还有员工的岗位不给删');

    // 员工小铅笔：改名字和形态，岗位没动不发。
    final chip = find.byKey(const ValueKey('agent-chip-macro-1'));
    await tester.tap(find.descendant(of: chip, matching: find.byIcon(Icons.edit_outlined)));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const ValueKey('agent-name')), '小宏');
    await tester.tap(find.byKey(const ValueKey('agent-style')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('土星').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('agent-save')));
    await tester.pumpAndSettle();
    final upd = h.host.requests.lastWhere((r) => r['type'] == 'update_agent');
    expect(upd, containsPair('agent_id', 'macro-1'));
    expect(upd, containsPair('name', '小宏'));
    expect(upd, containsPair('celestial_style', 'saturn'));
    expect(upd.containsKey('role'), isFalse);
    expect(h.state.agents.single.name, '小宏');
    expect(h.state.celestialOverrides['macro-1']?.id, 'saturn');

    // 删员工：先确认。
    await tester.tap(find.descendant(of: chip, matching: find.byIcon(Icons.edit_outlined)));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('agent-delete')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('confirm-delete-agent')));
    await tester.pumpAndSettle();
    expect(h.host.requests.last, containsPair('type', 'delete_agent'));
    expect(h.state.agents, isEmpty);

    // 删没有员工的岗位：先确认。
    await tester.tap(find.byKey(const ValueKey('delete-role-spare')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('confirm-delete-role')));
    await tester.pumpAndSettle();
    expect(h.host.requests.last, containsPair('role_id', 'spare'));
    expect(h.state.roles.map((r) => r.id), isNot(contains('spare')));
    await h.stop(tester);
  });

  testWidgets('“日常”权限的设备看不到改组织的入口', (tester) async {
    final h = Harness();
    await _openRoles(h, tester);
    expect(h.state.canEditOrg, isFalse);
    expect(find.byKey(const ValueKey('edit-role-macro')), findsNothing);
    expect(
      find.descendant(of: find.byKey(const ValueKey('agent-chip-macro-1')), matching: find.byIcon(Icons.edit_outlined)),
      findsNothing,
    );
    await h.stop(tester);
  });
}
