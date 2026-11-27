// 宽窗口三条固定通用轨道体系：轨道几何、微倾透视投影、容量槽位分配、满环级联挤出、拖拽磁吸计算。
//
// =============================================================================
// 【后端适配与协议匹配说明（2026-11-26 设计定稿）】
// =============================================================================
// 1. 轨道与星球彻底解耦（通用槽位骨架，不绑定具体员工/星球形态）：
//    - 轨道 0（inner  / 内环 · 核心与紧急）：容量上限 3（1/3 ~ 3/3）
//    - 轨道 1（middle / 中环 · 进行中与活跃）：容量上限 4（1/4 ~ 4/4）
//    - 轨道 2（outer  / 外环 · 空闲与待命）：容量上限 7（1/7 ~ 7/7）
//    - 常规总容量 = 3 + 4 + 7 = 14 颗；若团队员工数超过 14，外环（轨道 2）自动作为软兜底池超额接纳，绝不丢弃员工。
//
// 2. 自动入轨与手动拖放的优先级（后端持久化字段建议）：
//    - 自动入轨（未手动固定时，按 star_map.dart 实时算出的 urgency / working 动态分配）：
//        urgency >= 0.65            -> 轨道 0（内环）
//        urgency >= 0.20 || working -> 轨道 1（中环）
//        其余                        -> 轨道 2（外环）
//    - 手动拖放（用户把任意星球拖到某条环附近磁吸松手）：
//        视为用户手动指定该员工的关注层级，优先级高于自动 urgency。
//    - 建议后端在 Agent（或星图布局偏好接口）上增加两个可选字段以支持多端同步与跨重启记忆：
//        * `orbit_track`: int?      // 0 = 内环, 1 = 中环, 2 = 外环, null = 未固定（走自动紧急度）
//        * `orbit_pinned_at`: int?  // 毫秒时间戳（Unix ms），用于满环挤出时判断谁是“更晚放进去的新成员”
//    - 恢复自动排轨时，后端将该员工的 `orbit_track` 置回 null、`orbit_pinned_at` 置回 null 即可。
//
// 3. 满环自动挤出替换规则（方案 1A，客户端与后端建议保持同一确定性规则）：
//    - 当用户把星球 A 拖入目标环 T（例如内环已 3/3），A 必定成功吸附进 T，并立即触发级联挤出：
//        * 轨道 0 超过 3 颗 -> 挤出 1 颗到轨道 1；若轨道 1 随之超过 4 颗 -> 再挤出 1 颗到轨道 2；
//        * 若目标为轨道 2 且超过 7 颗，且总员工数 <= 14 -> 依次向有空位的内侧轨道（1 -> 0）退让；超过 14 人则轨道 2 软兜底。
//    - 同环内“谁留、谁被挤出”的确定性优先级（从最优先保留 → 最先被挤出）：
//        (1) 本次刚拖入的星球（priorityAgentId）绝对保留；
//        (2) 手动固定（有 OrbitPin）优先于自动入轨；
//        (3) 同为手动固定时，pinnedAtMs 更新的（后拖进来的）优先保留，最早放入的先被挤出；
//        (4) 同为自动入轨时，urgency 更高的优先保留，其次 importance，最后按 agent.id 字典序兜底。
//    - 每次拖拽落位后，[OrbitTrack.applyDrop] 会返回本次受影响的变更列表 [OrbitDropOutcome.changes]
//      （含被拖入的员工 + 被级联挤出的员工各自的 beforeTrack / afterTrack / pin），
//      后端适配时直接批量提交该变更列表（如 `PATCH /star-map/orbit-pins`）即可保证前后端完全一致。
// =============================================================================

import 'dart:math' as math;
import 'dart:ui' show Offset, lerpDouble;

import 'package:flutter/foundation.dart';

import '../../star_map.dart' show BotBody;

