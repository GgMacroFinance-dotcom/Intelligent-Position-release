// 锁屏推送（F-21）：连上主机后向中转登记、告诉主机订阅地址，令牌变了换新地址，开关，点通知。
// 原生那一半（FCM、APNs、解密、弹通知）和推送中转都换成假的。

import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/push/lock_push.dart';
import 'package:ip_app/src/storage.dart';
import 'package:ip_app/src/ui/me_page.dart';

import 'harness.dart';

class FakeChannel implements PushChannel {
  bool isAvailable = true;
  String token = 'fcm-token-1';
  int resets = 0;
  String? tapped;
  // ignore: close_sinks
  final tapsCtl = StreamController<String>.broadcast();
  // ignore: close_sinks
  final tokensCtl = StreamController<void>.broadcast();

  @override
  String get platform => 'fcm';

  @override
  Future<bool> available() async => isAvailable;

  @override
  Future<PushRegistration> registration() async => PushRegistration(token: token, p256dh: 'BPUB', auth: 'AUTH');

  @override
  Future<void> reset() async => resets++;

  @override
  Future<String?> takeTapped() async {
    final t = tapped;
    tapped = null;
    return t;
  }

  @override
  Stream<String> get taps => tapsCtl.stream;

  @override
  Stream<void> get tokenChanges => tokensCtl.stream;
}

/// 假中转：每次登记给一个新地址，记下收到的令牌。
class FakeRelay {
  final registered = <(String, String)>[];

  Future<String> register(Uri relay, String platform, String token) async {
    registered.add((platform, token));
    return 'https://push.example.com/w/sealed${registered.length}';
  }
}

({Harness h, FakeChannel channel, FakeRelay relay, LockPush push}) setup() {
  final channel = FakeChannel();
  final relay = FakeRelay();
  final push = LockPush(
    channel: channel,
    relay: Uri.parse('https://push.example.com'),
    store: MemorySecretStore(),
    register: relay.register,
  );
  final h = Harness(lockPush: push);
  h.host.handlers['subscribe_push'] = (_) => null;
  h.host.handlers['unsubscribe_push'] = (_) => null;
  return (h: h, channel: channel, relay: relay, push: push);
}

List<Map<String, Object?>> pushRequests(Harness h) => [
  for (final r in h.host.requests)
    if (r['type'] == 'subscribe_push' || r['type'] == 'unsubscribe_push') {...r}..remove('id'),
];

