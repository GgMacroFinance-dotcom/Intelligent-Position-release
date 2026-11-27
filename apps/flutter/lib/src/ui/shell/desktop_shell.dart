// 桌面外壳（设计重构，2026-11-26 维护者定）：
//   - 宽窗口底图：程序生成的文艺复兴字符画蓝天 + 金色弧光 + 飘动字符云（cosmos.dart）。
//   - 天体层（celestial_body.dart + orbit.dart）：中心是太阳（幕僚长），其他员工化作不同形态的字符画行星，
//     沿三条相对固定的通用纵深轨道围绕太阳公转（2026-11-26）：
//       * 内环（0 · 紧急/核心，上限 3）、中环（1 · 进行中/活跃，上限 4）、外环（2 · 空闲/待命，上限 7，超 14 人软兜底）；
//       * 轨道与具体星球解耦：默认按实时状态动态入轨，也支持直接按住任意星球拖拽到目标环附近磁吸落位；
//       * 拖动时在三条环边缘浮现 `1/3`、`2/3`、`3/3` 等容量徽章（平时隐藏保持干净）；满环拖入时按 1A 规则自动把最旧/最不急的星球平滑挤向外侧轨道；
//       * 后端适配与协议字段约定集中在 orbit.dart 顶部注释与 _syncOrbitChangesToBackend 钩子里。
//     所有天体与天空共享单一轻量时钟 CelestialClock（前台 20fps / 失焦 3fps / 窗口或待办展开时彻底 0fps）。
//   - 窗顶联动：点任意星球滑出浮空 macOS Liquid Glass 三栏工作窗时，太阳和全部员工星球一起平滑缩小、
//     横排浮在窗口上沿露出的蓝天条里，当前选中的高亮，点旁边另一颗星球无需关窗即可无缝切换员工。
//   - 完成待审批机制（2026-11-27，1C + 2 + 3B + 4 + 5A，见 completion_approval.dart）：
//     当员工任务完成等待审批时，星球缩小并平滑离轨停泊到页面左中下底栏（左下连接胶囊右侧），释放 3/4/7 轨道槽位、
//     锁定不可拖拽；与左上角红绿灯待办数据同源；点击左中下缩小星球直接滑出三栏员工大窗审批，通过后解除手动固定并飞回轨道。
//   - 角落：左上角 Apple Liquid Glass 红绿灯待办小组件；左下角 Liquid Glass“我”与连接状态；
//     右下角 Liquid Glass「+ 新建 Bot」（方案 1：9 种实机 GPU 星球选星新建，严格排除太阳）与暂留的“面板”入口。

import 'dart:async' show Timer;
import 'dart:io' show Platform;
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../app_state.dart';
import '../../protocol/client.dart';
import '../../protocol/models.dart';
import '../../star_map.dart';
import '../common.dart';
import 'agent_panel.dart';
import 'agent_sheet.dart';
import 'celestial_body.dart';
import 'completion_approval.dart';
import 'cosmos.dart';
import 'create_bot_dialog.dart';
import 'inbox_widget.dart';
import 'local_bot.dart';
import 'orbit.dart';
import 'planet_importance.dart';
import 'settings.dart';
import 'tokens.dart';

class DesktopShell extends StatefulWidget {
  const DesktopShell({super.key, required this.onOpenThread});

  /// 打开一个对话（会顺带记下来：窗口变窄退回原来的导航时直接打开它）。
  final void Function(String threadId) onOpenThread;

  @override
  State<DesktopShell> createState() => _DesktopShellState();
}

/// 浮空窗口里放的是什么：某一位员工的界面（点星球），还是待办、任务这些（点“面板”）。
enum _SheetView { agent, workspace }

class _DesktopShellState extends State<DesktopShell> {
  /// 全局所有天体与椭圆公转共用的单一轻量时钟。
  late final _clock = CelestialClock();

  /// 浮空窗口开着没有。
  bool _open = false;

  /// 浮空窗口是否处于绿灯切换的“大视野高度（86%）”。
  bool _sheetExpanded = false;

  /// 左上角待办小组件展开了没有。
  bool _inboxExpanded = false;

  /// 右下角「新建 Bot」居中 Liquid Glass 弹窗开着没有（由原“星球形态”胶囊升级合并，支持选星预览与新建）。
  bool _createBotOpen = false;

  /// 当前鼠标悬停在哪颗星球上（用于亮起标签并展开简短角色/状态副标题）。
  String? _hoveredBodyId;

  /// 天穹主舞台的坐标锚点（把拖动时的全局指针位置稳定换算成舞台内逻辑坐标，不受被拖星球自身位移影响）。
  final _stageKey = GlobalKey();

  /// 左下与右下各相邻的两块 Liquid Glass 胶囊分别共用局部 BackdropGroup（只模糊各自 ~200×38 的紧致区域，不跨全屏模糊）。
  final _leftGlassKey = BackdropKey();
  final _rightGlassKey = BackdropKey();

  /// 正在进行中的星球拖拽换轨状态（为 null 表示平时静止/公转态，轨道容量徽章隐藏）。
  _OrbitDragState? _drag;

  /// 按下后移动不足 [_dragThreshold] 逻辑像素前的暂存态：用来区分“点击开窗（容忍鼠标微抖）”与“拖拽换轨”。
  static const _dragThreshold = 6.0;
  _PendingPlanetDrag? _pendingDrag;

  /// 缓存最近一帧的轨道几何与在轨全员列表，供拖拽回调实时计算磁吸与满环挤出预览。
  OrbitGeometry? _lastOrbit;
  List<BotBody> _lastBodies = const [];
  OrbitLayout? _lastLayout;

  /// 左中下待审批胶囊上方的悬停展开浮层是否打开（含轻延时防误触与移出缓冲，支持长员工名完整展示与快捷审批）。
  bool _approvalPopoverOpen = false;
  Timer? _popoverOpenTimer;
  Timer? _popoverCloseTimer;

  /// 方案 A 滚轮调重要度时的极轻瞬态提示（仅滚动当下及停滚后 ~1.1 秒内显示，平时零常驻元素）。
  String? _zoomHudAgentId;
  Timer? _zoomHudTimer;

  /// 滚轮停手防抖定时器（~300ms 后触发预留的 [AppState.syncManualImportance]，后端暂不执行）。
  final _zoomSyncTimers = <String, Timer>{};

  _SheetView _view = _SheetView.workspace;

  /// 员工界面的右栏（Inspector）开着没有。
  bool _agentDetailOpen = true;

  /// 工作台窗里在看哪一页。先开待办（和手机首页一样）。
  PanelSection _section = PanelSection.inbox;

  /// 看的是哪位员工（员工界面、“面板”的右栏）；为空是幕僚长（或空位）。
  String? _agent;

  /// 右栏（员工详情）开着没有：换页时按那一页的默认，之后可以手动收起、展开。
  bool _detailOpen = PanelSection.inbox.detailByDefault;

  @override
  void dispose() {
    _popoverOpenTimer?.cancel();
    _popoverCloseTimer?.cancel();
    _zoomHudTimer?.cancel();
    for (final timer in _zoomSyncTimers.values) {
      timer.cancel();
    }
    _zoomSyncTimers.clear();
    _clock.dispose();
    super.dispose();
  }

  void _onApprovalHoverEnter() {
    _popoverCloseTimer?.cancel();
    _popoverCloseTimer = null;
    if (_approvalPopoverOpen || _open || _createBotOpen) return;
    _popoverOpenTimer ??= Timer(const Duration(milliseconds: 120), () {
      _popoverOpenTimer = null;
      if (mounted && !_open && !_createBotOpen) {
        setState(() => _approvalPopoverOpen = true);
      }
    });
  }

  void _onApprovalHoverExit() {
    _popoverOpenTimer?.cancel();
    _popoverOpenTimer = null;
    if (!_approvalPopoverOpen) return;
    _popoverCloseTimer?.cancel();
    _popoverCloseTimer = Timer(const Duration(milliseconds: 220), () {
      _popoverCloseTimer = null;
      if (mounted && _approvalPopoverOpen) {
        setState(() => _approvalPopoverOpen = false);
      }
    });
  }

  void _closeApprovalPopover() {
    _popoverOpenTimer?.cancel();
    _popoverOpenTimer = null;
    _popoverCloseTimer?.cancel();
    _popoverCloseTimer = null;
    if (_approvalPopoverOpen) {
      setState(() => _approvalPopoverOpen = false);
    }
  }

  void _toggleApprovalPopoverNow() {
    _popoverOpenTimer?.cancel();
    _popoverOpenTimer = null;
    _popoverCloseTimer?.cancel();
    _popoverCloseTimer = null;
    setState(() => _approvalPopoverOpen = !_approvalPopoverOpen);
  }

  StarMap _map(AppState state) => buildStarMap(
    agents: state.allAgents,
    roles: state.allRoles,
    tasks: state.tasks,
    approvals: state.approvals,
    working: state.agentsWorking,
    failures: state.agentFailures,
    now: DateTime.now().millisecondsSinceEpoch,
    reworkAgents: state.completionReworkAgents,
    celestialOverrides: state.celestialOverrides,
    importanceOverrides: state.importanceOverrideValues,
  );

  /// 太阳选中：浮空窗口开着、在看幕僚长（或空位）的界面。
  bool _sunSelected(StarMap map) =>
      _open && _view == _SheetView.agent && (_agent == null || _agent == map.chief?.agent.id);

  /// 某颗员工行星选中：浮空窗口开着、且正看这位员工。
  bool _planetSelected(String agentId) => _open && _view == _SheetView.agent && _agent == agentId;

  void _toggleSun() {
    final state = AppScope.read(context);
    final map = _map(state);
    if (_sunSelected(map)) {
      _close();
      return;
    }
    setState(() {
      _open = true;
      _inboxExpanded = false;
      _createBotOpen = false;
      _view = _SheetView.agent;
      _agent = null;
    });
    final chief = map.chief;
    if (chief != null) openAgentThread(state, map, chief.agent.id);
  }

  void _togglePlanet(String agentId) {
    final state = AppScope.read(context);
    final pendingApproval = state.completionApprovalForAgent(agentId);
    if (_planetSelected(agentId)) {
      _close();
      return;
    }
    setState(() {
      _open = true;
      _inboxExpanded = false;
      _createBotOpen = false;
      _view = _SheetView.agent;
      _agent = agentId;
    });
    if (!agentId.startsWith(_demoPrefix) && !state.isLocalBot(agentId)) {
      final tid = pendingApproval?.threadId;
      if (tid != null) {
        widget.onOpenThread(tid);
      } else {
        openAgentThread(state, _map(state), agentId);
      }
    }
  }

  /// 方案 A：鼠标悬停在轨道星球上滚动滚轮/双指上下滑，平滑放大或缩小并更新该星球重要度。
  /// 向上滚（[deltaY] < 0）放大提权，向下滚（[deltaY] > 0）缩小降权；太阳与待审批泊位星球不触发。
  void _onPlanetScrollZoom(String agentId, double currentImportance, double deltaY) {
    if (_open || _createBotOpen || deltaY == 0) return;
    final state = AppScope.read(context);
    // 单次滚轮步进限幅 ±14%，约 8~10 格滚轮从最小平滑走到最大，触控板微滑也细腻可控。
    final step = (-deltaY / 420.0).clamp(-0.14, 0.14);
    final next = (currentImportance + step).clamp(0.0, 1.0);
    state.setManualImportance(agentId, next, notifySync: false);

    // 仅在滚动当下短暂显示“重要度 XX% · 顺位 #N”，停滚 1.1 秒后自动淡出归静。
    _zoomHudTimer?.cancel();
    if (_zoomHudAgentId != agentId) {
      setState(() => _zoomHudAgentId = agentId);
    }
    _zoomHudTimer = Timer(const Duration(milliseconds: 1100), () {
      if (mounted && _zoomHudAgentId == agentId) {
        setState(() => _zoomHudAgentId = null);
      }
    });

    // 预留同步接口防抖：停滚 ~300ms 后再通知 ImportanceSyncPort（后端未接入时为空操作）。
    _zoomSyncTimers[agentId]?.cancel();
    _zoomSyncTimers[agentId] = Timer(const Duration(milliseconds: 300), () {
      _zoomSyncTimers.remove(agentId);
      if (mounted) {
        AppScope.read(context).syncManualImportance(agentId);
      }
    });
  }

  /// 从左中下待审批停泊区、上方展开浮层或左上角待办卡片点开对应员工的三栏工作大窗（3B）。
  void _openAgentFromApproval(String agentId) {
    final state = AppScope.read(context);
    final pendingApproval = state.completionApprovalForAgent(agentId);
    _popoverOpenTimer?.cancel();
    _popoverOpenTimer = null;
    _popoverCloseTimer?.cancel();
    _popoverCloseTimer = null;
    setState(() {
      _open = true;
      _inboxExpanded = false;
      _createBotOpen = false;
      _approvalPopoverOpen = false;
      _view = _SheetView.agent;
      _agent = agentId;
    });
    if (!agentId.startsWith(_demoPrefix) && !state.isLocalBot(agentId)) {
      final tid = pendingApproval?.threadId;
      if (tid != null) {
        widget.onOpenThread(tid);
      } else {
        openAgentThread(state, _map(state), agentId);
      }
    }
  }

  void _openPanel() => setState(() {
    _open = true;
    _inboxExpanded = false;
    _createBotOpen = false;
    _view = _SheetView.workspace;
  });

  void _toggleCreateBot() => setState(() {
    _createBotOpen = !_createBotOpen;
    if (_createBotOpen) {
      _open = false;
      _sheetExpanded = false;
      _inboxExpanded = false;
      _approvalPopoverOpen = false;
    }
  });

  void _onLocalBotCreated(LocalBotRecord record) {
    setState(() {
      _createBotOpen = false;
    });
    // 创建完成后直接回到天穹看到新星球以手选形态落入外环，同时轻提示说明本地演示态。
    ScaffoldMessenger.maybeOf(context)?.showSnackBar(
      SnackBar(
        content: Text('已将「${record.agent.name}」（${record.celestial.name}）添加到星图外环（本地演示）'),
        behavior: SnackBarBehavior.floating,
        duration: const Duration(seconds: 3),
      ),
    );
  }

  void _close() => setState(() {
    _open = false;
    _sheetExpanded = false;
  });

  void _toggleExpand() => setState(() => _sheetExpanded = !_sheetExpanded);

