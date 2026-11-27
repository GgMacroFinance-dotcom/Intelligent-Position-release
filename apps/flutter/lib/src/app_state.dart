// App 的状态：连的哪台主机、连接状态、员工、对话、审批和任务。页面只读这里、调这里的方法。

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'host.dart';
import 'notifications.dart';
import 'pairing.dart';
import 'protocol/client.dart';
import 'protocol/models.dart';
import 'push/lock_push.dart';
import 'qr/scanner.dart';
import 'star_map.dart';
import 'storage.dart';
import 'ui/shell/completion_approval.dart';
import 'ui/shell/local_bot.dart';
import 'ui/shell/orbit.dart';
import 'ui/shell/planet_importance.dart';

typedef ClientFactory = IpClient Function(HostAddress host, String token);
typedef Pairer = Future<Claimed> Function(HostAddress host, String code);

class AppState extends ChangeNotifier {
  AppState({
    required this.store,
    ClientFactory? clientFactory,
    Pairer? pairer,
    this.notifier = const SilentNotifier(),
    this.presenceEvery = const Duration(seconds: 20),
    this.scanner,
    this.lockPush,
  }) : _clientFactory = clientFactory ?? ((h, t) => IpClient(wsUri: h.wsUri, token: t)),
       _pairer = pairer ?? claimPairing;

  final HostStore store;

  /// 弹系统通知的那一层（F-16）。
  final SystemNotifier notifier;

  /// 在前台时多久报一次 `presence`（m2-spec D-33：主机 30 秒内没收到 `true` 就当不在前台）。
  final Duration presenceEvery;

  /// 扫配对二维码用的相机（F-19）；这个平台不能扫码时为 null，连接页不显示“扫码”。
  final QrScanner? scanner;

  /// 锁屏推送（F-21）；这个平台或构建没有时为 null，“我”页不显示开关。
  final LockPush? lockPush;
  final ClientFactory _clientFactory;
  final Pairer _pairer;

  /// 读完存储之前为 false，界面先显示空白。
  bool ready = false;

  /// 现在连的主机；没配对过时为空，显示“连接主机”页。
  HostEntry? host;
  IpClient? _client;
  final _subs = <StreamSubscription<Object?>>[];

  ConnStatus status = ConnStatus.closed;

  /// 主机拒绝了这台设备（令牌失效等），要重新配对。
  Rejected? rejected;
  bool outdated = false;
  int? serverProtocol;

  /// 加载列表失败时的提示。
  String? loadError;

  WhoAmI? me;
  List<Role> roles = [];
  List<Agent> agents = [];
  List<ThreadSummary> threads = [];
  List<Approval> approvals = [];
  List<Task> tasks = [];

  /// 员工写的报告，新改的在前。
  List<ReportInfo> reports = [];

  /// 今天的模型费用；还没拉到时为空。
  SpendToday? spend;

  /// 有路由规则时，新对话默认自动分派。
  bool hasRules = false;

  /// 关掉了记忆的对话（不进岗位记忆）；主机太旧没有这个请求时为空。
  Set<String>? memoryOff;

  // ---------- 对话 ----------

  /// 打开着的对话；新对话还没发第一条时为空。
  String? threadId;
  List<Message> messages = [];

  /// 正在流式输出的文字：员工 id → 到目前为止的文字。只记打开着的对话。
  final drafts = <String, String>{};

  /// 每个对话最近一次由哪位员工接手，下一条消息默认还找他。
  final threadAgent = <String, String>{};

  /// 每个对话底部的一行说明：谁开始处理、完成用了多少、失败原因、分派理由。
  final turnNote = <String, String>{};

  /// 正在处理一轮的员工。
  final agentsWorking = <String>{};

  /// 上一轮失败的员工 → 错误的第一行；之后成功跑完一轮就清掉。
  final agentFailures = <String, String>{};

  // ---------- 任务完成待审批（1C + 2 + 3B + 4 + 5A，数据同源单一真相） ----------
  //
  // TODO(backend-completion-approval) — 后端状态空缺说明：
  // 当前主机协议（protocol/models.dart + wire.dart）尚未提供 `Task.status = pending_approval`
  // 或 `Approval.subjectKind = task_completion`。
  // 此处通过 [pendingCompletionApprovals] 统一聚合未来后端下发项与前端演示模拟项 [_localCompletionApprovals]，
  // 让左中下待审批停泊区、左上角红绿灯待办与员工三栏工作大窗共享同一份状态源，任一入口审批后三处立即同步。
  final _localCompletionApprovals = <CompletionApproval>[];

  /// 审批通过的时间戳记录（`agentId -> Unix ms`）：用于桌面外壳按规则 2 自动解除该员工此前的手动轨道 pin，让其回自动排轨。
  final completionApprovedAtMs = <String, int>{};

