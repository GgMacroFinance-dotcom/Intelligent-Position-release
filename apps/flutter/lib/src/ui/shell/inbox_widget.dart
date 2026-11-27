// 左上角待办小组件（设计重构，2026-11-25 维护者定）：照 Apple Liquid Glass + macOS 红绿灯设计。
//   - 无多余图标装饰，顶部三颗红 / 黄 / 绿灯直接映射紧急度与状态：
//       红灯（#FF5F57）：紧急（员工提问、受阻任务、已过期任务）；
//       黄灯（#FEBC2E）：待定（路由决策卡、24 小时内到期任务）；
//       绿灯（#28C840）：动态（近 7 天完成任务与新报告）；没有任何红/黄项时只亮绿灯，收成安静的玻璃胶囊。
//   - 连续超椭圆圆角（ClipRSuperellipse + RoundedSuperellipseBorder），严格同心：
//       常驻小组件外圆角 26；展开态外圆角 28、内边距 12、内卡片圆角 16（28 - 12 = 16）。
//   - 顶部受光微渐变通透玻璃 + 0.5px 镜面高光边 + 柔和偏蓝环境投影；列表分隔线左缩进对齐文字起点。
//   - 点红/黄/绿灯或卡片原地展开成完整待办浮层，可直接拍板审批、改任务状态、看报告，
//     点“去对话”自动收起待办并滑出对应员工的三栏面板。

import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import '../../app_state.dart';
import '../../protocol/client.dart' show RequestError;
import '../../protocol/models.dart';
import '../common.dart';
import '../report.dart' show openReport;
import '../task_card.dart' show taskMeta, taskStatusText;
import 'completion_approval.dart';
import 'tokens.dart';

/// 红 / 黄 / 绿三档紧急度分组。
enum InboxLight {
  red('紧急', Color(0xFFFF5F57), Color(0x47B41912)),
  yellow('待定', Color(0xFFFEBC2E), Color(0x47B47300)),
  green('动态', Color(0xFF28C840), Color(0x47108224));

  const InboxLight(this.label, this.color, this.border);
  final String label;
  final Color color;
  final Color border;
}

/// 按红 / 黄 / 绿三档归类的待办数据（纯计算，便于测试）。
class InboxBuckets {
  const InboxBuckets({
    required this.agentQuestions,
    required this.blockedTasks,
    required this.overdueTasks,
    this.completionApprovals = const [],
    required this.routingApprovals,
    required this.dueSoonTasks,
    required this.recentDoneTasks,
    required this.recentReports,
  });

  factory InboxBuckets.from(AppState state, {int? nowMs}) {
    final now = nowMs ?? DateTime.now().millisecondsSinceEpoch;
    final dayMs = const Duration(hours: 24).inMilliseconds;
    final weekAgo = now - const Duration(days: 7).inMilliseconds;

    // 4 数据同源 + 5A 语义隔离：
    // - `agent_question`（员工中途提问）仍归入红灯紧急；
    // - `completionApprovals`（任务已完成 · 待审批）与左中下停泊区共用同一份 AppState 数据源，归入黄灯待定验收；
    // - `routingApprovals` 排除已归入 completionApprovals 的 `task_completion` 避免重复计数。
    final completion = state.pendingCompletionApprovals;
    final questions = state.pendingApprovals.where((a) => a.isAgentQuestion).toList();
    final routing = state.pendingApprovals.where((a) => !a.isAgentQuestion && !a.isTaskCompletion).toList();
    final blocked = state.blockedTasks;
    final overdue = state.tasks
        .where((t) => t.isOpen && t.status != 'blocked' && !t.isPendingApproval && t.dueMs != null && t.dueMs! < now)
        .toList();
    final dueSoon = state.tasks
        .where(
          (t) =>
              t.isOpen &&
              t.status != 'blocked' &&
              !t.isPendingApproval &&
              t.dueMs != null &&
              t.dueMs! >= now &&
              t.dueMs! - now <= dayMs,
        )
        .toList();
    final done = state.tasks.where((t) => t.status == 'done' && t.updatedAtMs >= weekAgo).toList()
      ..sort((a, b) => b.updatedAtMs.compareTo(a.updatedAtMs));
    final reports = state.reports.where((r) => r.modifiedMs >= weekAgo).toList()
      ..sort((a, b) => b.modifiedMs.compareTo(a.modifiedMs));

    return InboxBuckets(
      agentQuestions: questions,
      blockedTasks: blocked,
      overdueTasks: overdue,
      completionApprovals: completion,
      routingApprovals: routing,
      dueSoonTasks: dueSoon,
      recentDoneTasks: done.take(10).toList(),
      recentReports: reports.take(10).toList(),
    );
  }

  final List<Approval> agentQuestions;
  final List<Task> blockedTasks;
  final List<Task> overdueTasks;

  /// 任务已完成待审批列表（与页面左中下待审批停泊区同源）。
  final List<CompletionApproval> completionApprovals;
  final List<Approval> routingApprovals;
  final List<Task> dueSoonTasks;
  final List<Task> recentDoneTasks;
  final List<ReportInfo> recentReports;

  int get redCount => agentQuestions.length + blockedTasks.length + overdueTasks.length;
  int get yellowCount => completionApprovals.length + routingApprovals.length + dueSoonTasks.length;
  int get greenCount => recentDoneTasks.length + recentReports.length;
  int get actionCount => redCount + yellowCount;

