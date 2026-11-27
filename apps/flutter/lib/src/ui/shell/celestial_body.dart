// 多形态文艺复兴字符画天体（太阳、8 大行星形态、恒星），设计重构 2026-11-26。
// 所有天体共用同一对着色器（shaders/body.frag + shaders/body_surface.frag）与 cosmos.dart 字表；
// 极致轻量化与省电设计：
//   - 支持外部注入单一共享时钟 [CelestialClock]（所有星球共用 1 个 20fps/3fps 定时器，打开窗口或待办时彻底停表 0fps）；
//   - 按天体形态紧贴裁剪方框大小（球形行星不再浪费太阳光芒的透明像素区域）；
//   - 表面纹理是极小的离屏图，低频错峰烘焙（前台太阳 0.50s / 行星 1.50s / 后台加倍，且严格限制单帧全局最多烘焙 1 颗），body.frag 逐像素零噪声；
//   - 八大行星统一走细腻版（更小的字、`1 2 1` 插缝叠印、表面小图加密+平滑+两张之间插值）；太阳与恒星走旧版（恒星球边做平滑圆形切割）；
//   - 选中时整体自然变亮（240ms 平滑过渡，不加外框光环）；即使天体停转（frozen），选中过渡也照常走完。

import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';

import '../../star_map.dart' show Celestial, CelestialKind;
import 'cosmos.dart' show CosmosBackdrop, glyphAtlas, glyphCell;
import 'tokens.dart';

/// 10 种文艺复兴字符画天体形态（顺序与 shaders/body.frag、body_surface.frag 的 uStyle 严格一一对应）。
enum CelestialStyle {
  sun('太阳', Color(0xFFE6AD48)),
  ringed('土星', Color(0xFFD9B56F)),
  banded('木星', Color(0xFFD0834A)),
  oceanic('地球', Color(0xFF3E84D6)),
  terracotta('火星', Color(0xFFD6633E)),
  lunar('水星', Color(0xFFB8C4D8)),
  ice('天王星', Color(0xFF63D0DE)),
  deepSea('海王星', Color(0xFF3A72D8)),
  gilded('金星', Color(0xFFE3B18A)),
  star('恒星', Color(0xFFD8E6FF));

  const CelestialStyle(this.bodyName, this.fallbackColor);

  /// 天体本名（不附加修饰定语）。
  final String bodyName;
  final Color fallbackColor;

  /// 着色器里对应的形态编号。
  double get shaderId => index.toDouble();

  /// 八大行星统一走细腻版（更小的字 + 插缝叠印 + 平滑表面）；太阳与恒星保持旧版字符画风（恒星球边做平滑圆形切割）。
  bool get fine => this != CelestialStyle.sun && this != CelestialStyle.star;

  /// 从星图里的 [Celestial] 映射到对应着色器形态。
  static CelestialStyle fromCelestial(Celestial c) => switch (c.id) {
    'sun' => CelestialStyle.sun,
    'saturn' => CelestialStyle.ringed,
    'jupiter' => CelestialStyle.banded,
    'earth' => CelestialStyle.oceanic,
    'mars' => CelestialStyle.terracotta,
    'mercury' => CelestialStyle.lunar,
    'uranus' => CelestialStyle.ice,
    'neptune' => CelestialStyle.deepSea,
    'venus' => CelestialStyle.gilded,
    _ => c.kind == CelestialKind.star ? CelestialStyle.star : CelestialStyle.ringed,
  };
}

/// 全部天体与轨道共用的单一轻量时钟：避免 N 颗星球各自开定时器造成帧率叠加与 CPU 抖动。
/// 前台 20fps（2026-11-26 为公转顺滑从 10 提到 20），窗口失焦 3fps，冻结（面板/待办打开）/隐藏/减少动效时 0fps 完全零开销。
class CelestialClock extends ValueNotifier<double> with WidgetsBindingObserver {
  CelestialClock() : super(_initialTime) {
    WidgetsBinding.instance.addObserver(this);
    _life = WidgetsBinding.instance.lifecycleState ?? AppLifecycleState.resumed;
  }

