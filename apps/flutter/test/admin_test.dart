// “管理”权限才有的页（F-18）：日常设备看不到；设备页的添加、改名、改权限、吊销。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/ui/admin/devices_page.dart';
import 'package:ip_app/src/ui/admin/schedules_page.dart';

import 'fake_host.dart';
import 'harness.dart';

Json device(String id, String name, {String scope = 'daily', bool online = false}) => {
  'id': id,
  'name': name,
  'scope': scope,
  'paired_at_ms': 1000,
  'last_seen_ms': 2000,
  'online': online,
};

/// 当前设备是“管理”权限，主机上已经配对过两台设备。
Harness adminHarness() {
  final h = Harness();
  final devices = [device('d1', '张三的手机', scope: 'admin', online: true), device('d2', '办公室电脑')];
  h.host.handlers.addAll({
    'who_am_i': (_) => {'device': devices.firstWhere((d) => d['id'] == 'd1'), 'scope': devices.first['scope']},
    'list_devices': (_) => devices,
    'create_pairing': (req) => {
      'code': '482913',
      'name': req['name'],
      'expires_ms': DateTime.now().add(const Duration(minutes: 2)).millisecondsSinceEpoch,
      'link': '${req['link_base']}/#/pair/482913',
      'qr': ['1110111', '1010101', '1110111'],
    },
    'rename_device': (req) {
      devices.firstWhere((d) => d['id'] == req['device_id'])['name'] = req['name'];
      return devices;
    },
    'set_device_scope': (req) {
      devices.firstWhere((d) => d['id'] == req['device_id'])['scope'] = req['scope'];
      return devices;
    },
    'revoke_device': (req) {
      devices.removeWhere((d) => d['id'] == req['device_id']);
      return devices;
    },
  });
  return h;
}

Future<void> openAdmin(WidgetTester tester, Harness h, String page) async {
  await tester.tap(find.byTooltip('我'));
  await tester.pumpAndSettle();
  await h.scrollMe(tester, find.text(page));
  await tester.tap(find.text(page));
  await tester.pumpAndSettle();
}

Future<void> openDevices(WidgetTester tester, Harness h) => openAdmin(tester, h, '设备');

Json schedule(String id, {bool enabled = true, Json? last, Json? to}) => {
  'spec': {
    'id': id,
    'description': '每周一 CPI 点评',
    'cron': '0 9 * * 1',
    'utc_offset': '+08:00',
    'to': ?to,
    'text': '写一份上周 CPI 的点评',
    'enabled': enabled,
  },
  'next_run_ms': enabled ? 5000 : null,
  'last_run': last,
};