  /// 默认打开哪一档：有红开红，有黄开黄，否则开绿。
  InboxLight get defaultTab => redCount > 0
      ? InboxLight.red
      : yellowCount > 0
      ? InboxLight.yellow
      : InboxLight.green;
}

/// 提高背景饱和度 1.7 倍（Liquid Glass 透色鲜活感）。
const _liquidSaturate = <double>[
  1.551, -0.501, -0.050, 0, 0, //
  -0.149, 1.199, -0.050, 0, 0,
  -0.149, -0.501, 1.650, 0, 0,
  0, 0, 0, 1, 0,
];

const _ink = Color(0xFF1A1715);
const _ink2 = Color(0x8A1A1715);
const _ink3 = Color(0x6E1A1715);
const _hairline = Color(0x181A1715);

/// 左上角 Apple 风格待办小组件：平时常驻（无待办时收成绿灯胶囊），点击原地展开。
class TopLeftInboxWidget extends StatefulWidget {
  const TopLeftInboxWidget({
    super.key,
    required this.expanded,
    required this.maxHeight,
    required this.onExpandedChanged,
    required this.onOpenThread,
    this.onOpenAgent,
  });

  final bool expanded;
  final double maxHeight;
  final ValueChanged<bool> onExpandedChanged;

  /// 点“去对话 / 执行对话 / 派活对话”时回调，带上可选的员工 id 以便直接滑出该员工的三栏面板。
  final void Function(String threadId, {String? agentId}) onOpenThread;

  /// 点“完成待审批”卡片里的“打开工作窗”时回调，直接滑出对应员工的三栏面板（3B）。
  final ValueChanged<String>? onOpenAgent;

  @override
  State<TopLeftInboxWidget> createState() => _TopLeftInboxWidgetState();
}

class _TopLeftInboxWidgetState extends State<TopLeftInboxWidget> {
  InboxLight _tab = InboxLight.red;

  void _openOn(InboxLight tab) {
    setState(() => _tab = tab);
    if (!widget.expanded) widget.onExpandedChanged(true);
  }

  void _close() {
    if (widget.expanded) widget.onExpandedChanged(false);
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final buckets = InboxBuckets.from(state);

    if (widget.expanded) {
      return _ExpandedInboxCard(
        buckets: buckets,
        tab: _tab,
        maxHeight: widget.maxHeight,
        onTabChanged: (t) => setState(() => _tab = t),
        onClose: _close,
        onOpenThread: (threadId, {agentId}) {
          _close();
          widget.onOpenThread(threadId, agentId: agentId);
        },
        onOpenAgent: widget.onOpenAgent == null
            ? null
            : (agentId) {
                _close();
                widget.onOpenAgent!(agentId);
              },
      );
    }

    if (buckets.actionCount == 0) {
      return _AllClearPill(
        buckets: buckets,
        connected: state.connected,
        onTap: () => _openOn(InboxLight.green),
        onTapLight: _openOn,
      );
    }

    return _CompactInboxWidget(buckets: buckets, onExpand: () => _openOn(buckets.defaultTab), onTapLight: _openOn);
  }
}

/// Liquid Glass 玻璃容器：外层柔和环境阴影 + 真实超椭圆裁剪 + 模糊增饱和 + 顶部受光微渐变 + 镜面细高光。
class _LiquidGlass extends StatelessWidget {
  const _LiquidGlass({
    super.key,
    required this.radius,
    required this.child,
    this.expanded = false,
    this.padding = EdgeInsets.zero,
    this.width,
    this.maxHeight,
  });

  final double radius;
  final Widget child;
  final bool expanded;
  final EdgeInsetsGeometry padding;
  final double? width;
  final double? maxHeight;

  @override
  Widget build(BuildContext context) {
    final br = BorderRadius.circular(radius);
    final shape = RoundedSuperellipseBorder(
      borderRadius: br,
      side: BorderSide(color: expanded ? const Color(0x99FFFFFF) : const Color(0x80FFFFFF), width: 0.7),
    );
    final gradient = LinearGradient(
      begin: Alignment.topLeft,
      end: Alignment.bottomRight,
      colors: expanded
          ? const [Color(0xD2FFFFFF), Color(0xB8FCFAF7), Color(0xC2F7F4EE)]
          : const [Color(0xB5FFFFFF), Color(0x8AFCFAF7), Color(0x96F7F4EE)],
      stops: const [0.0, 0.58, 1.0],
    );
    Widget box = DecoratedBox(
      decoration: ShapeDecoration(
        shape: RoundedSuperellipseBorder(borderRadius: br),
        shadows: const [
          BoxShadow(color: Color(0x400E183A), offset: Offset(0, 16), blurRadius: 38, spreadRadius: -12),
          BoxShadow(color: Color(0x140E183A), offset: Offset(0, 2), blurRadius: 8),
        ],
      ),
      child: ClipRSuperellipse(
        borderRadius: br,
        child: BackdropFilter(
          filter: ui.ImageFilter.compose(
            outer: const ui.ColorFilter.matrix(_liquidSaturate),
            inner: ui.ImageFilter.blur(sigmaX: 34, sigmaY: 34),
          ),
          child: Container(
            width: width,
            constraints: maxHeight == null ? null : BoxConstraints(maxHeight: maxHeight!),
            decoration: ShapeDecoration(gradient: gradient, shape: shape),
            child: Material(
              type: MaterialType.transparency,
              child: Padding(padding: padding, child: child),
            ),
          ),
        ),
      ),
    );
    return Theme(data: _glassTheme, child: box);
  }
}

