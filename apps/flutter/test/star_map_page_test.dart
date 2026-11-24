// 星图页跑在假主机上：太阳是协调员、在问你的员工离太阳最近、点天体出气泡。

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/ui/shell/tokens.dart';
import 'package:ip_app/src/ui/inbox_page.dart';

import 'fake_host.dart';
import 'harness.dart';

Map<String, Object?> role(String id, String name, num? budget) => {
  'id': id,
  'name': name,
  'description': '',
  'model': {'default': 'fake/m'},
  'budget_per_day_usd': budget,
};

Harness staffed() {
  final h = Harness();
  h.host.handlers.addAll({
    'list_roles': (_) => [role('coordinator', '协调员', 10), role('macro', '宏观研究员', 2), role('ops', '运维测试', null)],
    'list_agents': (_) => [
      {'id': 'coord-1', 'role': 'coordinator', 'name': '小协'},
      {'id': 'macro-1', 'role': 'macro', 'name': '小宏'},
      {'id': 'ops-1', 'role': 'ops', 'name': '小维'},
    ],
    'list_approvals': (_) => [approval('q1', kind: 'agent_question', thread: 'tq')],
    'list_tasks': (_) => [
      {
        'id': 'k1',
        'title': '数据管道升级',
        'description': '',
        'status': 'in_progress',
        'role': 'ops',
        'assignee': 'ops-1',
        'thread_id': 'tops',
        'origin_thread_id': null,
        'result': null,
        'depth': 1,
        'created_by': {'kind': 'human', 'name': '张三'},
        'created_at_ms': 1,
        'updated_at_ms': 2,
        'due_ms': null,
      },
    ],
  });
  return h;
}

/// 星图一直在动，不能 pumpAndSettle。
Future<void> settle(WidgetTester tester) async {
  for (var i = 0; i < 6; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}

void main() {
  // 星图的测试按 1280 宽的视图写的；新外壳（>= Sizes.shellMin）里暂时没有星图，测试时把外壳的门槛调高。
  setUp(() => Sizes.shellMin = 100000);
  tearDown(() => Sizes.shellMin = 980);

  testWidgets('触控板的累计平移只应用一次，捏合和滚轮都围绕指针缩放', (tester) async {
    final h = staffed();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 860));
    await tester.tap(find.text('星图'));
    await settle(tester);
    final sun = find.byKey(const ValueKey('star-coord-1'));
    final start = tester.getCenter(sun);
    final size = tester.getSize(sun);
    final anchor = start + const Offset(80, 50);
    final pad = await tester.createGesture(kind: PointerDeviceKind.trackpad);
    await pad.panZoomStart(anchor);
    await pad.panZoomUpdate(anchor, pan: const Offset(2, 1));
    await tester.pump();
    expect(tester.getCenter(sun), offsetMoreOrLessEquals(start + const Offset(2, 1)));
    const pan = Offset(32, -20);
    await pad.panZoomUpdate(anchor, pan: pan);
    await tester.pump();
    expect(tester.getCenter(sun), offsetMoreOrLessEquals(start + pan));
    expect(tester.getSize(sun), size);
    await pad.panZoomUpdate(anchor, pan: pan, scale: 1.25);
    await pad.panZoomEnd();
    await tester.pump();
    final zoomed = anchor + pan + (start - anchor) * 1.25;
    expect(tester.getCenter(sun), offsetMoreOrLessEquals(zoomed));
    expect(tester.getSize(sun).width, closeTo(size.width * 1.25, 0.001));

    final mouse = TestPointer(12, PointerDeviceKind.mouse);
    await tester.sendEventToBinding(mouse.addPointer(location: anchor));
    await tester.sendEventToBinding(mouse.scroll(const Offset(0, -12)));
    await tester.pump();
    expect(tester.getCenter(sun), offsetMoreOrLessEquals(anchor + (zoomed - anchor) * math.exp(12 / 400)));
    await tester.sendEventToBinding(mouse.scroll(const Offset(0, 12)));
    await tester.pump();
    expect(tester.getCenter(sun), offsetMoreOrLessEquals(zoomed));
    await tester.sendEventToBinding(mouse.removePointer());
    await h.stop(tester);
  });

  testWidgets('协调员是太阳，在问你的员工离太阳最近，点开出气泡', (tester) async {
    final h = staffed();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 860));
    await tester.tap(find.text('星图'));
    await settle(tester);

    expect(find.text('小协 · 幕僚长'), findsOneWidget);
    expect(find.text('智能岗位 · Bot 星图'), findsOneWidget);
    final sunAt = tester.getCenter(find.byKey(const ValueKey('star-coord-1')));
    double dist(String id) => (tester.getCenter(find.byKey(ValueKey('star-$id'))) - sunAt).distance;
    // 小宏在问你（最紧急），小维有任务在做（进行中）。
    expect(dist('macro-1'), lessThan(dist('ops-1')));
    // 在问你的员工名字下面标出原因。
    expect(find.text('宏观研究员 · 在问你'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('star-macro-1')));
    await settle(tester);
    expect(find.text('为什么在这条轨道上'), findsOneWidget);
    expect(find.textContaining('在问你：'), findsWidgets);
    expect(find.byType(ApprovalCard), findsOneWidget);
    expect(find.textContaining('紧急度 100%'), findsOneWidget);
    // 打开的是它在问你的那个对话。
    expect(h.state.threadId, 'tq');
    expect(h.host.requests.where((r) => r['type'] == 'history').last['thread_id'], 'tq');

    // 点 [[关联]] 跳到幕僚长。
    await tester.tap(find.text('[[小协]]'));
    await settle(tester);
    expect(find.text('幕僚长在中心，统筹派活。'), findsOneWidget);

    await tester.tap(find.byTooltip('关闭气泡（Esc）'));
    await settle(tester);
    expect(find.text('幕僚长在中心，统筹派活。'), findsNothing);
    await h.stop(tester);
  });

  testWidgets('只有点中天体本身才打开气泡，外框的空白处不算', (tester) async {
    final h = staffed();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 860));
    await tester.tap(find.text('星图'));
    await settle(tester);
    final at = tester.getCenter(find.byKey(const ValueKey('star-ops-1')));
    // 外框至少 200 宽，天体本身小得多：点在旁边 70 像素处不算点中。
    await tester.tapAt(at + const Offset(70, 0));
    await settle(tester);
    expect(find.text('为什么在这条轨道上'), findsNothing);
    await tester.tapAt(tester.getCenter(find.byKey(const ValueKey('star-ops-1'))));
    await settle(tester);
    expect(find.text('为什么在这条轨道上'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('没有协调员时太阳空着，点开说明怎么配', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(390, 844));
    await tester.tap(find.text('星图'));
    await settle(tester);
    expect(find.text('幕僚长（空位）'), findsOneWidget);
    expect(find.text('宏观研究员'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('star-\u0000sun')));
    await settle(tester);
    expect(find.textContaining('主机上没有协调员岗位'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('搜索和岗位筛选、星图参数', (tester) async {
    final h = staffed();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 860));
    await tester.tap(find.text('星图'));
    await settle(tester);
    expect(find.text('全部 (3)'), findsOneWidget);
    expect(find.text('运维测试 (1)'), findsOneWidget);
    await tester.enterText(find.byType(TextField).first, '小维');
    await settle(tester);
    expect(find.byTooltip('清除搜索'), findsOneWidget);
    await tester.tap(find.text('星图参数'));
    await settle(tester);
    expect(find.text('公转动画'), findsOneWidget);
    await tester.tap(find.text('重新排布'));
    await settle(tester);
    await h.stop(tester);
  });
}