/// 某位员工被用户手动固定在某条轨道上的记录。
///
/// 后续对接后端时可直接序列化为：
/// `{"orbit_track": trackId, "orbit_pinned_at": pinnedAtMs}`。
@immutable
class OrbitPin {
  const OrbitPin({required this.trackId, required this.pinnedAtMs});

  /// 目标固定轨道编号：0 = 内环，1 = 中环，2 = 外环。
  final int trackId;

  /// 用户放入（或被挤出落位）的毫秒时间戳，用于同环满员时的确定性挤出顺序。
  final int pinnedAtMs;

  OrbitPin copyWith({int? trackId, int? pinnedAtMs}) =>
      OrbitPin(trackId: trackId ?? this.trackId, pinnedAtMs: pinnedAtMs ?? this.pinnedAtMs);

  Map<String, Object?> toJson() => {'orbit_track': trackId, 'orbit_pinned_at': pinnedAtMs};

  static OrbitPin? fromJson(Map<String, Object?> json) {
    final track = json['orbit_track'];
    if (track is! int || track < 0 || track >= OrbitTrack.all.length) return null;
    final at = json['orbit_pinned_at'];
    return OrbitPin(trackId: track, pinnedAtMs: at is int ? at : 0);
  }

  @override
  bool operator ==(Object other) => other is OrbitPin && other.trackId == trackId && other.pinnedAtMs == pinnedAtMs;

  @override
  int get hashCode => Object.hash(trackId, pinnedAtMs);
}

/// 一次手动拖拽落位（含满环级联挤出）中，单颗星球的轨道变化记录，供后端同步接口直接消费。
@immutable
class OrbitPinChange {
  const OrbitPinChange({
    required this.agentId,
    required this.fromTrackId,
    required this.toTrackId,
    required this.evicted,
    required this.pin,
  });

  final String agentId;
  final int fromTrackId;
  final int toTrackId;

  /// true 表示因目标环已满而被顺次挤出到相邻轨道；false 表示用户亲手拖入的星球。
  final bool evicted;

  /// 变更后的最新固定状态（后续可直接提交后端持久化）。
  final OrbitPin pin;
}

/// 一次拖拽松手落位的完整计算结果（新固定表 + 变更明细 + 最终全员槽位）。
@immutable
class OrbitDropOutcome {
  const OrbitDropOutcome({required this.pins, required this.changes, required this.layout});

  /// 更新后的全量手动固定表（传回 UI 状态 / 本地缓存 / 后端同步）。
  final Map<String, OrbitPin> pins;

  /// 本次真正发生轨道变更或重新置顶的员工列表（第一条为用户拖动的星球，其余为被级联挤出的星球）。
  final List<OrbitPinChange> changes;

  /// 更新后的全员轨道分配与计数快照。
  final OrbitLayout layout;
}

/// 单颗星球在轨道上的最终分配结果。
@immutable
class OrbitSlot {
  const OrbitSlot({
    required this.track,
    required this.phase,
    required this.slotIndex,
    required this.slotCount,
    required this.pinned,
  });

  final OrbitTrack track;

  /// 平近点角初相（弧度），保证同轨多星均匀错开、同速不追尾。
  final double phase;

  /// 在当前轨道中的槽位序号（0..slotCount-1）。
  final int slotIndex;

  /// 当前轨道上的总星球数。
  final int slotCount;

  /// 是否来自用户手动拖放固定。
  final bool pinned;
}

/// 某一时刻全员在三条固定轨道上的分配快照。
@immutable
class OrbitLayout {
  const OrbitLayout({required this.slots, required this.counts, required this.membersByTrack});

  /// 按 `agent.id` 索引的轨道与槽位信息。
  final Map<String, OrbitSlot> slots;

  /// 三条轨道当前在轨人数，下标严格对应轨道编号 0 / 1 / 2。
  final List<int> counts;

  /// 每条轨道上的成员 ID 列表（已按稳定顺序排列）。
  final List<List<String>> membersByTrack;