  /// 审批打回修改的员工集合（本地演示态让其飞回活跃中/内环；后端补齐 `pending_approval -> in_progress` 后可由任务态直接驱动）。
  final completionReworkAgents = <String>{};

  /// 演示模式（`IP_DEMO_APPROVALS`）是否已完成首轮自动注入。
  /// 挂在全局 [AppState] 而不是 [DesktopShell] 内部：避免窗口缩窄跨过 980px 切换窄屏再拉宽回来时重建外壳、把已审批清空的演示气泡重新“复活”。
  bool demoCompletionApprovalsSeeded = false;

  /// 用户手动拖拽固定到指定轨道的映射表（`agentId -> OrbitPin`）。
  /// 挂在全局 [AppState] 上，窗口跨 980px 宽/窄屏切换后仍完整保留用户拖好的轨道位置。
  /// TODO(backend-orbit-sync)：后续后端下发 `agent.orbit_track` / `agent.orbit_pinned_at` 时在此合并同步。
  final manualOrbitPins = <String, OrbitPin>{};

  // ---------- 星球重要度手动调节与预留传导/冲突接口（方案 A，后端暂不执行，详见 planet_importance.dart） ----------

  /// 用户通过滚轮或三栏左栏手动调节的重要度覆盖表（`agentId -> PlanetImportanceOverride`）。
  /// 挂在全局 [AppState] 上，窗口跨 980px 缩放切换后完整保留每颗星球调好的大小。
  final manualImportanceOverrides = <String, PlanetImportanceOverride>{};

  /// 预留：重要度变更传导至后端/AI 调度的同步端口（当前默认空实现，不发任何后端请求）。
  ImportanceSyncPort importanceSyncPort = const NoopImportanceSyncPort();

  /// 预留：未来资源紧张/争用时的先后顺序反馈端口（当前默认空实现，不弹任何占位提示）。
  ContentionFeedbackPort contentionFeedbackPort = const NoopContentionFeedbackPort();

  /// 供 [buildStarMap] 直接消费的 `agentId -> importance(0..1)` 视图。
  Map<String, double> get importanceOverrideValues => {
    for (final entry in manualImportanceOverrides.entries) entry.key: entry.value.importance,
  };

  PlanetImportanceOverride? manualImportanceFor(String agentId) => manualImportanceOverrides[agentId];

  bool hasManualImportance(String agentId) => manualImportanceOverrides.containsKey(agentId);

  /// 设置某颗星球（非太阳）的手动重要度（0.0..1.0）。
  ///
  /// [notifySync] 为 true 时立即触发预留的 [importanceSyncPort.syncOverride]（用于三栏左栏点按/松手）；
  /// 连续滚轮滚动时传 false，由外壳防抖停手后再调用 [syncManualImportance] 通知预留接口。
  double setManualImportance(String agentId, double importance, {int? nowMs, bool notifySync = true}) {
    final clamped = importance.clamp(0.0, 1.0);
    final override = PlanetImportanceOverride(
      agentId: agentId,
      importance: clamped,
      updatedAtMs: nowMs ?? DateTime.now().millisecondsSinceEpoch,
    );
    manualImportanceOverrides[agentId] = override;
    notifyListeners();
    if (notifySync) {
      unawaited(importanceSyncPort.syncOverride(override));
    }
    return clamped;
  }

  /// 在滚轮停手防抖后将当前手动覆盖通知预留同步端口（后端未接入时为空操作）。
  void syncManualImportance(String agentId) {
    final override = manualImportanceOverrides[agentId];
    if (override != null) {
      unawaited(importanceSyncPort.syncOverride(override));
    }
  }

  /// 复位某颗星球的手动重要度，恢复由系统按任务负荷与岗位预算自动计算大小。
  void resetManualImportance(String agentId) {
    if (manualImportanceOverrides.remove(agentId) == null) return;
    notifyListeners();
    unawaited(importanceSyncPort.clearOverride(agentId));
  }

  // ---------- 本地新建 Bot（方案 1 前端闭环，详见 ui/shell/local_bot.dart 后端空缺说明） ----------
  //
  // TODO(backend-create-bot) — 后端协议空缺说明：
  // 1. `rust/crates/ip-server/src/protocol.rs` & `ws.rs` 目前只有只读 `ListRoles` / `ListAgents`，
  //    尚未开放 `CreateRole` / `CreateAgent` / `UpdateAgent` / `DeleteAgent` RPC（虽然 `ip-runtime` 底层已有
  //    `Runtime::add_role` 与 `Runtime::spawn_agent`）；
  // 2. `rust/crates/ip-core/src/org.rs` 的 `Agent` 与 `ip-server/src/config.rs` 的 `AgentConfig` 尚无
  //    `celestial_style` 字段，无法持久化用户手选的星球形态；
  // 3. `RuntimeEvent` 尚无 `RoleAdded` / `AgentSpawned` / `AgentRemoved` 实时广播事件。
  // 当前由 [_localBots] 存在全局 [AppState] 中（跨 980px 缩放不丢失，且星图/顶栏/岗位页/三栏窗数据同源），
  // 待后端补齐接口后把 [createLocalBot] / [removeLocalBot] 替换为真实 RPC 即可无缝切换。
  final _localBots = <LocalBotRecord>[];

