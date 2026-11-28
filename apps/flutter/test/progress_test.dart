// 员工的进度和“要人看”的时间点（backlog F-24，m2-spec D-43）：解析、按时间往前推、时刻的写法，
// 以及星图头顶的进度条、气泡和员工面板里的依据、待办里的时间轴，都跑在假主机上。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/progress.dart';
import 'package:ip_app/src/protocol/wire.dart' as w2;
import 'package:ip_app/src/ui/shell/tokens.dart';

import 'fake_host.dart';
import 'harness.dart';
import 'star_map_page_test.dart' show settle, staffed;

Map<String, Object?> estimate({
  String scope = 'task',
  String thread = 'tcpi',
  String? task = 'k2',
  String title = '整理本周的 CPI',
  String phase = 'working',
  int started = 0,
  int done = 6,
  int? expected = 10,
  double? fraction = 0.6,
  int? eta,
  int? late,
  int samples = 5,
  String basis = '按宏观研究员最近 5 个做完的任务：中位数 8 步，每步约 1 秒',
}) => {
  'scope': scope,
  'thread_id': thread,
  'task_id': task,
  'title': title,
  'phase': phase,
  'started_ms': started,
  'steps_done': done,
  'steps_expected': expected,
  'fraction': fraction,
  'eta_ms': eta,
  'eta_late_ms': late,
  'samples': samples,
  'basis': basis,
};

Map<String, Object?> point(
  String kind,
  int at, {
  int? late,
  double? likelihood = 1,
  String text = '',
  String? agent,
  String? task,
  String? thread,
}) => {
  'kind': kind,
  'agent_id': agent,
  'role_id': null,
  'task_id': task,
  'thread_id': thread,
  'at_ms': at,
  'late_ms': late,
  'likelihood': likelihood,
  'text': text,
};

/// 宏观研究员在做“整理本周的 CPI”，运维在等你回答；幕僚长空闲。
Map<String, Object?> board(int now) {
  final cpi = estimate(eta: now + 240000, late: now + 300000);
  final ops = estimate(
    thread: 'tops',
    task: 'k1',
    title: '数据管道升级',
    phase: 'waiting_human',
    fraction: 0.4,
    done: 4,
    basis: '按运维测试最近 3 个做完的任务：中位数 10 步',
  );
  return {
    'now_ms': now,
    'agents': [
      {'agent_id': 'coord-1', 'phase': 'idle', 'headline': null, 'tasks': <Object>[]},
      {
        'agent_id': 'macro-1',
        'phase': 'working',
        'headline': cpi,
        'tasks': [cpi],
      },
      {
        'agent_id': 'ops-1',
        'phase': 'waiting_human',
        'headline': ops,
        'tasks': [ops],
      },
    ],
    'attention': [
      point('question', now - 30000, agent: 'ops-1', thread: 'tq', text: '小维 在问你：要不要重启？'),
      point('stalled', now - 200000, agent: 'ops-1', task: 'k9', thread: 'tstall', text: '小维 的任务停着：清理旧日志（没在做，也没在排队）'),
      point(
        'likely_question',
        now + 60000,
        late: now + 120000,
        likelihood: 0.67,
        agent: 'macro-1',
        task: 'k2',
        thread: 'tcpi',
        text: '小宏 可能会问你：整理本周的 CPI',
      ),
      point(
        'result',
        now + 240000,
        late: now + 300000,
        agent: 'macro-1',
        task: 'k2',
        thread: 'tcpi',
        text: '小宏 交回结果：整理本周的 CPI',
      ),
      point('budget', now + 1800000, likelihood: null, text: '按最近一小时的花费速度，宏观研究员今天的预算 \$2 会用完'),
    ],
  };
}

int _now() => DateTime.now().millisecondsSinceEpoch;

Harness withProgress() {
  final h = staffed();
  h.host.handlers['list_progress'] = (_) => board(_now());
  return h;
}

int progressRequests(Harness h) => h.host.requests.where((r) => r['type'] == 'list_progress').length;

