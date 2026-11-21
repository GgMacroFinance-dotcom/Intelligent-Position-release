// 桌面（macOS、Windows、Linux）的托盘和窗口（F-17），和 Tauri 版（apps/desktop）做的一样：
// 关窗口只是藏到托盘，连接不断，照样弹通知（F-16）；托盘显示待办数；托盘菜单里开关开机启动、退出。
// 只开一个：再打开一次时把开着的那个窗口叫出来，这在各平台的 runner 里做（linux/runner、windows/runner；macOS 本来就只开一个）。

import 'dart:async';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:tray_manager/tray_manager.dart' as tm;
import 'package:window_manager/window_manager.dart' as wm;

import '../app_state.dart';
import 'autostart.dart';

bool get isDesktop => !kIsWeb && (Platform.isMacOS || Platform.isWindows || Platform.isLinux);

/// 托盘菜单的一项。
class TrayItem {
  const TrayItem(this.key, this.label, {this.checked, this.enabled = true});
  const TrayItem.separator() : key = '', label = '', checked = null, enabled = true;

  final String key;
  final String label;

  /// 不为空时是能勾的一项。
  final bool? checked;
  final bool enabled;

  bool get isSeparator => key.isEmpty;
}

/// 托盘那一层。测试里换成假的。
abstract class TrayPort {
  Future<void> setup({
    required void Function() onClick,
    required void Function() onRightClick,
    required void Function(String key) onMenu,
  });

  /// 托盘上的待办数：提示文字，macOS、Linux 图标旁边再显示数字（Windows 的托盘图标旁边不能显示文字）。
  Future<void> setPending(int count, String tooltip);

  Future<void> setMenu(List<TrayItem> items);

  Future<void> popUpMenu();

  Future<void> dispose();
}

/// 窗口那一层。测试里换成假的。
abstract class WindowPort {
  /// 拦下关窗口：点关闭时回调 `onClose`，窗口不真的关。
  Future<void> setup({required void Function() onClose});
  Future<void> show();
  Future<void> hide();

  /// 真的退出。
  Future<void> quit();
}

/// 托盘提示：没有待办时只有名字。
String trayTooltip(int count) => count == 0 ? 'intelligent-position' : 'intelligent-position：$count 件待办';

class DesktopShell {
  DesktopShell({required this.state, required this.tray, required this.window, this.autostart});

  final AppState state;
  final TrayPort tray;
  final WindowPort window;
  final AutoStart? autostart;

  int _count = -1;
  int _taps = 0;
  bool _autostartOn = false;
  String? _autostartUnavailable;

  Future<void> init() async {
    await window.setup(onClose: () => unawaited(window.hide()));
    await tray.setup(onClick: _show, onRightClick: () => unawaited(tray.popUpMenu()), onMenu: _onMenu);
    final a = autostart;
    if (a == null) {
      _autostartUnavailable = '这个平台不支持';
    } else {
      try {
        _autostartUnavailable = await a.unavailableReason();
        if (_autostartUnavailable == null) _autostartOn = await a.isEnabled();
      } catch (e) {
        _autostartUnavailable = '读不到设置';
        debugPrint('读开机启动设置失败：$e');
      }
    }
    await _setMenu();
    state.addListener(_onState);
    _onState();
  }

  void _show() => unawaited(window.show());

  void _onState() {
    final n = state.inboxCount;
    if (n != _count) {
      _count = n;
      unawaited(tray.setPending(n, trayTooltip(n)));
    }
    // 点了系统通知：先把窗口叫出来，主界面再去那个对话（home_page.dart）。
    if (state.alertTaps != _taps) {
      _taps = state.alertTaps;
      _show();
    }
  }

  Future<void> _setMenu() => tray.setMenu([
    const TrayItem('show', '打开 intelligent-position'),
    TrayItem(
      'autostart',
      _autostartUnavailable == null ? '开机启动' : '开机启动（$_autostartUnavailable）',
      checked: _autostartOn,
      enabled: _autostartUnavailable == null,
    ),
    const TrayItem.separator(),
    const TrayItem('quit', '退出'),
  ]);

  Future<void> _onMenu(String key) async {
    switch (key) {
      case 'show':
        _show();
      case 'autostart':
        final a = autostart;
        if (a == null || _autostartUnavailable != null) return;
        try {
          await a.setEnabled(!_autostartOn);
          _autostartOn = await a.isEnabled();
        } catch (e) {
          debugPrint('改开机启动失败：$e');
        }
        await _setMenu();
      case 'quit':
        await quit();
    }
  }

  Future<void> quit() async {
    state.removeListener(_onState);
    await tray.dispose();
    await window.quit();
  }
}

// ---------- 用 tray_manager、window_manager 的实现 ----------

class SystemTray with tm.TrayListener implements TrayPort {
  void Function()? _onClick;
  void Function()? _onRightClick;
  void Function(String key)? _onMenu;

  @override
  Future<void> setup({
    required void Function() onClick,
    required void Function() onRightClick,
    required void Function(String key) onMenu,
  }) async {
    _onClick = onClick;
    _onRightClick = onRightClick;
    _onMenu = onMenu;
    tm.trayManager.addListener(this);
    await tm.trayManager.setIcon(Platform.isWindows ? 'assets/tray/tray.ico' : 'assets/tray/tray.png');
  }

  @override
  Future<void> setPending(int count, String tooltip) async {
    // Linux 没有提示文字，Windows 不能在图标旁边显示文字：不支持的平台抛错，忽略。
    try {
      await tm.trayManager.setToolTip(tooltip);
    } catch (_) {}
    if (!Platform.isWindows) {
      try {
        await tm.trayManager.setTitle(count == 0 ? '' : '$count');
      } catch (_) {}
    }
  }

  @override
  Future<void> setMenu(List<TrayItem> items) => tm.trayManager.setContextMenu(
    tm.Menu(
      items: [
        for (final i in items)
          if (i.isSeparator)
            tm.MenuItem.separator()
          else if (i.checked != null)
            tm.MenuItem.checkbox(key: i.key, label: i.label, checked: i.checked, disabled: !i.enabled)
          else
            tm.MenuItem(key: i.key, label: i.label, disabled: !i.enabled),
      ],
    ),
  );

  @override
  Future<void> popUpMenu() => tm.trayManager.popUpContextMenu();

  @override
  Future<void> dispose() async {
    tm.trayManager.removeListener(this);
    await tm.trayManager.destroy();
  }

  // Linux 的托盘图标收不到点击，点了直接出菜单（appindicator 的做法）。
  @override
  void onTrayIconMouseDown() => _onClick?.call();

  @override
  void onTrayIconRightMouseDown() => _onRightClick?.call();

  @override
  void onTrayMenuItemClick(tm.MenuItem menuItem) {
    final k = menuItem.key;
    if (k != null) _onMenu?.call(k);
  }
}

class SystemWindow with wm.WindowListener implements WindowPort {
  void Function()? _onClose;

  @override
  Future<void> setup({required void Function() onClose}) async {
    _onClose = onClose;
    await wm.windowManager.ensureInitialized();
    wm.windowManager.addListener(this);
    await wm.windowManager.setPreventClose(true);
  }

  @override
  void onWindowClose() => _onClose?.call();

  @override
  Future<void> show() async {
    await wm.windowManager.show();
    await wm.windowManager.focus();
  }

  @override
  Future<void> hide() => wm.windowManager.hide();

  @override
  Future<void> quit() async {
    wm.windowManager.removeListener(this);
    await wm.windowManager.setPreventClose(false);
    await wm.windowManager.destroy();
  }
}
