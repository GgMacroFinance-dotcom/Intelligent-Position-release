// Bot 星图（backlog F-22，docs/ip/apps-plan.md 9.7）。照维护者给的 Obsidian 图谱风格：深色玻璃面板、
// 悬停高亮关联、点击弹出可拖动的半透明对话气泡。背景是星空；幕僚长是中心的太阳，其他员工是行星和恒星，
// 离太阳越近越紧急，图标越大越重要（算法在 star_map.dart，理由在气泡里照原样给人看）。

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';
import 'package:flutter/services.dart';

import '../app_state.dart';
import '../protocol/models.dart';
import '../star_map.dart';
import 'celestial_painter.dart';
import 'chat_page.dart' show ThreadView;
import 'common.dart';
import 'inbox_page.dart' show ApprovalCard;

// Obsidian 深色主题的几种颜色，和维护者给的样稿一致。
const _ink = Color(0xFF0B0D13);
const _panel = Color(0xE6131620);
const _purple = Color(0xFFA855F7);
const _sunColor = Color(0xFFFBBF24);
const _rolePalette = [
  Color(0xFF38BDF8),
  Color(0xFFF472B6),
  Color(0xFF34D399),
  Color(0xFFFB923C),
  Color(0xFF22D3EE),
  Color(0xFFA3E635),
  Color(0xFFFB7185),
  Color(0xFF818CF8),
];

/// 星图页自己的深色主题：气泡里复用的审批卡、对话也跟着变深。
final obsidianTheme = ThemeData(
  brightness: Brightness.dark,
  colorSchemeSeed: _purple,
  useMaterial3: true,
  scaffoldBackgroundColor: _ink,
);

class StarMapPage extends StatefulWidget {
  const StarMapPage({super.key, required this.onOpenChat});

  /// 气泡里点“在对话页打开”：去对话页，打开现在这个对话。
  final VoidCallback onOpenChat;

  @override
  State<StarMapPage> createState() => _StarMapPageState();
}

/// 一颗天体这一帧在屏幕上的位置。
class _Placed {
  _Placed(this.body, this.pos, this.radius, this.color);
  final BotBody? body;
  final Offset pos;
  final double radius;
  final Color color;
}

class _BgStar {
  _BgStar(this.x, this.y, this.r, this.alpha, this.color, this.speed, this.phase, this.depth);
  final double x, y, r, alpha, speed, phase, depth;
  final Color color;
}

class _StarMapPageState extends State<StarMapPage> with SingleTickerProviderStateMixin {
  late final Ticker _ticker;
  final _clock = ValueNotifier<double>(0);
  Duration _last = Duration.zero;

  /// 公转走过的时间（秒）：关掉公转、悬停、按着屏幕或打开气泡时停下。
  double _orbitTime = 0;

  /// 手指（或鼠标键）按在星图上。触摸没有悬停，按下时就停住，免得点的时候天体挪开了。
  bool _pressing = false;

  /// 松开后再等一会儿才接着转（时钟读数）。
  double _resumeAt = 0;

  // 视图：平移（屏幕像素）和缩放。
  Offset _pan = Offset.zero;
  double _scale = 1;
  double _gestureScale = 1;
  Offset _gestureFocal = Offset.zero;
  Offset _gesturePan = Offset.zero;

  /// 平滑飞到这个世界坐标（点 [[关联]] 跳转、居中时）。
  Offset? _flyTo;
  double? _flyScale;

  String? _hovered;
  String? _selected;
  String _query = '';
  String? _roleFilter;

  bool _orbiting = true;
  bool _showOrbits = true;
  bool _showPulses = true;
  bool _showLabels = true;
  bool _settingsOpen = false;
  double _spacing = 1;

  /// 气泡左上角；为空时居中。
  Offset? _bubblePos;
  Size _viewport = Size.zero;

  /// 显示用的轨道半径，朝目标慢慢滑过去：一位员工变急时看得到它往里移。
  final _shownRadius = <String, double>{};
  final _searchCtl = TextEditingController();

  /// 点了天体后拿到焦点，Esc 才能关气泡。
  final _focus = FocusNode(debugLabel: '星图');
  late final List<_BgStar> _bgStars;

  @override
  void initState() {
    super.initState();
    final rnd = math.Random(42);
    _bgStars = List.generate(460, (i) {
      final roll = rnd.nextDouble();
      final color = roll < 0.1
          ? const Color(0xFFBFD4FF)
          : roll < 0.18
          ? const Color(0xFFFFE0B5)
          : Colors.white;
      final big = rnd.nextDouble() < 0.06;
      return _BgStar(
        rnd.nextDouble(),
        rnd.nextDouble(),
        big ? 1.2 + rnd.nextDouble() * 0.9 : 0.35 + rnd.nextDouble() * 0.75,
        0.25 + rnd.nextDouble() * 0.7,
        color,
        0.4 + rnd.nextDouble() * 2.2,
        rnd.nextDouble() * math.pi * 2,
        [0.015, 0.04, 0.08][rnd.nextInt(3)],
      );
    });
    _ticker = createTicker(_onTick);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    // 系统设了“减少动态效果”时不动：星不闪、不公转，只画一帧。
    final still = MediaQuery.maybeDisableAnimationsOf(context) ?? false;
    if (still && _ticker.isActive) _ticker.stop();
    if (!still && !_ticker.isActive) _ticker.start();
  }

  @override
  void dispose() {
    _ticker.dispose();
    _clock.dispose();
    _searchCtl.dispose();
    _focus.dispose();
    super.dispose();
  }

  void _onTick(Duration elapsed) {
    final dt = ((elapsed - _last).inMicroseconds / 1e6).clamp(0.0, 0.1);
    _last = elapsed;
    final paused = _hovered != null || _selected != null || _pressing || _clock.value < _resumeAt;
    if (_orbiting && !paused) _orbitTime += dt;
    final to = _flyTo;
    if (to != null) {
      final s = _flyScale ?? _scale;
      final targetPan = _homeOffset() - to * s;
      _pan = Offset.lerp(_pan, targetPan, 0.12)!;
      _scale += (s - _scale) * 0.12;
      if ((_pan - targetPan).distance < 1 && (s - _scale).abs() < 0.005) {
        _flyTo = null;
        _flyScale = null;
      }
    }
    _clock.value += dt;
  }

  // ---------- 坐标 ----------

  Offset _toScreen(Offset world, Size size) => size.center(Offset.zero) + _pan + world * _scale;

  void _zoomAt(Offset focal, double factor, Size size) {
    final next = (_scale * factor).clamp(0.35, 3.0);
    final world = (focal - size.center(Offset.zero) - _pan) / _scale;
    setState(() {
      _userMoved = true;
      _flyTo = null;
      _scale = next;
      _pan = focal - size.center(Offset.zero) - world * next;
    });
  }