  static const _initialTime = 40.0;

  /// 前台帧率：2026-11-26 为了公转更顺，从 10 提到 20（天空跟着同一个时钟走）。
  static const foregroundFps = 20;

  final _watch = Stopwatch();
  double _base = _initialTime;
  Timer? _timer;
  int _fps = 0;
  bool _frozen = false;
  // 先当作“没开”：第一次 sync(enabled: true) 一定会真正启动定时器（否则刚开 App 时天体和天空一直不动，要切一次窗口才开始动）。
  bool _enabled = false;
  AppLifecycleState _life = AppLifecycleState.resumed;

  /// 当前是否处于窗口失焦的低帧后台态（用于自动拉长表面纹理烘焙周期，进一步省电）。
  bool get isBackground => _fps > 0 && _fps <= 3;

  void sync({required bool frozen, required bool enabled}) {
    if (_frozen == frozen && _enabled == enabled) return;
    _frozen = frozen;
    _enabled = enabled;
    _schedule();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _life = state;
    _schedule();
  }

  void _schedule() {
    final fps = (_frozen || !_enabled || !CosmosBackdrop.animate)
        ? 0
        : switch (_life) {
            AppLifecycleState.resumed => foregroundFps,
            AppLifecycleState.inactive => 3,
            _ => 0,
          };
    if (fps == _fps) return;
    _fps = fps;
    _timer?.cancel();
    _timer = null;
    if (fps == 0) {
      _base = value;
      _watch
        ..stop()
        ..reset();
      return;
    }
    _watch.start();
    _timer = Timer.periodic(Duration(microseconds: 1000000 ~/ fps), (_) {
      value = _base + _watch.elapsedMicroseconds / 1e6;
    });
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _timer?.cancel();
    super.dispose();
  }
}

/// 所有天体共用的着色器程序（全局只加载一次）。
Future<(ui.FragmentProgram, ui.FragmentProgram)>? _programs;
Future<(ui.FragmentProgram, ui.FragmentProgram)> _loadPrograms() => _programs ??= () async {
  final body = await ui.FragmentProgram.fromAsset('shaders/body.frag');
  final surface = await ui.FragmentProgram.fromAsset('shaders/body_surface.frag');
  return (body, surface);
}();

class CelestialBody extends StatefulWidget {
  const CelestialBody({
    super.key,
    required this.radius,
    required this.selected,
    required this.onTap,
    this.style = CelestialStyle.sun,
    this.seed = 0.0,
    this.frozen = false,
    this.clock,
    this.onHoverChanged,
    this.onDragStart,
    this.onDragUpdate,
    this.onDragEnd,
    this.onDragCancel,
    this.onSecondaryTap,
    this.onScrollZoom,
    this.dragging = false,
    this.haze,
    this.repaint,
    this.semanticsLabel = '太阳',
  });

  /// 球的半径（逻辑像素）。光芒/行星环与光晕都在外面，整个控件是 [extent] 见方。
  final double radius;
  final bool selected;
  final VoidCallback onTap;

  /// 天体形态（太阳、土星环、木星红斑、地球、火星、月相水星、天王冰环、海王深蓝、金星厚云、恒星）。
  final CelestialStyle style;

  /// 0..1 的相位种子：让不同星球错峰烘焙表面小图、呈现不同自转初相。
  final double seed;

  /// 停住表面翻涌和光芒（面板开着时）。选中的过渡不受影响。
  final bool frozen;

  /// 可选共享时钟；不传时控件自己管定时器（单测或独立使用时方便）。
  final ValueListenable<double>? clock;

  /// 悬停状态回调（用于天穹上同步亮起星球标签与状态副标题）。
  final ValueChanged<bool>? onHoverChanged;

  /// 可选的拖拽换轨手势回调（行星在轨道上时传入，窗顶停泊与太阳不传）。
  final GestureDragStartCallback? onDragStart;
  final GestureDragUpdateCallback? onDragUpdate;
  final GestureDragEndCallback? onDragEnd;
  final VoidCallback? onDragCancel;

