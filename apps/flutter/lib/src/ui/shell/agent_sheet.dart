// 宽窗口的浮空玻璃窗口与工作台面板（设计重构，2026-11-26 维护者定）：
//   - 外观：四边轻微离地浮空的 macOS Liquid Glass 原生窗口（连续超椭圆外圆角 26、高模糊 + 1.75x 增饱和 +
//     顶部受光清透白渐变 + 0.8px 镜面高光边 + 柔和深蓝环境投影），让背后文艺复兴蓝天与金色弧光自然透色。
//   - 架构：去掉顶天立地的硬竖线与冗余的灰色拖拽横杠，改为明度与通透度分层——
//     左栏与右栏直接吃透背后的液态玻璃，中间聊天/主内容区是一块内缩 6px、同心超椭圆（圆角 20）的高亮悬浮面；
//     三栏顶栏严格落在同一条 42px 水平中线上，窗口左上角配备原生红绿灯（红关、黄收起、绿切换标准/高视野）。
//   - 交互：点空白天空、按 Esc、点红/黄灯、或按住顶栏空白区往下拉均可流畅收起；绿灯在 72% 与 86% 高度间切换。

import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../../app_state.dart';
import '../../protocol/models.dart';
import '../../star_map.dart';
import '../chat_page.dart' show ThreadList, ThreadView;
import '../common.dart';
import '../inbox_page.dart' show ApprovalCard, InboxPage;
import '../roles_page.dart' show RolesPage;
import '../tasks_page.dart' show TasksPage;
import 'tokens.dart';

/// 面板里的几页。
enum PanelSection {
  inbox('待办', Icons.inbox_outlined),
  chat('对话', Icons.chat_bubble_outline),
  tasks('任务', Icons.task_alt_outlined),
  roles('岗位', Icons.groups_outlined);

  const PanelSection(this.label, this.icon);
  final String label;
  final IconData icon;

  /// 右栏（员工详情）默认开不开：对话开着，别的页收起，给中间多留地方。
  bool get detailByDefault => this == PanelSection.chat;
}

