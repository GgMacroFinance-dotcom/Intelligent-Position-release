// 点星球（现在只有太阳）后的单员工浮空玻璃窗（设计重构，2026-11-26 维护者定）：
//   - 延续 Apple Liquid Glass + 原生 macOS 三栏窗口架构，简洁但高效：
//     左栏（232px，通透玻璃）：顶上原生红绿灯 + 圆形玻璃“+ 新对话”按钮，下面是这位员工的对话列表，最底下是安静的连接状态；
//     中间（同心超椭圆高亮内嵌面，圆角 20）：顶栏显示天体微标 + 名字 + 角色/在线状态 + 右侧检查器折叠按钮，下面是对话流与输入框；
//     右栏（296px，通透玻璃 Inspector，可折叠）：顶栏与左/中两栏对齐同一 42px 水平中线，直接放 Apple Segmented Control
//       切换「概况 / 在做什么 / 高级」，去掉重复的大头像块，把垂直空间留给高密度信息卡片。
//   - 删掉顶天立地的硬竖线与灰色拖拽横杠；按住三栏顶栏空白区往下拉仍可流畅收起窗口。

import 'dart:async';

import 'package:flutter/material.dart';

import '../../app_state.dart';
import '../../protocol/client.dart' show ConnStatus, IpRequests, RequestError;
import '../../protocol/models.dart';
import '../../star_map.dart';
import '../chat_page.dart' show ThreadView;
import '../common.dart';
import '../progress_bar.dart' show ProgressDetails, showsHeadBar;
import '../report.dart' show openReport;
import '../roles_page.dart' show usd;
import '../task_card.dart' show taskStatusLabel, taskStatusText;
import '../tasks_page.dart' show showNewTask;
import 'agent_sheet.dart';
import 'completion_approval.dart';
import 'local_bot.dart';
import 'orbit.dart';
import 'planet_importance.dart';
import 'settings.dart';
import 'tokens.dart';
import '../rework_note.dart';
import '../org_editor.dart';

/// 这位员工的一个对话（左栏的一行）。
typedef AgentThread = ({String threadId, String title, int atMs, int count, bool waiting});

/// 这位员工的对话：上次由它接手的、派给它的任务的执行对话、它在问你的那个对话、它最相关的对话。
/// 对话本身只记最后回复的员工（`ThreadSummary.lastAgentId`，已并进 `threadAgent`），所以再从任务、审批补上；按最近的时间排。
List<AgentThread> agentThreads(AppState state, StarMap map, String agentId) {
  final out = <String, AgentThread>{};
  final waitingThreads = {
    for (final a in state.pendingApprovals)
      if (a.subjectThreadId != null) a.subjectThreadId!,
  };
  ThreadSummary? summary(String id) {
    for (final t in state.threads) {
      if (t.threadId == id) return t;
    }
    return null;
  }

  void add(String? id, {String? title, int atMs = 0}) {
    if (id == null || out.containsKey(id)) return;
    final s = summary(id);
    final preview = s?.preview?.trim();
    out[id] = (
      threadId: id,
      title: (preview != null && preview.isNotEmpty) ? preview : (title ?? '对话'),
      atMs: s?.lastAtMs ?? atMs,
      count: s?.messageCount ?? 0,
      waiting: waitingThreads.contains(id),
    );
  }

  for (final t in state.threads) {
    if (state.threadAgent[t.threadId] == agentId) add(t.threadId);
  }
  for (final t in state.tasks) {
    if (t.assignee == agentId) add(t.threadId, title: t.title, atMs: t.updatedAtMs);
  }
  for (final a in state.pendingApprovals) {
    if (a.isAgentQuestion && a.subjectAgentId == agentId) {
      add(a.subjectThreadId, title: a.question, atMs: a.createdAtMs);
    }
  }
  add(map.byId(agentId)?.focusThreadId);
  return out.values.toList()..sort((a, b) => b.atMs.compareTo(a.atMs));
}

/// 员工三栏浮空窗口：左栏对话、中间内嵌高亮对话面、右栏 Inspector。[agentId] 为空是幕僚长（或空位）。
class AgentPanel extends StatelessWidget {
  const AgentPanel({
    super.key,
    required this.map,
    required this.agentId,
    required this.detailOpen,
    required this.onOpenThread,
    required this.onNewChat,
    required this.onSelectAgent,
    required this.onToggleDetail,
    required this.onClose,
    this.expanded = false,
    this.onToggleExpand,
  });

  final StarMap map;
  final String? agentId;
  final bool detailOpen;
  final void Function(String threadId) onOpenThread;

  /// 开一个发给这位员工的新对话。
  final void Function(String agentId) onNewChat;

  /// 右栏点 [[关联]]：换成那位员工。
  final void Function(String agentId) onSelectAgent;
  final VoidCallback onToggleDetail;
  final VoidCallback onClose;
  final bool expanded;
  final VoidCallback? onToggleExpand;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final b = agentId == null ? map.chief : map.byId(agentId!);
    final pendingCompletion = b == null ? null : state.completionApprovalForAgent(b.agent.id);

    // 不管有没有连上主机、有没有配幕僚长，始终保留左/中/右三栏原生窗口骨架。
    return Row(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SizedBox(
          width: 232,
          child: _AgentThreads(
            map: map,
            body: b,
            expanded: expanded,
            onClose: onClose,
            onToggleExpand: onToggleExpand,
            onOpenThread: onOpenThread,
            onNewChat: onNewChat,
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
                  child: _CenterHeader(
                    map: map,
                    body: b,
                    pendingCompletion: pendingCompletion,
                    detailOpen: detailOpen,
                    onToggleDetail: onToggleDetail,
                  ),
                ),
                Container(height: 0.5, color: s.edge),
                if (pendingCompletion != null) _CompletionApprovalBanner(item: pendingCompletion),
                Expanded(
                  child: b != null
                      ? (b.agent.role == 'demo'
                            ? _DemoApprovalBody(body: b, hasPending: pendingCompletion != null)
                            : state.localBotById(b.agent.id) != null
                            ? _LocalBotBody(
                                body: b,
                                record: state.localBotById(b.agent.id)!,
                                hasPending: pendingCompletion != null,
                                onRemoved: onClose,
                              )
                            : ThreadView(fixedAgentId: b.agent.id, onOpenThread: onOpenThread))
                      : _EmptyChatArea(map: map),
                ),
              ],
            ),
          ),
        ),
        if (detailOpen)
          SizedBox(
            width: 296,
            child: _AgentDetail(
              key: ValueKey('agent-detail-${b?.agent.id ?? 'empty'}'),
              map: map,
              body: b,
              onSelectAgent: onSelectAgent,
              onOpenThread: onOpenThread,
            ),
          ),
      ],
    );
  }
}

