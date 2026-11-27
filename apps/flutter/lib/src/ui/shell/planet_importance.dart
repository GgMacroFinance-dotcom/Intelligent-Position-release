// 星球重要度缩放、AI 资源分配顺位传导与未来资源争用反馈的预留接口契约（2026-11-27）。
//
// =============================================================================
// 【后端协议与调度空缺说明（BACKEND / WIRE GAP）—— 后端暂未执行，接口先行预留】
// =============================================================================
// 1. 交互与物理维度（与 orbit.dart 的三条轨道正交）：
//    - 离太阳远近（轨道 0/1/2）= 紧急度（何时做）；
//    - 星球体积大小（滚轮缩放 / 三栏左栏调节）= 重要度权重（0.0..1.0，给多少资源、冲突时谁先上）。
//    - 太阳（幕僚长）是总调度中枢，不参与普通员工之间的大小缩放与顺位竞争。
//
// 2. WebSocket / 存储协议预留（`rust/crates/ip-core/src/org.rs` & `ip-server/src/protocol.rs`）：
//    - 目前 `Agent` 只有 `{ id, role, name }`，尚未记录用户手动调节的重要度权重；
//    - 建议后续在 `Agent` / 星图偏好中增加可选字段：
//        * `importance: Option<f64>`          // 0.0..1.0，null 表示恢复系统自动计算
//        * `importance_updated_at: Option<i64>` // Unix ms，同权重并发冲突时作为稳定破平依据
//    - 建议后端开放两个管理/操控请求（由前端 [ImportanceSyncPort] 在防抖停手后调用）：
//        * `set_agent_importance`：{ agent_id, importance, updated_at_ms } -> Agent
//        * `clear_agent_importance`：{ agent_id } -> Agent
//
// 3. AI 资源分配与并发调度传导预留（`rust/crates/ip-runtime/src/agent.rs` & `budget.rs`）：
//    - 未来后端接通后，可按 [PriorityAllocator] 的确定性规则将重要度传导至：
//        (a) 全局优先级顺位（`#1, #2, ...`）：并发池/模型限流/共享工具槽位争用时按顺位放行；
//        (b) 配额与模型倾斜：总预算池紧张时按 [PriorityAllocation.resourceShare] 加权分配，
//            高顺位优先保留 `model.escalate` 资格，低顺位自动收紧至轻量默认模型；
//        (c) 幕僚长调度上下文：将当前顺位注入太阳提示词，使 AI 拆活派单自然偏向大星球。
//
// 4. 未来资源分配冲突反馈预留（`RuntimeEvent::ResourceContention` -> [ContentionFeedbackPort]）：
//    - 当出现全局并发上限、API 限流（429/RPM）、共享预算见底或单岗预算触顶时，后端可广播
//      [ResourceContentionSnapshot]，明确列出“冲突原因 + 按重要度排出的先后队列 + 被顺延等待的星球”，
//      前端届时直接订阅 [ContentionFeedbackPort] 即可在天穹和提示条中实时呈现，无需重构交互状态。
// =============================================================================

import 'dart:math' as math;

import 'package:flutter/foundation.dart';

import '../../star_map.dart' show BotBody;

/// 轨道上单颗员工星球的最小/最大逻辑球径（太阳为 50px，放大到顶 48px 仍克制在太阳之下）。
const double planetMinOrbitRadius = 18.0;
const double planetMaxOrbitRadius = 48.0;

/// 将重要度（0.0..1.0）映射为天穹轨道上的统一球体逻辑半径（18px..48px）。
///
/// 系统自动算出的重要度与用户手动滚轮/按钮调节共用同一条严格单调曲线，保证首次缩放零跳变。
double planetOrbitRadius(double importance) =>
    planetMinOrbitRadius + importance.clamp(0.0, 1.0) * (planetMaxOrbitRadius - planetMinOrbitRadius);

/// 将 0.0..1.0 重要度换算为整数百分比（0..100），供微提示与三栏左侧控件展示。
int importancePercent(double importance) => (importance.clamp(0.0, 1.0) * 100).round();

