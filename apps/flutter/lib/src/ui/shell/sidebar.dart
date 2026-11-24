// 桌面外壳的左栏（设计重构第一步，照 dusk bot 的左栏）：
// 顶上搜索和“新对话”；固定入口“待办”“星图”；下面按岗位分组列员工（头像、名字、最后一句）；
// 底部“我”（打开设置）和连接状态。可以拖宽拖窄，拖到最窄收起成一列头像。

import 'package:flutter/material.dart';

import '../../app_state.dart';
import '../../protocol/client.dart';
import '../../protocol/models.dart';
import '../../star_map.dart' show isChiefOfStaff;
import '../common.dart';
import 'avatar.dart';
import 'tokens.dart';

/// 外壳中间现在显示什么。
enum ShellView { inbox, stars, chat, tasks, roles }

/// 一位员工在左栏里的一行要显示的东西。
class AgentRowInfo {
  AgentRowInfo({required this.agent, required this.preview, required this.lastAtMs, required this.threadId});

  final Agent agent;

  /// 最后一句（这位员工最近的对话的预览）。
  final String? preview;
  final int? lastAtMs;

  /// 点这一行时打开的对话：这位员工最近的对话；没有时开新对话。
  final String? threadId;
}

/// 每位员工最近的对话。协议的对话列表不带员工，用 App 记下的“这个对话最近由谁接手”拼。
Map<String, AgentRowInfo> agentRows(AppState state) {
  final out = <String, AgentRowInfo>{};
  for (final a in state.agents) {
    ThreadSummary? last;
    for (final t in state.threads) {
      if (state.threadAgent[t.threadId] == a.id && (last == null || t.lastAtMs > last.lastAtMs)) last = t;
    }
    out[a.id] = AgentRowInfo(agent: a, preview: last?.preview, lastAtMs: last?.lastAtMs, threadId: last?.threadId);
  }
  return out;
}

class ShellSidebar extends StatelessWidget {
  const ShellSidebar({
    super.key,
    required this.width,
    required this.view,
    required this.selectedAgent,
    required this.onView,
    required this.onAgent,
    required this.onNewChat,
    required this.onSearch,
    required this.onSettings,
    required this.onAllThreads,
  });

  final double width;
  final ShellView view;

  /// 对话里正在看的员工（高亮那一行）。
  final String? selectedAgent;
  final void Function(ShellView view) onView;
  final void Function(Agent agent, String? threadId) onAgent;
  final VoidCallback onNewChat;
  final VoidCallback onSearch;
  final VoidCallback onSettings;
  final VoidCallback onAllThreads;

  bool get _rail => width <= Sizes.sidebarRail + 1;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final rows = agentRows(state);
    final roleById = {for (final r in state.roles) r.id: r};
    // 岗位按 ip.yaml 里的顺序；幕僚长的岗位放最前。
    final roles = [...state.roles]
      ..sort((a, b) {
        bool chief(Role r) => state.agents.any((x) => x.role == r.id && isChiefOfStaff(x, r));
        return (chief(b) ? 1 : 0) - (chief(a) ? 1 : 0);
      });
    final orphans = state.agents.where((a) => !roleById.containsKey(a.role)).toList();
    final waiting = {for (final a in state.pendingApprovals) ?a.subjectAgentId};
    Widget agentRow(Agent a) => _AgentRow(
      info: rows[a.id]!,
      rail: _rail,
      selected: view == ShellView.chat && selectedAgent == a.id,
      working: state.agentsWorking.contains(a.id),
      waiting: waiting.contains(a.id),
      failed: state.agentFailures.containsKey(a.id),
      chief: isChiefOfStaff(a, roleById[a.role]),
      onTap: () => onAgent(a, rows[a.id]!.threadId),
    );
    return Material(
      color: s.panel,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // 顶上：搜索和新对话。
          Padding(
            padding: EdgeInsets.fromLTRB(_rail ? 0 : 12, 14, _rail ? 0 : 10, 8),
            child: _rail
                ? Column(
                    children: [
                      _RoundIcon(icon: Icons.search, tip: '搜索（⌘K）', onTap: onSearch),
                      const SizedBox(height: 6),
                      _RoundIcon(icon: Icons.add, tip: '新对话（⌘N）', onTap: onNewChat),
                    ],
                  )
                : Row(
                    children: [
                      Expanded(
                        child: Text(
                          'intelligent-position',
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(color: s.inkSecondary, fontSize: 13, fontWeight: FontWeight.w500),
                        ),
                      ),
                      _RoundIcon(icon: Icons.search, tip: '搜索（⌘K）', onTap: onSearch),
                      const SizedBox(width: 6),
                      _RoundIcon(icon: Icons.add, tip: '新对话（⌘N）', onTap: onNewChat),
                    ],
                  ),
          ),
          Expanded(
            child: ListView(
              padding: EdgeInsets.symmetric(horizontal: _rail ? 10 : 8, vertical: 2),
              children: [
                _PlaceRow(
                  icon: Icons.inbox_outlined,
                  label: '待办',
                  count: state.inboxCount,
                  rail: _rail,
                  selected: view == ShellView.inbox,
                  onTap: () => onView(ShellView.inbox),
                ),
                _PlaceRow(
                  icon: Icons.auto_awesome_outlined,
                  label: '星图',
                  rail: _rail,
                  selected: view == ShellView.stars,
                  onTap: () => onView(ShellView.stars),
                ),
                _PlaceRow(
                  icon: Icons.task_alt_outlined,
                  label: '任务',
                  count: state.tasks.where((t) => t.isOpen).length,
                  muted: true,
                  rail: _rail,
                  selected: view == ShellView.tasks,
                  onTap: () => onView(ShellView.tasks),
                ),
                _PlaceRow(
                  icon: Icons.forum_outlined,
                  label: '对话',
                  rail: _rail,
                  selected: view == ShellView.chat && selectedAgent == null,
                  onTap: onAllThreads,
                ),
                const SizedBox(height: 10),
                if (state.agents.isEmpty && !_rail)
                  Padding(
                    padding: const EdgeInsets.fromLTRB(10, 8, 10, 8),
                    child: Text(
                      state.connected ? '主机上还没有数字员工。' : '连上主机后，员工会显示在这里。',
                      style: TextStyle(color: s.inkTertiary, fontSize: 13),
                    ),
                  ),
                for (final r in roles)
                  if (state.agents.any((a) => a.role == r.id)) ...[
                    if (!_rail)
                      _SectionHeader(
                        label: r.name ?? r.id,
                        onTap: () => onView(ShellView.roles),
                      )
                    else
                      const SizedBox(height: 8),
                    for (final a in state.agents.where((a) => a.role == r.id)) agentRow(a),
                  ],
                if (orphans.isNotEmpty) ...[
                  if (!_rail) const _SectionHeader(label: '其他') else const SizedBox(height: 8),
                  for (final a in orphans) agentRow(a),
                ],
              ],
            ),
          ),
          _Footer(rail: _rail, onSettings: onSettings),
        ],
      ),
    );
  }
}