final _glassTheme = glassTheme();

/// macOS 标志性的红 / 黄 / 绿三灯，每颗灯实时反映对应紧急度是否有事项，点击可直达该分组。
class _TrafficLights extends StatelessWidget {
  const _TrafficLights({
    super.key,
    required this.redOn,
    required this.yellowOn,
    required this.greenOn,
    required this.onTapLight,
  });

  final bool redOn;
  final bool yellowOn;
  final bool greenOn;
  final ValueChanged<InboxLight> onTapLight;

  @override
  Widget build(BuildContext context) {
    Widget dot(InboxLight light, bool on) => Tooltip(
      message: light.label,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: () => onTapLight(light),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 3.5, vertical: 2),
          child: AnimatedContainer(
            duration: Motion.dialog,
            curve: Motion.curve,
            width: 10.5,
            height: 10.5,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: on ? light.color : const Color(0x1C1A1715),
              border: Border.all(color: on ? light.border : const Color(0x1E1A1715), width: 0.5),
              boxShadow: on ? [BoxShadow(color: light.color.withValues(alpha: 0.42), blurRadius: 7)] : null,
            ),
          ),
        ),
      ),
    );

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [dot(InboxLight.red, redOn), dot(InboxLight.yellow, yellowOn), dot(InboxLight.green, greenOn)],
    );
  }
}

/// 列表行首的状态点（8px 实心圆点 + 极轻柔的同色微晕）。
class _StatusDot extends StatelessWidget {
  const _StatusDot(this.light, {this.size = 8});

  final InboxLight light;
  final double size;

  @override
  Widget build(BuildContext context) => Container(
    width: size,
    height: size,
    decoration: BoxDecoration(
      shape: BoxShape.circle,
      color: light.color,
      boxShadow: [BoxShadow(color: light.color.withValues(alpha: 0.22), spreadRadius: 2.2)],
    ),
  );
}

/// 统一顶部栏：无论收起（无待办胶囊 / 有待办小组件）还是展开，红绿灯和“待办”标题的坐标、字号、字重完全钉死，0 像素跳动。
class _InboxHeader extends StatelessWidget {
  const _InboxHeader({
    required this.redOn,
    required this.yellowOn,
    required this.greenOn,
    required this.onTapLight,
    required this.trailing,
    this.expandTitle = true,
  });

  final bool redOn;
  final bool yellowOn;
  final bool greenOn;
  final ValueChanged<InboxLight> onTapLight;
  final Widget trailing;
  final bool expandTitle;

  @override
  Widget build(BuildContext context) {
    const title = Text(
      '待办',
      key: ValueKey('inbox-header-title'),
      style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600, letterSpacing: -0.2, color: _ink),
    );
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4),
      child: SizedBox(
        height: 24,
        child: Row(
          mainAxisSize: expandTitle ? MainAxisSize.max : MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            _TrafficLights(
              key: const ValueKey('inbox-header-lights'),
              redOn: redOn,
              yellowOn: yellowOn,
              greenOn: greenOn,
              onTapLight: onTapLight,
            ),
            const SizedBox(width: 8),
            if (expandTitle) const Expanded(child: title) else title,
            if (!expandTitle) const SizedBox(width: 12),
            trailing,
          ],
        ),
      ),
    );
  }
}

/// 无待办时的极简形态：红黄熄灭、只亮绿灯，收成安静的玻璃胶囊（高 40、圆角 20，和展开态共用同一套圆角与顶栏锚点）。
class _AllClearPill extends StatelessWidget {
  const _AllClearPill({required this.buckets, required this.connected, required this.onTap, required this.onTapLight});

  final InboxBuckets buckets;
  final bool connected;
  final VoidCallback onTap;
  final ValueChanged<InboxLight> onTapLight;

  @override
  Widget build(BuildContext context) {
    final note = !connected
        ? '未连接'
        : buckets.greenCount > 0
        ? '${buckets.greenCount} 条动态'
        : '暂无';
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        key: const ValueKey('inbox-widget-pill'),
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: _LiquidGlass(
          radius: 20,
          padding: const EdgeInsets.fromLTRB(10, 8, 12, 8),
          child: _InboxHeader(
            redOn: false,
            yellowOn: false,
            greenOn: connected,
            onTapLight: onTapLight,
            expandTitle: false,
            trailing: Text(
              note,
              style: const TextStyle(
                fontSize: 12,
                fontWeight: FontWeight.w500,
                color: _ink3,
                fontFeatures: [ui.FontFeature.tabularFigures()],
              ),
            ),
          ),
        ),
      ),
    );
  }
}

typedef _PreviewItem = ({InboxLight light, String title, String meta});

/// 常驻小组件：统一顶栏（红绿灯 + 待办 + 等宽数字）+ 最急的 3 条（Apple 缩进细线）。
class _CompactInboxWidget extends StatelessWidget {
  const _CompactInboxWidget({required this.buckets, required this.onExpand, required this.onTapLight});

  final InboxBuckets buckets;
  final VoidCallback onExpand;
  final ValueChanged<InboxLight> onTapLight;

