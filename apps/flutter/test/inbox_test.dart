// 待办补齐和看报告（F-14），跑在假主机上。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/ui/report.dart';

import 'fake_host.dart';
import 'harness.dart';

Map<String, Object?> taskJson(
  String id,
  String title, {
  String status = 'blocked',
  String? result,
  int updated = 0,
  String? thread,
  String? origin,
}) => {
  'id': id,
  'title': title,
  'description': '',
  'status': status,
  'role': 'macro',
  'assignee': 'macro-1',
  'thread_id': thread,
  'origin_thread_id': origin,
  'result': result,
  'depth': 1,
  'created_by': {'kind': 'human', 'name': '张三'},
  'created_at_ms': 1,
  'updated_at_ms': updated,
  'due_ms': null,
};

int get now => DateTime.now().millisecondsSinceEpoch;

Harness withWork() {
  final h = Harness();
  final tasks = [
    taskJson('k1', '数据管道升级', result: '缺少只读数据库账号', updated: now - 1000, thread: 'tk1'),
    taskJson(
      'k2',
      '9 月 CPI 点评',
      status: 'done',
      result: '写好了，见 reports/2026-11-21-cpi.md。',
      updated: now - 2000,
      origin: 'to2',
    ),
    taskJson('k3', '去年的旧任务', status: 'done', updated: now - const Duration(days: 30).inMilliseconds),
  ];
  h.host.handlers.addAll({
    'list_approvals': (_) => <Object>[],
    'list_tasks': (_) => tasks,
    'update_task': (req) {
      final t = tasks.firstWhere((t) => t['id'] == req['task_id']);
      t['status'] = req['status'];
      t['updated_at_ms'] = now;
      return t;
    },
    'list_reports': (_) => [
      {'agent_id': 'macro-1', 'path': 'reports/2026-11-21-cpi.md', 'bytes': 100, 'modified_ms': now - 3000},
      {'agent_id': 'macro-1', 'path': 'reports/2025-01-01-old.md', 'bytes': 100, 'modified_ms': 1000},
    ],
    'read_report': (req) => {
      'agent_id': req['agent_id'],
      'path': req['path'],
      'text': '# 9 月 CPI 点评\n\n**结论**：同比 3.72%。\n\n上一份见 `reports/2026-10-21-cpi.md`。',
      'modified_ms': now - 3000,
      'truncated': false,
    },
  });
  return h;
}

void main() {
  testWidgets('受阻的任务能直接改状态', (tester) async {
    final h = withWork();
    await h.savePaired();
    await h.start(tester);
    expect(find.text('受阻的任务（1）'), findsOneWidget);
    expect(find.text('数据管道升级'), findsOneWidget);
    expect(find.text('受阻原因'), findsOneWidget);
    expect(find.text('缺少只读数据库账号'), findsOneWidget);
    expect(find.text('执行对话'), findsOneWidget);
    expect(find.descendant(of: find.byType(Badge), matching: find.text('1')), findsWidgets);

    await tester.tap(find.byType(DropdownButton<String>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('进行中').last);
    await tester.pumpAndSettle();
    final req = h.host.requests.lastWhere((r) => r['type'] == 'update_task');
    expect(req, containsPair('task_id', 'k1'));
    expect(req, containsPair('status', 'in_progress'));
    expect(req.containsKey('author'), isFalse);
    expect(find.text('受阻的任务（1）'), findsNothing);
    expect(find.text('没有等你处理的事。'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('最近 7 天完成的任务和新报告；报告在手机上整页打开', (tester) async {
    final h = withWork();
    await h.savePaired();
    await h.start(tester);
    expect(find.text('最近 7 天'), findsOneWidget);
    expect(find.text('9 月 CPI 点评'), findsOneWidget);
    expect(find.text('去年的旧任务'), findsNothing);
    await tester.scrollUntilVisible(find.text('2026-11-21-cpi.md'), 200, scrollable: find.byType(Scrollable).first);
    expect(find.text('2025-01-01-old.md'), findsNothing);

    // 完成的任务展开看结果，结果里提到的报告能点开。
    await tester.tap(find.text('9 月 CPI 点评'));
    await tester.pumpAndSettle();
    expect(find.textContaining('写好了'), findsOneWidget);

    await tester.tap(find.text('2026-11-21-cpi.md'));
    await tester.pumpAndSettle();
    expect(find.byType(ReportView), findsOneWidget);
    expect(find.textContaining('同比 3.72%'), findsOneWidget);
    final read = h.host.requests.lastWhere((r) => r['type'] == 'read_report');
    expect(read, containsPair('agent_id', 'macro-1'));
    expect(read, containsPair('path', 'reports/2026-11-21-cpi.md'));
    await tester.tap(find.byTooltip('关闭'));
    await tester.pumpAndSettle();
    expect(find.byType(ReportView), findsNothing);
    await h.stop(tester);
  });

  testWidgets('宽屏上报告在浮层里打开；消息里提到的报告能点开，按报告列表找写报告的人', (tester) async {
    final h = withWork();
    h.host.handlers['history'] = (req) => [
      // 协调员转述研究员的报告：hint 是协调员，列表里能找到真正写报告的研究员。
      message('m1', req['thread_id'] as String, agent: 'coord-1', text: 'reports/2026-11-21-cpi.md'),
    ];
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    await tester.tap(find.text('对话'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('最近 CPI 怎么样'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('reports/2026-11-21-cpi.md'));
    await tester.pumpAndSettle();
    expect(find.byType(Dialog), findsOneWidget);
    expect(find.textContaining('同比 3.72%'), findsOneWidget);
    expect(h.host.requests.lastWhere((r) => r['type'] == 'read_report'), containsPair('agent_id', 'macro-1'));
    await tester.tap(find.byTooltip('关闭'));
    await tester.pumpAndSettle();
    await h.stop(tester);
  });

  testWidgets('一轮结束后刷新报告列表', (tester) async {
    final h = withWork();
    await h.savePaired();
    await h.start(tester);
    final before = h.host.requests.where((r) => r['type'] == 'list_reports').length;
    h.host.event({
      'type': 'turn_finished',
      'agent_id': 'macro-1',
      'thread_id': 't1',
      'steps': 1,
      'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'cached_tokens': 0, 'cache_write_tokens': 0},
      'cost_usd': 0,
      'elapsed_ms': 10,
    });
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(h.host.requests.where((r) => r['type'] == 'list_reports').length, before + 1);
    await h.stop(tester);
  });
}
