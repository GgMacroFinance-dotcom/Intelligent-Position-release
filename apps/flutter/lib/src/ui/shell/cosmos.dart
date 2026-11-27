// 宽窗口的背景层（设计重构，2026-11-25 维护者定）：文艺复兴的人仰望的天空，明亮、温暖、有秩序。
// 全部用程序画（shaders/sky.frag），不用任何图片：天空铺满窗口（不要灰泥和金边），云是字符画，星是金点。
// 没有轨道、黄道带、拉丁名；太阳以后是单独一层、能单独选中。
// 这里只做几件事：开机时画一张小字表，按窗口大小算天空椭圆的位置，定时刷新。
//
// 省电（维护者定：低耗电优先）：
//   - 不跟着屏幕刷新：用定时器，窗口在前台时每秒 10 帧，窗口没焦点时每秒 4 帧（画面动得很慢，看不出差别；
//     2026-11-25 从 20 帧降到 10 帧试）；
//   - 窗口藏起来、最小化、这一页看不见（TickerMode 关了）、系统设了“减少动态效果”时停住，只留最后一帧；
//   - 每帧 CPU 只传十几个数，所有像素在 GPU 上算；
//   - 云的厚薄每秒只算 5 次（cloud.frag 画进一张小图，一格字一个像素），主着色器每帧只查表，
//     不再每格算 5 次噪声（云飘得很慢，看不出是一跳一跳的）；
//   - 2026-11-26 实测优化：宽窗口里天空和天体共用同一个时钟（[clock]），同一下刷新，
//     不再两个定时器错开多刷一倍的帧（实测后台从约 7 帧降回约 3 帧，GPU 降约一半）。
// 着色器读不出来（比如测试环境）时退回一张渐变，不报错。

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

/// 颜料：壁画的天空和灰泥（和 sky.frag 里的一致）。
abstract final class Pigment {
  static const ultra = Color(0xFF2E4A9E);
  static const azure = Color(0xFF6B94CC);
  static const azureLight = Color(0xFFB3C9E0);

  /// 窗口的底色：着色器还没读出来时先显示它。
  static const ground = azure;
}

class CosmosBackdrop extends StatefulWidget {
  const CosmosBackdrop({super.key, this.clock});

  /// 可选的共享时钟（宽窗口传天体用的那个）：传了就不自己开定时器，跟天体同一下刷新。
  /// 这一层被 TickerMode 关掉（面板、图鉴打开）时不跟着时钟走，停在最后一帧。
  final ValueListenable<double>? clock;

  /// 测试里关掉：不开定时器，也不读着色器。
  static bool animate = true;

  @override
  State<CosmosBackdrop> createState() => _CosmosBackdropState();
}

/// 字表（顺序和 sky.frag、body.frag 里的下标一致）：0 到 9 由疏到密，10 到 13 是线，14 到 17 是星，
/// 18 到 27 是行星环“插缝层”的另一套由疏到密（横笔画），28 到 37 是行星球面插缝层的竖笔画由疏到密（和主层交叉成排线）。
/// 由疏到密的都是居中的字形（不用 . _ 这种贴着上下边的），一行行排起来不会成横纹。
const _glyphs = [
  '·', '∙', ':', '•', '+', '*', 'o', '%', '#', '@', '|', '~', '/', r'\', '·', '•', '✦', '∙', //
  '·', '-', '~', '+', '=', 'x', '≈', '*', '&', '#', //
  '·', ':', '¦', 'ı', 'i', '|', '!', 'l', '‡', '‖',
];
const _atlasCols = 8, _atlasRows = 5;

/// 一格字的大小（逻辑像素）：4 像素的 Menlo。天体（celestial_body.dart）也用这个大小，和背景对得上。
const _fine = 4.0;
const _cell = Size(_fine * 0.6016, _fine * 1.257);
const glyphCell = _cell;

/// 白字透明底的小字表，背景和各个天体共用一张（只画一次）。
ui.Image? _sharedAtlas;
ui.Image glyphAtlas() => _sharedAtlas ??= _buildAtlas();