/// 打开一位员工最相关的对话（和原来星图一样）：在问你的、受阻或在做的任务的执行对话、这次打开过的；
/// 没有就开一个发给它的新对话。
void openAgentThread(AppState state, StarMap map, String id) {
  final b = map.byId(id);
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

/// 提高背景饱和度 1.75 倍的颜色矩阵（Liquid Glass 清透鲜活感）。
const liquidSaturateMatrix = <double>[
  1.590, -0.536, -0.054, 0, 0, //
  -0.160, 1.214, -0.054, 0, 0,
  -0.160, -0.536, 1.696, 0, 0,
  0, 0, 0, 1, 0,
];

/// 把顶栏空白区接进外层浮空窗的下拉收起手势（不额外占垂直高度）。
class _SheetDragScope extends InheritedWidget {
  const _SheetDragScope({required this.onUpdate, required this.onEnd, required super.child});

  final GestureDragUpdateCallback onUpdate;
  final GestureDragEndCallback onEnd;

  static _SheetDragScope? maybeOf(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_SheetDragScope>();

  @override
  bool updateShouldNotify(_SheetDragScope old) => onUpdate != old.onUpdate || onEnd != old.onEnd;
}

/// 包住窗口顶栏区域：按住往下拖可流畅下拉收起浮空窗。
class SheetDragRegion extends StatelessWidget {
  const SheetDragRegion({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final scope = _SheetDragScope.maybeOf(context);
    if (scope == null) return child;
    return GestureDetector(
      behavior: HitTestBehavior.translucent,
      onVerticalDragUpdate: scope.onUpdate,
      onVerticalDragEnd: scope.onEnd,
      child: child,
    );
  }
}

/// 从底部滑上来的浮空 macOS Liquid Glass 窗口外壳：开关动画、标准/大视野平滑伸缩、下拉收起、超椭圆玻璃。
class FrostedSheet extends StatefulWidget {
  const FrostedSheet({
    super.key,
    required this.open,
    this.expanded = false,
    required this.onClose,
    required this.child,
  });

  final bool open;

  /// 绿灯切换的大视野高度（86%），默认标准高度（72%）。
  final bool expanded;
  final VoidCallback onClose;
  final Widget child;

  /// 标准高度占窗口高度的比例（至少 2/3）；太阳停在窗口上沿露出的那条天空里。
  static const heightFactor = 0.72;

  /// 大视野专注高度比例。
  static const expandedHeightFactor = 0.86;

  /// 四周离屏留白：底部 16px，左右各 24px。
  static const bottomMargin = 16.0;
  static const horizontalMargin = 24.0;

  /// 严格同心超椭圆：外窗 26，内嵌主面缩进 6 -> 内圆角 20（26 - 6 = 20）。
  static const outerRadius = 26.0;
  static const innerInset = 6.0;
  static const innerRadius = outerRadius - innerInset;

  static double heightFor(double windowHeight, {bool expanded = false}) =>
      windowHeight * (expanded ? expandedHeightFactor : heightFactor);

  /// 浮空窗顶边离窗口顶多远。
  static double topFor(double windowHeight, {bool expanded = false}) =>
      windowHeight - bottomMargin - heightFor(windowHeight, expanded: expanded);

  @override
  State<FrostedSheet> createState() => _FrostedSheetState();
}

class _FrostedSheetState extends State<FrostedSheet> with TickerProviderStateMixin {
  late final AnimationController _c = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 320),
    reverseDuration: const Duration(milliseconds: 220),
    value: widget.open ? 1 : 0,
  );
  late final AnimationController _expand = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 260),
    value: widget.expanded ? 1 : 0,
  );

  /// 往下拉了多少（逻辑像素）。
  double _drag = 0;

  @override
  void didUpdateWidget(FrostedSheet old) {
    super.didUpdateWidget(old);
    if (old.open != widget.open) {
      _drag = 0;
      widget.open ? _c.animateTo(1, curve: Motion.curve) : _c.animateBack(0, curve: Curves.easeIn);
    }
    if (old.expanded != widget.expanded) {
      _expand.animateTo(widget.expanded ? 1 : 0, curve: Motion.curve);
    }
  }

  @override
  void dispose() {
    _c.dispose();
    _expand.dispose();
    super.dispose();
  }

  void _dragUpdate(DragUpdateDetails d) => setState(() => _drag = (_drag + d.delta.dy).clamp(0.0, 4000.0));

  void _dragEnd(DragEndDetails d, double height) {
    if (_drag > height / 3 || d.velocity.pixelsPerSecond.dy > 700) {
      widget.onClose();
    } else {
      setState(() => _drag = 0);
    }
  }

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, box) {
        final width = (box.maxWidth - FrostedSheet.horizontalMargin * 2).clamp(0.0, 1420.0);
        final normalH = FrostedSheet.heightFor(box.maxHeight, expanded: false);
        final expandedH = FrostedSheet.heightFor(box.maxHeight, expanded: true);
        return AnimatedBuilder(
          animation: Listenable.merge([_c, _expand]),
          builder: (context, child) {
            if (_c.value == 0) return const SizedBox.shrink();
            final height = ui.lerpDouble(normalH, expandedH, _expand.value)!;
            // 用位置（不用变换）做滑动：背景模糊放在变换图层里时，有的渲染后端会把整块丢掉（star_map_page.dart 里记过）。
            final bottom = ui.lerpDouble(-(height + 32), FrostedSheet.bottomMargin, _c.value)! - _drag;
            return _SheetDragScope(
              onUpdate: _dragUpdate,
              onEnd: (d) => _dragEnd(d, height),
              child: Stack(
                children: [
                  Positioned(
                    left: (box.maxWidth - width) / 2,
                    width: width,
                    height: height,
                    bottom: bottom,
                    child: child!,
                  ),
                ],
              ),
            );
          },
          child: _Frost(child: widget.child),
        );
      },
    );
  }
}

/// 浮空 macOS Liquid Glass 玻璃壳：外层柔和深蓝投影 + 真实超椭圆裁剪 + 36px 模糊增饱和 + 顶部受光微渐变 + 镜面高光边。
class _Frost extends StatelessWidget {
  const _Frost({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final br = BorderRadius.circular(FrostedSheet.outerRadius);
    return Theme(
      data: _glass,
      child: DecoratedBox(
        decoration: ShapeDecoration(
          shape: RoundedSuperellipseBorder(borderRadius: br),
          shadows: const [
            BoxShadow(color: Color(0x52081230), offset: Offset(0, 24), blurRadius: 56, spreadRadius: -14),
            BoxShadow(color: Color(0x1C081230), offset: Offset(0, 4), blurRadius: 14),
          ],
        ),
        child: ClipRSuperellipse(
          borderRadius: br,
          child: BackdropFilter(
            filter: ui.ImageFilter.compose(
              outer: const ui.ColorFilter.matrix(liquidSaturateMatrix),
              inner: ui.ImageFilter.blur(sigmaX: 36, sigmaY: 36),
            ),
            child: DecoratedBox(
              decoration: ShapeDecoration(
                gradient: const LinearGradient(
                  begin: Alignment.topLeft,
                  end: Alignment.bottomRight,
                  colors: [Color(0xBAFFFFFF), Color(0x94F4F8FE), Color(0xA6EFF5FD)],
                  stops: [0.0, 0.54, 1.0],
                ),
                shape: RoundedSuperellipseBorder(
                  borderRadius: br,
                  side: const BorderSide(color: Color(0xCCFFFFFF), width: 0.8),
                ),
              ),
              child: Material(type: MaterialType.transparency, child: child),
            ),
          ),
        ),
      ),
    );
  }
}

/// 窗口内全局清透 Liquid Glass 主题（深色模式下也一致）。
final _glass = glassTheme();

/// 窗口左上角 macOS 原生红绿灯（红关、黄收起、绿切换标准/高视野）；悬停时露出极细原生长符号。
class MacTrafficLights extends StatefulWidget {
  const MacTrafficLights({
    super.key,
    required this.onClose,
    this.onCollapse,
    this.onToggleExpand,
    this.expanded = false,
  });