  /// 可选右键回调（演示模式下用于在实机上一键触发「任务完成 · 待审批」模拟效果）。
  final VoidCallback? onSecondaryTap;

  /// 可选滚轮缩放回调（方案 A：悬停在轨道星球上滚动滚轮/双指上下滑时传出纵向 `scrollDelta.dy`；太阳与泊位星球不传）。
  final ValueChanged<double>? onScrollZoom;

  /// 当前是否正被用户拖拽（用于切换抓手光标并保持高亮）。
  final bool dragging;

  /// 纵深轨道上的远近（0 近 .. 1 最远）：远处隔着一层空气，偏天空蓝、略淡。不传就是 0。
  /// 用回调而不是 ValueListenable：公转时跟随共享时钟在同一帧读取，避免在 build 里改 notifier 额外触发第二帧重绘。
  final ValueGetter<double>? haze;

  /// 除时钟/选中/悬停外，额外触发天体重绘的动画（如窗顶停泊或换轨补间，在停表时也能顺滑过渡大气透视）。
  final Listenable? repaint;
  final String semanticsLabel;

  /// 光芒从球边往外伸多长（仅太阳与恒星使用）。
  double get rayLength => rayLengthFor(radius, style);

  /// 整个控件的边长：按形态紧贴球体/光芒/行星环裁剪，避免绘制多余透明像素。
  double get extent => extentFor(radius, style);

  static double rayLengthFor(double radius, [CelestialStyle style = CelestialStyle.sun]) => switch (style) {
    CelestialStyle.sun => radius * 0.62,
    CelestialStyle.star => radius * 0.72,
    _ => 0.0,
  };

  /// 半径是 [radius]、形态是 [style] 的天体占多大的方框（排版时直接算，不用先建控件）。
  static double extentFor(double radius, [CelestialStyle style = CelestialStyle.sun]) => switch (style) {
    CelestialStyle.sun => radius * 1.62 * 2 + 24,
    CelestialStyle.ringed => radius * 2.12 * 2 + 16,
    CelestialStyle.ice => radius * 1.80 * 2 + 16,
    CelestialStyle.star => radius * 1.72 * 2 + 20,
    _ => radius * 1.32 * 2 + 16,
  };

  @override
  State<CelestialBody> createState() => _CelestialBodyState();
}

class _CelestialBodyState extends State<CelestialBody> with WidgetsBindingObserver, SingleTickerProviderStateMixin {
  static const _still = 40.0;

  final _ownTime = ValueNotifier<double>(_still);
  final _hoverLift = ValueNotifier<double>(0.0);
  final _clock = Stopwatch();
  double _base = _still;
  Timer? _timer;
  int _fps = 0;
  AppLifecycleState _life = AppLifecycleState.resumed;
  bool _loading = false;

  ui.FragmentShader? _shader;
  ui.FragmentShader? _surfaceShader;
  ui.Image? _atlas;
  final _surface = _SurfaceMap();

  /// 选中的过渡：0 没选中，1 选中。
  late final AnimationController _sel = AnimationController(
    vsync: this,
    duration: Motion.dialog,
    value: widget.selected ? 1 : 0,
  );