  List<LocalBotRecord> get localBots => List.unmodifiable(_localBots);

  /// 合并主机员工与前端本地新建 Bot（去重，供星图、开窗顶栏、岗位页同源消费）。
  List<Agent> get allAgents {
    if (_localBots.isEmpty) return agents;
    final seen = {for (final a in agents) a.id};
    return [
      ...agents,
      for (final b in _localBots)
        if (seen.add(b.agent.id)) b.agent,
    ];
  }

  /// 合并主机岗位与前端本地新建岗位（去重，供星图、岗位页与三栏窗 Inspector 同源消费）。
  List<Role> get allRoles {
    if (_localBots.isEmpty) return roles;
    final seen = {for (final r in roles) r.id};
    return [
      ...roles,
      for (final b in _localBots)
        if (b.createdRole != null && seen.add(b.createdRole!.id)) b.createdRole!,
    ];
  }

  /// 用户手选的非太阳星球形态覆盖表（`agentId -> Celestial`）。
  /// TODO(backend-create-bot)：后端 `Agent` 下发 `celestial_style` 后，此处合并远端字段与本地新建项。
  Map<String, Celestial> get celestialOverrides => {
    for (final b in _localBots)
      if (b.celestial.kind != CelestialKind.sun) b.agent.id: b.celestial,
  };

  LocalBotRecord? localBotById(String agentId) {
    for (final b in _localBots) {
      if (b.agent.id == agentId) return b;
    }
    return null;
  }

  bool isLocalBot(String agentId) => localBotById(agentId) != null;

  /// 方案 1 本地新建 Bot：校验名称与非太阳形态，生成合法唯一 ID 并立刻注入全局星图与岗位列表。
  /// TODO(backend-create-bot)：后端补齐 `create_role` / `create_agent` 后改为发送 [CreateBotDraft.toPlannedWireJson]。
  LocalBotRecord createLocalBot(CreateBotDraft draft, {int? nowMs}) {
    final trimmedName = draft.name.trim();
    if (trimmedName.isEmpty) {
      throw ArgumentError('Bot 名称不能为空');
    }
    if (draft.celestial.kind == CelestialKind.sun) {
      throw ArgumentError('太阳仅保留给中心幕僚长，不可用于新建普通 Bot');
    }

    final existingRoleIds = {for (final r in allRoles) r.id};
    final existingAgentIds = {for (final a in allAgents) a.id};

    Role? createdRole;
    final String roleId;
    if (draft.existingRoleId != null && existingRoleIds.contains(draft.existingRoleId)) {
      roleId = draft.existingRoleId!;
    } else {
      final rawRoleName = (draft.newRoleName ?? '').trim();
      final displayRole = rawRoleName.isNotEmpty ? rawRoleName : trimmedName;
      roleId = generateUniqueSlug(displayRole, prefix: 'role', existingIds: existingRoleIds);
      final desc = (draft.newRoleDescription ?? '').trim();
      final model = draft.modelDefault.trim().isNotEmpty ? draft.modelDefault.trim() : 'openai/gpt-6-luna';
      final escalate = (draft.modelEscalate ?? '').trim();
      final budget = draft.budgetPerDayUsd;
      createdRole = Role(
        id: roleId,
        name: displayRole,
        description: desc.isNotEmpty ? desc : '$displayRole（本地新建岗位，待主机接入动态创建接口后同步）',
        model: model,
        escalateModel: escalate.isEmpty ? null : escalate,
        budgetPerDayUsd: (budget != null && budget > 0) ? budget : null,
      );
    }

    final agentId = generateUniqueSlug(trimmedName, prefix: 'bot', existingIds: existingAgentIds);
    final record = LocalBotRecord(
      agent: Agent(id: agentId, role: roleId, name: trimmedName),
      createdRole: createdRole,
      celestial: draft.celestial,
      tools: draft.tools,
      workspace: (draft.workspace ?? '').trim().isEmpty ? null : draft.workspace!.trim(),
      createdAtMs: nowMs ?? DateTime.now().millisecondsSinceEpoch,
    );
    _localBots.add(record);
    notifyListeners();
    return record;
  }

