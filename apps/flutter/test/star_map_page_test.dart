// 星图页跑在假主机上：太阳是协调员、在问你的员工离太阳最近、点天体出气泡。

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/star_map.dart' show buildStarMap;
import 'package:ip_app/src/ui/chat_page.dart' show Composer, ThreadView;
import 'package:ip_app/src/ui/shell/agent_panel.dart';
import 'package:ip_app/src/ui/shell/agent_sheet.dart';
import 'package:ip_app/src/ui/shell/celestial_body.dart';
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

  testWidgets('主机记了最后回复的员工：没挂任务、审批的历史闲聊重启后也归到这位员工左栏', (tester) async {
    final h = staffed();
    h.host.handlers['list_threads'] = (_) => [
      {'thread_id': 'tchat', 'last_at_ms': 3000, 'message_count': 4, 'preview': '上周的闲聊', 'last_agent_id': 'macro-1'},
      // 刚发出、员工还没回：没有归属。旧主机不给这一项时也一样。
      {'thread_id': 'tnew', 'last_at_ms': 2000, 'message_count': 1, 'preview': '刚发的'},
    ];
    await h.savePaired();
    await h.start(tester, size: const Size(390, 844));
    expect(h.state.threadAgent['tchat'], 'macro-1');
    expect(h.state.threadAgent.containsKey('tnew'), isFalse);
    final map = buildStarMap(
      agents: h.state.agents,
      roles: h.state.roles,
      tasks: h.state.tasks,
      approvals: h.state.approvals,
      working: const {},
      failures: const {},
      now: 0,
    );
    final mine = agentThreads(h.state, map, 'macro-1');
    expect(mine.map((t) => t.threadId), contains('tchat'));
    expect(mine.firstWhere((t) => t.threadId == 'tchat').title, '上周的闲聊');
    expect(agentThreads(h.state, map, 'ops-1').map((t) => t.threadId), isNot(contains('tchat')));
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

  testWidgets('宽窗口新外壳：点太阳滑上来幕僚长的面板，内容和原来星图的气泡一样，[[关联]] 能换人', (tester) async {
    Sizes.shellMin = 980;
    final h = staffed();
    h.host.handlers.addAll({
      'who_am_i': (_) => {
        'device': {
          'id': 'd1',
          'name': '张三的 Mac',
          'scope': 'admin',
          'paired_at_ms': 1000,
          'last_seen_ms': 2000,
          'online': true,
        },
        'scope': 'admin',
      },
      'list_reports': (_) => [
        {'agent_id': 'ops-1', 'path': 'reports/2026-11-25-巡检.md', 'bytes': 64, 'modified_ms': 3000},
      ],
      'read_report': (req) => {
        'agent_id': 'ops-1',
        'path': req['path'],
        'text': '# 巡检报告\n一切正常。',
        'modified_ms': 3000,
        'truncated': false,
      },
      'list_schedules': (_) => [
        {
          'spec': {
            'id': 'morning-ops',
            'description': '每日巡检',
            'cron': '0 9 * * *',
            'utc_offset': '+08:00',
            'to': {'role': 'ops', 'agent': 'ops-1', 'escalate': false},
            'text': '跑一遍健康检查',
            'enabled': true,
          },
          'next_run_ms': 4000,
          'last_run': null,
        },
      ],
      'update_task': (req) => {
        'id': 'k1',
        'title': '数据管道升级',
        'description': '',
        'status': req['status'],
        'role': 'ops',
        'assignee': 'ops-1',
        'thread_id': 'tops',
        'origin_thread_id': null,
        'result': null,
        'depth': 1,
        'created_by': {'kind': 'human', 'name': '张三'},
        'created_at_ms': 1,
        'updated_at_ms': 3,
        'due_ms': null,
      },
    });
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 860));
    final sun = find.byKey(const ValueKey('sun'));
    await tester.tap(sun);
    await tester.pumpAndSettle();
    // 点太阳是幕僚长的 dusk bot 式界面：左栏它的对话，中间对话，右栏详情（概况是原来星图气泡的内容）。
    expect(find.byType(AgentPanel), findsOneWidget);
    expect(find.text('小协的对话'), findsOneWidget);
    expect(find.byType(ThreadView), findsOneWidget);
    // 单员工面板里的输入框不显示多余的“发给谁”下拉框。
    expect(find.descendant(of: find.byType(Composer), matching: find.byType(DropdownButton<String?>)), findsNothing);
    expect(find.byType(AgentSheetContent), findsOneWidget);
    expect(find.text('幕僚长在中心，统筹派活。'), findsOneWidget);
    expect(find.text('为什么这么大'), findsOneWidget);
    expect(find.text('模型'), findsOneWidget);
    expect(find.text('今日缓存命中'), findsOneWidget);
    // 待办、任务、岗位这些不在这里。
    expect(find.byKey(const ValueKey('panel-section-inbox')), findsNothing);

    // [[关联]] 点了换成那位员工：小宏在问你，审批卡片直接嵌在对话流里（右栏不再重复画一份）。
    await tester.tap(find.text('[[小宏]]'));
    await tester.pumpAndSettle();
    expect(find.text('小宏的对话'), findsOneWidget);
    expect(find.text('为什么在这条轨道上'), findsOneWidget);
    expect(find.textContaining('在问你：'), findsWidgets);
    expect(find.byType(ApprovalCard), findsOneWidget);
    expect(find.descendant(of: find.byType(ThreadView), matching: find.byType(ApprovalCard)), findsOneWidget);
    expect(find.byKey(const ValueKey('agent-thread-tq')), findsOneWidget);
    expect(h.state.threadId, 'tq');
    // 看的不是幕僚长，太阳不算选中。
    expect(tester.widget<CelestialBody>(sun).selected, isFalse);

    // 右栏另外两页：在做什么、高级。
    await tester.tap(find.byKey(const ValueKey('agent-tab-doing')));
    await tester.pumpAndSettle();
    expect(find.text('手上的任务'), findsOneWidget);
    expect(find.text('最近报告'), findsOneWidget);
    expect(find.byKey(const ValueKey('agent-new-task')), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('agent-tab-advanced')));
    await tester.pumpAndSettle();
    expect(find.textContaining('ip.yaml'), findsOneWidget);

    // 左栏“新对话”：开一个发给小宏的新对话。
    await tester.tap(find.byKey(const ValueKey('agent-new-chat')));
    await tester.pumpAndSettle();
    expect(h.state.threadId, isNull);
    expect(h.state.currentAgent, 'macro-1');

    // 右下角“面板”里点员工，也换成那位员工的界面：管理员能看到它的定时任务、任务改状态、最近报告。
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('panel-bubble')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('panel-agent-ops-1')));
    await tester.pumpAndSettle();
    expect(find.text('小维的对话'), findsOneWidget);
    expect(find.byKey(const ValueKey('agent-thread-tops')), findsOneWidget);
    // 概况里有属于小维的定时任务。
    expect(find.text('每日巡检'), findsOneWidget);
    expect(find.byKey(const ValueKey('agent-schedule-run-morning-ops')), findsOneWidget);

    // “在做什么”里可以直接改任务状态、打开这位员工的报告。
    await tester.tap(find.byKey(const ValueKey('agent-tab-doing')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('agent-task-status-k1')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('已完成').last);
    await tester.pumpAndSettle();
    final upd = h.host.requests.lastWhere((r) => r['type'] == 'update_task');
    expect(upd['task_id'], 'k1');
    expect(upd['status'], 'done');

    await tester.tap(find.byKey(const ValueKey('agent-report-reports/2026-11-25-巡检.md')));
    await tester.pumpAndSettle();
    expect(find.text('一切正常。'), findsOneWidget);

    // 关掉报告弹窗后测试窗顶横排星球无缝切换 + 星球形态图鉴。
    await tester.tap(find.byTooltip('关闭'));
    await tester.pumpAndSettle();
    final macroPlanet = find.byKey(const ValueKey('planet-macro-1'));
    final opsPlanet = find.byKey(const ValueKey('planet-ops-1'));
    expect(macroPlanet, findsOneWidget);
    expect(opsPlanet, findsOneWidget);
    final openSheet = tester.getRect(find.byType(AgentPanel));
    expect(tester.getRect(macroPlanet).bottom, lessThanOrEqualTo(openSheet.top));
    expect(tester.getRect(opsPlanet).bottom, lessThanOrEqualTo(openSheet.top));
    await tester.tap(macroPlanet);
    await tester.pumpAndSettle();
    expect(find.text('小宏的对话'), findsOneWidget);

    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byType(AgentPanel), findsNothing);
    await tester.tap(find.byKey(const ValueKey('create-bot-bubble')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('create-bot-modal')), findsOneWidget);
    expect(find.byKey(const ValueKey('create-bot-celestial-ringed')), findsOneWidget);
    expect(find.byKey(const ValueKey('create-bot-celestial-banded')), findsOneWidget);
    expect(find.byKey(const ValueKey('create-bot-celestial-oceanic')), findsOneWidget);
    expect(find.byKey(const ValueKey('create-bot-celestial-star')), findsOneWidget);
    expect(find.byKey(const ValueKey('create-bot-celestial-sun')), findsNothing);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('create-bot-modal')), findsNothing);
    await h.stop(tester);
  });
}