  void _center() => setState(() {
    _flyTo = Offset.zero;
    _flyScale = _homeScale();
  });

  /// 默认视图：宽屏上太阳往右挪一点，让开左上角的标题卡片；窄屏上往下挪，让开标题和筛选。
  Offset _homeOffset() => _viewport.width >= 768 ? const Offset(110, 0) : const Offset(0, 70);

  /// 默认缩放：最外圈轨道刚好放得下。
  double _homeScale() {
    final room = math.min(_viewport.width - (_viewport.width >= 768 ? 260 : 24), _viewport.height - 150);
    return (room / 2 / 430).clamp(0.4, 1.0);
  }

  /// 人拖过或缩放过视图：之后窗口变大变小都不再自动重新摆。
  bool _userMoved = false;
  Size _homedFor = Size.zero;

  // ---------- 角色和颜色 ----------

  Color _roleColor(StarMap map, List<Role> roles, String roleId) {
    if (map.chief?.agent.role == roleId) return _purple;
    final ids = roles.map((r) => r.id).where((id) => id != map.chief?.agent.role).toList()..sort();
    final i = ids.indexOf(roleId);
    return _rolePalette[(i < 0 ? roleId.hashCode : i) % _rolePalette.length];
  }

  String _roleName(List<Role> roles, String roleId) {
    for (final r in roles) {
      if (r.id == roleId) return r.displayName;
    }
    return roleId;
  }

  bool _matches(BotBody b, List<Role> roles) {
    if (_roleFilter != null && b.agent.role != _roleFilter) return false;
    final q = _query.trim().toLowerCase();
    if (q.isEmpty) return true;
    return '${b.agent.name} ${b.agent.id} ${_roleName(roles, b.agent.role)} ${b.celestial.name} ${b.celestial.english}'
        .toLowerCase()
        .contains(q);
  }

  // ---------- 打开气泡 ----------

  /// 点开一位员工：打开它最相关的对话（在问你的、受阻或在做的任务的执行对话、这次打开过的），
  /// 没有就开一个发给它的新对话。
  void _select(StarMap map, String id) {
    final state = AppScope.read(context);
    final b = map.byId(id);
    setState(() {
      _selected = id;
      _settingsOpen = false;
    });
    _focus.requestFocus();
    if (b == null) return;
    var thread = b.focusThreadId;
    if (thread == null) {
      for (final t in state.threads) {
        if (state.threadAgent[t.threadId] == id) {
          thread = t.threadId;
          break;
        }
      }
    }
    if (thread != null) {
      if (state.threadId != thread) state.openThread(thread);
    } else {
      state.startNewChat();
      state.chooseAgent(id);
    }
  }

  void _jumpTo(StarMap map, String id, Offset world) {
    _select(map, id);
    setState(() {
      _flyTo = world;
      _flyScale = math.max(_scale, 1.0);
    });
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final map = buildStarMap(
      agents: state.agents,
      roles: state.roles,
      tasks: state.tasks,
      approvals: state.approvals,
      working: state.agentsWorking,
      failures: state.agentFailures,
      now: DateTime.now().millisecondsSinceEpoch,
    );
    if (_selected != null && map.byId(_selected!) == null && _selected != _vacantSun) _selected = null;
    return Theme(
      data: obsidianTheme,
      child: CallbackShortcuts(
        bindings: {const SingleActivator(LogicalKeyboardKey.escape): () => setState(() => _selected = null)},
        child: Focus(
          focusNode: _focus,
          child: LayoutBuilder(
            builder: (context, box) {
              _viewport = box.biggest;
              if (!_userMoved && !_viewport.isEmpty && _viewport != _homedFor) {
                _homedFor = _viewport;
                _pan = _homeOffset();
                _scale = _homeScale();
              }
              final wide = box.maxWidth >= 768;
              return ColoredBox(
                color: _ink,
                child: Stack(
                  children: [
                    Positioned.fill(child: _sky(map, state)),
                    Positioned(top: 12, left: 12, right: wide ? null : 60, child: _titleCard(map, state, wide)),
                    Positioned(top: 12, right: 12, child: _settings()),
                    if (wide) Positioned(left: 12, bottom: 12, child: _stats(map, state)),
                    Positioned(right: 12, bottom: 12, child: _zoomBar(box.biggest)),
                    if (_selected != null) _bubble(map, state, box.biggest),
                  ],
                ),
              );
            },
          ),
        ),
      ),
    );
  }

  /// 太阳位置没有幕僚长时用的 id。
  static const _vacantSun = '\u0000sun';

  // ---------- 星空、轨道、连线和天体 ----------

  Widget _sky(StarMap map, AppState state) {
    return Listener(
      onPointerDown: (_) => _pressing = true,
      onPointerUp: (_) {
        _pressing = false;
        _resumeAt = _clock.value + 1.5;
      },
      onPointerCancel: (_) => _pressing = false,
      onPointerSignal: (e) {
        if (e is PointerScrollEvent) _zoomAt(e.localPosition, e.scrollDelta.dy > 0 ? 0.9 : 1.1, _viewport);
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => setState(() {
          _selected = null;
          _settingsOpen = false;
        }),
        onScaleStart: (d) {
          _userMoved = true;
          _flyTo = null;
          _gestureScale = _scale;
          _gestureFocal = d.localFocalPoint;
          _gesturePan = _pan;
        },
        onScaleUpdate: (d) => setState(() {
          final next = (_gestureScale * d.scale).clamp(0.35, 3.0);
          final world = (_gestureFocal - _viewport.center(Offset.zero) - _gesturePan) / _gestureScale;
          _scale = next;
          _pan = d.localFocalPoint - _viewport.center(Offset.zero) - world * next;
        }),
        child: ValueListenableBuilder<double>(
          valueListenable: _clock,
          builder: (context, t, _) {
            final size = _viewport;
            final placed = _place(map, state, size);
            final focus = _hovered ?? _selected;
            final neighbors = focus == null ? const <String>{} : {focus, ...map.neighbors(focus)};
            final filtering = _query.trim().isNotEmpty || _roleFilter != null;
            double alphaOf(String id, BotBody? b) {
              if (focus != null) return neighbors.contains(id) || (focus == _vacantSun) ? 1 : 0.16;
              if (filtering && b != null) return _matches(b, state.roles) ? 1 : 0.15;
              return 1;
            }

            return Stack(
              clipBehavior: Clip.none,
              children: [
                Positioned.fill(
                  child: CustomPaint(
                    painter: _SkyPainter(
                      t: t,
                      pan: _pan,
                      scale: _scale,
                      stars: _bgStars,
                      sunPos: _toScreen(Offset.zero, size),
                      placed: placed,
                      links: map.links,
                      showOrbits: _showOrbits,
                      showPulses: _showPulses,
                      spacing: _spacing,
                      focus: focus,
                      neighbors: neighbors,
                      alphaOf: alphaOf,
                      tether: _selected == null ? null : _tetherTarget(size),
                    ),
                  ),
                ),
                for (final p in placed)
                  _bodyWidget(map, state, p, t, alphaOf(p.body?.agent.id ?? _vacantSun, p.body), size),
              ],
            );
          },
        ),
      ),
    );
  }