/// 天空明暗和云的布局按这个椭圆排（铺满窗口，不裁切）：半轴是窗口宽高的一半。
const _domeX = 0.5, _domeY = 0.5;

class _CosmosBackdropState extends State<CosmosBackdrop> with WidgetsBindingObserver {
  /// 不动的时候停在这一刻。
  static const _still = 40.0;

  final _time = ValueNotifier<double>(_still);
  final _clock = Stopwatch();
  double _base = _still;
  Timer? _timer;
  int _fps = 0;
  AppLifecycleState _life = AppLifecycleState.resumed;
  bool _loading = false;

  ui.FragmentShader? _shader;
  ui.FragmentShader? _cloudShader;
  ui.Image? _atlas;
  final _clouds = _CloudMap();

  ValueListenable<double>? _ext;
  bool _extActive = false;

  void _attach() {
    final c = widget.clock;
    if (identical(c, _ext)) return;
    _ext?.removeListener(_onExt);
    _ext = c;
    c?.addListener(_onExt);
  }

  void _onExt() {
    final c = _ext;
    if (_extActive && c != null) _time.value = c.value;
  }

  @override
  void didUpdateWidget(CosmosBackdrop old) {
    super.didUpdateWidget(old);
    if (!identical(old.clock, widget.clock)) {
      _attach();
      _fps = -1;
      _schedule();
    }
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _life = WidgetsBinding.instance.lifecycleState ?? AppLifecycleState.resumed;
  }

