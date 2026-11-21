import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/host.dart';

void main() {
  group('HostAddress', () {
    test('只留下协议、主机和端口', () {
      expect(HostAddress.parse('https://ip.example.com/').text, 'https://ip.example.com');
      expect(HostAddress.parse(' https://IP.example.com ').text, 'https://ip.example.com');
      expect(HostAddress.parse('https://ip.example.com:443').text, 'https://ip.example.com');
      expect(HostAddress.parse('http://ip.example.com:80').text, 'http://ip.example.com');
      expect(HostAddress.parse('https://ip.example.com:8443').text, 'https://ip.example.com:8443');
      expect(HostAddress.parse('http://[::1]:8787').text, 'http://[::1]:8787');
    });

    test('没写协议时按 https 算', () {
      expect(HostAddress.parse('ip.example.com').text, 'https://ip.example.com');
    });

    test('带路径、查询、片段、用户名或别的协议的不收', () {
      for (final bad in [
        '',
        'https://ip.example.com/console',
        'https://ip.example.com/?token=x',
        'https://ip.example.com/#/chat',
        'https://user:pw@ip.example.com',
        'ftp://ip.example.com',
        'wss://ip.example.com',
        'https://',
      ]) {
        expect(() => HostAddress.parse(bad), throwsFormatException, reason: bad);
      }
    });

    test('明文连局域网地址要人确认，本机和 tailnet 不用', () {
      expect(HostAddress.parse('https://ip.example.com').needsTrust, isFalse);
      expect(HostAddress.parse('http://localhost:8787').needsTrust, isFalse);
      expect(HostAddress.parse('http://127.0.0.1:8787').needsTrust, isFalse);
      expect(HostAddress.parse('http://[::1]:8787').needsTrust, isFalse);
      expect(HostAddress.parse('http://ip-host.tail1234.ts.net').needsTrust, isFalse);
      expect(HostAddress.parse('http://192.168.1.20:8787').needsTrust, isTrue);
      expect(HostAddress.parse('http://10.0.2.2:8787').needsTrust, isTrue);
      expect(HostAddress.parse('http://ip.example.com').needsTrust, isTrue);
      // 以 ts.net 结尾但不是它的子域名的，不算。
      expect(HostAddress.parse('http://evilts.net').needsTrust, isTrue);
    });

    test('ws 和配对的地址', () {
      final h = HostAddress.parse('https://ip.example.com:8443');
      expect(h.wsUri.toString(), 'wss://ip.example.com:8443/ws');
      expect(h.pairUri.toString(), 'https://ip.example.com:8443/pair');
      expect(HostAddress.parse('http://localhost:8787').wsUri.toString(), 'ws://localhost:8787/ws');
    });
  });

  group('PairingLink', () {
    test('从控制台的配对链接取出地址和配对码', () {
      final link = PairingLink.tryParse('https://ip.example.com/#/pair/123456')!;
      expect(link.host.text, 'https://ip.example.com');
      expect(link.code, '123456');
      expect(PairingLink.tryParse(' http://localhost:8787#/pair/000001 ')!.code, '000001');
    });

    test('不是配对链接的返回 null', () {
      expect(PairingLink.tryParse('https://ip.example.com'), isNull);
      expect(PairingLink.tryParse('https://ip.example.com/#/pair/12345'), isNull);
      expect(PairingLink.tryParse('https://ip.example.com/#/pair/1234567'), isNull);
      expect(PairingLink.tryParse('https://ip.example.com/x/#/pair/123456'), isNull);
    });
  });

  test('配对码允许带空格', () {
    expect(normalizePairingCode('123 456'), '123456');
    expect(normalizePairingCode('12345'), isNull);
    expect(normalizePairingCode('12345a'), isNull);
  });
}