  /// 每位员工这一帧的位置：按员工 id 排好均分角度，半径按紧急度，公转时越靠里转得越快。
  List<_Placed> _place(StarMap map, AppState state, Size size) {
    final out = <_Placed>[
      _Placed(map.chief, _toScreen(Offset.zero, size), 46 * _scale, map.chief == null ? _sunColor : _purple),
    ];
    final ids = map.bodies.map((b) => b.agent.id).toList()..sort();
    final n = ids.length;
    for (final b in map.bodies) {
      final target = orbitRadius(b.urgency, inner: 130 * _spacing, outer: 130 * _spacing + 260 * _spacing);
      final shown = _shownRadius[b.agent.id] ?? target;
      final r = shown + (target - shown) * 0.06;
      _shownRadius[b.agent.id] = r;
      final i = ids.indexOf(b.agent.id);
      final base = -math.pi / 2 + i * 2 * math.pi / math.max(1, n) + 0.35;
      final omega = 2 * math.pi / 120 * math.pow(130 * _spacing / r, 1.5);
      final a = base + _orbitTime * omega;
      final world = Offset(math.cos(a), math.sin(a)) * r;
      out.add(
        _Placed(
          b,
          _toScreen(world, size),
          bodyRadius(b.importance) * _scale,
          _roleColor(map, state.roles, b.agent.role),
        ),
      );
    }
    return out;
  }

  Offset _tetherTarget(Size size) {
    final s = _bubbleSize(size);
    final pos = _bubblePos ?? Offset((size.width - s.width) / 2, (size.height - s.height) / 2);
    return pos + Offset(s.width / 2, s.height / 2);
  }

