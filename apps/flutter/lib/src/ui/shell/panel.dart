// 右栏：员工面板（设计重构第一步，照 dusk bot 的 Bot 面板；requirements.txt N-7）。
// 头像和名字在顶上，下面三个标签：概况、任务、岗位。
// 能改的只有现在主机就支持的（任务状态）；要主机配合的（改模型、改指令、开关定时任务、按员工设权限）
// 先把位置放好，置灰并写明“主机还不支持”（requirements.txt 第二部分）。

import 'package:flutter/material.dart';

import '../../protocol/models.dart';
import '../../star_map.dart' show isChiefOfStaff;
import '../common.dart';
import '../report.dart';
import '../roles_page.dart' show usd;
import '../task_card.dart';
import 'avatar.dart';
import 'tokens.dart';

class AgentPanel extends StatefulWidget {
  const AgentPanel({super.key, required this.agentId, required this.onClose, required this.onOpenThread});

  final String agentId;
  final VoidCallback onClose;
  final void Function(String threadId) onOpenThread;

  @override
  State<AgentPanel> createState() => _AgentPanelState();
}

class _AgentPanelState extends State<AgentPanel> {
  int _tab = 0;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final agent = state.agents.where((a) => a.id == widget.agentId).firstOrNull;
    if (agent == null) return const SizedBox.shrink();
    final role = state.roles.where((r) => r.id == agent.role).firstOrNull;
    final chief = isChiefOfStaff(agent, role);
    final waiting = state.pendingApprovals.any((a) => a.subjectAgentId == agent.id);
    return Material(
      color: s.panel,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 10, 8, 0),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                IconButton(
                  tooltip: '关闭面板（Esc）',
                  onPressed: widget.onClose,
                  icon: Icon(Icons.close, size: 18, color: s.inkSecondary),
                ),
              ],
            ),
          ),
          AgentAvatar(
            id: agent.id,
            name: agent.name,
            size: 64,
            working: state.agentsWorking.contains(agent.id),
            waiting: waiting,
            failed: state.agentFailures.containsKey(agent.id),
            chief: chief,
          ).center(),
          const SizedBox(height: 10),
          Text(
            agent.name,
            textAlign: TextAlign.center,
            style: TextStyle(color: s.ink, fontSize: 17, fontWeight: FontWeight.w600),
          ),
          const SizedBox(height: 2),
          Text(
            [if (chief) '幕僚长', role?.displayName ?? agent.role].join(' · '),
            textAlign: TextAlign.center,
            style: TextStyle(color: s.inkSecondary, fontSize: 13),
          ),
          const SizedBox(height: 14),
          Center(
            child: _Tabs(
              labels: const ['概况', '任务', '岗位'],
              index: _tab,
              onTap: (i) => setState(() => _tab = i),
            ),
          ),
          const SizedBox(height: 10),
          Expanded(
            child: switch (_tab) {
              0 => _Overview(agent: agent, onOpenThread: widget.onOpenThread),
              1 => _TasksTab(agent: agent, onOpenThread: widget.onOpenThread),
              _ => _RoleTab(agent: agent, role: role),
            },
          ),
        ],
      ),
    );
  }
}

extension on Widget {
  Widget center() => Center(child: this);
}

/// 一排胶囊标签，选中的是白底（照 dusk 的 Details / Computer / Advanced）。
class _Tabs extends StatelessWidget {
  const _Tabs({required this.labels, required this.index, required this.onTap});

  final List<String> labels;
  final int index;
  final void Function(int) onTap;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final (i, l) in labels.indexed)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 2),
            child: Material(
              color: i == index ? s.raised : Colors.transparent,
              borderRadius: BorderRadius.circular(8),
              child: InkWell(
                borderRadius: BorderRadius.circular(8),
                onTap: () => onTap(i),
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 5),
                  child: Text(
                    l,
                    style: TextStyle(
                      color: i == index ? s.ink : s.inkSecondary,
                      fontSize: 13,
                      fontWeight: i == index ? FontWeight.w600 : FontWeight.w500,
                    ),
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}

/// 面板里的一块：白底圆角卡片（照 dusk 的“Routines”“Model”块）。
class PanelCard extends StatelessWidget {
  const PanelCard({super.key, this.title, required this.children});

  final String? title;
  final List<Widget> children;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(14, 0, 14, 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (title != null)
            Padding(
              padding: const EdgeInsets.fromLTRB(4, 6, 4, 6),
              child: Text(title!, style: TextStyle(color: s.inkTertiary, fontSize: 12.5)),
            ),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
            decoration: BoxDecoration(color: s.composer, borderRadius: BorderRadius.circular(Radii.xl)),
            child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: children),
          ),
        ],
      ),
    );
  }
}