  int countOf(int trackId) => counts[trackId];
}

/// 指针拖拽过程中对某条轨道的磁吸计算结果。
@immutable
class OrbitSnapTarget {
  const OrbitSnapTarget({
    required this.track,
    required this.ecc,
    required this.pose,
    required this.distance,
    required this.snapped,
  });

  final OrbitTrack track;

  /// 轨道上最近点的偏近点角（弧度）。
  final double ecc;

  /// 该最近点在屏幕上的透视位置、深度与缩放。
  final OrbitPose pose;

  /// 指针距离轨道最近点的屏幕像素距离。
  final double distance;

  /// 是否已进入磁吸捕获半径（进入后星球会吸附向金线，松手即落轨）。
  final bool snapped;
}

/// 三条相对固定的通用轨道（不绑定具体星球，星球按实时状态或手动拖放动态入轨）：
///   0 内环（核心/紧急，上限 3）、1 中环（进行中/活跃，上限 4）、2 外环（空闲/待命，上限 7）。
/// 每条轨道配克制微倾角（[rollDeg]）与略微递进的压扁比（[flatten]），
/// 保持扁椭圆主基调的同时拉开上下挤压带，避免多星堆在一条水平线上。
@immutable
class OrbitTrack {
  const OrbitTrack({
    required this.id,
    required this.key,
    required this.label,
    required this.capacity,
    required this.frac,
    required this.rollDeg,
    required this.flatten,
    required this.phaseOffset,
  });

  /// 轨道编号（0 / 1 / 2），与后端协议 `orbit_track` 一一对应。
  final int id;

  /// 语义键（'inner' / 'middle' / 'outer'），便于日志与后端语义对照。
  final String key;

  /// 界面短名称（拖动时在环边缘徽章展示）。
  final String label;

  /// 轨道常规容量上限：内环 3、中环 4、外环 7（总常规容量 14，超过时外环软兜底）。
  final int capacity;

  /// 相对最外圈半宽的比例。
  final double frac;

  /// 克制微倾角（度，围绕视线轴滚转）。
  final double rollDeg;

  /// 纵向压扁系数（越靠外略微放开一点，拉开远/近挤压带）。
  final double flatten;

  /// 同轨默认起始相位（弧度），三条环彼此错开，少人时不排成一条直线。
  final double phaseOffset;

  double get rollRad => rollDeg * math.pi / 180.0;

  static const all = [
    OrbitTrack(
      id: 0,
      key: 'inner',
      label: '内环',
      capacity: 3,
      frac: 0.52,
      rollDeg: -6.5,
      flatten: 0.30,
      phaseOffset: -1.15,
    ),
    OrbitTrack(
      id: 1,
      key: 'middle',
      label: '中环',
      capacity: 4,
      frac: 0.76,
      rollDeg: 5.5,
      flatten: 0.32,
      phaseOffset: 0.45,
    ),
    OrbitTrack(
      id: 2,
      key: 'outer',
      label: '外环',
      capacity: 7,
      frac: 1.00,
      rollDeg: -2.5,
      flatten: 0.34,
      phaseOffset: -0.38,
    ),
  ];

  static OrbitTrack byId(int id) => all[id.clamp(0, all.length - 1)];

  /// 常规总容量（3 + 4 + 7 = 14）。
  static const totalCapacity = 14;

  /// 仅按员工实时状态决定的自动默认轨道（未手动拖放固定时生效）。
  static OrbitTrack autoFor(BotBody body) {
    if (body.urgency >= 0.65) return all[0];
    if (body.urgency >= 0.20 || body.working) return all[1];
    return all[2];
  }