  /// 移除一条前端本地新建的 Bot（同步清理其轨道固定与演示待审批记录）。
  /// TODO(backend-create-bot)：后端补齐 `delete_agent` 后改为调用远端接口。
  void removeLocalBot(String agentId) {
    final before = _localBots.length;
    _localBots.removeWhere((b) => b.agent.id == agentId);
    if (_localBots.length == before) return;
    manualOrbitPins.remove(agentId);
    manualImportanceOverrides.remove(agentId);
    _localCompletionApprovals.removeWhere((x) => x.agentId == agentId);
    completionApprovedAtMs.remove(agentId);
    completionReworkAgents.remove(agentId);
    notifyListeners();
  }

  // ---------- 通知（F-16） ----------

  /// 这台设备的通知设置；还没拉到、或者主机太旧没有这个请求时为空。
  NotifySettings? notify;

  /// 系统允不允许这个 App 弹通知；还没问过时为空。
  bool? notificationsAllowed;

  /// App 在不在前台（窗口有焦点、手机上没切走）。在前台时不弹通知，界面自己会更新。
  bool foreground = true;

  /// 点了通知、还没被主界面处理的去向：`(threadId,)`，threadId 为空是去“待办”。
  (String?,)? pendingOpen;

  /// 点过几次通知；桌面上每点一次把窗口叫出来一次（F-17）。
  int alertTaps = 0;

  Timer? _presenceTimer;

  /// 新对话发给谁；null 是“自动分派”。
  String? agentForNew;
  bool _agentForNewChosen = false;

  bool get connected => status == ConnStatus.open;

  /// 连着的客户端，给“管理”的各页直接发请求用（F-18）；没连上时抛错。
  IpClient get client {
    final c = _client;
    if (c == null || c.status != ConnStatus.open) throw const RequestError('尚未连接');
    return c;
  }

  /// 重新问一次主机这台设备是谁、什么权限（在设备页改了自己的权限后用）。
  Future<void> refreshMe() async {
    me = await client.whoAmI();
    notifyListeners();
  }

  bool get isAdmin => me?.scope == Scope.admin;

  List<Approval> get pendingApprovals => approvals.where((a) => a.pending).toList();
  List<Task> get blockedTasks => tasks.where((t) => t.status == 'blocked').toList();

  /// 所有处于「任务已完成 · 待审批」的统一列表（左中下泊位、左上角待办、三栏窗同源读取）。
  List<CompletionApproval> get pendingCompletionApprovals => CompletionApproval.collect(
    approvals: approvals,
    tasks: tasks,
    agentName: agentName,
    localPending: _localCompletionApprovals,
  );

  /// 某位员工当前的主待审批记录（若存在，其星球缩小离轨并停泊到左中下底栏）。
  CompletionApproval? completionApprovalForAgent(String agentId) {
    for (final item in pendingCompletionApprovals) {
      if (item.agentId == agentId) return item;
    }
    return null;
  }

  /// 导航角标：待决定的卡片 + 受阻的任务 + 未重复计数的完成待审批任务，与左上角红绿灯待办保持一致。
  int get inboxCount {
    final regularApprovals = pendingApprovals.where((a) => !a.isTaskCompletion).length;
    return regularApprovals + blockedTasks.length + pendingCompletionApprovals.length;
  }

  /// TODO(backend-completion-approval)：在后端补齐「完成待审批」协议前，触发一条本地模拟记录供实机联调与演示。
  /// 若该员工已存在本地待审批项则忽略重复添加。
  void triggerDemoCompletionApproval({
    required String agentId,
    required String agentName,
    String? taskTitle,
    String? summary,
    int? nowMs,
  }) {
    if (completionApprovalForAgent(agentId) != null) return;
    final ts = nowMs ?? DateTime.now().millisecondsSinceEpoch;
    completionReworkAgents.remove(agentId);
    _localCompletionApprovals.add(
      CompletionApproval(
        id: 'demo:$agentId:$ts',
        agentId: agentId,
        agentName: agentName,
        taskTitle: taskTitle ?? '$agentName 提交的阶段交付物',
        summary: summary ?? '任务执行完毕，结果已整理就绪，等待你的审批验收后归档。',
        createdAtMs: ts,
        source: CompletionSource.localDemo,
      ),
    );
    notifyListeners();
  }