/// 卡片里的一行：左边名字和说明，右边值。
class PanelRow extends StatelessWidget {
  const PanelRow({super.key, required this.label, this.detail, this.trailing, this.onTap, this.disabled = false});

  final String label;
  final String? detail;
  final Widget? trailing;
  final VoidCallback? onTap;

  /// 要主机配合、现在用不了的：字变淡。
  final bool disabled;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final row = Padding(
      padding: const EdgeInsets.symmetric(vertical: 9),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(label, style: TextStyle(color: disabled ? s.inkTertiary : s.ink, fontSize: 13.5)),
                if (detail != null)
                  Padding(
                    padding: const EdgeInsets.only(top: 1),
                    child: Text(detail!, style: TextStyle(color: s.inkTertiary, fontSize: 12)),
                  ),
              ],
            ),
          ),
          if (trailing != null) ...[const SizedBox(width: 10), trailing!],
        ],
      ),
    );
    return onTap == null ? row : InkWell(onTap: onTap, child: row);
  }
}

/// 要主机配合的入口：开关置灰，悬停说明为什么。
class NotYetSwitch extends StatelessWidget {
  const NotYetSwitch({super.key, this.value = false});

  final bool value;

  @override
  Widget build(BuildContext context) => Tooltip(
    message: '主机还不支持，在 ip.yaml 里改',
    child: Transform.scale(scale: 0.8, child: Switch(value: value, onChanged: null)),
  );
}

class _Overview extends StatelessWidget {
  const _Overview({required this.agent, required this.onOpenThread});

  final Agent agent;
  final void Function(String threadId) onOpenThread;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final asks = state.pendingApprovals.where((a) => a.subjectAgentId == agent.id).toList();
    final mine = state.tasks.where((t) => t.assignee == agent.id && t.isOpen).toList();
    final failure = state.agentFailures[agent.id];
    final reports = state.reports.where((r) => r.agentId == agent.id).take(4).toList();
    final status = state.agentsWorking.contains(agent.id)
        ? ('正在处理', s.success)
        : failure != null
        ? ('上一轮失败了', s.danger)
        : asks.isNotEmpty
        ? ('在等你回答', s.warning)
        : ('空闲', s.inkTertiary);
    return ListView(
      padding: const EdgeInsets.only(bottom: 20),
      children: [
        PanelCard(
          children: [
            PanelRow(
              label: '状态',
              detail: failure,
              trailing: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Container(
                    width: 7,
                    height: 7,
                    decoration: BoxDecoration(color: status.$2, shape: BoxShape.circle),
                  ),
                  const SizedBox(width: 6),
                  Text(status.$1, style: TextStyle(color: s.ink, fontSize: 13)),
                ],
              ),
            ),
            PanelRow(
              label: '在问你',
              trailing: Text('${asks.length}', style: TextStyle(color: s.ink, fontSize: 13)),
            ),
            PanelRow(
              label: '手上的任务',
              trailing: Text('${mine.length}', style: TextStyle(color: s.ink, fontSize: 13)),
            ),
          ],
        ),
        if (asks.isNotEmpty)
          PanelCard(
            title: '在问你',
            children: [
              for (final a in asks)
                PanelRow(
                  label: a.question,
                  detail: '发起于 ${formatTime(a.createdAtMs)}',
                  trailing: Icon(Icons.chevron_right, size: 16, color: s.inkTertiary),
                  onTap: a.subjectThreadId == null ? null : () => onOpenThread(a.subjectThreadId!),
                ),
            ],
          ),
        PanelCard(
          title: '定时任务',
          children: [
            PanelRow(
              label: '在设置“定时任务”里看',
              detail: '新建、开关定时任务要主机支持（requirements N-25）',
              disabled: true,
              trailing: const NotYetSwitch(),
            ),
          ],
        ),
        PanelCard(
          title: '最近的报告',
          children: [
            if (reports.isEmpty) const PanelRow(label: '还没有', disabled: true),
            for (final r in reports)
              PanelRow(
                label: r.fileName,
                detail: formatTime(r.modifiedMs),
                trailing: Icon(Icons.description_outlined, size: 16, color: s.inkTertiary),
                onTap: () => openReport(context, r.path, hint: r.agentId),
              ),
          ],
        ),
      ],
    );
  }
}