  Widget _bodyWidget(StarMap map, AppState state, _Placed p, double t, double alpha, Size size) {
    final b = p.body;
    final id = b?.agent.id ?? _vacantSun;
    final celestial = b?.celestial ?? sun;
    final hovered = _hovered == id;
    final selected = _selected == id;
    final searchHit = b != null && _query.trim().isNotEmpty && _matches(b, state.roles);
    // 画布至少 44 像素见方：缩小看时天体只有几个像素，手指也要点得中。
    final box = math.max(p.radius * (celestial.kind == CelestialKind.planet ? 3.0 : 5.4), 44.0);
    final showLabel = _showLabels || hovered || selected || searchHit || b == null || b == map.chief;
    final name = b == null ? '幕僚长（空位）' : (b == map.chief ? '${b.agent.name} · 幕僚长' : b.agent.name);
    final tag = b == null ? '没配协调员岗位' : _roleName(state.roles, b.agent.role);
    final hot = b != null && b != map.chief && b.urgency >= urgencyBands.first.at;
    // 名字可能比天体宽：外框至少 200 宽，天体居中。
    final width = math.max(box, 200.0);
    return Positioned(
      left: p.pos.dx - width / 2,
      top: p.pos.dy - box / 2,
      width: width,
      child: Opacity(
        opacity: alpha,
        // 外框为了放名字比天体宽；只认画出来的天体和名字，不然旁边那颗星看不见的外框会挡住点击。
        child: MouseRegion(
          opaque: false,
          hitTestBehavior: HitTestBehavior.deferToChild,
          cursor: SystemMouseCursors.click,
          onEnter: (_) => setState(() => _hovered = id),
          onExit: (_) => setState(() => _hovered = _hovered == id ? null : _hovered),
          child: GestureDetector(
            onTap: () => b == null ? setState(() => _selected = _vacantSun) : _select(map, id),
            child: Semantics(
              button: true,
              label: '$name，$tag，${celestial.name}',
              child: SizedBox(
                width: width,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    CustomPaint(
                      key: ValueKey('star-$id'),
                      size: Size.square(box),
                      painter: _BodyPainter(
                        celestial: celestial,
                        radius: p.radius,
                        t: t,
                        ring: selected ? Colors.white : (hovered ? p.color : null),
                        searchHit: searchHit,
                        hot: hot,
                        color: p.color,
                        box: box,
                      ),
                    ),
                    if (showLabel)
                      Transform.translate(
                        offset: Offset(0, -box / 2 + p.radius * (celestial.kind == CelestialKind.planet ? 1.15 : 0.9)),
                        child: SizedBox(
                          width: width,
                          child: Column(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              Text(
                                name,
                                maxLines: 1,
                                overflow: TextOverflow.ellipsis,
                                style: TextStyle(
                                  color: hovered || selected ? Colors.white : const Color(0xFFE2E8F0),
                                  fontSize: 12,
                                  fontWeight: hovered || selected || b == map.chief ? FontWeight.w700 : FontWeight.w500,
                                  shadows: const [
                                    Shadow(color: _ink, blurRadius: 4),
                                    Shadow(color: _ink, blurRadius: 2),
                                  ],
                                ),
                              ),
                              if (_scale >= 0.78 || hovered || selected)
                                Text(
                                  hot ? '$tag · ${b.reasons.first.text.split('：').first}' : tag,
                                  maxLines: 1,
                                  overflow: TextOverflow.ellipsis,
                                  style: TextStyle(
                                    color: hot ? const Color(0xFFFACC15) : p.color,
                                    fontSize: 10,
                                    shadows: const [Shadow(color: _ink, blurRadius: 3)],
                                  ),
                                ),
                            ],
                          ),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  // ---------- 浮在上面的面板 ----------

  BoxDecoration _glass({double radius = 16, Color? border}) => BoxDecoration(
    color: _panel,
    borderRadius: BorderRadius.circular(radius),
    border: Border.all(color: border ?? Colors.white.withValues(alpha: 0.1)),
    boxShadow: const [BoxShadow(color: Color(0x99000000), blurRadius: 24, offset: Offset(0, 8))],
  );

  Widget _blurred({required Widget child, double radius = 16}) => ClipRRect(
    borderRadius: BorderRadius.circular(radius),
    child: BackdropFilter(filter: ui.ImageFilter.blur(sigmaX: 14, sigmaY: 14), child: child),
  );

  Widget _titleCard(StarMap map, AppState state, bool wide) {
    final roleIds = <String>{for (final a in state.agents) a.role}.toList()..sort();
    final countFor = {for (final r in roleIds) r: state.agents.where((a) => a.role == r).length};
    return ConstrainedBox(
      constraints: const BoxConstraints(maxWidth: 440),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        mainAxisSize: MainAxisSize.min,
        children: [
          _blurred(
            child: Container(
              padding: const EdgeInsets.all(12),
              decoration: _glass(),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      Container(
                        width: 28,
                        height: 28,
                        alignment: Alignment.center,
                        decoration: BoxDecoration(
                          borderRadius: BorderRadius.circular(8),
                          gradient: const LinearGradient(colors: [Color(0xFFA855F7), Color(0xFF4F46E5)]),
                          boxShadow: const [BoxShadow(color: Color(0x4DA855F7), blurRadius: 10)],
                        ),
                        child: const Text(
                          'I.P.',
                          style: TextStyle(color: Colors.white, fontSize: 10.5, fontWeight: FontWeight.w800),
                        ),
                      ),
                      const SizedBox(width: 10),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Row(
                              children: [
                                const Flexible(
                                  child: Text(
                                    '智能岗位 · Bot 星图',
                                    overflow: TextOverflow.ellipsis,
                                    style: TextStyle(color: Colors.white, fontSize: 13.5, fontWeight: FontWeight.w700),
                                  ),
                                ),
                                const SizedBox(width: 6),
                                _chip('Star Map', _purple),
                              ],
                            ),
                            const SizedBox(height: 2),
                            const Text(
                              '离太阳越近越紧急 · 图标越大越重要 · 点天体打开对话',
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(color: Color(0xFF94A3B8), fontSize: 10.5),
                            ),
                          ],
                        ),
                      ),
                    ],
                  ),
                  const SizedBox(height: 10),
                  SizedBox(
                    height: 34,
                    child: TextField(
                      controller: _searchCtl,
                      onChanged: (v) => setState(() => _query = v),
                      style: const TextStyle(fontSize: 12.5, color: Color(0xFFF1F5F9)),
                      decoration: InputDecoration(
                        isDense: true,
                        hintText: '搜索 Bot、岗位或天体（如：宏观、木星）',
                        hintStyle: const TextStyle(fontSize: 12, color: Color(0xFF64748B)),
                        prefixIcon: const Icon(Icons.search, size: 16, color: Color(0xFF94A3B8)),
                        prefixIconConstraints: const BoxConstraints(minWidth: 32),
                        suffixIcon: _query.isEmpty
                            ? null
                            : IconButton(
                                tooltip: '清除搜索',
                                iconSize: 14,
                                onPressed: () => setState(() {
                                  _searchCtl.clear();
                                  _query = '';
                                }),
                                icon: const Icon(Icons.close),
                              ),
                        filled: true,
                        fillColor: _ink.withValues(alpha: 0.9),
                        contentPadding: const EdgeInsets.symmetric(vertical: 8),
                        enabledBorder: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(12),
                          borderSide: BorderSide(color: Colors.white.withValues(alpha: 0.1)),
                        ),
                        focusedBorder: OutlineInputBorder(
                          borderRadius: BorderRadius.circular(12),
                          borderSide: BorderSide(color: _purple.withValues(alpha: 0.6)),
                        ),
                      ),
                    ),
                  ),
                ],
              ),
            ),
          ),
          if (roleIds.isNotEmpty) ...[
            const SizedBox(height: 8),
            _blurred(
              child: Container(
                padding: const EdgeInsets.all(7),
                decoration: _glass(),
                // 宽屏上折行排开；窄屏上一行横着滑，不占太多地方。
                child: Builder(
                  builder: (context) {
                    final pills = [
                      _pill('全部 (${state.agents.length})', _purple, _roleFilter == null, () => _roleFilter = null),
                      for (final r in roleIds)
                        _pill(
                          '${_roleName(state.roles, r)} (${countFor[r]})',
                          _roleColor(map, state.roles, r),
                          _roleFilter == r,
                          () => _roleFilter = _roleFilter == r ? null : r,
                        ),
                    ];
                    if (wide) return Wrap(spacing: 6, runSpacing: 6, children: pills);
                    return SingleChildScrollView(
                      scrollDirection: Axis.horizontal,
                      child: Row(
                        children: [for (final p in pills) Padding(padding: const EdgeInsets.only(right: 6), child: p)],
                      ),
                    );
                  },
                ),
              ),
            ),
          ],
        ],
      ),
    );
  }

  Widget _chip(String text, Color color) => Container(
    padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1.5),
    decoration: BoxDecoration(
      color: color.withValues(alpha: 0.2),
      borderRadius: BorderRadius.circular(5),
      border: Border.all(color: color.withValues(alpha: 0.35)),
    ),
    child: Text(
      text,
      style: TextStyle(color: Color.lerp(color, Colors.white, 0.45), fontSize: 10, fontWeight: FontWeight.w600),
    ),
  );

  Widget _pill(String text, Color color, bool active, VoidCallback onTap) => InkWell(
    borderRadius: BorderRadius.circular(9),
    onTap: () => setState(onTap),
    child: Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 4),
      decoration: BoxDecoration(
        color: active ? color.withValues(alpha: 0.25) : Colors.white.withValues(alpha: 0.05),
        borderRadius: BorderRadius.circular(9),
        border: Border.all(color: active ? color.withValues(alpha: 0.5) : Colors.white.withValues(alpha: 0.1)),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: 7,
            height: 7,
            decoration: BoxDecoration(color: color, shape: BoxShape.circle),
          ),
          const SizedBox(width: 5),
          Text(
            text,
            style: TextStyle(
              fontSize: 11,
              fontWeight: FontWeight.w500,
              color: active ? Color.lerp(color, Colors.white, 0.6) : const Color(0xFFCBD5E1),
            ),
          ),
        ],
      ),
    ),
  );

  Widget _settings() {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.end,
      children: [
        _blurred(
          radius: 12,
          child: Material(
            color: _panel,
            shape: RoundedRectangleBorder(
              borderRadius: BorderRadius.circular(12),
              side: BorderSide(color: Colors.white.withValues(alpha: 0.1)),
            ),
            child: InkWell(
              borderRadius: BorderRadius.circular(12),
              onTap: () => setState(() => _settingsOpen = !_settingsOpen),
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
                child: Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Icon(Icons.tune, size: 16, color: Color(0xFFC084FC)),
                    if (_viewport.width >= 640) ...[
                      const SizedBox(width: 6),
                      const Text('星图参数', style: TextStyle(fontSize: 12, color: Color(0xFFE2E8F0))),
                    ],
                  ],
                ),
              ),
            ),
          ),
        ),
        if (_settingsOpen) ...[
          const SizedBox(height: 8),
          _blurred(
            child: Container(
              width: 250,
              padding: const EdgeInsets.all(14),
              decoration: _glass(border: Colors.white.withValues(alpha: 0.15)),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Row(
                    children: [
                      const Expanded(
                        child: Text(
                          '星图显示设置',
                          style: TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: Colors.white),
                        ),
                      ),
                      TextButton(
                        style: TextButton.styleFrom(
                          visualDensity: VisualDensity.compact,
                          foregroundColor: const Color(0xFFD8B4FE),
                        ),
                        onPressed: () => setState(() {
                          _orbitTime = 0;
                          _spacing = 1;
                          _center();
                        }),
                        child: const Text('重新排布', style: TextStyle(fontSize: 11)),
                      ),
                    ],
                  ),
                  const Divider(height: 12),
                  Row(
                    children: [
                      const Expanded(
                        child: Text('轨道间距', style: TextStyle(fontSize: 11, color: Color(0xFF94A3B8))),
                      ),
                      Text(
                        '${(_spacing * 100).round()}%',
                        style: const TextStyle(fontSize: 11, fontFamily: 'monospace', color: Color(0xFFE2E8F0)),
                      ),
                    ],
                  ),
                  SliderTheme(
                    data: const SliderThemeData(
                      trackHeight: 3,
                      thumbShape: RoundSliderThumbShape(enabledThumbRadius: 6),
                    ),
                    child: Slider(value: _spacing, min: 0.7, max: 1.6, onChanged: (v) => setState(() => _spacing = v)),
                  ),
                  _toggle('公转动画', _orbiting, (v) => _orbiting = v),
                  _toggle('显示紧急度轨道', _showOrbits, (v) => _showOrbits = v),
                  _toggle('显示数据流光点', _showPulses, (v) => _showPulses = v),
                  _toggle('常驻显示 Bot 名字', _showLabels, (v) => _showLabels = v),
                ],
              ),
            ),
          ),
        ],
      ],
    );
  }

  Widget _toggle(String label, bool value, void Function(bool) set) => SizedBox(
    height: 32,
    child: Row(
      children: [
        Expanded(
          child: Text(label, style: const TextStyle(fontSize: 12, color: Color(0xFFCBD5E1))),
        ),
        Transform.scale(
          scale: 0.7,
          child: Switch(value: value, onChanged: (v) => setState(() => set(v))),
        ),
      ],
    ),
  );

  Widget _stats(StarMap map, AppState state) {
    final urgent = map.bodies.where((b) => b.urgency >= urgencyBands.first.at).length;
    return IgnorePointer(
      child: _blurred(
        radius: 12,
        child: Container(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          decoration: _glass(radius: 12),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                width: 7,
                height: 7,
                decoration: BoxDecoration(
                  color: state.connected ? const Color(0xFF34D399) : const Color(0xFF64748B),
                  shape: BoxShape.circle,
                ),
              ),
              const SizedBox(width: 6),
              Text(
                state.connected ? '星图实时运行中' : '没连上主机',
                style: const TextStyle(fontSize: 11, color: Color(0xFFE2E8F0), fontWeight: FontWeight.w500),
              ),
              Text(
                ' · ${state.agents.length} 个 Bot / $urgent 个要你处理${map.routingWaiting > 0 ? ' / ${map.routingWaiting} 条等分派' : ''} · 距离 = 紧急度 · 大小 = 重要度',
                style: const TextStyle(fontSize: 11, color: Color(0xFF94A3B8)),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget _zoomBar(Size size) {
    Widget icon(IconData i, String tip, VoidCallback onTap) => IconButton(
      tooltip: tip,
      visualDensity: VisualDensity.compact,
      iconSize: 18,
      color: const Color(0xFFCBD5E1),
      onPressed: onTap,
      icon: Icon(i),
    );
    return _blurred(
      radius: 12,
      child: Container(
        padding: const EdgeInsets.all(4),
        decoration: _glass(radius: 12),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            icon(Icons.add, '放大', () => _zoomAt(size.center(Offset.zero), 1.2, size)),
            SizedBox(
              width: 44,
              child: Text(
                '${(_scale * 100).round()}%',
                textAlign: TextAlign.center,
                style: const TextStyle(fontSize: 11.5, fontFamily: 'monospace', color: Color(0xFFCBD5E1)),
              ),
            ),
            icon(Icons.remove, '缩小', () => _zoomAt(size.center(Offset.zero), 1 / 1.2, size)),
            Container(width: 1, height: 16, color: Colors.white.withValues(alpha: 0.15)),
            TextButton(
              style: TextButton.styleFrom(
                foregroundColor: const Color(0xFFE2E8F0),
                visualDensity: VisualDensity.compact,
              ),
              onPressed: _center,
              child: const Text('居中视图', style: TextStyle(fontSize: 12)),
            ),
          ],
        ),
      ),
    );
  }

  // ---------- 半透明对话气泡 ----------

  Size _bubbleSize(Size viewport) {
    final w = viewport.width < 640 ? viewport.width - 20 : math.min(820.0, viewport.width - 40);
    final h = math.min(680.0, viewport.height * (viewport.width < 640 ? 0.8 : 0.7));
    return Size(w, h);
  }

  Widget _bubble(StarMap map, AppState state, Size viewport) {
    final size = _bubbleSize(viewport);
    final pos = _bubblePos ?? Offset((viewport.width - size.width) / 2, (viewport.height - size.height) / 2);
    final id = _selected!;
    final b = map.byId(id);
    final celestial = b?.celestial ?? sun;
    final color = b == null ? _sunColor : (b == map.chief ? _purple : _roleColor(map, state.roles, b.agent.role));
    final wide = size.width >= 600;

    Widget header() => GestureDetector(
      behavior: HitTestBehavior.opaque,
      onPanUpdate: (d) => setState(() {
        final next = (_bubblePos ?? pos) + d.delta;
        _bubblePos = Offset(
          next.dx.clamp(-size.width + 80, viewport.width - 80),
          next.dy.clamp(0, viewport.height - 48),
        );
      }),
      child: MouseRegion(
        cursor: SystemMouseCursors.move,
        child: Container(
          padding: const EdgeInsets.fromLTRB(14, 10, 8, 10),
          decoration: BoxDecoration(
            color: Colors.white.withValues(alpha: 0.04),
            border: Border(bottom: BorderSide(color: Colors.white.withValues(alpha: 0.1))),
          ),
          child: Row(
            children: [
              ValueListenableBuilder<double>(
                valueListenable: _clock,
                builder: (_, t, _) => CelestialIcon(celestial, size: 34, t: t),
              ),
              const SizedBox(width: 10),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Flexible(
                          child: Text(
                            b == null ? '幕僚长（空位）' : b.agent.name,
                            overflow: TextOverflow.ellipsis,
                            style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w700, color: Colors.white),
                          ),
                        ),
                        const SizedBox(width: 8),
                        if (b != null) _chip(b == map.chief ? '幕僚长' : _roleName(state.roles, b.agent.role), color),
                        const SizedBox(width: 6),
                        _chip('${celestial.name} ${celestial.english}', _sunColor),
                      ],
                    ),
                    const SizedBox(height: 2),
                    Text(
                      '按住顶栏可拖动气泡 · 背景实时透视星图',
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(fontSize: 10.5, color: Colors.white.withValues(alpha: 0.55)),
                    ),
                  ],
                ),
              ),
              if (b != null)
                TextButton.icon(
                  onPressed: widget.onOpenChat,
                  icon: const Icon(Icons.open_in_new, size: 14),
                  label: Text(wide ? '在对话页打开' : '对话页', style: const TextStyle(fontSize: 11.5)),
                ),
              IconButton(
                tooltip: '关闭气泡（Esc）',
                onPressed: () => setState(() => _selected = null),
                icon: const Icon(Icons.close, size: 18),
              ),
            ],
          ),
        ),
      ),
    );

    Widget strip() {
      if (b == null) return const SizedBox.shrink();
      final band = urgencyBands.firstWhere((x) => b.urgency >= x.at);
      final neighborIds = map.neighbors(b.agent.id).toList();
      return Container(
        padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 7),
        decoration: BoxDecoration(
          color: Colors.black.withValues(alpha: 0.25),
          border: Border(bottom: BorderSide(color: Colors.white.withValues(alpha: 0.1))),
        ),
        child: SingleChildScrollView(
          scrollDirection: Axis.horizontal,
          child: Row(
            children: [
              if (b != map.chief) ...[
                _chip('紧急度 ${(b.urgency * 100).round()}% · ${band.label}', _urgencyColor(b.urgency)),
                const SizedBox(width: 6),
                _chip('重要度 ${(b.importance * 100).round()}%', const Color(0xFF22D3EE)),
                const SizedBox(width: 8),
              ],
              Text('[[关联]]：', style: TextStyle(fontSize: 11, color: Colors.white.withValues(alpha: 0.5))),
              if (neighborIds.isEmpty)
                Text('暂无', style: TextStyle(fontSize: 11, color: Colors.white.withValues(alpha: 0.4))),
              for (final nid in neighborIds)
                Padding(padding: const EdgeInsets.only(right: 6), child: _linkChip(map, state, nid)),
            ],
          ),
        ),
      );
    }

    Widget why() => ListView(
      padding: const EdgeInsets.all(14),
      children: [
        if (b == null) ...[
          const Text('太阳的位置留给幕僚长。', style: TextStyle(fontWeight: FontWeight.w600)),
          const SizedBox(height: 6),
          Text(
            '主机上没有协调员岗位。在 ip.yaml 里加一个 id 含 coordinator 的岗位（或岗位名里有“协调”“幕僚”），'
            '它的员工就会坐到这里，其他员工绕着它转。',
            style: TextStyle(fontSize: 12.5, color: Colors.white.withValues(alpha: 0.75), height: 1.5),
          ),
          if (map.routingWaiting > 0) ...[
            const SizedBox(height: 10),
            Text('有 ${map.routingWaiting} 条消息等你分派，在“待办”里。', style: const TextStyle(color: Color(0xFFFACC15))),
          ],
        ] else ...[
          Text(
            b == map.chief ? '幕僚长在中心，统筹派活。' : '为什么在这条轨道上',
            style: const TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: Colors.white),
          ),
          const SizedBox(height: 8),
          for (final r in b.reasons) _reasonRow(r),
          if (b == map.chief && map.routingWaiting > 0)
            _reasonRow(Reason('有 ${map.routingWaiting} 条消息等你分派（在“待办”里）', 0.8)),
          const SizedBox(height: 12),
          const Text(
            '为什么这么大',
            style: TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: Colors.white),
          ),
          const SizedBox(height: 6),
          for (final n in b.importanceNotes)
            Padding(
              padding: const EdgeInsets.only(bottom: 4),
              child: Text(
                '· $n',
                style: TextStyle(fontSize: 12, color: Colors.white.withValues(alpha: 0.7), height: 1.4),
              ),
            ),
          for (final a in state.pendingApprovals.where((a) => a.isAgentQuestion && a.subjectAgentId == b.agent.id))
            Padding(
              padding: const EdgeInsets.only(top: 10),
              child: ApprovalCard(approval: a, onOpenThread: (thread) => AppScope.read(context).openThread(thread)),
            ),
        ],
      ],
    );

    final body = b == null
        ? why()
        : wide
        ? Row(
            children: [
              SizedBox(width: size.width * 0.4, child: why()),
              VerticalDivider(width: 1, color: Colors.white.withValues(alpha: 0.12)),
              const Expanded(child: ThreadView()),
            ],
          )
        : Column(
            children: [
              ConstrainedBox(
                constraints: BoxConstraints(maxHeight: size.height * 0.32),
                child: why(),
              ),
              Divider(height: 1, color: Colors.white.withValues(alpha: 0.12)),
              const Expanded(child: ThreadView()),
            ],
          );

    return Positioned(
      left: pos.dx,
      top: pos.dy,
      width: size.width,
      height: size.height,
      // 不加缩放、淡入的打开动画：背景模糊放在透明度或变换图层里时，有的渲染后端（Impeller 的 GL 后端）会把整个气泡丢掉。
      child: KeyedSubtree(
        key: ValueKey(id),
        child: ClipRRect(
          borderRadius: BorderRadius.circular(24),
          child: BackdropFilter(
            filter: ui.ImageFilter.blur(sigmaX: 22, sigmaY: 22),
            child: Material(
              color: const Color(0xA6111522),
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(24),
                side: BorderSide(color: color.withValues(alpha: 0.4)),
              ),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  header(),
                  strip(),
                  Expanded(child: body),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }

  Widget _reasonRow(Reason r) {
    final c = _urgencyColor(r.weight);
    final thread = r.threadId;
    return InkWell(
      borderRadius: BorderRadius.circular(8),
      onTap: thread == null ? null : () => AppScope.read(context).openThread(thread),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 4, horizontal: 2),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(
              margin: const EdgeInsets.only(top: 5, right: 8),
              width: 7,
              height: 7,
              decoration: BoxDecoration(
                color: c,
                shape: BoxShape.circle,
                boxShadow: [BoxShadow(color: c.withValues(alpha: 0.6), blurRadius: 6)],
              ),
            ),
            Expanded(
              child: Text(
                r.text,
                style: TextStyle(fontSize: 12.5, color: Colors.white.withValues(alpha: 0.88), height: 1.4),
              ),
            ),
            if (thread != null) Icon(Icons.chevron_right, size: 16, color: Colors.white.withValues(alpha: 0.4)),
          ],
        ),
      ),
    );
  }

  Widget _linkChip(StarMap map, AppState state, String id) {
    final b = map.byId(id);
    if (b == null) return const SizedBox.shrink();
    final color = b == map.chief ? _purple : _roleColor(map, state.roles, b.agent.role);
    return InkWell(
      borderRadius: BorderRadius.circular(6),
      onTap: () {
        final placed = _place(map, state, _viewport);
        final p = placed.firstWhere((x) => x.body?.agent.id == id, orElse: () => placed.first);
        final world = (p.pos - _viewport.center(Offset.zero) - _pan) / _scale;
        _jumpTo(map, id, world);
      },
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
        decoration: BoxDecoration(
          color: Colors.white.withValues(alpha: 0.06),
          borderRadius: BorderRadius.circular(6),
          border: Border.all(color: Colors.white.withValues(alpha: 0.1)),
        ),
        child: Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Container(
              width: 6,
              height: 6,
              decoration: BoxDecoration(color: color, shape: BoxShape.circle),
            ),
            const SizedBox(width: 4),
            Text('[[${b.agent.name}]]', style: const TextStyle(fontSize: 10.5, color: Color(0xFFE2E8F0))),
          ],
        ),
      ),
    );
  }
}

