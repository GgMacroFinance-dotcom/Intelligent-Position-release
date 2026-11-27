import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/protocol/models.dart';
import 'package:ip_app/src/ui/rework_note.dart';
import 'package:ip_app/src/ui/shell/agent_panel.dart';
import 'package:ip_app/src/ui/shell/completion_approval.dart';
import 'package:ip_app/src/ui/shell/desktop_shell.dart';
import 'package:ip_app/src/ui/shell/inbox_widget.dart';

import 'harness.dart';

void main() {
  testWidgets('主机的验收卡片（D-41）：解析员工、对话、任务，和待验收的任务去重；打回带上要改的', (tester) async {
    final h = Harness();
    h.host.handlers['list_approvals'] = (_) => [
      {
        'id': 'ap9',
        'question': '验收 宏观研究员 交回的任务「写 CPI 点评」',
        'options': [
          {'label': '通过', 'consequence': '任务结项'},
          {'label': '打回修改', 'consequence': '任务退回进行中'},
        ],
        'recommended': null,
        'reason': '初稿',
        'subject': {'kind': 'task_completion', 'agent_id': 'macro-1', 'task_id': 'k5', 'thread_id': 'tw'},
        'state': {'state': 'pending'},
        'created_at_ms': 2000,
      },
    ];
    h.host.handlers['list_tasks'] = (_) => [
      {
        'id': 'k5',
        'title': '写 CPI 点评',
        'description': '',
        'status': 'pending_approval',
        'role': 'macro',
        'assignee': 'macro-1',
        'thread_id': 'tw',
        'origin_thread_id': null,
        'result': '初稿',
        'depth': 1,
        'created_by': {'kind': 'human', 'name': '张三'},
        'created_at_ms': 1,
        'updated_at_ms': 2000,
        'due_ms': null,
        'needs_approval': true,
      },
    ];
    h.host.handlers['resolve_approval'] = (req) => {
      ...(h.host.handlers['list_approvals']!(req) as List).first as Map<String, dynamic>,
      'state': {'state': 'resolved', 'choice': req['choice'], 'by': '张三的手机', 'at_ms': 3000},
    };
    await h.savePaired();
    await h.start(tester);
    final card = h.state.approvals.single;
    expect(card.isTaskCompletion, isTrue);
    expect((card.subjectAgentId, card.subjectThreadId, card.subjectTaskId), ('macro-1', 'tw', 'k5'));
    expect(h.state.tasks.single.isPendingApproval, isTrue);
    final items = h.state.pendingCompletionApprovals;
    expect(items.length, 1, reason: '卡片和待验收的任务是同一件事，只算一条');
    expect(items.single.source, CompletionSource.backendApproval);
    expect(items.single.summary, '初稿');

    // 待办里点“打回修改”：先问要改什么，取消就不打回。
    await tester.tap(find.widgetWithText(OutlinedButton, '打回修改'));
    await tester.pumpAndSettle();
    expect(find.text('打回「验收 宏观研究员 交回的任务「写 CPI 点评」」'), findsOneWidget);
    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();
    expect(h.host.requests.where((r) => r['type'] == 'resolve_approval'), isEmpty);

    await tester.tap(find.widgetWithText(OutlinedButton, '打回修改'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const ValueKey('rework-note')), '  加上 PPI  ');
    await tester.tap(find.byKey(const ValueKey('rework-confirm')));
    await tester.pumpAndSettle();
    final req = h.host.requests.lastWhere((r) => r['type'] == 'resolve_approval');
    expect(req, containsPair('approval_id', 'ap9'));
    expect(req, containsPair('choice', 1));
    expect(req, containsPair('note', '加上 PPI'));
    await h.stop(tester);
  });

  testWidgets('打回时不写意见：照样打回，不带 note', (tester) async {
    final item = CompletionApproval(
      id: 'approval:ap1',
      agentId: 'macro-1',
      agentName: '小宏',
      taskTitle: '写周报',
      summary: '初稿',
      createdAtMs: 1,
      source: CompletionSource.backendApproval,
    );
    String? got = 'unset';
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (context) =>
              TextButton(onPressed: () async => got = await askReworkNote(context, item), child: const Text('打回')),
        ),
      ),
    );
    await tester.tap(find.text('打回'));
    await tester.pumpAndSettle();
    expect(find.textContaining('会发给小宏'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('rework-confirm')));
    await tester.pumpAndSettle();
    expect(got, '');
    // 本地演示项不弹框。
    final demo = CompletionApproval(
      id: 'demo:1',
      agentId: 'x',
      agentName: 'x',
      taskTitle: 't',
      summary: '',
      createdAtMs: 1,
      source: CompletionSource.localDemo,
    );
    final ctx = tester.element(find.text('打回'));
    expect(await askReworkNote(ctx, demo), '');
  });

  test('5A 语义隔离：仅采集 task_completion 审批与 pending_approval 任务，不吞并 agent_question', () {
    final approvals = [
      const Approval(
        id: 'q1',
        question: '这个参数按 5% 还是 10%？',
        options: [ApprovalOption('5%', ''), ApprovalOption('10%', '')],
        reason: '执行中途确认',
        subjectKind: 'agent_question',
        subjectAgentId: 'macro',
        subjectThreadId: 'th-q',
        createdAtMs: 1000,
      ),
      const Approval(
        id: 'c1',
        question: 'Q4 报告已写完请审批',
        options: [ApprovalOption('通过', ''), ApprovalOption('打回', '')],
        reason: '已整理 6 家竞品数据',
        subjectKind: 'task_completion',
        subjectAgentId: 'macro',
        subjectThreadId: 'th-1',
        createdAtMs: 2000,
      ),
    ];
    final tasks = [
      // 同员工同对话已被 backendApproval 采集，应自动去重。
      const Task(
        id: 't1',
        title: 'Q4 报告',
        description: '',
        status: 'pending_approval',
        assignee: 'macro',
        threadId: 'th-1',
        result: '草稿完成',
        createdAtMs: 1500,
        updatedAtMs: 1900,
      ),
      // 另一位员工仅有 pending_approval 任务，应自动被采集。
      const Task(
        id: 't2',
        title: '周报汇总',
        description: '',
        status: 'pending_approval',
        assignee: 'research',
        threadId: 'th-2',
        result: '本周 12 条进展已汇总完毕',
        createdAtMs: 1600,
        updatedAtMs: 3000,
      ),
      // 普通进行中任务不应进入待审批列表。
      const Task(
        id: 't3',
        title: '仍在调研中',
        description: '',
        status: 'in_progress',
        assignee: 'dev',
        createdAtMs: 1700,
        updatedAtMs: 2500,
      ),
    ];

    final list = CompletionApproval.collect(
      approvals: approvals,
      tasks: tasks,
      agentName: (id) => switch (id) {
        'macro' => '小宏',
        'research' => '小研',
        _ => id,
      },
    );

    expect(list.length, 2);
    expect(list[0].agentId, 'research');
    expect(list[0].source, CompletionSource.backendTask);
    expect(list[0].summary, '本周 12 条进展已汇总完毕');
    expect(list[1].agentId, 'macro');
    expect(list[1].source, CompletionSource.backendApproval);
    expect(list[1].taskTitle, 'Q4 报告已写完请审批');
  });

  testWidgets('1C + 2 + 3B + 4：完成待审批离轨停泊左中下、与左上角待办同源、点击开三栏窗审批后飞回轨道', (tester) async {
    final h = Harness();
    h.host.handlers['list_agents'] = (_) => [
      {'id': 'chief', 'role': 'coordinator', 'name': '小协'},
      {'id': 'macro', 'role': 'macro', 'name': '宏观策略高级研究员'},
      {'id': 'research', 'role': 'research', 'name': '小研'},
    ];
    h.host.handlers['list_approvals'] = (_) => const <Object>[];
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));

    expect(find.byType(DesktopShell), findsOneWidget);
    // 平时无待审批项：左中下停泊区完全隐藏。
    expect(find.byKey(const ValueKey('completion-approval-dock')), findsNothing);
    expect(h.state.inboxCount, 0);

    // 触发长名员工完成待审批：底栏胶囊保持等宽 222×38 仅显示固定“完成 · 待审批”不截断，上方浮层完整展示长名与快捷审批。
    h.state.triggerDemoCompletionApproval(
      agentId: 'macro',
      agentName: '宏观策略高级研究员',
      taskTitle: '宏观简报整理完成',
      summary: '已完成三季度数据汇总，等待审批。',
    );
    await tester.pumpAndSettle();

    final dockFinder = find.byKey(const ValueKey('completion-approval-dock'));
    final capsuleFinder = find.byKey(const ValueKey('approval-dock-capsule'));
    expect(dockFinder, findsOneWidget);
    expect(capsuleFinder, findsOneWidget);
    // 验证位于左下角连接胶囊右侧、贴近底栏，且与左侧连接胶囊严格等宽（222px）等高（38px）（1C 位置 1）。
    final dockRect = tester.getRect(dockFinder);
    expect(dockRect.left, closeTo(292.0, 1.0));
    expect(dockRect.width, closeTo(222.0, 1.0));
    expect(dockRect.height, closeTo(38.0, 1.0));
    expect(dockRect.bottom, closeTo(800 - 16, 1.0));
    expect(find.text('完成 · 待审批'), findsOneWidget);

    // 4 数据同源：左上角待办计数与黄灯分组立即出现同一条记录。
    final buckets = InboxBuckets.from(h.state);
    expect(buckets.completionApprovals.length, 1);
    expect(buckets.yellowCount, 1);
    expect(h.state.inboxCount, 1);
    expect(find.text('宏观简报整理完成'), findsOneWidget);

    // 点底栏胶囊立即向上展开浮层，完整展示长名「宏观策略高级研究员」，点行按 3B 打开三栏工作大窗。
    await tester.tap(capsuleFinder);
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('completion-approval-popover')), findsOneWidget);
    expect(find.text('宏观策略高级研究员'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('popover-completion-row-macro')));
    await tester.pumpAndSettle();
    expect(find.byType(AgentPanel), findsOneWidget);
    expect(find.byKey(const ValueKey('agent-completion-banner-macro')), findsOneWidget);

    // 点击「审批通过」：三栏窗、左上角待办与左中下停泊区同时同步清空，并记录通过时间以解除手动 pin。
    await tester.tap(find.byKey(const ValueKey('agent-completion-approve-macro')));
    await tester.pumpAndSettle();
    expect(h.state.pendingCompletionApprovals, isEmpty);
    expect(h.state.completionApprovedAtMs['macro'], isNotNull);
    expect(h.state.inboxCount, 0);
    expect(find.byKey(const ValueKey('agent-completion-banner-macro')), findsNothing);

    // Esc 收起窗口后，左中下停泊区已自动隐藏。
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('completion-approval-dock')), findsNothing);

    // 再测浮层极轻快捷审批（✓ / ↩）与双向同步（4）：展开浮层直接点「打回修改」，停泊区与待办立刻同步清空。
    h.state.triggerDemoCompletionApproval(agentId: 'research', agentName: '小研', taskTitle: '行业对比表初稿');
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('approval-dock-capsule')), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('approval-dock-capsule')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('popover-completion-reject-research')));
    await tester.pumpAndSettle();

    expect(h.state.pendingCompletionApprovals, isEmpty);
    expect(h.state.completionReworkAgents.contains('research'), isTrue);
    expect(find.byKey(const ValueKey('completion-approval-dock')), findsNothing);

    await h.stop(tester);
  });

  testWidgets('开窗顶栏星球按 太阳 → 内环 → 中环 → 外环 → 待审批 严格等距排列且水平对齐', (tester) async {
    final h = Harness();
    final now = DateTime.now().millisecondsSinceEpoch;
    h.host.handlers['list_agents'] = (_) => [
      {'id': 'chief', 'role': 'coordinator', 'name': '小协'},
      {'id': 'outer_a', 'role': 'r_out', 'name': '外环星'},
      {'id': 'mid_a', 'role': 'r_mid', 'name': '中环星'},
      {'id': 'inner_a', 'role': 'r_in', 'name': '内环星'},
      {'id': 'done_a', 'role': 'r_done', 'name': '待审星'},
    ];
    h.host.handlers['list_approvals'] = (_) => const <Object>[];
    h.host.handlers['list_tasks'] = (_) => [
      // 内环：blocked -> urgency 0.80 -> Track 0
      {
        'id': 't_in',
        'title': '受阻任务',
        'description': '',
        'status': 'blocked',
        'assignee': 'inner_a',
        'depth': 0,
        'created_by': {'kind': 'human', 'name': '人'},
        'created_at_ms': now - 1000,
        'updated_at_ms': now - 1000,
      },
      // 中环：in_progress -> urgency 0.30 -> Track 1
      {
        'id': 't_mid',
        'title': '进行中任务',
        'description': '',
        'status': 'in_progress',
        'assignee': 'mid_a',
        'depth': 0,
        'created_by': {'kind': 'human', 'name': '人'},
        'created_at_ms': now - 1000,
        'updated_at_ms': now - 1000,
      },
      // outer_a 无任务 -> urgency 0.0 -> Track 2（外环）
    ];
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));

    // 将 done_a 置为「完成 · 待审批」（Q1=A 应排在顶栏最右侧、外环之后）。
    h.state.triggerDemoCompletionApproval(agentId: 'done_a', agentName: '待审星', taskTitle: '交付完毕');
    await tester.pumpAndSettle();

    // 点太阳打开三栏工作大窗，进入窗顶天体横排态。
    await tester.tap(find.byKey(const ValueKey('sun')));
    await tester.pumpAndSettle();

    double centerX(String key) => tester.getCenter(find.byKey(ValueKey(key))).dx;
    double centerY(String key) => tester.getCenter(find.byKey(ValueKey(key))).dy;

    final xSun = centerX('sun');
    final xIn = centerX('planet-inner_a');
    final xMid = centerX('planet-mid_a');
    final xOut = centerX('planet-outer_a');
    final xDone = centerX('planet-done_a');

    // 1. 严格按 太阳 < 内环 < 中环 < 外环 < 待审批 排布。
    expect(xSun, lessThan(xIn));
    expect(xIn, lessThan(xMid));
    expect(xMid, lessThan(xOut));
    expect(xOut, lessThan(xDone));

    // 2. 非太阳行星中心点严格等距（Q2=A）。
    final pitch1 = xMid - xIn;
    final pitch2 = xOut - xMid;
    final pitch3 = xDone - xOut;
    expect(pitch2, closeTo(pitch1, 0.5));
    expect(pitch3, closeTo(pitch1, 0.5));

    // 3. 太阳与所有行星处于同一水平中线，且名字标签全部严格落在同一条水平基线上。
    final ySun = centerY('sun');
    expect(centerY('planet-inner_a'), closeTo(ySun, 0.5));
    expect(centerY('planet-mid_a'), closeTo(ySun, 0.5));
    expect(centerY('planet-outer_a'), closeTo(ySun, 0.5));
    expect(centerY('planet-done_a'), closeTo(ySun, 0.5));

    double sunLabelTop() => tester
        .getTopLeft(find.descendant(of: find.byKey(const ValueKey('sun-label-slot')), matching: find.text('小协')))
        .dy;
    double planetLabelTop(String id, String text) =>
        tester.getTopLeft(find.descendant(of: find.byKey(ValueKey('planet-item-$id')), matching: find.text(text))).dy;
    final ySunLabel = sunLabelTop();
    expect(planetLabelTop('inner_a', '内环星'), closeTo(ySunLabel, 0.5));
    expect(planetLabelTop('mid_a', '中环星'), closeTo(ySunLabel, 0.5));
    expect(planetLabelTop('outer_a', '外环星'), closeTo(ySunLabel, 0.5));
    expect(planetLabelTop('done_a', '待审星'), closeTo(ySunLabel, 0.5));

    await h.stop(tester);
  });

  testWidgets('跨 980px 宽窄屏缩放重建外壳后不复活已清空演示气泡，竖向缩放时缩小星球紧跟胶囊底边', (tester) async {
    final h = Harness();
    h.host.handlers['list_agents'] = (_) => [
      {'id': 'chief', 'role': 'coordinator', 'name': '小协'},
      {'id': 'macro', 'role': 'macro', 'name': '小宏'},
      {'id': 'research', 'role': 'research', 'name': '小研'},
    ];
    h.host.handlers['list_approvals'] = (_) => const <Object>[];
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));

    // 模拟已完成首轮演示注入，并触发一条待审批。
    h.state.demoCompletionApprovalsSeeded = true;
    h.state.triggerDemoCompletionApproval(agentId: 'macro', agentName: '小宏', taskTitle: '周报完成');
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('completion-approval-dock')), findsOneWidget);

    // 竖向拉高窗口：无需等待补间动画，缩小星球中心 Y 应立即与底部胶囊对齐。
    tester.view.physicalSize = const Size(1280, 920);
    await tester.pump();
    final dockRect = tester.getRect(find.byKey(const ValueKey('completion-approval-dock')));
    final planetCenter = tester.getCenter(find.byKey(const ValueKey('planet-macro')));
    expect(planetCenter.dy, closeTo(dockRect.center.dy, 1.0));

    // 缩窄跨过 980px 切换窄屏导航，再拉宽回 1280px：待审批仍保持、demoSeed 标记保持 true 不重复注入。
    tester.view.physicalSize = const Size(820, 800);
    await tester.pumpAndSettle();
    expect(find.byType(DesktopShell), findsNothing);
    expect(h.state.demoCompletionApprovalsSeeded, isTrue);

    tester.view.physicalSize = const Size(1280, 800);
    await tester.pumpAndSettle();
    expect(find.byType(DesktopShell), findsOneWidget);
    expect(h.state.demoCompletionApprovalsSeeded, isTrue);
    expect(find.byKey(const ValueKey('completion-approval-dock')), findsOneWidget);

    // 审批清空后再跨 980px 来回缩放：气泡保持隐藏，不会重新复活。
    await h.state.decideCompletionApproval(h.state.pendingCompletionApprovals.first, CompletionDecision.approve);
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('completion-approval-dock')), findsNothing);

    tester.view.physicalSize = const Size(820, 800);
    await tester.pumpAndSettle();
    tester.view.physicalSize = const Size(1280, 800);
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('completion-approval-dock')), findsNothing);
    expect(h.state.demoCompletionApprovalsSeeded, isTrue);

    await h.stop(tester);
  });
}