  final VoidCallback onClose;
  final VoidCallback? onCollapse;
  final VoidCallback? onToggleExpand;
  final bool expanded;

  @override
  State<MacTrafficLights> createState() => _MacTrafficLightsState();
}

class _MacTrafficLightsState extends State<MacTrafficLights> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    Widget dot({
      required Key key,
      required String tooltip,
      required Color fill,
      required Color ring,
      required _TrafficGlyph glyph,
      required VoidCallback? onTap,
    }) => Tooltip(
      message: tooltip,
      child: GestureDetector(
        key: key,
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 3.5, vertical: 6),
          child: Container(
            width: 12,
            height: 12,
            decoration: BoxDecoration(
              color: onTap == null ? fill.withValues(alpha: 0.4) : fill,
              shape: BoxShape.circle,
              border: Border.all(color: ring, width: 0.6),
            ),
            child: _hover && onTap != null ? CustomPaint(painter: _TrafficGlyphPainter(glyph)) : null,
          ),
        ),
      ),
    );

    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          dot(
            key: const ValueKey('window-close'),
            tooltip: '关闭面板（Esc）',
            fill: const Color(0xFFFF5F57),
            ring: const Color(0x47B41912),
            glyph: _TrafficGlyph.close,
            onTap: widget.onClose,
          ),
          dot(
            key: const ValueKey('window-collapse'),
            tooltip: '收起窗口',
            fill: const Color(0xFFFEBC2E),
            ring: const Color(0x47B47300),
            glyph: _TrafficGlyph.minimize,
            onTap: widget.onCollapse ?? widget.onClose,
          ),
          dot(
            key: const ValueKey('window-expand'),
            tooltip: widget.expanded ? '恢复标准高度' : '展开更高视野',
            fill: const Color(0xFF28C840),
            ring: const Color(0x47108224),
            glyph: widget.expanded ? _TrafficGlyph.restore : _TrafficGlyph.expand,
            onTap: widget.onToggleExpand,
          ),
        ],
      ),
    );
  }
}

enum _TrafficGlyph { close, minimize, expand, restore }

class _TrafficGlyphPainter extends CustomPainter {
  const _TrafficGlyphPainter(this.glyph);

  final _TrafficGlyph glyph;

  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()
      ..color = const Color(0xA6000000)
      ..strokeWidth = 1.15
      ..strokeCap = StrokeCap.round
      ..style = PaintingStyle.stroke;
    final c = size.center(Offset.zero);
    switch (glyph) {
      case _TrafficGlyph.close:
        canvas.drawLine(c + const Offset(-2.3, -2.3), c + const Offset(2.3, 2.3), p);
        canvas.drawLine(c + const Offset(2.3, -2.3), c + const Offset(-2.3, 2.3), p);
      case _TrafficGlyph.minimize:
        canvas.drawLine(c + const Offset(-2.8, 0), c + const Offset(2.8, 0), p);
      case _TrafficGlyph.expand:
        // 两条对角微箭头表示展开更高视野。
        canvas.drawLine(c + const Offset(-2.3, 2.3), c + const Offset(2.3, -2.3), p);
        canvas.drawLine(c + const Offset(0.2, -2.3), c + const Offset(2.3, -2.3), p);
        canvas.drawLine(c + const Offset(2.3, -2.3), c + const Offset(2.3, -0.2), p);
        canvas.drawLine(c + const Offset(-2.3, 0.2), c + const Offset(-2.3, 2.3), p);
        canvas.drawLine(c + const Offset(-2.3, 2.3), c + const Offset(-0.2, 2.3), p);
      case _TrafficGlyph.restore:
        canvas.drawLine(c + const Offset(-2.5, 0), c + const Offset(2.5, 0), p);
        canvas.drawLine(c + const Offset(0, -2.5), c + const Offset(0, 2.5), p);
    }
  }

  @override
  bool shouldRepaint(_TrafficGlyphPainter old) => old.glyph != glyph;
}

/// 顶栏精致的圆形液态玻璃小按钮（+ 新对话、收起/展开右栏等）。
class GlassIconButton extends StatefulWidget {
  const GlassIconButton({
    super.key,
    required this.icon,
    required this.tooltip,
    required this.onPressed,
    this.size = 26,
    this.iconSize = 15,
    this.active = false,
  });

  final IconData icon;
  final String tooltip;
  final VoidCallback? onPressed;
  final double size;
  final double iconSize;
  final bool active;

  @override
  State<GlassIconButton> createState() => _GlassIconButtonState();
}