/// 天体的小圆点（太阳或空位用金色渐变与微光，其他员工用岗位色）。
Widget _bodyDot(AppState state, StarMap map, BotBody? b, double size) {
  final isSun = b == null || b == map.chief;
  final color = isSun ? panelGold : roleColorFor(state.allRoles, b.agent.role);
  return Container(
    width: size,
    height: size,
    decoration: BoxDecoration(
      shape: BoxShape.circle,
      color: isSun ? null : color,
      gradient: isSun
          ? const RadialGradient(
              center: Alignment(-0.2, -0.25),
              colors: [Color(0xFFFFF7D6), Color(0xFFEAB44C), Color(0xFFBD6B28)],
              stops: [0, 0.58, 1],
            )
          : null,
      boxShadow: [BoxShadow(color: color.withValues(alpha: isSun ? 0.42 : 0.28), blurRadius: 8)],
    ),
  );
}

/// 中间高亮内嵌面的原生 macOS 顶栏（高度 42px，与左栏红绿灯、右栏 Segmented Control 严格对齐）。
class _CenterHeader extends StatelessWidget {
  const _CenterHeader({
    required this.map,
    required this.body,
    required this.pendingCompletion,
    required this.detailOpen,
    required this.onToggleDetail,
  });

  final StarMap map;
  final BotBody? body;
  final CompletionApproval? pendingCompletion;
  final bool detailOpen;
  final VoidCallback onToggleDetail;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final b = body;

    final String name;
    final String subtitle;
    final (String label, Color color)? statusBadge;
    if (b != null) {
      final chief = b == map.chief;
      final isLocal = state.isLocalBot(b.agent.id);
      final role = chief ? '幕僚长' : (b.agent.role == 'demo' ? '演示星球' : roleNameFor(state.allRoles, b.agent.role));
      name = b.agent.name;
      subtitle = '$role · ${b.celestial.name}';
      statusBadge = pendingCompletion != null
          ? ('已完成 · 待审批', const Color(0xFFB4690E))
          : isLocal
          ? ('本地新建', const Color(0xFF1F63C6))
          : b.working
          ? ('正在处理', s.warning)
          : ('在线', const Color(0xFF1A9E38));
    } else if (!state.connected) {
      name = '幕僚长';
      subtitle = '太阳';
      statusBadge = switch (state.status) {
        _ when state.rejected != null => ('被拒绝', s.danger),
        ConnStatus.connecting => ('连接中', s.warning),
        _ => ('未连接', s.inkSecondary),
      };
    } else {
      name = '幕僚长（空位）';
      subtitle = '太阳';
      statusBadge = ('未配置', s.inkSecondary);
    }

    return SizedBox(
      height: 42,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(14, 0, 10, 0),
        child: Row(
          children: [
            _bodyDot(state, map, b, 15),
            const SizedBox(width: 9),
            Flexible(
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  Flexible(
                    child: Text(
                      name,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600, color: s.ink, letterSpacing: -0.1),
                    ),
                  ),
                  const SizedBox(width: 8),
                  Text(subtitle, style: TextStyle(fontSize: 12, color: s.inkTertiary)),
                  if (statusBadge case (final label, final color)) ...[
                    const SizedBox(width: 8),
                    Container(
                      padding: const EdgeInsets.symmetric(horizontal: 7.5, vertical: 2.5),
                      decoration: BoxDecoration(
                        color: color.withValues(alpha: 0.12),
                        borderRadius: BorderRadius.circular(999),
                      ),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Container(
                            width: 5.5,
                            height: 5.5,
                            decoration: BoxDecoration(color: color, shape: BoxShape.circle),
                          ),
                          const SizedBox(width: 4.5),
                          Text(
                            label,
                            style: TextStyle(fontSize: 10.5, fontWeight: FontWeight.w600, color: color),
                          ),
                        ],
                      ),
                    ),
                  ],
                ],
              ),
            ),
            const SizedBox(width: 8),
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

/// 3B：员工三栏工作大窗顶部的「已完成 · 待审批」验收条（与左中下停泊区、左上角待办共用 AppState 同一数据源）。
class _CompletionApprovalBanner extends StatefulWidget {
  const _CompletionApprovalBanner({required this.item});

  final CompletionApproval item;

  @override
  State<_CompletionApprovalBanner> createState() => _CompletionApprovalBannerState();
}

class _CompletionApprovalBannerState extends State<_CompletionApprovalBanner> {
  CompletionDecision? _busy;

  Future<void> _decide(CompletionDecision decision) async {
    String? note;
    if (decision == CompletionDecision.reject) {
      // 先问要改什么；取消就不打回。
      note = await askReworkNote(context, widget.item);
      if (note == null || !mounted) return;
    }
    setState(() => _busy = decision);
    try {
      await AppScope.read(context)
          .decideCompletionApproval(widget.item, decision, note: (note?.isEmpty ?? true) ? null : note);
    } on RequestError catch (e) {
      if (mounted) showError(context, '审批操作失败：$e');
    } finally {
      if (mounted) setState(() => _busy = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final item = widget.item;
    final canAct = _busy == null && (item.source == CompletionSource.localDemo || state.connected);

    return Container(
      key: ValueKey('agent-completion-banner-${item.agentId}'),
      padding: const EdgeInsets.fromLTRB(16, 12, 16, 12),
      decoration: BoxDecoration(
        gradient: const LinearGradient(
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
          colors: [Color(0x1FF59E0B), Color(0x1410B981)],
        ),
        border: Border(bottom: BorderSide(color: s.edge, width: 0.5)),
      ),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.center,
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Container(
                      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
                      decoration: BoxDecoration(
                        color: const Color(0x24D97706),
                        borderRadius: BorderRadius.circular(999),
                      ),
                      child: const Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Icon(Icons.task_alt_rounded, size: 12, color: Color(0xFFB45309)),
                          SizedBox(width: 4),
                          Text(
                            '已完成 · 待审批',
                            style: TextStyle(fontSize: 10.5, fontWeight: FontWeight.w700, color: Color(0xFFB45309)),
                          ),
                        ],
                      ),
                    ),
                    if (item.source == CompletionSource.localDemo) ...[
                      const SizedBox(width: 6),
                      Text('本地演示', style: TextStyle(fontSize: 10.5, color: s.inkTertiary)),
                    ],
                  ],
                ),
                const SizedBox(height: 5),
                Text(
                  item.taskTitle,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 13.5, fontWeight: FontWeight.w600, color: s.ink),
                ),
                if (item.summary.isNotEmpty) ...[
                  const SizedBox(height: 2),
                  Text(
                    item.summary,
                    maxLines: 2,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(fontSize: 12, color: s.inkSecondary, height: 1.38),
                  ),
                ],
              ],
            ),
          ),
          const SizedBox(width: 14),
          OutlinedButton(
            key: ValueKey('agent-completion-reject-${item.agentId}'),
            onPressed: canAct ? () => _decide(CompletionDecision.reject) : null,
            style: OutlinedButton.styleFrom(
              padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 9),
              visualDensity: VisualDensity.compact,
            ),
            child: _busy == CompletionDecision.reject
                ? SizedBox.square(dimension: 13, child: CircularProgressIndicator(strokeWidth: 2, color: s.ink))
                : const Text('打回修改', style: TextStyle(fontSize: 12, fontWeight: FontWeight.w600)),
          ),
          const SizedBox(width: 8),
          FilledButton(
            key: ValueKey('agent-completion-approve-${item.agentId}'),
            onPressed: canAct ? () => _decide(CompletionDecision.approve) : null,
            style: FilledButton.styleFrom(
              backgroundColor: const Color(0xFF15181F),
              foregroundColor: const Color(0xFFF8FAFC),
              padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 9),
              visualDensity: VisualDensity.compact,
            ),
            child: _busy == CompletionDecision.approve
                ? const SizedBox.square(
                    dimension: 13,
                    child: CircularProgressIndicator(strokeWidth: 2, color: Colors.white),
                  )
                : const Text('审批通过', style: TextStyle(fontSize: 12, fontWeight: FontWeight.w600)),
          ),
        ],
      ),
    );
  }
}

