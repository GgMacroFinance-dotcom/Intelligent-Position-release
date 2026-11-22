// 锁屏推送（backlog F-21，m2-spec D-35）：App 被系统杀掉、手机锁屏以后也能收到主机的通知。
//
// 主机照旧发 Web Push（D-33），收件的“推送服务”是项目运营的推送中转（rust/crates/ip-relay）。这里做登记：
// 向系统要推送令牌（Android 是 FCM，iPhone 是 APNs），连同这台手机自己的 Web Push 公钥交给中转换一个订阅地址，
// 再用 `subscribe_push` 告诉主机。收推送、解密、弹通知在原生代码里做（Android：IpPush.kt；
// iPhone：通知扩展 ios/NotificationService），不用启动 Flutter。私钥只在手机上。

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import '../protocol/client.dart';
import '../storage.dart';

/// 推送中转的地址，构建时用 `--dart-define=IP_PUSH_RELAY=https://…` 给。没给时没有锁屏推送。
const pushRelay = String.fromEnvironment('IP_PUSH_RELAY');

/// 向系统和中转登记要用的东西。
class PushRegistration {
  const PushRegistration({required this.token, required this.p256dh, required this.auth});

  /// 系统的设备推送令牌。
  final String token;

  /// 这台手机的 Web Push 公钥（65 字节未压缩，base64url）和 16 字节的 auth。
  final String p256dh;
  final String auth;
}

/// 系统推送通道的原生那一半。测试里换成假的。
abstract interface class PushChannel {
  /// 交给中转的平台名：`fcm` 或 `apns`。
  String get platform;

  /// 这个构建配了系统推送没有（Android 要在构建时给 Firebase 的配置，iPhone 要带推送权利签名）。
  Future<bool> available();

  /// 拿推送令牌和这台手机的 Web Push 公钥；没有密钥时原生那边生成一套。
  Future<PushRegistration> registration();

  /// 关掉：删掉密钥和推送令牌，之后发来的都解不开。
  Future<void> reset();

  /// App 是被点锁屏推送的通知打开的：要去的对话（空串是去“待办”）。只给一次。
  Future<String?> takeTapped();

  /// App 开着时点了锁屏推送的通知。
  Stream<String> get taps;

  /// 推送令牌变了，旧的订阅地址作废，要重新登记。
  Stream<void> get tokenChanges;
}

/// 原生那一半，经方法通道 `ip/push`，两个平台的约定一样。Android 在 android/app/src/main/kotlin/…/IpPush.kt、
/// MainActivity.kt；iPhone 在 ios/Runner/IpPush.swift、AppDelegate.swift。
class NativePushChannel implements PushChannel {
  NativePushChannel(this.platform) {
    _channel.setMethodCallHandler((call) async {
      switch (call.method) {
        case 'tapped':
          _taps.add(call.arguments as String? ?? '');
        case 'tokenChanged':
          _tokens.add(null);
      }
    });
  }

  static const _channel = MethodChannel('ip/push');
  final _taps = StreamController<String>.broadcast();
  final _tokens = StreamController<void>.broadcast();

  @override
  final String platform;

  @override
  Future<bool> available() async => await _channel.invokeMethod<bool>('available') ?? false;

  @override
  Future<PushRegistration> registration() async {
    final r = await _channel.invokeMapMethod<String, String>('registration');
    if (r == null) throw const PushError('这个版本没有配系统推送');
    return PushRegistration(token: r['token']!, p256dh: r['p256dh']!, auth: r['auth']!);
  }

  @override
  Future<void> reset() => _channel.invokeMethod<void>('reset');

  @override
  Future<String?> takeTapped() => _channel.invokeMethod<String>('takeTapped');

  @override
  Stream<String> get taps => _taps.stream;

  @override
  Stream<void> get tokenChanges => _tokens.stream;
}

/// 这个平台、这个构建能用的推送通道；不能用时为 null。
PushChannel? platformPushChannel() {
  if (kIsWeb || pushRelay.isEmpty) return null;
  if (Platform.isAndroid) return NativePushChannel('fcm');
  if (Platform.isIOS) return NativePushChannel('apns');
  return null;
}

class PushError implements Exception {
  const PushError(this.message);
  final String message;

  @override
  String toString() => message;
}

/// 向中转登记，换一个订阅地址。
typedef RelayRegister = Future<String> Function(Uri relay, String platform, String token);