class _GlassIconButtonState extends State<GlassIconButton> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final enabled = widget.onPressed != null;
    final bg = !enabled
        ? const Color(0x47FFFFFF)
        : _hover
        ? const Color(0xE6FFFFFF)
        : widget.active
        ? const Color(0xBAFFFFFF)
        : const Color(0x8CFFFFFF);
    return Tooltip(
      message: widget.tooltip,
      child: MouseRegion(
        cursor: enabled ? SystemMouseCursors.click : SystemMouseCursors.basic,
        onEnter: (_) => setState(() => _hover = true),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: widget.onPressed,
          child: AnimatedContainer(
            duration: Motion.fast,
            curve: Motion.curve,
            width: widget.size,
            height: widget.size,
            decoration: BoxDecoration(
              color: bg,
              shape: BoxShape.circle,
              border: Border.all(color: const Color(0xC2FFFFFF), width: 0.65),
              boxShadow: const [BoxShadow(color: Color(0x120E183A), offset: Offset(0, 1), blurRadius: 3)],
            ),
            child: Center(
              child: Icon(
                widget.icon,
                size: widget.iconSize,
                color: enabled ? (widget.active ? s.ink : s.inkSecondary) : s.inkTertiary,
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Apple 风格 Segmented Control（浅凹槽 + 白色微投影滑块）。
class GlassSegmentedControl<T> extends StatelessWidget {
  const GlassSegmentedControl({super.key, required this.items, required this.value, required this.onChanged});

  final List<({T value, String label, Key? key})> items;
  final T value;
  final ValueChanged<T> onChanged;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Container(
      height: 28,
      padding: const EdgeInsets.all(2.5),
      decoration: ShapeDecoration(
        color: s.inset,
        shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(9.5)),
      ),
      child: Row(
        children: [
          for (final item in items)
            Expanded(
              child: _SegmentItem(
                key: item.key,
                label: item.label,
                selected: item.value == value,
                onTap: () => onChanged(item.value),
              ),
            ),
        ],
      ),
    );
  }
}

class _SegmentItem extends StatefulWidget {
  const _SegmentItem({super.key, required this.label, required this.selected, required this.onTap});

  final String label;
  final bool selected;
  final VoidCallback onTap;

  @override
  State<_SegmentItem> createState() => _SegmentItemState();
}

class _SegmentItemState extends State<_SegmentItem> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (_) => setState(() => _hover = false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onTap,
        child: AnimatedContainer(
          duration: Motion.fast,
          curve: Motion.curve,
          alignment: Alignment.center,
          decoration: ShapeDecoration(
            color: widget.selected
                ? const Color(0xF2FFFFFF)
                : _hover
                ? const Color(0x47FFFFFF)
                : Colors.transparent,
            shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(7.2)),
            shadows: widget.selected
                ? const [BoxShadow(color: Color(0x180E183A), offset: Offset(0, 1), blurRadius: 3)]
                : null,
          ),
          child: Text(
            widget.label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: TextStyle(
              fontSize: 12,
              color: widget.selected ? s.ink : s.inkSecondary,
              fontWeight: widget.selected ? FontWeight.w600 : FontWeight.w500,
            ),
          ),
        ),
      ),
    );
  }
}

/// 中间高亮内嵌主面：严格同心超椭圆圆角（外窗 26 - 缩进 6 = 20），取代生硬的 VerticalDivider。
class GlassSurface extends StatelessWidget {
  const GlassSurface({super.key, required this.margin, required this.child});

  final EdgeInsetsGeometry margin;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final br = BorderRadius.circular(FrostedSheet.innerRadius);
    return Padding(
      padding: margin,
      child: DecoratedBox(
        decoration: ShapeDecoration(
          gradient: const LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: [Color(0xE8FFFFFF), Color(0xD9FCFDFF)],
          ),
          shape: RoundedSuperellipseBorder(
            borderRadius: br,
            side: const BorderSide(color: Color(0xEBFFFFFF), width: 0.7),
          ),
          shadows: const [BoxShadow(color: Color(0x0E0E183A), offset: Offset(0, 2), blurRadius: 14)],
        ),
        child: ClipRSuperellipse(
          borderRadius: br,
          child: Material(type: MaterialType.transparency, child: child),
        ),
      ),
    );
  }
}

/// 右栏 Inspector 通用玻璃卡片（14px 超椭圆 + 半透白底 + 镜面细边）。
class GlassInspectorCard extends StatelessWidget {
  const GlassInspectorCard({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
    this.margin = const EdgeInsets.only(bottom: 8),
  });

  final Widget child;
  final EdgeInsetsGeometry padding;
  final EdgeInsetsGeometry margin;

  @override
  Widget build(BuildContext context) {
    final br = BorderRadius.circular(14);
    return Padding(
      padding: margin,
      child: DecoratedBox(
        decoration: ShapeDecoration(
          color: const Color(0x94FFFFFF),
          shape: RoundedSuperellipseBorder(
            borderRadius: br,
            side: const BorderSide(color: Color(0xADFFFFFF), width: 0.6),
          ),
          shadows: const [BoxShadow(color: Color(0x0A0E183A), offset: Offset(0, 1), blurRadius: 4)],
        ),
        child: ClipRSuperellipse(
          borderRadius: br,
          child: Padding(padding: padding, child: child),
        ),
      ),
    );
  }
}