  /// 统一的「完成待审批」决策入口（由左中下泊位打开的三栏工作窗与左上角待办卡片共同调用，保证双向同步）。
  ///
  /// 规则 2：
  /// - [CompletionDecision.approve]：记录通过时间（桌面外壳据此自动解除该员工的手动轨道 pin 并飞回自动排轨）；
  /// - [CompletionDecision.reject]：标记回活跃修改态（星球飞回中环/内环继续处理）。
  Future<void> decideCompletionApproval(CompletionApproval item, CompletionDecision decision) async {
    final now = DateTime.now().millisecondsSinceEpoch;
    switch (item.source) {
      case CompletionSource.backendApproval:
        // TODO(backend-completion-approval)：后端新增 `task_completion` Approval 后，
        // 约定 choice 0 = 通过（后端联动将关联任务置 done），choice 1 = 打回修改（任务置回 in_progress）。
        final approval = item.approval;
        if (approval != null) {
          await resolveApproval(approval, decision == CompletionDecision.approve ? 0 : 1);
        }
      case CompletionSource.backendTask:
        // TODO(backend-completion-approval)：后端新增 `pending_approval` TaskStatus 后，
        // 通过 -> `done`，打回 -> `in_progress`。
        final task = item.task;
        if (task != null) {
          await updateTask(task, decision == CompletionDecision.approve ? 'done' : 'in_progress');
        }
      case CompletionSource.localDemo:
        _localCompletionApprovals.removeWhere((x) => x.id == item.id);
    }

    if (decision == CompletionDecision.approve) {
      completionReworkAgents.remove(item.agentId);
      completionApprovedAtMs[item.agentId] = now;
    } else {
      completionReworkAgents.add(item.agentId);
    }
    notifyListeners();
  }

  String agentName(String id) {
    for (final a in allAgents) {
      if (a.id == id) return a.name;
    }
    return id;
  }

  /// 打开着的对话现在找谁：上次接手的员工，或者新对话选的那位；null 是自动分派。
  String? get currentAgent {
    final id = threadId;
    if (id != null && threadAgent.containsKey(id)) return threadAgent[id];
    return agentForNew;
  }

  // ---------- 主机 ----------

  Future<void> init() async {
    await notifier.init(_alertTapped);
    final push = lockPush;
    if (push != null) {
      // 点锁屏推送的通知打开 App，和点本地通知一样去那个对话或“待办”。
      push.channel.taps.listen((t) => _alertTapped(t.isEmpty ? null : t));
      push.channel.tokenChanges.listen((_) => unawaited(_syncLockPush()));
      try {
        final t = await push.channel.takeTapped();
        if (t != null) _alertTapped(t.isEmpty ? null : t);
      } catch (_) {}
    }
    final hosts = await store.hosts();
    if (hosts.isNotEmpty) {
      final token = await store.token(hosts.first.address);
      if (token != null) _connect(hosts.first, token);
    }
    ready = true;
    notifyListeners();
  }

  /// 配对：先确认安全存储能用，再用一次性的配对码换令牌，存好后连上。
  Future<void> pair(HostAddress address, String code, {bool trusted = false}) async {
    try {
      await store.probe();
    } catch (e) {
      throw PairingError('系统的安全存储用不了，令牌没地方放：$e');
    }
    final claimed = await _pairer(address, code);
    final entry = HostEntry(address: address, deviceName: claimed.device.name, trusted: trusted);
    await store.save(entry, claimed.token);
    _connect(entry, claimed.token);
    notifyListeners();
  }

  /// 断开并删掉这台主机和它的令牌。主机那边的设备记录要在控制台“设备”页吊销。
  Future<void> forgetHost() async {
    final h = host;
    if (h != null) await lockPush?.forget(_client?.status == ConnStatus.open ? _client : null, h.address.text);
    await _disconnect();
    if (h != null) await store.forget(h.address);
    _reset();
    host = null;
    notifyListeners();
  }

  void _connect(HostEntry entry, String token) {
    host = entry;
    rejected = null;
    final client = _clientFactory(entry.address, token);
    _client = client;
    _subs
      ..add(client.statusChanges.listen(_onStatus))
      ..add(client.events.listen(_onEvent))
      ..add(client.lagged.listen((_) => unawaited(_loadAll())))
      ..add(client.alerts.listen(_onAlert))
      ..add(client.dismissals.listen((tag) => unawaited(notifier.cancel(tag))));
    status = ConnStatus.connecting;
    client.connect();
  }

  Future<void> _disconnect() async {
    // 取消订阅不用等：不再有事件进来就行。
    for (final s in _subs) {
      unawaited(s.cancel());
    }
    _subs.clear();
    final c = _client;
    _client = null;
    _presenceTimer?.cancel();
    _presenceTimer = null;
    await c?.dispose();
    status = ConnStatus.closed;
  }