/// 演示星球点开三栏窗时的说明占位（真实员工会直接显示完整的 [ThreadView] 对话流）。
class _DemoApprovalBody extends StatelessWidget {
  const _DemoApprovalBody({required this.body, required this.hasPending});

  final BotBody body;
  final bool hasPending;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 420),
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: GlassInspectorCard(
            padding: const EdgeInsets.all(18),
            margin: EdgeInsets.zero,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  hasPending ? '${body.agent.name} 的交付物正在等待审批' : '${body.agent.name}（演示星球）',
                  style: TextStyle(fontSize: 14, fontWeight: FontWeight.w600, color: s.ink),
                ),
                const SizedBox(height: 6),
                Text(
                  hasPending
                      ? '点上方「审批通过」会让星球解除手动固定并飞回自动轨道；点「打回修改」会让它飞回活跃轨道继续处理，同时左上角待办会实时同步。'
                      : '可在左侧底部直接调节重要度或一键复位，收起窗口（点空白蓝天或按 Esc）即可在天穹上看到星球大小变化，也支持悬停滚轮直接缩放。',
                  style: TextStyle(fontSize: 12, color: s.inkSecondary, height: 1.55),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 只建在本机的 Bot 的中央展示区（主机上的员工走正常的对话区）。
class _LocalBotBody extends StatelessWidget {
  const _LocalBotBody({required this.body, required this.record, required this.hasPending, required this.onRemoved});

  final BotBody body;
  final LocalBotRecord record;
  final bool hasPending;
  final VoidCallback onRemoved;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final r = _role(state, record.agent.role);

    return Center(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 460),
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: GlassInspectorCard(
            padding: const EdgeInsets.all(20),
            margin: EdgeInsets.zero,
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    Expanded(
                      child: Text(
                        '${record.agent.name} · ${record.celestial.name}',
                        style: TextStyle(fontSize: 15, fontWeight: FontWeight.w600, color: s.ink),
                      ),
                    ),
                    Container(
                      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2.5),
                      decoration: BoxDecoration(
                        color: const Color(0x141F63C6),
                        borderRadius: BorderRadius.circular(999),
                      ),
                      child: const Text(
                        '本地新建',
                        style: TextStyle(fontSize: 11, fontWeight: FontWeight.w600, color: Color(0xFF1F63C6)),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 10),
                _kv(s, '员工 ID', record.agent.id),
                _kv(s, '所属岗位', r == null ? record.agent.role : '${r.displayName}（${r.id}）'),
                if (r != null) _kv(s, '模型', r.escalateModel == null ? r.model : '${r.model} → ${r.escalateModel}'),
                if (r?.budgetPerDayUsd case final b?) _kv(s, '每日预算', usd(b)),
                _kv(s, '沙箱工具', record.tools ? '已开启（shell / 文件读写）' : '已关闭'),
                if (record.workspace != null) _kv(s, '工作目录', record.workspace!),
                const SizedBox(height: 10),
                Text(
                  hasPending
                      ? '当前处于演示待审批态，可点上方「审批通过」或「打回修改」体验完整闭环。'
                      : '这个 Bot 只建在本机，主机上没有，所以还不能和它对话：新建时没连上主机、这台设备是“日常”权限，或者主机太旧。连上主机并给这台设备“管理”权限后新建的 Bot 会建在主机上。',
                  style: TextStyle(fontSize: 11.5, color: s.inkSecondary, height: 1.55),
                ),
                const SizedBox(height: 14),
                Row(
                  children: [
                    OutlinedButton.icon(
                      key: ValueKey('remove-local-bot-${record.agent.id}'),
                      onPressed: () {
                        AppScope.read(context).removeLocalBot(record.agent.id);
                        onRemoved();
                      },
                      style: OutlinedButton.styleFrom(
                        foregroundColor: s.danger,
                        side: BorderSide(color: s.danger.withValues(alpha: 0.35)),
                        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(10)),
                      ),
                      icon: const Icon(Icons.delete_outline_rounded, size: 15),
                      label: const Text('移除此本地 Bot', style: TextStyle(fontSize: 12.5)),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 中间区域在未连接或幕僚长空位时的占位：超椭圆提示卡片加底部禁用的输入框骨架。
class _EmptyChatArea extends StatelessWidget {
  const _EmptyChatArea({required this.map});

  final StarMap map;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final host = state.host?.address.origin.host ?? '';
    final disconnected = !state.connected;

    final IconData icon;
    final String title;
    final String detail;
    if (disconnected) {
      if (state.rejected != null) {
        icon = Icons.lock_outline;
        title = '连接被主机拒绝';
        detail = state.rejected!.message;
      } else if (state.status == ConnStatus.connecting) {
        icon = Icons.sync;
        title = '正在连接主机…';
        detail = host.isEmpty ? '正在尝试连接，连上后就能和幕僚长对话。' : '正在连接 $host，连上后就能和幕僚长对话。';
      } else {
        icon = Icons.cloud_off_outlined;
        title = '未连接主机';
        detail = host.isEmpty ? '还没有连上主机，先在设置里检查连接。' : '当前无法连上 $host，连上后就能和幕僚长对话。';
      }
    } else {
      icon = Icons.wb_sunny_outlined;
      title = '还没有配置幕僚长';
      detail = '在主机的 ip.yaml 里给协调岗位写上 coordinator: true（并给它配一位员工），重启主机后就能在这里直接对话。';
    }

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        Expanded(
          child: Center(
            child: ConstrainedBox(
              constraints: const BoxConstraints(maxWidth: 420),
              child: Container(
                margin: const EdgeInsets.symmetric(horizontal: 24),
                padding: const EdgeInsets.all(20),
                decoration: ShapeDecoration(
                  color: const Color(0xD9F4F7FC),
                  shape: RoundedSuperellipseBorder(
                    borderRadius: BorderRadius.circular(18),
                    side: BorderSide(color: s.edge),
                  ),
                ),
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Row(
                      children: [
                        Icon(icon, size: 18, color: disconnected ? s.warning : panelGold),
                        const SizedBox(width: 8),
                        Expanded(
                          child: Text(
                            title,
                            style: TextStyle(fontSize: 15, fontWeight: FontWeight.w600, color: s.ink),
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 8),
                    Text(detail, style: TextStyle(fontSize: 13, color: s.inkSecondary, height: 1.55)),
                    if (!disconnected && map.routingWaiting > 0) ...[
                      const SizedBox(height: 10),
                      Text(
                        '有 ${map.routingWaiting} 条消息等你分派，在“待办”里。',
                        style: TextStyle(fontSize: 12.5, color: s.warning),
                      ),
                    ],
                    if (disconnected) ...[
                      const SizedBox(height: 14),
                      FilledButton.icon(
                        key: const ValueKey('agent-open-settings'),
                        onPressed: () => showSettings(context, tab: SettingsTab.general),
                        icon: const Icon(Icons.settings_outlined, size: 16),
                        label: const Text('前往连接设置'),
                      ),
                    ],
                  ],
                ),
              ),
            ),
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 14),
          child: Container(
            padding: const EdgeInsets.fromLTRB(14, 10, 10, 10),
            decoration: ShapeDecoration(
              color: s.composer.withValues(alpha: 0.72),
              shape: RoundedSuperellipseBorder(
                borderRadius: BorderRadius.circular(16),
                side: BorderSide(color: s.composerRing),
              ),
            ),
            child: Row(
              children: [
                Expanded(
                  child: Text(
                    disconnected ? '连接主机后即可发消息…' : '配置幕僚长后即可发消息…',
                    style: TextStyle(fontSize: 13.5, color: s.inkTertiary),
                  ),
                ),
                Container(
                  width: 28,
                  height: 28,
                  decoration: BoxDecoration(color: s.control, shape: BoxShape.circle),
                  child: Icon(Icons.arrow_upward_rounded, size: 16, color: s.inkTertiary),
                ),
              ],
            ),
          ),
        ),
      ],
    );
  }
}

/// 左栏：顶上原生红绿灯 + 圆形玻璃“+ 新对话”，下面对话列表，底部安静的连接状态与设置入口。
class _AgentThreads extends StatelessWidget {
  const _AgentThreads({
    required this.map,
    required this.body,
    required this.expanded,
    required this.onClose,
    required this.onToggleExpand,
    required this.onOpenThread,
    required this.onNewChat,
  });

  final StarMap map;
  final BotBody? body;
  final bool expanded;
  final VoidCallback onClose;
  final VoidCallback? onToggleExpand;
  final void Function(String threadId) onOpenThread;
  final void Function(String agentId) onNewChat;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final b = body;
    final threads = b == null ? const <AgentThread>[] : agentThreads(state, map, b.agent.id);
    final newSelected = b != null && state.threadId == null && state.currentAgent == b.agent.id;

    Widget row({
      required Key key,
      required String title,
      String? subtitle,
      required bool selected,
      VoidCallback? onTap,
      Widget? trailing,
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
              padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 8),
              child: Row(
                children: [
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          title,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: TextStyle(
                            fontSize: 13,
                            color: onTap == null ? s.inkTertiary : s.ink,
                            fontWeight: selected ? FontWeight.w600 : FontWeight.w500,
                          ),
                        ),
                        if (subtitle != null) ...[
                          const SizedBox(height: 1.5),
                          Text(
                            subtitle,
                            style: TextStyle(
                              fontSize: 11,
                              color: s.inkTertiary,
                              fontFeatures: const [FontFeature.tabularFigures()],
                            ),
                          ),
                        ],
                      ],
                    ),
                  ),
                  if (trailing != null) ...[const SizedBox(width: 6), trailing],
                ],
              ),
            ),
          ),
        ),
      ),
    );

    final emptyHint = b != null ? '还没有和${b.agent.name}的对话。' : (state.connected ? '配置幕僚长后会显示这里的对话。' : '连接主机后会显示这里的对话。');

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SheetDragRegion(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(14, FrostedSheet.innerInset, 10, 0),
            child: SizedBox(
              height: 42,
              child: Row(
                children: [
                  MacTrafficLights(onClose: onClose, onToggleExpand: onToggleExpand, expanded: expanded),
                  const Spacer(),
                  GlassIconButton(
                    key: const ValueKey('agent-new-chat'),
                    icon: Icons.add_rounded,
                    tooltip: '新对话',
                    iconSize: 16,
                    active: newSelected,
                    onPressed: b == null ? null : () => onNewChat(b.agent.id),
                  ),
                ],
              ),
            ),
          ),
        ),
        Padding(
          padding: const EdgeInsets.fromLTRB(20, 6, 12, 6),
          child: Text(
            '${b?.agent.name ?? '幕僚长'}的对话',
            overflow: TextOverflow.ellipsis,
            style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: s.inkTertiary, letterSpacing: 0.3),
          ),
        ),
        Expanded(
          child: threads.isEmpty
              ? Padding(
                  padding: const EdgeInsets.fromLTRB(20, 4, 18, 8),
                  child: Text(emptyHint, style: TextStyle(fontSize: 12.5, color: s.inkTertiary, height: 1.5)),
                )
              : ListView(
                  padding: const EdgeInsets.only(bottom: 8),
                  children: [
                    for (final t in threads)
                      row(
                        key: ValueKey('agent-thread-${t.threadId}'),
                        title: t.title,
                        subtitle: [
                          if (t.atMs > 0) formatTime(t.atMs),
                          if (t.count > 0) '${t.count} 条',
                        ].join(' · ').ifEmptyNull,
                        selected: state.threadId == t.threadId,
                        trailing: t.waiting
                            ? Tooltip(
                                message: '等你处理',
                                child: Container(
                                  width: 6.5,
                                  height: 6.5,
                                  decoration: BoxDecoration(color: s.warning, shape: BoxShape.circle),
                                ),
                              )
                            : null,
                        onTap: () => onOpenThread(t.threadId),
                      ),
                  ],
                ),
        ),
        if (b != null && b != map.chief) _SidebarImportanceControl(body: b, map: map),
        const _SidebarConnectionFoot(),
      ],
    );
  }
}