  Offset? _globalToStage(Offset global) {
    final box = _stageKey.currentContext?.findRenderObject() as RenderBox?;
    return box?.globalToLocal(global);
  }

  /// 开始按住某颗星球（此时还不进入拖拽态，防止点击时的 1~5px 鼠标微抖误触发拖动）。
  void _onPlanetDragStart(String agentId, DragStartDetails details) {
    if (_open || _createBotOpen) return;
    final stagePos = _globalToStage(details.globalPosition);
    final orbit = _lastOrbit;
    final slot = _lastLayout?.slots[agentId];
    if (stagePos == null || orbit == null || slot == null) return;
    final pose = OrbitPose.onTrack(orbit: orbit, track: slot.track, phase: slot.phase, t: _clock.value - 40.0);
    _pendingDrag = _PendingPlanetDrag(
      agentId: agentId,
      originTrackId: slot.track.id,
      startGlobal: details.globalPosition,
      grabOffset: pose.position - stagePos,
      startScale: pose.scale,
    );
  }

  /// 拖拽移动：超过 6px 后正式进入拖拽态，按指针位置实时计算三条环的磁吸与满环挤出预览。
  void _onPlanetDragUpdate(String agentId, DragUpdateDetails details) {
    final orbit = _lastOrbit;
    if (orbit == null) return;

    var drag = _drag;
    if (drag == null) {
      final pending = _pendingDrag;
      if (pending == null || pending.agentId != agentId) return;
      if ((details.globalPosition - pending.startGlobal).distance < _dragThreshold) return;
      drag = _buildDragState(
        orbit: orbit,
        agentId: agentId,
        originTrackId: pending.originTrackId,
        grabOffset: pending.grabOffset,
        fallbackScale: pending.startScale,
        globalPos: details.globalPosition,
      );
      _pendingDrag = null;
      if (drag != null) setState(() => _drag = drag);
      return;
    }

    if (drag.agentId != agentId) return;
    final next = _buildDragState(
      orbit: orbit,
      agentId: agentId,
      originTrackId: drag.originTrackId,
      grabOffset: drag.grabOffset,
      fallbackScale: drag.fallbackScale,
      globalPos: details.globalPosition,
    );
    if (next != null) setState(() => _drag = next);
  }

  /// 松手：若未真正拖动则当作普通点击；若在磁吸区内则提交换轨（满环触发 1A 级联挤出），否则平滑弹回原轨。
  void _onPlanetDragEnd(String agentId, DragEndDetails details) {
    final pending = _pendingDrag;
    if (pending != null && pending.agentId == agentId) {
      _pendingDrag = null;
      _togglePlanet(agentId);
      return;
    }
    final drag = _drag;
    if (drag == null || drag.agentId != agentId) return;

    if (drag.snap.snapped) {
      final state = AppScope.read(context);
      final nowMs = DateTime.now().millisecondsSinceEpoch;
      final outcome = OrbitTrack.applyDrop(
        bodies: _lastBodies,
        currentPins: state.manualOrbitPins,
        agentId: agentId,
        targetTrackId: drag.snap.track.id,
        nowMs: nowMs,
      );
      setState(() {
        state.manualOrbitPins
          ..clear()
          ..addAll(outcome.pins);
        _drag = null;
      });
      _syncOrbitChangesToBackend(outcome.changes);
    } else {
      setState(() => _drag = null);
    }
  }

  void _onPlanetDragCancel(String agentId) {
    if (_pendingDrag?.agentId == agentId) _pendingDrag = null;
    if (_drag?.agentId == agentId) setState(() => _drag = null);
  }

  _OrbitDragState? _buildDragState({
    required OrbitGeometry orbit,
    required String agentId,
    required int originTrackId,
    required Offset grabOffset,
    required double fallbackScale,
    required Offset globalPos,
  }) {
    final stagePos = _globalToStage(globalPos);
    if (stagePos == null) return null;
    final pointerCenter = stagePos + grabOffset;
    final snap = OrbitPose.nearestTrack(orbit, pointerCenter);

    // 磁吸手感：进入 snapRadius(56px) 开始温和拉向金线，进入 24px 内完全贴合在金线上顺滑滑动。
    Offset visualCenter = pointerCenter;
    double visualScale = fallbackScale * 1.08;
    OrbitDropOutcome? preview;
    if (snap.snapped) {
      final lock = 1.0 - ((snap.distance - 24.0) / (OrbitPose.snapRadius - 24.0)).clamp(0.0, 1.0);
      final pull = Curves.easeOutCubic.transform(lock);
      visualCenter = Offset.lerp(pointerCenter, snap.pose.position, 0.28 + 0.72 * pull)!;
      visualScale = ui.lerpDouble(fallbackScale, snap.pose.scale, pull)! * 1.08;
      if (snap.track.id != originTrackId) {
        preview = OrbitTrack.applyDrop(
          bodies: _lastBodies,
          currentPins: AppScope.read(context).manualOrbitPins,
          agentId: agentId,
          targetTrackId: snap.track.id,
          nowMs: DateTime.now().millisecondsSinceEpoch,
        );
      }
    }

    return _OrbitDragState(
      agentId: agentId,
      originTrackId: originTrackId,
      grabOffset: grabOffset,
      fallbackScale: fallbackScale,
      visualCenter: visualCenter,
      visualScale: visualScale,
      snap: snap,
      preview: preview,
    );
  }

  /// 后端轨道偏好同步钩子（预留给后端适配）：
  ///
  /// 每次用户拖放落位后触发，[changes] 包含：
  ///   - 被拖入的星球（`evicted == false`）及其最新 `pin`（`orbit_track`, `orbit_pinned_at`）；
  ///   - 因目标环满员（3 / 4 / 7）而被 1A 级联挤出到相邻环的星球（`evicted == true`）及其最新 `pin`。
  ///
  /// TODO(backend-orbit-sync)：
  ///   后端提供轨道持久化接口（建议 `PATCH /star-map/orbit-pins`，body 为
  ///   `{"changes": [{"agent_id": c.agentId, ...c.pin.toJson()}]}`）后，
  ///   过滤掉 `c.agentId.startsWith(_demoPrefix)` 的演示星球，将真实员工的变更批量上报即可。
  void _syncOrbitChangesToBackend(List<OrbitPinChange> changes) {
    assert(() {
      final real = [
        for (final c in changes)
          if (!c.agentId.startsWith(_demoPrefix)) c,
      ];
      if (real.isNotEmpty) {
        final summary = real
            .map((c) => '${c.agentId}:${c.fromTrackId}->${c.toTrackId}${c.evicted ? "(挤出)" : ""}')
            .join(', ');
        debugPrint('[orbit-sync] 待同步后端轨道变更: $summary');
      }
      return true;
    }());
  }

  /// 从左上角待办卡片点“去对话”：有明确员工就直接滑出那位员工的三栏浮空窗，否则进工作台对话页。
  void _openThreadFromInbox(String threadId, {String? agentId}) {
    final state = AppScope.read(context);
    final who = agentId ?? state.threadAgent[threadId];
    widget.onOpenThread(threadId);
    setState(() {
      _inboxExpanded = false;
      _createBotOpen = false;
      _open = true;
      if (who != null) {
        _view = _SheetView.agent;
        _agent = who;
      } else {
        _view = _SheetView.workspace;
        _section = PanelSection.chat;
      }
    });
  }

  void _selectSection(PanelSection p) => setState(() {
    _section = p;
    _detailOpen = p.detailByDefault;
  });

  void _toggleDetail() => setState(() => _detailOpen = !_detailOpen);

  /// 点员工（“面板”左栏）、点 [[关联]]：换成那位员工的界面，打开和它最相关的对话。
  void _selectAgent(String id) {
    final state = AppScope.read(context);
    setState(() {
      _open = true;
      _createBotOpen = false;
      _view = _SheetView.agent;
      _agent = id;
    });
    if (!state.isLocalBot(id)) openAgentThread(state, _map(state), id);
  }

  /// 待办、任务里点了某个对话（或者点了通知）：在“面板”的对话页打开它。
  void _openThread(String threadId) {
    final state = AppScope.read(context);
    widget.onOpenThread(threadId);
    setState(() {
      _open = true;
      _createBotOpen = false;
      _view = _SheetView.workspace;
      _section = PanelSection.chat;
      _agent = state.threadAgent[threadId] ?? _agent;
    });
  }

  /// 员工界面里点了某个对话：就在员工界面里打开。
  void _openAgentThread(String threadId) => widget.onOpenThread(threadId);

  /// 员工界面左栏的“新对话”：开一个发给这位员工的新对话。
  void _newChatWith(String agentId) {
    final state = AppScope.read(context);
    state.startNewChat();
    state.chooseAgent(agentId);
  }

  void _newChat() {
    AppScope.read(context).startNewChat();
    setState(() => _section = PanelSection.chat);
  }

  /// 岗位页点“和 X 对话”：开一个发给这位员工的新对话。
  void _chatWith(String agentId) {
    final state = AppScope.read(context);
    state.startNewChat();
    state.chooseAgent(agentId);
    setState(() {
      _section = PanelSection.chat;
      _agent = agentId;
    });
  }

  void _settings() => showSettings(context);