/// 圆形的小图标按钮（搜索、+），照 dusk 的顶栏。
class _RoundIcon extends StatelessWidget {
  const _RoundIcon({required this.icon, required this.tip, required this.onTap});

  final IconData icon;
  final String tip;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Tooltip(
      message: tip,
      child: Material(
        color: s.composer,
        shape: CircleBorder(side: BorderSide(color: s.edge, width: 0.5)),
        child: InkWell(
          customBorder: const CircleBorder(),
          onTap: onTap,
          child: SizedBox.square(dimension: 32, child: Icon(icon, size: 17, color: s.ink)),
        ),
      ),
    );
  }
}

/// 一行能点的底：选中是 raised，悬停淡一点。
class _RowSurface extends StatefulWidget {
  const _RowSurface({
    super.key,
    required this.selected,
    required this.onTap,
    required this.child,
    this.height,
    this.radius = 12,
  });

  final bool selected;
  final VoidCallback onTap;
  final Widget child;
  final double? height;
  final double radius;

  @override
  State<_RowSurface> createState() => _RowSurfaceState();
}

class _RowSurfaceState extends State<_RowSurface> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final color = widget.selected
        ? s.raised
        : _hover
        ? s.raised.withValues(alpha: 0.5)
        : Colors.transparent;
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
          height: widget.height,
          decoration: BoxDecoration(color: color, borderRadius: BorderRadius.circular(widget.radius)),
          child: widget.child,
        ),
      ),
    );
  }
}

/// 固定入口：待办、星图、任务、全部对话。
class _PlaceRow extends StatelessWidget {
  const _PlaceRow({
    required this.icon,
    required this.label,
    required this.rail,
    required this.selected,
    required this.onTap,
    this.count = 0,
    this.muted = false,
  });

  final IconData icon;
  final String label;
  final bool rail;
  final bool selected;
  final VoidCallback onTap;

  /// 右边的数字；`muted` 时是灰的（只是个数），否则是醒目的角标（要你处理）。
  final int count;
  final bool muted;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final badge = count == 0
        ? null
        : Container(
            constraints: const BoxConstraints(minWidth: 18),
            padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
            decoration: BoxDecoration(
              color: muted ? Colors.transparent : s.accent,
              borderRadius: BorderRadius.circular(9),
            ),
            child: Text(
              '$count',
              textAlign: TextAlign.center,
              style: TextStyle(color: muted ? s.inkTertiary : Colors.white, fontSize: 11.5, fontWeight: FontWeight.w600),
            ),
          );
    final iconW = Icon(icon, size: 18, color: selected ? s.ink : s.inkSecondary);
    return Padding(
      padding: const EdgeInsets.only(bottom: 1),
      child: Tooltip(
        message: rail ? label : '',
        child: _RowSurface(
          selected: selected,
          onTap: onTap,
          height: rail ? 40 : 34,
          radius: 10,
          child: rail
              ? Center(
                  child: Badge(
                    isLabelVisible: count > 0 && !muted,
                    label: Text('$count'),
                    backgroundColor: s.accent,
                    child: iconW,
                  ),
                )
              : Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 10),
                  child: Row(
                    children: [
                      iconW,
                      const SizedBox(width: 10),
                      Expanded(
                        child: Text(
                          label,
                          style: TextStyle(
                            color: selected ? s.ink : s.inkSecondary,
                            fontSize: 13.5,
                            fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
                          ),
                        ),
                      ),
                      ?badge,
                    ],
                  ),
                ),
        ),
      ),
    );
  }
}