  /// 计算当前全员的轨道分配：先应用用户手动固定 [manualPins]，再按 3 / 4 / 7 容量做确定性级联平衡，
  /// 最后在每条轨道内按稳定 ID 均匀排布初相（同轨同速不追尾）。
  static OrbitLayout assign(
    List<BotBody> bodies, {
    Map<String, OrbitPin> manualPins = const {},
    String? priorityAgentId,
  }) {
    final byIdMap = <String, BotBody>{for (final b in bodies) b.agent.id: b};
    final buckets = <List<BotBody>>[[], [], []];

    for (final b in bodies) {
      final pin = manualPins[b.agent.id];
      final tid = pin != null ? pin.trackId.clamp(0, all.length - 1) : autoFor(b).id;
      buckets[tid].add(b);
    }

    _enforceCapacities(
      buckets: buckets,
      manualPins: manualPins,
      totalCount: bodies.length,
      priorityAgentId: priorityAgentId,
    );

    final slots = <String, OrbitSlot>{};
    final counts = <int>[0, 0, 0];
    final members = <List<String>>[[], [], []];

    for (final tk in all) {
      final group = buckets[tk.id]..sort((a, b) => a.agent.id.compareTo(b.agent.id));
      final m = group.length;
      counts[tk.id] = m;
      members[tk.id] = [for (final b in group) b.agent.id];
      for (var slot = 0; slot < m; slot++) {
        final b = group[slot];
        final id = b.agent.id;
        // 确认该星最终是否在自己手动固定的环上（保留 pinned 标记供 UI/调试用）。
        final pin = manualPins[id];
        slots[id] = OrbitSlot(
          track: tk,
          phase: tk.phaseOffset + slot * 2 * math.pi / m,
          slotIndex: slot,
          slotCount: m,
          pinned: pin != null && pin.trackId == tk.id && byIdMap.containsKey(id),
        );
      }
    }

    return OrbitLayout(slots: slots, counts: counts, membersByTrack: members);
  }

  /// 模拟/执行把 [agentId] 手动拖入 [targetTrackId] 的结果（含满环 1A 自动级联挤出）。
  ///
  /// - 拖动预览时可调用本方法预测“松手后哪颗星会被挤出、各环人数如何变化”，不会修改外部状态；
  /// - 松手落位时调用本方法并接管返回的 [OrbitDropOutcome.pins]，同时把 [OrbitDropOutcome.changes]
  ///   交给后端同步回调（后续接 API 时直接复写这里）。
  static OrbitDropOutcome applyDrop({
    required List<BotBody> bodies,
    required Map<String, OrbitPin> currentPins,
    required String agentId,
    required int targetTrackId,
    required int nowMs,
  }) {
    final clampedTarget = targetTrackId.clamp(0, all.length - 1);
    final beforeLayout = assign(bodies, manualPins: currentPins);
    final nextPins = Map<String, OrbitPin>.from(currentPins);

    // 1. 记录本次用户亲手拖入的星球（优先级最高，必定留在该环）。
    nextPins[agentId] = OrbitPin(trackId: clampedTarget, pinnedAtMs: nowMs);

    // 2. 用新固定表跑一次容量平衡，算出最终每个人落哪条环。
    final afterLayout = assign(bodies, manualPins: nextPins, priorityAgentId: agentId);

    // 3. 把因为满环被级联挤出到相邻轨道的星球也同步写入 nextPins（时间戳早于本次拖入），
    //    确保被挤出的星球稳稳留在新环，不会在下一次刷新时又跳回原环。
    final changes = <OrbitPinChange>[];
    final draggedBefore = beforeLayout.slots[agentId]?.track.id ?? clampedTarget;
    final draggedAfter = afterLayout.slots[agentId]?.track.id ?? clampedTarget;
    final draggedPin = OrbitPin(trackId: draggedAfter, pinnedAtMs: nowMs);
    nextPins[agentId] = draggedPin;
    changes.add(
      OrbitPinChange(
        agentId: agentId,
        fromTrackId: draggedBefore,
        toTrackId: draggedAfter,
        evicted: false,
        pin: draggedPin,
      ),
    );

    var evictOrder = 1;
    for (final b in bodies) {
      final id = b.agent.id;
      if (id == agentId) continue;
      final beforeTrack = beforeLayout.slots[id]?.track.id;
      final afterTrack = afterLayout.slots[id]?.track.id;
      if (beforeTrack == null || afterTrack == null || beforeTrack == afterTrack) continue;
      final evictedPin = OrbitPin(trackId: afterTrack, pinnedAtMs: nowMs - evictOrder);
      evictOrder++;
      nextPins[id] = evictedPin;
      changes.add(
        OrbitPinChange(agentId: id, fromTrackId: beforeTrack, toTrackId: afterTrack, evicted: true, pin: evictedPin),
      );
    }

    // 4. 用同步完挤出记录的 nextPins 再生成一次最终快照，保证 pinned 标记与后续渲染完全一致。
    final finalLayout = assign(bodies, manualPins: nextPins, priorityAgentId: agentId);
    return OrbitDropOutcome(pins: nextPins, changes: changes, layout: finalLayout);
  }