/// 三栏大窗左侧栏底部的「星球重要度调节 + 一键复位」紧凑玻璃控件（仅非太阳员工星球显示）。
/// 与天穹悬停滚轮共享同一套 [AppState.manualImportanceOverrides] 与预留顺位计算器 [PriorityAllocator]。
class _SidebarImportanceControl extends StatelessWidget {
  const _SidebarImportanceControl({required this.body, required this.map});

  final BotBody body;
  final StarMap map;

  static const _step = 0.10;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final id = body.agent.id;
    final isManual = state.hasManualImportance(id);
    final value = body.importance.clamp(0.0, 1.0);
    final pct = importancePercent(value);

    final trackByAgent = <String, int>{
      for (final b in map.bodies) b.agent.id: state.manualOrbitPins[b.agent.id]?.trackId ?? OrbitTrack.autoFor(b).id,
    };
    final rank = PriorityAllocator.compute(
      bodies: map.bodies,
      trackByAgent: trackByAgent,
      overrides: state.manualImportanceOverrides,
    ).of(id)?.rank;

    void setTo(double next, {bool notifySync = true}) {
      state.setManualImportance(id, next, notifySync: notifySync);
    }

    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 6, 10, 2),
      child: Container(
        key: ValueKey('agent-importance-control-$id'),
        padding: const EdgeInsets.fromLTRB(10, 8, 10, 7),
        decoration: ShapeDecoration(
          color: s.raised.withValues(alpha: 0.76),
          shape: RoundedSuperellipseBorder(
            borderRadius: BorderRadius.circular(12),
            side: BorderSide(color: s.edge, width: 0.7),
          ),
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Text(
                  '重要度',
                  style: TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: s.inkSecondary),
                ),
                const SizedBox(width: 6),
                Container(
                  padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1.5),
                  decoration: BoxDecoration(
                    color: (isManual ? s.accent : s.inkTertiary).withValues(alpha: isManual ? 0.14 : 0.10),
                    borderRadius: BorderRadius.circular(999),
                  ),
                  child: Text(
                    rank != null ? '$pct% · #$rank' : '$pct%',
                    key: ValueKey('agent-importance-value-$id'),
                    style: TextStyle(
                      fontSize: 10.5,
                      fontWeight: FontWeight.w600,
                      color: isManual ? s.accent : s.inkSecondary,
                      fontFeatures: const [FontFeature.tabularFigures()],
                    ),
                  ),
                ),
                const Spacer(),
                Tooltip(
                  message: isManual ? '恢复按任务与预算自动计算的大小' : '当前为自动大小',
                  child: InkWell(
                    key: ValueKey('agent-importance-reset-$id'),
                    onTap: isManual ? () => state.resetManualImportance(id) : null,
                    borderRadius: BorderRadius.circular(6),
                    hoverColor: s.raisedHover,
                    child: Padding(
                      padding: const EdgeInsets.symmetric(horizontal: 5, vertical: 2),
                      child: Row(
                        mainAxisSize: MainAxisSize.min,
                        children: [
                          Icon(
                            Icons.restart_alt_rounded,
                            size: 13,
                            color: isManual ? s.accent : s.inkTertiary.withValues(alpha: 0.55),
                          ),
                          const SizedBox(width: 2),
                          Text(
                            isManual ? '复位' : '自动',
                            style: TextStyle(
                              fontSize: 11,
                              fontWeight: isManual ? FontWeight.w600 : FontWeight.w500,
                              color: isManual ? s.accent : s.inkTertiary.withValues(alpha: 0.65),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 5),
            Row(
              children: [
                _ImportanceStepButton(
                  key: ValueKey('agent-importance-dec-$id'),
                  icon: Icons.remove_rounded,
                  tooltip: '调小星球（降低重要度）',
                  onTap: value > 0.001 ? () => setTo(value - _step) : null,
                ),
                Expanded(
                  child: SliderTheme(
                    data: SliderTheme.of(context).copyWith(
                      trackHeight: 2.6,
                      activeTrackColor: isManual ? s.accent : s.inkSecondary.withValues(alpha: 0.55),
                      inactiveTrackColor: s.edge,
                      thumbColor: Colors.white,
                      overlayColor: s.accent.withValues(alpha: 0.12),
                      thumbShape: const RoundSliderThumbShape(enabledThumbRadius: 5.5, elevation: 1.5),
                      overlayShape: const RoundSliderOverlayShape(overlayRadius: 11),
                    ),
                    child: Slider(
                      key: ValueKey('agent-importance-slider-$id'),
                      value: value,
                      onChanged: (v) => setTo(v, notifySync: false),
                      onChangeEnd: (_) => state.syncManualImportance(id),
                    ),
                  ),
                ),
                _ImportanceStepButton(
                  key: ValueKey('agent-importance-inc-$id'),
                  icon: Icons.add_rounded,
                  tooltip: '放大星球（提高重要度）',
                  onTap: value < 0.999 ? () => setTo(value + _step) : null,
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

class _ImportanceStepButton extends StatelessWidget {
  const _ImportanceStepButton({super.key, required this.icon, required this.tooltip, required this.onTap});

  final IconData icon;
  final String tooltip;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final enabled = onTap != null;
    return Tooltip(
      message: tooltip,
      child: Material(
        color: Colors.transparent,
        shape: const CircleBorder(),
        child: InkWell(
          customBorder: const CircleBorder(),
          hoverColor: s.raisedHover,
          onTap: onTap,
          child: SizedBox.square(
            dimension: 22,
            child: Icon(icon, size: 14, color: enabled ? s.inkSecondary : s.inkTertiary.withValues(alpha: 0.4)),
          ),
        ),
      ),
    );
  }
}

/// 左栏最底下的连接状态与设置快捷入口（窗口打开时随时可看、可点）。
class _SidebarConnectionFoot extends StatelessWidget {
  const _SidebarConnectionFoot();

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final (String label, Color color) = switch (state.status) {
      _ when state.rejected != null => ('被拒绝', s.danger),
      ConnStatus.open => ('已连接', const Color(0xFF28C840)),
      ConnStatus.connecting => ('连接中', s.warning),
      ConnStatus.closed => ('未连接', s.danger),
    };
    final host = state.host?.address.origin.host ?? '';
    return Padding(
      padding: const EdgeInsets.fromLTRB(10, 4, 10, 10),
      child: ClipRSuperellipse(
        borderRadius: BorderRadius.circular(11),
        child: InkWell(
          hoverColor: s.raisedHover,
          onTap: () => showSettings(context),
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 7),
            child: Row(
              children: [
                Container(
                  width: 6.5,
                  height: 6.5,
                  decoration: BoxDecoration(
                    color: color,
                    shape: BoxShape.circle,
                    boxShadow: [BoxShadow(color: color.withValues(alpha: 0.45), blurRadius: 5)],
                  ),
                ),
                const SizedBox(width: 7),
                Expanded(
                  child: Text(
                    host.isEmpty ? label : '$label · $host',
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(fontSize: 11.5, color: s.inkSecondary, fontWeight: FontWeight.w500),
                  ),
                ),
                Tooltip(
                  message: '连接与设置',
                  child: Icon(Icons.tune_rounded, size: 14, color: s.inkTertiary),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 右栏的三页（对应 dusk 的 Details / Computer / Advanced）。
enum _DetailTab {
  overview('概况'),
  doing('在做什么'),
  advanced('高级');

  const _DetailTab(this.label);
  final String label;
}

/// 右栏：Apple Inspector 检查器（顶栏 Segmented Control 与左/中两栏对齐同一水平中线）。
class _AgentDetail extends StatefulWidget {
  const _AgentDetail({
    super.key,
    required this.map,
    required this.body,
    required this.onSelectAgent,
    required this.onOpenThread,
  });

  final StarMap map;
  final BotBody? body;
  final void Function(String agentId) onSelectAgent;
  final void Function(String threadId) onOpenThread;

  @override
  State<_AgentDetail> createState() => _AgentDetailState();
}

class _AgentDetailState extends State<_AgentDetail> {
  var _tab = _DetailTab.overview;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final b = widget.body;
    final map = widget.map;

    final Widget body = switch (_tab) {
      _DetailTab.overview => AgentSheetContent(
        map: map,
        agentId: b?.agent.id,
        onSelectAgent: widget.onSelectAgent,
        onOpenThread: widget.onOpenThread,
        showHeader: false,
        hideApprovalsInThread: state.threadId,
        footer: b != null
            ? Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (state.isAdmin && state.connected)
                    _AgentSchedulesBox(agentId: b.agent.id, roleId: b.agent.role, onOpenThread: widget.onOpenThread),
                  _ModelBox(roleId: b.agent.role),
                ],
              )
            : _card(s, [_kv(s, '模型', '—'), _kv(s, '今天岗位花费', '—'), _kv(s, '今日缓存命中', '—')]),
      ),
      _DetailTab.doing => _Doing(body: b, onOpenThread: widget.onOpenThread),
      _DetailTab.advanced => _Advanced(roleId: b?.agent.role, agentId: b?.agent.id),
    };

    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        SheetDragRegion(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(10, FrostedSheet.innerInset, 12, 0),
            child: SizedBox(
              height: 42,
              child: Center(
                child: GlassSegmentedControl<_DetailTab>(
                  value: _tab,
                  onChanged: (t) => setState(() => _tab = t),
                  items: [
                    for (final t in _DetailTab.values) (value: t, label: t.label, key: ValueKey('agent-tab-${t.name}')),
                  ],
                ),
              ),
            ),
          ),
        ),
        const SizedBox(height: 2),
        Expanded(child: body),
      ],
    );
  }
}

/// 一块右栏 Inspector 玻璃卡片。
Widget _card(Shell s, List<Widget> children) => GlassInspectorCard(
  child: Column(crossAxisAlignment: CrossAxisAlignment.stretch, children: children),
);

Widget _kv(Shell s, String k, String v) => Padding(
  padding: const EdgeInsets.symmetric(vertical: 3),
  child: Row(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Text(k, style: TextStyle(fontSize: 12, color: s.ink)),
      const SizedBox(width: 12),
      Expanded(
        child: Text(
          v,
          textAlign: TextAlign.right,
          style: TextStyle(
            fontSize: 12,
            color: s.inkSecondary,
            fontWeight: FontWeight.w500,
            fontFeatures: const [FontFeature.tabularFigures()],
          ),
        ),
      ),
    ],
  ),
);

Role? _role(AppState state, String id) {
  for (final r in state.allRoles) {
    if (r.id == id) return r;
  }
  return null;
}

extension on String {
  String? get ifEmptyNull => isEmpty ? null : this;
}

String _cacheHitText(Usage? u) {
  if (u == null || u.promptTokens == 0) return '还没有调用';
  final rate = (100 * u.cachedTokens / u.promptTokens).round();
  return '$rate%（${u.cachedTokens} / ${u.promptTokens}）';
}

/// 概况里这位员工 / 岗位相关的定时任务（仅“管理”权限可见，可立即运行）。
class _AgentSchedulesBox extends StatefulWidget {
  const _AgentSchedulesBox({required this.agentId, required this.roleId, required this.onOpenThread});

  final String agentId;
  final String roleId;
  final void Function(String threadId) onOpenThread;

  @override
  State<_AgentSchedulesBox> createState() => _AgentSchedulesBoxState();
}

class _AgentSchedulesBoxState extends State<_AgentSchedulesBox> {
  List<Schedule>? _schedules;
  final _running = <String>{};

  @override
  void initState() {
    super.initState();
    unawaited(_load());
  }

  @override
  void didUpdateWidget(_AgentSchedulesBox old) {
    super.didUpdateWidget(old);
    if (old.agentId != widget.agentId || old.roleId != widget.roleId) {
      unawaited(_load());
    }
  }

  Future<void> _load() async {
    final state = AppScope.read(context);
    if (!state.connected || !state.isAdmin) return;
    try {
      final all = await state.client.listSchedules();
      if (!mounted) return;
      final mine = all.where((s) {
        final to = s.to;
        if (to == null) return false;
        if (to.agent != null) return to.agent == widget.agentId;
        return to.role == widget.roleId;
      }).toList();
      setState(() => _schedules = mine);
    } on RequestError {
      // 权限变动、主机旧版本或断开时不打断主面板。
    }
  }

  Future<void> _runNow(Schedule sc) async {
    final state = AppScope.read(context);
    setState(() => _running.add(sc.id));
    try {
      final r = await state.client.runSchedule(sc.id);
      await _load();
      if (!mounted) return;
      widget.onOpenThread(r.message.threadId);
    } on RequestError catch (e) {
      if (mounted) showError(context, '运行失败：$e');
    } finally {
      if (mounted) setState(() => _running.remove(sc.id));
    }
  }

  @override
  Widget build(BuildContext context) {
    final list = _schedules;
    if (list == null || list.isEmpty) return const SizedBox.shrink();
    final s = Shell.of(context);
    return _card(s, [
      Text(
        '定时任务',
        style: TextStyle(fontSize: 11, fontWeight: FontWeight.w600, color: s.inkTertiary, letterSpacing: 0.3),
      ),
      const SizedBox(height: 6),
      for (final sc in list)
        Padding(
          padding: const EdgeInsets.only(bottom: 6),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      sc.description.isEmpty ? sc.id : sc.description,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(fontSize: 12, color: s.ink, fontWeight: FontWeight.w500),
                    ),
                    Text(
                      !sc.enabled
                          ? '${sc.cron} · 已停用'
                          : sc.nextRunMs != null
                          ? '${sc.cron} · 下次 ${formatTime(sc.nextRunMs!)}'
                          : sc.cron,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: TextStyle(fontSize: 11, color: s.inkTertiary),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 6),
              TextButton(
                key: ValueKey('agent-schedule-run-${sc.id}'),
                style: TextButton.styleFrom(
                  visualDensity: VisualDensity.compact,
                  padding: const EdgeInsets.symmetric(horizontal: 8),
                ),
                onPressed: _running.contains(sc.id) ? null : () => _runNow(sc),
                child: Text(_running.contains(sc.id) ? '运行中…' : '立即运行', style: const TextStyle(fontSize: 11.5)),
              ),
            ],
          ),
        ),
    ]);
  }
}

/// 概况最底下：岗位的模型、今天的花费和缓存命中（带简洁的算力进度条）。
class _ModelBox extends StatelessWidget {
  const _ModelBox({required this.roleId});

  final String roleId;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final r = _role(state, roleId);
    if (r == null) return const SizedBox.shrink();
    final spent = state.spend?.roles[roleId];
    final budget = r.budgetPerDayUsd;
    final over = spent != null && budget != null && budget > 0 && spent >= budget;
    final tokens = state.spend?.tokens[roleId];
    return _card(s, [
      Text(
        '模型与算力',
        style: TextStyle(fontSize: 11, fontWeight: FontWeight.w600, color: s.inkTertiary, letterSpacing: 0.3),
      ),
      const SizedBox(height: 5),
      _kv(s, '模型', r.escalateModel == null ? r.model : '${r.model} → ${r.escalateModel}'),
      _kv(s, '今天岗位花费', spent == null ? '—' : (budget == null ? usd(spent) : '${usd(spent)} / ${usd(budget)}')),
      if (over)
        Padding(
          padding: const EdgeInsets.only(top: 2, bottom: 2),
          child: Text(
            '已用满，UTC 0 点后恢复',
            textAlign: TextAlign.right,
            style: TextStyle(fontSize: 11, color: s.danger, fontWeight: FontWeight.w600),
          ),
        ),
      if (spent != null && budget != null && budget > 0)
        Padding(
          padding: const EdgeInsets.only(top: 5, bottom: 5),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(999),
            child: LinearProgressIndicator(
              value: (spent / budget).clamp(0.0, 1.0),
              minHeight: 4,
              color: over ? s.danger : s.accent,
              backgroundColor: s.inset,
            ),
          ),
        ),
      _kv(s, '今日缓存命中', _cacheHitText(tokens)),
    ]);
  }
}

/// 在做什么：手上的任务（可直接改状态 / 新建）、派出去的任务、最近报告、正在处理和上一轮失败。
class _Doing extends StatefulWidget {
  const _Doing({required this.body, required this.onOpenThread});

  final BotBody? body;
  final void Function(String threadId) onOpenThread;

  @override
  State<_Doing> createState() => _DoingState();
}

class _DoingState extends State<_Doing> {
  final _saving = <String>{};

  Future<void> _setStatus(Task t, String status) async {
    if (status == t.status) return;
    setState(() => _saving.add(t.id));
    try {
      await AppScope.read(context).updateTask(t, status);
    } on RequestError catch (e) {
      if (mounted) showError(context, '改状态失败：$e');
    } finally {
      if (mounted) setState(() => _saving.remove(t.id));
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final b = widget.body;
    Widget heading(String text, {Widget? trailing}) => Padding(
      padding: const EdgeInsets.fromLTRB(4, 6, 2, 5),
      child: Row(
        children: [
          Expanded(
            child: Text(
              text,
              style: TextStyle(fontSize: 11, color: s.inkTertiary, fontWeight: FontWeight.w600, letterSpacing: 0.3),
            ),
          ),
          ?trailing,
        ],
      ),
    );

    if (b == null) {
      return ListView(
        padding: const EdgeInsets.fromLTRB(10, 4, 12, 12),
        children: [
          _card(s, [_kv(s, '现在', state.connected ? '空位' : '未连接')]),
          heading('手上的任务'),
          _card(s, [Text('暂无。', style: TextStyle(fontSize: 12, color: s.inkTertiary))]),
          heading('最近报告'),
          _card(s, [Text('暂无。', style: TextStyle(fontSize: 12, color: s.inkTertiary))]),
        ],
      );
    }

    final id = b.agent.id;
    int order(Task t) => t.isOpen ? 0 : 1;
    final mine = state.tasks.where((t) => t.assignee == id).toList()
      ..sort((a, b) => order(a) != order(b) ? order(a) - order(b) : b.updatedAtMs - a.updatedAtMs);
    final sent = state.tasks.where((t) => t.createdByAgent == id && t.isOpen).toList();
    final reports = state.reports.where((r) => r.agentId == id).take(10).toList();
    final failure = state.agentFailures[id];

    Widget taskRow(Task t) {
      final thread = t.threadId;
      final busy = _saving.contains(t.id);
      final c = t.status == 'blocked'
          ? s.danger
          : t.isOpen
          ? s.accent
          : s.inkTertiary;
      return Padding(
        padding: const EdgeInsets.symmetric(vertical: 3.5),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            Expanded(
              child: InkWell(
                borderRadius: BorderRadius.circular(8),
                onTap: thread == null ? null : () => widget.onOpenThread(thread),
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 3),
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(t.title, style: TextStyle(fontSize: 12, color: s.ink, height: 1.35)),
                      ),
                      if (thread != null) Icon(Icons.chevron_right_rounded, size: 15, color: s.inkTertiary),
                    ],
                  ),
                ),
              ),
            ),
            const SizedBox(width: 4),
            if (busy)
              const SizedBox.square(dimension: 14, child: CircularProgressIndicator(strokeWidth: 2))
            else
              DropdownButtonHideUnderline(
                child: DropdownButton<String>(
                  key: ValueKey('agent-task-status-${t.id}'),
                  value: taskStatusText.containsKey(t.status) ? t.status : null,
                  hint: Text(taskStatusLabel(t.status), style: TextStyle(fontSize: 11.5, color: c)),
                  isDense: true,
                  dropdownColor: s.menu,
                  borderRadius: BorderRadius.circular(Radii.menu),
                  icon: Icon(Icons.expand_more_rounded, size: 14, color: c),
                  style: TextStyle(fontSize: 11.5, color: c, fontWeight: FontWeight.w600),
                  items: [
                    for (final MapEntry(key: v, value: label) in taskStatusText.entries)
                      DropdownMenuItem(
                        value: v,
                        child: Text(label, style: TextStyle(fontSize: 12, color: s.ink)),
                      ),
                  ],
                  onChanged: !state.connected ? null : (v) => v == null ? null : _setStatus(t, v),
                ),
              ),
          ],
        ),
      );
    }

    return ListView(
      padding: const EdgeInsets.fromLTRB(10, 4, 12, 12),
      children: [
        _card(s, [_kv(s, '现在', b.working ? '正在处理一轮' : '空闲'), if (failure != null) _kv(s, '上一轮失败', failure)]),
        // 主机估的进度和依据（F-24）；旧主机没有。
        if (state.progress case final board? when showsHeadBar(board.of(id)))
          _card(s, [ProgressDetails(progress: board.of(id)!, board: board, ink: s.ink)]),
        heading(
          '手上的任务',
          trailing: TextButton.icon(
            key: const ValueKey('agent-new-task'),
            style: TextButton.styleFrom(
              visualDensity: VisualDensity.compact,
              padding: const EdgeInsets.symmetric(horizontal: 6),
            ),
            onPressed: state.connected ? () => showNewTask(context, initialAssignee: id) : null,
            icon: const Icon(Icons.add_rounded, size: 14),
            label: const Text('新建', style: TextStyle(fontSize: 11.5)),
          ),
        ),
        if (mine.isEmpty)
          _card(s, [Text('没有。', style: TextStyle(fontSize: 12, color: s.inkTertiary))])
        else
          _card(s, [for (final t in mine.take(20)) taskRow(t)]),
        if (sent.isNotEmpty) ...[
          heading('派给别人、还没结束的'),
          _card(s, [for (final t in sent) taskRow(t)]),
        ],
        heading('最近报告'),
        if (reports.isEmpty)
          _card(s, [Text('还没有报告。', style: TextStyle(fontSize: 12, color: s.inkTertiary))])
        else
          _card(s, [
            for (final r in reports)
              InkWell(
                key: ValueKey('agent-report-${r.path}'),
                borderRadius: BorderRadius.circular(8),
                onTap: () => openReport(context, r.path, hint: id),
                child: Padding(
                  padding: const EdgeInsets.symmetric(vertical: 5),
                  child: Row(
                    children: [
                      Icon(Icons.description_outlined, size: 15, color: s.accent),
                      const SizedBox(width: 8),
                      Expanded(
                        child: Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            Text(
                              r.fileName,
                              maxLines: 1,
                              overflow: TextOverflow.ellipsis,
                              style: TextStyle(fontSize: 12, color: s.ink, fontWeight: FontWeight.w500),
                            ),
                            Text(formatTime(r.modifiedMs), style: TextStyle(fontSize: 11, color: s.inkTertiary)),
                          ],
                        ),
                      ),
                      Icon(Icons.chevron_right_rounded, size: 15, color: s.inkTertiary),
                    ],
                  ),
                ),
              ),
          ]),
      ],
    );
  }
}

