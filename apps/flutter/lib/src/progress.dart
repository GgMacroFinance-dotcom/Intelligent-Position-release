// 员工的任务进度和“要人看”的时间点（backlog F-24，docs/ip/progress-design.md，m2-spec D-43）。
// 主机 `list_progress` 回的表换成界面用的样子；两次拉取之间按预计做完的时刻把进度条往前推。
//
// 主机的时钟和这台设备的不一定一致：表里的时刻都按主机的算，显示前用 [ProgressBoard.toDevice] 换成本机的时刻。

import 'dart:math' as math;

import 'protocol/models.dart' show Json;
import 'protocol/wire.dart' as w;

export 'protocol/wire.dart' show AttentionKind, ProgressPhase;

/// 没做完时进度最多显示到这里（和主机一致）。
const maxFraction = 0.95;

/// 一项估算：一个任务，或没挂任务的对话里的这一轮。估不出的字段为空，[basis] 写为什么。
class ProgressEstimate {
  const ProgressEstimate({
    required this.isTask,
    required this.threadId,
    required this.taskId,
    required this.title,
    required this.phase,
    required this.startedMs,
    required this.stepsDone,
    required this.stepsExpected,
    required this.fraction,
    required this.etaMs,
    required this.etaLateMs,
    required this.samples,
    required this.basis,
  });

  factory ProgressEstimate.fromWire(w.Estimate e) => ProgressEstimate(
    isTask: e.scope == w.EstimateScope.task,
    threadId: e.threadId.isEmpty ? null : e.threadId,
    taskId: e.taskId,
    title: e.title,
    phase: e.phase,
    startedMs: e.startedMs.toInt(),
    stepsDone: e.stepsDone.toInt(),
    stepsExpected: e.stepsExpected?.toInt(),
    fraction: e.fraction?.toDouble(),
    etaMs: e.etaMs?.toInt(),
    etaLateMs: e.etaLateMs?.toInt(),
    samples: e.samples.toInt(),
    basis: e.basis,
  );

  final bool isTask;
  final String? threadId;
  final String? taskId;

  /// 任务标题；一轮的是“这一轮”。
  final String title;
  final w.ProgressPhase phase;

  /// 主机的时刻。
  final int startedMs;
  final int stepsDone;
  final int? stepsExpected;

  /// 0 到 1；估不出时为空，界面画不定长的进度条。
  final double? fraction;

  /// 预计做完、最晚做完的时刻（主机的时刻）。在等人、受阻、停着的为空。
  final int? etaMs;
  final int? etaLateMs;

  /// 用了几个历史样本；0 表示没估。
  final int samples;

  /// 估算依据，给人看的一句话。
  final String basis;
}

/// 一位员工在干什么、手上的事做到哪了。
class AgentProgress {
  const AgentProgress({required this.agentId, required this.phase, required this.headline, required this.tasks});

  factory AgentProgress.fromWire(w.AgentProgress a) => AgentProgress(
    agentId: a.agentId,
    phase: a.phase,
    headline: a.headline == null ? null : ProgressEstimate.fromWire(a.headline!),
    tasks: [for (final t in a.tasks) ProgressEstimate.fromWire(t)],
  );

  final String agentId;
  final w.ProgressPhase phase;

  /// 头顶进度条显示的那一项；没事时为空。
  final ProgressEstimate? headline;
  final List<ProgressEstimate> tasks;
}

/// 一个要人看的时间点。
class AttentionItem {
  const AttentionItem({
    required this.kind,
    required this.agentId,
    required this.roleId,
    required this.taskId,
    required this.threadId,
    required this.atMs,
    required this.lateMs,
    required this.likelihood,
    required this.text,
  });

  factory AttentionItem.fromWire(w.AttentionPoint p) => AttentionItem(
    kind: p.kind,
    agentId: p.agentId,
    roleId: p.roleId,
    taskId: p.taskId,
    threadId: p.threadId,
    atMs: p.atMs.toInt(),
    lateMs: p.lateMs?.toInt(),
    likelihood: p.likelihood?.toDouble(),
    text: p.text,
  );