  /// 点了系统通知：打开面板去那个对话，没有对话的展开左上角待办（F-16）。推进去的页面先关掉。
  void _handlePendingOpen() {
    final state = AppScope.read(context);
    if (state.pendingOpen == null) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final open = state.takePendingOpen();
      if (open == null) return;
      Navigator.of(context).popUntil((r) => r.isFirst);
      final (threadId,) = open;
      if (threadId != null) {
        _openThread(threadId);
      } else {
        setState(() {
          _open = false;
          _sheetExpanded = false;
          _createBotOpen = false;
          _inboxExpanded = true;
        });
      }
    });
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    _handlePendingOpen();
    final map = _map(state);
    final sunSelected = _sunSelected(map);
    final freezeSky = _open || _inboxExpanded;
    final visible = TickerMode.valuesOf(context).enabled;
    final disableAnim = MediaQuery.disableAnimationsOf(context);
    // 只有大窗口/待办展开时停转；打开星球图鉴时保持低帧运行，方便实机观察行星自转与云层流动。
    _clock.sync(frozen: freezeSky, enabled: visible && !disableAnim);

    return CallbackShortcuts(
      bindings: {
        const SingleActivator(LogicalKeyboardKey.comma, meta: true): _settings,
        const SingleActivator(LogicalKeyboardKey.escape): () {
          if (_open) {
            _close();
          } else if (_createBotOpen) {
            setState(() => _createBotOpen = false);
          } else if (_approvalPopoverOpen) {
            _closeApprovalPopover();
          } else if (_inboxExpanded) {
            setState(() => _inboxExpanded = false);
          }
        },
      },
      child: Focus(
        autofocus: true,
        child: Scaffold(
          backgroundColor: Pigment.ground,
          body: LayoutBuilder(
            builder: (context, box) {
              // 规则 2：审批通过后自动解除该员工此前手动拖拽固定的轨道 pin，让其回自动排轨（空闲时落外环）。
              final manualOrbitPins = state.manualOrbitPins;
              for (final entry in state.completionApprovedAtMs.entries) {
                final pin = manualOrbitPins[entry.key];
                if (pin != null && pin.pinnedAtMs <= entry.value) {
                  manualOrbitPins.remove(entry.key);
                }
              }

              final demoBodies = _demoBodiesFor(
                map.bodies,
                reworkAgents: state.completionReworkAgents,
                importanceOverrides: state.importanceOverrideValues,
              );
              final bodies = [...map.bodies, ...demoBodies];
              final agentsSettled = state.host == null || state.agents.isNotEmpty;
              if (_demoApprovalCount > 0 &&
                  !state.demoCompletionApprovalsSeeded &&
                  agentsSettled &&
                  bodies.isNotEmpty) {
                state.demoCompletionApprovalsSeeded = true;
                final seedTargets = bodies.take(_demoApprovalCount).toList();
                WidgetsBinding.instance.addPostFrameCallback((_) {
                  if (!mounted) return;
                  final st = AppScope.read(context);
                  for (var i = 0; i < seedTargets.length; i++) {
                    final b = seedTargets[i];
                    st.triggerDemoCompletionApproval(
                      agentId: b.agent.id,
                      agentName: b.agent.name,
                      taskTitle: i == 0 ? 'Q4 竞品调研简报整理完毕' : '新版本发布检查清单已核验',
                      summary: i == 0 ? '已整理 6 家竞品动态并输出结论摘要，等待你的审批通过后归档。' : '全部 12 项上线前校验均已通过，请确认审批放行。',
                    );
                  }
                });
              }

              final bodyById = {for (final b in bodies) b.agent.id: b};
              final completionList = state.pendingCompletionApprovals;
              final completionByAgent = CompletionApproval.primaryByAgent(completionList);
              final dockedEntries = <({BotBody body, CompletionApproval approval})>[
                for (final entry in completionByAgent.entries)
                  if (bodyById[entry.key] case final b?) (body: b, approval: entry.value),
              ];
              final dockedIds = {for (final e in dockedEntries) e.body.agent.id};
              // 规则 2：进入「完成 · 待审批」的星球暂时脱离三条公转轨道，释放 3/4/7 容量槽位。
              final orbitBodies = [
                for (final b in bodies)
                  if (!dockedIds.contains(b.agent.id)) b,
              ];

              final strip = FrostedSheet.topFor(box.maxHeight, expanded: _sheetExpanded);
              final centerX = box.maxWidth / 2;
              final centerY = box.maxHeight * 0.48;

              // 三条固定通用轨道的几何：太阳居中，外圈留足左右/上下安全边距（不贴窗边、不压待办和底部胶囊），
              // 内圈最小半宽保底避开太阳本体；三条轨道按 [0.52, 0.76, 1.00] 分布并各自微倾。
              final maxRx = OrbitPose.fitOuterRx(
                width: box.maxWidth,
                topRoom: centerY - 96,
                bottomRoom: box.maxHeight - centerY - 118,
              );
              final minRx = math.min(math.max(_sunRadius + 130, maxRx * OrbitTrack.all.first.frac), maxRx * 0.72);
              final orbit = OrbitGeometry(center: Offset(centerX, centerY), minRx: minRx, maxRx: maxRx);

              // 按手动拖放覆盖 + 实时状态 + 3/4/7 容量上限，只为在轨员工分配轨道槽位（待审批星球已离轨释放槽位）。
              final layout = OrbitTrack.assign(orbitBodies, manualPins: manualOrbitPins);
              _lastOrbit = orbit;
              _lastBodies = orbitBodies;
              _lastLayout = layout;

              // 预留 AI 资源分配与先后顺位计算器：按有效重要度与轨道实时算出 #1..#N 顺位（滚动时即显顺位微提示）。
              final trackByAgent = <String, int>{
                for (final entry in layout.slots.entries) entry.key: entry.value.track.id,
              };
              final prioritySnapshot = PriorityAllocator.compute(
                bodies: bodies,
                trackByAgent: trackByAgent,
                overrides: state.manualImportanceOverrides,
              );

              // 1C（位置 1）：页面左中下底栏单一孪生胶囊——紧邻左下角“已连接”胶囊右侧 8px，
              // 与“已连接”严格等宽（222px）、等高（38px）、同圆角（20）与同款冷白 Liquid Glass；
              // 胶囊内只显示固定“完成 · 待审批”与计数，永不因长员工名溢出；悬停或点击后在正上方展开 308px 清爽浮层展示完整长名与极轻快捷审批。
              const footerWidth = Sizes.sidebar - 20;
              const dockWidth = footerWidth - 38.0 - 8.0;
              const dockLeft = 16.0 + footerWidth + 8.0;
              const dockBottom = 16.0;
              const dockHeight = 38.0;
              final visibleDocked = dockedEntries.take(3).toList();
              final dockCenterY = box.maxHeight - dockBottom - dockHeight / 2;
              final planetStartX = visibleDocked.length >= 3 ? dockLeft + 19.0 : dockLeft + 20.0;
              final planetStep = visibleDocked.length >= 3 ? 18.0 : 26.0;
              final dockCenterByAgent = <String, Offset>{};
              for (var i = 0; i < visibleDocked.length; i++) {
                dockCenterByAgent[visibleDocked[i].body.agent.id] = Offset(planetStartX + i * planetStep, dockCenterY);
              }
              final dockTextStart = visibleDocked.isEmpty
                  ? 12.0
                  : (planetStartX - dockLeft) + (visibleDocked.length - 1) * planetStep + 17.0;
              final showApprovalPopover = !_open && !_createBotOpen && dockedEntries.isNotEmpty && _approvalPopoverOpen;

              final activeDrag = (!_open && !_createBotOpen) ? _drag : null;
              final usedCelestialCounts = <CelestialStyle, int>{};
              for (final b in bodies) {
                final st = CelestialStyle.fromCelestial(b.celestial);
                usedCelestialCounts[st] = (usedCelestialCounts[st] ?? 0) + 1;
              }
              final snappedTrackId = (activeDrag != null && activeDrag.snap.snapped) ? activeDrag.snap.track.id : null;
              final previewEvictedTo = <String, int>{
                if (activeDrag?.preview != null)
                  for (final c in activeDrag!.preview!.changes)
                    if (c.evicted) c.agentId: c.toTrackId,
              };

              // 窗顶横排天体切换栏布局（窗口打开时，太阳在最左，其余星球严格按“距太阳远近”等距横排）：
              //   太阳 → 内环（0）→ 中环（1）→ 外环（2）→ 已完成待审批（Q1=A，纯等距无多余分隔符 Q2=A）。
              //   包含用户手动拖拽固定到各环的结果；同环内按固定优先、紧急度降序、ID 稳定排序。
              int rankWithinTrack(String aId, String bId) {
                final aPin = manualOrbitPins.containsKey(aId) ? 1 : 0;
                final bPin = manualOrbitPins.containsKey(bId) ? 1 : 0;
                if (aPin != bPin) return bPin.compareTo(aPin);
                final aUrg = bodyById[aId]?.urgency ?? 0.0;
                final bUrg = bodyById[bId]?.urgency ?? 0.0;
                final u = bUrg.compareTo(aUrg);
                if (u != 0) return u;
                return aId.compareTo(bId);
              }

              final topOrderedIds = <String>[];
              final topSeen = <String>{};
              for (final tk in OrbitTrack.all) {
                final group = List<String>.from(layout.membersByTrack[tk.id])..sort(rankWithinTrack);
                for (final id in group) {
                  if (topSeen.add(id)) topOrderedIds.add(id);
                }
              }
              for (final e in dockedEntries) {
                if (topSeen.add(e.body.agent.id)) topOrderedIds.add(e.body.agent.id);
              }
              for (final b in bodies) {
                if (topSeen.add(b.agent.id)) topOrderedIds.add(b.agent.id);
              }
              final topRankByAgent = <String, int>{for (var i = 0; i < topOrderedIds.length; i++) topOrderedIds[i]: i};

              // 太阳坐标与缩放（窗顶保持主角体量且绝不撞上浮空窗顶边）。
              final sunExtent = CelestialBody.extentFor(_sunRadius, CelestialStyle.sun);
              final sunScale = _open ? ((strip - 16) / sunExtent).clamp(0.18, 0.48) : 1.0;
              final sunTopVisualR = _sunRadius * sunScale;
              final topCenterY = strip >= 120 ? strip * 0.43 : strip * 0.50;

              // 除太阳外全部行星统一视觉球体大小 + 严格等距排布：
              // 太阳永远明显大于行星（即使在 86% 扩屏窄天空条里也约束在 0.72× 太阳半径内）；
              // 太阳右侧按“半径差 + 日冕留白”补出光学呼吸缝，使其与第 1 颗行星的净空视觉上等同于行星间净空，其余行星中心距完全一致。
              final planetCount = topOrderedIds.length;
              final topPlanetRBase = strip >= 120
                  ? ((strip - 56) * 0.30).clamp(16.0, 23.5)
                  : math.min(((strip - 26) * 0.30).clamp(12.0, 20.0), sunTopVisualR * 0.72);
              final availTopSpan = math.max(120.0, box.maxWidth - 156.0);
              final roughSunGap = planetCount > 0 ? (sunTopVisualR - topPlanetRBase + 6.0).clamp(8.0, 26.0) : 0.0;
              final topPitch = planetCount == 0 ? 0.0 : ((availTopSpan - roughSunGap) / planetCount).clamp(56.0, 112.0);
              final topPlanetRadius = math.min(topPlanetRBase, (topPitch * 0.28).clamp(14.0, 23.5));
              final sunExtraGap = planetCount > 0 ? (sunTopVisualR - topPlanetRadius + 6.0).clamp(8.0, 26.0) : 0.0;
              final totalTopSpan = planetCount == 0 ? 0.0 : sunExtraGap + planetCount * topPitch;
              final sunTopX = centerX - totalTopSpan / 2;
              double topPlanetXForRank(int rank) => sunTopX + sunExtraGap + (rank + 1) * topPitch;
              // 窗顶所有天体（含太阳与全部行星、星芒恒星）共用同一条名字水平基线，杜绝高低参差或星芒压字。
              final topLabelBaselineY = (topCenterY + math.max(sunTopVisualR + 7.0, topPlanetRadius + 13.0)).clamp(
                8.0,
                strip - 22.0,
              );

              final sunCx = _open ? sunTopX : centerX;
              final sunCy = _open ? topCenterY : centerY;

              final chief = map.chief;
              final sunLabel = chief?.agent.name ?? (state.connected ? '幕僚长' : '幕僚长');
              final sunSub = chief != null
                  ? (chief.working ? '幕僚长 · 正在处理' : '幕僚长 · 太阳')
                  : (state.connected ? '太阳 · 空位' : '未连接主机');
              final sunUrgentColor = chief != null ? _statusDotColor(chief, routingWaiting: map.routingWaiting) : null;
              final allowDemoTrigger = _demoPlanetCount > 0 || _demoApprovalCount > 0;
              final panelMap = (_agent != null && _agent!.startsWith(_demoPrefix))
                  ? StarMap(chief: map.chief, bodies: bodies, links: map.links, routingWaiting: map.routingWaiting)
                  : map;

              return Stack(
                key: _stageKey,
                children: [
                  // 1. 文艺复兴字符画蓝天背景（点击空白天空收起窗口/待办/新建弹窗；打开大玻璃窗或新建弹窗时停帧省电）。
                  Positioned.fill(
                    child: GestureDetector(
                      behavior: HitTestBehavior.opaque,
                      onTap: _open
                          ? _close
                          : _createBotOpen
                          ? () => setState(() => _createBotOpen = false)
                          : _approvalPopoverOpen
                          ? _closeApprovalPopover
                          : _inboxExpanded
                          ? () => setState(() => _inboxExpanded = false)
                          : null,
                      child: TickerMode(
                        enabled: !freezeSky && !_createBotOpen,
                        child: CosmosBackdrop(clock: _clock),
                      ),
                    ),
                  ),

                  // 2. 三条固定通用轨道金线层（统一画在太阳与所有行星后面，球永远在线前面；
                  //    平时按外环包围盒紧致离屏缓存成 1 张 Retina 贴图，20fps 公转零矢量细分开销；拖动时目标环亮起并实时画磁吸落点）。
                  if (!_open && orbitBodies.isNotEmpty)
                    Positioned.fill(
                      key: const ValueKey('orbit-tracks'),
                      child: IgnorePointer(
                        child: _CachedOrbitTracks(
                          orbit: orbit,
                          dragActive: activeDrag != null,
                          highlightedTrackId: snappedTrackId,
                          snapPoint: snappedTrackId != null ? activeDrag!.snap.pose.position : null,
                        ),
                      ),
                    ),

                  // 2.5 1C（位置 1）：左中下「完成 · 待审批」单一孪生玻璃胶囊（与左下“已连接”严格等宽 222×38、圆角 20；
                  //     画在星球层后面，缩小后的真实 GPU 星球落在胶囊左侧；悬停/点击后在正上方浮出清爽待审批列表）。
                  if (!_open && visibleDocked.isNotEmpty)
                    Positioned(
                      key: const ValueKey('completion-approval-dock'),
                      left: dockLeft,
                      bottom: dockBottom,
                      child: MouseRegion(
                        onEnter: (_) => _onApprovalHoverEnter(),
                        onExit: (_) => _onApprovalHoverExit(),
                        child: _CompletionApprovalDock(
                          totalCount: dockedEntries.length,
                          width: dockWidth,
                          height: dockHeight,
                          textStart: dockTextStart,
                          popoverOpen: showApprovalPopover,
                          onTap: _toggleApprovalPopoverNow,
                        ),
                      ),
                    ),

                  // 3. 太阳与员工行星：按远近排先后（远的先画、被太阳和近的挡住），待审批停泊星球浮于 1C 玻璃槽之上，拖拽中置顶；
                  //    公转只听共享时钟 _clock，不触发整棵 DesktopShell 重建；子项带稳定 key，换序或离轨/回轨绝不重建着色器。
                  Positioned.fill(
                    // 带稳定 key：前面的轨道层/待审停泊托盘显隐时，这一层（连同太阳、行星的状态）不会被错认、重建。
                    key: const ValueKey('orbit-system'),
                    child: _DepthSortedOrbit(
                      clock: _clock,
                      draggingAgentId: activeDrag?.agentId,
                      planets: [
                        for (var idx = 0; idx < bodies.length; idx++)
                          if (!dockedIds.contains(bodies[idx].agent.id) ||
                              dockCenterByAgent.containsKey(bodies[idx].agent.id) ||
                              _open)
                            () {
                              final body = bodies[idx];
                              final id = body.agent.id;
                              final slot = layout.slots[id];
                              final dockCenter = dockCenterByAgent[id];
                              final isDocked = dockCenter != null;
                              final isDragged = !isDocked && activeDrag?.agentId == id;
                              final style = CelestialStyle.fromCelestial(body.celestial);
                              final dockRadius = style == CelestialStyle.ringed ? 8.8 : 11.0;
                              final dockScale = (dockRadius / _OrbitingPlanetItem.bakeRadius).clamp(0.24, 0.50);
                              final alloc = prioritySnapshot.of(id);
                              final zoomPill = (!_open && !isDocked && _zoomHudAgentId == id && alloc != null)
                                  ? '重要度 ${importancePercent(body.importance)}% · 顺位 #${alloc.rank}'
                                  : null;
                              return _OrbitingPlanetItem(
                                key: ValueKey('planet-item-$id'),
                                body: body,
                                track: slot?.track,
                                slotPhase: slot?.phase,
                                roles: state.allRoles,
                                clock: _clock,
                                sheetOpen: _open,
                                selected: _planetSelected(id),
                                hovered: _hoveredBodyId == id,
                                frozen: freezeSky || _createBotOpen,
                                orbit: orbit,
                                stripHeight: strip,
                                topX: topPlanetXForRank(topRankByAgent[id] ?? idx),
                                topY: topCenterY,
                                topPlanetRadius: topPlanetRadius,
                                topLabelY: topLabelBaselineY,
                                dragCenter: isDragged ? activeDrag!.visualCenter : null,
                                dragScale: isDragged ? activeDrag!.visualScale : null,
                                approvalDockCenter: dockCenter,
                                approvalDockScale: isDocked ? dockScale : null,
                                evictTargetTrackId: previewEvictedTo[id],
                                zoomPillText: zoomPill,
                                onHoverChanged: (h) {
                                  setState(() => _hoveredBodyId = h ? id : null);
                                  if (isDocked) {
                                    h ? _onApprovalHoverEnter() : _onApprovalHoverExit();
                                  }
                                },
                                onTap: () => isDocked ? _openAgentFromApproval(id) : _togglePlanet(id),
                                // 方案 A：天穹在轨星球支持悬停滚轮平滑放大缩小；太阳与左下泊位星球不触发。
                                onScrollZoom: (_open || isDocked || _createBotOpen)
                                    ? null
                                    : (dy) => _onPlanetScrollZoom(id, body.importance, dy),
                                // 规则 2：待审批期间锁定禁止拖拽，必须先完成审批。
                                onDragStart: (_open || isDocked || _createBotOpen)
                                    ? null
                                    : (d) => _onPlanetDragStart(id, d),
                                onDragUpdate: (_open || isDocked || _createBotOpen)
                                    ? null
                                    : (d) => _onPlanetDragUpdate(id, d),
                                onDragEnd: (_open || isDocked || _createBotOpen)
                                    ? null
                                    : (d) => _onPlanetDragEnd(id, d),
                                onDragCancel: (_open || isDocked || _createBotOpen)
                                    ? null
                                    : () => _onPlanetDragCancel(id),
                                onSecondaryTap:
                                    (!_open &&
                                        !isDocked &&
                                        !_createBotOpen &&
                                        (allowDemoTrigger || state.isLocalBot(id)))
                                    ? () =>
                                          AppScope.read(context)
                                              .triggerDemoCompletionApproval(agentId: id, agentName: body.agent.name)
                                    : null,
                              );
                            }(),
                      ],
                      sun: [
                        AnimatedPositioned(
                          key: const ValueKey('sun-slot'),
                          duration: const Duration(milliseconds: 300),
                          curve: Motion.curve,
                          left: sunCx - sunExtent / 2,
                          top: sunCy - sunExtent / 2,
                          width: sunExtent,
                          height: sunExtent,
                          child: AnimatedScale(
                            duration: const Duration(milliseconds: 300),
                            curve: Motion.curve,
                            scale: sunScale,
                            child: CelestialBody(
                              key: const ValueKey('sun'),
                              radius: _sunRadius,
                              style: CelestialStyle.sun,
                              selected: sunSelected,
                              frozen: freezeSky || _createBotOpen,
                              clock: _clock,
                              onHoverChanged: (h) => setState(() => _hoveredBodyId = h ? '__sun__' : null),
                              semanticsLabel: chief != null
                                  ? '太阳：${chief.agent.name}'
                                  : (state.connected ? '太阳（幕僚长空位）' : '太阳（未连接）'),
                              onTap: _toggleSun,
                            ),
                          ),
                        ),
                        // 太阳的轻量名字标签（窗口合起时在球下方，窗口打开且高度充足时悬在窗顶小球下沿）。
                        if (!_open || strip >= 120)
                          AnimatedPositioned(
                            key: const ValueKey('sun-label-slot'),
                            duration: const Duration(milliseconds: 300),
                            curve: Motion.curve,
                            left: sunCx - 76,
                            top: _open ? topLabelBaselineY : sunCy + _sunRadius + 16,
                            width: 152,
                            child: IgnorePointer(
                              child: _PlanetLabel(
                                name: sunLabel,
                                subtitle: _open ? null : sunSub,
                                emphasized: sunSelected || _hoveredBodyId == '__sun__',
                                compact: _open,
                                dotColor: sunUrgentColor,
                              ),
                            ),
                          ),
                      ],
                    ),
                  ),

                  // 3.5 拖拽换轨时的轨道边缘容量徽章（方案 2A：平时完全隐藏保持天穹干净，开始拖拽星球时淡入显示 1/3、2/3、3/3 等）。
                  if (!_open && !_createBotOpen && bodies.isNotEmpty)
                    Positioned.fill(
                      key: const ValueKey('orbit-capacity-badges'),
                      child: IgnorePointer(
                        child: _OrbitCapacityBadges(
                          orbit: orbit,
                          stageWidth: box.maxWidth,
                          layout: layout,
                          drag: activeDrag,
                        ),
                      ),
                    ),

                  // 3.8 1C 悬停/点击展开的「完成 · 待审批」轻浮层（画在星球层之上，距底部胶囊 8px 内置鼠标无断层桥接；
                  //     308px 宽给足长员工名空间，每行支持整行直开三栏窗 3B，或右侧极轻快捷审批 ✓ / ↩）。
                  if (showApprovalPopover)
                    Positioned(
                      key: const ValueKey('completion-approval-popover'),
                      left: dockLeft,
                      bottom: dockBottom + dockHeight,
                      child: MouseRegion(
                        onEnter: (_) => _onApprovalHoverEnter(),
                        onExit: (_) => _onApprovalHoverExit(),
                        child: Padding(
                          padding: const EdgeInsets.only(bottom: 8),
                          child: _CompletionApprovalPopover(
                            entries: dockedEntries,
                            roles: state.allRoles,
                            maxHeight: math.min(288.0, math.max(156.0, box.maxHeight - 160)),
                            hoveredAgentId: _hoveredBodyId,
                            onRowHoverChanged: (id, h) => setState(
                              () => _hoveredBodyId = h ? id : (_hoveredBodyId == id ? null : _hoveredBodyId),
                            ),
                            onOpenAgent: _openAgentFromApproval,
                          ),
                        ),
                      ),
                    ),

                  // 4. 左上角：Apple Liquid Glass + 红绿灯紧急度待办小组件（与左中下待审批停泊区数据同源；浮空窗打开时卸掉省模糊层）。
                  if (!_open)
                    Positioned(
                      left: 16,
                      top: 16,
                      child: TopLeftInboxWidget(
                        expanded: _inboxExpanded,
                        maxHeight: (box.maxHeight - 86).clamp(240.0, 900.0),
                        onExpandedChanged: (v) => setState(() => _inboxExpanded = v),
                        onOpenThread: _openThreadFromInbox,
                        onOpenAgent: _openAgentFromApproval,
                      ),
                    ),

                  // 6. 平时天穹底部的 Liquid Glass 胶囊：左下“我”与连接状态，右下「+ 新建 Bot」（合并原星球形态预览）与暂留“面板”。
                  if (!_open) ...[
                    Align(
                      alignment: Alignment.bottomLeft,
                      child: Padding(
                        padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
                        child: SizedBox(
                          width: Sizes.sidebar - 20,
                          child: BackdropGroup(
                            backdropKey: _leftGlassKey,
                            child: _Footer(onSettings: _settings, frosted: true),
                          ),
                        ),
                      ),
                    ),
                    Align(
                      alignment: Alignment.bottomRight,
                      child: Padding(
                        padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
                        child: BackdropGroup(
                          backdropKey: _rightGlassKey,
                          child: Row(
                            mainAxisSize: MainAxisSize.min,
                            children: [
                              _CreateBotBubble(active: _createBotOpen, onTap: _toggleCreateBot),
                              const SizedBox(width: 8),
                              _PanelBubble(onTap: _openPanel),
                            ],
                          ),
                        ),
                      ),
                    ),
                  ],

                  // 7. 居中浮起的「新建 Bot」macOS Liquid Glass 弹窗（9 种实机 GPU 星球可选，严格排除太阳，关掉后零额外开销）。
                  if (_createBotOpen)
                    Positioned.fill(
                      child: CreateBotModal(
                        clock: _clock,
                        usedCelestialCounts: usedCelestialCounts,
                        onClose: () => setState(() => _createBotOpen = false),
                        onCreated: _onLocalBotCreated,
                      ),
                    ),

                  // 8. 浮空 macOS Liquid Glass 三栏大窗（员工工作窗 / 暂留工作台窗）。
                  Positioned.fill(
                    child: FrostedSheet(
                      key: const ValueKey('agent-sheet'),
                      open: _open,
                      expanded: _sheetExpanded,
                      onClose: _close,
                      child: _view == _SheetView.agent
                          ? AgentPanel(
                              key: ValueKey('agent-panel-${_agent ?? map.chief?.agent.id}'),
                              map: panelMap,
                              agentId: _agent,
                              detailOpen: _agentDetailOpen,
                              expanded: _sheetExpanded,
                              onToggleExpand: _toggleExpand,
                              onOpenThread: _openAgentThread,
                              onNewChat: _newChatWith,
                              onSelectAgent: _selectAgent,
                              onToggleDetail: () => setState(() => _agentDetailOpen = !_agentDetailOpen),
                              onClose: _close,
                            )
                          : ShellPanel(
                              map: map,
                              section: _section,
                              agentId: _agent,
                              detailOpen: _detailOpen,
                              expanded: _sheetExpanded,
                              onToggleExpand: _toggleExpand,
                              onSection: _selectSection,
                              onSelectAgent: _selectAgent,
                              onOpenThread: _openThread,
                              onNewChat: _newChat,
                              onChatWith: _chatWith,
                              onToggleDetail: _toggleDetail,
                              onClose: _close,
                              footer: _Footer(onSettings: _settings, frosted: false),
                            ),
                    ),
                  ),
                ],
              );
            },
          ),
        ),
      ),
    );
  }
}

