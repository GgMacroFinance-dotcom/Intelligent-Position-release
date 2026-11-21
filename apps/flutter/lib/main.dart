// intelligent-position 的各平台 App（docs/ip/apps-plan.md 第 9 节）。

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import 'src/app_state.dart';
import 'src/desktop/autostart.dart';
import 'src/desktop/shell.dart';
import 'src/notifications.dart';
import 'src/qr/scanner.dart';
import 'src/storage.dart';
import 'src/ui/common.dart';
import 'src/ui/connect_page.dart';
import 'src/ui/home_page.dart';

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  final state = AppState(
    store: HostStore(SystemSecretStore()),
    notifier: LocalNotifier(),
    scanner: platformQrScanner(),
  );
  runApp(IpApp(state: state));
  state.init();
  // 桌面：关窗口留在托盘、托盘显示待办数、开机启动（F-17）。
  if (isDesktop) {
    unawaited(
      DesktopShell(
        state: state,
        tray: SystemTray(),
        window: SystemWindow(),
        autostart: platformAutoStart(),
      ).init().catchError((Object e) => debugPrint('托盘用不了：$e')),
    );
  }
}

/// 和网页控制台一样的主色。
const _accent = Color(0xFF2F5BD3);

class IpApp extends StatefulWidget {
  const IpApp({super.key, required this.state});

  final AppState state;

  @override
  State<IpApp> createState() => _IpAppState();
}

class _IpAppState extends State<IpApp> {
  late final AppLifecycleListener _lifecycle;

  @override
  void initState() {
    super.initState();
    // 手机从后台回来时连接多半已经断了，不等退避马上重连。
    // 前台是 resumed：桌面上窗口失去焦点是 inactive，这时和网页控制台一样算不在前台，来了通知要弹。
    _lifecycle = AppLifecycleListener(
      onResume: widget.state.resumed,
      onStateChange: (s) => widget.state.setForeground(s == AppLifecycleState.resumed),
    );
  }

  @override
  void dispose() {
    _lifecycle.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AppScope(
      state: widget.state,
      child: MaterialApp(
        title: 'intelligent-position',
        debugShowCheckedModeBanner: false,
        theme: ThemeData(colorSchemeSeed: _accent, useMaterial3: true),
        darkTheme: ThemeData(colorSchemeSeed: _accent, brightness: Brightness.dark, useMaterial3: true),
        locale: const Locale('zh', 'CN'),
        supportedLocales: const [Locale('zh', 'CN')],
        localizationsDelegates: GlobalMaterialLocalizations.delegates,
        home: const _Root(),
      ),
    );
  }
}

class _Root extends StatefulWidget {
  const _Root();

  @override
  State<_Root> createState() => _RootState();
}

class _RootState extends State<_Root> {
  bool _hadHost = false;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    if (!state.ready) return const Scaffold();
    final hasHost = state.host != null;
    // 忘掉主机（“我”页、顶上的“重新配对”）时，推进去的页面（“我”、对话、报告）一起关掉，回到“连接主机”。
    if (_hadHost && !hasHost) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (mounted) Navigator.of(context).popUntil((r) => r.isFirst);
      });
    }
    _hadHost = hasHost;
    // 换了主机时主界面整个重建，不带着上一台主机的页面状态。
    return hasHost ? HomePage(key: ValueKey(state.host!.address)) : const ConnectPage();
  }
}
