import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/protocol/models.dart';
import 'package:ip_app/src/star_map.dart';
import 'package:ip_app/src/ui/shell/orbit.dart';

BotBody _body(String id, {double urgency = 0.0, double importance = 0.5, bool working = false}) => BotBody(
  agent: Agent(id: id, role: 'worker', name: id),
  celestial: planets.first,
  urgency: urgency,
  importance: importance,
  reasons: const [Reason('空闲', 0)],
  importanceNotes: const [],
  focusThreadId: null,
  working: working,
);

void main() {
  test('三条固定轨道容量上限为 内3 / 中4 / 外7，常规自动分轨按状态归位', () {
    expect(OrbitTrack.all.map((t) => t.capacity), [3, 4, 7]);
    expect(OrbitTrack.totalCapacity, 14);

    final bodies = [
      _body('u1', urgency: 0.9),
      _body('u2', urgency: 0.7),
      _body('m1', urgency: 0.4),
      _body('m2', working: true),
      _body('o1'),
      _body('o2'),
    ];
    final layout = OrbitTrack.assign(bodies);
    expect(layout.counts, [2, 2, 2]);
    expect(layout.slots['u1']!.track.id, 0);
    expect(layout.slots['m2']!.track.id, 1);
    expect(layout.slots['o1']!.track.id, 2);
  });

  test('方案 1A：内环已满 3/3 再拖入第 4 颗时，自动把优先级最低的一颗挤出到中环，并产出后端同步变更', () {
    final bodies = [
      _body('a', urgency: 0.90),
      _body('b', urgency: 0.80),
      _body('c', urgency: 0.68), // 内环里紧急度最低，应被挤出
      _body('d', urgency: 0.00), // 外环，被用户手动拖入内环
    ];
    final outcome = OrbitTrack.applyDrop(
      bodies: bodies,
      currentPins: const {},
      agentId: 'd',
      targetTrackId: 0,
      nowMs: 1000,
    );

    expect(outcome.layout.counts, [3, 1, 0]);
    expect(outcome.layout.slots['d']!.track.id, 0);
    expect(outcome.layout.slots['a']!.track.id, 0);
    expect(outcome.layout.slots['b']!.track.id, 0);
    expect(outcome.layout.slots['c']!.track.id, 1);

    // 变更明细包含用户拖入的 d 和被挤出的 c，供后端批量持久化
    expect(outcome.changes.length, 2);
    expect(outcome.changes.first.agentId, 'd');
    expect(outcome.changes.first.evicted, isFalse);
    expect(outcome.changes.first.toTrackId, 0);
    expect(outcome.changes.last.agentId, 'c');
    expect(outcome.changes.last.evicted, isTrue);
    expect(outcome.changes.last.fromTrackId, 0);
    expect(outcome.changes.last.toTrackId, 1);
    // 被挤出的星球也被写入 pin，防止下一次 build 又被自动紧急度拉回内环
    expect(outcome.pins['c']?.trackId, 1);
  });

  test('级联挤出：内环 3/3 且中环 4/4 时再拖入内环，顺次向中环、外环级联挤出', () {
    final bodies = [
      _body('i1', urgency: 0.95),
      _body('i2', urgency: 0.85),
      _body('i3', urgency: 0.70),
      _body('m1', urgency: 0.55),
      _body('m2', urgency: 0.45),
      _body('m3', urgency: 0.35),
      _body('m4', urgency: 0.25),
      _body('o1', urgency: 0.00),
    ];
    final outcome = OrbitTrack.applyDrop(
      bodies: bodies,
      currentPins: const {},
      agentId: 'o1',
      targetTrackId: 0,
      nowMs: 2000,
    );

    expect(outcome.layout.counts, [3, 4, 1]);
    expect(outcome.layout.slots['o1']!.track.id, 0);
    expect(outcome.layout.slots['i3']!.track.id, 1);
    expect(outcome.layout.slots['m4']!.track.id, 2);
    expect(outcome.changes.map((c) => '${c.agentId}:${c.fromTrackId}->${c.toTrackId}'), [
      'o1:2->0',
      'i3:0->1',
      'm4:1->2',
    ]);
  });

  test('手动固定的先后顺序：内环全是手动固定时，最早放入的先被挤出', () {
    final bodies = [_body('p1'), _body('p2'), _body('p3'), _body('p4')];
    final pins = {
      'p1': const OrbitPin(trackId: 0, pinnedAtMs: 100), // 最早放入
      'p2': const OrbitPin(trackId: 0, pinnedAtMs: 200),
      'p3': const OrbitPin(trackId: 0, pinnedAtMs: 300),
    };
    final outcome = OrbitTrack.applyDrop(
      bodies: bodies,
      currentPins: pins,
      agentId: 'p4',
      targetTrackId: 0,
      nowMs: 400,
    );
    expect(outcome.layout.slots['p4']!.track.id, 0);
    expect(outcome.layout.slots['p1']!.track.id, 1);
    expect(outcome.layout.counts, [3, 1, 0]);
  });

  test('超过 14 位员工时，外环软兜底不丢弃任何员工', () {
    final bodies = [for (var i = 0; i < 16; i++) _body('a$i')];
    final layout = OrbitTrack.assign(bodies);
    expect(layout.slots.length, 16);
    expect(layout.counts.reduce((a, b) => a + b), 16);
  });

  test('拖拽磁吸计算：靠近某条轨道进入捕获半径即准确吸附到该环', () {
    const orbit = OrbitGeometry(center: Offset(764, 397), minRx: 342, maxRx: 657);
    for (final track in OrbitTrack.all) {
      final pointOnTrack = OrbitPose.projectTrack(orbit, track, 0.7).position;
      final nearPoint = pointOnTrack + const Offset(8, -10);
      final snap = OrbitPose.nearestTrack(orbit, nearPoint);
      expect(snap.snapped, isTrue);
      expect(snap.track.id, track.id);
      expect(snap.distance, lessThan(15));
    }
    // 拖到遥远角落时不误吸
    final farSnap = OrbitPose.nearestTrack(orbit, const Offset(10, 10));
    expect(farSnap.snapped, isFalse);
  });

  test('OrbitPin JSON 往返兼容后端字段约定', () {
    const pin = OrbitPin(trackId: 1, pinnedAtMs: 1764200000000);
    final json = pin.toJson();
    expect(json, {'orbit_track': 1, 'orbit_pinned_at': 1764200000000});
    expect(OrbitPin.fromJson(json), pin);
    expect(OrbitPin.fromJson({'orbit_track': 9}), isNull);
  });
}