void main() {
  test('订阅地址要在同一个中转上', () {
    final relay = Uri.parse('https://push.example.com');
    expect(sameRelay(relay, 'https://push.example.com/w/abc'), isTrue);
    expect(sameRelay(relay, 'https://evil.example.com/w/abc'), isFalse);
    expect(sameRelay(relay, 'http://push.example.com/w/abc'), isFalse);
    expect(sameRelay(relay, 'https://push.example.com:8443/w/abc'), isFalse);
  });

  test('原生通道 ip/push：Android、iPhone 同一套方法名和参数', () async {
    TestWidgetsFlutterBinding.ensureInitialized();
    final messenger = TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    const ch = MethodChannel('ip/push');
    final calls = <String>[];
    Object? registration = {'token': '0a1b', 'p256dh': 'BPUB', 'auth': 'AUTH'};
    messenger.setMockMethodCallHandler(ch, (call) async {
      calls.add(call.method);
      return switch (call.method) {
        'available' => true,
        'registration' => registration,
        'takeTapped' => 't9',
        _ => null,
      };
    });
    addTearDown(() => messenger.setMockMethodCallHandler(ch, null));

    final native = NativePushChannel('apns');
    expect(native.platform, 'apns');
    expect(await native.available(), isTrue);
    final reg = await native.registration();
    expect((reg.token, reg.p256dh, reg.auth), ('0a1b', 'BPUB', 'AUTH'));
    expect(await native.takeTapped(), 't9');
    await native.reset();
    expect(calls, ['available', 'registration', 'takeTapped', 'reset']);
    registration = null;
    await expectLater(native.registration(), throwsA(isA<PushError>()));

    // 原生那边调过来的：点了通知（空串是去“待办”）、令牌变了。
    final taps = <String>[];
    var changed = 0;
    native.taps.listen(taps.add);
    native.tokenChanges.listen((_) => changed++);
    Future<void> fromNative(String method, [Object? arg]) => messenger.handlePlatformMessage(
      'ip/push',
      const StandardMethodCodec().encodeMethodCall(MethodCall(method, arg)),
      (_) {},
    );
    await fromNative('tapped', 't1');
    await fromNative('tapped', '');
    await fromNative('tokenChanged');
    await pumpEventQueue();
    expect(taps, ['t1', '']);
    expect(changed, 1);
  });

  testWidgets('连上后登记，告诉主机订阅地址；重连时同一个地址再告诉一次，不重新登记', (tester) async {
    final (:h, :channel, :relay, :push) = setup();
    await h.savePaired();
    await h.start(tester);
    expect(relay.registered, [('fcm', 'fcm-token-1')]);
    expect(pushRequests(h), [
      {'type': 'subscribe_push', 'endpoint': 'https://push.example.com/w/sealed1', 'p256dh': 'BPUB', 'auth': 'AUTH'},
    ]);
    expect(push.state, LockPushState.on);

    // 断开重连：主机的订阅可能丢了，再登记一次同一个地址。
    await h.host.last.close();
    await tester.pump(const Duration(seconds: 1));
    await tester.pumpAndSettle();
    expect(relay.registered, hasLength(1));
    expect(pushRequests(h).where((r) => r['type'] == 'subscribe_push'), hasLength(2));
    expect(pushRequests(h).last['endpoint'], 'https://push.example.com/w/sealed1');
    await h.stop(tester);
  });

  testWidgets('推送令牌变了：换新地址，删掉主机上的旧订阅', (tester) async {
    final (:h, :channel, :relay, :push) = setup();
    await h.savePaired();
    await h.start(tester);
    channel.token = 'fcm-token-2';
    channel.tokensCtl.add(null);
    await tester.pumpAndSettle();
    expect(relay.registered.last, ('fcm', 'fcm-token-2'));
    expect(pushRequests(h).skip(1).toList(), [
      {'type': 'subscribe_push', 'endpoint': 'https://push.example.com/w/sealed2', 'p256dh': 'BPUB', 'auth': 'AUTH'},
      {'type': 'unsubscribe_push', 'endpoint': 'https://push.example.com/w/sealed1'},
    ]);
    await h.stop(tester);
  });

  testWidgets('系统没给通知权限时不登记；“我”页的开关关掉会删订阅和密钥', (tester) async {
    final (:h, :channel, :relay, :push) = setup();
    h.notifier.allow = false;
    await h.savePaired();
    await h.start(tester);
    expect(relay.registered, isEmpty);
    expect(push.state, LockPushState.off);

    h.notifier.allow = true;
    await h.state.askNotificationPermission();
    await tester.pumpAndSettle();
    expect(push.state, LockPushState.on);

    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    await h.scrollMe(tester, find.text('锁屏推送'));
    expect(find.textContaining('锁屏以后也能收到'), findsOneWidget);
    await tester.tap(find.text('锁屏推送'));
    await tester.pumpAndSettle();
    expect(pushRequests(h).last, {'type': 'unsubscribe_push', 'endpoint': 'https://push.example.com/w/sealed1'});
    expect(channel.resets, 1);
    expect(push.state, LockPushState.off);
    expect(await push.enabled(), isFalse);

    // 再打开：重新登记（旧记录已删）。
    await tester.tap(find.text('锁屏推送'));
    await tester.pumpAndSettle();
    expect(relay.registered, hasLength(2));
    expect(push.state, LockPushState.on);
    await h.stop(tester);
  });

  testWidgets('登记失败时“我”页写出原因；构建没配推送时开关灰着', (tester) async {
    final (:h, :channel, :relay, :push) = setup();
    h.host.handlers.remove('subscribe_push');
    await h.savePaired();
    await h.start(tester);
    expect(push.state, LockPushState.failed);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    await h.scrollMe(tester, find.text('锁屏推送'));
    expect(find.textContaining('登记失败'), findsOneWidget);
    await h.stop(tester);

    final other = setup();
    other.channel.isAvailable = false;
    await other.h.savePaired();
    await other.h.start(tester);
    expect(other.push.state, LockPushState.unavailable);
    expect(other.relay.registered, isEmpty);
    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    await other.h.scrollMe(tester, find.text('锁屏推送'));
    expect(find.text('这个版本没有配系统推送'), findsOneWidget);
    final tile = tester.widget<SwitchListTile>(find.widgetWithText(SwitchListTile, '锁屏推送'));
    expect(tile.onChanged, isNull);
    await other.h.stop(tester);
  });

  testWidgets('点锁屏推送的通知：开着时去那个对话；被它打开时也一样', (tester) async {
    final (:h, :channel, :relay, :push) = setup();
    channel.tapped = '';
    await h.savePaired();
    await h.start(tester);
    // 被点通知打开、没有对话：去“待办”（takePendingOpen 已被主界面取走，这里看结果）。
    expect(find.text('等你决定（1）'), findsOneWidget);

    await tester.tap(find.byTooltip('我'));
    await tester.pumpAndSettle();
    channel.tapsCtl.add('t1');
    await tester.pumpAndSettle();
    expect(find.byType(MePage), findsNothing);
    expect(h.state.threadId, 't1');
    await h.stop(tester);
  });

  testWidgets('忘掉主机时删掉订阅和密钥', (tester) async {
    final (:h, :channel, :relay, :push) = setup();
    await h.savePaired();
    await h.start(tester);
    await h.state.forgetHost();
    await tester.pumpAndSettle();
    expect(pushRequests(h).last, {'type': 'unsubscribe_push', 'endpoint': 'https://push.example.com/w/sealed1'});
    expect(channel.resets, 1);
    await h.stop(tester);
  });
}
