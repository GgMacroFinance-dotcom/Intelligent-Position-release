// 主机列表和设备令牌放系统的安全存储（apps-plan 9.3）：iOS、macOS 钥匙串，Android Keystore 加密，
// Windows 凭据管理，Linux 的 Secret Service。主机列表一开始就按列表存（Q-9），界面先只用第一台。

import 'dart:convert';
import 'dart:io';

import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'host.dart';

/// 键值存储。测试里换成内存里的。
abstract class SecretStore {
  Future<String?> read(String key);
  Future<void> write(String key, String value);
  Future<void> delete(String key);
}

class SystemSecretStore implements SecretStore {
  SystemSecretStore()
    : _storage = FlutterSecureStorage(
        // macOS 用普通的登录钥匙串：数据保护钥匙串要“钥匙串共享”权限和描述文件，没签名的 App 用不了。
        mOptions: Platform.isMacOS
            ? const MacOsOptions(usesDataProtectionKeychain: false)
            : MacOsOptions.defaultOptions,
      );

  final FlutterSecureStorage _storage;

  @override
  Future<String?> read(String key) => _storage.read(key: key);

  @override
  Future<void> write(String key, String value) => _storage.write(key: key, value: value);

  @override
  Future<void> delete(String key) => _storage.delete(key: key);
}

class MemorySecretStore implements SecretStore {
  final values = <String, String>{};

  @override
  Future<String?> read(String key) async => values[key];

  @override
  Future<void> write(String key, String value) async => values[key] = value;

  @override
  Future<void> delete(String key) async => values.remove(key);
}

/// 配对过的一台主机。令牌另外存，不在这里。
class HostEntry {
  const HostEntry({required this.address, required this.deviceName, this.trusted = false});

  factory HostEntry.fromJson(Map<String, dynamic> j) => HostEntry(
    address: HostAddress.parse(j['address'] as String),
    deviceName: j['device_name'] as String? ?? '',
    trusted: j['trusted'] as bool? ?? false,
  );

  final HostAddress address;

  /// 主机给这台设备起的名字，消息和审批按它署名。
  final String deviceName;

  /// 人勾过“我信任这个网络”（明文连局域网地址时要）。
  final bool trusted;

  Map<String, dynamic> toJson() => {'address': address.text, 'device_name': deviceName, 'trusted': trusted};
}

class HostStore {
  HostStore(this._secrets);

  final SecretStore _secrets;

  static const _hostsKey = 'hosts';
  static String _tokenKey(HostAddress a) => 'token:${a.text}';

  Future<List<HostEntry>> hosts() async {
    final text = await _secrets.read(_hostsKey);
    if (text == null) return [];
    try {
      return [for (final e in jsonDecode(text) as List) HostEntry.fromJson(e as Map<String, dynamic>)];
    } catch (_) {
      return [];
    }
  }

  Future<String?> token(HostAddress address) => _secrets.read(_tokenKey(address));

  /// 先确认安全存储写得进去、读得回来，再去用一次性的配对码（D-34 的做法）。
  Future<void> probe() async {
    const key = 'probe';
    await _secrets.write(key, 'ok');
    final back = await _secrets.read(key);
    await _secrets.delete(key);
    if (back != 'ok') throw StateError('系统的安全存储写不进去');
  }

  /// 存下一台主机和它的令牌，放在列表最前面（当前用的那台）。
  Future<void> save(HostEntry entry, String token) async {
    await _secrets.write(_tokenKey(entry.address), token);
    final list = (await hosts()).where((h) => h.address != entry.address).toList()..insert(0, entry);
    await _secrets.write(_hostsKey, jsonEncode([for (final h in list) h.toJson()]));
  }

  /// 删掉一台主机和它的令牌。
  Future<void> forget(HostAddress address) async {
    await _secrets.delete(_tokenKey(address));
    final list = (await hosts()).where((h) => h.address != address).toList();
    await _secrets.write(_hostsKey, jsonEncode([for (final h in list) h.toJson()]));
  }
}