  ValueListenable<double> get _activeTime => widget.clock ?? _ownTime;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _life = WidgetsBinding.instance.lifecycleState ?? AppLifecycleState.resumed;
  }

  @override
  void didUpdateWidget(CelestialBody old) {
    super.didUpdateWidget(old);
    if (old.selected != widget.selected) {
      widget.selected ? _sel.animateTo(1, curve: Motion.curve) : _sel.animateTo(0, curve: Motion.curve);
    }
    if (old.frozen != widget.frozen || old.clock != widget.clock) _schedule();
  }

  /// 选中过渡现在走到哪了（0 没选中，1 选中），测试用。
  @visibleForTesting
  double get selection => _sel.value;

  Future<void> _load() async {
    try {
      final (body, surface) = await _loadPrograms();
      final atlas = glyphAtlas();
      if (!mounted) return;
      setState(() {
        _shader = body.fragmentShader();
        _surfaceShader = surface.fragmentShader();
        _atlas = atlas;
      });
    } catch (e) {
      debugPrint('天体的着色器读不出来，画一个简单的圆：$e');
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

  /// 没传共享时钟时自己调度：前台 10 帧，没焦点 3 帧，看不见或停住时 0 帧。
  void _schedule() {
    if (widget.clock != null) {
      _timer?.cancel();
      _timer = null;
      _fps = 0;
      return;
    }
    final visible = TickerMode.valuesOf(context).enabled;
    final still = widget.frozen || !CosmosBackdrop.animate || MediaQuery.disableAnimationsOf(context);
    final fps = still || !visible
        ? 0
        : switch (_life) {
            AppLifecycleState.resumed => 10,
            AppLifecycleState.inactive => 3,
            _ => 0,
          };
    if (fps == _fps) return;
    _fps = fps;
    _timer?.cancel();
    _timer = null;
    if (fps == 0) {
      _base = _ownTime.value;
      _clock
        ..stop()
        ..reset();
      return;
    }
    _clock.start();
    _timer = Timer.periodic(Duration(microseconds: 1000000 ~/ fps), (_) {
      _ownTime.value = _base + _clock.elapsedMicroseconds / 1e6;
    });
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _timer?.cancel();
    _ownTime.dispose();
    _hoverLift.dispose();
    _sel.dispose();
    _shader?.dispose();
    _surfaceShader?.dispose();
    _surface.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final w = widget;
    final extent = w.extent;
    // 最小可点直径 48 逻辑像素，小球也能轻松点中；方框四个角依然不挡点击。
    final hitDiameter = (w.radius * 2 + 16).clamp(48.0, extent);
    return SizedBox.square(
      dimension: extent,
      child: Semantics(
        button: true,
        selected: w.selected,
        label: w.semanticsLabel,
        child: Stack(
          children: [
            Positioned.fill(
              child: IgnorePointer(
                child: RepaintBoundary(
                  child: CustomPaint(
                    painter: _BodyPainter(
                      time: _activeTime,
                      selection: _sel,
                      hoverLift: _hoverLift,
                      haze: w.haze,
                      repaint: w.repaint,
                      shader: _shader,
                      surfaceShader: _surfaceShader,
                      atlas: _atlas,
                      surface: _surface,
                      radius: w.radius,
                      rayLength: w.rayLength,
                      style: w.style,
                      seed: w.seed,
                      dpr: MediaQuery.devicePixelRatioOf(context),
                    ),
                  ),
                ),
              ),
            ),
            // 只有球本身（再大一圈，好点）能点/能拖，光芒、环外和方框角落不挡点击。
            Center(
              child: MouseRegion(
                cursor: w.dragging
                    ? SystemMouseCursors.grabbing
                    : (w.onDragStart != null ? SystemMouseCursors.grab : SystemMouseCursors.click),
                onEnter: (_) {
                  _hoverLift.value = 0.45;
                  w.onHoverChanged?.call(true);
                },
                onExit: (_) {
                  _hoverLift.value = 0.0;
                  w.onHoverChanged?.call(false);
                },
                child: Listener(
                  onPointerSignal: w.onScrollZoom == null
                      ? null
                      : (event) {
                          if (event is PointerScrollEvent && event.scrollDelta.dy != 0) {
                            GestureBinding.instance.pointerSignalResolver.register(event, (e) {
                              w.onScrollZoom?.call((e as PointerScrollEvent).scrollDelta.dy);
                            });
                          }
                        },
                  child: GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: w.onTap,
                    onSecondaryTap: w.onSecondaryTap,
                    onPanStart: w.onDragStart,
                    onPanUpdate: w.onDragUpdate,
                    onPanEnd: w.onDragEnd,
                    onPanCancel: w.onDragCancel,
                    child: SizedBox.square(dimension: hitDiameter, child: const _RoundHit()),
                  ),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 圆形的点击区域：方框四个角上点不中。
class _RoundHit extends SingleChildRenderObjectWidget {
  const _RoundHit();

  @override
  RenderObject createRenderObject(BuildContext context) => _RenderRoundHit();
}

class _RenderRoundHit extends RenderProxyBox {
  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) {
    final c = size.center(Offset.zero);
    if ((position - c).distance > size.shortestSide / 2) return false;
    result.add(BoxHitTestEntry(this, position));
    return true;
  }

  @override
  void performLayout() => size = constraints.biggest;
}

/// 行星细腻版的一格字：3.4 像素的 Menlo，行距压到 1.18（背景和太阳、恒星仍是 4 像素的 [glyphCell]）。
/// 行距压紧 + body.frag 里字形拉高，上下两行几乎挨上，行缝不再成横纹。
const fineGlyphCell = Size(3.4 * 0.6016, 3.4 * 1.18);

/// 表面肌理离屏小图。
/// 旧版（太阳、恒星）：一格字一像素、最近邻取样，和上一版一致。
/// 细腻版（八大行星）：方框直角网格，每 [fineStep] 逻辑像素一个像素，双线性取样；
/// 留着上一张，body.frag 在两张之间按时间插值，流动是连续的。低频错峰烘焙，且全局单帧最多烘焙 1 颗。
class _SurfaceMap {
  static const fineStep = 1.5;

  /// 全局单帧烘焙预算：记录上次执行同步纹理烘焙时的时钟值 `t`。
  /// 同一时钟帧内已有天体烘焙过时，其余到期的顺延一帧再烘，避免多星齐烘造成 GPU 帧时间毛刺。
  static double _lastBakeTick = double.negativeInfinity;

  ui.Image? image;
  ui.Image? prev;
  double _extent = 0;
  double _radius = 0;
  CelestialStyle? _style;
  double _seed = -1;
  double _at = double.negativeInfinity;
  double _nextDelay = 0.0;
  double _bakeInterval = 1.0;
  int cols = 0, rows = 0;

  /// 表面肌理刷新周期（秒）：太阳 0.50s，恒星 1.00s，带双帧插值的细腻八大行星 1.50s；失焦后台态整体再翻倍。
  static double _interval(CelestialStyle style, {required bool slow}) {
    final base = switch (style) {
      CelestialStyle.sun => 0.50,
      CelestialStyle.star => 1.00,
      _ => 1.50,
    };
    return slow ? base * 2.0 : base;
  }

  ui.Image refresh(
    ui.FragmentShader shader,
    double extent,
    double radius,
    double t,
    CelestialStyle style,
    double seed, {
    bool slow = false,
  }) {
    final interval = _interval(style, slow: slow);
    final img = image;
    final sameGeom = img != null && extent == _extent && radius == _radius && style == _style && seed == _seed;
    if (sameGeom) {
      final elapsed = (t - _at).abs();
      final wait = _nextDelay > 0 ? _nextDelay : interval;
      if (elapsed < wait) return img;
      // 已有缓存纹理时严守单帧 1 颗的烘焙预算，避免同一帧多次同步 GPU 纹理上传。
      if (_lastBakeTick == t) return img;
    }

    final fine = style.fine;
    if (fine) {
      cols = (extent / fineStep).ceil() + 1;
      rows = cols;
    } else {
      cols = (extent / glyphCell.width).ceil() + 1;
      rows = (extent / glyphCell.height).ceil() + 1;
    }
    var i = 0;
    for (final v in [
      cols, rows, glyphCell.width, glyphCell.height, extent, extent, t, radius, style.shaderId, seed, //
      fine ? fineStep : 0.0,
    ]) {
      shader.setFloat(i++, v.toDouble());
    }
    final recorder = ui.PictureRecorder();
    Canvas(recorder).drawRect(Rect.fromLTWH(0, 0, cols.toDouble(), rows.toDouble()), Paint()..shader = shader);
    final picture = recorder.endRecording();
    final next = picture.toImageSync(cols, rows);
    picture.dispose();
    prev?.dispose();
    if (fine && sameGeom) {
      // 刚才那张留作插值起点。
      prev = img;
    } else {
      prev = null;
      img?.dispose();
    }
    image = next;
    _extent = extent;
    _radius = radius;
    _style = style;
    _seed = seed;
    _at = t;
    _bakeInterval = interval;
    // 首次烘焙按 seed 错开下一次到期时间，之后按完整 interval 周期错峰更新。
    _nextDelay = sameGeom ? interval : (0.2 + 0.8 * seed) * interval;
    if (sameGeom) _lastBakeTick = t;
    return next;
  }

  /// 细腻版：从上一张到这一张走到哪了（0..1）。没有上一张时是 1。
  double blend(double t) {
    if (prev == null || _bakeInterval <= 0) return 1.0;
    return ((t - _at) / _bakeInterval).clamp(0.0, 1.0);
  }

  void dispose() {
    image?.dispose();
    image = null;
    prev?.dispose();
    prev = null;
  }
}

class _BodyPainter extends CustomPainter {
  _BodyPainter({
    required this.time,
    required this.selection,
    required this.hoverLift,
    required this.haze,
    required this.repaint,
    required this.shader,
    required this.surfaceShader,
    required this.atlas,
    required this.surface,
    required this.radius,
    required this.rayLength,
    required this.style,
    required this.seed,
    required this.dpr,
  }) : super(repaint: Listenable.merge([time, selection, hoverLift, ?repaint]));

  final ValueListenable<double> time;
  final Animation<double> selection;
  final ValueListenable<double> hoverLift;
  final ValueGetter<double>? haze;
  final Listenable? repaint;
  final ui.FragmentShader? shader;
  final ui.FragmentShader? surfaceShader;
  final ui.Image? atlas;
  final _SurfaceMap surface;
  final double radius;
  final double rayLength;
  final CelestialStyle style;
  final double seed;
  final double dpr;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.isEmpty) return;
    final s = shader, ss = surfaceShader, a = atlas;
    final sel = (selection.value + hoverLift.value * (1.0 - selection.value)).clamp(0.0, 1.0);
    if (s == null || ss == null || a == null) {
      final c = size.center(Offset.zero);
      final base = style.fallbackColor;
      final lifted = Color.lerp(base, Colors.white, sel * 0.18)!;
      canvas.drawCircle(c, radius, Paint()..color = lifted);
      return;
    }
    final t = time.value;
    final tClock = time;
    final slow = tClock is CelestialClock && tClock.isBackground;
    final map = surface.refresh(ss, size.width, radius, t, style, seed, slow: slow);
    final fine = style.fine;
    final cell = fine ? fineGlyphCell : glyphCell;
    var i = 0;
    for (final v in [
      size.width, size.height, t, dpr, cell.width, cell.height, //
      radius, rayLength, sel, style.shaderId, surface.cols, surface.rows,
      fine ? _SurfaceMap.fineStep : 0.0, surface.blend(t), haze?.call() ?? 0.0,
    ]) {
      s.setFloat(i++, v.toDouble());
    }
    final surfaceFilter = fine ? FilterQuality.low : FilterQuality.none;
    s.setImageSampler(0, a, filterQuality: FilterQuality.medium);
    s.setImageSampler(1, map, filterQuality: surfaceFilter);
    s.setImageSampler(2, surface.prev ?? map, filterQuality: surfaceFilter);
    canvas.drawRect(Offset.zero & size, Paint()..shader = s);
  }

  @override
  bool shouldRepaint(_BodyPainter old) =>
      old.shader != shader ||
      old.surfaceShader != surfaceShader ||
      old.atlas != atlas ||
      old.radius != radius ||
      old.rayLength != rayLength ||
      old.style != style ||
      old.seed != seed ||
      old.dpr != dpr ||
      old.time != time ||
      old.selection != selection ||
      old.hoverLift != hoverLift ||
      old.haze != haze ||
      old.repaint != repaint;
}