void main() {
  group('解析和计算', () {
    // 主机的时钟比这台设备快 49 秒：表里的时刻都按主机的算。
    final b = ProgressBoard.fromJson({
      ...board(50000),
      'agents': [
        {
          'agent_id': 'a',
          'phase': 'working',
          'headline': estimate(fraction: 0.5, eta: 60000, late: 65000),
          'tasks': <Object>[],
        },
        {
          'agent_id': 'q',
          'phase': 'queued',
          'headline': estimate(phase: 'queued', fraction: 0.5, eta: 60000),
          'tasks': <Object>[],
        },
        {
          'agent_id': 'n',
          'phase': 'working',
          'headline': estimate(fraction: null, expected: null, eta: null, samples: 0),
          'tasks': <Object>[],
        },
      ],
    }, receivedAtMs: 1000);
    final a = b.of('a')!.headline!;

    test('在做的按时间往预计做完推，最多 95%；没在做的、估不出的不动', () {
      expect(b.hostTimeAt(1000), 50000);
      expect(b.toDevice(60000), 11000);
      expect(fractionAt(a, b, 1000), 0.5);
      expect(fractionAt(a, b, 6000), closeTo(0.75, 1e-9));
      expect(fractionAt(a, b, 20000), maxFraction);
      expect(fractionAt(b.of('q')!.headline!, b, 20000), 0.5);
      expect(fractionAt(b.of('n')!.headline!, b, 20000), isNull);
    });

    test('预计时刻的写法', () {
      expect(etaText(a, b, 1000), '约 10 秒');
      expect(etaText(a, b, 12000), '快好了');
      expect(etaText(a, b, 20000), '比预计久');
      expect(etaText(b.of('n')!.headline!, b, 1000), isNull);
      expect(durationText(150000), '3 分钟');
      expect(durationText(2 * 3600 * 1000), '2.0 小时');
    });

    test('时间轴上时刻的写法按本地时间', () {
      final now = DateTime(2026, 11, 28, 10, 0);
      final ms = now.millisecondsSinceEpoch;
      final local = ProgressBoard(hostNowMs: ms, receivedAtMs: ms, agents: const {}, attention: const []);
      AttentionItem at(DateTime t, [DateTime? late]) =>
          AttentionItem.fromWire(w(point('result', t.millisecondsSinceEpoch, late: late?.millisecondsSinceEpoch)));
      expect(attentionTimeText(at(now.subtract(const Duration(seconds: 10))), local, now), '现在');
      expect(attentionTimeText(at(now.add(const Duration(seconds: 10))), local, now), '马上');
      expect(attentionTimeText(at(DateTime(2026, 11, 28, 10, 20)), local, now), '约 10:20');
      expect(
        attentionTimeText(at(DateTime(2026, 11, 28, 10, 20), DateTime(2026, 11, 28, 10, 35)), local, now),
        '约 10:20–10:35',
      );
      expect(attentionTimeText(at(DateTime(2026, 11, 29, 9, 10)), local, now), '约 明天 09:10');
      expect(
        attentionTimeText(at(DateTime(2026, 11, 28, 23, 50), DateTime(2026, 11, 29, 0, 10)), local, now),
        '约 23:50–明天 00:10',
      );
    });

    test('可能性的写法，时间轴不重复别处已经列了的', () {
      final items = b.attention;
      expect(likelihoodText(items.firstWhere((p) => p.kind == AttentionKind.result)), isNull);
      expect(likelihoodText(items.firstWhere((p) => p.kind == AttentionKind.likelyQuestion)), '可能 67%');
      expect(likelihoodText(items.firstWhere((p) => p.kind == AttentionKind.budget)), '按速度估');
      expect(timelineItems(b).map((p) => p.kind), [
        AttentionKind.stalled,
        AttentionKind.likelyQuestion,
        AttentionKind.result,
        AttentionKind.budget,
      ]);
      expect(timelineItems(b, skip: {AttentionKind.stalled}).first.kind, AttentionKind.likelyQuestion);
      final routed = ProgressBoard.fromJson({
        'now_ms': 0,
        'agents': <Object>[],
        'attention': [
          point('unassigned', 0, thread: 't'),
          point('unassigned', 0, task: 'k', thread: 't'),
          point('review', 0, task: 'k'),
          point('blocked', 0, task: 'k'),
        ],
      }, receivedAtMs: 0);
      expect(timelineItems(routed).map((p) => p.taskId), ['k'], reason: '等你分派的消息在“等你决定”里，没负责人的任务才列');
    });
  });

  group('手机', () {
    setUp(() => Sizes.shellMin = 100000);
    tearDown(() => Sizes.shellMin = 980);

    testWidgets('星图头顶的进度条、气泡里的依据；待办里的时间轴，点了去对话', (tester) async {
      final h = withProgress();
      await h.savePaired();
      await h.start(tester, size: const Size(390, 844));

      // 待办：时间轴列在等你处理的事后面，员工提问不重复。
      expect(find.text('接下来要你看的（4）'), findsOneWidget);
      expect(find.text('小宏 交回结果：整理本周的 CPI'), findsOneWidget);
      expect(find.text('可能 67%'), findsOneWidget);
      expect(find.text('按速度估'), findsOneWidget);
      expect(find.text('现在'), findsOneWidget, reason: '停着的任务已经要看了');
      expect(find.text('小维 在问你：要不要重启？'), findsNothing);

      await tester.tap(find.text('星图'));
      await settle(tester);
      expect(find.byKey(const ValueKey('star-progress-macro-1')), findsOneWidget);
      expect(find.byKey(const ValueKey('star-progress-ops-1')), findsOneWidget);
      expect(find.byKey(const ValueKey('star-progress-coord-1')), findsNothing, reason: '空闲的不画');

      await tester.tap(find.byKey(const ValueKey('star-macro-1')));
      await settle(tester);
      expect(find.byKey(const ValueKey('progress-details-macro-1')), findsOneWidget);
      expect(find.text('手上的事 · 在做'), findsOneWidget);
      expect(find.text('按宏观研究员最近 5 个做完的任务：中位数 8 步，每步约 1 秒'), findsOneWidget);
      expect(find.textContaining('6/10 步'), findsOneWidget);
      await h.stop(tester);
    });

    testWidgets('点时间轴里的一项打开它的对话', (tester) async {
      final h = withProgress();
      await h.savePaired();
      await h.start(tester, size: const Size(390, 844));
      await tester.tap(find.text('小宏 交回结果：整理本周的 CPI'));
      await tester.pumpAndSettle();
      expect(h.state.threadId, 'tcpi');
      await h.stop(tester);
    });

    testWidgets('员工走了一步、一轮结束、任务变了以后防抖重拉；旧主机不画也不再拉', (tester) async {
      final h = withProgress();
      await h.savePaired();
      await h.start(tester, size: const Size(390, 844));
      expect(progressRequests(h), 1);

      // 一轮里接连几条消息只重拉一次。
      h.host.event({'type': 'turn_started', 'agent_id': 'macro-1', 'thread_id': 'tcpi'});
      for (var i = 0; i < 3; i++) {
        h.host.event({'type': 'message', 'message': message('m$i', 'tcpi', agent: 'macro-1', text: '第 $i 步')});
      }
      await tester.pump(const Duration(milliseconds: 500));
      expect(progressRequests(h), 1);
      await tester.pump(const Duration(milliseconds: 700));
      expect(progressRequests(h), 2);

      // 没有事件时每分钟也拉一次：停着的任务过一阵才算要人看。
      await tester.pump(const Duration(seconds: 61));
      await tester.pump(const Duration(seconds: 2));
      expect(progressRequests(h), 3);
      await h.stop(tester);

      final old = staffed();
      await old.savePaired();
      await old.start(tester, size: const Size(390, 844));
      expect(old.state.progressOnHost, isFalse);
      expect(find.textContaining('接下来要你看的'), findsNothing);
      old.host.event({'type': 'turn_started', 'agent_id': 'macro-1', 'thread_id': 'tcpi'});
      await tester.pump(const Duration(seconds: 2));
      expect(progressRequests(old), 1, reason: '旧主机没有这个请求，只试一次');
      await old.stop(tester);
    });
  });

  testWidgets('桌面：天穹上星球头顶的进度条，员工面板里的依据，待办里的“接下来要你看的”', (tester) async {
    final h = withProgress();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    expect(find.byKey(const ValueKey('planet-progress-macro-1')), findsOneWidget);
    expect(find.byKey(const ValueKey('planet-progress-ops-1')), findsOneWidget);
    expect(find.byKey(const ValueKey('planet-progress-coord-1')), findsNothing);

    await tester.tap(find.byKey(const ValueKey('inbox-widget-compact')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('inbox-widget-expanded')), findsOneWidget);
    expect(find.text('接下来要你看的'), findsOneWidget);
    expect(find.text('小宏 可能会问你：整理本周的 CPI'), findsOneWidget);
    await h.stop(tester);
  });
}

/// 测试里直接从 JSON 拼一项时间点。
w2.AttentionPoint w(Map<String, Object?> j) => w2.AttentionPoint.fromJson(j);