  Future<void> _load() async {
    try {
      final program = await ui.FragmentProgram.fromAsset('shaders/sky.frag');
      final cloud = await ui.FragmentProgram.fromAsset('shaders/cloud.frag');
      final atlas = glyphAtlas();
      if (!mounted) return;
      setState(() {
        _shader = program.fragmentShader();
        _cloudShader = cloud.fragmentShader();
        _atlas = atlas;
      });
    } catch (e) {
      debugPrint('天穹的着色器读不出来，用渐变代替：$e');
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _life = state;
    _schedule();
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (CosmosBackdrop.animate && !_loading) {
      _loading = true;
      _load();
    }
    _schedule();
  }

  /// 按现在的情况定刷新的快慢：前台 10 帧，没焦点 4 帧，看不见时停。
  void _schedule() {
    final visible = TickerMode.valuesOf(context).enabled;
    final still = !CosmosBackdrop.animate || MediaQuery.disableAnimationsOf(context);
    _attach();
    if (widget.clock != null) {
      // 跟共享时钟走：自己不开定时器。
      _timer?.cancel();
      _timer = null;
      _fps = 0;
      _extActive = visible && !still;
      if (_extActive) _onExt();
      return;
    }
    final fps = still || !visible
        ? 0
        : switch (_life) {
            AppLifecycleState.resumed => 10,
            AppLifecycleState.inactive => 4,
            _ => 0,
          };
    if (fps == _fps) return;
    _fps = fps;
    _timer?.cancel();
    _timer = null;
    if (fps == 0) {
      _base = _time.value;
      _clock
        ..stop()
        ..reset();
      return;
    }
    _clock.start();
    _timer = Timer.periodic(Duration(microseconds: 1000000 ~/ fps), (_) {
      _time.value = _base + _clock.elapsedMicroseconds / 1e6;
    });
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _timer?.cancel();
    _ext?.removeListener(_onExt);
    _time.dispose();
    _shader?.dispose();
    _cloudShader?.dispose();
    // 字表是共用的，不在这里释放。
    _clouds.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return RepaintBoundary(
      child: CustomPaint(
        painter: _SkyPainter(_time, _shader, _cloudShader, _atlas, _clouds, MediaQuery.devicePixelRatioOf(context)),
        size: Size.infinite,
      ),
    );
  }
}

/// 云厚的小图：一格字一个像素（R 云厚，G 天空颜料的深浅，B 星成簇的地方）。
/// 窗口大小变了、或者离上次算过了 [interval] 秒，就重新算一次。
/// 白字透明底的小字表：一格一个字，画得比实际大，缩小后边缘干净。
ui.Image _buildAtlas() {
  const size = 32.0;
  const cw = 22.0, ch = 40.0;
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  for (final (i, g) in _glyphs.indexed) {
    final tp = TextPainter(
      text: TextSpan(
        text: g,
        style: const TextStyle(
          fontFamily: 'Menlo',
          fontFamilyFallback: ['SF Mono', 'DejaVu Sans Mono', 'Noto Sans Mono', 'monospace'],
          fontSize: size,
          height: 1,
          color: Colors.white,
        ),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    final ox = (i % _atlasCols) * cw, oy = (i ~/ _atlasCols) * ch;
    tp.paint(canvas, Offset(ox + (cw - tp.width) / 2, oy + (ch - tp.height) / 2));
    tp.dispose();
  }
  final picture = recorder.endRecording();
  final image = picture.toImageSync((cw * _atlasCols).toInt(), (ch * _atlasRows).toInt());
  picture.dispose();
  return image;
}

class _CloudMap {
  static const interval = 0.2;

  ui.Image? image;
  Size _size = Size.zero;
  double _at = double.negativeInfinity;
  int cols = 0, rows = 0;

  /// 需要时用云的着色器重新画一张，返回现在的图。
  ui.Image refresh(ui.FragmentShader shader, Size size, double t, double cx, double cy, double ax, double by) {
    final img = image;
    if (img != null && size == _size && (t - _at).abs() < interval) return img;
    cols = (size.width / _cell.width).ceil() + 1;
    rows = (size.height / _cell.height).ceil() + 1;
    var i = 0;
    for (final v in [cols, rows, _cell.width, _cell.height, t, cx, cy, ax, by]) {
      shader.setFloat(i++, v.toDouble());
    }
    final recorder = ui.PictureRecorder();
    Canvas(recorder).drawRect(Rect.fromLTWH(0, 0, cols.toDouble(), rows.toDouble()), Paint()..shader = shader);
    final picture = recorder.endRecording();
    final next = picture.toImageSync(cols, rows);
    picture.dispose();
    img?.dispose();
    image = next;
    _size = size;
    _at = t;
    return next;
  }

  void dispose() {
    image?.dispose();
    image = null;
  }
}

class _SkyPainter extends CustomPainter {
  _SkyPainter(this.time, this.shader, this.cloudShader, this.atlas, this.clouds, this.dpr) : super(repaint: time);

  final ValueNotifier<double> time;
  final ui.FragmentShader? shader;
  final ui.FragmentShader? cloudShader;
  final ui.Image? atlas;
  final _CloudMap clouds;
  final double dpr;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.isEmpty) return;
    final rect = Offset.zero & size;
    final s = shader, cs = cloudShader, a = atlas;
    if (s == null || cs == null || a == null) {
      canvas.drawRect(
        rect,
        Paint()
          ..shader = ui.Gradient.linear(
            rect.topCenter,
            rect.bottomCenter,
            const [Pigment.ultra, Pigment.azure, Pigment.azureLight],
            const [0, 0.5, 1],
          ),
      );
      return;
    }
    final w = size.width, h = size.height;
    final ax = w * _domeX, by = h * _domeY;
    final cx = w / 2, cy = h * 0.48;
    final t = time.value;
    final map = clouds.refresh(cs, size, t, cx, cy, ax, by);
    var i = 0;
    for (final v in [w, h, t, dpr, _cell.width, _cell.height, cx, cy, ax, by, clouds.cols, clouds.rows]) {
      s.setFloat(i++, v.toDouble());
    }
    s.setImageSampler(0, a, filterQuality: FilterQuality.medium);
    s.setImageSampler(1, map, filterQuality: FilterQuality.none);
    canvas.drawRect(rect, Paint()..shader = s);
  }

  @override
  bool shouldRepaint(_SkyPainter old) =>
      old.shader != shader ||
      old.cloudShader != cloudShader ||
      old.atlas != atlas ||
      old.dpr != dpr ||
      old.time != time;
}