/// 演示用的假星球：启动时设环境变量 IP_DEMO_PLANETS=数量（最多 12），自动跳过真实员工已占用的形态，
/// 按顺序补齐八大行星 + 金箔恒星（不重复），分布在内/中/外三条固定轨道上看效果；不设就没有。
/// 可选 IP_DEMO_APPROVALS=数量（最多 5）：启动后自动把前 N 颗星球置为「完成 · 待审批」停泊到左中下底栏预览效果。
const _demoPrefix = '__demo__';
final int _demoPlanetCount = (int.tryParse(Platform.environment['IP_DEMO_PLANETS'] ?? '') ?? 0).clamp(0, 12);
final int _demoApprovalCount = (int.tryParse(Platform.environment['IP_DEMO_APPROVALS'] ?? '') ?? 0).clamp(0, 5);

List<BotBody> _demoBodiesFor(
  List<BotBody> real, {
  Set<String> reworkAgents = const {},
  Map<String, double> importanceOverrides = const {},
}) {
  if (_demoPlanetCount == 0) return const [];
  final used = {for (final b in real) b.celestial.id};
  final catalog = <({Celestial celestial, String label})>[
    for (final p in planets) (celestial: p, label: '演示·${p.name[0]}'),
    (celestial: stars.first, label: '演示·恒星'),
  ];
  final missing = [
    for (final item in catalog)
      if (!used.contains(item.celestial.id)) item,
  ];
  final pool = missing.isNotEmpty ? missing : catalog;
  // 让补齐的演示星球均衡落在内圈(2)、中圈(3)、外圈(其余)上，一眼能看全三条轨道。
  const urgencies = [0.85, 0.72, 0.48, 0.36, 0.24, 0.08, 0.04, 0.80, 0.42, 0.12, 0.68, 0.30];
  const importances = [0.34, 0.44, 0.56, 0.48, 0.38, 0.42, 0.30, 0.46, 0.50, 0.28, 0.54, 0.40];
  return [
    for (var i = 0; i < _demoPlanetCount; i++)
      () {
        final id = '$_demoPrefix$i';
        final rework = reworkAgents.contains(id);
        final autoImp = importances[i % importances.length];
        final manualImp = importanceOverrides[id];
        final effImp = (manualImp ?? autoImp).clamp(0.0, 1.0);
        return BotBody(
          agent: Agent(id: id, role: 'demo', name: pool[i % pool.length].label),
          celestial: pool[i % pool.length].celestial,
          urgency: rework ? 0.52 : urgencies[i % urgencies.length],
          importance: effImp,
          reasons: rework ? const [Reason('审批打回 · 修改中', 0.52)] : const [Reason('演示', 0)],
          importanceNotes: [
            if (manualImp != null) '已手动调节重要度：${importancePercent(effImp)}%（自动参考 ${importancePercent(autoImp)}%）',
          ],
          focusThreadId: null,
          working: rework,
        );
      }(),
  ];
}

/// 星球状态微点语义色（与左上角红绿灯待办保持同一色系：红=紧急、黄=待定、绿=正在处理）。
Color? _statusDotColor(BotBody b, {int routingWaiting = 0}) {
  if (b.urgency >= 0.8) return const Color(0xFFFF5F57);
  if (b.urgency >= 0.6 || routingWaiting > 0) return const Color(0xFFFEBC2E);
  if (b.working) return const Color(0xFF28C840);
  return null;
}

/// 按下星球后尚未超过拖拽阈值前的暂存信息（区分点击与拖拽）。
class _PendingPlanetDrag {
  const _PendingPlanetDrag({
    required this.agentId,
    required this.originTrackId,
    required this.startGlobal,
    required this.grabOffset,
    required this.startScale,
  });

  final String agentId;
  final int originTrackId;
  final Offset startGlobal;
  final Offset grabOffset;
  final double startScale;
}

/// 拖拽换轨进行中的实时状态（用于跟手绘制、轨道磁吸高亮与环边缘容量预览）。
class _OrbitDragState {
  const _OrbitDragState({
    required this.agentId,
    required this.originTrackId,
    required this.grabOffset,
    required this.fallbackScale,
    required this.visualCenter,
    required this.visualScale,
    required this.snap,
    required this.preview,
  });

  final String agentId;
  final int originTrackId;
  final Offset grabOffset;
  final double fallbackScale;
  final Offset visualCenter;
  final double visualScale;
  final OrbitSnapTarget snap;

  /// 当磁吸到新环时预演的落位结果（用来展示 `2/3 → 3/3` / `3/3 · 替换` 及标出将被挤出的星球）。
  final OrbitDropOutcome? preview;
}

/// 单颗员工行星（统一单树 + 窗顶停泊 + 1C 左中下待审批缩落泊位 + 拖拽磁吸 + 换轨/挤出平滑过渡：永不重建着色器）。
class _OrbitingPlanetItem extends StatefulWidget {
  const _OrbitingPlanetItem({
    super.key,
    required this.body,
    required this.track,
    required this.slotPhase,
    required this.roles,
    required this.clock,
    required this.sheetOpen,
    required this.selected,
    required this.hovered,
    required this.frozen,
    required this.orbit,
    required this.stripHeight,
    required this.topX,
    required this.topY,
    required this.topPlanetRadius,
    required this.topLabelY,
    this.dragCenter,
    this.dragScale,
    this.approvalDockCenter,
    this.approvalDockScale,
    this.evictTargetTrackId,
    this.zoomPillText,
    required this.onHoverChanged,
    required this.onTap,
    this.onScrollZoom,
    this.onDragStart,
    this.onDragUpdate,
    this.onDragEnd,
    this.onDragCancel,
    this.onSecondaryTap,
  });

  /// 星球离屏肌理的固定烘焙球径：滚轮缩放时只平滑改 Transform.scale，避免每帧重烘离屏纹理造成卡顿。
  static const double bakeRadius = 32.0;

  final BotBody body;

  /// 在轨时为所属轨道与槽位初相；离轨进入左中下待审批停泊区时为空。
  final OrbitTrack? track;
  final double? slotPhase;
  final List<Role> roles;
  final CelestialClock clock;
  final bool sheetOpen;
  final bool selected;
  final bool hovered;
  final bool frozen;
  final OrbitGeometry orbit;
  final double stripHeight;
  final double topX;
  final double topY;

  /// 开窗后顶栏中除太阳外全部行星统一的视觉球体基准半径（与轨道上的 importance 大小解耦）。
  final double topPlanetRadius;

  /// 开窗后顶栏全部天体（含太阳与行星）共用的名字水平基线 Y 坐标。
  final double topLabelY;

  /// 正在被用户拖拽时的屏幕中心与缩放；为 null 表示不在拖拽中。
  final Offset? dragCenter;
  final double? dragScale;

  /// 进入「完成 · 待审批」后在页面左中下停泊区（1C）的目标中心与缩小比例；为 null 表示在轨公转。
  final Offset? approvalDockCenter;
  final double? approvalDockScale;