  void _reset() {
    me = null;
    roles = [];
    agents = [];
    threads = [];
    approvals = [];
    tasks = [];
    reports = [];
    spend = null;
    hasRules = false;
    threadId = null;
    messages = [];
    drafts.clear();
    threadAgent.clear();
    turnNote.clear();
    agentsWorking.clear();
    agentFailures.clear();
    _localCompletionApprovals.clear();
    completionApprovedAtMs.clear();
    completionReworkAgents.clear();
    agentForNew = null;
    _agentForNewChosen = false;
    loadError = null;
    notify = null;
  }

  /// App 回到前台：断着的话马上重连。
  void resumed() => _client?.reconnectNow();

  void _onStatus(ConnStatus s) {
    final c = _client;
    if (c == null) return;
    status = s;
    rejected = c.rejected;
    serverProtocol = c.serverProtocol;
    outdated = c.outdated;
    // 重连时服务端补发了断开期间的事件，界面已经跟上，不用整个重新拉（D-29）。
    if (s == ConnStatus.open && !c.resumed) unawaited(_loadAll());
    if (s == ConnStatus.open) {
      _reportPresence();
      // 第一次连上时向系统要通知权限：配对以前问，人还不知道要通知什么。
      if (notificationsAllowed == null) {
        unawaited(askNotificationPermission());
      } else {
        unawaited(_syncLockPush());
      }
    }
    notifyListeners();
  }

  // ---------- 通知（F-16） ----------

  /// 向系统要通知权限。“我”页上被拒绝后可以再点一次。
  Future<void> askNotificationPermission() async {
    notificationsAllowed = await notifier.requestPermission();
    notifyListeners();
    await _syncLockPush();
  }

  /// 连着主机时登记锁屏推送（F-21）：同一个订阅再告诉主机一次，令牌变了换新的。
  Future<void> _syncLockPush() async {
    final push = lockPush;
    final c = _client;
    final h = host;
    if (push == null || c == null || h == null || c.status != ConnStatus.open) return;
    await push.sync(c, h.address.text, permitted: notificationsAllowed == true);
  }

  /// “我”页的锁屏推送开关。
  Future<void> setLockPush(bool on) async {
    final push = lockPush;
    final h = host;
    if (push == null || h == null) return;
    final c = _client;
    await push.setEnabled(on, c != null && c.status == ConnStatus.open ? c : null, h.address.text);
    notifyListeners();
  }

  /// 服务端已按这台设备的设置筛过；这里只管弹不弹：App 在前台时不弹。
  void _onAlert(Alert a) {
    if (foreground) return;
    unawaited(notifier.show(a));
  }

  void _alertTapped(String? threadId) {
    pendingOpen = (threadId,);
    alertTaps++;
    notifyListeners();
  }

  /// 关掉推进去的页面，回主界面打开这个对话（“管理”各页里点“对话”时用）。
  void openThreadFromAnywhere(String threadId) {
    pendingOpen = (threadId,);
    notifyListeners();
  }

  /// 岗位名；没有名字时用 id。
  String roleName(String id) {
    for (final r in roles) {
      if (r.id == id) return r.name ?? r.id;
    }
    return id;
  }

  /// 主界面处理完点通知的去向后调。
  (String?,)? takePendingOpen() {
    final p = pendingOpen;
    pendingOpen = null;
    return p;
  }

  /// App 切到前台或后台（窗口得到、失去焦点）。
  void setForeground(bool value) {
    if (foreground == value) return;
    foreground = value;
    _reportPresence();
  }

  /// 告诉主机在不在前台；在前台时每 [presenceEvery] 报一次。报不上就算了，主机 30 秒后当作不在前台。
  void _reportPresence() {
    _presenceTimer?.cancel();
    _presenceTimer = null;
    final c = _client;
    if (c == null || c.status != ConnStatus.open) return;
    c.presence(foreground).catchError((Object _) {});
    if (foreground) _presenceTimer = Timer(presenceEvery, _reportPresence);
  }

  /// 改这台设备的通知设置。
  Future<void> saveNotify(NotifySettings s) async {
    final c = _client;
    if (c == null) throw const RequestError('尚未连接');
    notify = await c.setNotifySettings(s);
    notifyListeners();
  }