/// 某位员工被用户手动调节重要度（放大/缩小星球）的覆盖记录。
@immutable
class PlanetImportanceOverride {
  const PlanetImportanceOverride({required this.agentId, required this.importance, required this.updatedAtMs});

  final String agentId;

  /// 手动重要度权重（0.0 = 最小/低优 .. 1.0 = 最大/最高优）。
  final double importance;

  /// 最近一次手动调节的毫秒时间戳（Unix ms），用于同权重时的确定性顺位裁决。
  final int updatedAtMs;

  int get percent => importancePercent(importance);

  PlanetImportanceOverride copyWith({double? importance, int? updatedAtMs}) => PlanetImportanceOverride(
    agentId: agentId,
    importance: (importance ?? this.importance).clamp(0.0, 1.0),
    updatedAtMs: updatedAtMs ?? this.updatedAtMs,
  );

  Map<String, Object?> toJson() => {
    'agent_id': agentId,
    'importance': double.parse(importance.toStringAsFixed(4)),
    'updated_at_ms': updatedAtMs,
  };

  /// 预留给未来后端 WebSocket RPC 的建议载荷格式（当前不发送）。
  Map<String, Object?> toPlannedWireJson() => {
    'type': 'set_agent_importance',
    'agent_id': agentId,
    'importance': double.parse(importance.toStringAsFixed(4)),
    'updated_at_ms': updatedAtMs,
  };

  static PlanetImportanceOverride? fromJson(Map<String, Object?> json) {
    final id = json['agent_id'];
    final raw = json['importance'];
    if (id is! String || id.isEmpty || raw is! num || raw.isNaN) return null;
    final at = json['updated_at_ms'];
    return PlanetImportanceOverride(
      agentId: id,
      importance: raw.toDouble().clamp(0.0, 1.0),
      updatedAtMs: at is int ? at : 0,
    );
  }

  @override
  bool operator ==(Object other) =>
      other is PlanetImportanceOverride &&
      other.agentId == agentId &&
      (other.importance - importance).abs() < 1e-6 &&
      other.updatedAtMs == updatedAtMs;

  @override
  int get hashCode => Object.hash(agentId, importance.toStringAsFixed(4), updatedAtMs);
}

/// 重要度档位（供未来 AI 配额倾斜、模型升级准入与界面状态说明统一引用）。
enum PriorityTier {
  /// 核心最高优（>= 75%）：资源争用时最先拿槽位，优先保留 `model.escalate` 资格。
  critical('核心优先'),

  /// 重点保障（45%..74%）：常规高优执行，预算紧张时优先保障。
  elevated('重点保障'),

  /// 标准均衡（20%..44%）：按默认策略分配算力与预算。
  standard('标准分配'),

  /// 后台顺延（< 20%）：资源充足时照常运行，冲突时优先让位于大星球。
  deferred('紧张时退让');

  const PriorityTier(this.label);

  final String label;

  static PriorityTier fromImportance(double importance) {
    final v = importance.clamp(0.0, 1.0);
    if (v >= 0.75) return PriorityTier.critical;
    if (v >= 0.45) return PriorityTier.elevated;
    if (v >= 0.20) return PriorityTier.standard;
    return PriorityTier.deferred;
  }
}

/// 单颗星球经 [PriorityAllocator] 计算后的实时顺位与预留 AI 配额分配结果。
@immutable
class PriorityAllocation {
  const PriorityAllocation({
    required this.agentId,
    required this.rank,
    required this.effectiveImportance,
    required this.manual,
    required this.orbitTrackId,
    required this.resourceShare,
    required this.tier,
  });

  final String agentId;

  /// 全队资源优先顺位（从 1 开始，#1 最优先）。
  final int rank;

  /// 当前生效的综合重要度（0.0..1.0；手动覆盖优先于系统自动计算）。
  final double effectiveImportance;

  /// true 表示由用户手动放大/缩小指定；false 表示系统按任务负荷与预算自动推算。
  final bool manual;

  /// 所在轨道编号（0 内环 / 1 中环 / 2 外环），同重要度时作为次级破平维度。
  final int orbitTrackId;

