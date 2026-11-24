// 宽窗口的背景：用字符画出来的文艺复兴天穹（设计重构，2026-11-24 维护者定：A 的颜色 + B 的结构）。
//   颜色照斯克罗维尼礼拜堂（Giotto）的天顶：青金石群青的底，金箔的星，边上一圈熟褐的暗角。
//   结构照文艺复兴的天球图（哥白尼《天体运行论》里的同心圆）：中心是太阳，外面六层行星天球，
//   每层顶上写着拉丁名，最外一圈是黄道带（I 到 XII，每 30 度一格、每 5 度一个刻度）。
// 将来这就是星图：太阳的位置留给幕僚长，天球留给员工。
// 会流动的有四样：天球上一串串金色的光点沿着轨道走（里圈快、外圈慢，相邻两圈方向相反）；
//   底下一层淡蓝的字符像以太一样绕着太阳慢慢旋转、变形；恒星明暗闪烁；太阳的光芒慢慢转、长短起伏。
//   黄道带十几分钟转一圈。系统设了“减少动态效果”时停在一帧。
// 画法参考了 ascii-engine 的思路（亮度选字、线条顺着方向用 - | / \、标点沿着河道流动），代码是自己写的。
// 一帧里所有字符用一次 drawRawAtlas 画：先把用到的字画成一张白色的字表，每个格子按颜色染色。

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

/// 颜料。
abstract final class Pigment {
  /// 青金石群青：天穹的底，中间亮、四周深。
  static const lapisDeep = Color(0xFF0A1133);
  static const lapis = Color(0xFF16205A);
  static const lapisLight = Color(0xFF243378);

  /// 流动的以太：比底色亮一点的群青。
  static const aether = Color(0xFF6A80D6);

  /// 金箔。
  static const gold = Color(0xFFC9A24A);
  static const goldBright = Color(0xFFF3D98E);

  /// 朱砂：太阳的外缘（以后用在“要你处理”）。
  static const cinnabar = Color(0xFFB5482F);

  /// 熟褐：四周的暗角，像老壁画的边。
  static const umber = Color(0xFF2E1E10);

  /// 羊皮纸白：天球上的拉丁名。
  static const parchment = Color(0xFFEFE6D2);
}

class CosmosBackdrop extends StatefulWidget {
  const CosmosBackdrop({super.key});

  /// 测试里关掉动画：一直有新帧时 pumpAndSettle 等不完。
  static bool animate = true;

  @override
  State<CosmosBackdrop> createState() => _CosmosBackdropState();
}

class _CosmosBackdropState extends State<CosmosBackdrop> with SingleTickerProviderStateMixin {
  /// 不动的时候停在这一刻：各层都已经摆开了。
  static const _still = 40.0;

  late final Ticker _ticker = createTicker(_tick);
  final _time = ValueNotifier<double>(_still);
  double _base = _still;
  Duration _last = Duration.zero;
  _GlyphSheet? _sheet;

  void _tick(Duration elapsed) {
    // 流动很慢，每秒 30 帧就够，省电。
    if (elapsed - _last < const Duration(milliseconds: 33)) return;
    _last = elapsed;
    _time.value = _base + elapsed.inMicroseconds / 1e6;
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    final still = !CosmosBackdrop.animate || MediaQuery.disableAnimationsOf(context);
    if (still && _ticker.isActive) _ticker.stop();
    if (!still && !_ticker.isActive) {
      _base = _time.value;
      _last = Duration.zero;
      _ticker.start();
    }
  }

  @override
  void dispose() {
    _ticker.dispose();
    _time.dispose();
    _sheet?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final dpr = MediaQuery.devicePixelRatioOf(context);
    if (_sheet?.dpr != dpr) {
      _sheet?.dispose();
      _sheet = _GlyphSheet(dpr);
    }
    return RepaintBoundary(
      child: CustomPaint(painter: _CosmosPainter(_time, _sheet!), size: Size.infinite),
    );
  }
}

class _CosmosPainter extends CustomPainter {
  _CosmosPainter(this.time, this.sheet) : super(repaint: time);

