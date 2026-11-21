// App 的状态：连的哪台主机、连接状态、员工、对话、审批和任务。页面只读这里、调这里的方法。

import 'dart:async';

import 'package:flutter/foundation.dart';

import 'host.dart';
import 'pairing.dart';
import 'protocol/client.dart';
import 'protocol/models.dart';
import 'storage.dart';

typedef ClientFactory = IpClient Function(HostAddress host, String token);
typedef Pairer = Future<Claimed> Function(HostAddress host, String code);

class AppState extends ChangeNotifier {
  AppState({required this.store, ClientFactory? clientFactory, Pairer? pairer})
    : _clientFactory = clientFactory ?? ((h, t) => IpClient(wsUri: h.wsUri, token: t)),
      _pairer = pairer ?? claimPairing;

  final HostStore store;
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

  /// 有路由规则时，新对话默认自动分派。
  bool hasRules = false;

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

  /// 新对话发给谁；null 是“自动分派”。
  String? agentForNew;
  bool _agentForNewChosen = false;

  bool get connected => status == ConnStatus.open;
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
      ..add(client.lagged.listen((_) => unawaited(_loadAll())));
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
      ]);
      if (_client != c) return;
      roles = r[0] as List<Role>;
      agents = r[1] as List<Agent>;
      threads = r[2] as List<ThreadSummary>;
      hasRules = r[3] as bool;
      approvals = r[4] as List<Approval>;
      tasks = r[5] as List<Task>;
      reports = r[6] as List<ReportInfo>;
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
    unawaited(_disconnect());
    super.dispose();
  }
}
