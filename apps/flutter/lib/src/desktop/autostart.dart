// 开机启动（F-17）。不用第三方包：Linux 写 XDG 自动启动的 .desktop 文件，Windows 写注册表的 Run 键，
// macOS 用系统的 SMAppService（macOS 13 以上，Swift 那边在 macos/Runner/MainFlutterWindow.swift）。
// 开机启动时带 `--hidden`，Linux、Windows 的窗口不弹出来，只在托盘里（macOS 的登录项不带参数，窗口照常打开）。

import 'dart:io';

import 'package:flutter/services.dart';

/// 开机启动时带的参数：窗口不弹出来，只在托盘里。
const kHiddenArg = '--hidden';

abstract class AutoStart {
  /// 这台电脑上能不能设开机启动；不能时托盘菜单里的那一项灰掉，并说明原因。
  Future<String?> unavailableReason();

  Future<bool> isEnabled();

  Future<void> setEnabled(bool on);
}

/// 按平台挑一个；手机上没有。
AutoStart? platformAutoStart() {
  if (Platform.isLinux) {
    final config = Platform.environment['XDG_CONFIG_HOME'];
    final home = Platform.environment['HOME'];
    final dir = config != null && config.isNotEmpty ? config : (home == null ? null : '$home/.config');
    if (dir == null) return null;
    return LinuxAutoStart(dir: '$dir/autostart', executable: Platform.resolvedExecutable);
  }
  if (Platform.isWindows) return WindowsAutoStart(executable: Platform.resolvedExecutable);
  if (Platform.isMacOS) return MacAutoStart();
  return null;
}

/// XDG 自动启动：`~/.config/autostart/<id>.desktop`，GNOME、KDE、Xfce 等都认。
class LinuxAutoStart implements AutoStart {
  LinuxAutoStart({required this.dir, required this.executable});

  final String dir;
  final String executable;

  File get file => File('$dir/dev.intelligent_position.ip_app.desktop');

  /// Exec 里的路径按 Desktop Entry 规范加引号、转义。
  static String quote(String path) => '"${path.replaceAllMapped(RegExp(r'["`$\\]'), (m) => '\\${m[0]}')}"';

  String get entry =>
      '[Desktop Entry]\n'
      'Type=Application\n'
      'Name=intelligent-position\n'
      'Comment=开机启动，只在托盘里\n'
      'Exec=${quote(executable)} $kHiddenArg\n'
      'Terminal=false\n'
      'X-GNOME-Autostart-enabled=true\n';

  @override
  Future<String?> unavailableReason() async => null;

  @override
  Future<bool> isEnabled() async {
    if (!file.existsSync()) return false;
    // 换了安装位置后旧文件指着不存在的程序，当作没开。
    return file.readAsStringSync().contains(quote(executable));
  }

  @override
  Future<void> setEnabled(bool on) async {
    if (on) {
      await Directory(dir).create(recursive: true);
      await file.writeAsString(entry);
    } else if (file.existsSync()) {
      await file.delete();
    }
  }
}

typedef ProcessRunner = Future<ProcessResult> Function(String exe, List<String> args);

/// 当前用户的 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`，用系统自带的 reg.exe 改，不用管理员权限。
class WindowsAutoStart implements AutoStart {
  WindowsAutoStart({required this.executable, ProcessRunner? run}) : _run = run ?? Process.run;

  final String executable;
  final ProcessRunner _run;

  static const key = r'HKCU\Software\Microsoft\Windows\CurrentVersion\Run';
  static const name = 'intelligent-position';

  String get command => '"$executable" $kHiddenArg';

  @override
  Future<String?> unavailableReason() async => null;

  @override
  Future<bool> isEnabled() async {
    final r = await _run('reg', ['query', key, '/v', name]);
    return r.exitCode == 0 && '${r.stdout}'.contains(executable);
  }

  @override
  Future<void> setEnabled(bool on) async {
    final r = on
        ? await _run('reg', ['add', key, '/v', name, '/t', 'REG_SZ', '/d', command, '/f'])
        : await _run('reg', ['delete', key, '/v', name, '/f']);
    // 关的时候本来就没有也算成功。
    if (r.exitCode != 0 && on) throw ProcessException('reg', const [], '${r.stderr}'.trim(), r.exitCode);
  }
}

/// macOS：`SMAppService.mainApp`，在“系统设置 > 通用 > 登录项”里能看到、能关。
class MacAutoStart implements AutoStart {
  MacAutoStart([MethodChannel? channel]) : _channel = channel ?? const MethodChannel('ip_app/autostart');

  final MethodChannel _channel;

  @override
  Future<String?> unavailableReason() async {
    try {
      final ok = await _channel.invokeMethod<bool>('available') ?? false;
      return ok ? null : '要 macOS 13 以上';
    } on Object {
      return '这个版本不支持';
    }
  }

  @override
  Future<bool> isEnabled() async => await _channel.invokeMethod<bool>('isEnabled') ?? false;

  @override
  Future<void> setEnabled(bool on) => _channel.invokeMethod<void>('setEnabled', {'on': on});
}