void main() {
  testWidgets('“日常”权限的设备在“我”页看不到“管理”', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    expect(find.text('管理'), findsNothing);
    expect(find.text('添加设备、改权限、吊销'), findsNothing);
    await h.stop(tester);
  });

  testWidgets('设备页：列出设备，生成配对码和二维码，那台设备配上后收起', (tester) async {
    final h = adminHarness();
    await h.savePaired();
    await h.start(tester);
    await openDevices(tester, h);
    expect(find.text('配对过的设备（2）'), findsOneWidget);
    expect(find.text('张三的手机（这台）'), findsOneWidget);
    expect(find.textContaining('在线'), findsWidgets);

    await tester.enterText(find.widgetWithText(TextField, '添加设备'), '李四的平板');
    await tester.tap(find.text('生成配对码'));
    await tester.pumpAndSettle();
    final req = h.host.requests.lastWhere((r) => r['type'] == 'create_pairing');
    expect(req['name'], '李四的平板');
    expect(req['link_base'], 'https://ip.example.com');
    expect(find.text('给「李四的平板」配对'), findsOneWidget);
    expect(find.text('482913'), findsOneWidget);
    expect(find.byType(QrView), findsOneWidget);
    expect(find.text('https://ip.example.com/#/pair/482913'), findsOneWidget);

    // 那台设备配上了：下一次看列表时收起配对码。
    (h.host.handlers['list_devices']!({}) as List).add(device('d3', '李四的平板', online: true));
    await tester.pump(const Duration(seconds: 3));
    await tester.pumpAndSettle();
    expect(find.text('482913'), findsNothing);
    expect(find.text('配对过的设备（3）'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('设备页：改名、给管理权限要确认、吊销要确认', (tester) async {
    final h = adminHarness();
    await h.savePaired();
    await h.start(tester);
    await openDevices(tester, h);
    final row = find.ancestor(of: find.text('办公室电脑'), matching: find.byType(ListTile));

    await tester.tap(find.descendant(of: row, matching: find.byTooltip('更多')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('改名'));
    await tester.pumpAndSettle();
    await tester.enterText(find.widgetWithText(TextField, '新的设备名'), '会议室电脑');
    await tester.tap(find.text('保存'));
    await tester.pumpAndSettle();
    expect(find.text('会议室电脑'), findsOneWidget);

    final row2 = find.ancestor(of: find.text('会议室电脑'), matching: find.byType(ListTile));
    await tester.tap(find.descendant(of: row2, matching: find.text('日常')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('管理').last);
    await tester.pumpAndSettle();
    expect(find.text('给「会议室电脑」“管理”权限？'), findsOneWidget);
    await tester.tap(find.text('给管理权限'));
    await tester.pumpAndSettle();
    final scopeReq = h.host.requests.lastWhere((r) => r['type'] == 'set_device_scope');
    expect((scopeReq['device_id'], scopeReq['scope']), ('d2', 'admin'));

    await tester.tap(find.descendant(of: row2, matching: find.byTooltip('更多')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('吊销'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, '吊销'));
    await tester.pumpAndSettle();
    expect(find.text('会议室电脑'), findsNothing);
    expect(find.text('配对过的设备（1）'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('把这台自己降成“日常”：退出设备页，“管理”一节不见了', (tester) async {
    final h = adminHarness();
    await h.savePaired();
    await h.start(tester);
    await openDevices(tester, h);
    final row = find.ancestor(of: find.text('张三的手机（这台）'), matching: find.byType(ListTile));
    await tester.tap(find.descendant(of: row, matching: find.text('管理')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('日常').last);
    await tester.pumpAndSettle();
    expect(find.byType(DevicesPage), findsNothing);
    expect(h.state.isAdmin, isFalse);
    expect(find.text('添加设备、改权限、吊销'), findsNothing);
    await h.stop(tester);
  });

  testWidgets('定时任务页：看下次、上次运行，立即运行后能打开那个对话', (tester) async {
    final h = adminHarness();
    final schedules = [
      schedule('cpi', to: {'role': 'macro', 'agent': 'macro-1', 'escalate': false}),
      schedule('old', enabled: false, last: {'at_ms': 3000, 'ok': false, 'detail': '模型超时'}),
    ];
    h.host.handlers.addAll({
      'list_schedules': (_) => schedules,
      'run_schedule': (req) {
        schedules[0]['last_run'] = {'at_ms': 4000, 'ok': true, 'detail': 't9', 'thread_id': 't9'};
        return {'message': message('m9', 't9', human: '定时任务', text: '写一份上周 CPI 的点评')};
      },
    });
    await h.savePaired();
    await h.start(tester);
    await openAdmin(tester, h, '定时任务');
    expect(find.byType(SchedulesPage), findsOneWidget);
    expect(find.text('0 9 * * 1（UTC+08:00）· 发给 宏观研究 · 宏观研究员'), findsOneWidget);
    expect(find.text('0 9 * * 1（UTC+08:00）· 发给 按路由规则'), findsOneWidget);
    expect(find.text('已停用'), findsOneWidget);
    expect(find.textContaining('失败：模型超时'), findsOneWidget);
    expect(find.text('还没运行过'), findsOneWidget);

    await tester.tap(find.text('立即运行').first);
    await tester.pumpAndSettle();
    expect(h.host.requests.lastWhere((r) => r['type'] == 'run_schedule')['schedule_id'], 'cpi');
    expect(find.textContaining('已发出'), findsOneWidget);
    expect(find.textContaining('上次').evaluate().length, 2);

    await tester.tap(find.text('打开对话'));
    await tester.pumpAndSettle();
    expect(find.byType(SchedulesPage), findsNothing);
    expect(h.state.threadId, 't9');
    await h.stop(tester);
  });

  testWidgets('没有定时任务时说明在哪配', (tester) async {
    final h = adminHarness();
    h.host.handlers['list_schedules'] = (_) => <Object>[];
    await h.savePaired();
    await h.start(tester);
    await openAdmin(tester, h, '定时任务');
    expect(find.textContaining('ip.yaml'), findsOneWidget);
    await h.stop(tester);
  });
}
