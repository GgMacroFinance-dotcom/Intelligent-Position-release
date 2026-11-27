// 整个 App 跑在假主机上：配对、待办里的审批卡、对话的流式回复。

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/ui/chat_page.dart' show ThreadList;
import 'package:ip_app/src/ui/inbox_page.dart' show InboxPage;
import 'package:ip_app/src/ui/roles_page.dart' show RolesPage;
import 'package:ip_app/src/ui/tasks_page.dart' show TasksPage;
import 'package:ip_app/src/ui/shell/agent_panel.dart';
import 'package:ip_app/src/ui/shell/agent_sheet.dart';
import 'package:ip_app/src/ui/shell/celestial_body.dart';
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

  testWidgets('宽窗口里太阳能单独选中：点太阳从底部滑上来员工面板，再点、点空白处、按 Esc 收起', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    final sun = find.byKey(const ValueKey('sun'));
    expect(sun, findsOneWidget);
    bool selected() => tester.widget<CelestialBody>(sun).selected;
    // 选中过渡真正走到哪了（以前太阳被 TickerMode 关掉，过渡停在 0，选中了也不变亮）。
    double lift() => (tester.state(sun) as dynamic).selection as double;
    expect(selected(), isFalse);
    expect(find.byType(AgentSheetContent), findsNothing);

    // 点太阳：选中，面板滑上来，里面是原来星图气泡的内容。
    await tester.tap(sun);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    expect(lift(), inExclusiveRange(0.0, 1.0));
    await tester.pumpAndSettle();
    expect(selected(), isTrue);
    expect(lift(), 1.0);
    expect(tester.widget<CelestialBody>(sun).frozen, isTrue);
    expect(find.byType(AgentSheetContent), findsOneWidget);
    // 默认的假主机没有协调员：点太阳仍然展示完整的左/中/右三栏骨架。
    expect(find.text('幕僚长的对话'), findsOneWidget);
    expect(find.text('还没有配置幕僚长'), findsOneWidget);
    expect(find.text('幕僚长（空位）'), findsWidgets);
    expect(find.text('太阳的位置留给幕僚长。'), findsOneWidget);
    // 浮空窗口四边留白、至少盖住窗口高度的 2/3，太阳升到窗口上沿露出的天空里且不重叠。
    final sheet = tester.getRect(find.byType(AgentPanel));
    expect(sheet.bottom, closeTo(800 - FrostedSheet.bottomMargin, 0.5));
    expect(sheet.top, lessThanOrEqualTo(800 / 3));
    expect(tester.getRect(sun).bottom, lessThanOrEqualTo(sheet.top));
    // 三栏顶栏（左红绿灯、中右收起按钮、右 Segmented Control）落在同一条水平中线上。
    final tlY = tester.getCenter(find.byKey(const ValueKey('window-close'))).dy;
    final toggleY = tester.getCenter(find.byTooltip('收起员工详情')).dy;
    final tabY = tester.getCenter(find.byKey(const ValueKey('agent-tab-overview'))).dy;
    expect(toggleY, closeTo(tlY, 0.5));
    expect(tabY, closeTo(tlY, 0.5));
    // 绿灯在标准高度（72%）和大视野高度（86%）之间切换，太阳同步让位不重叠。
    await tester.tap(find.byKey(const ValueKey('window-expand')));
    await tester.pumpAndSettle();
    final tallSheet = tester.getRect(find.byType(AgentPanel));
    expect(tallSheet.height, greaterThan(sheet.height + 80));
    expect(tester.getRect(sun).bottom, lessThanOrEqualTo(tallSheet.top));
    await tester.tap(find.byKey(const ValueKey('window-expand')));
    await tester.pumpAndSettle();
    expect(tester.getRect(find.byType(AgentPanel)).height, closeTo(sheet.height, 0.5));
    // 点太阳是看幕僚长的员工界面（不是待办、任务那些）。
    expect(find.byType(ShellPanel), findsNothing);
    // 右栏三页和收起/展开在空位骨架下也能用。
    await tester.tap(find.byKey(const ValueKey('agent-tab-doing')));
    await tester.pumpAndSettle();
    expect(find.text('手上的任务'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('agent-tab-overview')));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('收起员工详情'));
    await tester.pumpAndSettle();
    expect(find.byType(AgentSheetContent), findsNothing);
    await tester.tap(find.byTooltip('展开员工详情'));
    await tester.pumpAndSettle();
    expect(find.byType(AgentSheetContent), findsOneWidget);

    // 再点太阳：收起，亮度退回去。
    await tester.tap(sun);
    await tester.pumpAndSettle();
    expect(selected(), isFalse);
    expect(lift(), 0.0);
    expect(find.byType(AgentSheetContent), findsNothing);

    // 点空白处（左上角的天空）收起。
    await tester.tap(sun);
    await tester.pumpAndSettle();
    await tester.tapAt(const Offset(200, 120));
    await tester.pumpAndSettle();
    expect(selected(), isFalse);
    expect(find.byType(AgentSheetContent), findsNothing);

    // 按 Esc 收起。
    await tester.tap(sun);
    await tester.pumpAndSettle();
    expect(find.byType(AgentSheetContent), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byType(AgentSheetContent), findsNothing);

    // 太阳的光芒和光晕不挡点击：点方框的角上（球外面）不会选中。
    final box = tester.getRect(sun);
    await tester.tapAt(box.topLeft + const Offset(6, 6));
    await tester.pumpAndSettle();
    expect(selected(), isFalse);
    await h.stop(tester);
  });

  testWidgets('未连接主机时点太阳仍展示三栏骨架，中间提示去连接设置，不再误报没配协调员', (tester) async {
    final h = Harness();
    h.host.failNext = Exception('offline');
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    h.host.failNext = Exception('offline');
    await tester.tap(find.byKey(const ValueKey('sun')));
    await tester.pumpAndSettle();

    expect(h.state.status, ConnStatus.closed);
    expect(find.byType(AgentPanel), findsOneWidget);
    expect(find.text('幕僚长的对话'), findsOneWidget);
    expect(find.text('未连接主机'), findsOneWidget);
    expect(find.text('当前未连接主机。'), findsOneWidget);
    expect(find.textContaining('主机上没有协调员岗位'), findsNothing);
    expect(find.byKey(const ValueKey('agent-tab-doing')), findsOneWidget);
    expect(find.byKey(const ValueKey('agent-tab-advanced')), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('agent-open-settings')));
    await tester.pumpAndSettle();
    expect(find.text('断开并忘掉这台主机'), findsOneWidget);
    await h.stop(tester);
  });

  testWidgets('左上角 Apple 红绿灯待办小组件：平时展示待办，点击展开按红黄绿筛选，可直接拍板并收成绿灯胶囊', (tester) async {
    final h = Harness();
    h.host.handlers['resolve_approval'] = (req) => approval('a1', pending: false);
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));

    // 默认假主机有 1 张路由审批卡（黄灯待定）：左上角直接显示小组件。
    expect(find.byKey(const ValueKey('inbox-widget-compact')), findsOneWidget);
    expect(find.text('这条消息该交给谁？'), findsOneWidget);
    final lights = find.byKey(const ValueKey('inbox-header-lights'));
    final title = find.byKey(const ValueKey('inbox-header-title'));
    final compactLightsPos = tester.getTopLeft(lights);
    final compactTitlePos = tester.getTopLeft(title);
    final compactTitleSize = tester.widget<Text>(title).style?.fontSize;

    // 点开原地展开，红绿灯和“待办”标题的坐标与字号 0 像素跳动。
    await tester.tap(find.byKey(const ValueKey('inbox-widget-compact')));
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('inbox-widget-expanded')), findsOneWidget);
    expect(tester.getTopLeft(lights), compactLightsPos);
    expect(tester.getTopLeft(title), compactTitlePos);
    expect(tester.widget<Text>(title).style?.fontSize, compactTitleSize);
    expect(find.text('宏观研究员（推荐）'), findsOneWidget);

    // 切到红灯、绿灯分段再切回黄灯。
    await tester.tap(find.byKey(const ValueKey('inbox-light-tab-red')));
    await tester.pumpAndSettle();
    expect(find.text('没有紧急待办。'), findsOneWidget);
    await tester.tap(find.byKey(const ValueKey('inbox-light-tab-yellow')));
    await tester.pumpAndSettle();

    // 拍板后待办清空，按 Esc 收起变成绿灯胶囊；红绿灯与“待办”位置、字号依然纹丝不动。
    await tester.tap(find.text('宏观研究员（推荐）'));
    await tester.pumpAndSettle();
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();
    expect(find.byKey(const ValueKey('inbox-widget-expanded')), findsNothing);
    expect(find.byKey(const ValueKey('inbox-widget-pill')), findsOneWidget);
    expect(find.text('暂无'), findsOneWidget);
    expect(tester.getTopLeft(lights), compactLightsPos);
    expect(tester.getTopLeft(title), compactTitlePos);
    expect(tester.widget<Text>(title).style?.fontSize, compactTitleSize);
    await h.stop(tester);
  });

  testWidgets('宽窗口右下角“面板”打开三栏面板：待办、对话、任务、岗位都能进，右栏能收起，通知点进来也打开面板', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    final sun = find.byKey(const ValueKey('sun'));
    expect(find.byType(ShellPanel), findsNothing);

    // 打开面板：先是待办，右栏默认收起；太阳不算选中（没在看幕僚长）。
    await tester.tap(find.byKey(const ValueKey('panel-bubble')));
    await tester.pumpAndSettle();
    expect(find.byType(ShellPanel), findsOneWidget);
    expect(find.byType(InboxPage), findsOneWidget);
    expect(find.text('这条消息该交给谁？'), findsOneWidget);
    expect(find.byType(AgentSheetContent), findsNothing);
    expect(tester.widget<CelestialBody>(sun).selected, isFalse);
    expect(tester.widget<CelestialBody>(sun).frozen, isTrue);

    // 右栏能展开、收起。
    await tester.tap(find.byTooltip('展开员工详情'));
    await tester.pumpAndSettle();
    expect(find.byType(AgentSheetContent), findsOneWidget);
    await tester.tap(find.byTooltip('收起员工详情'));
    await tester.pumpAndSettle();
    expect(find.byType(AgentSheetContent), findsNothing);

    // 对话：对话列表加对话，右栏默认开着。
    await tester.tap(find.byKey(const ValueKey('panel-section-chat')));
    await tester.pumpAndSettle();
    expect(find.byType(ThreadList), findsOneWidget);
    expect(find.text('最近 CPI 怎么样'), findsWidgets);
    expect(find.byType(AgentSheetContent), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('panel-section-tasks')));
    await tester.pumpAndSettle();
    expect(find.byType(TasksPage), findsOneWidget);
    expect(find.byType(AgentSheetContent), findsNothing);

    await tester.tap(find.byKey(const ValueKey('panel-section-roles')));
    await tester.pumpAndSettle();
    expect(find.byType(RolesPage), findsOneWidget);

    // 关掉再从“面板”打开：回到上次那一页。
    await tester.tap(find.byTooltip('关闭面板（Esc）'));
    await tester.pumpAndSettle();
    expect(find.byType(ShellPanel), findsNothing);
    await tester.tap(find.byKey(const ValueKey('panel-bubble')));
    await tester.pumpAndSettle();
    expect(find.byType(RolesPage), findsOneWidget);
    await tester.sendKeyEvent(LogicalKeyboardKey.escape);
    await tester.pumpAndSettle();

    // 点了通知（或“管理”里点“看对话”）：打开面板，在对话页打开那个对话。
    h.state.openThreadFromAnywhere('t1');
    await tester.pumpAndSettle();
    expect(find.byType(ShellPanel), findsOneWidget);
    expect(find.byType(ThreadList), findsOneWidget);
    expect(h.state.threadId, 't1');
    await h.stop(tester);
  });

  testWidgets('宽窗口里是天穹和左下角“我”、连接状态；点了打开设置', (tester) async {
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
