import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/star_map.dart';
import 'package:ip_app/src/ui/shell/agent_panel.dart';
import 'package:ip_app/src/ui/shell/celestial_body.dart';
import 'package:ip_app/src/ui/shell/desktop_shell.dart';
import 'package:ip_app/src/ui/shell/local_bot.dart';

import 'harness.dart';

void main() {
  test('可选星球严格包含 9 种非太阳形态，排除太阳，并支持生成合法唯一 ID 与预期协议载荷', () {
    expect(selectableBotCelestials.length, 9);
    expect(selectableBotCelestials.any((c) => c.kind == CelestialKind.sun), isFalse);
    expect(selectableCelestialById('sun'), isNull);
    expect(selectableCelestialById('saturn')?.id, 'saturn');
    expect(selectableCelestialById('star')?.kind, CelestialKind.star);

    final slug1 = generateUniqueSlug('小研助手', prefix: 'bot', existingIds: {'bot-1'});
    expect(slug1, 'bot-2');
    final slug2 = generateUniqueSlug('Alpha Quant!', prefix: 'bot', existingIds: const {});
    expect(slug2, 'alpha-quant');

    final draft = CreateBotDraft(
      name: '小研',
      celestial: selectableCelestialById('saturn')!,
      newRoleName: '行业研究员',
      newRoleDescription: '跟踪半导体行业与财报点评',
      modelDefault: 'openai/gpt-6-luna',
      modelEscalate: 'openai/gpt-6-astra',
      budgetPerDayUsd: 3.5,
      tools: false,
      workspace: '/tmp/research',
    );
    final wire = draft.toPlannedWireJson(agentId: 'bot-1', roleId: 'role-1');
    expect((wire['create_role'] as Map)['budget_per_day_usd'], 3.5);
    expect((wire['create_agent'] as Map)['celestial_style'], 'saturn');
    expect((wire['create_agent'] as Map)['tools'], isFalse);
  });

  testWidgets('右下角「新建 Bot」弹窗：9 种星球可选、排除太阳、支持重复选星、新建岗位与高级配置、落轨与移除全闭环', (tester) async {
    final h = Harness();
    h.host.handlers['list_roles'] = (_) => [
      {
        'id': 'coordinator',
        'name': '协调员',
        'description': '统筹派活',
        'model': {'default': 'openai/gpt-6-astra', 'escalate': null},
        'budget_per_day_usd': 10.0,
      },
      {
        'id': 'macro',
        'name': '宏观研究',
        'description': '宏观研报',
        'model': {'default': 'openai/gpt-6-luna', 'escalate': null},
        'budget_per_day_usd': 2.0,
      },
    ];
    h.host.handlers['list_agents'] = (_) => [
      {'id': 'chief', 'role': 'coordinator', 'name': '小协'},
      {'id': 'macro-1', 'role': 'macro', 'name': '小宏'},
    ];
    h.host.handlers['list_approvals'] = (_) => const <Object>[];
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));

    expect(find.byType(DesktopShell), findsOneWidget);
    // 1B：右下角已升级为「新建 Bot」胶囊，不再保留旧的独立「星球形态」胶囊。
    expect(find.byKey(const ValueKey('create-bot-bubble')), findsOneWidget);
    expect(find.text('新建 Bot'), findsOneWidget);
    expect(find.text('星球形态'), findsNothing);

    // 算出 macro-1 自动分到的星球形态，专门重复选同一颗，验证「允许重复选 + 已有员工使用提示」。
    final existingMap = buildStarMap(
      agents: h.state.allAgents,
      roles: h.state.allRoles,
      tasks: h.state.tasks,
      approvals: h.state.approvals,
      working: h.state.agentsWorking,
      failures: h.state.agentFailures,
      now: 1000,
    );
    final usedStyle = CelestialStyle.fromCelestial(existingMap.byId('macro-1')!.celestial);

    // 打开居中极简 440px Liquid Glass 弹窗（参照 duskbot 构图，无多余定语与大蓝横幅）。
    await tester.tap(find.byKey(const ValueKey('create-bot-bubble')));
    await tester.pumpAndSettle();
    final modalFinder = find.byKey(const ValueKey('create-bot-modal'));
    expect(modalFinder, findsOneWidget);
    expect(tester.getRect(modalFinder).width, closeTo(440.0, 1.0));
    expect(find.textContaining('环带巨星'), findsNothing);
    expect(find.textContaining('银霜月相星'), findsNothing);
    expect(find.textContaining('金箔恒星'), findsNothing);

    // 默认首屏极简：打开时默认选中地球，且岗位与高级配置默认收起。
    expect(find.byKey(const ValueKey('create-bot-hero-oceanic')), findsOneWidget);
    expect(tester.widget<Text>(find.byKey(const ValueKey('create-bot-selected-label'))).data, startsWith('地球'));
    expect(find.byKey(const ValueKey('create-bot-role-mode-new')), findsNothing);

    // 严格排除太阳，9 种可选行星/恒星纯图标全部呈现。
    expect(find.byKey(const ValueKey('create-bot-celestial-sun')), findsNothing);
    for (final c in selectableBotCelestials) {
      final st = CelestialStyle.fromCelestial(c);
      expect(find.byKey(ValueKey('create-bot-celestial-${st.name}')), findsOneWidget);
    }

    // 刻意选择与小宏相同的星球形态：大球下方只标本名与在轨颗数，依然允许重复创建。
    await tester.tap(find.byKey(ValueKey('create-bot-celestial-${usedStyle.name}')));
    await tester.pumpAndSettle();
    expect(
      tester.widget<Text>(find.byKey(const ValueKey('create-bot-selected-label'))).data,
      '${usedStyle.bodyName} · 在轨 1 颗',
    );

    // 填写 Bot 名称，点左下角「岗位与高级设置」展开填写新岗位、预算与关闭沙箱工具开关。
    await tester.enterText(find.byKey(const ValueKey('create-bot-name-input')), '小星');
    await tester.tap(find.byKey(const ValueKey('create-bot-advanced-toggle')));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.byKey(const ValueKey('create-bot-role-mode-new')));
    await tester.tap(find.byKey(const ValueKey('create-bot-role-mode-new')));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.byKey(const ValueKey('create-bot-role-name-input')));
    await tester.enterText(find.byKey(const ValueKey('create-bot-role-name-input')), '量化策略');
    await tester.enterText(find.byKey(const ValueKey('create-bot-role-desc-input')), '跟踪因子与回测信号');
    await tester.ensureVisible(find.byKey(const ValueKey('create-bot-budget-input')));
    await tester.enterText(find.byKey(const ValueKey('create-bot-budget-input')), '4.5');
    await tester.ensureVisible(find.byKey(const ValueKey('create-bot-tools-switch')));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('create-bot-tools-switch')));
    await tester.pumpAndSettle();

    // 提交创建。
    await tester.tap(find.byKey(const ValueKey('create-bot-submit')));
    await tester.pumpAndSettle();

    expect(find.byKey(const ValueKey('create-bot-modal')), findsNothing);
    expect(h.state.localBots.length, 1);
    final created = h.state.localBots.single;
    expect(created.agent.name, '小星');
    expect(CelestialStyle.fromCelestial(created.celestial), usedStyle);
    expect(created.createdRole?.name, '量化策略');
    expect(created.createdRole?.budgetPerDayUsd, 4.5);
    expect(created.tools, isFalse);

    // 立刻以手选星球形态出现在天穹上，且跨 980px 缩放不丢失。
    final planetFinder = find.byKey(ValueKey('planet-${created.agent.id}'));
    expect(planetFinder, findsOneWidget);
    final bodyWidget = tester.widget<CelestialBody>(planetFinder);
    expect(bodyWidget.style, usedStyle);

    tester.view.physicalSize = const Size(820, 800);
    await tester.pumpAndSettle();
    tester.view.physicalSize = const Size(1280, 800);
    await tester.pumpAndSettle();
    expect(find.byKey(ValueKey('planet-${created.agent.id}')), findsOneWidget);

    // 点开新星球进入三栏工作大窗，展示本地新建档案并可一键移除。
    await tester.tap(find.byKey(ValueKey('planet-${created.agent.id}')));
    await tester.pumpAndSettle();
    expect(find.byType(AgentPanel), findsOneWidget);
    expect(find.text('本地新建'), findsWidgets);
    expect(find.text('已关闭'), findsOneWidget);

    await tester.tap(find.byKey(ValueKey('remove-local-bot-${created.agent.id}')));
    await tester.pumpAndSettle();
    expect(h.state.localBots, isEmpty);
    expect(find.byType(AgentPanel), findsNothing);
    expect(find.byKey(ValueKey('planet-${created.agent.id}')), findsNothing);

    await h.stop(tester);
  });
}