/// 高级：岗位说明、模型、预算、岗位记忆。这台设备有“管理”权限时能在线改岗位和员工（m2-spec D-42、D-44）。
class _Advanced extends StatelessWidget {
  const _Advanced({required this.roleId, this.agentId});

  final String? roleId;
  final String? agentId;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final s = Shell.of(context);
    final r = roleId == null ? null : _role(state, roleId!);
    // 只建在本机的 Bot 和它的岗位不在主机上，改不了。
    final hostRole = r == null ? null : state.roles.where((x) => x.id == r.id).firstOrNull;
    final hostAgent = agentId == null ? null : state.agents.where((a) => a.id == agentId).firstOrNull;
    final canEdit = state.canEditOrg && (hostRole != null || hostAgent != null);
    return ListView(
      padding: const EdgeInsets.fromLTRB(10, 4, 12, 12),
      children: [
        if (canEdit)
          _card(s, [
            Wrap(
              spacing: 8,
              runSpacing: 6,
              children: [
                if (hostRole != null)
                  TextButton.icon(
                    key: const ValueKey('advanced-edit-role'),
                    onPressed: () => showRoleEditor(context, hostRole),
                    icon: const Icon(Icons.tune_rounded, size: 16),
                    label: const Text('编辑岗位'),
                  ),
                if (hostAgent != null)
                  TextButton.icon(
                    key: const ValueKey('advanced-edit-agent'),
                    onPressed: () => showAgentEditor(context, hostAgent),
                    icon: const Icon(Icons.badge_outlined, size: 16),
                    label: const Text('编辑员工'),
                  ),
              ],
            ),
          ]),
        _card(s, [
          _kv(s, '岗位', r?.displayName ?? roleId ?? '幕僚长'),
          _kv(s, '模型', r?.model ?? '—'),
          if (r?.escalateModel case final m?) _kv(s, '难题升级到', m),
          _kv(s, '每天预算', r?.budgetPerDayUsd == null ? '—' : usd(r!.budgetPerDayUsd!)),
        ]),
        if (r != null && r.description.trim().isNotEmpty)
          _card(s, [
            Text(
              '岗位说明',
              style: TextStyle(fontSize: 11, fontWeight: FontWeight.w600, color: s.inkTertiary, letterSpacing: 0.3),
            ),
            const SizedBox(height: 6),
            SelectableText(r.description.trim(), style: TextStyle(fontSize: 12, color: s.inkSecondary, height: 1.55)),
          ]),
        if (state.isAdmin)
          _card(s, [
            InkWell(
              borderRadius: BorderRadius.circular(8),
              onTap: () => showSettings(context, tab: SettingsTab.memory),
              child: Row(
                children: [
                  Expanded(
                    child: Text(
                      '岗位记忆',
                      style: TextStyle(fontSize: 12.5, color: s.ink, fontWeight: FontWeight.w500),
                    ),
                  ),
                  Icon(Icons.chevron_right_rounded, size: 16, color: s.inkTertiary),
                ],
              ),
            ),
          ]),
        Padding(
          padding: const EdgeInsets.only(top: 6, left: 6, right: 6),
          child: Text(
            canEdit
                ? '在这里改的岗位和员工存在主机上，不写回 ip.yaml；改了会重启相关的员工，他们有人在干活时主机会拒绝。'
                : '名字、岗位说明、模型、预算和能用的工具写在主机的 ip.yaml 里，改了重启主机生效；'
                      '这台设备有“管理”权限时也能在这里在线改。',
            style: TextStyle(fontSize: 11.5, color: s.inkTertiary, height: 1.5),
          ),
        ),
      ],
    );
  }
}
