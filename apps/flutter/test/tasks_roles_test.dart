// 任务页、岗位页（F-15）和顶栏的“我”，跑在假主机上。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/ui/shell/desktop_shell.dart';
import 'package:ip_app/src/ui/chat_page.dart';

import 'harness.dart';

int get now => DateTime.now().millisecondsSinceEpoch;

Map<String, Object?> task(String id, String title, {String status = 'open', int? due, String? result}) => {
  'id': id,
  'title': title,
  'description': '',
  'status': status,
  'role': 'macro',
  'assignee': 'macro-1',
  'thread_id': null,
  'origin_thread_id': null,
  'result': result,
  'depth': 1,
  'created_by': {'kind': 'human', 'name': '张三'},
  'created_at_ms': 1,
  'updated_at_ms': now - 1000,
  'due_ms': due,
};

Harness withTasks() {
  final h = Harness();
  final tasks = [
    task('k1', '9 月经营日报', due: now - 3600 * 1000),
    task('k2', '季度预缴核对', status: 'in_progress'),
    task('k3', '旧的点评', status: 'done', result: '写好了'),
  ];
  h.host.handlers.addAll({
    'list_tasks': (_) => tasks,
    'create_task': (req) {
      final t = task('k9', req['title'] as String, due: req['due_ms'] as int?)
        ..['description'] = req['description'] ?? ''
        ..['assignee'] = req['assignee'];
      tasks.insert(0, t);
      return t;
    },
  });
  return h;
}

