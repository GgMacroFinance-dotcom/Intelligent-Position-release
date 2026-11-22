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
import 'storage.dart';

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

  /// 导航角标：待决定的卡片加受阻的任务，和网页控制台一样。
  int get inboxCount => pendingApprovals.length + blockedTasks.length;

  String agentName(String id) {
    for (final a in agents) {
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