/// 工作台三栏窗口（点右下角“面板”进入）：与员工窗共享同一套 macOS Liquid Glass 几何与顶栏基线。
class ShellPanel extends StatelessWidget {
  const ShellPanel({
    super.key,
    required this.map,
    required this.section,
    required this.agentId,
    required this.detailOpen,
    required this.onSection,
    required this.onSelectAgent,
    required this.onOpenThread,
    required this.onNewChat,
    required this.onChatWith,
    required this.onToggleDetail,
    required this.onClose,
    required this.footer,
    this.expanded = false,
    this.onToggleExpand,
  });

  final StarMap map;
  final PanelSection section;

  /// 右栏看的是哪位员工；为空是幕僚长（或空位）。
  final String? agentId;
  final bool detailOpen;
  final void Function(PanelSection) onSection;
  final void Function(String agentId) onSelectAgent;
  final void Function(String threadId) onOpenThread;
  final VoidCallback onNewChat;
  final void Function(String agentId) onChatWith;
  final VoidCallback onToggleDetail;
  final VoidCallback onClose;

  /// 左栏最底下：“我”和连接状态。
  final Widget footer;
  final bool expanded;
  final VoidCallback? onToggleExpand;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final middle = switch (section) {
      PanelSection.inbox => InboxPage(onOpenThread: onOpenThread),
      PanelSection.chat => _ChatSection(onOpenThread: onOpenThread, onNewChat: onNewChat),
      PanelSection.tasks => TasksPage(onOpenThread: onOpenThread),
      PanelSection.roles => RolesPage(onChat: onChatWith),
    };
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          width: 240,
          child: _Sidebar(
            map: map,
            section: section,
            agentId: agentId,
            expanded: expanded,
            onClose: onClose,
            onToggleExpand: onToggleExpand,
            onSection: onSection,
            onSelectAgent: onSelectAgent,
            footer: footer,
          ),
        ),
        Expanded(
          child: GlassSurface(
            margin: EdgeInsets.fromLTRB(
              0,
              FrostedSheet.innerInset,
              detailOpen ? 0 : FrostedSheet.innerInset,
              FrostedSheet.innerInset,
            ),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SheetDragRegion(
                  child: _TopBar(title: section.label, detailOpen: detailOpen, onToggleDetail: onToggleDetail),
                ),
                Container(height: 0.5, color: s.edge),
                Expanded(child: middle),
              ],
            ),
          ),
        ),
        if (detailOpen)
          SizedBox(
            width: 304,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                const SheetDragRegion(child: SizedBox(height: 12)),
                Expanded(
                  child: AgentSheetContent(
                    key: ValueKey('detail-$agentId'),
                    map: map,
                    agentId: agentId,
                    onSelectAgent: onSelectAgent,
                    onOpenThread: onOpenThread,
                  ),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

/// 中间内嵌面顶栏（高度 42px，与左栏红绿灯严格对齐同一水平中线）。
class _TopBar extends StatelessWidget {
  const _TopBar({required this.title, required this.detailOpen, required this.onToggleDetail});

  final String title;
  final bool detailOpen;
  final VoidCallback onToggleDetail;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return SizedBox(
      height: 42,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 0, 10, 0),
        child: Row(
          children: [
            Expanded(
              child: Text(
                title,
                style: TextStyle(fontSize: 14.5, fontWeight: FontWeight.w600, color: s.ink, letterSpacing: -0.1),
              ),
            ),
            GlassIconButton(
              icon: detailOpen ? Icons.view_sidebar : Icons.view_sidebar_outlined,
              tooltip: detailOpen ? '收起员工详情' : '展开员工详情',
              active: detailOpen,
              onPressed: onToggleDetail,
            ),
          ],
        ),
      ),
    );
  }
}

/// 左栏：顶上原生红绿灯，下面四个入口与员工名单，最底下是“我”和连接状态。
class _Sidebar extends StatelessWidget {
  const _Sidebar({
    required this.map,
    required this.section,
    required this.agentId,
    required this.expanded,
    required this.onClose,
    required this.onToggleExpand,
    required this.onSection,
    required this.onSelectAgent,
    required this.footer,
  });

  final StarMap map;
  final PanelSection section;
  final String? agentId;
  final bool expanded;
  final VoidCallback onClose;
  final VoidCallback? onToggleExpand;
  final void Function(PanelSection) onSection;
  final void Function(String agentId) onSelectAgent;
  final Widget footer;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final selectedAgent = agentId ?? map.chief?.agent.id;