class _TasksTab extends StatelessWidget {
  const _TasksTab({required this.agent, required this.onOpenThread});

  final Agent agent;
  final void Function(String threadId) onOpenThread;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final open = state.tasks.where((t) => t.assignee == agent.id && t.isOpen).toList();
    final done = state.tasks.where((t) => t.assignee == agent.id && !t.isOpen).take(10).toList();
    if (open.isEmpty && done.isEmpty) {
      return Padding(
        padding: const EdgeInsets.all(24),
        child: Text('${agent.name} 手上没有任务。', textAlign: TextAlign.center, style: TextStyle(color: s.inkTertiary)),
      );
    }
    return ListView(
      padding: const EdgeInsets.fromLTRB(14, 0, 14, 20),
      children: [
        for (final t in open) TaskCard(key: ValueKey(t.id), task: t, onOpenThread: onOpenThread),
        if (done.isNotEmpty)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 8, 4, 6),
            child: Text('已结束', style: TextStyle(color: s.inkTertiary, fontSize: 12.5)),
          ),
        for (final t in done)
          PanelRow(
            label: t.title,
            detail: '${taskStatusText[t.status] ?? t.status} · ${formatTime(t.updatedAtMs)}',
            onTap: t.threadId == null ? null : () => onOpenThread(t.threadId!),
          ),
      ],
    );
  }
}

class _RoleTab extends StatelessWidget {
  const _RoleTab({required this.agent, required this.role});

  final Agent agent;
  final Role? role;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final r = role;
    if (r == null) return const SizedBox.shrink();
    final spent = state.spend?.roles[r.id] ?? 0;
    final limit = r.budgetPerDayUsd;
    final u = state.spend?.tokens[r.id];
    final hit = u == null || u.promptTokens == 0 ? '还没有调用' : '${(u.cachedTokens / u.promptTokens * 100).round()}%';
    return ListView(
      padding: const EdgeInsets.only(bottom: 20),
      children: [
        PanelCard(
          title: '模型',
          children: [
            PanelRow(
              label: r.model,
              detail: r.escalateModel == null ? null : '升级：${r.escalateModel}',
              trailing: Tooltip(
                message: '换模型要主机支持（requirements N-24）',
                child: Icon(Icons.unfold_more, size: 16, color: s.inkTertiary),
              ),
            ),
          ],
        ),
        PanelCard(
          title: '今天',
          children: [
            PanelRow(
              label: '模型费用',
              detail: limit == null ? '没设每日预算' : '每日预算 \$${limit.toStringAsFixed(limit % 1 == 0 ? 0 : 2)}',
              trailing: Text(usd(spent), style: TextStyle(color: s.ink, fontSize: 13)),
            ),
            if (limit != null && limit > 0)
              Padding(
                padding: const EdgeInsets.only(bottom: 10),
                child: ClipRRect(
                  borderRadius: BorderRadius.circular(2),
                  child: LinearProgressIndicator(
                    value: (spent / limit).clamp(0, 1),
                    minHeight: 4,
                    backgroundColor: s.card,
                    color: spent >= limit ? s.danger : s.accent,
                  ),
                ),
              ),
            PanelRow(label: '提示词缓存命中', trailing: Text(hit, style: TextStyle(color: s.ink, fontSize: 13))),
          ],
        ),
        PanelCard(
          title: '权限',
          children: const [
            PanelRow(
              label: '先问我再做',
              detail: '按员工设审批级别要主机支持（requirements N-26）',
              disabled: true,
              trailing: NotYetSwitch(value: true),
            ),
          ],
        ),
        if (r.description.isNotEmpty)
          PanelCard(
            title: '岗位说明',
            children: [
              Padding(
                padding: const EdgeInsets.symmetric(vertical: 10),
                child: Text(r.description, style: TextStyle(color: s.inkSecondary, fontSize: 13, height: 1.5)),
              ),
            ],
          ),
      ],
    );
  }
}