  Future<void> _loadAll() async {
    final c = _client;
    if (c == null) return;
    try {
      me = await c.whoAmI();
      final r = await Future.wait<Object>([
        c.listRoles(),
        c.listAgents(),
        c.listThreads(),
        c.hasRules(),
        c.listApprovals(),
        c.listTasks(),
        c.listReports(),
        c.spendToday(),
      ]);
      if (_client != c) return;
      roles = r[0] as List<Role>;
      agents = r[1] as List<Agent>;
      threads = r[2] as List<ThreadSummary>;
      hasRules = r[3] as bool;
      approvals = r[4] as List<Approval>;
      tasks = r[5] as List<Task>;
      reports = r[6] as List<ReportInfo>;
      spend = r[7] as SpendToday;
      try {
        memoryOff = (await c.listMemoryOffThreads()).toSet();
      } on RequestError {
        // 主机太旧，没有对话记忆开关。
        memoryOff = null;
      }
      try {
        notify = await c.getNotifySettings();
      } on RequestError {
        // 主机太旧，没有通知设置。
        notify = null;
      }
      if (_client != c) return;
      if (!_agentForNewChosen) agentForNew = hasRules || agents.isEmpty ? null : agents.first.id;
      loadError = null;
      notifyListeners();
      if (threadId != null) await _loadThread();
    } on RequestError catch (e) {
      // 连接断了的话重连后会再拉一遍。
      if (_client == c && c.status == ConnStatus.open) {
        loadError = '加载失败：$e';
        notifyListeners();
      }
    }
  }

  // ---------- 对话 ----------

  Future<void> openThread(String id) async {
    threadId = id;
    messages = [];
    drafts.clear();
    notifyListeners();
    await _loadThread();
  }

  void startNewChat() {
    threadId = null;
    messages = [];
    drafts.clear();
    notifyListeners();
  }

  void chooseAgent(String? agentId) {
    final id = threadId;
    if (id != null) {
      if (agentId == null) {
        threadAgent.remove(id);
      } else {
        threadAgent[id] = agentId;
      }
    } else {
      agentForNew = agentId;
      _agentForNewChosen = true;
    }
    notifyListeners();
  }

  /// 关掉或重新打开一个对话的记忆（要“管理”权限）。关掉时这个对话已提炼进岗位记忆的内容会被抹掉。
  Future<void> setThreadMemory(String id, {required bool enabled}) async {
    memoryOffChanged(await client.setThreadMemory(id, enabled: enabled));
  }

  /// 主机回了新的“关掉了记忆的对话”清单（对话页和记忆页共用）。
  void memoryOffChanged(List<String> off) {
    memoryOff = off.toSet();
    notifyListeners();
  }

  Future<void> _loadThread() async {
    final c = _client;
    final id = threadId;
    if (c == null || id == null) return;
    final list = await c.history(id);
    if (threadId != id) return;
    messages = list;
    for (final m in list.reversed) {
      if (m.author case AgentAuthor(:final agentId)) {
        threadAgent[id] = agentId;
        break;
      }
    }
    notifyListeners();
  }

  /// 发一条消息。新对话的第一条发出后，这个对话就是打开着的对话。
  Future<void> send(String text) async {
    final c = _client;
    if (c == null) throw const RequestError('尚未连接');
    final res = await c.send(text, agentId: currentAgent, threadId: threadId);
    final m = res.message;
    final agent = res.agentId;
    if (agent != null) threadAgent[m.threadId] = agent;
    if (res.routedReason != null && agent != null) {
      turnNote[m.threadId] = '自动分派给 ${agentName(agent)}。${res.routedReason}';
    } else if (res.approvalId != null) {
      turnNote[m.threadId] = '路由拿不准，已在“待办”里发起决策卡片，选定岗位后员工会接着处理。';
    }
    threadId ??= m.threadId;
    if (threadId == m.threadId && !messages.any((x) => x.id == m.id)) messages = [...messages, m];
    notifyListeners();
  }

  Future<void> resolveApproval(Approval a, int choice) async {
    final c = _client;
    if (c == null) throw const RequestError('尚未连接');
    _upsertApproval(await c.resolveApproval(a.id, choice));
    notifyListeners();
  }

  void _upsertApproval(Approval a) {
    approvals = [a, ...approvals.where((x) => x.id != a.id)]..sort((x, y) => y.createdAtMs.compareTo(x.createdAtMs));
  }

  /// 新建任务，指定负责人时主机另开一个对话交给他。
  Future<Task> createTask(String title, {String? description, String? assignee, int? dueMs}) async {
    final c = _client;
    if (c == null) throw const RequestError('尚未连接');
    final t = await c.createTask(title, description: description, assignee: assignee, dueMs: dueMs);
    tasks = [t, ...tasks.where((x) => x.id != t.id)];
    notifyListeners();
    return t;
  }

  /// 改任务的状态（待办里受阻的任务、完成的任务）。
  Future<void> updateTask(Task t, String status) async {
    final c = _client;
    if (c == null) throw const RequestError('尚未连接');
    final next = await c.updateTask(t.id, status: status);
    tasks = [next, ...tasks.where((x) => x.id != next.id)]..sort((a, b) => b.updatedAtMs.compareTo(a.updatedAtMs));
    notifyListeners();
  }