Color _urgencyColor(double u) => u >= 0.8
    ? const Color(0xFFF87171)
    : u >= 0.6
    ? const Color(0xFFFBBF24)
    : u >= 0.3
    ? const Color(0xFF38BDF8)
    : const Color(0xFF64748B);

/// 一颗天体本身，外加选中、悬停、搜索命中、要你处理的光圈。
class _BodyPainter extends CustomPainter {
  _BodyPainter({
    required this.celestial,
    required this.radius,
    required this.t,
    required this.ring,
    required this.searchHit,
    required this.hot,
    required this.color,
    required this.box,
  });

  final Celestial celestial;
  final double radius;

  /// 画布的边长，天体在正中。
  final double box;
  final double t;
  final Color? ring;
  final bool searchHit;
  final bool hot;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final c = size.center(Offset.zero);
    // 要你处理的员工：一圈红色的呼吸光环。
    if (hot) {
      final pulse = (t * 0.8) % 1;
      canvas.drawCircle(
        c,
        radius * (1.3 + pulse * 0.9),
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2
          ..color = const Color(0xFFF87171).withValues(alpha: 0.7 * (1 - pulse)),
      );
    }
    paintCelestial(canvas, celestial, c, radius, t);
    if (searchHit) {
      canvas.drawCircle(
        c,
        radius * 1.35 + 3 + math.sin(t * 6) * 2.5,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2
          ..color = const Color(0xFFFACC15),
      );
    }
    final r = ring;
    if (r != null) {
      canvas.drawCircle(
        c,
        radius * 1.32 + 2 + math.sin(t * 4) * 1.5,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2
          ..color = r.withValues(alpha: 0.85),
      );
    }
  }

  /// 只有天体本身（含一点光晕）算点中，外框的空白处不算。
  @override
  bool? hitTest(Offset position) => (position - Offset(box / 2, box / 2)).distance <= math.max(radius * 1.45, 22);

  @override
  bool shouldRepaint(_BodyPainter old) => true;
}

