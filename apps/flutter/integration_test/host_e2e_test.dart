// 连一台真主机的端到端测试：在真的 App 里配对、看待办、开新对话、发消息、等员工流式回复，最后忘掉主机。
// 用系统真的安全存储和网络，不在 CI 跑（要一台主机和一个新鲜的配对码）。用法见 apps/flutter/README.md。
//
//   flutter test integration_test -d <设备> \
//     --dart-define=IP_E2E_URL=https://ip.example.com --dart-define=IP_E2E_CODE=123456
//
// 可选 `IP_E2E_SHOTS=<目录>`：每一步截一张屏，只在 Linux（X11，要装 ImageMagick 的 import）上有用。

import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:ip_app/main.dart';
import 'package:ip_app/src/app_state.dart';
import 'package:ip_app/src/host.dart';
import 'package:ip_app/src/protocol/client.dart';
import 'package:ip_app/src/protocol/models.dart';
import 'package:ip_app/src/storage.dart';
import 'package:ip_app/src/ui/inbox_page.dart';

const url = String.fromEnvironment('IP_E2E_URL');
const code = String.fromEnvironment('IP_E2E_CODE');
const question = String.fromEnvironment('IP_E2E_TEXT', defaultValue: '9 月美国 CPI 同比多少？');
const shots = String.fromEnvironment('IP_E2E_SHOTS');

/// 一句路由规则匹配不上的话，用来走一遍决策卡；给空字符串跳过这一段。
const unrouted = String.fromEnvironment('IP_E2E_UNROUTED', defaultValue: '帮我整理一下这周要关注的事');

/// 一直 pump 到条件满足；超时就失败，并说明在等什么。
Future<void> pumpUntil(WidgetTester tester, bool Function() done, String what, {int seconds = 60}) async {
  final end = DateTime.now().add(Duration(seconds: seconds));
  while (!done()) {
    if (DateTime.now().isAfter(end)) fail('等了 $seconds 秒，$what');
    await tester.pump(const Duration(milliseconds: 100));
  }
}

/// 窄屏上是右下角的按钮，宽屏上是列表顶上的一行。
Future<void> tapNewChat(WidgetTester tester) async {
  await backIfPushed(tester);
  final button = find.byTooltip('新对话');
  await tester.tap(button.evaluate().isNotEmpty ? button : find.text('新对话').first);
  await tester.pumpAndSettle();
}

/// 往输入框里写字。桌面上的集成测试里，`enterText` 在输入框一直有焦点时会把字送丢，所以直接改输入框的内容；
/// 这个测试测的是和主机的来回，真键盘输入另外在真机上试。
void typeInto(WidgetTester tester, Finder field, String text) {
  tester.widget<TextField>(field).controller!.text = text;
}

/// 窄屏上对话是推进去的一屏，先返回主界面。
Future<void> backIfPushed(WidgetTester tester) async {
  final back = find.byType(BackButton);
  if (back.evaluate().isEmpty) return;
  // pageBack() 按英文提示“Back”找，界面是中文的。
  await tester.tap(back);
  await tester.pumpAndSettle();
}