  List<_PreviewItem> _topItems(AppState state) {
    final out = <_PreviewItem>[];
    for (final a in buckets.agentQuestions) {
      final who = a.subjectAgentId != null ? '${state.agentName(a.subjectAgentId!)} 提问' : '员工提问';
      out.add((light: InboxLight.red, title: a.question, meta: '$who · ${formatTime(a.createdAtMs)}'));
    }
    for (final t in buckets.blockedTasks) {
      final who = t.assignee != null ? state.agentName(t.assignee!) : (t.role ?? '任务');
      out.add((light: InboxLight.red, title: t.title, meta: '$who · 任务受阻'));
    }
    for (final t in buckets.overdueTasks) {
      final who = t.assignee != null ? state.agentName(t.assignee!) : (t.role ?? '任务');
      out.add((light: InboxLight.red, title: t.title, meta: '$who · 已过期'));
    }
    for (final c in buckets.completionApprovals) {
      out.add((light: InboxLight.yellow, title: c.taskTitle, meta: '${c.agentName} · 完成 · 待审批'));
    }
    for (final a in buckets.routingApprovals) {
      out.add((light: InboxLight.yellow, title: a.question, meta: '等你决定 · ${formatTime(a.createdAtMs)}'));
    }
    for (final t in buckets.dueSoonTasks) {
      final who = t.assignee != null ? state.agentName(t.assignee!) : (t.role ?? '任务');
      out.add((light: InboxLight.yellow, title: t.title, meta: '$who · 24 小时内到期'));
    }
    return out.take(3).toList();
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final items = _topItems(state);
    final total = buckets.actionCount;
    final more = total - items.length;
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      child: GestureDetector(
        key: const ValueKey('inbox-widget-compact'),
        behavior: HitTestBehavior.opaque,
        onTap: onExpand,
        child: _LiquidGlass(
          width: 276,
          radius: 20,
          padding: const EdgeInsets.fromLTRB(10, 8, 10, 10),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              _InboxHeader(
                redOn: buckets.redCount > 0,
                yellowOn: buckets.yellowCount > 0,
                greenOn: buckets.greenCount > 0,
                onTapLight: onTapLight,
                trailing: Text(
                  '$total',
                  style: const TextStyle(
                    fontSize: 16,
                    fontWeight: FontWeight.w600,
                    height: 1.0,
                    letterSpacing: -0.4,
                    color: _ink,
                    fontFeatures: [ui.FontFeature.tabularFigures()],
                  ),
                ),
              ),
              const SizedBox(height: 4),
              Padding(
                padding: const EdgeInsets.symmetric(horizontal: 6),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    for (final (i, item) in items.indexed) ...[
                      if (i > 0)
                        const Padding(
                          padding: EdgeInsets.only(left: 19),
                          child: Divider(height: 1, thickness: 0.5, color: _hairline),
                        ),
                      GestureDetector(
                        behavior: HitTestBehavior.opaque,
                        onTap: () => onTapLight(item.light),
                        child: Padding(
                          padding: const EdgeInsets.symmetric(vertical: 6),
                          child: Row(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              Padding(padding: const EdgeInsets.only(top: 4.5), child: _StatusDot(item.light)),
                              const SizedBox(width: 11),
                              Expanded(
                                child: Column(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    Text(
                                      item.title,
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: const TextStyle(
                                        fontSize: 13,
                                        fontWeight: FontWeight.w500,
                                        height: 1.28,
                                        letterSpacing: -0.15,
                                        color: _ink,
                                      ),
                                    ),
                                    const SizedBox(height: 1),
                                    Text(
                                      item.meta,
                                      maxLines: 1,
                                      overflow: TextOverflow.ellipsis,
                                      style: const TextStyle(
                                        fontSize: 11,
                                        color: _ink2,
                                        fontFeatures: [ui.FontFeature.tabularFigures()],
                                      ),
                                    ),
                                  ],
                                ),
                              ),
                            ],
                          ),
                        ),
                      ),
                    ],
                    if (more > 0)
                      Padding(
                        padding: const EdgeInsets.only(top: 4, bottom: 2),
                        child: Text(
                          '还有 $more 项待处理 ›',
                          textAlign: TextAlign.right,
                          style: const TextStyle(fontSize: 11, fontWeight: FontWeight.w500, color: _ink3),
                        ),
                      ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 原地展开后的完整待办浮层：外圆角 28、内边距 12、内卡片严格同心圆角 16。
class _ExpandedInboxCard extends StatelessWidget {
  const _ExpandedInboxCard({
    required this.buckets,
    required this.tab,
    required this.maxHeight,
    required this.onTabChanged,
    required this.onClose,
    required this.onOpenThread,
    this.onOpenAgent,
  });

  final InboxBuckets buckets;
  final InboxLight tab;
  final double maxHeight;
  final ValueChanged<InboxLight> onTabChanged;
  final VoidCallback onClose;
  final void Function(String threadId, {String? agentId}) onOpenThread;
  final ValueChanged<String>? onOpenAgent;

  int _countFor(InboxLight l) => switch (l) {
    InboxLight.red => buckets.redCount,
    InboxLight.yellow => buckets.yellowCount,
    InboxLight.green => buckets.greenCount,
  };

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final items = switch (tab) {
      InboxLight.red => [
        for (final a in buckets.agentQuestions)
          _GlassApprovalCard(
            key: ValueKey('inbox-a-${a.id}'),
            light: InboxLight.red,
            approval: a,
            onOpenThread: onOpenThread,
          ),
        for (final t in buckets.blockedTasks)
          _GlassTaskCard(
            key: ValueKey('inbox-t-${t.id}'),
            light: InboxLight.red,
            badge: '受阻任务',
            task: t,
            onOpenThread: onOpenThread,
          ),
        for (final t in buckets.overdueTasks)
          _GlassTaskCard(
            key: ValueKey('inbox-t-${t.id}'),
            light: InboxLight.red,
            badge: '已过期',
            task: t,
            onOpenThread: onOpenThread,
          ),
      ],
      InboxLight.yellow => [
        for (final c in buckets.completionApprovals)
          _GlassCompletionApprovalCard(
            key: ValueKey('inbox-c-${c.id}'),
            item: c,
            onOpenAgent: onOpenAgent,
            onOpenThread: onOpenThread,
          ),
        for (final a in buckets.routingApprovals)
          _GlassApprovalCard(
            key: ValueKey('inbox-a-${a.id}'),
            light: InboxLight.yellow,
            approval: a,
            onOpenThread: onOpenThread,
          ),
        for (final t in buckets.dueSoonTasks)
          _GlassTaskCard(
            key: ValueKey('inbox-t-${t.id}'),
            light: InboxLight.yellow,
            badge: '24 小时内到期',
            task: t,
            onOpenThread: onOpenThread,
          ),
      ],
      InboxLight.green => [
        for (final t in buckets.recentDoneTasks)
          _GlassDoneTaskCard(key: ValueKey('inbox-d-${t.id}'), task: t, onOpenThread: onOpenThread),
        for (final r in buckets.recentReports)
          _GlassReportCard(key: ValueKey('inbox-r-${r.agentId}-${r.path}'), report: r),
      ],
    };

    final emptyText = !state.connected
        ? '连上主机后，等你处理的事会显示在这里。'
        : switch (tab) {
            InboxLight.red => '没有紧急待办。',
            InboxLight.yellow => '没有待定或即将到期的事项。',
            InboxLight.green => '最近 7 天还没有完成任务或新报告。',
          };

    return _LiquidGlass(
      key: const ValueKey('inbox-widget-expanded'),
      width: 356,
      maxHeight: maxHeight,
      radius: 20,
      expanded: true,
      padding: const EdgeInsets.fromLTRB(10, 8, 10, 10),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _InboxHeader(
            redOn: buckets.redCount > 0,
            yellowOn: buckets.yellowCount > 0,
            greenOn: buckets.greenCount > 0 || buckets.actionCount == 0,
            onTapLight: onTabChanged,
            trailing: Tooltip(
              message: '收起待办（Esc）',
              child: InkWell(
                key: const ValueKey('inbox-widget-close'),
                customBorder: const CircleBorder(),
                onTap: onClose,
                child: Container(
                  width: 22,
                  height: 22,
                  decoration: const BoxDecoration(shape: BoxShape.circle, color: Color(0x101A1715)),
                  child: const Icon(Icons.close, size: 13, color: _ink2),
                ),
              ),
            ),
          ),
          const SizedBox(height: 7),
          // Apple 分段筛选条：外圆角 12，内边距 2.5，选中项同心圆角 9.5。
          Container(
            margin: const EdgeInsets.only(bottom: 8),
            padding: const EdgeInsets.all(2.5),
            decoration: ShapeDecoration(
              color: const Color(0x0D1A1715),
              shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(12)),
            ),
            child: Row(
              children: [
                for (final l in InboxLight.values)
                  Expanded(
                    child: GestureDetector(
                      key: ValueKey('inbox-light-tab-${l.name}'),
                      behavior: HitTestBehavior.opaque,
                      onTap: () => onTabChanged(l),
                      child: AnimatedContainer(
                        duration: Motion.fast,
                        curve: Motion.curve,
                        padding: const EdgeInsets.symmetric(vertical: 4.5),
                        decoration: ShapeDecoration(
                          color: tab == l ? const Color(0xE0FFFFFF) : Colors.transparent,
                          shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(9.5)),
                          shadows: tab == l
                              ? const [BoxShadow(color: Color(0x141A1715), offset: Offset(0, 1), blurRadius: 4)]
                              : null,
                        ),
                        child: Row(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            Container(
                              width: 7,
                              height: 7,
                              decoration: BoxDecoration(shape: BoxShape.circle, color: l.color),
                            ),
                            const SizedBox(width: 5),
                            Text(
                              '${l.label} ${_countFor(l)}',
                              style: TextStyle(
                                fontSize: 12,
                                fontWeight: tab == l ? FontWeight.w600 : FontWeight.w500,
                                color: tab == l ? _ink : _ink2,
                                fontFeatures: const [ui.FontFeature.tabularFigures()],
                              ),
                            ),
                          ],
                        ),
                      ),
                    ),
                  ),
              ],
            ),
          ),
          Flexible(
            child: items.isEmpty
                ? Padding(
                    padding: const EdgeInsets.symmetric(vertical: 18, horizontal: 14),
                    child: Text(
                      emptyText,
                      textAlign: TextAlign.center,
                      style: const TextStyle(fontSize: 12.5, color: _ink2, height: 1.45),
                    ),
                  )
                : ListView.separated(
                    shrinkWrap: true,
                    padding: const EdgeInsets.only(bottom: 2),
                    itemCount: items.length,
                    separatorBuilder: (_, _) => const SizedBox(height: 6),
                    itemBuilder: (_, i) => items[i],
                  ),
          ),
        ],
      ),
    );
  }
}

