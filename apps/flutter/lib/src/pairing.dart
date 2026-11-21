// 用 6 位配对码换这台设备专属的令牌：`POST /pair`（m2-spec D-30）。

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'host.dart';
import 'protocol/models.dart';

class PairingError implements Exception {
  const PairingError(this.message);
  final String message;

  @override
  String toString() => message;
}

/// 换令牌。原生 App 发的请求不带 `Origin`，主机按非浏览器客户端处理。
Future<Claimed> claimPairing(HostAddress host, String code, {HttpClient? http}) async {
  final client = http ?? HttpClient();
  client.connectionTimeout = const Duration(seconds: 15);
  try {
    final req = await client.postUrl(host.pairUri);
    req.headers.contentType = ContentType.json;
    req.write(jsonEncode({'code': code}));
    final res = await req.close().timeout(const Duration(seconds: 30));
    final text = await utf8.decodeStream(res).timeout(const Duration(seconds: 30));
    if (res.statusCode == 200) {
      try {
        return Claimed.fromJson(jsonDecode(text) as Json);
      } catch (_) {
        throw const PairingError('主机的回复看不懂：这个地址可能不是 intelligent-position 的主机');
      }
    }
    throw PairingError(switch (res.statusCode) {
      400 => '配对码不对、过期了或者已经用过。在主机上重新生成一个再试',
      429 => '猜错的次数太多，待用的配对码都作废了。在主机上重新生成一个再试',
      403 => '主机拒绝了配对：${text.trim()}',
      404 => '这个地址上没有配对接口：确认地址是 intelligent-position 的主机，并且主机版本支持设备配对',
      _ => '配对失败（HTTP ${res.statusCode}）${_errorText(text)}',
    });
  } on PairingError {
    rethrow;
  } on HandshakeException {
    throw const PairingError('HTTPS 证书校验没通过。App 不会跳过证书校验，请检查主机的证书');
  } on TimeoutException {
    throw const PairingError('连主机超时了，检查地址和网络');
  } on SocketException catch (e) {
    throw PairingError('连不上主机：${e.osError?.message ?? e.message}');
  } on HttpException catch (e) {
    throw PairingError('连不上主机：${e.message}');
  } finally {
    if (http == null) client.close(force: true);
  }
}

String _errorText(String body) {
  try {
    final error = (jsonDecode(body) as Json)['error'];
    if (error is String && error.isNotEmpty) return '：$error';
  } catch (_) {}
  return '';
}