  final ValueListenable<double> time;
  final _GlyphSheet sheet;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.isEmpty) return;
    final rect = Offset.zero & size;
    // 1. 群青的底：中间亮一点，往外变深。
    canvas.drawRect(
      rect,
      Paint()
        ..shader = ui.Gradient.radial(
          rect.center,
          size.longestSide * 0.7,
          const [Pigment.lapisLight, Pigment.lapis, Pigment.lapisDeep],
          const [0, 0.5, 1],
        ),
    );
    // 2. 字符。
    final n = sheet.frame(size, time.value);
    if (n > 0) {
      canvas.drawRawAtlas(
        sheet.image,
        Float32List.sublistView(sheet.transforms, 0, n * 4),
        Float32List.sublistView(sheet.rects, 0, n * 4),
        Int32List.sublistView(sheet.colors, 0, n),
        BlendMode.modulate,
        null,
        Paint()..filterQuality = FilterQuality.low,
      );
    }
    // 3. 熟褐的暗角。
    canvas.drawRect(
      rect,
      Paint()
        ..shader = ui.Gradient.radial(
          rect.center,
          size.longestSide * 0.72,
          [Pigment.umber.withValues(alpha: 0), Pigment.umber.withValues(alpha: 0.35), Pigment.umber.withValues(alpha: 0.8)],
          const [0.5, 0.8, 1],
        ),
    );
  }

  @override
  bool shouldRepaint(_CosmosPainter old) => old.sheet != sheet || old.time != time;
}

// ---------- 小工具 ----------

double _smooth(double e0, double e1, double x) {
  final t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
  return t * t * (3 - 2 * t);
}

/// 整数格点上的伪随机数，0 到 1。跨平台、跨启动都一样。
double _hash(int x, int y) {
  var h = (x * 374761393 + y * 668265263) & 0xFFFFFFFF;
  h = ((h ^ (h >> 13)) * 1274126177) & 0xFFFFFFFF;
  return ((h ^ (h >> 16)) & 0xFFFFFF) / 0xFFFFFF;
}

/// 平滑的值噪声，0 到 1。
double _noise(double x, double y) {
  final xi = x.floor(), yi = y.floor();
  final xf = x - xi, yf = y - yi;
  final u = xf * xf * (3 - 2 * xf), v = yf * yf * (3 - 2 * yf);
  final a = _hash(xi, yi), b = _hash(xi + 1, yi), c = _hash(xi, yi + 1), d = _hash(xi + 1, yi + 1);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
}

int _rgb(Color c) => c.toARGB32() & 0xFFFFFF;

/// 从 a 到 b 的 17 级颜色。
List<int> _ramp(Color a, Color b) => [for (var i = 0; i <= 16; i++) _rgb(Color.lerp(a, b, i / 16)!)];

int _argb(double alpha, int rgb) => ((alpha.clamp(0.0, 1.0) * 255).round() << 24) | rgb;