/// 展开待办里的同心内卡片（圆角 12，和外圆角 20 / 内边距 10 保持顺滑同心，无硬边框）。
class _InnerCard extends StatelessWidget {
  const _InnerCard({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) => Container(
    padding: const EdgeInsets.fromLTRB(12, 10, 12, 10),
    decoration: ShapeDecoration(
      color: const Color(0xA6FFFFFF),
      shape: RoundedSuperellipseBorder(
        borderRadius: BorderRadius.circular(12),
        side: const BorderSide(color: Color(0x80FFFFFF), width: 0.5),
      ),
      shadows: const [BoxShadow(color: Color(0x0A1A1715), offset: Offset(0, 1), blurRadius: 3)],
    ),
    child: child,
  );
}

/// 展开待办里的「任务已完成 · 待审批」卡片（与左中下停泊区同源，支持直接通过/打回，或一键滑出员工三栏窗）。
class _GlassCompletionApprovalCard extends StatefulWidget {
  const _GlassCompletionApprovalCard({super.key, required this.item, this.onOpenAgent, required this.onOpenThread});

  final CompletionApproval item;
  final ValueChanged<String>? onOpenAgent;
  final void Function(String threadId, {String? agentId}) onOpenThread;

  @override
  State<_GlassCompletionApprovalCard> createState() => _GlassCompletionApprovalCardState();
}

class _GlassCompletionApprovalCardState extends State<_GlassCompletionApprovalCard> {
  CompletionDecision? _busy;