/// 岗位分组的标题，照 dusk 的“部门”标题：小字、灰色。点了去岗位页。
class _SectionHeader extends StatelessWidget {
  const _SectionHeader({required this.label, this.onTap});

  final String label;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 12, 10, 4),
      child: MouseRegion(
        cursor: onTap == null ? MouseCursor.defer : SystemMouseCursors.click,
        child: GestureDetector(
          onTap: onTap,
          child: Text(label, style: TextStyle(color: s.inkTertiary, fontSize: 12.5, fontWeight: FontWeight.w500)),
        ),
      ),
    );
  }
}

/// 一位员工：头像、名字、最后一句（58 高），照 dusk 的联系人行。
class _AgentRow extends StatelessWidget {
  const _AgentRow({
    required this.info,
    required this.rail,
    required this.selected,
    required this.working,
    required this.waiting,
    required this.failed,
    required this.chief,
    required this.onTap,
  });

  final AgentRowInfo info;
  final bool rail;
  final bool selected;
  final bool working;
  final bool waiting;
  final bool failed;
  final bool chief;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final a = info.agent;
    final avatar = AgentAvatar(
      id: a.id,
      name: a.name,
      size: rail ? 36 : 40,
      working: working,
      waiting: waiting,
      failed: failed,
      chief: chief,
    );
    final line = waiting
        ? '在等你回答'
        : failed
        ? '上一轮失败了'
        : working
        ? '正在处理…'
        : (info.preview ?? '还没有对话');
    final lineColor = waiting
        ? s.warning
        : failed
        ? s.danger
        : working
        ? s.success
        : s.inkSecondary;
    return Padding(
      padding: const EdgeInsets.only(bottom: 1),
      child: Tooltip(
        message: rail ? a.name : '',
        child: _RowSurface(
          key: ValueKey('agent-row-${a.id}'),
          selected: selected,
          onTap: onTap,
          height: rail ? 52 : 58,
          child: rail
              ? Center(child: avatar)
              : Padding(
                  padding: const EdgeInsets.all(8),
                  child: Row(
                    children: [
                      avatar,
                      const SizedBox(width: 10),
                      Expanded(
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Row(
                              children: [
                                Flexible(
                                  child: Text(
                                    a.name,
                                    overflow: TextOverflow.ellipsis,
                                    style: TextStyle(color: s.ink, fontSize: 14, fontWeight: FontWeight.w500),
                                  ),
                                ),
                                if (info.lastAtMs != null) ...[
                                  const SizedBox(width: 6),
                                  const Spacer(),
                                  Text(
                                    formatTime(info.lastAtMs!),
                                    style: TextStyle(color: s.inkTertiary, fontSize: 11.5),
                                  ),
                                ],
                              ],
                            ),
                            const SizedBox(height: 2),
                            Text(
                              line,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(color: lineColor, fontSize: 13),
                            ),
                          ],
                        ),
                      ),
                    ],
                  ),
                ),
        ),
      ),
    );
  }
}

/// 底部：“我”（打开设置）和连接状态。
class _Footer extends StatelessWidget {
  const _Footer({required this.rail, required this.onSettings});

  final bool rail;
  final VoidCallback onSettings;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final (String label, Color color) = switch (state.status) {
      _ when state.rejected != null => ('被主机拒绝', s.danger),
      ConnStatus.open => ('已连接', s.success),
      ConnStatus.connecting => ('正在连接', s.warning),
      ConnStatus.closed => ('断开了', s.danger),
    };
    final me = Tooltip(
      message: '我',
      child: Material(
        color: s.composer,
        shape: CircleBorder(side: BorderSide(color: s.edge, width: 0.5)),
        child: InkWell(
          customBorder: const CircleBorder(),
          onTap: onSettings,
          child: SizedBox.square(dimension: 36, child: Icon(Icons.person_outline, size: 18, color: s.ink)),
        ),
      ),
    );
    if (rail) return Padding(padding: const EdgeInsets.only(bottom: 14, top: 6), child: Center(child: me));
    final host = state.host?.address.origin.host ?? '';
    return Padding(
      padding: const EdgeInsets.fromLTRB(12, 6, 12, 14),
      child: Row(
        children: [
          me,
          const SizedBox(width: 8),
          Expanded(
            child: Material(
              color: s.composer,
              shape: StadiumBorder(side: BorderSide(color: s.edge, width: 0.5)),
              child: InkWell(
                customBorder: const StadiumBorder(),
                onTap: onSettings,
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 9),
                  child: Row(
                    children: [
                      Container(
                        width: 7,
                        height: 7,
                        decoration: BoxDecoration(color: color, shape: BoxShape.circle),
                      ),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Text(
                          host.isEmpty ? label : '$label · $host',
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(color: s.ink, fontSize: 13, fontWeight: FontWeight.w500),
                        ),
                      ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