var _shot = 0;
Future<void> shot(String name) async {
  if (shots.isEmpty || !Platform.isLinux) return;
  _shot++;
  await Process.run('import', ['-window', 'root', '$shots/${_shot.toString().padLeft(2, '0')}-$name.png']);
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('配对、待办、发消息、流式回复', (tester) async {
    if (url.isEmpty || code.isEmpty) {
      markTestSkipped('没给 IP_E2E_URL 和 IP_E2E_CODE，跳过');
      return;
    }
    final address = HostAddress.parse(url);
    final store = HostStore(SystemSecretStore());
    // 上次没跑完留下的，先清掉。
    await store.forget(address);
    final state = AppState(store: store);
    await tester.pumpWidget(IpApp(state: state));
    await state.init();
    await tester.pumpAndSettle();
    await shot('connect');

    await tester.enterText(find.widgetWithText(TextField, '主机地址'), url);
    await tester.enterText(find.widgetWithText(TextField, '配对码'), code);
    await tester.tap(find.text('配对并连接'));
    await tester.pumpAndSettle();
    if (address.needsTrust) {
      await tester.tap(find.text('我信任这个网络'));
      await tester.pumpAndSettle();
    }
    await shot('confirm');
    await tester.tap(find.widgetWithText(FilledButton, '连接'));
    await pumpUntil(
      tester,
      () => state.connected && state.me != null,
      '还没连上主机：${state.rejected?.message ?? state.status}',
    );
    expect(state.me?.device, isNotNull, reason: '用设备令牌连上时 who_am_i 应该回这台设备');
    expect(await store.token(address), startsWith('ipd_'));
    await tester.pumpAndSettle();
    await shot('inbox');

    await tester.tap(find.text('对话').last);
    await tester.pumpAndSettle();
    await tapNewChat(tester);
    typeInto(tester, find.widgetWithText(TextField, '写给员工……'), question);
    await tester.tap(find.byTooltip('发送'));
    await pumpUntil(tester, () => state.threadId != null, '消息没发出去');
    final thread = state.threadId!;

    // 先看到流式的“正在输入”，再看到员工的整条回复。
    await pumpUntil(tester, () => state.drafts.isNotEmpty || _agentReplied(state), '员工没开始回复');
    if (state.drafts.isNotEmpty) await shot('streaming');
    await pumpUntil(tester, () => _agentReplied(state) && state.drafts.isEmpty, '员工没回完', seconds: 180);
    await pumpUntil(tester, () => state.turnNote[thread]?.contains('完成') ?? false, '没收到一轮结束', seconds: 30);
    await tester.pumpAndSettle();
    await shot('reply');

    final mine = state.messages.firstWhere((m) => m.author is HumanAuthor);
    expect((mine.author as HumanAuthor).name, state.me!.device!.name, reason: '设备发的消息应该署设备名');

    // 路由规则匹配不上、又没配语义分派时，主机发决策卡给人：在待办里选定岗位后，员工接着处理。
    // 宽屏和窄屏（手机）都能走：窄屏上新对话推进新的一屏，要先返回。
    if (state.hasRules && unrouted.isNotEmpty) {
      final before = {for (final a in state.pendingApprovals) a.id};
      await tapNewChat(tester);
      typeInto(tester, find.widgetWithText(TextField, '写给员工……'), unrouted);
      await tester.tap(find.byTooltip('发送'));
      await tester.pump(const Duration(milliseconds: 500));
      await shot('sent');
      // 只认这次新出的、属于这个对话的卡；决策卡（事件）可能比 send 的回复先到。
      bool mine(Approval a) => !before.contains(a.id) && a.subjectThreadId == state.threadId;
      await pumpUntil(
        tester,
        () => state.threadId != null && state.pendingApprovals.any(mine),
        '没出决策卡（对话 ${state.threadId}，待决定 ${state.pendingApprovals.length} 张）',
      );
      final routedThread = state.threadId!;
      await tester.pumpAndSettle();
      await shot('routing-note');
      await backIfPushed(tester);
      await tester.tap(find.text('待办').last);
      await tester.pumpAndSettle();
      await shot('inbox-card');
      // 卡片按发起时间新的在前，最上面那张就是这次的。选第二个选项。
      final ours = state.pendingApprovals.firstWhere(mine);
      expect(state.pendingApprovals.first.id, ours.id);
      final option = ours.options[1].label;
      await tester.tap(
        find.descendant(of: find.byType(ApprovalCard).first, matching: find.textContaining(option)).first,
      );
      await pumpUntil(tester, () => !state.pendingApprovals.any((a) => a.id == ours.id), '选了没生效');
      await pumpUntil(
        tester,
        () => state.threadId == routedThread && _agentReplied(state) && state.drafts.isEmpty,
        '选定后员工没接着回复',
        seconds: 180,
      );
      await tester.tap(find.text('对话').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byType(ListTile).at(1));
      await tester.pumpAndSettle();
      await shot('routed-thread');
    }

    // 收尾：忘掉主机，回到“连接主机”。主机那边的设备记录要在控制台“设备”页吊销。
    await backIfPushed(tester);
    await tester.tap(find.text('我').last);
    await tester.pumpAndSettle();
    await tester.tap(find.text('断开并忘掉这台主机'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(FilledButton, '断开并忘掉'));
    await pumpUntil(tester, () => state.host == null, '没回到“连接主机”');
    await tester.pumpAndSettle();
    expect(find.text('连接主机'), findsOneWidget);
    expect(await store.token(address), isNull);
    expect(state.status, ConnStatus.closed);
  });
}

bool _agentReplied(AppState s) => s.messages.any((m) => m.author is AgentAuthor && m.body is TextBody);