  Future<void> _decide(CompletionDecision decision) async {
    setState(() => _busy = decision);
    try {
      await AppScope.read(context).decideCompletionApproval(widget.item, decision);
    } on RequestError catch (e) {
      if (mounted) showError(context, '审批操作失败：$e');
    } finally {
      if (mounted) setState(() => _busy = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final item = widget.item;
    final canAct = _busy == null && (item.source == CompletionSource.localDemo || state.connected);
    return _InnerCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              const _StatusDot(InboxLight.yellow, size: 7),
              const SizedBox(width: 7),
              Expanded(
                child: Text(
                  '${item.agentName} · 完成 · 待审批',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Color(0xFF9C5B00)),
                ),
              ),
              Text(
                formatTime(item.createdAtMs),
                style: const TextStyle(fontSize: 11, color: _ink3, fontFeatures: [ui.FontFeature.tabularFigures()]),
              ),
            ],
          ),
          const SizedBox(height: 6),
          Text(
            item.taskTitle,
            style: const TextStyle(
              fontSize: 13.5,
              fontWeight: FontWeight.w600,
              height: 1.38,
              letterSpacing: -0.15,
              color: _ink,
            ),
          ),
          if (item.summary.isNotEmpty) ...[
            const SizedBox(height: 4),
            Text(item.summary, style: const TextStyle(fontSize: 12, color: _ink2, height: 1.4)),
          ],
          if (widget.onOpenAgent != null || item.threadId != null) ...[
            const SizedBox(height: 6),
            Align(
              alignment: Alignment.centerRight,
              child: GestureDetector(
                onTap: () {
                  if (widget.onOpenAgent != null) {
                    widget.onOpenAgent!(item.agentId);
                  } else if (item.threadId != null) {
                    widget.onOpenThread(item.threadId!, agentId: item.agentId);
                  }
                },
                child: const Text(
                  '打开工作窗 ›',
                  style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Color(0xFF8A5A12)),
                ),
              ),
            ),
          ],
          const SizedBox(height: 9),
          Row(
            children: [
              Expanded(
                child: Material(
                  color: canAct ? _ink : _ink.withValues(alpha: 0.45),
                  shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(10)),
                  child: InkWell(
                    key: ValueKey('inbox-completion-approve-${item.agentId}'),
                    customBorder: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(10)),
                    onTap: canAct ? () => _decide(CompletionDecision.approve) : null,
                    child: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 8),
                      child: Center(
                        child: _busy == CompletionDecision.approve
                            ? const SizedBox.square(
                                dimension: 14,
                                child: CircularProgressIndicator(strokeWidth: 2, color: Color(0xFFFDFBF7)),
                              )
                            : const Text(
                                '审批通过',
                                style: TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: Color(0xFFFDFBF7)),
                              ),
                      ),
                    ),
                  ),
                ),
              ),
              const SizedBox(width: 8),
              Expanded(
                child: Material(
                  color: const Color(0x121A1715),
                  shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(10)),
                  child: InkWell(
                    key: ValueKey('inbox-completion-reject-${item.agentId}'),
                    customBorder: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(10)),
                    onTap: canAct ? () => _decide(CompletionDecision.reject) : null,
                    child: Padding(
                      padding: const EdgeInsets.symmetric(vertical: 8),
                      child: Center(
                        child: _busy == CompletionDecision.reject
                            ? const SizedBox.square(
                                dimension: 14,
                                child: CircularProgressIndicator(strokeWidth: 2, color: _ink),
                              )
                            : const Text(
                                '打回修改',
                                style: TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: _ink),
                              ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}

/// 展开待办里的审批 / 提问卡片：可直接拍板选项，点“去对话”滑出员工三栏面板。
class _GlassApprovalCard extends StatefulWidget {
  const _GlassApprovalCard({super.key, required this.light, required this.approval, required this.onOpenThread});

  final InboxLight light;
  final Approval approval;
  final void Function(String threadId, {String? agentId}) onOpenThread;

  @override
  State<_GlassApprovalCard> createState() => _GlassApprovalCardState();
}

class _GlassApprovalCardState extends State<_GlassApprovalCard> {
  int? _sending;

  Future<void> _choose(int i) async {
    setState(() => _sending = i);
    try {
      await AppScope.read(context).resolveApproval(widget.approval, i);
    } on RequestError catch (e) {
      if (mounted) showError(context, '操作失败：$e');
    } finally {
      if (mounted) setState(() => _sending = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final a = widget.approval;
    final thread = a.subjectThreadId;
    final headerLabel = a.subjectAgentId != null ? '${state.agentName(a.subjectAgentId!)} 提问' : '等你决定';
    return _InnerCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              _StatusDot(widget.light, size: 7),
              const SizedBox(width: 7),
              Expanded(
                child: Text(
                  headerLabel,
                  style: const TextStyle(fontSize: 11.5, fontWeight: FontWeight.w500, color: _ink2),
                ),
              ),
              Text(
                formatTime(a.createdAtMs),
                style: const TextStyle(fontSize: 11, color: _ink3, fontFeatures: [ui.FontFeature.tabularFigures()]),
              ),
            ],
          ),
          const SizedBox(height: 6),
          Text(
            a.question,
            style: const TextStyle(
              fontSize: 13.5,
              fontWeight: FontWeight.w600,
              height: 1.38,
              letterSpacing: -0.15,
              color: _ink,
            ),
          ),
          if (a.reason.isNotEmpty) ...[
            const SizedBox(height: 4),
            Text(a.reason, style: const TextStyle(fontSize: 12, color: _ink2, height: 1.4)),
          ],
          if (thread != null) ...[
            const SizedBox(height: 6),
            Align(
              alignment: Alignment.centerRight,
              child: GestureDetector(
                onTap: () => widget.onOpenThread(thread, agentId: a.subjectAgentId),
                child: const Text(
                  '去对话 ›',
                  style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Color(0xFF8A5A12)),
                ),
              ),
            ),
          ],
          const SizedBox(height: 10),
          for (final (i, o) in a.options.indexed)
            Padding(
              padding: EdgeInsets.only(bottom: i == a.options.length - 1 ? 0 : 6),
              child: _GlassOptionButton(
                option: o,
                recommended: a.recommended == i,
                chosen: a.resolution?.choice == i,
                busy: _sending == i,
                onPressed: a.pending && _sending == null && state.connected ? () => _choose(i) : null,
              ),
            ),
        ],
      ),
    );
  }
}

