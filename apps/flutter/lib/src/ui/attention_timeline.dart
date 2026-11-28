// “接下来要你看的”时间轴（backlog F-24，m2-spec D-43）：主机估出的、要人看的时间点按时刻排。
// 手机的待办页和桌面外壳的待办共用；别处已经列了的（员工提问、验收卡片、受阻的任务、等你分派的消息）不重复。

import 'package:flutter/material.dart';

import '../progress.dart';
import 'progress_bar.dart' show progressColor;

/// 每种时间点的颜色：已经发生的偏红，估出来的偏蓝绿。
Color attentionColor(AttentionKind kind) => switch (kind) {
  AttentionKind.failed || AttentionKind.overdue || AttentionKind.dueRisk => const Color(0xFFEF4444),
  AttentionKind.stalled => progressColor(ProgressPhase.stalled),
  AttentionKind.stepLimit || AttentionKind.budget || AttentionKind.due => const Color(0xFFF59E0B),
  AttentionKind.likelyQuestion => const Color(0xFFEAB308),
  AttentionKind.result || AttentionKind.reply => const Color(0xFF10B981),
  _ => const Color(0xFF64748B),
};

class AttentionTimeline extends StatelessWidget {
  const AttentionTimeline({
    super.key,
    required this.board,
    required this.items,
    required this.onOpen,
    this.max,
    this.ink = const Color(0xFF1A1715),
    this.muted = const Color(0xFF6B6460),
    this.dense = false,
  });

  final ProgressBoard board;
  final List<AttentionItem> items;

  /// 点了一项：去它的对话（有的话）。
  final void Function(AttentionItem item) onOpen;

  /// 最多列几项；省略时全列。
  final int? max;
  final Color ink;
  final Color muted;

  /// 桌面待办里的紧凑样子：一项一行。
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final now = DateTime.now();
    final shown = max == null ? items : items.take(max!).toList();
    return Column(
      key: const ValueKey('attention-timeline'),
      crossAxisAlignment: CrossAxisAlignment.stretch,
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final (i, item) in shown.indexed)
          _Row(
            key: ValueKey('attention-$i-${item.kind.wire}-${item.taskId ?? item.threadId ?? ''}'),
            item: item,
            time: attentionTimeText(item, board, now),
            onTap: item.threadId == null ? null : () => onOpen(item),
            ink: ink,
            muted: muted,
            dense: dense,
          ),
        if (max != null && items.length > max!)
          Padding(
            padding: const EdgeInsets.only(top: 2, left: 4),
            child: Text('还有 ${items.length - max!} 项', style: TextStyle(fontSize: 11, color: muted)),
          ),
      ],
    );
  }
}

class _Row extends StatelessWidget {
  const _Row({
    super.key,
    required this.item,
    required this.time,
    required this.onTap,
    required this.ink,
    required this.muted,
    required this.dense,
  });

  final AttentionItem item;
  final String time;
  final VoidCallback? onTap;
  final Color ink;
  final Color muted;
  final bool dense;

  @override
  Widget build(BuildContext context) {
    final color = attentionColor(item.kind);
    final chance = likelihoodText(item);
    return InkWell(
      borderRadius: BorderRadius.circular(8),
      onTap: onTap,
      child: Padding(
        padding: EdgeInsets.symmetric(vertical: dense ? 3 : 6, horizontal: 4),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            SizedBox(
              width: dense ? 76 : 92,
              child: Text(
                time,
                style: TextStyle(
                  fontSize: dense ? 11 : 12,
                  fontWeight: FontWeight.w600,
                  color: time == '现在' ? color : ink,
                  fontFeatures: const [FontFeature.tabularFigures()],
                ),
              ),
            ),
            Container(
              margin: EdgeInsets.only(top: dense ? 4 : 5, right: 7),
              width: 6,
              height: 6,
              decoration: BoxDecoration(shape: BoxShape.circle, color: color),
            ),
            Expanded(
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    item.text,
                    maxLines: dense ? 1 : 3,
                    overflow: TextOverflow.ellipsis,
                    style: TextStyle(fontSize: dense ? 11.5 : 13, color: ink, height: 1.35),
                  ),
                  if (chance != null)
                    Text(
                      chance,
                      key: const ValueKey('attention-chance'),
                      style: TextStyle(fontSize: 10.5, color: muted),
                    ),
                ],
              ),
            ),
            if (onTap != null && !dense) Icon(Icons.chevron_right_rounded, size: 16, color: muted),
          ],
        ),
      ),
    );
  }
}