/// 星空、紧急度轨道、连线、数据流光点和牵引线。
class _SkyPainter extends CustomPainter {
  _SkyPainter({
    required this.t,
    required this.pan,
    required this.scale,
    required this.stars,
    required this.sunPos,
    required this.placed,
    required this.links,
    required this.showOrbits,
    required this.showPulses,
    required this.spacing,
    required this.focus,
    required this.neighbors,
    required this.alphaOf,
    required this.tether,
  });

  final double t;
  final Offset pan;
  final double scale;
  final List<_BgStar> stars;
  final Offset sunPos;
  final List<_Placed> placed;
  final List<BotLink> links;
  final bool showOrbits;
  final bool showPulses;
  final double spacing;
  final String? focus;
  final Set<String> neighbors;
  final double Function(String id, BotBody? b) alphaOf;
  final Offset? tether;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    // 1. 深空底色。
    canvas.drawRect(
      rect,
      Paint()
        ..shader = RadialGradient(
          colors: const [Color(0xFF141830), Color(0xFF0B0D13), Color(0xFF05060A)],
          stops: const [0.0, 0.55, 1.0],
          center: Alignment(pan.dx / size.width * 0.1, pan.dy / size.height * 0.1),
        ).createShader(rect),
    );
    // 2. 星云和银河带，随平移有一点视差。
    final m = size.longestSide;
    for (final (x, y, r, color) in [
      (0.18, 0.28, 0.42, const Color(0x2EA855F7)),
      (0.82, 0.72, 0.5, const Color(0x266366F1)),
      (0.7, 0.12, 0.3, const Color(0x1A22D3EE)),
      (0.35, 0.85, 0.32, const Color(0x14F472B6)),
    ]) {
      final c = Offset(x * size.width, y * size.height) + pan * 0.03;
      final box = Rect.fromCircle(center: c, radius: r * m);
      // 渐变淡到透明，画成矩形就够了。不用 drawCircle：Impeller 的 GL 后端在一些驱动上画带渐变的大圆会出方块。
      canvas.drawRect(
        box,
        Paint()..shader = RadialGradient(colors: [color, color.withValues(alpha: 0)]).createShader(box),
      );
    }
    canvas.save();
    canvas.translate(size.width / 2 + pan.dx * 0.02, size.height / 2 + pan.dy * 0.02);
    canvas.rotate(-0.42);
    final band = Rect.fromCenter(center: Offset.zero, width: m * 2, height: m * 0.36);
    canvas.drawRect(
      band,
      Paint()
        ..shader = const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [Color(0x00FFFFFF), Color(0x0FC7D2FE), Color(0x00FFFFFF)],
        ).createShader(band),
    );
    canvas.restore();
    // 3. 闪烁的星点（三层视差）。
    final dot = Paint();
    for (final s in stars) {
      var x = (s.x * size.width + pan.dx * s.depth) % size.width;
      var y = (s.y * size.height + pan.dy * s.depth) % size.height;
      if (x < 0) x += size.width;
      if (y < 0) y += size.height;
      final a = (s.alpha * (0.6 + 0.4 * math.sin(t * s.speed + s.phase))).clamp(0.0, 1.0);
      dot.color = s.color.withValues(alpha: a);
      canvas.drawCircle(Offset(x, y), s.r, dot);
      if (s.r > 1.2) {
        dot.color = s.color.withValues(alpha: a * 0.18);
        canvas.drawCircle(Offset(x, y), s.r * 3.2, dot);
      }
    }

    // 4. 紧急度轨道：一档一圈，标上名字。
    if (showOrbits) {
      final ring = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.2;
      for (final b in urgencyBands) {
        final r = orbitRadius(b.at, inner: 130 * spacing, outer: 130 * spacing + 260 * spacing) * scale;
        ring.color = _urgencyColor(b.at).withValues(alpha: 0.16);
        // 画成折线：Impeller 的 GL 后端在一些驱动（如 Mesa 软件渲染）上画 1 像素的圆描边，会画成一整块方的。
        canvas.drawPath(
          Path()..addPolygon([
            for (var i = 0; i < 160; i++) sunPos + Offset(math.cos(i * math.pi / 80), math.sin(i * math.pi / 80)) * r,
          ], true),
          ring,
        );
        _label(canvas, b.label, sunPos + Offset(0, -r), _urgencyColor(b.at).withValues(alpha: 0.75), 10);
      }
    }

    // 5. 连线：太阳到每位员工（派活），员工之间的任务（派活的 → 负责人）。
    final chief = placed.first;
    final chiefId = chief.body?.agent.id ?? '\u0000sun';
    final pulse = (t * 0.35) % 1;
    for (final p in placed.skip(1)) {
      final id = p.body!.agent.id;
      final isFocus = focus != null && (focus == id || focus == chiefId);
      final a = focus != null ? (isFocus ? 0.85 : 0.05) : 0.22 * alphaOf(id, p.body);
      _edge(canvas, chief.pos, p.pos, _sunColor, p.color, a, isFocus ? 1.8 : 1.0);
      if (showPulses && p.body!.working && a > 0.1) _pulse(canvas, chief.pos, p.pos, pulse, p.color, isFocus);
    }
    for (final l in links) {
      final from = placed.where((p) => p.body?.agent.id == l.from).firstOrNull;
      final to = placed.where((p) => p.body?.agent.id == l.to).firstOrNull;
      if (from == null || to == null) continue;
      final isFocus = focus != null && (focus == l.from || focus == l.to);
      final a = focus != null ? (isFocus ? 0.92 : 0.06) : 0.4;
      _edge(canvas, from.pos, to.pos, from.color, to.color, a, isFocus ? 2.2 : 1.2);
      _arrow(canvas, from.pos, to.pos, to.radius + 6, to.color.withValues(alpha: a));
      if (showPulses && l.active && a > 0.15) _pulse(canvas, from.pos, to.pos, (pulse + 0.5) % 1, from.color, isFocus);
      // 任务名只在悬停、选中或放大看时显示，平时不挤在太阳旁边。
      if (isFocus || (focus == null && scale >= 1.25)) {
        _label(
          canvas,
          l.label,
          // 标签放在靠近负责人的一侧，免得压在太阳的名字上。
          Offset.lerp(from.pos, to.pos, 0.62)!,
          isFocus ? const Color(0xFFF3E8FF) : const Color(0xFF94A3B8),
          10,
        );
      }
    }

    // 6. 选中的员工到气泡的牵引线（虚线，往气泡流动）。
    final tg = tether;
    if (tg != null) {
      final sel = placed.where((p) => (p.body?.agent.id ?? '\u0000sun') == focus).firstOrNull;
      if (sel != null) {
        final ctrl = Offset((sel.pos.dx + tg.dx) / 2, math.min(sel.pos.dy, tg.dy) - 45);
        final path = Path()
          ..moveTo(sel.pos.dx, sel.pos.dy)
          ..quadraticBezierTo(ctrl.dx, ctrl.dy, tg.dx, tg.dy);
        final paint = Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2
          ..shader = ui.Gradient.linear(sel.pos, tg, [
            sel.color.withValues(alpha: 0.8),
            Colors.white.withValues(alpha: 0.2),
          ]);
        for (final metric in path.computeMetrics()) {
          const dash = 6.0, gap = 5.0;
          var d = -(t * 40 % (dash + gap));
          while (d < metric.length) {
            final s = math.max(0.0, d);
            final e = math.min(metric.length, d + dash);
            if (e > s) canvas.drawPath(metric.extractPath(s, e), paint);
            d += dash + gap;
          }
        }
      }
    }
  }

  void _edge(Canvas canvas, Offset a, Offset b, Color ca, Color cb, double alpha, double width) {
    if (alpha <= 0.01) return;
    canvas.drawLine(
      a,
      b,
      Paint()
        ..strokeWidth = width
        ..shader = ui.Gradient.linear(a, b, [ca.withValues(alpha: alpha), cb.withValues(alpha: alpha)]),
    );
  }

  void _arrow(Canvas canvas, Offset from, Offset to, double back, Color color) {
    final angle = math.atan2(to.dy - from.dy, to.dx - from.dx);
    canvas.save();
    canvas.translate(to.dx - math.cos(angle) * back, to.dy - math.sin(angle) * back);
    canvas.rotate(angle);
    canvas.drawPath(
      Path()
        ..moveTo(0, 0)
        ..lineTo(-7, -3.2)
        ..lineTo(-5.5, 0)
        ..lineTo(-7, 3.2)
        ..close(),
      Paint()..color = color,
    );
    canvas.restore();
  }

  void _pulse(Canvas canvas, Offset a, Offset b, double p, Color color, bool focus) {
    final at = Offset.lerp(a, b, p)!;
    canvas.drawCircle(at, focus ? 5 : 4, Paint()..color = color.withValues(alpha: 0.35));
    canvas.drawCircle(at, focus ? 2.6 : 2, Paint()..color = Colors.white);
  }

  void _label(Canvas canvas, String text, Offset at, Color color, double fontSize) {
    final tp = TextPainter(
      text: TextSpan(
        text: text,
        style: TextStyle(color: color, fontSize: fontSize, fontWeight: FontWeight.w500),
      ),
      textDirection: TextDirection.ltr,
    )..layout();
    final box = Rect.fromCenter(center: at, width: tp.width + 8, height: tp.height + 2);
    canvas.drawRRect(RRect.fromRectAndRadius(box, const Radius.circular(4)), Paint()..color = const Color(0xE00B0D13));
    tp.paint(canvas, box.topLeft + const Offset(4, 1));
  }

  @override
  bool shouldRepaint(_SkyPainter old) => true;
}
