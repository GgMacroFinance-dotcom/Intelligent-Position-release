// 系统通知（F-16）：在后台才弹、按标签撤、点开去那个对话或“待办”、presence、这台设备的通知设置。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/notifications.dart';
import 'package:ip_app/src/ui/me_page.dart';

import 'fake_host.dart';
import 'harness.dart';

void main() {
  test('通知 id 由标签算出来，同一标签同一个 id，都是正的 31 位', () {
    expect(alertId('approval:a1'), alertId('approval:a1'));
    expect(alertId('approval:a1'), isNot(alertId('approval:a2')));
    // Android 原生弹锁屏推送（F-21，IpPush.kt）用同样的算法，WebPushCryptoTest.kt 里核对同样这三个数。
    expect(alertId(''), 18652613);
    expect(alertId('approval:a1'), 518277434);
    expect(alertId('thread:对话'), 157519743);
    for (final t in ['', 'thread:t1', 'task:${'x' * 200}']) {
      expect(alertId(t), inInclusiveRange(0, 0x7fffffff));
    }
  });

  testWidgets('在前台不弹，切到后台才弹；dismiss 一律撤', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester);
    // 连上后要过一次通知权限。
    expect(h.notifier.asked, 1);

    h.host.last.push(alertFrame());
    await tester.pump();
    expect(h.notifier.shown, isEmpty);

    h.state.setForeground(false);
    h.host.last.push(alertFrame(tag: 'thread:t1'));
    await tester.pump();
    expect(h.notifier.shown.single.tag, 'thread:t1');
    expect(h.notifier.shown.single.threadId, 't1');
    expect(h.notifier.shown.single.roleId, 'macro');

    h.host.last.push({'type': 'dismiss', 'tag': 'thread:t1'});
    await tester.pump();
    expect(h.notifier.cancelled, ['thread:t1']);
    await h.stop(tester);
  });

  testWidgets('presence：连上报一次，前后台切换马上报，在前台时定时报', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester);
    List<Object?> reports() => [
      for (final r in h.host.requests)
        if (r['type'] == 'presence') r['active'],
    ];
    expect(reports(), [true]);
    h.state.setForeground(false);
    await tester.pump();
    expect(reports(), [true, false]);
    // 在后台时不定时报。
    await tester.pump(const Duration(seconds: 25));
    expect(reports(), [true, false]);
    h.state.setForeground(true);
    await tester.pump();
    await tester.pump(const Duration(seconds: 21));
    expect(reports(), [true, false, true, true]);
    await h.stop(tester);
  });

  testWidgets('点通知：没有对话的去“待办”，有对话的去那个对话', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester);
    // 在任务页打开“我”页时点一条没有对话的通知：“我”页关掉，回到待办。
    await tester.tap(find.text('任务'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    h.notifier.tap(null);
    await tester.pumpAndSettle();
    expect(find.byType(MePage), findsNothing);
    expect(find.text('等你决定（1）'), findsOneWidget);

    h.notifier.tap('t1');
    await tester.pumpAndSettle();
    expect(h.state.threadId, 't1');
    expect(find.text('最近 CPI 怎么样'), findsWidgets);
    // 手机上对话推成整屏，底部导航收起来。
    expect(find.byType(NavigationBar), findsNothing);
    await h.stop(tester);
  });

  testWidgets('“我”页改通知设置：级别、预览、静音、岗位', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    expect(find.text('这台设备的通知'), findsOneWidget);

    Map<String, dynamic> lastSaved() =>
        h.host.requests.lastWhere((r) => r['type'] == 'set_notify_settings')['settings'] as Map<String, dynamic>;

    await tester.tap(find.text('只通知要我处理的'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('全部').last);
    await tester.pumpAndSettle();
    expect(lastSaved()['level'], 'all');

    await h.scrollMe(tester, find.text('通知里显示内容预览'));
    await tester.tap(find.text('通知里显示内容预览'));
    await tester.pumpAndSettle();
    expect(lastSaved()['preview'], false);
    expect(lastSaved()['level'], 'all');

    await h.scrollMe(tester, find.text('静音 1 小时'));
    await tester.tap(find.text('静音 1 小时'));
    await tester.pumpAndSettle();
    final until = lastSaved()['mute_until_ms'] as int;
    expect(until, greaterThan(DateTime.now().millisecondsSinceEpoch));
    expect(find.text('取消静音'), findsOneWidget);
    await tester.tap(find.text('取消静音'));
    await tester.pumpAndSettle();
    expect(lastSaved()['mute_until_ms'], isNull);

    await h.scrollMe(tester, find.widgetWithText(CheckboxListTile, '宏观研究'));
    await tester.tap(find.widgetWithText(CheckboxListTile, '宏观研究'));
    await tester.pumpAndSettle();
    expect(lastSaved()['muted_roles'], ['macro']);
    await h.stop(tester);
  });

  testWidgets('系统没允许通知时“我”页提示，能再问一次', (tester) async {
    final h = Harness();
    h.notifier.allow = false;
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    expect(find.text('系统没允许这个 App 发通知'), findsOneWidget);
    h.notifier.allow = true;
    await tester.tap(find.text('再问一次'));
    await tester.pumpAndSettle();
    expect(h.notifier.asked, 2);
    expect(find.text('系统没允许这个 App 发通知'), findsNothing);
    await h.stop(tester);
  });

  testWidgets('主机太旧没有通知设置时说明', (tester) async {
    final h = Harness();
    h.host.handlers.remove('get_notify_settings');
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    expect(find.text('主机没有通知设置（版本太旧），请升级主机。'), findsOneWidget);
    await h.stop(tester);
  });
}
