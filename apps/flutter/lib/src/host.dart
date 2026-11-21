// 主机地址的规整和信任规则（docs/ip/apps-plan.md 4.1、9.3）。

/// 一台主机的地址：只留 `scheme://主机[:端口]`，主机名小写，默认端口省略。
class HostAddress {
  HostAddress._(this.origin);

  /// 规整后的地址，如 `https://ip.example.com`。
  final Uri origin;

  /// 解析人填的地址。没写协议时按 `https://` 算；只认 http、https，不能带路径、查询、片段或用户名。
  factory HostAddress.parse(String input) {
    var text = input.trim();
    if (text.isEmpty) throw const FormatException('请填主机地址，例如 https://ip.example.com');
    if (!text.contains('://')) text = 'https://$text';
    final Uri uri;
    try {
      uri = Uri.parse(text);
    } on FormatException {
      throw FormatException('$input 不是网址，例如 https://ip.example.com');
    }
    final scheme = uri.scheme.toLowerCase();
    if (scheme != 'http' && scheme != 'https') {
      throw const FormatException('主机地址要以 https:// 或 http:// 开头');
    }
    if (uri.host.isEmpty) throw FormatException('$input 缺少主机名');
    final extra =
        (uri.path.isNotEmpty && uri.path != '/') || uri.hasQuery || uri.hasFragment || uri.userInfo.isNotEmpty;
    if (extra) {
      throw const FormatException('主机地址只写到域名和端口，例如 https://ip.example.com');
    }
    final defaultPort = scheme == 'https' ? 443 : 80;
    final port = uri.hasPort && uri.port != defaultPort ? uri.port : null;
    return HostAddress._(Uri(scheme: scheme, host: uri.host.toLowerCase(), port: port));
  }

  /// 存储和显示用的文字。
  String get text => origin.toString();

  bool get isHttps => origin.scheme == 'https';

  /// 本机地址：`localhost`、`127.x.x.x`、`::1`。
  bool get isLoopback {
    final h = origin.host;
    return h == 'localhost' || h == '::1' || RegExp(r'^127\.\d{1,3}\.\d{1,3}\.\d{1,3}$').hasMatch(h);
  }

  /// Tailscale 的 MagicDNS 名字，流量在 WireGuard 里，`http://` 也是加密的。
  bool get isTailnet => origin.host.endsWith('.ts.net');

  /// 明文连非本机、非 tailnet 的地址：要人勾选“我信任这个网络”才连。
  bool get needsTrust => !isHttps && !isLoopback && !isTailnet;

  /// `/ws` 的地址。
  Uri get wsUri => origin.replace(scheme: isHttps ? 'wss' : 'ws', path: '/ws');

  /// `POST /pair` 的地址。
  Uri get pairUri => origin.replace(path: '/pair');

  @override
  bool operator ==(Object other) => other is HostAddress && other.text == text;

  @override
  int get hashCode => text.hashCode;

  @override
  String toString() => text;
}

/// 从控制台给的配对链接（二维码里就是它）取出主机地址和配对码。
class PairingLink {
  const PairingLink(this.host, this.code);

  final HostAddress host;
  final String code;

  /// 形如 `https://ip.example.com/#/pair/123456`；不是这种格式时返回 null。
  static PairingLink? tryParse(String input) {
    final m = RegExp(r'^\s*(\S+?)/?#/pair/(\d{6})\s*$').firstMatch(input);
    if (m == null) return null;
    try {
      return PairingLink(HostAddress.parse(m.group(1)!), m.group(2)!);
    } on FormatException {
      return null;
    }
  }
}

/// 配对码是 6 位数字；允许人填时带空格。
String? normalizePairingCode(String input) {
  final code = input.replaceAll(RegExp(r'\s'), '');
  return RegExp(r'^\d{6}$').hasMatch(code) ? code : null;
}