  final w.AttentionKind kind;
  final String? agentId;
  final String? roleId;
  final String? taskId;
  final String? threadId;

  /// 预计什么时候要人看（主机的时刻）；已经发生的是发生的时刻。
  final int atMs;

  /// 最晚什么时候；确定的时间点为空。
  final int? lateMs;

  /// 有多大可能真的要人看；已经发生的是 1，按速度外推的为空。
  final double? likelihood;

  /// 主机写的一句话，不含时刻。
  final String text;

  /// 还没发生、是估出来的。
  bool get predicted => kind.predicted;
}

extension AttentionKindLabel on w.AttentionKind {
  /// 是估出来的（还没发生）。预算那一种两样都有，看时刻和可能性。
  bool get predicted => switch (this) {
    w.AttentionKind.dueRisk ||
    w.AttentionKind.due ||
    w.AttentionKind.likelyQuestion ||
    w.AttentionKind.result ||
    w.AttentionKind.reply ||
    w.AttentionKind.stepLimit ||
    w.AttentionKind.budget => true,
    _ => false,
  };

  /// 短标签。
  String get label => switch (this) {
    w.AttentionKind.question => '在问你',
    w.AttentionKind.review => '等你验收',
    w.AttentionKind.blocked => '受阻',
    w.AttentionKind.failed => '失败了',
    w.AttentionKind.stalled => '停着',
    w.AttentionKind.unassigned => '等你分配',
    w.AttentionKind.overdue => '过了截止',
    w.AttentionKind.dueRisk => '可能赶不上',
    w.AttentionKind.due => '快到截止',
    w.AttentionKind.likelyQuestion => '可能会问你',
    w.AttentionKind.result => '交回结果',
    w.AttentionKind.reply => '回复你',
    w.AttentionKind.stepLimit => '可能中断',
    w.AttentionKind.budget => '预算',
    w.AttentionKind.unknown => '要你看',
  };
}

extension ProgressPhaseLabel on w.ProgressPhase {
  String get label => switch (this) {
    w.ProgressPhase.idle => '空闲',
    w.ProgressPhase.queued => '排队中',
    w.ProgressPhase.working => '在做',
    w.ProgressPhase.waitingHuman => '在等你',
    w.ProgressPhase.waitingColleague => '等同事',
    w.ProgressPhase.blocked => '受阻',
    w.ProgressPhase.stalled => '停着',
    w.ProgressPhase.failed => '上一轮失败',
    w.ProgressPhase.unknown => '',
  };

  /// 进度条要不要随时间往前走。
  bool get moving => this == w.ProgressPhase.working;
}

/// 主机的一张进度表，带上收到它时这台设备的时刻。
class ProgressBoard {
  ProgressBoard({required this.hostNowMs, required this.receivedAtMs, required this.agents, required this.attention});

  factory ProgressBoard.fromJson(Json j, {required int receivedAtMs}) {
    final b = w.ProgressBoard.fromJson(j);
    return ProgressBoard(
      hostNowMs: b.nowMs.toInt(),
      receivedAtMs: receivedAtMs,
      agents: {for (final a in b.agents) a.agentId: AgentProgress.fromWire(a)},
      attention: [for (final p in b.attention) AttentionItem.fromWire(p)],
    );
  }

  /// 主机算这张表的时刻。
  final int hostNowMs;

  /// 这台设备收到它的时刻。
  final int receivedAtMs;
  final Map<String, AgentProgress> agents;

  /// 按时刻排，同一时刻可能性大的在前（主机排好的）。
  final List<AttentionItem> attention;

  AgentProgress? of(String agentId) => agents[agentId];

  /// 这台设备的 [deviceMs] 对应主机的哪个时刻。
  int hostTimeAt(int deviceMs) => hostNowMs + (deviceMs - receivedAtMs);

  /// 主机的时刻换成这台设备的时刻（用来按本地时区显示）。
  int toDevice(int hostMs) => hostMs - hostNowMs + receivedAtMs;
}