    Widget row({
      required Widget leading,
      required String label,
      required bool selected,
      required VoidCallback onTap,
      Widget? trailing,
      Key? key,
    }) => Padding(
      key: key,
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 1.5),
      child: DecoratedBox(
        decoration: ShapeDecoration(
          color: selected ? s.raised : Colors.transparent,
          shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(11)),
          shadows: selected ? const [BoxShadow(color: Color(0x120E183A), offset: Offset(0, 1), blurRadius: 4)] : null,
        ),
        child: ClipRSuperellipse(
          borderRadius: BorderRadius.circular(11),
          child: InkWell(
            hoverColor: s.raisedHover,
            onTap: onTap,
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7.5),
              child: Row(
                children: [
                  leading,
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      label,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(
                        fontSize: 13,
                        color: s.ink,
                        fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
                      ),
                    ),
                  ),
                  ?trailing,
                ],
              ),
            ),
          ),
        ),
      ),
    );

    Widget heading(String text) => Padding(
      padding: const EdgeInsets.fromLTRB(20, 14, 16, 4),
      child: Text(
        text,
        style: TextStyle(fontSize: 11, color: s.inkTertiary, fontWeight: FontWeight.w600, letterSpacing: 0.3),
      ),
    );

    Widget agentRow(BotBody b, {required bool chief}) {
      final color = chief ? panelGold : roleColorFor(state.allRoles, b.agent.role);
      final note = b.working
          ? Text('在做', style: TextStyle(fontSize: 11, color: s.inkTertiary))
          : b.urgency >= 0.6 && !chief
          ? Container(
              width: 7,
              height: 7,
              decoration: BoxDecoration(color: urgencyColorFor(b.urgency), shape: BoxShape.circle),
            )
          : null;
      return row(
        key: ValueKey('panel-agent-${b.agent.id}'),
        leading: Container(
          width: 8.5,
          height: 8.5,
          decoration: BoxDecoration(color: color, shape: BoxShape.circle),
        ),
        label: b.agent.name,
        selected: section == PanelSection.chat && selectedAgent == b.agent.id,
        onTap: () => onSelectAgent(b.agent.id),
        trailing: note,
      );
    }

    // 员工按岗位分组，岗位按名单里的顺序。
    final groups = <String, List<BotBody>>{};
    for (final b in map.bodies) {
      groups.putIfAbsent(b.agent.role, () => []).add(b);
    }
    final roleOrder = groups.keys.toList()
      ..sort((a, b) {
        int at(String id) {
          final i = state.allRoles.indexWhere((r) => r.id == id);
          return i < 0 ? 1 << 20 : i;
        }

        return at(a).compareTo(at(b));
      });

    final badge = state.inboxCount;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SheetDragRegion(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(14, FrostedSheet.innerInset, 10, 0),
            child: SizedBox(
              height: 42,
              child: Row(
                children: [MacTrafficLights(onClose: onClose, onToggleExpand: onToggleExpand, expanded: expanded)],
              ),
            ),
          ),
        ),
        Expanded(
          child: ListView(
            padding: const EdgeInsets.only(top: 4, bottom: 8),
            children: [
              for (final p in PanelSection.values)
                row(
                  key: ValueKey('panel-section-${p.name}'),
                  leading: Icon(p.icon, size: 17, color: section == p ? s.ink : s.inkSecondary),
                  label: p.label,
                  selected: section == p,
                  onTap: () => onSection(p),
                  trailing: p == PanelSection.inbox && badge > 0
                      ? Container(
                          padding: const EdgeInsets.symmetric(horizontal: 6.5, vertical: 1.5),
                          decoration: BoxDecoration(color: s.danger, borderRadius: BorderRadius.circular(999)),
                          child: Text(
                            '$badge',
                            style: const TextStyle(fontSize: 10.5, fontWeight: FontWeight.w600, color: Colors.white),
                          ),
                        )
                      : null,
                ),
              heading('幕僚长'),
              if (map.chief case final c?)
                agentRow(c, chief: true)
              else
                Padding(
                  padding: const EdgeInsets.fromLTRB(20, 2, 16, 2),
                  child: Text('空位', style: TextStyle(fontSize: 12.5, color: s.inkTertiary)),
                ),
              for (final role in roleOrder) ...[
                heading(roleNameFor(state.allRoles, role)),
                for (final b in groups[role]!) agentRow(b, chief: false),
              ],
            ],
          ),
        ),
        Padding(padding: const EdgeInsets.fromLTRB(12, 6, 10, 12), child: footer),
      ],
    );
  }
}

/// 对话页：左边对话列表，右边对话（能直接发消息）。
class _ChatSection extends StatelessWidget {
  const _ChatSection({required this.onOpenThread, required this.onNewChat});

  final void Function(String threadId) onOpenThread;
  final VoidCallback onNewChat;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final state = AppScope.of(context);
    final id = state.threadId;
    final agent = state.currentAgent;
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          width: 236,
          child: ThreadList(onOpen: onOpenThread, onNew: onNewChat),
        ),
        Container(width: 0.5, color: s.edge),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(16, 9, 16, 9),
                child: Text(
                  id == null ? '新对话' : (agent == null ? '对话' : '和 ${state.agentName(agent)} 的对话'),
                  style: TextStyle(fontSize: 12.5, color: s.inkSecondary, fontWeight: FontWeight.w600),
                ),
              ),
              Container(height: 0.5, color: s.edge),
              const Expanded(child: ThreadView()),
            ],
          ),
        ),
      ],
    );
  }
}

