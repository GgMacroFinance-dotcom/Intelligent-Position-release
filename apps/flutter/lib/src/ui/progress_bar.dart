// 星球头顶的进度条（backlog F-24，m2-spec D-43）：手机星图页和桌面外壳的天穹共用。
// 估得出的画比例，在做的随时间往前走；估不出的画来回走的一小段；在等你、受阻、停着的换颜色、不动。

import 'dart:math' as math;

import 'package:flutter/material.dart';

import '../progress.dart';

/// 各状态的颜色。
Color progressColor(ProgressPhase phase) => switch (phase) {
  ProgressPhase.working => const Color(0xFF34D399),
  ProgressPhase.queued => const Color(0xFF94A3B8),
  ProgressPhase.waitingHuman => const Color(0xFFFACC15),
  ProgressPhase.waitingColleague => const Color(0xFF60A5FA),
  ProgressPhase.blocked || ProgressPhase.failed => const Color(0xFFF87171),
  ProgressPhase.stalled => const Color(0xFFA8A29E),
  ProgressPhase.idle || ProgressPhase.unknown => const Color(0xFF94A3B8),
};

/// 头顶要不要画：有一项估算，且不是空闲。
bool showsHeadBar(AgentProgress? p) => p != null && p.headline != null && p.phase != ProgressPhase.idle;

/// 进度条旁边的一行字：“60% · 约 3 分钟”“在等你 · 60%”“在做 · 还估不出”。
String progressCaption(AgentProgress p, ProgressBoard board, int deviceNowMs) {
  final e = p.headline;
  if (e == null) return p.phase.label;
  final f = fractionAt(e, board, deviceNowMs);
  final pct = f == null ? null : '${(f * 100).round()}%';
  if (p.phase.moving) {
    final eta = etaText(e, board, deviceNowMs);
    return [pct ?? '还估不出', ?eta].join(' · ');
  }
  return [p.phase.label, ?pct].join(' · ');
}

/// 一根细进度条。[t] 是动画时钟（秒），只用来画估不出时来回走的那一小段。
class HeadProgressBar extends StatelessWidget {
  const HeadProgressBar({
    super.key,
    required this.progress,
    required this.board,
    required this.deviceNowMs,
    required this.t,
    this.width = 40,
    this.height = 4,
  });

  final AgentProgress progress;
  final ProgressBoard board;
  final int deviceNowMs;
  final double t;
  final double width;
  final double height;

  @override
  Widget build(BuildContext context) {
    final e = progress.headline!;
    final fraction = fractionAt(e, board, deviceNowMs);
    return Semantics(
      label: '进度：${progressCaption(progress, board, deviceNowMs)}',
      child: CustomPaint(
        size: Size(width, height),
        painter: _BarPainter(
          fraction: fraction,
          color: progressColor(progress.phase),
          moving: progress.phase.moving,
          t: t,
        ),
      ),
    );
  }
}

class _BarPainter extends CustomPainter {
  _BarPainter({required this.fraction, required this.color, required this.moving, required this.t});

  final double? fraction;
  final Color color;
  final bool moving;
  final double t;

  @override
  void paint(Canvas canvas, Size size) {
    final r = Radius.circular(size.height / 2);
    final track = RRect.fromRectAndRadius(Offset.zero & size, r);
    // 深色底上的白线，带一圈暗边，在亮的行星前面也看得清。
    canvas.drawRRect(track.inflate(0.8), Paint()..color = const Color(0x990A1432));
    canvas.drawRRect(track, Paint()..color = Colors.white.withValues(alpha: 0.16));
    final f = fraction;
    final paint = Paint()..color = color;
    if (f != null) {
      if (f <= 0) return;
      canvas.drawRRect(
        RRect.fromRectAndRadius(Rect.fromLTWH(0, 0, size.width * f.clamp(0.0, 1.0), size.height), r),
        paint,
      );
      return;
    }
    if (!moving) {
      // 估不出、也没在做：整条淡淡地染上状态色。
      canvas.drawRRect(track, Paint()..color = color.withValues(alpha: 0.45));
      return;
    }
    // 估不出、在做：一小段来回走。
    final seg = size.width * 0.35;
    final x = (math.sin(t * 2.4) * 0.5 + 0.5) * (size.width - seg);
    canvas.drawRRect(RRect.fromRectAndRadius(Rect.fromLTWH(x, 0, seg, size.height), r), paint);
  }

  @override
  bool shouldRepaint(_BarPainter old) =>
      old.fraction != fraction ||
      old.color != color ||
      old.moving != moving ||
      (fraction == null && moving && old.t != t);
}

/// 气泡、员工面板里的“手上的事”：头顶那一项和每个没结束的任务，各带进度条、预计时刻和估算依据。
/// [ink] 是正文颜色，深色气泡里用白色、浅色面板里用深色。
class ProgressDetails extends StatelessWidget {
  const ProgressDetails({super.key, required this.progress, required this.board, this.ink = Colors.white});

  final AgentProgress progress;
  final ProgressBoard board;
  final Color ink;

  @override
  Widget build(BuildContext context) {
    final now = DateTime.now().millisecondsSinceEpoch;
    final head = progress.headline;
    // 头顶那一项是任务时它也在任务列表里，不重复列。
    final items = [
      if (head != null && !head.isTask) head,
      ...progress.tasks,
      if (head != null && head.isTask && !progress.tasks.any((t) => t.taskId == head.taskId)) head,
    ];
    return Column(
      key: ValueKey('progress-details-${progress.agentId}'),
      crossAxisAlignment: CrossAxisAlignment.start,
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          '手上的事 · ${progress.phase.label}',
          style: TextStyle(fontSize: 12.5, fontWeight: FontWeight.w600, color: ink),
        ),
        const SizedBox(height: 6),
        for (final e in items) _EstimateRow(estimate: e, board: board, nowMs: now, ink: ink),
      ],
    );
  }
}

class _EstimateRow extends StatelessWidget {
  const _EstimateRow({required this.estimate, required this.board, required this.nowMs, required this.ink});

  final ProgressEstimate estimate;
  final ProgressBoard board;
  final int nowMs;
  final Color ink;

  @override
  Widget build(BuildContext context) {
    final e = estimate;
    final f = fractionAt(e, board, nowMs);
    final eta = etaText(e, board, nowMs);
    final steps = e.stepsExpected == null ? '第 ${e.stepsDone} 步' : '${e.stepsDone}/${e.stepsExpected} 步';
    final facts = [e.phase.label, if (f != null) '${(f * 100).round()}%', steps, ?eta].join(' · ');
    return Padding(
      padding: const EdgeInsets.only(bottom: 8),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              Expanded(
                child: Text(
                  e.title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(fontSize: 12, color: ink.withValues(alpha: 0.9)),
                ),
              ),
              const SizedBox(width: 8),
              HeadProgressBar(
                progress: AgentProgress(agentId: '', phase: e.phase, headline: e, tasks: const []),
                board: board,
                deviceNowMs: nowMs,
                t: nowMs / 1000,
                width: 56,
              ),
            ],
          ),
          const SizedBox(height: 2),
          Text(
            facts,
            style: TextStyle(
              fontSize: 11,
              color: progressColor(e.phase),
              fontFeatures: const [FontFeature.tabularFigures()],
            ),
          ),
          Text(e.basis, style: TextStyle(fontSize: 11, color: ink.withValues(alpha: 0.55), height: 1.35)),
        ],
      ),
    );
  }
}
