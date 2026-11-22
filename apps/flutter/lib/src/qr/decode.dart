// 从相机画面里解二维码（F-19）。纯 Dart：用 zxing2（ZXing 的 Dart 移植），不用谷歌的 ML Kit 之类闭源组件。
// 各平台的相机给的格式不一样，先统一成一张灰度图（`LumaFrame`），再交给 `decodeQr`。

import 'dart:isolate';
import 'dart:typed_data';

import 'package:zxing2/qrcode.dart';

/// 一张灰度图：每个像素一个字节，一行 `stride` 个字节（可以比 `width` 长）。
class LumaFrame {
  const LumaFrame(this.bytes, this.width, this.height, {int? stride}) : stride = stride ?? width;

  /// Android、iOS 的 YUV 画面直接取亮度那一面（Y 平面），不用转换。
  factory LumaFrame.yPlane(Uint8List plane, int width, int height, int bytesPerRow) =>
      LumaFrame(plane, width, height, stride: bytesPerRow);

  /// 每像素 4 字节的画面（macOS 相机给的位图）。透明通道不是第一个就是最后一个，中间两个一定是颜色，
  /// 所以取第 2、3 个字节的平均当亮度，不管是 BGRA、RGBA 还是 ARGB 都对。黑白二维码用这个够了。
  /// 宽过 1000 时隔一个取一个，解得快些。
  factory LumaFrame.from32bpp(Uint8List bytes, int width, int height, int bytesPerRow) {
    final step = width > 1000 ? 2 : 1;
    final w = width ~/ step, h = height ~/ step;
    final out = Uint8List(w * h);
    for (var y = 0; y < h; y++) {
      final row = y * step * bytesPerRow;
      for (var x = 0; x < w; x++) {
        final i = row + x * step * 4;
        if (i + 2 >= bytes.length) break;
        out[y * w + x] = (bytes[i + 1] + bytes[i + 2]) >> 1;
      }
    }
    return LumaFrame(out, w, h);
  }

  final Uint8List bytes;
  final int width;
  final int height;
  final int stride;
}

/// 解出二维码里的文字；画面里没有、或者没认出来时返回 null。
String? decodeQr(LumaFrame f) {
  if (f.width <= 0 || f.height <= 0 || f.bytes.length < f.stride * (f.height - 1) + f.width) return null;
  final source = _LumaSource(f);
  final hints = DecodeHints()..put(DecodeHintType.tryHarder);
  try {
    return QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source)), hints: hints).text;
  } on ReaderException {
    return null;
  } on ArgumentError {
    return null;
  }
}

/// 把 `LumaFrame` 交给 zxing2。
class _LumaSource extends LuminanceSource {
  _LumaSource(this.f) : super(f.width, f.height);

  final LumaFrame f;

  @override
  Int8List getRow(int y, Int8List? row) {
    final out = row != null && row.length >= width ? row : Int8List(width);
    final start = y * f.stride;
    for (var x = 0; x < width; x++) {
      out[x] = f.bytes[start + x];
    }
    return out;
  }

  @override
  Int8List getMatrix() {
    final out = Int8List(width * height);
    for (var y = 0; y < height; y++) {
      final start = y * f.stride;
      for (var x = 0; x < width; x++) {
        out[y * width + x] = f.bytes[start + x];
      }
    }
    return out;
  }
}

/// 在另一个 isolate 里解，不卡界面。
Future<String?> decodeQrInBackground(LumaFrame f) => Isolate.run(() => decodeQr(f));