  /// 归一化后的理论资源配额占比（全队之和为 1.0，供未来预算池与并发槽加权分配消费）。
  final double resourceShare;

  /// 对应的资源分配策略档位。
  final PriorityTier tier;
}

/// 全队所有非太阳星球在某一时刻的确定性顺位快照。
@immutable
class PrioritySnapshot {
  const PrioritySnapshot({required this.ordered, required this.byAgent});

  /// 按资源优先顺位从高到低（#1 -> #N）排列的完整列表。
  final List<PriorityAllocation> ordered;

  /// 按 `agentId` 快速查表。
  final Map<String, PriorityAllocation> byAgent;

  PriorityAllocation? of(String agentId) => byAgent[agentId];
}

/// 预留的 AI 资源分配与先后顺位计算器（纯函数、无副作用、确定性可复现）。
///
/// 排序破平优先级（从高到低）：
/// 1. 有效重要度 `effectiveImportance` 更高（放大后的星球直接跃升顺位）；
/// 2. 所在轨道更靠内（0 内环 > 1 中环 > 2 外环）；
/// 3. 手动调权优先于自动计算；同为手动调权时，最近一次调权（`updatedAtMs` 更新）优先；
/// 4. 实时紧急度 `urgency` 更高；
/// 5. `agent.id` 字典序兜底，保证跨端结果绝对一致。
class PriorityAllocator {
  const PriorityAllocator._();

  /// 保底权重系数，避免 0% 重要度星球在资源宽松时分到 0 算力。
  static const double _weightFloor = 0.15;

  static PrioritySnapshot compute({
    required List<BotBody> bodies,
    Map<String, int> trackByAgent = const {},
    Map<String, PlanetImportanceOverride> overrides = const {},
  }) {
    if (bodies.isEmpty) {
      return const PrioritySnapshot(ordered: [], byAgent: {});
    }

    final entries =
        <({BotBody body, double imp, bool manual, int trackId, int updatedAtMs})>[
          for (final b in bodies)
            () {
              final ov = overrides[b.agent.id];
              final imp = (ov?.importance ?? b.importance).clamp(0.0, 1.0);
              return (
                body: b,
                imp: imp,
                manual: ov != null,
                trackId: (trackByAgent[b.agent.id] ?? 2).clamp(0, 2),
                updatedAtMs: ov?.updatedAtMs ?? 0,
              );
            }(),
        ]..sort((a, b) {
          final impDiff = b.imp - a.imp;
          if (impDiff.abs() > 1e-4) return impDiff > 0 ? 1 : -1;
          final trackCmp = a.trackId.compareTo(b.trackId);
          if (trackCmp != 0) return trackCmp;
          if (a.manual != b.manual) return a.manual ? -1 : 1;
          final atCmp = b.updatedAtMs.compareTo(a.updatedAtMs);
          if (atCmp != 0) return atCmp;
          final urgCmp = b.body.urgency.compareTo(a.body.urgency);
          if (urgCmp != 0) return urgCmp;
          return a.body.agent.id.compareTo(b.body.agent.id);
        });

    // 柔和非线性加权（1.25 次幂）：让放大的高优星球更清晰地获得资源倾斜，同时保留保底权重。
    final rawWeights = [for (final e in entries) _weightFloor + math.pow(e.imp, 1.25).toDouble()];
    final totalWeight = rawWeights.fold<double>(0.0, (sum, w) => sum + w);

    final ordered = <PriorityAllocation>[];
    final byAgent = <String, PriorityAllocation>{};
    for (var i = 0; i < entries.length; i++) {
      final e = entries[i];
      final alloc = PriorityAllocation(
        agentId: e.body.agent.id,
        rank: i + 1,
        effectiveImportance: e.imp,
        manual: e.manual,
        orbitTrackId: e.trackId,
        resourceShare: totalWeight > 0 ? rawWeights[i] / totalWeight : 1.0 / entries.length,
        tier: PriorityTier.fromImportance(e.imp),
      );
      ordered.add(alloc);
      byAgent[alloc.agentId] = alloc;
    }
    return PrioritySnapshot(ordered: List.unmodifiable(ordered), byAgent: Map.unmodifiable(byAgent));
  }
}