  /// 当别的星球拖入当前满环且预演会挤出本星球时，记录本星球将被挤往的目标环编号（用于柔光提示）。
  final int? evictTargetTrackId;

  /// 方案 A 滚轮调重要度时短暂显示的瞬态读数（如 `重要度 78% · 顺位 #1`），停滚后自动清空为 null。
  final String? zoomPillText;

  final ValueChanged<bool> onHoverChanged;
  final VoidCallback onTap;
  final ValueChanged<double>? onScrollZoom;
  final GestureDragStartCallback? onDragStart;
  final GestureDragUpdateCallback? onDragUpdate;
  final GestureDragEndCallback? onDragEnd;
  final VoidCallback? onDragCancel;
  final VoidCallback? onSecondaryTap;

  bool get isApprovalDocked => approvalDockCenter != null;

  double get _seed => ((body.agent.id.hashCode & 0x7fffffff) % 997) / 997.0;

  /// 这颗星球在共享时钟 [t]（秒）时的目标轨道位置与远近；离轨待审时为空。
  OrbitPose? poseAt(double t) {
    final tr = track;
    final ph = slotPhase;
    if (tr == null || ph == null) return null;
    return OrbitPose.onTrack(orbit: orbit, track: tr, phase: ph, t: t - 40.0);
  }

  @override
  State<_OrbitingPlanetItem> createState() => _OrbitingPlanetItemState();
}

class _OrbitingPlanetItemState extends State<_OrbitingPlanetItem> with TickerProviderStateMixin {
  /// 当前帧的纵深大气透视（0 近 .. 1 最远），普通 double 不触发二次重绘帧，由 _BodyPainter 在同一帧的 paint 阶段读取。
  double _hazeValue = 0.0;

  double _readHaze() => _hazeValue;

  late final AnimationController _dock = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 300),
    value: widget.sheetOpen ? 1 : 0,
  );

  /// 换轨、被挤出、松手吸回、同轨匀位或审批后从 1C 泊位飞回轨道时的平滑过渡（0 起点 → 1 到达目标轨道槽位）。
  late final AnimationController _lane = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 650),
    value: 1,
  );

  /// 进入左中下「完成 · 待审批」泊位（或泊位内横移补位）时的平滑缩小滑落动画（0 起点 → 1 停稳在 1C 泊位）。
  late final AnimationController _approvalDock = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 560),
    value: widget.approvalDockCenter != null ? 1 : 0,
  );

  /// 滚轮放大/缩小或三栏左栏调节重要度时的丝滑尺寸补间（140ms，不重烘着色器纹理）。
  late final AnimationController _sizeAnim = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 140),
    value: 1,
  );

  double get _targetSizeFactor => planetOrbitRadius(widget.body.importance) / _OrbitingPlanetItem.bakeRadius;
  late double _fromSizeFactor = _targetSizeFactor;
  late double _toSizeFactor = _targetSizeFactor;

  double get _currentSizeFactor =>
      ui.lerpDouble(_fromSizeFactor, _toSizeFactor, Curves.easeOutCubic.transform(_sizeAnim.value))!;

  late final Listenable _motion = Listenable.merge([_dock, _lane, _approvalDock, _sizeAnim]);
  late Listenable _orbitAnim = Listenable.merge([widget.clock, _dock, _lane, _approvalDock, _sizeAnim]);

  late double _fromFrac = (widget.track ?? OrbitTrack.all.last).frac;
  late double _fromRoll = (widget.track ?? OrbitTrack.all.last).rollRad;
  late double _fromFlatten = (widget.track ?? OrbitTrack.all.last).flatten;
  double _phaseDelta = 0.0;

  /// 松手落位或审批结束飞回轨道时的起点坐标与缩放。
  bool _glidingFromDrop = false;
  Offset _dropStartPos = Offset.zero;
  double _dropStartScale = 1.0;

  /// 飞入 1C 待审批泊位时的起点快照（保证从实时轨道坐标丝滑缩小落入泊位，不跳变）。
  late Offset _approvalFromPos = widget.approvalDockCenter ?? widget.orbit.center;
  late double _approvalFromScale = widget.approvalDockScale ?? 1.0;
  double _approvalFromHaze = 0.0;

  /// 记录上一帧实际绘制的坐标、缩放与雾度，用于状态切换时无缝接力起点。
  late Offset _lastPos = widget.approvalDockCenter ?? widget.orbit.center;
  late double _lastScale = widget.approvalDockScale ?? 1.0;
  double _lastHaze = 0.0;

  double _curFrac(double k) => ui.lerpDouble(_fromFrac, (widget.track ?? OrbitTrack.all.last).frac, k)!;
  double _curRoll(double k) => ui.lerpDouble(_fromRoll, (widget.track ?? OrbitTrack.all.last).rollRad, k)!;
  double _curFlatten(double k) => ui.lerpDouble(_fromFlatten, (widget.track ?? OrbitTrack.all.last).flatten, k)!;

  double _curMean(double t, double k) {
    final tr = widget.track ?? OrbitTrack.all.last;
    final ph = widget.slotPhase ?? 0.0;
    final targetN = OrbitPose.meanMotion(widget.orbit, tr.frac);
    final targetM = ph + targetN * (t - 40.0);
    return targetM + _phaseDelta * (1.0 - k);
  }

  @override
  void didUpdateWidget(_OrbitingPlanetItem old) {
    super.didUpdateWidget(old);
    if (old.clock != widget.clock) {
      _orbitAnim = Listenable.merge([widget.clock, _dock, _lane, _approvalDock, _sizeAnim]);
    }
    if (old.sheetOpen != widget.sheetOpen) {
      widget.sheetOpen ? _dock.animateTo(1, curve: Motion.curve) : _dock.animateTo(0, curve: Motion.curve);
    }
    final nextSize = _targetSizeFactor;
    if ((nextSize - _toSizeFactor).abs() > 1e-4) {
      _fromSizeFactor = _currentSizeFactor;
      _toSizeFactor = nextSize;
      _sizeAnim
        ..value = 0
        ..animateTo(1, duration: const Duration(milliseconds: 140), curve: Curves.easeOutCubic);
    }

    // 0a. 进入左中下「完成 · 待审批」停泊区（或同排槽位横移补位）：从当前实时画面坐标平滑缩小滑落到 1C 泊位；
    //     若只是用户拖拽窗口高度导致底栏 Y 坐标整体跟随，则直接紧跟胶囊底边不重播补间，避免缩小星球短暂甩出气泡外。
    if (widget.approvalDockCenter != null) {
      final oldCenter = old.approvalDockCenter;
      if (oldCenter == null) {
        _approvalFromPos = _lastPos;
        _approvalFromScale = _lastScale;
        _approvalFromHaze = _lastHaze;
        _glidingFromDrop = false;
        _lane.value = 1;
        _approvalDock
          ..value = 0
          ..animateTo(1, duration: const Duration(milliseconds: 560), curve: Motion.curve);
      } else if ((oldCenter.dx - widget.approvalDockCenter!.dx).abs() > 0.5) {
        _approvalFromPos = _lastPos;
        _approvalFromScale = _lastScale;
        _approvalFromHaze = _lastHaze;
        _glidingFromDrop = false;
        _lane.value = 1;
        _approvalDock
          ..value = 0
          ..animateTo(1, duration: const Duration(milliseconds: 280), curve: Motion.curve);
      } else if ((oldCenter.dy - widget.approvalDockCenter!.dy).abs() > 0.5) {
        final dy = widget.approvalDockCenter!.dy - oldCenter.dy;
        _approvalFromPos = Offset(_approvalFromPos.dx, _approvalFromPos.dy + dy);
      }
      return;
    }

    // 0b. 审批完成（通过或打回）：从 1C 泊位平滑放大飞回最新分配的轨道槽位。
    if (old.approvalDockCenter != null && widget.approvalDockCenter == null) {
      final tr = widget.track ?? OrbitTrack.all.last;
      _approvalDock.value = 0;
      _glidingFromDrop = true;
      _dropStartPos = _lastPos;
      _dropStartScale = _lastScale;
      _fromFrac = tr.frac;
      _fromRoll = tr.rollRad;
      _fromFlatten = tr.flatten;
      _phaseDelta = 0.0;
      _lane
        ..value = 0
        ..animateTo(1, duration: const Duration(milliseconds: 620), curve: Motion.curve);
      return;
    }

    // 1. 刚结束拖拽松手：从松手时的视觉位置丝滑过渡到最终落位轨道（无论成功换轨还是取消弹回）。
    if (old.dragCenter != null && widget.dragCenter == null) {
      final tr = widget.track ?? OrbitTrack.all.last;
      _glidingFromDrop = true;
      _dropStartPos = old.dragCenter!;
      _dropStartScale = old.dragScale ?? 1.0;
      _fromFrac = tr.frac;
      _fromRoll = tr.rollRad;
      _fromFlatten = tr.flatten;
      _phaseDelta = 0.0;
      _lane
        ..value = 0
        ..animateTo(1, duration: const Duration(milliseconds: 520), curve: Motion.curve);
      return;
    }
    // 2. 正在拖动中不启动换轨补间。
    if (widget.dragCenter != null) {
      _glidingFromDrop = false;
      _lane.value = 1;
      return;
    }
    // 3. 轨道变更（比如被满环挤出到外侧轨道）或同轨匀位：沿最短弧平滑过渡，不瞬移。
    final oldTr = old.track;
    final newTr = widget.track;
    final oldPh = old.slotPhase;
    final newPh = widget.slotPhase;
    if (oldTr != null &&
        newTr != null &&
        oldPh != null &&
        newPh != null &&
        (oldTr.id != newTr.id || (oldPh - newPh).abs() > 1e-4)) {
      final k = _lane.value;
      final t = widget.clock.value;
      final curFrac = _glidingFromDrop ? oldTr.frac : ui.lerpDouble(_fromFrac, oldTr.frac, k)!;
      final curRoll = _glidingFromDrop ? oldTr.rollRad : ui.lerpDouble(_fromRoll, oldTr.rollRad, k)!;
      final curFlatten = _glidingFromDrop ? oldTr.flatten : ui.lerpDouble(_fromFlatten, oldTr.flatten, k)!;
      final oldN = OrbitPose.meanMotion(old.orbit, oldTr.frac);
      final curM = oldPh + oldN * (t - 40.0) + (_glidingFromDrop ? 0.0 : _phaseDelta * (1.0 - k));
      final newN = OrbitPose.meanMotion(widget.orbit, newTr.frac);
      final newM = newPh + newN * (t - 40.0);
      _glidingFromDrop = false;
      _fromFrac = curFrac;
      _fromRoll = curRoll;
      _fromFlatten = curFlatten;
      _phaseDelta = math.atan2(math.sin(curM - newM), math.cos(curM - newM));
      _lane
        ..value = 0
        ..animateTo(1, duration: const Duration(milliseconds: 680), curve: Motion.curve);
    }
  }

  @override
  void dispose() {
    _dock.dispose();
    _lane.dispose();
    _approvalDock.dispose();
    _sizeAnim.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final w = widget;
    final body = w.body;
    final style = CelestialStyle.fromCelestial(body.celestial);
    // 以固定高质量球径 bakeRadius 烘焙离屏表面，再按重要度曲线 planetOrbitRadius(importance) 平滑缩放；
    // 滚轮调大小时零同步 GPU 重烘，同时保持窗顶切换栏与左中下泊位的统一视觉尺寸。
    const radius = _OrbitingPlanetItem.bakeRadius;
    final extent = CelestialBody.extentFor(radius, style);
    final seed = w._seed;
    final role = body.agent.role == 'demo' ? '演示星球' : roleNameFor(w.roles, body.agent.role);
    final statusWord = w.isApprovalDocked ? '已完成 · 待审批' : (body.working ? '正在处理' : body.reasons.first.text);
    final dotColor = w.isApprovalDocked ? const Color(0xFFFEBC2E) : _statusDotColor(body);
    final isDragging = w.dragCenter != null;
    final evictTrack = w.evictTargetTrackId != null ? OrbitTrack.byId(w.evictTargetTrackId!) : null;

    // 窗顶停泊缩放：按统一视觉球体基准半径归一化（带环的土星/天王星与星芒恒星按光学体量微收），
    // 不再按外框 extent 挤压土星，确保除太阳外全部行星视觉上大小一致，且绝不撞上浮空窗顶边。
    final topOpticalFactor = switch (style) {
      CelestialStyle.ringed => 0.84,
      CelestialStyle.star => 0.88,
      CelestialStyle.ice => 0.94,
      _ => 1.0,
    };
    final targetTopR = w.topPlanetRadius * topOpticalFactor * (w.selected ? 1.08 : 1.0);
    final maxSafeScale = ((w.stripHeight - 10) / extent).clamp(0.20, 1.35);
    final openScale = (targetTopR / radius).clamp(0.24, maxSafeScale);

    final orb = CelestialBody(
      key: ValueKey('planet-${body.agent.id}'),
      radius: radius,
      style: style,
      seed: seed,
      selected: w.selected || isDragging || w.zoomPillText != null,
      frozen: w.frozen,
      clock: w.clock,
      onHoverChanged: w.onHoverChanged,
      onScrollZoom: w.onScrollZoom,
      onDragStart: w.onDragStart,
      onDragUpdate: w.onDragUpdate,
      onDragEnd: w.onDragEnd,
      onDragCancel: w.onDragCancel,
      onSecondaryTap: w.onSecondaryTap,
      dragging: isDragging,
      haze: _readHaze,
      repaint: _motion,
      semanticsLabel: w.isApprovalDocked
          ? '${body.celestial.name}：${body.agent.name}（已完成，待审批）'
          : '${body.celestial.name}：${body.agent.name}',
      onTap: w.onTap,
    );

    return AnimatedBuilder(
      animation: _orbitAnim,
      child: orb,
      builder: (context, child) {
        final d = _dock.value;
        final k = _lane.value;
        final a = _approvalDock.value;
        final t = w.clock.value;
        final sizeFactor = _currentSizeFactor;

        final Offset orbitPos;
        final double orbitScale;
        final double orbitHaze;
        double orbitLabelFade = 1.0;

        if (w.approvalDockCenter != null) {
          orbitPos = Offset.lerp(_approvalFromPos, w.approvalDockCenter!, a)!;
          orbitScale = ui.lerpDouble(_approvalFromScale, w.approvalDockScale ?? 0.5, a)!;
          orbitHaze = _approvalFromHaze * (1.0 - a);
          // 落入 1C 玻璃泊位时淡出悬浮白字标签，改由玻璃槽位内的高对比深色标签接管显示“完成 · 待审批”。
          orbitLabelFade = (1.0 - a * 1.8).clamp(0.0, 1.0);
        } else if (isDragging) {
          orbitPos = w.dragCenter!;
          orbitScale = (w.dragScale ?? 1.08) * sizeFactor;
          orbitHaze = 0.0;
        } else if (k < 1.0 && _glidingFromDrop) {
          final target = w.poseAt(t) ?? OrbitPose(w.orbit.center, 0, 1, 0);
          orbitPos = Offset.lerp(_dropStartPos, target.position, k)!;
          orbitScale = ui.lerpDouble(_dropStartScale, target.scale * sizeFactor, k)!;
          orbitHaze = target.haze * k;
        } else {
          final pose = k >= 1.0
              ? (w.poseAt(t) ?? OrbitPose(w.orbit.center, 0, 1, 0))
              : OrbitPose.fromParams(
                  orbit: w.orbit,
                  frac: _curFrac(k),
                  rollRad: _curRoll(k),
                  flatten: _curFlatten(k),
                  meanAnomaly: _curMean(t, k),
                );
          orbitPos = pose.position;
          orbitScale = pose.scale * sizeFactor;
          orbitHaze = pose.haze;
        }

        _lastPos = orbitPos;
        _lastScale = orbitScale;
        _lastHaze = orbitHaze;

        final cx = ui.lerpDouble(orbitPos.dx, w.topX, d)!;
        final cy = ui.lerpDouble(orbitPos.dy, w.topY, d)!;
        final scale = ui.lerpDouble(orbitScale, openScale, d)!;
        _hazeValue = orbitHaze * (1.0 - d);
        final dockBlendLabel = ui.lerpDouble(orbitLabelFade, 1.0, d)!;
        final labelOpacity = ((1.0 - 0.45 * _hazeValue) * dockBlendLabel).clamp(0.0, 1.0);
        final showLabel = (!w.sheetOpen || w.stripHeight >= 120) && labelOpacity > 0.01;
        final orbitLabelY =
            orbitPos.dy +
            radius * orbitScale +
            switch (style) {
              CelestialStyle.star => 18,
              CelestialStyle.ringed => 12,
              _ => 9,
            };
        // 窗顶统一名字水平基线：太阳与所有行星的名字严格对齐在同一条水平线上，不再参差不齐。
        final labelY = ui.lerpDouble(orbitLabelY, w.topLabelY, d)!;
        final labelSubtitle = w.sheetOpen
            ? null
            : evictTrack != null
            ? '将移至${evictTrack.label}'
            : '$role · $statusWord';
        Widget planetChild = Transform.scale(scale: scale, child: child!);
        if (evictTrack != null) {
          planetChild = Opacity(opacity: 0.62, child: planetChild);
        }
        return Stack(
          children: [
            Positioned(left: cx - extent / 2, top: cy - extent / 2, width: extent, height: extent, child: planetChild),
            if (showLabel)
              Positioned(
                left: cx - 88,
                top: labelY,
                width: 176,
                child: IgnorePointer(
                  child: _PlanetLabel(
                    agentId: body.agent.id,
                    name: body.agent.name,
                    subtitle: labelSubtitle,
                    zoomPillText: w.zoomPillText,
                    emphasized: w.selected || w.hovered || isDragging || evictTrack != null || w.zoomPillText != null,
                    compact: w.sheetOpen,
                    dotColor: evictTrack != null ? const Color(0xFFFFC75F) : dotColor,
                    opacity: labelOpacity,
                  ),
                ),
              ),
          ],
        );
      },
    );
  }
}