void main() {
  testWidgets('任务页：未完成和已结束，过期的标出来', (tester) async {
    final h = withTasks();
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.text('任务'));
    await tester.pumpAndSettle();
    expect(find.text('未完成（2）'), findsOneWidget);
    expect(find.text('9 月经营日报'), findsOneWidget);
    expect(find.textContaining('已过期'), findsOneWidget);
    expect(find.text('旧的点评'), findsNothing);
    await tester.tap(find.text('已结束（1）'));
    await tester.pumpAndSettle();
    expect(find.text('旧的点评'), findsOneWidget);
    expect(find.text('9 月经营日报'), findsNothing);
    await h.stop(tester);
  });

  testWidgets('新建任务：标题、说明、负责人', (tester) async {
    final h = withTasks();
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.text('任务'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('新建任务'));
    await tester.pumpAndSettle();
    // 没写标题不能建。
    await tester.tap(find.widgetWithText(FilledButton, '新建'));
    await tester.pumpAndSettle();
    expect(find.text('请写任务标题'), findsOneWidget);

    await tester.enterText(find.widgetWithText(TextField, '标题'), '写 10 月 CPI 点评');
    await tester.enterText(find.widgetWithText(TextField, '说明（可选）'), '两页以内');
    await tester.tap(find.text('暂不指定负责人'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('宏观研究员（macro）').last);
    await tester.pumpAndSettle();
    expect(find.textContaining('另开一个对话'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, '新建'));
    await tester.pumpAndSettle();

    final req = h.host.requests.lastWhere((r) => r['type'] == 'create_task');
    expect(req, containsPair('title', '写 10 月 CPI 点评'));
    expect(req, containsPair('description', '两页以内'));
    expect(req, containsPair('assignee', 'macro-1'));
    expect(req.containsKey('due_ms'), isFalse);
    expect(req.containsKey('author'), isFalse);
    expect(req.containsKey('needs_approval'), isFalse, reason: '没勾“做完要我验收”不带这一项');
    expect(find.text('已新建任务：写 10 月 CPI 点评'), findsOneWidget);
    expect(find.text('未完成（3）'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('新建任务：指定了负责人才能勾“做完要我验收”（D-41）', (tester) async {
    final h = withTasks();
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.text('任务'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('新建任务'));
    await tester.pumpAndSettle();
    expect(find.text('做完要我验收'), findsNothing);
    await tester.enterText(find.widgetWithText(TextField, '标题'), '写 CPI 点评');
    await tester.tap(find.text('暂不指定负责人'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('宏观研究员（macro）').last);
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('new-task-needs-approval')));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, '新建'));
    await tester.pumpAndSettle();
    final req = h.host.requests.lastWhere((r) => r['type'] == 'create_task');
    expect(req, containsPair('needs_approval', true));
    await h.stop(tester);
  });

  testWidgets('等验收的任务显示“待验收”，选“已完成”交给主机当作验收通过', (tester) async {
    final h = withTasks();
    h.host.handlers['list_tasks'] = (_) => [task('k5', 'CPI 点评', status: 'pending_approval', result: '初稿')];
    h.host.handlers['update_task'] = (req) => task('k5', 'CPI 点评', status: req['status'] as String, result: '初稿');
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.text('任务'));
    await tester.pumpAndSettle();
    expect(find.text('待验收'), findsOneWidget);
    await tester.tap(find.text('待验收'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('已完成').last);
    await tester.pumpAndSettle();
    final req = h.host.requests.lastWhere((r) => r['type'] == 'update_task');
    expect(req, containsPair('task_id', 'k5'));
    expect(req, containsPair('status', 'done'));
    await h.stop(tester);
  });

  testWidgets('岗位页：今天的花费、缓存命中、模型、和员工对话', (tester) async {
    final h = withTasks();
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.text('岗位'));
    await tester.pumpAndSettle();
    // 假主机：宏观 0.0123 + 自动分派 0.0004；缓存 8000 / (10000 + 500)。
    // 和控制台一样，一分钱以上显示两位小数，不到一分钱显示四位。
    expect(find.textContaining(r'模型费用合计 $0.01，其中自动分派 $0.0004'), findsOneWidget);
    expect(find.textContaining('提示词缓存命中 76%'), findsOneWidget);
    expect(find.textContaining('宏观研究'), findsWidgets);
    expect(find.text('fake/m'), findsOneWidget);
    expect(find.text(r'$0.01'), findsOneWidget);
    expect(find.textContaining('80%（输入 10000 token'), findsOneWidget);

    await tester.tap(find.text('和 宏观研究员 对话'));
    await tester.pumpAndSettle();
    expect(find.byType(ThreadScreen), findsOneWidget);
    expect(h.state.threadId, isNull);
    expect(h.state.currentAgent, 'macro-1');
    await h.stop(tester);
  });

  testWidgets('一轮结束后刷新今天的花费', (tester) async {
    final h = withTasks();
    await h.savePaired();
    await h.start(tester);
    final before = h.host.requests.where((r) => r['type'] == 'spend_today').length;
    h.host.event({
      'type': 'turn_finished',
      'agent_id': 'macro-1',
      'thread_id': 't1',
      'steps': 1,
      'usage': {'prompt_tokens': 1, 'completion_tokens': 1, 'cached_tokens': 0, 'cache_write_tokens': 0},
      'cost_usd': 0.001,
      'elapsed_ms': 10,
    });
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(h.host.requests.where((r) => r['type'] == 'spend_today').length, before + 1);
    await h.stop(tester);
  });

  testWidgets('“我”在顶栏；忘掉主机后推进去的页面一起关掉，回到连接主机', (tester) async {
    final h = withTasks();
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    await h.scrollMe(tester, find.text('断开并忘掉这台主机'));
    expect(find.text('断开并忘掉这台主机'), findsOneWidget);
    await tester.tap(find.text('断开并忘掉这台主机'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, '断开并忘掉'));
    await tester.pumpAndSettle();
    expect(find.text('连接主机'), findsOneWidget);
    expect(find.text('断开并忘掉这台主机'), findsNothing);
    await h.stop(tester);
  });

  testWidgets('宽屏上“我”在导航栏底部', (tester) async {
    final h = withTasks();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    // 宽窗口：“我”在左下角，点了是设置弹窗。
    expect(find.byType(DesktopShell), findsOneWidget);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    await h.scrollMe(tester, find.text('断开并忘掉这台主机'));
    expect(find.text('断开并忘掉这台主机'), findsOneWidget);
    await h.stop(tester);
  });
}