// =============================================================================
// 预留传导接口 1：重要度同步端口（当前默认空实现，后端暂不执行）
// =============================================================================

/// 面向未来后端 `set_agent_importance` / `clear_agent_importance` RPC 的同步接口。
abstract interface class ImportanceSyncPort {
  /// 用户通过滚轮（防抖停手后）或三栏左侧控件调整了某颗星球的重要度。
  Future<void> syncOverride(PlanetImportanceOverride override);

  /// 用户点击复位，恢复由系统按任务负荷与预算自动计算该星球重要度。
  Future<void> clearOverride(String agentId);
}

/// 当前阶段默认空实现：不向主机发送任何请求，仅保留本地前端缩放与顺位闭环。
class NoopImportanceSyncPort implements ImportanceSyncPort {
  const NoopImportanceSyncPort();

  @override
  Future<void> syncOverride(PlanetImportanceOverride override) async {}

  @override
  Future<void> clearOverride(String agentId) async {}
}

// =============================================================================
// 预留反馈接口 2：未来资源分配冲突与先后顺序反馈（当前仅定义契约，不渲染占位弹窗）
// =============================================================================

/// 未来触发资源先后顺序调度的冲突类型。
enum ResourceContentionKind {
  /// 全局并发执行槽位已满。
  concurrencyCap,

  /// 上游模型接口触发速率限制（429 / RPM / TPM）。
  modelRateLimit,

  /// 团队共享日预算池紧张。
  sharedBudgetPool,

  /// 单个岗位每日预算触顶。
  roleBudgetCap,

  /// 共享工具（如浏览器/MCP 会话）并发争用。
  sharedToolSlot,
}

/// 资源冲突发生时，队列中单颗星球的裁决结果。
enum ResourceQueueDisposition {
  /// 高顺位：优先放行执行或保留强模型资格。
  prioritized,

  /// 排队等待：因顺位靠后暂缓执行，待前序星球释放资源。
  queued,

  /// 降级执行：退回默认轻量模型或收紧上下文配额以继续运行。
  downgraded,

  /// 暂停：预算见底或显式让行。
  paused,
}

/// 冲突队列中的单条员工记录（解释“排第几位、让给了谁、为什么”）。
@immutable
class ResourceQueueEntry {
  const ResourceQueueEntry({
    required this.agentId,
    required this.rank,
    required this.effectiveImportance,
    required this.disposition,
    this.waitingBehindAgentId,
    required this.reason,
  });

  final String agentId;
  final int rank;
  final double effectiveImportance;
  final ResourceQueueDisposition disposition;
  final String? waitingBehindAgentId;
  final String reason;
}

/// 一次资源分配冲突的完整反馈快照（供未来天穹让行状态与队列浮条直接消费）。
@immutable
class ResourceContentionSnapshot {
  const ResourceContentionSnapshot({
    required this.kind,
    required this.occurredAtMs,
    required this.summary,
    required this.queue,
  });

  final ResourceContentionKind kind;
  final int occurredAtMs;
  final String summary;
  final List<ResourceQueueEntry> queue;
}

/// 面向未来后端 `RuntimeEvent::ResourceContention` 推送流的反馈订阅端口。
abstract interface class ContentionFeedbackPort {
  /// 当前正在发生的资源冲突快照；无冲突时为 null。
  ResourceContentionSnapshot? get current;

  /// 资源冲突状态变更流（未来接入 WebSocket 事件推送）。
  Stream<ResourceContentionSnapshot?> get updates;
}

/// 当前阶段默认空实现：不产生任何模拟告警，界面保持安静。
class NoopContentionFeedbackPort implements ContentionFeedbackPort {
  const NoopContentionFeedbackPort();

  @override
  ResourceContentionSnapshot? get current => null;

  @override
  Stream<ResourceContentionSnapshot?> get updates => const Stream<ResourceContentionSnapshot?>.empty();
}