/// 此刻的进度比例。在做的从主机给的比例出发，按时间往预计做完推，最多 [maxFraction]；
/// 没在做的（排队、在等人、受阻……）停在主机给的比例。估不出时为空。
double? fractionAt(ProgressEstimate e, ProgressBoard board, int deviceNowMs) {
  final f0 = e.fraction;
  if (f0 == null) return null;
  final eta = e.etaMs;
  if (!e.phase.moving || eta == null || f0 >= maxFraction) return f0;
  final span = eta - board.hostNowMs;
  if (span <= 0) return math.max(f0, maxFraction);
  final k = ((board.hostTimeAt(deviceNowMs) - board.hostNowMs) / span).clamp(0.0, 1.0);
  return math.min(maxFraction, f0 + (1 - f0) * k);
}

/// “约 3 分钟”“快好了”“比预计久”；估不出时为空。
String? etaText(ProgressEstimate e, ProgressBoard board, int deviceNowMs) {
  final eta = e.etaMs;
  if (eta == null) return null;
  final left = eta - board.hostTimeAt(deviceNowMs);
  if (left > 0) return '约 ${durationText(left)}';
  final late = e.etaLateMs;
  if (late != null && board.hostTimeAt(deviceNowMs) > late) return '比预计久';
  return '快好了';
}

/// “40 秒”“3 分钟”“1.5 小时”。
String durationText(int ms) {
  final secs = ms / 1000;
  if (secs < 60) return '${math.max(1, secs.round())} 秒';
  if (secs < 90 * 60) return '${(secs / 60).round()} 分钟';
  return '${(secs / 3600).toStringAsFixed(1)} 小时';
}

/// 时间轴上一项的时刻怎么写：已经到了的写“现在”，一分钟以内的写“马上”；今天的写“14:20”，
/// 不是今天的带上“明天”或日期；有最晚时刻的写成一段“14:20–14:35”。[now] 是这台设备的现在。
String attentionTimeText(AttentionItem item, ProgressBoard board, DateTime now) {
  final at = board.toDevice(item.atMs);
  final left = at - now.millisecondsSinceEpoch;
  if (left <= 0) return '现在';
  if (left < 60 * 1000) return '马上';
  final today = DateTime(now.year, now.month, now.day);
  ({String day, String hm}) clock(int ms) {
    final t = DateTime.fromMillisecondsSinceEpoch(ms);
    final hm = '${t.hour.toString().padLeft(2, '0')}:${t.minute.toString().padLeft(2, '0')}';
    final day = switch (DateTime(t.year, t.month, t.day).difference(today).inDays) {
      0 => '',
      1 => '明天 ',
      _ => '${t.month}月${t.day}日 ',
    };
    return (day: day, hm: hm);
  }

  final start = clock(at);
  final late = item.lateMs;
  if (late == null || board.toDevice(late) - at < 60 * 1000) return '约 ${start.day}${start.hm}';
  final end = clock(board.toDevice(late));
  // 同一天的只写一次日期。
  return '约 ${start.day}${start.hm}–${end.day == start.day ? '' : end.day}${end.hm}';
}

/// “可能 67%”“按速度估”；确定的为空。
String? likelihoodText(AttentionItem item) {
  final p = item.likelihood;
  if (p == null) return '按速度估';
  if (p >= 1) return null;
  return '可能 ${(p * 100).round()}%';
}

/// 时间轴上列哪些：页面上别的地方已经列了的（员工提问、验收卡片、受阻的任务、等你分派的消息）不重复，
/// [skip] 里的种类也不列。
List<AttentionItem> timelineItems(ProgressBoard board, {Set<w.AttentionKind> skip = const {}}) => [
  for (final p in board.attention)
    if (!skip.contains(p.kind) &&
        p.kind != w.AttentionKind.question &&
        p.kind != w.AttentionKind.review &&
        p.kind != w.AttentionKind.blocked &&
        !(p.kind == w.AttentionKind.unassigned && p.taskId == null))
      p,
];
