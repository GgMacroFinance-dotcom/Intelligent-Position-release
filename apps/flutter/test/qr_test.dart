// 扫码配对（F-19）：解二维码的纯 Dart 部分。用 zxing2 自己的编码器画二维码，再按各平台相机的格式交给解码。

import 'dart:typed_data';

import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/qr/decode.dart';
import 'package:zxing2/qrcode.dart';

/// 把 `text` 画成灰度图：每格 `cell` 像素，四周留 4 格白边，再贴进一张更大的灰底图（模拟相机画面）。
({Uint8List luma, int width, int height}) render(String text, {int cell = 4, int pad = 40}) {
  final m = Encoder.encode(text, ErrorCorrectionLevel.m).matrix!;
  final size = (m.width + 8) * cell;
  final width = size + pad * 2, height = size + pad * 2;
  final luma = Uint8List(width * height)..fillRange(0, width * height, 128);
  for (var y = 0; y < size; y++) {
    for (var x = 0; x < size; x++) {
      final mx = x ~/ cell - 4, my = y ~/ cell - 4;
      final dark = mx >= 0 && my >= 0 && mx < m.width && my < m.height && m.get(mx, my) == 1;
      luma[(y + pad) * width + x + pad] = dark ? 10 : 245;
    }
  }
  return (luma: luma, width: width, height: height);
}

void main() {
  const link = 'https://ip.example.com/#/pair/123456';

  test('Y 平面（Android、iOS）：一行比宽度长也能解', () {
    final r = render(link);
    const stride = 16;
    final plane = Uint8List((r.width + stride) * r.height);
    for (var y = 0; y < r.height; y++) {
      plane.setRange(y * (r.width + stride), y * (r.width + stride) + r.width, r.luma, y * r.width);
    }
    expect(decodeQr(LumaFrame.yPlane(plane, r.width, r.height, r.width + stride)), link);
  });

  test('每像素 4 字节（macOS）：BGRA 和 ARGB 都能解，宽图隔点取', () {
    for (final alphaFirst in [false, true]) {
      for (final cell in [4, 30]) {
        final r = render(link, cell: cell);
        final bytes = Uint8List(r.width * r.height * 4);
        for (var i = 0; i < r.width * r.height; i++) {
          final v = r.luma[i];
          bytes.setAll(i * 4, alphaFirst ? [255, v, v, v] : [v, v, v, 255]);
        }
        expect(r.width > 1000, cell == 30);
        expect(decodeQr(LumaFrame.from32bpp(bytes, r.width, r.height, r.width * 4)), link, reason: '$alphaFirst $cell');
      }
    }
  });

  test('没有二维码、或者数据不够长时返回 null', () {
    expect(decodeQr(LumaFrame(Uint8List(200 * 200)..fillRange(0, 40000, 200), 200, 200)), isNull);
    expect(decodeQr(LumaFrame(Uint8List(10), 200, 200)), isNull);
  });

  test('在另一个 isolate 里解', () async {
    final r = render(link);
    expect(await decodeQrInBackground(LumaFrame(r.luma, r.width, r.height)), link);
  });
}