/// 右栏：员工详情（Apple Inspector 分组卡片样式）。[agentId] 为空是幕僚长（或空位）。
class AgentSheetContent extends StatelessWidget {
  const AgentSheetContent({
    super.key,
    required this.map,
    required this.agentId,
    required this.onSelectAgent,
    required this.onOpenThread,
    this.showHeader = true,
    this.hideApprovalsInThread,
    this.footer,
  });

  final StarMap map;
  final String? agentId;
  final void Function(String agentId) onSelectAgent;
  final void Function(String threadId) onOpenThread;

  /// 顶上的名字一行。员工界面的中间顶栏已经给出名字与角色，右栏不再重复。
  final bool showHeader;

  /// 跳过已经嵌在当前对话里的审批卡片，避免左/右两栏同时画两份一样的卡片。
  final String? hideApprovalsInThread;

  /// 放在最底下的一块（员工界面里是定时任务、模型和今天的花费）。
  final Widget? footer;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final b = agentId == null ? map.chief : map.byId(agentId!);
    final celestial = b?.celestial ?? sun;
    final roleColor = b == null || b == map.chief ? panelGold : roleColorFor(state.allRoles, b.agent.role);
    final sectionTitle = TextStyle(fontSize: 11, fontWeight: FontWeight.w600, color: s.inkTertiary, letterSpacing: 0.3);
    final cardTitle = TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: s.ink);

    final header = Row(
      children: [
        Container(
          width: 24,
          height: 24,
          decoration: const BoxDecoration(
            shape: BoxShape.circle,
            gradient: RadialGradient(
              colors: [Color(0xFFFFF6D2), Color(0xFFEAB44C), Color(0xFFBD6B28)],
              stops: [0, 0.58, 1],
            ),
          ),
        ),
        const SizedBox(width: 9),
        Expanded(
          child: Text(
            b != null ? b.agent.name : (state.connected ? '幕僚长（空位）' : '幕僚长'),
            overflow: TextOverflow.ellipsis,
            style: TextStyle(fontSize: 15, fontWeight: FontWeight.w600, color: s.ink),
          ),
        ),
      ],
    );

    final chips = Wrap(
      spacing: 6,
      runSpacing: 6,
      children: [
        if (b != null) panelChip(b == map.chief ? '幕僚长' : roleNameFor(state.allRoles, b.agent.role), roleColor),
        panelChip('${celestial.name} ${celestial.english}', panelGold),
        if (b != null && b != map.chief) ...[
          panelChip(
            '紧急度 ${(b.urgency * 100).round()}% · ${urgencyBands.firstWhere((x) => b.urgency >= x.at).label}',
            urgencyColorFor(b.urgency),
          ),
          panelChip('重要度 ${(b.importance * 100).round()}%', const Color(0xFF5A5243)),
        ],
      ],
    );

    final neighborIds = b == null ? const <String>[] : map.neighbors(b.agent.id).toList();
    final approvals = b == null
        ? const <Approval>[]
        : state.pendingApprovals
              .where(
                (a) =>
                    a.isAgentQuestion &&
                    a.subjectAgentId == b.agent.id &&
                    (hideApprovalsInThread == null || a.subjectThreadId != hideApprovalsInThread),
              )
              .toList();

    return ListView(
      padding: const EdgeInsets.fromLTRB(10, 4, 12, 12),
      children: [
        GlassInspectorCard(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              if (showHeader) ...[header, const SizedBox(height: 10)],
              chips,
            ],
          ),
        ),
        if (b != null)
          GlassInspectorCard(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('[[关联]]', style: sectionTitle),
                const SizedBox(height: 7),
                if (neighborIds.isEmpty)
                  Text('暂无', style: TextStyle(fontSize: 12, color: s.inkTertiary))
                else
                  Wrap(
                    spacing: 6,
                    runSpacing: 6,
                    children: [for (final nid in neighborIds) _linkChip(context, state, nid)],
                  ),
              ],
            ),
          ),
        if (b == null && !state.connected)
          GlassInspectorCard(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('当前未连接主机。', style: cardTitle),
                const SizedBox(height: 6),
                Text(
                  '连上主机后会在这里显示幕僚长的状态、关联员工和待处理审批。',
                  style: TextStyle(fontSize: 12, color: s.inkSecondary, height: 1.5),
                ),
              ],
            ),
          )
        else if (b == null)
          GlassInspectorCard(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text('太阳的位置留给幕僚长。', style: cardTitle),
                const SizedBox(height: 6),
                Text(
                  '主机上没有协调员岗位。在 ip.yaml 里加一个 id 含 coordinator 的岗位（或岗位名里有“协调”“幕僚”），'
                  '它的员工就会坐到这里，其他员工绕着它转。',
                  style: TextStyle(fontSize: 12, color: s.inkSecondary, height: 1.5),
                ),
                if (map.routingWaiting > 0) ...[
                  const SizedBox(height: 8),
                  Text('有 ${map.routingWaiting} 条消息等你分派，在“待办”里。', style: TextStyle(fontSize: 12, color: s.warning)),
                ],
              ],
            ),
          )
        else ...[
          GlassInspectorCard(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(b == map.chief ? '幕僚长在中心，统筹派活。' : '为什么在这条轨道上', style: cardTitle),
                const SizedBox(height: 6),
                for (final r in b.reasons) _reasonRow(context, r),
                if (b == map.chief && map.routingWaiting > 0)
                  _reasonRow(context, Reason('有 ${map.routingWaiting} 条消息等你分派（在“待办”里）', 0.8)),
                const SizedBox(height: 10),
                Container(height: 0.5, color: s.edge),
                const SizedBox(height: 9),
                Text('为什么这么大', style: sectionTitle),
                const SizedBox(height: 5),
                for (final n in b.importanceNotes)
                  Padding(
                    padding: const EdgeInsets.only(bottom: 3),
                    child: Text('· $n', style: TextStyle(fontSize: 12, color: s.inkSecondary, height: 1.45)),
                  ),
              ],
            ),
          ),
          for (final a in approvals)
            Padding(
              padding: const EdgeInsets.only(bottom: 8),
              child: ApprovalCard(approval: a, onOpenThread: onOpenThread),
            ),
        ],
        ?footer,
      ],
    );
  }

  Widget _reasonRow(BuildContext context, Reason r) {
    final s = Shell.of(context);
    final c = urgencyColorFor(r.weight);
    final thread = r.threadId;
    return InkWell(
      borderRadius: BorderRadius.circular(8),
      onTap: thread == null ? null : () => onOpenThread(thread),
      child: Padding(
        padding: const EdgeInsets.symmetric(vertical: 3.5, horizontal: 2),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Container(
              margin: const EdgeInsets.only(top: 5, right: 8),
              width: 6.5,
              height: 6.5,
              decoration: BoxDecoration(color: c, shape: BoxShape.circle),
            ),
            Expanded(
              child: Text(r.text, style: TextStyle(fontSize: 12, color: s.ink, height: 1.4)),
            ),
            if (thread != null) Icon(Icons.chevron_right_rounded, size: 16, color: s.inkTertiary),
          ],
        ),
      ),
    );
  }

  Widget _linkChip(BuildContext context, AppState state, String id) {
    final s = Shell.of(context);
    final b = map.byId(id);
    if (b == null) return const SizedBox.shrink();
    final color = b == map.chief ? panelGold : roleColorFor(state.allRoles, b.agent.role);
    return Material(
      color: const Color(0xB8FFFFFF),
      shape: const StadiumBorder(),
      child: InkWell(
        customBorder: const StadiumBorder(),
        onTap: () => onSelectAgent(id),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 4),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              Container(
                width: 6,
                height: 6,
                decoration: BoxDecoration(color: color, shape: BoxShape.circle),
              ),
              const SizedBox(width: 5),
              Text(
                '[[${b.agent.name}]]',
                style: TextStyle(fontSize: 11, fontWeight: FontWeight.w500, color: s.ink),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 暖琥珀金（太阳与幕僚长专属点缀色）。
const panelGold = Color(0xFFB37414);

/// 岗位的颜色：在清透白玻璃上饱和、克制且易辨识的苹果语义色系。
const _rolePalette = [
  Color(0xFF007AFF),
  Color(0xFFAF52DE),
  Color(0xFF1A9E38),
  Color(0xFFD97706),
  Color(0xFF0EA5E9),
  Color(0xFFE11D48),
  Color(0xFF4F46E5),
  Color(0xFF0D9488),
];

Color roleColorFor(List<Role> roles, String role) {
  final i = roles.indexWhere((r) => r.id == role);
  return _rolePalette[(i < 0 ? role.hashCode : i) % _rolePalette.length];
}

String roleNameFor(List<Role> roles, String id) {
  for (final r in roles) {
    if (r.id == id) return r.name ?? r.id;
  }
  return id;
}

/// 轻盈无框线苹果语义小胶囊（去除了生硬描边）。
Widget panelChip(String text, Color color) => Container(
  padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
  decoration: BoxDecoration(color: color.withValues(alpha: 0.12), borderRadius: BorderRadius.circular(999)),
  child: Text(
    text,
    style: TextStyle(fontSize: 11, color: color, fontWeight: FontWeight.w600),
  ),
);

/// 紧急度对应的苹果语义色（在清透白玻璃上高对比易读）。
Color urgencyColorFor(double u) => u >= 0.8
    ? const Color(0xFFE0312B)
    : u >= 0.6
    ? const Color(0xFFD97706)
    : u >= 0.3
    ? const Color(0xFF007AFF)
    : const Color(0xFF64748B);