class _GlassOptionButton extends StatelessWidget {
  const _GlassOptionButton({
    required this.option,
    required this.recommended,
    required this.chosen,
    required this.busy,
    required this.onPressed,
  });

  final ApprovalOption option;
  final bool recommended;
  final bool chosen;
  final bool busy;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    final highlighted = recommended || chosen;
    final bg = highlighted ? _ink : const Color(0x0F1A1715);
    final fg = highlighted ? const Color(0xFFFDFBF7) : _ink;
    final sub = highlighted ? const Color(0xC4FDFBF7) : _ink2;
    return Material(
      color: onPressed == null ? bg.withValues(alpha: 0.45) : bg,
      shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(11)),
      child: InkWell(
        customBorder: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(11)),
        onTap: onPressed,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
          child: Row(
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      recommended ? '${option.label}（推荐）' : option.label,
                      style: TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: fg),
                    ),
                    if (option.consequence.isNotEmpty) ...[
                      const SizedBox(height: 2),
                      Text(option.consequence, style: TextStyle(fontSize: 11.5, color: sub, height: 1.35)),
                    ],
                  ],
                ),
              ),
              if (busy) SizedBox.square(dimension: 15, child: CircularProgressIndicator(strokeWidth: 2, color: fg)),
              if (chosen) Icon(Icons.check, size: 16, color: fg),
            ],
          ),
        ),
      ),
    );
  }
}

/// 展开待办里的受阻 / 过期 / 即将到期任务卡片：可直接改状态或跳转对话。
class _GlassTaskCard extends StatefulWidget {
  const _GlassTaskCard({
    super.key,
    required this.light,
    required this.badge,
    required this.task,
    required this.onOpenThread,
  });

  final InboxLight light;
  final String badge;
  final Task task;
  final void Function(String threadId, {String? agentId}) onOpenThread;

  @override
  State<_GlassTaskCard> createState() => _GlassTaskCardState();
}

class _GlassTaskCardState extends State<_GlassTaskCard> {
  bool _saving = false;