  /// 按 3 / 4 / 7 常规容量做级联挤出：
  ///   - 内环(0) > 3 -> 挤出优先级最低者到中环(1)；
  ///   - 中环(1) > 4 -> 挤出优先级最低者到外环(2)；
  ///   - 外环(2) > 7 且总人数 <= 14 -> 挤出优先级最低者向内退让到有空位的中环(1)或内环(0)；
  ///   - 若总人数 > 14，则外环作为软兜底池接纳剩余星球。
  static void _enforceCapacities({
    required List<List<BotBody>> buckets,
    required Map<String, OrbitPin> manualPins,
    required int totalCount,
    required String? priorityAgentId,
  }) {
    int compareStayPriority(BotBody a, BotBody b) {
      // 返回负数表示 a 更优先留下；正数表示 b 更优先留下（排在末尾的先被挤出）。
      if (a.agent.id == priorityAgentId) return -1;
      if (b.agent.id == priorityAgentId) return 1;
      final pinA = manualPins[a.agent.id];
      final pinB = manualPins[b.agent.id];
      if ((pinA != null) != (pinB != null)) return pinA != null ? -1 : 1;
      if (pinA != null && pinB != null && pinA.pinnedAtMs != pinB.pinnedAtMs) {
        return pinB.pinnedAtMs.compareTo(pinA.pinnedAtMs); // 越晚放入越优先留下
      }
      if (a.urgency != b.urgency) return b.urgency.compareTo(a.urgency);
      if (a.importance != b.importance) return b.importance.compareTo(a.importance);
      return a.agent.id.compareTo(b.agent.id);
    }

    // 正向级联：内环 -> 中环 -> 外环
    for (var tid = 0; tid < all.length - 1; tid++) {
      final cap = all[tid].capacity;
      if (buckets[tid].length <= cap) continue;
      buckets[tid].sort(compareStayPriority);
      final overflow = buckets[tid].sublist(cap);
      buckets[tid].removeRange(cap, buckets[tid].length);
      buckets[tid + 1].addAll(overflow);
    }

    // 反向退让：外环超过 7 且总人数未超 14 时，把多余的顺次退让到有空位的中环/内环
    final outerCap = all[2].capacity;
    if (totalCount <= totalCapacity && buckets[2].length > outerCap) {
      buckets[2].sort(compareStayPriority);
      while (buckets[2].length > outerCap) {
        final target = buckets[1].length < all[1].capacity
            ? 1
            : buckets[0].length < all[0].capacity
            ? 0
            : -1;
        if (target == -1) break;
        buckets[target].add(buckets[2].removeLast());
      }
    }
  }
}

/// 三轨道的几何基准（中心在太阳，[minRx] 保证窄窗下内圈不压太阳，[maxRx] 留出窗边安全距）。
@immutable
class OrbitGeometry {
  const OrbitGeometry({required this.center, required this.minRx, required this.maxRx});

  /// 太阳（轨道焦点）在屏幕上的位置。
  final Offset center;

