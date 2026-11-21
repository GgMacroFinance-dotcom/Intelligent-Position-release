// 桌面的托盘和窗口（F-17）：关窗口藏到托盘、托盘待办数、点通知叫出窗口、托盘菜单、开机启动。

import 'dart:io';

import 'package:flutter/widgets.dart';

import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/desktop/autostart.dart';
import 'package:ip_app/src/desktop/shell.dart';

import 'fake_host.dart';
import 'harness.dart';

class FakeTray implements TrayPort {
  final pending = <(int, String)>[];
  List<TrayItem> menu = [];
  int popUps = 0;
  bool disposed = false;
  late void Function() click;
  late void Function() rightClick;
  late void Function(String) pick;

  @override
  Future<void> setup({
    required void Function() onClick,
    required void Function() onRightClick,
    required void Function(String key) onMenu,
  }) async {
    click = onClick;
    rightClick = onRightClick;
    pick = onMenu;
  }

  @override
  Future<void> setPending(int count, String tooltip) async => pending.add((count, tooltip));

  @override
  Future<void> setMenu(List<TrayItem> items) async => menu = items;

  @override
  Future<void> popUpMenu() async => popUps++;

  @override
  Future<void> dispose() async => disposed = true;

  TrayItem item(String key) => menu.firstWhere((i) => i.key == key);
}

class FakeWindow implements WindowPort {
  final calls = <String>[];
  late void Function() close;

  @override
  Future<void> setup({required void Function() onClose}) async => close = onClose;

  @override
  Future<void> show() async => calls.add('show');

  @override
  Future<void> hide() async => calls.add('hide');

  @override
  Future<void> quit() async => calls.add('quit');
}

class FakeAutoStart implements AutoStart {
  FakeAutoStart({this.reason});
  final String? reason;
  bool on = false;

  @override
  Future<String?> unavailableReason() async => reason;

  @override
  Future<bool> isEnabled() async => on;

  @override
  Future<void> setEnabled(bool value) async => on = value;
}

void main() {
  test('托盘提示', () {
    expect(trayTooltip(0), 'intelligent-position');
    expect(trayTooltip(3), 'intelligent-position：3 件待办');
  });

  testWidgets('关窗口藏到托盘；托盘显示待办数并跟着变；点托盘、点通知叫出窗口', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    final tray = FakeTray();
    final window = FakeWindow();
    final shell = DesktopShell(state: h.state, tray: tray, window: window, autostart: FakeAutoStart());
    await shell.init();
    // 假主机有一张待决定的审批卡。
    expect(tray.pending.last, (1, 'intelligent-position：1 件待办'));

    window.close();
    expect(window.calls, ['hide']);

    // 又来一张卡：数字跟着变。数没变时不重复设。
    h.host.event({'type': 'approval_changed', 'approval': approval('a2')});
    await tester.pump();
    expect(tray.pending.last.$1, 2);
    final n = tray.pending.length;
    h.state.notifyListeners();
    expect(tray.pending.length, n);

    tray.click();
    expect(window.calls.last, 'show');
    tray.rightClick();
    expect(tray.popUps, 1);

    // 窗口藏着时点了通知：叫出窗口，主界面去那个对话。
    window.calls.clear();
    h.notifier.tap('t1');
    await tester.pumpAndSettle();
    expect(window.calls, ['show']);
    expect(h.state.threadId, 't1');
    await shell.quit();
    await h.stop(tester);
  });

  testWidgets('托盘菜单：打开、开机启动、退出', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    final tray = FakeTray();
    final window = FakeWindow();
    final auto = FakeAutoStart();
    final shell = DesktopShell(state: h.state, tray: tray, window: window, autostart: auto);
    await shell.init();
    expect([for (final i in tray.menu) i.isSeparator ? '—' : i.label], ['打开 intelligent-position', '开机启动', '—', '退出']);
    expect(tray.item('autostart').checked, isFalse);

    tray.pick('autostart');
    await tester.pump();
    expect(auto.on, isTrue);
    expect(tray.item('autostart').checked, isTrue);
    tray.pick('autostart');
    await tester.pump();
    expect(auto.on, isFalse);

    tray.pick('show');
    expect(window.calls.last, 'show');

    tray.pick('quit');
    await tester.pump();
    expect(tray.disposed, isTrue);
    expect(window.calls.last, 'quit');
    // 退出后不再跟着状态改托盘。
    final n = tray.pending.length;
    h.host.event({'type': 'approval_changed', 'approval': approval('a3')});
    await tester.pump();
    expect(tray.pending.length, n);
    await h.stop(tester);
  });

  testWidgets('设不了开机启动时那一项灰掉，写明原因', (tester) async {
    final h = Harness();
    await h.savePaired();
    await h.start(tester, size: const Size(1280, 800));
    final tray = FakeTray();
    final shell = DesktopShell(
      state: h.state,
      tray: tray,
      window: FakeWindow(),
      autostart: FakeAutoStart(reason: '要 macOS 13 以上'),
    );
    await shell.init();
    expect(tray.item('autostart').label, '开机启动（要 macOS 13 以上）');
    expect(tray.item('autostart').enabled, isFalse);
    await shell.quit();
    await h.stop(tester);
  });

  group('开机启动', () {
    test('Linux：写、读、删 XDG 自动启动文件，路径加引号', () async {
      final dir = await Directory.systemTemp.createTemp('autostart');
      addTearDown(() => dir.delete(recursive: true));
      final a = LinuxAutoStart(dir: '${dir.path}/autostart', executable: r'/opt/ip app/ip_app');
      expect(await a.isEnabled(), isFalse);
      await a.setEnabled(true);
      expect(await a.isEnabled(), isTrue);
      final text = a.file.readAsStringSync();
      expect(text, contains('Exec="/opt/ip app/ip_app" --hidden\n'));
      expect(text, startsWith('[Desktop Entry]\n'));
      // 换了安装位置：旧文件当作没开。
      expect(await LinuxAutoStart(dir: '${dir.path}/autostart', executable: '/usr/bin/ip_app').isEnabled(), isFalse);
      await a.setEnabled(false);
      expect(a.file.existsSync(), isFalse);
      await a.setEnabled(false);
      expect(LinuxAutoStart.quote(r'/a"b$c\d'), r'"/a\"b\$c\\d"');
    });

    test('Windows：用 reg 改当前用户的 Run 键', () async {
      final calls = <List<String>>[];
      var exists = false;
      final a = WindowsAutoStart(
        executable: r'C:\Program Files\ip\ip_app.exe',
        run: (exe, args) async {
          calls.add([exe, ...args]);
          final out = exists ? r'intelligent-position REG_SZ "C:\Program Files\ip\ip_app.exe" --hidden' : '';
          if (args.first == 'add') exists = true;
          if (args.first == 'delete') exists = false;
          return ProcessResult(0, args.first == 'query' && !exists ? 1 : 0, out, '');
        },
      );
      expect(await a.isEnabled(), isFalse);
      await a.setEnabled(true);
      expect(calls.last, [
        'reg',
        'add',
        r'HKCU\Software\Microsoft\Windows\CurrentVersion\Run',
        '/v',
        'intelligent-position',
        '/t',
        'REG_SZ',
        '/d',
        r'"C:\Program Files\ip\ip_app.exe" --hidden',
        '/f',
      ]);
      expect(await a.isEnabled(), isTrue);
      await a.setEnabled(false);
      expect(calls.last.sublist(0, 2), ['reg', 'delete']);
    });
  });
}