  /// 报告是谁写的：先在报告列表里找这个路径，找不到就用 `hint`（比如写这条消息的员工）。
  /// 协调员转述研究员的报告时 hint 是协调员，列表里能找到真正写报告的人（和控制台一样）。
  String? reportOwner(String path, String? hint) {
    final found = reports.where((r) => r.path == path).toList();
    if (hint != null && found.any((r) => r.agentId == hint)) return hint;
    return found.isNotEmpty ? found.first.agentId : hint;
  }

  /// 读一份报告。列表里还没有这份时先刷新一次列表。
  Future<ReportText> readReport(String path, {String? hint}) async {
    final c = _client;
    if (c == null) throw const RequestError('尚未连接');
    if (!reports.any((r) => r.path == path)) {
      try {
        reports = await c.listReports();
      } on RequestError {
        // 读不到列表就用 hint 试。
      }
    }
    final agent = reportOwner(path, hint);
    if (agent == null) throw RequestError('找不到报告 $path');
    return c.readReport(agent, path);
  }

  Timer? _reportsTimer;
  Timer? _spendTimer;

  /// 一轮结束后刷新今天的费用（合并短时间内的多次）。
  void _refreshSpendSoon() {
    _spendTimer?.cancel();
    _spendTimer = Timer(const Duration(milliseconds: 500), () async {
      final c = _client;
      if (c == null || !connected) return;
      try {
        spend = await c.spendToday();
        notifyListeners();
      } on RequestError {
        // 断线重连后会整个重新拉。
      }
    });
  }

  /// 一轮结束后员工可能写了新报告，刷新列表（合并短时间内的多次）。
  void _refreshReportsSoon() {
    _reportsTimer?.cancel();
    _reportsTimer = Timer(const Duration(milliseconds: 800), () async {
      final c = _client;
      if (c == null || !connected) return;
      try {
        reports = await c.listReports();
        notifyListeners();
      } on RequestError {
        // 断线重连后会整个重新拉。
      }
    });
  }

  Timer? _threadsTimer;

  /// 有新消息时刷新对话列表，合并短时间内的多次。
  void _refreshThreadsSoon() {
    _threadsTimer?.cancel();
    _threadsTimer = Timer(const Duration(milliseconds: 300), () async {
      final c = _client;
      if (c == null || !connected) return;
      try {
        threads = await c.listThreads();
        notifyListeners();
      } on RequestError {
        // 断线重连后会整个重新拉。
      }
    });
  }

  void _onEvent(RuntimeEvent e) {
    switch (e) {
      case Delta(:final agentId, :final threadId, :final text):
        if (threadId != this.threadId) return;
        drafts[agentId] = (drafts[agentId] ?? '') + text;
      case MessageEvent(:final message):
        if (message.threadId == threadId) {
          if (message.author case AgentAuthor(:final agentId)) drafts.remove(agentId);
          if (!messages.any((x) => x.id == message.id)) messages = [...messages, message];
        }
        _refreshThreadsSoon();
      case TurnStarted(:final agentId, :final threadId):
        agentsWorking.add(agentId);
        turnNote[threadId] = '${agentName(agentId)} 开始处理…';
      case TurnFinished():
        _refreshReportsSoon();
        _refreshSpendSoon();
        drafts.remove(e.agentId);
        agentsWorking.remove(e.agentId);
        agentFailures.remove(e.agentId);
        final u = e.usage;
        final cached = u.promptTokens > 0 ? (100 * u.cachedTokens / u.promptTokens).round() : 0;
        final cost = e.costUsd > 0 ? '，约 \$${e.costUsd.toStringAsFixed(4)}' : '';
        turnNote[e.threadId] =
            '${agentName(e.agentId)} 完成：${e.steps} 步，${(e.elapsedMs / 1000).toStringAsFixed(1)} 秒，'
            '${u.promptTokens + u.completionTokens} tokens（缓存命中 $cached%）$cost';
      case TurnFailed(:final agentId, :final threadId, :final error):
        _refreshSpendSoon();
        drafts.remove(agentId);
        agentsWorking.remove(agentId);
        agentFailures[agentId] = error.split('\n').first;
        turnNote[threadId] = '${agentName(agentId)} 这轮失败：$error';
      case TaskChanged(:final task):
        tasks = [task, ...tasks.where((t) => t.id != task.id)]..sort((a, b) => b.updatedAtMs.compareTo(a.updatedAtMs));
      case ApprovalChanged(:final approval):
        _upsertApproval(approval);
      case UnknownEvent():
        return;
    }
    notifyListeners();
  }

  @override
  void dispose() {
    _threadsTimer?.cancel();
    _reportsTimer?.cancel();
    _spendTimer?.cancel();
    _presenceTimer?.cancel();
    unawaited(_disconnect());
    super.dispose();
  }
}