  /// 内圈最小半宽 / 外圈左右半宽（逻辑像素）。
  final double minRx;
  final double maxRx;

  /// 把轨道比例 [frac]（0.52..1.00）映射为实际左右半宽，窄窗下自动保底到 [minRx]。
  double radiusFor(double frac) {
    final innerFrac = OrbitTrack.all.first.frac;
    final t = ((frac - innerFrac) / (1.0 - innerFrac)).clamp(0.0, 1.0);
    return lerpDouble(minRx, maxRx, t)!;
  }

  @override
  bool operator ==(Object other) =>
      other is OrbitGeometry && other.center == center && other.minRx == minRx && other.maxRx == maxRx;

  @override
  int get hashCode => Object.hash(center, minRx, maxRx);
}

/// 一颗星球在纵深微倾轨道上的位置与远近。
///
/// 轨道是后倾平放并带克制微倾角的椭圆（太阳在焦点上），近日点朝向观众、在屏幕下侧，远日点在上侧。
/// 按开普勒方程走：平近点角匀速增加，离太阳近时转得快、远时转得慢；外圈周期按半长轴的 1.5 次方变长。
class OrbitPose {
  const OrbitPose(this.position, this.depth, this.scale, this.haze);

  /// 轨道离心率：0.22，近日点和远日点的速度差约 1.6 倍，看得出快慢又不夸张。
  static const eccentricity = 0.22;

  /// 透视的“视距”（以最外圈半长轴为 1）：2.6 时外圈最近约 1.43 倍、最远约 0.68 倍（约 2.1× 直径比）。
  static const viewDistance = 2.6;

  /// 最外圈的平均角速度（弧度/秒，约 11.9 分钟一圈）；内圈按开普勒第三定律更快。
  static const outerMeanMotion = 0.0088;

  /// 拖拽磁吸捕获半径（逻辑像素）：指针离任意轨道金线不超过该距离即触发磁吸高亮与自动吸附。
  static const snapRadius = 56.0;

  final Offset position;

  /// 远近：正数离屏幕近（下侧），负数远（上侧）；按最外圈半长轴归一，约 -1.22 .. 0.78。
  final double depth;

  /// 近大远小的缩放（外圈约 0.68 .. 1.43）。
  final double scale;

  /// 远处的空气感（0 .. 1）。
  final double haze;

  static double meanMotion(OrbitGeometry orbit, double frac) {
    final b = orbit.radiusFor(frac);
    return outerMeanMotion * math.pow(orbit.maxRx / b, 1.5);
  }

  static double _keplerE(double m) {
    const e = eccentricity;
    var ecc = m;
    for (var i = 0; i < 4; i++) {
      ecc -= (ecc - e * math.sin(ecc) - m) / (1 - e * math.cos(ecc));
    }
    return ecc;
  }

  static OrbitPose onTrack({
    required OrbitGeometry orbit,
    required OrbitTrack track,
    required double phase,
    required double t,
  }) => fromParams(
    orbit: orbit,
    frac: track.frac,
    rollRad: track.rollRad,
    flatten: track.flatten,
    meanAnomaly: phase + meanMotion(orbit, track.frac) * t,
  );

  static OrbitPose fromParams({
    required OrbitGeometry orbit,
    required double frac,
    required double rollRad,
    required double flatten,
    required double meanAnomaly,
  }) => project(orbit, orbit.radiusFor(frac), rollRad, flatten, _keplerE(meanAnomaly));

