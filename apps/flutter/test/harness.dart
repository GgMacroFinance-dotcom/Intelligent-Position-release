// 界面测试用的整套 App：假主机、内存里的安全存储、假的配对。

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/main.dart';
import 'package:ip_app/src/app_state.dart';
import 'package:ip_app/src/host.dart';
import 'package:ip_app/src/notifications.dart';
import 'package:ip_app/src/pairing.dart';
import 'package:ip_app/src/protocol/client.dart';
import 'package:ip_app/src/protocol/models.dart';
import 'package:ip_app/src/storage.dart';
import 'package:ip_app/src/ui/me_page.dart';

import 'fake_host.dart';

class Harness {
  Harness() {
    installDefaults(host);
    state = AppState(
      store: HostStore(secrets),
      notifier: notifier,
      clientFactory: (h, token) => IpClient(wsUri: h.wsUri, token: token, connector: host.connect),
      pairer: (h, code) async {
        paired.add((h, code));
        if (code != '123456') throw const PairingError('配对码不对、过期了或者已经用过');
        return Claimed.fromJson({
          'token': 'ipd_abc',
          'device': {'id': 'd1', 'name': '张三的手机', 'scope': 'daily'},
        });
      },
    );
  }

  final host = FakeHost();
  final notifier = FakeNotifier();
  final secrets = MemorySecretStore();
  final paired = <(HostAddress, String)>[];
  late final AppState state;

  Future<void> start(WidgetTester tester, {Size size = const Size(390, 844)}) async {
    tester.view.physicalSize = size;
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(IpApp(state: state));
    await state.init();
    await tester.pumpAndSettle();
  }

  /// 已经配对过：存储里有主机和令牌。
  Future<void> savePaired() =>
      HostStore(secrets)
          .save(HostEntry(address: HostAddress.parse('https://ip.example.com'), deviceName: '张三的手机'), 'ipd_abc');

  /// 在“我”页里往下滚到 `finder`（后面的主界面里也有能滚的列表，要指定“我”页的）。
  Future<void> scrollMe(WidgetTester tester, Finder finder) => tester.scrollUntilVisible(
    finder,
    200,
    scrollable: find.descendant(of: find.byType(MePage), matching: find.byType(Scrollable)).first,
  );

  Future<void> stop(WidgetTester tester) async {
    await tester.pumpWidget(const SizedBox());
    state.dispose();
    await tester.pump(const Duration(seconds: 1));
  }
}

/// 记下弹了什么、撤了什么，能模拟人点通知。
class FakeNotifier implements SystemNotifier {
  final shown = <Alert>[];
  final cancelled = <String>[];
  bool allow = true;
  int asked = 0;
  AlertTap? _onTap;

  void tap(String? threadId) => _onTap!(threadId);

  @override
  Future<void> init(AlertTap onTap) async => _onTap = onTap;

  @override
  Future<bool> requestPermission() async {
    asked++;
    return allow;
  }

  @override
  Future<void> show(Alert alert) async => shown.add(alert);

  @override
  Future<void> cancel(String tag) async => cancelled.add(tag);
}