  Future<void> _setStatus(String status) async {
    if (status == widget.task.status) return;
    setState(() => _saving = true);
    try {
      await AppScope.read(context).updateTask(widget.task, status);
    } on RequestError catch (e) {
      if (mounted) showError(context, '改状态失败：$e');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final t = widget.task;
    final who = t.assignee != null ? state.agentName(t.assignee!) : (t.role ?? '未指派');
    return _InnerCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              _StatusDot(widget.light, size: 7),
              const SizedBox(width: 7),
              Expanded(
                child: Text(
                  '${widget.badge} · $who',
                  style: const TextStyle(fontSize: 11.5, fontWeight: FontWeight.w500, color: _ink2),
                ),
              ),
              Text(
                formatTime(t.updatedAtMs),
                style: const TextStyle(fontSize: 11, color: _ink3, fontFeatures: [ui.FontFeature.tabularFigures()]),
              ),
            ],
          ),
          const SizedBox(height: 6),
          Text(
            t.title,
            style: const TextStyle(fontSize: 13.5, fontWeight: FontWeight.w600, height: 1.35, color: _ink),
          ),
          if (t.result case final r? when r.isNotEmpty) ...[
            const SizedBox(height: 4),
            Text(
              r,
              maxLines: 3,
              overflow: TextOverflow.ellipsis,
              style: const TextStyle(fontSize: 12, color: _ink2, height: 1.4),
            ),
          ],
          const SizedBox(height: 8),
          Row(
            children: [
              if (_saving)
                const SizedBox.square(dimension: 14, child: CircularProgressIndicator(strokeWidth: 2))
              else
                Container(
                  padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 2),
                  decoration: BoxDecoration(
                    color: widget.light.color.withValues(alpha: 0.12),
                    borderRadius: BorderRadius.circular(999),
                  ),
                  child: DropdownButtonHideUnderline(
                    child: DropdownButton<String>(
                      key: ValueKey('inbox-task-status-${t.id}'),
                      value: taskStatusText.containsKey(t.status) ? t.status : null,
                      isDense: true,
                      dropdownColor: const Color(0xFFFFFCF5),
                      borderRadius: BorderRadius.circular(Radii.menu),
                      icon: Icon(Icons.expand_more, size: 14, color: widget.light.color),
                      style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: widget.light.color),
                      items: [
                        for (final MapEntry(key: v, value: label) in taskStatusText.entries)
                          DropdownMenuItem(
                            value: v,
                            child: Text(label, style: const TextStyle(fontSize: 12, color: _ink)),
                          ),
                      ],
                      onChanged: !state.connected ? null : (v) => v == null ? null : _setStatus(v),
                    ),
                  ),
                ),
              const Spacer(),
              if (t.threadId case final tid?)
                GestureDetector(
                  onTap: () => widget.onOpenThread(tid, agentId: t.assignee),
                  child: const Text(
                    '执行对话 ›',
                    style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Color(0xFF8A5A12)),
                  ),
                ),
            ],
          ),
        ],
      ),
    );
  }
}

/// 绿灯动态：最近 7 天完成的任务。
class _GlassDoneTaskCard extends StatefulWidget {
  const _GlassDoneTaskCard({super.key, required this.task, required this.onOpenThread});

  final Task task;
  final void Function(String threadId, {String? agentId}) onOpenThread;

  @override
  State<_GlassDoneTaskCard> createState() => _GlassDoneTaskCardState();
}

class _GlassDoneTaskCardState extends State<_GlassDoneTaskCard> {
  bool _expanded = false;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final t = widget.task;
    final hasResult = t.result != null && t.result!.isNotEmpty;
    return _InnerCard(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Row(
            children: [
              const _StatusDot(InboxLight.green, size: 7),
              const SizedBox(width: 7),
              const Expanded(
                child: Text(
                  '已完成',
                  style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w500, color: _ink2),
                ),
              ),
              if (hasResult)
                GestureDetector(
                  onTap: () => setState(() => _expanded = !_expanded),
                  child: Text(
                    _expanded ? '收起结果' : '看结果',
                    style: const TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Color(0xFF8A5A12)),
                  ),
                ),
            ],
          ),
          const SizedBox(height: 5),
          Text(
            t.title,
            style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600, color: _ink),
          ),
          const SizedBox(height: 2),
          Text(taskMeta(state, t), style: const TextStyle(fontSize: 11, color: _ink2)),
          if (_expanded && hasResult) ...[
            const SizedBox(height: 8),
            MessageMarkdown(t.result!, reportHint: t.assignee),
          ],
          if (t.threadId != null || t.originThreadId != null) ...[
            const SizedBox(height: 6),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                if (t.threadId case final tid?)
                  GestureDetector(
                    onTap: () => widget.onOpenThread(tid, agentId: t.assignee),
                    child: const Text(
                      '执行对话 ›',
                      style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Color(0xFF8A5A12)),
                    ),
                  ),
                if (t.threadId != null && t.originThreadId != null) const SizedBox(width: 12),
                if (t.originThreadId case final oid?)
                  GestureDetector(
                    onTap: () => widget.onOpenThread(oid, agentId: t.createdByAgent),
                    child: const Text(
                      '派活对话 ›',
                      style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Color(0xFF8A5A12)),
                    ),
                  ),
              ],
            ),
          ],
        ],
      ),
    );
  }
}

/// 绿灯动态：最近 7 天新报告。
class _GlassReportCard extends StatelessWidget {
  const _GlassReportCard({super.key, required this.report});

  final ReportInfo report;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    return GestureDetector(
      onTap: () => openReport(context, report.path, hint: report.agentId),
      child: _InnerCard(
        child: Row(
          children: [
            const _StatusDot(InboxLight.green, size: 7),
            const SizedBox(width: 10),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    report.fileName,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis,
                    style: const TextStyle(fontSize: 13, fontWeight: FontWeight.w600, color: _ink),
                  ),
                  const SizedBox(height: 2),
                  Text(
                    '${state.agentName(report.agentId)} · ${formatTime(report.modifiedMs)}',
                    style: const TextStyle(fontSize: 11, color: _ink2),
                  ),
                ],
              ),
            ),
            const Icon(Icons.chevron_right, size: 16, color: _ink3),
          ],
        ),
      ),
    );
  }
}
