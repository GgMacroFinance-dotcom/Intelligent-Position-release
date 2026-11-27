import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/protocol/models.dart';
import 'package:ip_app/src/star_map.dart';
import 'package:ip_app/src/ui/shell/agent_panel.dart';
import 'package:ip_app/src/ui/shell/planet_importance.dart';

import 'harness.dart';

class _RecordingImportanceSyncPort implements ImportanceSyncPort {
  final synced = <PlanetImportanceOverride>[];
  final cleared = <String>[];

  @override
  Future<void> syncOverride(PlanetImportanceOverride override) async => synced.add(override);

  @override
  Future<void> clearOverride(String agentId) async => cleared.add(agentId);
}

void main() {
  test('重要度半径曲线单调连续，顺位计算器确定性排布，预留同步与冲突反馈接口契约完备', () {
    expect(planetOrbitRadius(0.0), planetMinOrbitRadius);
    expect(planetOrbitRadius(1.0), planetMaxOrbitRadius);
    expect(planetOrbitRadius(0.5), greaterThan(planetOrbitRadius(0.2)));
    expect(planetOrbitRadius(1.0), lessThan(50.0)); // 始终克制在 50px 太阳之下

    final ov = const PlanetImportanceOverride(agentId: 'alpha', importance: 0.82, updatedAtMs: 2000);
    expect(PlanetImportanceOverride.fromJson(ov.toJson()), ov);
    expect(ov.toPlannedWireJson()['type'], 'set_agent_importance');
    expect(ov.toPlannedWireJson()['agent_id'], 'alpha');

    BotBody mk(String id, {double imp = 0.4, double urg = 0.2}) => BotBody(
      agent: Agent(id: id, role: 'r', name: id),
      celestial: planets.first,
      urgency: urg,
      importance: imp,
      reasons: const [Reason('空闲', 0)],
      importanceNotes: const [],
      focusThreadId: null,
      working: false,
    );

    final snap = PriorityAllocator.compute(
      bodies: [mk('a', imp: 0.4), mk('b', imp: 0.4), mk('c', imp: 0.3)],
      trackByAgent: const {'a': 2, 'b': 0, 'c': 2},
      overrides: {'c': const PlanetImportanceOverride(agentId: 'c', importance: 0.85, updatedAtMs: 5000)},
    );

    // c 手动放大到 85% 跃升为 #1；a 与 b 同为 40%，但 b 在内环（0）排在外环（2）的 a 前面。
    expect(snap.ordered.map((e) => e.agentId).toList(), ['c', 'b', 'a']);
    expect(snap.of('c')!.rank, 1);
    expect(snap.of('c')!.manual, isTrue);
    expect(snap.of('c')!.tier, PriorityTier.critical);
    expect(snap.of('b')!.rank, 2);
    expect(snap.of('a')!.rank, 3);
    final shareSum = snap.ordered.fold<double>(0.0, (s, e) => s + e.resourceShare);
    expect(shareSum, closeTo(1.0, 1e-6));
    expect(snap.of('c')!.resourceShare, greaterThan(snap.of('b')!.resourceShare));

    // 预留的冲突反馈端口默认安静（不触发任何模拟告警）。
    const noopContention = NoopContentionFeedbackPort();
    expect(noopContention.current, isNull);
  });

  testWidgets('方案 A：悬停天穹星球滚轮平滑放大缩小并短暂显示顺位，太阳不缩放，三栏左栏可调节与一键复位', (tester) async {
    final h = Harness();
    final syncPort = _RecordingImportanceSyncPort();
    h.state.importanceSyncPort = syncPort;
    h.host.handlers['list_roles'] = (_) => [
      {
        'id': 'coordinator',
        'name': '协调员',
        'description': '统筹',
        'model': {'default': 'openai/gpt-6-astra', 'escalate': null},
        'budget_per_day_usd': 10.0,
      },
      {
        'id': 'research',
        'name': '研究',
        'description': '研报',
        'model': {'default': 'openai/gpt-6-luna', 'escalate': null},
        'budget_per_day_usd': 4.0,
      },
      {
        'id': 'ops',
        'name': '运营',
        'description': '运维',
        'model': {'default': 'openai/gpt-6-luna', 'escalate': null},
        'budget_per_day_usd': 2.0,
      },
    ];
    h.host.handlers['list_agents'] = (_) => [
      {'id': 'chief', 'role': 'coordinator', 'name': '小协'},
      {'id': 'alpha', 'role': 'research', 'name': '小研'},
      {'id': 'beta', 'role': 'ops', 'name': '小运'},
    ];
    h.host.handlers['list_approvals'] = (_) => const <Object>[];
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));

    final betaFinder = find.byKey(const ValueKey('planet-beta'));
    expect(betaFinder, findsOneWidget);
    final betaCenter = tester.getCenter(betaFinder);

    // 向上滚轮放大 beta（scrollDelta.dy < 0）
    final pointer = TestPointer(1, PointerDeviceKind.mouse);
    await tester.sendEventToBinding(pointer.hover(betaCenter));
    await tester.pump();
    await tester.sendEventToBinding(pointer.scroll(const Offset(0, -180)));
    await tester.pump();
    await tester.sendEventToBinding(pointer.scroll(const Offset(0, -180)));
    await tester.pump();

    expect(h.state.hasManualImportance('beta'), isTrue);
    final zoomedImp = h.state.manualImportanceFor('beta')!.importance;
    expect(zoomedImp, greaterThan(0.25));

    // 滚动当下出现极轻瞬态顺位提示“重要度 XX% · 顺位 #N”
    final pillFinder = find.byKey(const ValueKey('planet-zoom-pill-beta'));
    expect(pillFinder, findsOneWidget);
    final pillText = tester.widget<Text>(find.descendant(of: pillFinder, matching: find.byType(Text))).data!;
    expect(pillText, contains('重要度 ${importancePercent(zoomedImp)}%'));
    expect(pillText, contains('顺位 #1'));

    // 停滚 300ms 后触发预留同步端口；1.1s 后瞬态提示自动淡出归静。
    await tester.pump(const Duration(milliseconds: 350));
    expect(syncPort.synced.length, 1);
    expect(syncPort.synced.single.agentId, 'beta');
    await tester.pump(const Duration(milliseconds: 900));
    expect(find.byKey(const ValueKey('planet-zoom-pill-beta')), findsNothing);

    // 向下滚轮缩小 beta（scrollDelta.dy > 0）
    await tester.sendEventToBinding(pointer.scroll(const Offset(0, 180)));
    await tester.pump();
    expect(h.state.manualImportanceFor('beta')!.importance, lessThan(zoomedImp));
    await tester.pump(const Duration(milliseconds: 1200));

    // 太阳不参与滚轮缩放
    final sunCenter = tester.getCenter(find.byKey(const ValueKey('sun')));
    await tester.sendEventToBinding(pointer.hover(sunCenter));
    await tester.sendEventToBinding(pointer.scroll(const Offset(0, -180)));
    await tester.pump();
    expect(h.state.hasManualImportance('chief'), isFalse);

    // 跨 980px 宽窄屏缩放后完整保留手动调节的重要度
    final savedImp = h.state.manualImportanceFor('beta')!.importance;
    tester.view.physicalSize = const Size(820, 800);
    await tester.pumpAndSettle();
    tester.view.physicalSize = const Size(1280, 800);
    await tester.pumpAndSettle();
    expect(h.state.manualImportanceFor('beta')!.importance, closeTo(savedImp, 1e-6));

    // 点击 beta 打开三栏工作大窗：左侧栏显示重要度调节与直接复位控件
    await tester.tap(find.byKey(const ValueKey('planet-beta')));
    await tester.pumpAndSettle();
    expect(find.byType(AgentPanel), findsOneWidget);
    expect(find.byKey(const ValueKey('agent-importance-control-beta')), findsOneWidget);

    // 点击左栏「+」按钮放大重要度，立即同步到预留端口
    final beforePlus = h.state.manualImportanceFor('beta')!.importance;
    await tester.tap(find.byKey(const ValueKey('agent-importance-inc-beta')));
    await tester.pumpAndSettle();
    expect(h.state.manualImportanceFor('beta')!.importance, closeTo((beforePlus + 0.10).clamp(0.0, 1.0), 1e-4));
    expect(syncPort.synced.last.agentId, 'beta');

    // 点击左栏「复位」按钮恢复自动大小，并触发 clearOverride 预留接口
    await tester.tap(find.byKey(const ValueKey('agent-importance-reset-beta')));
    await tester.pumpAndSettle();
    expect(h.state.hasManualImportance('beta'), isFalse);
    expect(syncPort.cleared, contains('beta'));

    // 切换到太阳时，左栏不显示员工重要度调节控件
    await tester.tap(find.byKey(const ValueKey('sun')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('agent-importance-control-chief')), findsNothing);

    await h.stop(tester);
  });
}
