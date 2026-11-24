// 整个 App 跑在假主机上：配对、待办里的审批卡、对话的流式回复。

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/ui/shell/desktop_shell.dart';
import 'package:ip_app/src/protocol/client.dart';

import 'fake_host.dart';
import 'harness.dart';

void main() {
  testWidgets('没配对过时先“连接主机”，配对后进待办，能选审批卡的选项', (tester) async {
    final h = Harness();
    h.host.handlers['resolve_approval'] = (req) => approval('a1', pending: false);
    await h.start(tester);
    expect(find.text('连接主机'), findsOneWidget);

    // 粘贴控制台的配对链接，自动拆成地址和配对码。
    await tester.enterText(find.widgetWithText(TextField, '主机地址'), 'https://ip.example.com/#/pair/123456');
    await tester.pump();
    String field(String label) => tester.widget<TextField>(find.widgetWithText(TextField, label)).controller!.text;
    expect(field('主机地址'), 'https://ip.example.com');
    expect(field('配对码'), '123456');

    await tester.tap(find.text('配对并连接'));
    await tester.pumpAndSettle();
    expect(find.text('连接这台主机？'), findsOneWidget);
    await tester.tap(find.text('连接'));
    await tester.pumpAndSettle();

    expect(h.paired.single.$2, '123456');
    expect(h.secrets.values['token:https://ip.example.com'], 'ipd_abc');
    expect(h.host.connects.single.$2['Authorization'], 'Bearer ipd_abc');
    expect(find.text('等你决定（1）'), findsOneWidget);
    expect(find.text('这条消息该交给谁？'), findsOneWidget);
    // 导航上的角标。
    expect(find.descendant(of: find.byType(Badge), matching: find.text('1')), findsWidgets);

    await tester.tap(find.text('宏观研究员（推荐）'));
    await tester.pumpAndSettle();
    final req = h.host.requests.lastWhere((r) => r['type'] == 'resolve_approval');
    expect(req['approval_id'], 'a1');
    expect(req['choice'], 0);
    expect(find.text('没有等你处理的事。'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('明文连局域网地址时要勾“我信任这个网络”', (tester) async {
    final h = Harness();
    await h.start(tester);
    await tester.enterText(find.widgetWithText(TextField, '主机地址'), 'http://192.168.1.20:8787');
    await tester.enterText(find.widgetWithText(TextField, '配对码'), '123456');
    await tester.tap(find.text('配对并连接'));
    await tester.pumpAndSettle();
    final connect = find.widgetWithText(FilledButton, '连接');
    expect(tester.widget<FilledButton>(connect).onPressed, isNull);
    await tester.tap(find.text('我信任这个网络'));
    await tester.pumpAndSettle();
    expect(tester.widget<FilledButton>(connect).onPressed, isNotNull);
    await tester.tap(connect);
    await tester.pumpAndSettle();
    expect((await h.state.store.hosts()).single.trusted, isTrue);
    await h.stop(tester);
  });

  testWidgets('配对码不对时留在“连接主机”，显示原因', (tester) async {
    final h = Harness();
    await h.start(tester);
    await tester.enterText(find.widgetWithText(TextField, '主机地址'), 'ip.example.com');
    await tester.enterText(find.widgetWithText(TextField, '配对码'), '000000');
    await tester.tap(find.text('配对并连接'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('连接'));
    await tester.pumpAndSettle();
    expect(find.text('配对码不对、过期了或者已经用过'), findsOneWidget);
    expect(h.secrets.values.containsKey('hosts'), isFalse);
    await h.stop(tester);
  });

  testWidgets('手机上开新对话、发消息、看流式回复', (tester) async {
    final h = Harness();
    h.host.handlers['send'] = (req) => {
      'message': message('m2', 't2', human: '张三的手机', text: req['text'] as String, at: 2000),
      'agent_id': 'macro-1',
      'decision': null,
      'approval_id': null,
    };
    await h.savePaired();
    await h.start(tester);
    await tester.tap(find.text('对话'));
    await tester.pumpAndSettle();
    expect(find.text('最近 CPI 怎么样'), findsOneWidget);

    await tester.tap(find.byTooltip('新对话'));
    await tester.pumpAndSettle();
    expect(find.text('新对话'), findsWidgets);
    await tester.tap(find.byType(DropdownButton<String?>));
    await tester.pumpAndSettle();
    await tester.tap(find.text('宏观研究员（macro）').last);
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '9 月 CPI 同比多少？');
    await tester.tap(find.byTooltip('发送'));
    await tester.pumpAndSettle();
    final sent = h.host.requests.lastWhere((r) => r['type'] == 'send');
    expect(sent['agent_id'], 'macro-1');
    expect(sent.containsKey('thread_id'), isFalse);
    expect(sent.containsKey('author'), isFalse);
    expect(h.state.threadId, 't2');

    h.host.event({'type': 'turn_started', 'agent_id': 'macro-1', 'thread_id': 't2'});
    h.host.event({'type': 'delta', 'agent_id': 'macro-1', 'thread_id': 't2', 'text': '9 月 CPI '});
    h.host.event({'type': 'delta', 'agent_id': 'macro-1', 'thread_id': 't2', 'text': '同比 **3.7%**'});
    await tester.pump();
    await tester.pump();
    expect(find.textContaining('正在输入'), findsOneWidget);
    expect(find.textContaining('开始处理'), findsOneWidget);

    h.host.event({
      'type': 'message',
      'message': message('m3', 't2', agent: 'macro-1', text: '9 月 CPI 同比 **3.7%**（2026-09）', at: 3000),
    });
    h.host.event({
      'type': 'turn_finished',
      'agent_id': 'macro-1',
      'thread_id': 't2',
      'steps': 2,
      'usage': {'prompt_tokens': 1000, 'completion_tokens': 100, 'cached_tokens': 900, 'cache_write_tokens': 0},
      'cost_usd': 0.001,
      'elapsed_ms': 2500,
    });
    await tester.pump(const Duration(milliseconds: 400));
    await tester.pumpAndSettle();
    expect(find.textContaining('正在输入'), findsNothing);
    expect(find.textContaining('缓存命中 90%'), findsOneWidget);
    expect(h.state.messages.map((m) => m.id), ['m2', 'm3']);
    await h.stop(tester);
  });

  testWidgets('宽窗口里先只有黑底和左下角“我”、连接状态；点了打开设置', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    expect(find.byType(DesktopShell), findsOneWidget);
    expect(find.byType(NavigationRail), findsNothing);
    expect(find.text('已连接 · ip.example.com'), findsOneWidget);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    expect(find.text('断开并忘掉这台主机'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('宽屏上导航在左边，对话列表和对话并排', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(960, 800));
    expect(find.byType(NavigationRail), findsOneWidget);
    await tester.tap(find.text('对话'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('最近 CPI 怎么样'));
    await tester.pumpAndSettle();
    // 列表里一条，对话里一条（人写的消息）。
    expect(find.text('最近 CPI 怎么样'), findsNWidgets(2));
    expect(find.byTooltip('发送'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('扣除侧栏后进入单栏，跨断点保留会话、草稿与返回操作', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(880, 760));
    await tester.tap(find.text('对话'));
    await tester.pumpAndSettle();
    expect(find.byType(NavigationRail), findsOneWidget);
    expect(find.byTooltip('发送'), findsNothing);
    await tester.tap(find.text('最近 CPI 怎么样'));
    await tester.pumpAndSettle();
    final thread = h.state.threadId;
    expect(thread, isNotNull);
    expect(find.byType(BackButton), findsOneWidget);
    await tester.enterText(find.byType(TextField), '未发出的草稿');
    final editor = tester.widget<TextField>(find.byType(TextField)).controller;

    // 960 是对话列表和对话并排；外壳（>= 980）里暂时没有对话，不在这里切。
    for (final width in [960.0, 880.0, 390.0, 800.0]) {
      tester.view.physicalSize = Size(width, 760);
      await tester.pumpAndSettle();
      expect(h.state.threadId, thread);
      expect(tester.widget<TextField>(find.byType(TextField)).controller, same(editor));
      expect(editor!.text, '未发出的草稿');
      expect(find.byType(BackButton), width == 960 ? findsNothing : findsOneWidget);
    }

    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(find.text('最近 CPI 怎么样'), findsOneWidget);
    expect(find.byTooltip('发送'), findsNothing);

    await tester.tap(find.text('待办'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('查看对话'));
    await tester.pumpAndSettle();
    expect(h.state.threadId, 't1');
    expect(find.byTooltip('发送'), findsOneWidget);
    await tester.tap(find.byType(BackButton));
    await tester.pumpAndSettle();
    await tester.tap(find.text('新对话'));
    await tester.pumpAndSettle();
    expect(h.state.threadId, isNull);
    expect(find.byTooltip('发送'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('宽屏默认输入框中的草稿在收窄后仍可编辑', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(960, 800));
    await tester.tap(find.text('对话'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '尚未创建对话');
    await tester.pump();
    tester.view.physicalSize = const Size(390, 800);
    await tester.pumpAndSettle();
    expect(find.byType(BackButton), findsOneWidget);
    expect(tester.widget<TextField>(find.byType(TextField)).controller!.text, '尚未创建对话');
    await h.stop(tester);
  });

  testWidgets('主机吊销了这台设备：提示重新配对，不再重连', (tester) async {
    final h = Harness();
    await h.savePaired();
    h.host.failNext = const WebSocketException('not upgraded', 401);
    await h.start(tester);
    expect(find.textContaining('要重新配对'), findsOneWidget);
    await tester.tap(find.text('重新配对'));
    await tester.pumpAndSettle();
    expect(find.text('连接主机'), findsOneWidget);
    expect(h.secrets.values['token:https://ip.example.com'], isNull);
    expect(h.state.status, ConnStatus.closed);
    await h.stop(tester);
  });
}