/// 太阳和员工行星放在同一层，按远近排先后：远的先画（被太阳和近的挡住），近的后画（挡住太阳）；
/// 停泊在左中下 1C 待审批区的星球画在最前层（仅低于正在拖拽中的星球），确保落在玻璃底托上方清晰可点。
/// 仅在前后遮挡顺序真正发生变化时才触发 Stack 重建，平时公转零整层重建开销。
class _DepthSortedOrbit extends StatefulWidget {
  const _DepthSortedOrbit({required this.clock, required this.planets, required this.sun, this.draggingAgentId});

  final CelestialClock clock;
  final List<_OrbitingPlanetItem> planets;
  final List<Widget> sun;
  final String? draggingAgentId;

  @override
  State<_DepthSortedOrbit> createState() => _DepthSortedOrbitState();
}

class _DepthSortedOrbitState extends State<_DepthSortedOrbit> {
  List<String> _backIds = const [];
  List<String> _frontIds = const [];
  List<String> _dockedIds = const [];

  @override
  void initState() {
    super.initState();
    _syncOrder();
    widget.clock.addListener(_onTick);
  }

  @override
  void didUpdateWidget(_DepthSortedOrbit old) {
    super.didUpdateWidget(old);
    if (old.clock != widget.clock) {
      old.clock.removeListener(_onTick);
      widget.clock.addListener(_onTick);
    }
    _syncOrder();
  }

  @override
  void dispose() {
    widget.clock.removeListener(_onTick);
    super.dispose();
  }

  ({List<String> back, List<String> front, List<String> docked}) _computeOrder() {
    final t = widget.clock.value;
    final withDepth = <(String, double)>[];
    final docked = <String>[];
    for (final p in widget.planets) {
      final id = p.body.agent.id;
      if (id == widget.draggingAgentId) continue;
      if (p.isApprovalDocked) {
        docked.add(id);
        continue;
      }
      final depth = p.sheetOpen ? 0.0 : (p.poseAt(t)?.depth ?? 0.0);
      withDepth.add((id, depth));
    }
    withDepth.sort((x, y) => x.$2.compareTo(y.$2));
    final back = <String>[];
    final front = <String>[];
    for (final (id, depth) in withDepth) {
      if (depth < 0) {
        back.add(id);
      } else {
        front.add(id);
      }
    }
    return (back: back, front: front, docked: docked);
  }

  void _syncOrder() {
    final next = _computeOrder();
    _backIds = next.back;
    _frontIds = next.front;
    _dockedIds = next.docked;
  }

  void _onTick() {
    final next = _computeOrder();
    if (listEquals(_backIds, next.back) && listEquals(_frontIds, next.front) && listEquals(_dockedIds, next.docked)) {
      return;
    }
    setState(() {
      _backIds = next.back;
      _frontIds = next.front;
      _dockedIds = next.docked;
    });
  }

  @override
  Widget build(BuildContext context) {
    final byId = {for (final p in widget.planets) p.body.agent.id: p};
    final dragged = widget.draggingAgentId != null ? byId[widget.draggingAgentId!] : null;
    return Stack(
      children: [
        for (final id in _backIds) ?byId[id],
        ...widget.sun,
        for (final id in _frontIds) ?byId[id],
        for (final id in _dockedIds) ?byId[id],
        ?dragged,
      ],
    );
  }
}

/// 1C（位置 1）：页面左中下底栏的「完成 · 待审批」单一孪生玻璃胶囊。
/// 直接复用 [_SkyGlassCapsule]，与左下角“已连接”气泡严格等宽（222px）、等高（38px）、同圆角（20）与同款冷白玻璃；
/// 胶囊内只保留固定短文案“完成 · 待审批”和琥珀计数（永不因长员工名截断或撑宽），
/// 悬停或点击胶囊后在正上方展开 [_CompletionApprovalPopover] 查看完整长名与极轻快捷审批。
class _CompletionApprovalDock extends StatelessWidget {
  const _CompletionApprovalDock({
    required this.totalCount,
    required this.width,
    required this.height,
    required this.textStart,
    required this.popoverOpen,
    required this.onTap,
  });