/// `POST <中转>/register`。回来的地址要在同一个中转上，不是的话不用。
Future<String> registerWithRelay(Uri relay, String platform, String token) async {
  if (relay.scheme != 'https') throw const PushError('推送中转要用 https');
  final client = HttpClient()..connectionTimeout = const Duration(seconds: 10);
  try {
    final req = await client.postUrl(relay.resolve('/register'));
    req.headers.contentType = ContentType.json;
    req.write(jsonEncode({'platform': platform, 'token': token}));
    final resp = await req.close().timeout(const Duration(seconds: 15));
    final body = await resp.transform(utf8.decoder).join();
    if (resp.statusCode != 200) throw PushError('推送中转拒绝登记（${resp.statusCode}）');
    final endpoint = (jsonDecode(body) as Map<String, Object?>)['endpoint'];
    if (endpoint is! String || !sameRelay(relay, endpoint)) throw const PushError('推送中转回的地址不对');
    return endpoint;
  } on PushError {
    rethrow;
  } catch (e) {
    throw PushError('连不上推送中转：$e');
  } finally {
    client.close(force: true);
  }
}

/// 订阅地址是不是这个中转上的。
bool sameRelay(Uri relay, String endpoint) {
  final e = Uri.tryParse(endpoint);
  return e != null && e.scheme == relay.scheme && e.host == relay.host && e.port == relay.port;
}

enum LockPushState {
  /// 这个平台或构建没有锁屏推送。
  unavailable,

  /// 人在“我”页关掉了，或者系统没给通知权限。
  off,

  /// 正在登记。
  registering,

  /// 登记好了。
  on,

  /// 登记失败，原因在 [LockPush.error]。
  failed,
}

/// 登记、换令牌、关掉。AppState 在连上主机、拿到通知权限、令牌变了时调 [sync]。
class LockPush extends ChangeNotifier {
  LockPush({required this.channel, required this.relay, required this.store, RelayRegister? register})
    : _register = register ?? registerWithRelay;

  final PushChannel channel;
  final Uri relay;

  /// 记着向中转登记过的令牌和订阅地址，和开关。
  final SecretStore store;
  final RelayRegister _register;

  LockPushState state = LockPushState.off;
  String? error;
  bool? _available;
  Future<void>? _running;

  static const _enabledKey = 'push.enabled';
  static String _recordKey(String host) => 'push.v1.$host';

  void _set(LockPushState s, [String? e]) {
    state = s;
    error = e;
    notifyListeners();
  }

  /// 人有没有在“我”页关掉。默认开着。
  Future<bool> enabled() async => await store.read(_enabledKey) != 'false';

  /// 按现在的情况登记一遍：同一个令牌和地址再告诉主机一次（主机的数据库可能换过），令牌变了就换新地址。
  /// 同一时间只跑一个。
  Future<void> sync(IpClient client, String host, {required bool permitted}) {
    final running = _running;
    if (running != null) return running;
    final f = _sync(client, host, permitted).whenComplete(() => _running = null);
    _running = f;
    return f;
  }

  Future<void> _sync(IpClient client, String host, bool permitted) async {
    try {
      _available ??= await channel.available();
    } catch (_) {
      _available = false;
    }
    if (_available != true) return _set(LockPushState.unavailable);
    if (!permitted || !await enabled()) return _set(LockPushState.off);
    _set(LockPushState.registering);
    try {
      final reg = await channel.registration();
      final old = await _read(host);
      var endpoint = old?.token == reg.token ? old?.endpoint : null;
      endpoint ??= await _register(relay, channel.platform, reg.token);
      await client.subscribePush(endpoint, reg.p256dh, reg.auth);
      // 换了地址：主机上旧的那个删掉，不然一台设备最多 5 个订阅很快就满了。
      if (old != null && old.endpoint != endpoint) {
        await client.unsubscribePush(old.endpoint).catchError((Object _) {});
      }
      await _write(host, _Record(reg.token, endpoint));
      _set(LockPushState.on);
    } catch (e) {
      _set(LockPushState.failed, '$e');
    }
  }

  /// “我”页的开关。
  Future<void> setEnabled(bool on, IpClient? client, String host) async {
    await store.write(_enabledKey, on ? 'true' : 'false');
    if (on) {
      if (client != null) await sync(client, host, permitted: true);
      return;
    }
    await forget(client, host);
    _set(LockPushState.off);
  }

  /// 关掉或者断开主机：告诉主机删掉订阅，删掉密钥和推送令牌。主机连不上也照样删本机的。
  Future<void> forget(IpClient? client, String host) async {
    final old = await _read(host);
    if (old != null && client != null) {
      await client.unsubscribePush(old.endpoint).catchError((Object _) {});
    }
    await store.delete(_recordKey(host));
    if (_available == true) await channel.reset().catchError((Object _) {});
  }

  Future<_Record?> _read(String host) async {
    final raw = await store.read(_recordKey(host));
    if (raw == null) return null;
    try {
      final j = jsonDecode(raw) as Map<String, Object?>;
      return _Record(j['token']! as String, j['endpoint']! as String);
    } catch (_) {
      return null;
    }
  }

  Future<void> _write(String host, _Record r) =>
      store.write(_recordKey(host), jsonEncode({'token': r.token, 'endpoint': r.endpoint}));
}

class _Record {
  const _Record(this.token, this.endpoint);
  final String token;
  final String endpoint;
}