  /// 轨道上偏近点角为 [ecc] 的点投影到屏幕：真透视 + 克制滚转微倾。
  static OrbitPose project(OrbitGeometry orbit, double b, double rollRad, double flatten, double ecc) {
    const e = eccentricity;
    final sq = math.sqrt(1 - e * e);
    final a = b / sq;
    final aOuter = orbit.maxRx / sq;
    // 轨道平面坐标：z 朝向观众（近日点在 +z），x 屏幕左右；太阳在原点。顺时针转（下侧往左走）。
    final z = a * (math.cos(ecc) - e);
    final x = -b * math.sin(ecc);
    final depth = z / aOuter;
    final scale = viewDistance / (viewDistance - depth);
    final xp = x * math.cos(rollRad);
    final yp = z * flatten + x * math.sin(rollRad);
    final haze = (-depth).clamp(0.0, 1.0);
    return OrbitPose(orbit.center + Offset(xp * scale, yp * scale), depth, scale, haze);
  }

  static OrbitPose projectTrack(OrbitGeometry orbit, OrbitTrack track, double ecc) =>
      project(orbit, orbit.radiusFor(track.frac), track.rollRad, track.flatten, ecc);

  /// 外圈左右半宽：收进 43% 窗宽留出星球与土星环的安全边距，同时保证上下最远/最近点不碰顶底 UI。
  static double fitOuterRx({required double width, required double topRoom, required double bottomRoom}) {
    const e = eccentricity;
    final outer = OrbitTrack.all.last;
    final sq = math.sqrt(1 - e * e);
    final far = (1 + e) * outer.flatten * viewDistance / (viewDistance + 1 + e);
    final near = (1 - e) * outer.flatten * viewDistance / (viewDistance - 1 + e);
    final aMax = math.min(topRoom / far, bottomRoom / near);
    return math.min(width * 0.43, aMax * sq).clamp(220.0, 760.0);
  }

  /// 每条轨道边缘容量徽章（如 `2/3`、`3/4`、`4/7`）的锚点：取在每条环的右侧外缘，
  /// 三条环横纵天然错开，彼此不重叠、也不碰太阳和角落胶囊。
  static Offset badgeAnchor(OrbitGeometry orbit, OrbitTrack track) => projectTrack(orbit, track, -math.pi / 2).position;

  /// 计算屏幕坐标 [pointer] 离三条固定轨道哪一条最近，以及是否进入磁吸区 [snapRadius]。
  /// 先粗采 72 点定位区间，再做 4 轮局部细分，亚像素精度且每次拖动只花微秒级 CPU。
  static OrbitSnapTarget nearestTrack(OrbitGeometry orbit, Offset pointer, {double radius = snapRadius}) {
    OrbitTrack bestTrack = OrbitTrack.all.first;
    var bestEcc = 0.0;
    var bestPose = projectTrack(orbit, bestTrack, 0);
    var bestDistSq = double.infinity;

    const coarse = 72;
    const step = (math.pi * 2) / coarse;
    for (final tk in OrbitTrack.all) {
      var localEcc = 0.0;
      var localDistSq = double.infinity;
      for (var i = 0; i < coarse; i++) {
        final ecc = i * step;
        final p = projectTrack(orbit, tk, ecc);
        final dSq = (p.position - pointer).distanceSquared;
        if (dSq < localDistSq) {
          localDistSq = dSq;
          localEcc = ecc;
        }
      }
      // 局部三分/网格细化 4 轮，让拖拽沿着轨道滑动时完全丝滑不跳动。
      var span = step;
      for (var iter = 0; iter < 4; iter++) {
        span *= 0.45;
        for (final delta in [-span, span]) {
          final cand = localEcc + delta;
          final p = projectTrack(orbit, tk, cand);
          final dSq = (p.position - pointer).distanceSquared;
          if (dSq < localDistSq) {
            localDistSq = dSq;
            localEcc = cand;
          }
        }
      }
      if (localDistSq < bestDistSq) {
        bestDistSq = localDistSq;
        bestTrack = tk;
        bestEcc = localEcc;
        bestPose = projectTrack(orbit, tk, localEcc);
      }
    }

    final dist = math.sqrt(bestDistSq);
    return OrbitSnapTarget(track: bestTrack, ecc: bestEcc, pose: bestPose, distance: dist, snapped: dist <= radius);
  }
}
