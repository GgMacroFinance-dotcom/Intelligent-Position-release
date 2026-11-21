import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/host.dart';
import 'package:ip_app/src/pairing.dart';
import 'package:ip_app/src/storage.dart';

/// 本机起一个假的 `/pair`：按配对码回不同的状态。
Future<(HttpServer, List<Map<String, Object?>>)> fakePairServer() async {
  final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
  final seen = <Map<String, Object?>>[];
  server.listen((req) async {
    final body = jsonDecode(await utf8.decodeStream(req)) as Map<String, dynamic>;
    seen.add({'path': req.uri.path, 'origin': req.headers.value('origin'), 'body': body});
    final res = req.response..headers.contentType = ContentType.json;
    switch (body['code']) {
      case '123456':
        res.write(
          jsonEncode({
            'token': 'ipd_abc',
            'device': {
              'id': 'd1',
              'name': '张三的手机',
              'scope': 'daily',
              'paired_at_ms': 1,
              'last_seen_ms': 1,
              'online': false,
            },
          }),
        );
      case '999999':
        res.statusCode = 429;
        res.write(jsonEncode({'error': '猜错太多次'}));
      default:
        res.statusCode = 400;
        res.write(jsonEncode({'error': '配对码不对或已过期'}));
    }
    await res.close();
  });
  return (server, seen);
}

void main() {
  test('用配对码换到设备令牌，请求不带 Origin', () async {
    final (server, seen) = await fakePairServer();
    final host = HostAddress.parse('http://127.0.0.1:${server.port}');
    final claimed = await claimPairing(host, '123456');
    expect(claimed.token, 'ipd_abc');
    expect(claimed.device.name, '张三的手机');
    expect(seen.single, {
      'path': '/pair',
      'origin': null,
      'body': {'code': '123456'},
    });
    await server.close();
  });

  test('配对码不对、猜错太多次时给人能看懂的话', () async {
    final (server, _) = await fakePairServer();
    final host = HostAddress.parse('http://127.0.0.1:${server.port}');
    await expectLater(
      claimPairing(host, '000000'),
      throwsA(isA<PairingError>().having((e) => e.message, 'message', contains('过期'))),
    );
    await expectLater(
      claimPairing(host, '999999'),
      throwsA(isA<PairingError>().having((e) => e.message, 'message', contains('太多'))),
    );
    await server.close();
  });

  test('连不上时报连不上', () async {
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    final port = server.port;
    await server.close();
    await expectLater(
      claimPairing(HostAddress.parse('http://127.0.0.1:$port'), '123456'),
      throwsA(isA<PairingError>().having((e) => e.message, 'message', contains('连不上'))),
    );
  });

  test('主机列表和令牌分开存，忘掉时一起删', () async {
    final secrets = MemorySecretStore();
    final store = HostStore(secrets);
    await store.probe();
    final a = HostAddress.parse('https://a.example.com');
    final b = HostAddress.parse('https://b.example.com');
    await store.save(HostEntry(address: a, deviceName: '手机'), 'ipd_a');
    await store.save(HostEntry(address: b, deviceName: '电脑', trusted: true), 'ipd_b');
    expect((await store.hosts()).map((h) => h.address), [b, a]);
    expect((await store.hosts()).first.trusted, isTrue);
    expect(await store.token(a), 'ipd_a');
    // 令牌不写进主机列表。
    expect(secrets.values['hosts'], isNot(contains('ipd_')));
    await store.forget(b);
    expect((await store.hosts()).map((h) => h.address), [a]);
    expect(await store.token(b), isNull);
    expect(secrets.values.containsKey('probe'), isFalse);
  });
}
