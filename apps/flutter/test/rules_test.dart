// 路由规则页（F-18 之四）：类型能原样转回 JSON；试一试、草案、看规则、编辑并保存。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/protocol/router.dart';
import 'package:ip_app/src/ui/admin/rules_page.dart';

import 'admin_test.dart' show adminHarness, openAdmin;
import 'fake_host.dart';

Json rule(String id, {List<String> any = const [], String role = 'macro', String? agent}) => {
  'id': id,
  'description': '',
  'when': {'any': any, 'all': <String>[], 'none': <String>[], 'from': <String>[]},
  'to': {'role': role, 'agent': ?agent, 'escalate': false},
};

void main() {
  test('RouterSpec 原样转回 JSON，兜底和语义匹配都在', () {
    final Json j = {
      'rules': [
        {
          'id': 'cpi',
          'description': '物价',
          'when': {
            'any': ['CPI', '通胀'],
            'all': <String>[],
            'none': ['股票'],
            'regex': r'\d+月',
            'from': ['slack:U1'],
          },
          'to': {'role': 'macro', 'agent': 'macro-1', 'escalate': true},
        },
      ],
      'fallback': {'role': 'macro'},
      'semantic': {'model': 'openai/gpt-6-luna', 'min_confidence': 0.8},
    };
    expect(RouterSpec.fromJson(j).toJson(), j);
    final ask = RouterSpec.fromJson({'rules': <Object>[], 'fallback': 'ask'});
    expect(ask.toJson(), {'rules': <Object>[], 'fallback': 'ask'});
    expect(Rule.fromJson((j['rules'] as List)[0] as Json).when.describe(), r'含任一：CPI、通胀；不含：股票；正则：\d+月；发送人：slack:U1');
    expect(splitList(' CPI，通胀、 PPI,\n'), ['CPI', '通胀', 'PPI']);
  });

  testWidgets('路由规则页：试一试、看规则、采纳草案先编辑再保存、忽略草案', (tester) async {
    final h = adminHarness();
    Json spec = {
      'rules': [
        rule('cpi', any: ['CPI']),
      ],
      'fallback': 'ask',
    };
    var drafts = [
      {
        'id': 'dr1',
        'rule': rule('服务器', any: ['服务器'], role: 'macro'),
        'source': '张三的手机 把「服务器又挂了」交给了 宏观研究',
        'created_at_ms': 1000,
      },
      {'id': 'dr2', 'rule': rule('别的'), 'source': '另一条', 'created_at_ms': 1000},
    ];
    h.host.handlers.addAll({
      'list_rules': (_) => {'spec': spec, 'from_console': false},
      'list_rule_drafts': (_) => drafts,
      'route': (req) => {
        'kind': 'routed',
        'by': 'rule',
        'rule_id': 'cpi',
        'role': 'macro',
        'agent': null,
        'escalate': false,
        'reason': '命中规则 cpi',
      },
      'save_rules': (req) {
        spec = req['spec'] as Json;
        drafts = drafts.where((d) => d['id'] != req['accepted_draft']).toList();
        return spec;
      },
      'dismiss_rule_draft': (req) {
        drafts = drafts.where((d) => d['id'] != req['draft_id']).toList();
        return drafts;
      },
    });
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 2400));
    await openAdmin(tester, h, '路由规则');
    expect(find.byType(RulesPage), findsOneWidget);
    expect(find.text('规则（1 条，按顺序匹配，第一条命中的生效）'), findsOneWidget);
    expect(find.textContaining('含任一：CPI'), findsOneWidget);
    expect(find.text('兜底：交给人决定'), findsOneWidget);
    expect(find.textContaining('当前规则来自 ip.yaml'), findsOneWidget);

    await tester.enterText(find.widgetWithText(TextField, '输入一句话，看看会分给谁'), '9 月 CPI 多少');
    await tester.tap(find.text('路由'));
    await tester.pumpAndSettle();
    expect(h.host.requests.lastWhere((r) => r['type'] == 'route')['text'], '9 月 CPI 多少');
    expect(find.text('→ 宏观研究。命中规则 cpi'), findsOneWidget);

    expect(find.text('规则草案（2）'), findsOneWidget);
    await tester.tap(find.text('忽略').last);
    await tester.pumpAndSettle();
    expect(find.text('规则草案（1）'), findsOneWidget);

    await tester.tap(find.text('采纳（先编辑）'));
    await tester.pumpAndSettle();
    expect(find.text('采纳草案：改好后保存'), findsOneWidget);
    // 草案接在原来的规则后面：第 2 条。
    final second = find.widgetWithText(TextFormField, '含任一').at(1);
    expect(tester.widget<TextFormField>(second).initialValue, '服务器');
    await tester.enterText(second, '服务器、宕机');
    await tester.tap(find.text('保存并生效'));
    await tester.pumpAndSettle();
    final save = h.host.requests.lastWhere((r) => r['type'] == 'save_rules');
    expect(save['accepted_draft'], 'dr1');
    final rules = (save['spec'] as Json)['rules'] as List;
    expect(rules.length, 2);
    expect(((rules[1] as Json)['when'] as Json)['any'], ['服务器', '宕机']);
    expect(find.text('规则（2 条，按顺序匹配，第一条命中的生效）'), findsOneWidget);
    expect(find.textContaining('规则草案'), findsNothing);
    expect(find.textContaining('当前规则是在控制台或 App 保存的'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('编辑规则：新增、下移、删除、语义匹配、兜底；主机拒绝时显示原因，留在编辑', (tester) async {
    final h = adminHarness();
    var reject = true;
    h.host.handlers.addAll({
      'list_rules': (_) => {
        'spec': {
          'rules': [
            rule('a', any: ['甲']),
            rule('b', any: ['乙']),
          ],
          'fallback': 'ask',
        },
        'from_console': true,
      },
      'list_rule_drafts': (_) => <Object>[],
      'save_rules': (req) {
        if (reject) throw '规则名重复：a';
        return req['spec'];
      },
    });
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 2600));
    await openAdmin(tester, h, '路由规则');
    await tester.tap(find.text('编辑'));
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('下移').first);
    await tester.pumpAndSettle();
    await tester.tap(find.text('新规则'));
    await tester.pumpAndSettle();
    expect(find.widgetWithText(TextFormField, '规则名'), findsNWidgets(3));
    await tester.tap(find.byTooltip('删除规则').last);
    await tester.pumpAndSettle();

    await tester.tap(find.text('规则都没命中时做语义匹配'));
    await tester.pumpAndSettle();
    await tester.enterText(find.widgetWithText(TextFormField, '把握至少'), '0.85');
    await tester.tap(find.text('交给人决定'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('交给 宏观研究').last);
    await tester.pumpAndSettle();

    await tester.tap(find.text('保存并生效'));
    await tester.pumpAndSettle();
    expect(find.text('没有保存：规则名重复：a'), findsOneWidget);
    expect(find.text('编辑规则'), findsOneWidget);

    reject = false;
    await tester.tap(find.text('保存并生效'));
    await tester.pumpAndSettle();
    final spec = h.host.requests.lastWhere((r) => r['type'] == 'save_rules')['spec'] as Json;
    expect([for (final r in spec['rules'] as List) (r as Json)['id']], ['b', 'a']);
    expect(spec['fallback'], {'role': 'macro'});
    expect(spec['semantic'], {'model': 'fake/m', 'min_confidence': 0.85});
    expect(find.text('编辑规则'), findsNothing);
    expect(find.text('兜底：交给 宏观研究'), findsOneWidget);
    await h.stop(tester);
  });
}