/// 一张白色的字表，加上每一帧的格子。
class _GlyphSheet {
  _GlyphSheet(this.dpr) {
    final m = TextPainter(text: const TextSpan(text: 'M', style: _style), textDirection: TextDirection.ltr)..layout();
    cw = m.width;
    ch = m.height * 1.08;
    m.dispose();
    for (final (i, g) in _chars.indexed) {
      _index[g] = i;
    }
    _sw = (cw * dpr).ceil();
    _sh = (ch * dpr).ceil();
    final rows = (_chars.length / _sheetCols).ceil();
    final recorder = ui.PictureRecorder();
    final canvas = Canvas(recorder);
    final big = _style.copyWith(fontSize: _fontSize * dpr);
    _glyphRects = Float32List(_chars.length * 4);
    for (final (i, g) in _chars.indexed) {
      final ox = (i % _sheetCols) * _sw.toDouble(), oy = (i ~/ _sheetCols) * _sh.toDouble();
      final tp = TextPainter(text: TextSpan(text: g, style: big), textDirection: TextDirection.ltr)..layout();
      tp.paint(canvas, Offset(ox + (_sw - tp.width) / 2, oy + (_sh - tp.height) / 2));
      tp.dispose();
      _glyphRects
        ..[i * 4] = ox
        ..[i * 4 + 1] = oy
        ..[i * 4 + 2] = ox + _sw
        ..[i * 4 + 3] = oy + _sh;
    }
    final picture = recorder.endRecording();
    image = picture.toImageSync(_sheetCols * _sw, rows * _sh);
    picture.dispose();

    _gDash = _g('-');
    _gPipe = _g('|');
    _gSlash = _g('/');
    _gBack = _g(r'\');
    _gDot = _g('·');
    _gBullet = _g('•');
    _gStar = _g('✦');
    _gPlus = _g('+');
    _gAst = _g('*');
    _sunRamp = [for (final c in '.:-=+*#%@'.split('')) _g(c)];
    _aetherRamp = [for (final c in ['.', '·', ':', '~']) _g(c)];
  }

  final double dpr;

  static const _fontSize = 12.0;
  static const _style = TextStyle(
    fontFamily: 'Menlo',
    fontFamilyFallback: ['SF Mono', 'DejaVu Sans Mono', 'Noto Sans Mono', 'Courier New', 'monospace'],
    fontSize: _fontSize,
    height: 1.0,
    color: Colors.white,
  );
  static const _sheetCols = 16;
  static final _chars = [for (var c = 33; c < 127; c++) String.fromCharCode(c), '·', '•', '∙', '✦', '°'];

  static const _spheres = ['MERCVRIVS', 'VENVS', 'TELLVS', 'MARS', 'IVPITER', 'SATVRNVS'];
  static const _roman = ['I', 'II', 'III', 'IV', 'V', 'VI', 'VII', 'VIII', 'IX', 'X', 'XI', 'XII'];

  static final _goldRgb = _rgb(Pigment.gold);
  static final _aetherRgb = _rgb(Pigment.aether);
  static final _goldToBright = _ramp(Pigment.gold, Pigment.goldBright);
  static final _cinnToGold = _ramp(Pigment.cinnabar, Pigment.gold);
  static final _labelColor = _argb(0.8, _rgb(Pigment.parchment));
  static final _numeralColor = _argb(0.92, _rgb(Pigment.goldBright));

  /// 一个字符格子的宽和高（逻辑像素）。
  late final double cw, ch;
  late final int _sw, _sh;
  late final ui.Image image;
  late final Float32List _glyphRects;
  final _index = <String, int>{};

  late final int _gDash, _gPipe, _gSlash, _gBack, _gDot, _gBullet, _gStar, _gPlus, _gAst;
  late final List<int> _sunRamp, _aetherRamp;

  Float32List transforms = Float32List(0);
  Float32List rects = Float32List(0);
  Int32List colors = Int32List(0);
  Int32List _overG = Int32List(0);
  Int32List _overC = Int32List(0);

  // 这一帧的布局（每帧开头算一次）。
  double _cx = 0, _cy = 0, _halfDiag = 1, _rSun = 1, _rRay = 1, _rZoIn = 1, _rZoOut = 1, _r0 = 1, _step = 1, _rot = 0;

  // _cell 算出的这一格：字和颜色；-1 是空着。
  int _cg = -1, _cc = 0;

  int _g(String c) => _index[c] ?? _index['?']!;

  void dispose() => image.dispose();

  void _ensure(int n) {
    if (colors.length >= n) return;
    transforms = Float32List(n * 4);
    rects = Float32List(n * 4);
    colors = Int32List(n);
    _overG = Int32List(n);
    _overC = Int32List(n);
  }

  /// 方向 (vx, vy) 上的一笔用哪个字（屏幕坐标，y 朝下）。
  int _dir(double vx, double vy) {
    final ax = vx.abs(), ay = vy.abs();
    if (ay < ax * 0.414) return _gDash;
    if (ax < ay * 0.414) return _gPipe;
    return vx * vy > 0 ? _gBack : _gSlash;
  }

  /// 算一帧：填好 transforms、rects、colors，返回字符个数。
  int frame(Size size, double t) {
    final cols = (size.width / cw).ceil(), rows = (size.height / ch).ceil();
    final n = cols * rows;
    _ensure(n);
    _overG.fillRange(0, n, -1);

    _cx = size.width / 2;
    _cy = size.height / 2;
    final minDim = math.min(size.width, size.height);
    _halfDiag = math.sqrt(_cx * _cx + _cy * _cy);
    _rZoOut = minDim * 0.47;
    _rZoIn = _rZoOut - ch * 2.7;
    _rSun = math.max(minDim * 0.05, ch * 1.8);
    _rRay = _rSun * 2.4;
    _r0 = _rRay + ch * 1.5;
    _step = (_rZoIn - ch * 1.8 - _r0) / (_spheres.length - 1);
    _rot = t * 0.006;

    // 盖在上面的字：天球的拉丁名、黄道带的罗马数字。空格表示这一格留空。
    void put(String text, double x, double y, int argb) {
      final row = (y / ch).floor();
      if (row < 0 || row >= rows) return;
      final start = (x / cw - text.length / 2).round();
      for (var i = 0; i < text.length; i++) {
        final col = start + i;
        if (col < 0 || col >= cols) continue;
        final idx = row * cols + col;
        final c = text[i];
        _overG[idx] = c == ' ' ? -2 : _g(c);
        _overC[idx] = argb;
      }
    }

    for (final (i, name) in _spheres.indexed) {
      put(' $name ', _cx, _cy - (_r0 + _step * i), _labelColor);
    }
    final rm = (_rZoIn + _rZoOut) / 2;
    for (final (k, numeral) in _roman.indexed) {
      final a = _rot + (k + 0.5) * math.pi / 6 - math.pi / 2;
      put(numeral, _cx + math.cos(a) * rm, _cy + math.sin(a) * rm, _numeralColor);
    }

    final scale = 1 / dpr;
    var k = 0;
    for (var row = 0; row < rows; row++) {
      final dy = (row + 0.5) * ch - _cy;
      for (var col = 0; col < cols; col++) {
        final idx = row * cols + col;
        final o = _overG[idx];
        if (o != -1) {
          if (o < 0) continue;
          _cg = o;
          _cc = _overC[idx];
        } else {
          final dx = (col + 0.5) * cw - _cx;
          _cell(col, row, dx, dy, math.sqrt(dx * dx + dy * dy), t);
          if (_cg < 0) continue;
        }
        final j = k * 4, gj = _cg * 4;
        transforms
          ..[j] = scale
          ..[j + 1] = 0
          ..[j + 2] = col * cw
          ..[j + 3] = row * ch;
        rects
          ..[j] = _glyphRects[gj]
          ..[j + 1] = _glyphRects[gj + 1]
          ..[j + 2] = _glyphRects[gj + 2]
          ..[j + 3] = _glyphRects[gj + 3];
        colors[k] = _cc;
        k++;
      }
    }
    return k;
  }

  /// 一个格子画什么。(dx, dy) 是格子中心离太阳的位移，r 是距离。结果放在 _cg、_cc。
  void _cell(int col, int row, double dx, double dy, double r, double t) {
    _cg = -1;
    // 角落里淡出，和熟褐的暗角接上。
    final fade = 1 - _smooth(0.82, 1.08, r / _halfDiag);
    if (fade <= 0.01) return;

    // 太阳：越往中心越密越亮，外缘带一点朱砂，火苗跟着噪声跳。
    if (r < _rSun) {
      final v = 1 - r / _rSun;
      final f = _noise(dx * 0.09 + t * 0.9, dy * 0.09 - t * 0.7) - 0.5;
      final q = (v * 0.9 + 0.1 + f * 0.45).clamp(0.0, 1.0);
      _cg = _sunRamp[(q * (_sunRamp.length - 1)).round()];
      _cc = _argb(1, q > 0.5 ? _goldToBright[((q - 0.5) * 32).round()] : _cinnToGold[(q * 32).round()]);
      return;
    }

    final th = math.atan2(dy, dx);
    // 一条圆线在这一格算不算“压在线上”：左右两侧（线是竖的）每行只取一格，上下两侧（线是横的）每列只取一格，
    // 线就只有一个字宽，斜的地方也不会叠成两三道。
    final thick = math.max(0.5 * cw * (dx / r).abs(), 0.5 * ch * (dy / r).abs());

    if (r < _rRay) {
      // 太阳的光芒：十二道长的，中间夹十二道短的点线；慢慢转，长短起伏。
      const seg = math.pi / 6;
      final base = t * 0.012;
      final reach = (r - _rSun) / (_rRay - _rSun);
      final m = (th - base) % seg;
      final len = 0.62 + 0.38 * math.sin(t * 0.6 + ((th - base) / seg).round() * 1.7);
      if (math.min(m, seg - m) * r < cw * 0.55 && reach < len) {
        final s = 1 - reach / len;
        _cg = _dir(dx, dy);
        _cc = _argb((0.35 + 0.65 * s) * fade, _goldToBright[(s * 16).round()]);
        return;
      }
      final m2 = (th - base + seg / 2) % seg;
      if (math.min(m2, seg - m2) * r < cw * 0.45 && reach < len * 0.55) {
        _cg = _gDot;
        _cc = _argb((1 - reach / (len * 0.55)) * 0.75 * fade, _goldRgb);
        return;
      }
    } else if (r > _rZoIn - ch && r < _rZoOut + ch) {
      // 黄道带：内外两圈，每 30 度一道界线，靠外一圈每 5 度一个刻度。
      if ((r - _rZoOut).abs() < thick || (r - _rZoIn).abs() < thick) {
        _cg = _dir(-dy, dx);
        _cc = _argb(0.85 * fade, _goldRgb);
        return;
      }
      if (r > _rZoIn && r < _rZoOut) {
        final a = th - _rot + math.pi / 2;
        const seg = math.pi / 6;
        final m = a % seg;
        if (math.min(m, seg - m) * r < cw * 0.55) {
          _cg = _dir(dx, dy);
          _cc = _argb(0.75 * fade, _goldRgb);
          return;
        }
        if (r > _rZoOut - ch * 1.1) {
          const s5 = math.pi / 36;
          final m5 = a % s5;
          if (math.min(m5, s5 - m5) * r < cw * 0.4) {
            _cg = _dir(dx, dy);
            _cc = _argb(0.5 * fade, _goldRgb);
            return;
          }
        }
        return;
      }
    } else if (r > _r0 - ch && r < _r0 + _step * (_spheres.length - 1) + ch) {
      // 行星天球：金线，一串串光点沿着轨道走（里圈快、外圈慢，相邻两圈方向相反）。
      final i = ((r - _r0) / _step).round().clamp(0, _spheres.length - 1);
      if ((r - (_r0 + _step * i)).abs() < thick) {
        final dir = i.isEven ? 1.0 : -1.0;
        final s = math.sin(th * (4 + i) - t * dir * 0.8 / (1 + i * 0.45));
        final bead = s > 0 ? math.pow(s, 14).toDouble() : 0.0;
        _cg = bead > 0.88 ? _gBullet : _dir(-dy, dx);
        _cc = _argb((0.32 + 0.68 * bead) * fade, _goldToBright[(bead * 16).round()]);
        return;
      }
    }

    // 恒星：金色的，明暗慢慢闪；大一点的是八角星。
    final h = _hash(col * 7 + 3, row * 13 + 1);
    if (h < 0.0065) {
      final h2 = _hash(col * 11 + 5, row * 3 + 9);
      final tw = 0.5 + 0.5 * math.sin(t * (0.6 + h2 * 0.9) + h * 900);
      final big = h2 < 0.3;
      _cg = big ? (tw > 0.55 ? _gStar : _gPlus) : (tw > 0.75 ? _gAst : _gDot);
      _cc = _argb((0.3 + 0.7 * tw * tw) * fade, big ? _goldToBright[(tw * 16).round()] : _goldRgb);
      return;
    }

    // 流动的以太：噪声场绕着太阳旋转，里圈转得快，被拉成一缕一缕的。
    final ph = th + t * 0.03 * (300 / (r + 300));
    final px = math.cos(ph) * r * 0.012, py = math.sin(ph) * r * 0.012;
    final v = _noise(px + 11.3, py - 4.7 + t * 0.015) * 0.65 + _noise(px * 2.1 - 3.1, py * 2.1 + 8.2 - t * 0.02) * 0.35;
    if (v > 0.57) {
      final q = ((v - 0.57) / 0.3).clamp(0.0, 1.0);
      _cg = _aetherRamp[(q * (_aetherRamp.length - 1)).round()];
      _cc = _argb((0.1 + 0.36 * q) * fade, _aetherRgb);
    }
  }
}