  final int totalCount;
  final double width;
  final double height;
  final double textStart;
  final bool popoverOpen;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    const ink = Color(0xFF14171F);
    return _SkyGlassCapsule(
      key: const ValueKey('approval-dock-capsule'),
      onTap: onTap,
      child: SizedBox(
        width: width,
        height: height,
        child: Padding(
          padding: EdgeInsets.fromLTRB(textStart, 0, 11, 0),
          child: Row(
            children: [
              const Expanded(
                child: Text(
                  '完成 · 待审批',
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: TextStyle(color: ink, fontSize: 12.5, fontWeight: FontWeight.w600),
                ),
              ),
              const SizedBox(width: 6),
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1.5),
                decoration: BoxDecoration(
                  color: popoverOpen ? const Color(0xFFD97706) : const Color(0xFFE59500),
                  borderRadius: BorderRadius.circular(999),
                ),
                child: Text(
                  '$totalCount',
                  style: const TextStyle(
                    fontSize: 10.5,
                    fontWeight: FontWeight.w600,
                    color: Colors.white,
                    fontFeatures: [ui.FontFeature.tabularFigures()],
                  ),
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

/// 1C 悬停/点击向上展开的「完成 · 待审批」Liquid Glass 轻浮层（宽 308px，专治长员工名）：
/// 与左上角待办沿用同一套冷白玻璃与 0.5px 极淡分隔线（零框中套框）；
/// 每行整行点击按 3B 直接打开三栏工作窗，右侧提供极轻快捷按钮（✓ 审批通过 / ↩ 打回修改），
/// 悬停任意一行时底栏对应的缩小真 GPU 星球会同步提亮呼应。
class _CompletionApprovalPopover extends StatelessWidget {
  const _CompletionApprovalPopover({
    required this.entries,
    required this.roles,
    required this.maxHeight,
    required this.hoveredAgentId,
    required this.onRowHoverChanged,
    required this.onOpenAgent,
  });

  final List<({BotBody body, CompletionApproval approval})> entries;
  final List<Role> roles;
  final double maxHeight;
  final String? hoveredAgentId;
  final void Function(String agentId, bool hovered) onRowHoverChanged;
  final ValueChanged<String> onOpenAgent;

  @override
  Widget build(BuildContext context) {
    final radius = BorderRadius.circular(20);
    final shape = RoundedSuperellipseBorder(
      borderRadius: radius,
      side: const BorderSide(color: Color(0x99FFFFFF), width: 0.7),
    );
    return TweenAnimationBuilder<double>(
      tween: Tween(begin: 0.0, end: 1.0),
      duration: Motion.msg,
      curve: Motion.curve,
      builder: (context, t, child) => Opacity(
        opacity: t,
        child: Transform.translate(offset: Offset(0, (1.0 - t) * 6.0), child: child),
      ),
      child: DecoratedBox(
        decoration: ShapeDecoration(
          shape: RoundedSuperellipseBorder(borderRadius: radius),
          shadows: const [
            BoxShadow(color: Color(0x400E183A), offset: Offset(0, 14), blurRadius: 34, spreadRadius: -10),
            BoxShadow(color: Color(0x140E183A), offset: Offset(0, 2), blurRadius: 8),
          ],
        ),
        child: ClipRSuperellipse(
          borderRadius: radius,
          child: BackdropFilter(
            filter: ui.ImageFilter.compose(
              outer: const ui.ColorFilter.matrix(liquidSaturateMatrix),
              inner: ui.ImageFilter.blur(sigmaX: 28, sigmaY: 28),
            ),
            child: Container(
              width: 308,
              constraints: BoxConstraints(maxHeight: maxHeight),
              padding: const EdgeInsets.fromLTRB(10, 9, 10, 9),
              decoration: ShapeDecoration(
                gradient: const LinearGradient(
                  begin: Alignment.topLeft,
                  end: Alignment.bottomRight,
                  colors: [Color(0xD4FFFFFF), Color(0xB8F5F8FE), Color(0xC4EFF4FD)],
                  stops: [0.0, 0.56, 1.0],
                ),
                shape: shape,
              ),
              child: Material(
                type: MaterialType.transparency,
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.stretch,
                  children: [
                    Padding(
                      padding: const EdgeInsets.fromLTRB(6, 0, 6, 6),
                      child: Row(
                        children: [
                          Container(
                            width: 7,
                            height: 7,
                            decoration: BoxDecoration(
                              shape: BoxShape.circle,
                              color: const Color(0xFFFEBC2E),
                              boxShadow: [
                                BoxShadow(color: const Color(0xFFFEBC2E).withValues(alpha: 0.45), blurRadius: 6),
                              ],
                            ),
                          ),
                          const SizedBox(width: 7),
                          const Expanded(
                            child: Text(
                              '完成 · 待审批',
                              style: TextStyle(
                                fontSize: 12.5,
                                fontWeight: FontWeight.w600,
                                color: Color(0xFF14171F),
                                letterSpacing: -0.1,
                              ),
                            ),
                          ),
                          Text(
                            '${entries.length} 项',
                            style: const TextStyle(
                              fontSize: 11,
                              fontWeight: FontWeight.w500,
                              color: Color(0x7314171F),
                              fontFeatures: [ui.FontFeature.tabularFigures()],
                            ),
                          ),
                        ],
                      ),
                    ),
                    const Divider(height: 1, thickness: 0.5, color: Color(0x1814171F)),
                    Flexible(
                      child: ListView.separated(
                        shrinkWrap: true,
                        padding: const EdgeInsets.only(top: 4),
                        itemCount: entries.length,
                        separatorBuilder: (_, _) => const Padding(
                          padding: EdgeInsets.only(left: 20, right: 6),
                          child: Divider(height: 1, thickness: 0.5, color: Color(0x1414171F)),
                        ),
                        itemBuilder: (context, i) {
                          final e = entries[i];
                          final id = e.body.agent.id;
                          final dotColor = e.body.agent.role == 'demo'
                              ? CelestialStyle.fromCelestial(e.body.celestial).fallbackColor
                              : roleColorFor(roles, e.body.agent.role);
                          return _CompletionPopoverRow(
                            key: ValueKey('popover-completion-row-$id'),
                            item: e.approval,
                            dotColor: dotColor,
                            hovered: hoveredAgentId == id,
                            onHoverChanged: (h) => onRowHoverChanged(id, h),
                            onOpenAgent: () => onOpenAgent(id),
                          );
                        },
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _CompletionPopoverRow extends StatefulWidget {
  const _CompletionPopoverRow({
    super.key,
    required this.item,
    required this.dotColor,
    required this.hovered,
    required this.onHoverChanged,
    required this.onOpenAgent,
  });

  final CompletionApproval item;
  final Color dotColor;
  final bool hovered;
  final ValueChanged<bool> onHoverChanged;
  final VoidCallback onOpenAgent;

  @override
  State<_CompletionPopoverRow> createState() => _CompletionPopoverRowState();
}

class _CompletionPopoverRowState extends State<_CompletionPopoverRow> {
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
    return MouseRegion(
      cursor: SystemMouseCursors.click,
      onEnter: (_) => widget.onHoverChanged(true),
      onExit: (_) => widget.onHoverChanged(false),
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: widget.onOpenAgent,
        child: AnimatedContainer(
          duration: Motion.fast,
          curve: Motion.curve,
          padding: const EdgeInsets.fromLTRB(6, 7, 4, 7),
          decoration: ShapeDecoration(
            color: widget.hovered ? const Color(0x6BFFFFFF) : Colors.transparent,
            shape: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(11)),
          ),
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.center,
            children: [
              Container(
                width: 7,
                height: 7,
                decoration: BoxDecoration(
                  color: widget.dotColor,
                  shape: BoxShape.circle,
                  boxShadow: [BoxShadow(color: widget.dotColor.withValues(alpha: 0.35), blurRadius: 4)],
                ),
              ),
              const SizedBox(width: 9),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Row(
                      children: [
                        Expanded(
                          child: Text(
                            item.agentName,
                            maxLines: 1,
                            overflow: TextOverflow.ellipsis,
                            style: const TextStyle(
                              fontSize: 13,
                              fontWeight: FontWeight.w600,
                              color: Color(0xFF14171F),
                              height: 1.2,
                            ),
                          ),
                        ),
                        const SizedBox(width: 6),
                        Text(
                          formatTime(item.createdAtMs),
                          style: const TextStyle(
                            fontSize: 10.5,
                            color: Color(0x6E14171F),
                            fontFeatures: [ui.FontFeature.tabularFigures()],
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 2),
                    Text(
                      item.taskTitle,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: const TextStyle(fontSize: 11.5, color: Color(0x8F14171F), height: 1.25),
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 8),
              _QuickDecisionButton(
                key: ValueKey('popover-completion-approve-${item.agentId}'),
                tooltip: '审批通过',
                icon: Icons.check_rounded,
                color: const Color(0xFF15803D),
                bg: const Color(0x1C16A34A),
                busy: _busy == CompletionDecision.approve,
                onTap: canAct ? () => _decide(CompletionDecision.approve) : null,
              ),
              const SizedBox(width: 5),
              _QuickDecisionButton(
                key: ValueKey('popover-completion-reject-${item.agentId}'),
                tooltip: '打回修改',
                icon: Icons.undo_rounded,
                color: const Color(0xFF9A5800),
                bg: const Color(0x1414171F),
                busy: _busy == CompletionDecision.reject,
                onTap: canAct ? () => _decide(CompletionDecision.reject) : null,
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _QuickDecisionButton extends StatelessWidget {
  const _QuickDecisionButton({
    super.key,
    required this.tooltip,
    required this.icon,
    required this.color,
    required this.bg,
    required this.busy,
    required this.onTap,
  });

  final String tooltip;
  final IconData icon;
  final Color color;
  final Color bg;
  final bool busy;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return Tooltip(
      message: tooltip,
      child: Material(
        color: onTap == null ? bg.withValues(alpha: 0.45) : bg,
        shape: const CircleBorder(),
        child: InkWell(
          customBorder: const CircleBorder(),
          onTap: onTap,
          child: SizedBox.square(
            dimension: 26,
            child: Center(
              child: busy
                  ? SizedBox.square(dimension: 12, child: CircularProgressIndicator(strokeWidth: 1.8, color: color))
                  : Icon(icon, size: 14.5, color: color),
            ),
          ),
        ),
      ),
    );
  }
}

/// 星球下方的极轻文字标签：平时柔和象牙白，悬停/选中时清晰亮起并展现角色与实时状态。
/// 远近淡化直接乘进文字与光点透明度，不套外层 Opacity，避免每帧分配离屏 saveLayer。
class _PlanetLabel extends StatelessWidget {
  const _PlanetLabel({
    this.agentId,
    required this.name,
    this.subtitle,
    this.zoomPillText,
    required this.emphasized,
    required this.compact,
    this.dotColor,
    this.opacity = 1.0,
  });

  final String? agentId;
  final String name;
  final String? subtitle;
  final String? zoomPillText;
  final bool emphasized;
  final bool compact;
  final Color? dotColor;
  final double opacity;

  @override
  Widget build(BuildContext context) {
    final alpha = opacity.clamp(0.0, 1.0);
    final shadow = [
      Shadow(
        color: const Color(0xB80A1432).withValues(alpha: (0xB8 / 255.0) * alpha),
        offset: const Offset(0, 1),
        blurRadius: 8,
      ),
    ];
    final nameBase = emphasized ? const Color(0xFFFFFCF2) : const Color(0xD9FFF9EC);
    final subBase = const Color(0xD6E8F0FF);
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Row(
          mainAxisSize: MainAxisSize.min,
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            if (dotColor != null) ...[
              Container(
                width: 6.5,
                height: 6.5,
                decoration: BoxDecoration(
                  color: dotColor!.withValues(alpha: dotColor!.a * alpha),
                  shape: BoxShape.circle,
                  boxShadow: [BoxShadow(color: dotColor!.withValues(alpha: dotColor!.a * 0.72 * alpha), blurRadius: 6)],
                ),
              ),
              const SizedBox(width: 5),
            ],
            Flexible(
              child: Text(
                name,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                style: TextStyle(
                  fontSize: compact ? 11.5 : 13,
                  fontWeight: emphasized ? FontWeight.w600 : FontWeight.w500,
                  color: nameBase.withValues(alpha: nameBase.a * alpha),
                  shadows: shadow,
                ),
              ),
            ),
          ],
        ),
        if (!compact && zoomPillText != null)
          Padding(
            padding: const EdgeInsets.only(top: 4),
            child: Container(
              key: agentId == null ? null : ValueKey('planet-zoom-pill-$agentId'),
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2.5),
              decoration: BoxDecoration(
                color: const Color(0xBA0C1630).withValues(alpha: (0xBA / 255.0) * alpha),
                borderRadius: BorderRadius.circular(999),
                border: Border.all(
                  color: const Color(0x52FFE8B8).withValues(alpha: (0x52 / 255.0) * alpha),
                  width: 0.7,
                ),
                boxShadow: [
                  BoxShadow(
                    color: const Color(0x45050D22).withValues(alpha: (0x45 / 255.0) * alpha),
                    offset: const Offset(0, 2),
                    blurRadius: 8,
                  ),
                ],
              ),
              child: Text(
                zoomPillText!,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontSize: 10.5,
                  fontWeight: FontWeight.w600,
                  color: const Color(0xFFFDF6E4).withValues(alpha: alpha),
                  fontFeatures: const [ui.FontFeature.tabularFigures()],
                ),
              ),
            ),
          )
        else if (!compact && subtitle != null)
          AnimatedOpacity(
            duration: Motion.fast,
            opacity: emphasized ? 0.92 : 0.0,
            child: Padding(
              padding: const EdgeInsets.only(top: 2),
              child: Text(
                subtitle!,
                maxLines: 1,
                overflow: TextOverflow.ellipsis,
                textAlign: TextAlign.center,
                style: TextStyle(
                  fontSize: 11,
                  color: subBase.withValues(alpha: subBase.a * alpha),
                  shadows: shadow,
                ),
              ),
            ),
          ),
      ],
    );
  }
}

/// 三条固定通用轨道金线的离屏紧致缓存容器：
/// Impeller 不会自动把 RepaintBoundary 里的约 950 次矢量线段/圆点光栅化缓存，平时 20fps 公转若每帧重放矢量指令，
/// 会带来可观的 CPU 路径细分与 GPU 绘制提交开销。
/// 这里把静态金线 + 星点按外环实际包围盒以 Retina 1:1 烘焙成一张 ui.Image，只在窗口缩放或拖拽高亮环切换时重烘，
/// 平时每帧只执行 1 次贴图绘制；拖拽时的磁吸落点微光环仍以 3 个圆实时叠加绘制。
class _CachedOrbitTracks extends StatefulWidget {
  const _CachedOrbitTracks({required this.orbit, this.dragActive = false, this.highlightedTrackId, this.snapPoint});

  final OrbitGeometry orbit;
  final bool dragActive;
  final int? highlightedTrackId;
  final Offset? snapPoint;

  @override
  State<_CachedOrbitTracks> createState() => _CachedOrbitTracksState();
}

class _CachedOrbitTracksState extends State<_CachedOrbitTracks> {
  final _cache = _OrbitTracksCache();

  @override
  void dispose() {
    _cache.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      painter: _OrbitTracksPainter(
        cache: _cache,
        dpr: MediaQuery.devicePixelRatioOf(context),
        orbit: widget.orbit,
        dragActive: widget.dragActive,
        highlightedTrackId: widget.highlightedTrackId,
        snapPoint: widget.snapPoint,
      ),
    );
  }
}

class _OrbitTracksCache {
  ui.Image? image;
  Rect bounds = Rect.zero;
  Size _stageSize = Size.zero;
  double _dpr = 0;
  OrbitGeometry? _orbit;
  bool _dragActive = false;
  int? _highlightedTrackId;

  ui.Image? ensure({
    required Size stageSize,
    required double dpr,
    required OrbitGeometry orbit,
    required bool dragActive,
    required int? highlightedTrackId,
  }) {
    if (stageSize.isEmpty || dpr <= 0) return null;
    final existing = image;
    if (existing != null &&
        _stageSize == stageSize &&
        _dpr == dpr &&
        _orbit == orbit &&
        _dragActive == dragActive &&
        _highlightedTrackId == highlightedTrackId) {
      return existing;
    }

    // 按三条环的真实投影外接框紧致裁出烘焙区域（留足 12px 柔光边距），比整屏缓存省约 65% 显存与填充带宽。
    var minX = double.infinity;
    var minY = double.infinity;
    var maxX = double.negativeInfinity;
    var maxY = double.negativeInfinity;
    for (final track in OrbitTrack.all) {
      for (var i = 0; i < 64; i++) {
        final p = OrbitPose.projectTrack(orbit, track, (i / 64) * math.pi * 2).position;
        if (p.dx < minX) minX = p.dx;
        if (p.dy < minY) minY = p.dy;
        if (p.dx > maxX) maxX = p.dx;
        if (p.dy > maxY) maxY = p.dy;
      }
    }
    const pad = 12.0;
    final left = (minX - pad).clamp(0.0, stageSize.width).floorToDouble();
    final top = (minY - pad).clamp(0.0, stageSize.height).floorToDouble();
    final right = (maxX + pad).clamp(left, stageSize.width).ceilToDouble();
    final bottom = (maxY + pad).clamp(top, stageSize.height).ceilToDouble();
    final rect = Rect.fromLTRB(left, top, right, bottom);
    if (rect.isEmpty) return null;

    final pw = (rect.width * dpr).ceil().clamp(1, 8192);
    final ph = (rect.height * dpr).ceil().clamp(1, 8192);
    final recorder = ui.PictureRecorder();
    final canvas = Canvas(recorder);
    canvas
      ..scale(dpr, dpr)
      ..translate(-rect.left, -rect.top);
    _OrbitTracksPainter.paintTracks(canvas, orbit, dragActive: dragActive, highlightedTrackId: highlightedTrackId);
    final picture = recorder.endRecording();
    final next = picture.toImageSync(pw, ph);
    picture.dispose();

    existing?.dispose();
    image = next;
    bounds = Rect.fromLTWH(rect.left, rect.top, pw / dpr, ph / dpr);
    _stageSize = stageSize;
    _dpr = dpr;
    _orbit = orbit;
    _dragActive = dragActive;
    _highlightedTrackId = highlightedTrackId;
    return next;
  }

  void dispose() {
    image?.dispose();
    image = null;
  }
}

/// 文艺复兴金箔细线 + 星点轨道（三条固定通用轨道，对应内环 3 / 中环 4 / 外环 7），和星球走完全相同的微倾纵深椭圆：
/// 整层画在太阳与全部行星后面（球永远在线前面，不会被金线切过球脸）；金丝线本身仍按 3D 深度近实远淡、近粗远细。
/// 当用户拖拽星球时，三条环整体微亮，进入磁吸区的目标环点亮暖金高光并在落点处绘制磁吸光环。
class _OrbitTracksPainter extends CustomPainter {
  const _OrbitTracksPainter({
    required this.cache,
    required this.dpr,
    required this.orbit,
    this.dragActive = false,
    this.highlightedTrackId,
    this.snapPoint,
  });

  final _OrbitTracksCache cache;
  final double dpr;
  final OrbitGeometry orbit;
  final bool dragActive;
  final int? highlightedTrackId;
  final Offset? snapPoint;

  static void paintTracks(Canvas canvas, OrbitGeometry orbit, {required bool dragActive, int? highlightedTrackId}) {
    final haloPaint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round;
    final linePaint = Paint()
      ..style = PaintingStyle.stroke
      ..strokeCap = StrokeCap.round;
    final dotPaint = Paint()..style = PaintingStyle.fill;

    for (var rIdx = 0; rIdx < OrbitTrack.all.length; rIdx++) {
      final track = OrbitTrack.all[rIdx];
      final b = orbit.radiusFor(track.frac);
      final tier = 1.0 - rIdx * 0.09;
      final isHit = highlightedTrackId == track.id;
      final boost = isHit ? 1.95 : (dragActive ? 1.28 : 1.0);
      final widthBoost = isHit ? 1.55 : (dragActive ? 1.12 : 1.0);

      // 1. 连续闭合细金丝线（按深度分段调粗细与透明度：近处暖金清晰，远处收细变淡融进天色）
      const segs = 128;
      var prev = OrbitPose.projectTrack(orbit, track, 0);
      for (var i = 1; i <= segs; i++) {
        final ecc = (i / segs) * math.pi * 2;
        final cur = OrbitPose.projectTrack(orbit, track, ecc);
        final k = (prev.scale + cur.scale) * 0.5;
        final haze = (prev.haze + cur.haze) * 0.5;
        final near = ((k - 0.68) / 0.75).clamp(0.0, 1.0);
        final depthVis = 0.45 + 0.55 * near;

        // 外层极淡暖金柔光（保证在下方浅蓝/白云处依然能隐约看见金线；磁吸命中时柔光加宽）
        haloPaint
          ..strokeWidth = (isHit ? 5.2 : 2.6) * (0.65 + 0.35 * near)
          ..color = (isHit ? const Color(0xFFFFD878) : const Color(0xFFD4A246)).withValues(
            alpha: (0.16 * tier * depthVis * boost).clamp(0.0, 0.62),
          );
        canvas.drawLine(prev.position, cur.position, haloPaint);

        // 内层纤细金箔线
        linePaint
          ..strokeWidth = (0.85 + 0.45 * near) * widthBoost
          ..color = Color.lerp(
            isHit ? const Color(0xFFFFF8DF) : const Color(0xFFFFEEBD),
            const Color(0xFFE7CC8E),
            isHit ? haze * 0.35 : haze,
          )!.withValues(alpha: (0.36 * tier * depthVis * boost).clamp(0.0, 0.92));
        canvas.drawLine(prev.position, cur.position, linePaint);

        prev = cur;
      }

      // 2. 沿金线点缀细碎金点（近大亮、远小淡）
      final dotSteps = (b * 0.16).round().clamp(28, 72);
      final baseR = (rIdx == 0 ? 1.15 : 1.0) * (isHit ? 1.25 : 1.0);
      for (var i = 0; i < dotSteps; i++) {
        final ecc = (i / dotSteps) * math.pi * 2;
        final pose = OrbitPose.projectTrack(orbit, track, ecc);
        final k = pose.scale;
        final near = ((k - 0.68) / 0.75).clamp(0.0, 1.0);
        dotPaint.color = const Color(0xFFFFF1C6)
            .withValues(alpha: (0.46 * tier * (0.42 + 0.58 * near) * boost).clamp(0.0, 0.95));
        canvas.drawCircle(pose.position, baseR * k, dotPaint);
      }
    }
  }

  @override
  void paint(Canvas canvas, Size size) {
    final img = cache.ensure(
      stageSize: size,
      dpr: dpr,
      orbit: orbit,
      dragActive: dragActive,
      highlightedTrackId: highlightedTrackId,
    );
    if (img != null) {
      canvas.drawImageRect(
        img,
        Rect.fromLTWH(0, 0, img.width.toDouble(), img.height.toDouble()),
        cache.bounds,
        Paint()..filterQuality = FilterQuality.none,
      );
    }

    // 3. 磁吸落点微光环（清楚标示松手吸附在轨道上的位置，仅拖拽吸附时实时画 3 个圆）
    if (snapPoint != null) {
      final p = snapPoint!;
      canvas.drawCircle(
        p,
        16,
        Paint()
          ..style = PaintingStyle.fill
          ..color = const Color(0x38FFE7A0),
      );
      canvas.drawCircle(
        p,
        11,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.6
          ..color = const Color(0xE0FFF4CC),
      );
      canvas.drawCircle(
        p,
        3.2,
        Paint()
          ..style = PaintingStyle.fill
          ..color = const Color(0xFFFFFAF0),
      );
    }
  }

  @override
  bool shouldRepaint(_OrbitTracksPainter old) =>
      old.cache != cache ||
      old.dpr != dpr ||
      old.orbit != orbit ||
      old.dragActive != dragActive ||
      old.highlightedTrackId != highlightedTrackId ||
      old.snapPoint != snapPoint;
}

/// 拖拽星球时浮现在三条轨道右侧边缘的容量徽章（方案 2A：平时完全隐藏，拖动时淡入显示 `1/3`、`2/3`、`3/3` / `x/4` / `x/7`）。
class _OrbitCapacityBadges extends StatelessWidget {
  const _OrbitCapacityBadges({required this.orbit, required this.stageWidth, required this.layout, required this.drag});

  final OrbitGeometry orbit;
  final double stageWidth;
  final OrbitLayout layout;
  final _OrbitDragState? drag;

  @override
  Widget build(BuildContext context) {
    final active = drag != null;
    final snappedId = (drag != null && drag!.snap.snapped) ? drag!.snap.track.id : null;
    final previewLayout = drag?.preview?.layout;
    final evictedCount = drag?.preview == null ? 0 : drag!.preview!.changes.where((c) => c.evicted).length;

    return AnimatedOpacity(
      duration: Motion.fast,
      curve: Motion.curve,
      opacity: active ? 1.0 : 0.0,
      child: Stack(
        children: [
          for (final track in OrbitTrack.all)
            () {
              final anchor = OrbitPose.badgeAnchor(orbit, track);
              final isTarget = snappedId == track.id;
              final isOrigin = drag?.originTrackId == track.id;
              final curCount = layout.countOf(track.id);
              final nextCount = previewLayout?.countOf(track.id) ?? curCount;
              final willReplace = isTarget && !isOrigin && curCount >= track.capacity && evictedCount > 0;
              final willAdd = isTarget && !isOrigin && !willReplace && nextCount != curCount;

              final countText = willAdd
                  ? '$curCount/${track.capacity} → $nextCount/${track.capacity}'
                  : '$curCount/${track.capacity}';
              final tag = isTarget
                  ? (isOrigin
                        ? '当前'
                        : willReplace
                        ? '替换'
                        : '吸附')
                  : null;

              final left = (anchor.dx + 12).clamp(12.0, math.max(12.0, stageWidth - 138.0)).toDouble();
              final top = anchor.dy - 14;
              return Positioned(
                key: ValueKey('orbit-badge-${track.id}'),
                left: left,
                top: top,
                child: AnimatedScale(
                  duration: Motion.fast,
                  scale: isTarget ? 1.06 : 0.96,
                  child: Container(
                    padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 4),
                    decoration: BoxDecoration(
                      color: isTarget
                          ? (willReplace ? const Color(0xCC5A3310) : const Color(0xCC24355E))
                          : const Color(0x9E162242),
                      borderRadius: BorderRadius.circular(999),
                      border: Border.all(
                        color: isTarget
                            ? (willReplace ? const Color(0xE6FFD17A) : const Color(0xEBFFEAB0))
                            : const Color(0x5CEAD7A6),
                        width: isTarget ? 1.15 : 0.75,
                      ),
                      boxShadow: const [BoxShadow(color: Color(0x45081228), blurRadius: 12, offset: Offset(0, 4))],
                    ),
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        Container(
                          width: 6,
                          height: 6,
                          decoration: BoxDecoration(
                            shape: BoxShape.circle,
                            color: isTarget
                                ? (willReplace ? const Color(0xFFFFC75F) : const Color(0xFFFFEEB4))
                                : const Color(0xB8F5DFAA),
                          ),
                        ),
                        const SizedBox(width: 6),
                        Text(
                          '${track.label} $countText',
                          style: TextStyle(
                            fontSize: 11.5,
                            fontWeight: isTarget ? FontWeight.w600 : FontWeight.w500,
                            fontFeatures: const [ui.FontFeature.tabularFigures()],
                            color: isTarget ? const Color(0xFFFFFCF0) : const Color(0xE0F2E8CF),
                            height: 1.1,
                          ),
                        ),
                        if (tag != null) ...[
                          const SizedBox(width: 5),
                          Text(
                            tag,
                            style: TextStyle(
                              fontSize: 10.5,
                              fontWeight: FontWeight.w600,
                              color: willReplace ? const Color(0xFFFFD37C) : const Color(0xFFFFE7A6),
                              height: 1.1,
                            ),
                          ),
                        ],
                      ],
                    ),
                  ),
                ),
              );
            }(),
        ],
      ),
    );
  }
}

/// 天穹底部通用的 Liquid Glass 胶囊外壳（与左上角待办和浮空大窗统一清透白玻璃体系）。
class _SkyGlassCapsule extends StatelessWidget {
  const _SkyGlassCapsule({super.key, required this.onTap, required this.child, this.circle = false});

  final VoidCallback onTap;
  final Widget child;
  final bool circle;

  @override
  Widget build(BuildContext context) {
    final radius = BorderRadius.circular(circle ? 999 : 20);
    final shape = circle
        ? const CircleBorder(side: BorderSide(color: Color(0x99FFFFFF), width: 0.7))
        : RoundedSuperellipseBorder(
            borderRadius: radius,
            side: const BorderSide(color: Color(0x99FFFFFF), width: 0.7),
          );
    final shadowShape = circle ? const CircleBorder() : RoundedSuperellipseBorder(borderRadius: radius);
    Widget inner = BackdropFilter.grouped(
      filter: ui.ImageFilter.compose(
        outer: const ui.ColorFilter.matrix(liquidSaturateMatrix),
        inner: ui.ImageFilter.blur(sigmaX: 28, sigmaY: 28),
      ),
      child: DecoratedBox(
        decoration: ShapeDecoration(
          gradient: const LinearGradient(
            begin: Alignment.topLeft,
            end: Alignment.bottomRight,
            colors: [Color(0xBAFFFFFF), Color(0x8CF4F8FE), Color(0x9CEFF5FD)],
            stops: [0.0, 0.56, 1.0],
          ),
          shape: shape,
        ),
        child: Material(
          type: MaterialType.transparency,
          child: InkWell(customBorder: shadowShape, hoverColor: const Color(0x33FFFFFF), onTap: onTap, child: child),
        ),
      ),
    );

    return DecoratedBox(
      decoration: ShapeDecoration(
        shape: shadowShape,
        shadows: const [
          BoxShadow(color: Color(0x38081230), offset: Offset(0, 10), blurRadius: 26, spreadRadius: -8),
          BoxShadow(color: Color(0x14081230), offset: Offset(0, 2), blurRadius: 6),
        ],
      ),
      child: circle ? ClipOval(child: inner) : ClipRSuperellipse(borderRadius: radius, child: inner),
    );
  }
}

/// 右下角：Liquid Glass「+ 新建 Bot」入口（由原“星球形态”胶囊升级合并，支持实时预览全部 9 种可选星球并直接创建）。
class _CreateBotBubble extends StatelessWidget {
  const _CreateBotBubble({required this.active, required this.onTap});

  final bool active;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    const ink = Color(0xFF14171F);
    return Tooltip(
      message: active ? '关闭新建 Bot 弹窗' : '选择星球形态并新建 Bot',
      child: _SkyGlassCapsule(
        key: const ValueKey('create-bot-bubble'),
        onTap: onTap,
        child: SizedBox(
          height: 38,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 13),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                Icon(active ? Icons.close_rounded : Icons.add_rounded, size: 16.5, color: ink),
                const SizedBox(width: 5.5),
                Text(
                  active ? '取消新建' : '新建 Bot',
                  style: const TextStyle(color: ink, fontSize: 13, fontWeight: FontWeight.w600),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 右下角：Liquid Glass “面板”胶囊，点了打开暂留的工作台窗口。有待办时带红点角标。
class _PanelBubble extends StatelessWidget {
  const _PanelBubble({required this.onTap});

  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final badge = state.inboxCount;
    const ink = Color(0xFF14171F);
    return Tooltip(
      message: '打开面板',
      child: _SkyGlassCapsule(
        key: const ValueKey('panel-bubble'),
        onTap: onTap,
        child: SizedBox(
          height: 38,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 14),
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Icon(Icons.space_dashboard_outlined, size: 15.5, color: ink),
                const SizedBox(width: 7),
                const Text(
                  '面板',
                  style: TextStyle(color: ink, fontSize: 13, fontWeight: FontWeight.w600),
                ),
                if (badge > 0) ...[
                  const SizedBox(width: 7),
                  Container(
                    padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1.5),
                    decoration: BoxDecoration(color: const Color(0xFFFF5F57), borderRadius: BorderRadius.circular(999)),
                    child: Text(
                      '$badge',
                      style: const TextStyle(fontSize: 10.5, fontWeight: FontWeight.w600, color: Colors.white),
                    ),
                  ),
                ],
              ],
            ),
          ),
        ),
      ),
    );
  }
}

/// 太阳的半径（逻辑像素）：直径 160。
const _sunRadius = 80.0;

/// 左下角：“我”（打开设置）和连接状态的胶囊。浮在天穹上用 Liquid Glass，嵌在工作台左栏里用轻量半透白底。
class _Footer extends StatelessWidget {
  const _Footer({required this.onSettings, required this.frosted});

  final VoidCallback onSettings;
  final bool frosted;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final (String label, Color color) = switch (state.status) {
      _ when state.rejected != null => ('被主机拒绝', const Color(0xFFFF5F57)),
      ConnStatus.open => ('已连接', const Color(0xFF28C840)),
      ConnStatus.connecting => ('正在连接', const Color(0xFFFEBC2E)),
      ConnStatus.closed => ('断开了', const Color(0xFFFF5F57)),
    };
    final host = state.host?.address.origin.host ?? '';
    const ink = Color(0xFF14171F);

    final avatarContent = const SizedBox.square(
      dimension: 38,
      child: Icon(Icons.person_outline_rounded, size: 18, color: ink),
    );
    final statusContent = SizedBox(
      height: 38,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 13),
        child: Row(
          children: [
            Container(
              width: 7,
              height: 7,
              decoration: BoxDecoration(
                color: color,
                shape: BoxShape.circle,
                boxShadow: [BoxShadow(color: color.withValues(alpha: 0.45), blurRadius: 5)],
              ),
            ),
            const SizedBox(width: 8),
            Expanded(
              child: Text(
                host.isEmpty ? label : '$label · $host',
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: ink, fontSize: 12.5, fontWeight: FontWeight.w600),
              ),
            ),
          ],
        ),
      ),
    );

    if (frosted) {
      return Row(
        children: [
          Tooltip(
            message: '我',
            child: _SkyGlassCapsule(circle: true, onTap: onSettings, child: avatarContent),
          ),
          const SizedBox(width: 8),
          Expanded(
            child: _SkyGlassCapsule(onTap: onSettings, child: statusContent),
          ),
        ],
      );
    }

    return Row(
      children: [
        Tooltip(
          message: '我',
          child: Material(
            color: const Color(0x94FFFFFF),
            shape: const CircleBorder(side: BorderSide(color: Color(0xB3FFFFFF), width: 0.6)),
            child: InkWell(customBorder: const CircleBorder(), onTap: onSettings, child: avatarContent),
          ),
        ),
        const SizedBox(width: 8),
        Expanded(
          child: Material(
            color: const Color(0x94FFFFFF),
            shape: RoundedSuperellipseBorder(
              borderRadius: BorderRadius.circular(18),
              side: const BorderSide(color: Color(0xB3FFFFFF), width: 0.6),
            ),
            child: InkWell(
              customBorder: RoundedSuperellipseBorder(borderRadius: BorderRadius.circular(18)),
              onTap: onSettings,
              child: statusContent,
            ),
          ),
        ),
      ],
    );
  }
}
