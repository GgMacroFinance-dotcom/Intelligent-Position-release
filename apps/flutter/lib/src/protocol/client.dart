// `/ws` 客户端：请求和回复按 id 配对，事件广播给订阅者，断线后自动重连。
// 重连时带上次收到的事件序号，服务端补发断开期间的事件（docs/ip/m2-spec.md D-29）。
// 和网页控制台的 rust/web/src/client.js 是同一套做法。

import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'models.dart';

/// 这份 App 说的 `/ws` 协议版本，和服务端 `events.rs` 的 `PROTOCOL_VERSION` 对应。
const int kProtocol = 1;

enum ConnStatus { connecting, open, closed }

/// 一条连上的 WebSocket。测试里换成假的。
abstract class FrameSocket {
  Stream<dynamic> get frames;
  void send(String text);
  Future<void> close();
}

/// 建一条连接：`headers` 里带令牌。连不上时抛错；主机拒绝握手时抛 [WebSocketException]，带 HTTP 状态码。
typedef SocketConnector = Future<FrameSocket> Function(Uri uri, Map<String, String> headers);

class _IoFrameSocket implements FrameSocket {
  _IoFrameSocket(this._ws);
  final WebSocket _ws;

  @override
  Stream<dynamic> get frames => _ws;

  @override
  void send(String text) => _ws.add(text);

  @override
  Future<void> close() => _ws.close();
}

/// 用 `dart:io` 连。原生 App 不带 `Origin`，主机按非浏览器客户端处理（apps-plan 9.2）。
Future<FrameSocket> ioConnect(Uri uri, Map<String, String> headers) async {
  // 连接交给调用方，由它关。
  // ignore: close_sinks
  final ws = await WebSocket.connect(uri.toString(), headers: headers).timeout(const Duration(seconds: 15));
  // 手机切到后台、换网络时连接常常悄悄断掉；定时 ping，收不到 pong 就当断开，触发重连。
  ws.pingInterval = const Duration(seconds: 20);
  return _IoFrameSocket(ws);
}

/// 请求失败：主机回了错误，或者连接断了、超时了。
class RequestError implements Exception {
  const RequestError(this.message);
  final String message;

  @override
  String toString() => message;
}

/// 主机拒绝了这个连接，重连也没用，要人处理：401 是令牌不对或设备已吊销，403 是来源或地址不被接受。
class Rejected {
  const Rejected(this.status, this.message);
  final int status;
  final String message;
}

class IpClient {
  IpClient({
    required this.wsUri,
    this.token,
    SocketConnector? connector,
    this.firstRetry = const Duration(milliseconds: 500),
    this.maxRetry = const Duration(seconds: 8),
    this.requestTimeout = const Duration(seconds: 30),
  }) : _connector = connector ?? ioConnect,
       _retry = firstRetry;

  final Uri wsUri;
  final String? token;
  final SocketConnector _connector;
  final Duration firstRetry;
  final Duration maxRetry;
  final Duration requestTimeout;

  ConnStatus status = ConnStatus.connecting;

  /// 服务端本次启动的标识；连上过才有。
  String? epoch;

  /// 收到的最后一个事件序号。
  int seq = 0;

  /// 这次连上时服务端是否补发了断开期间的事件。为 false 时要重新拉列表。
  bool resumed = false;

  /// 服务端说的协议版本。
  int? serverProtocol;

  /// 主机拒绝了连接时才有；有了就不再重连。
  Rejected? rejected;

  /// 服务端的协议版本和这份 App 不同：App 或主机该升级了。
  bool get outdated => serverProtocol != null && serverProtocol != kProtocol;

  final _status = StreamController<ConnStatus>.broadcast(sync: true);
  final _events = StreamController<RuntimeEvent>.broadcast(sync: true);
  final _lagged = StreamController<void>.broadcast(sync: true);
  final _alerts = StreamController<Alert>.broadcast(sync: true);
  final _dismiss = StreamController<String>.broadcast(sync: true);

  Stream<ConnStatus> get statusChanges => _status.stream;
  Stream<RuntimeEvent> get events => _events.stream;

  /// 补不上断开期间或跟不上时漏掉的事件，要整个重新拉。
  Stream<void> get lagged => _lagged.stream;

  /// 服务端按这台设备的通知设置筛过的通知（m2-spec D-32）。
  Stream<Alert> get alerts => _alerts.stream;

  /// 撤掉某个标签的通知。
  Stream<String> get dismissals => _dismiss.stream;

  FrameSocket? _socket;
  StreamSubscription<dynamic>? _sub;
  Timer? _timer;
  Duration _retry;
  bool _stopped = false;
  bool _opening = false;
  int _nextId = 1;
  final _pending = <int, Completer<Object?>>{};

  void connect() {
    _stopped = false;
    rejected = null;
    _open();
  }

  /// 不等退避，马上重连（App 从后台回到前台时用）。连着的时候什么都不做。
  void reconnectNow() {
    if (_stopped || rejected != null || _socket != null || _opening) return;
    _timer?.cancel();
    _timer = null;
    _retry = firstRetry;
    _open();
  }

  Future<void> _open() async {
    _timer = null;
    _setStatus(ConnStatus.connecting);
    var uri = wsUri;
    final e = epoch;
    if (e != null) uri = uri.replace(queryParameters: {...uri.queryParameters, 'since': '$seq', 'epoch': e});
    // 令牌放请求头，不放地址，免得出现在反向代理的访问日志里。
    final t = token;
    final headers = t == null || t.isEmpty ? <String, String>{} : {'Authorization': 'Bearer $t'};
    final FrameSocket socket;
    _opening = true;
    try {
      socket = await _connector(uri, headers);
    } on WebSocketException catch (err) {
      final code = err.httpStatusCode;
      if (code == 401 || code == 403) {
        rejected = Rejected(code!, code == 401 ? '主机不认这台设备的令牌：可能已经吊销了，要重新配对' : '主机拒绝了这个连接（403），请检查主机地址和主机的访问令牌设置');
        _setStatus(ConnStatus.closed);
        return;
      }
      _scheduleRetry();
      return;
    } catch (_) {
      _scheduleRetry();
      return;
    } finally {
      _opening = false;
    }
    if (_stopped) {
      await socket.close();
      return;
    }
    _socket = socket;
    _sub = socket.frames.listen(_onData, onDone: _onClosed, onError: (Object _) => _onClosed(), cancelOnError: true);
  }

  void _scheduleRetry() {
    if (_stopped) return;
    _setStatus(ConnStatus.closed);
    _timer = Timer(_retry, _open);
    final next = _retry * 2;
    _retry = next > maxRetry ? maxRetry : next;
  }

  void _onClosed() {
    if (_socket == null) return;
    _socket = null;
    _sub = null;
    _failPending('连接已断开');
    _scheduleRetry();
  }

  void _failPending(String why) {
    final pending = _pending.values.toList();
    _pending.clear();
    for (final c in pending) {
      c.completeError(RequestError(why));
    }
  }

  void _setStatus(ConnStatus s) {
    status = s;
    _status.add(s);
  }

  void _onData(dynamic data) {
    if (data is! String) return;
    final ServerFrame frame;
    try {
      frame = ServerFrame.fromJson(jsonDecode(data) as Json);
    } catch (_) {
      // 认不出的帧不该把连接弄断。
      return;
    }
    switch (frame) {
      case ReplyFrame(:final id, :final ok, :final data, :final error):
        final c = id is int ? _pending.remove(id) : null;
        if (c == null) return;
        ok ? c.complete(data) : c.completeError(RequestError(error ?? '请求失败'));
      case HelloFrame():
        epoch = frame.epoch;
        seq = frame.seq;
        resumed = frame.resumed;
        serverProtocol = frame.protocol;
        _retry = firstRetry;
        _setStatus(ConnStatus.open);
      case EventFrame(:final event):
        seq = frame.seq;
        _events.add(event);
      case LaggedFrame():
        _lagged.add(null);
      case AlertFrame(:final alert):
        _alerts.add(alert);
      case DismissFrame(:final tag):
        _dismiss.add(tag);
      case UnknownFrame():
        break;
    }
  }

  /// 发请求，返回回复里的 `data`。
  Future<Object?> request(Json request) {
    final socket = _socket;
    if (socket == null || status != ConnStatus.open) return Future.error(const RequestError('尚未连接'));
    final id = _nextId++;
    final c = Completer<Object?>();
    _pending[id] = c;
    socket.send(jsonEncode({'id': id, ...request}));
    return c.future.timeout(
      requestTimeout,
      onTimeout: () {
        _pending.remove(id);
        throw const RequestError('请求超时');
      },
    );
  }

  /// 停止重连并断开。
  Future<void> close() async {
    _stopped = true;
    _timer?.cancel();
    _timer = null;
    final socket = _socket;
    _socket = null;
    unawaited(_sub?.cancel());
    _sub = null;
    _failPending('连接已关闭');
    _setStatus(ConnStatus.closed);
    // 不等对方回关闭帧：主机不回的话会一直卡在这里，“断开并忘掉”也就卡住了。
    unawaited(socket?.close());
  }

  Future<void> dispose() async {
    await close();
    await Future.wait([_status.close(), _events.close(), _lagged.close(), _alerts.close(), _dismiss.close()]);
  }
}

/// App 用到的请求，回复解析成类型。请求类型的说明见 `rust/crates/ip-server/src/protocol.rs`。
extension IpRequests on IpClient {
  Future<List<T>> _list<T>(Json req, T Function(Json) f) async => [
    for (final e in (await request(req) as List? ?? const [])) f(e as Json),
  ];

  Future<WhoAmI> whoAmI() async => WhoAmI.fromJson(await request({'type': 'who_am_i'}) as Json);

  Future<List<Role>> listRoles() => _list({'type': 'list_roles'}, Role.fromJson);

  Future<List<Agent>> listAgents() => _list({'type': 'list_agents'}, Agent.fromJson);

  Future<List<ThreadSummary>> listThreads({int limit = 100}) =>
      _list({'type': 'list_threads', 'limit': limit}, ThreadSummary.fromJson);

  Future<List<Message>> history(String threadId, {int limit = 200}) =>
      _list({'type': 'history', 'thread_id': threadId, 'limit': limit}, Message.fromJson);

  /// 有没有路由规则：有就默认自动分派。
  Future<bool> hasRules() async {
    final view = await request({'type': 'list_rules'}) as Json;
    return ((view['spec'] as Json?)?['rules'] as List?)?.isNotEmpty ?? false;
  }

  Future<List<Approval>> listApprovals({int limit = 50}) =>
      _list({'type': 'list_approvals', 'limit': limit}, Approval.fromJson);

  Future<List<Task>> listTasks({int limit = 200}) => _list({'type': 'list_tasks', 'limit': limit}, Task.fromJson);

  Future<Approval> resolveApproval(String approvalId, int choice) async => Approval.fromJson(
    await request({'type': 'resolve_approval', 'approval_id': approvalId, 'choice': choice}) as Json,
  );

  /// 改任务的状态（`open`、`in_progress`、`blocked`、`done`、`cancelled`），回复改完的任务。
  /// 用设备令牌连上时主机按设备名署名。
  Future<Task> updateTask(String taskId, {String? status, String? result}) async => Task.fromJson(
    await request({'type': 'update_task', 'task_id': taskId, 'status': ?status, 'result': ?result}) as Json,
  );

  /// 新建任务。指定负责人时主机另开一个对话把任务交给他；都不指定时留给人分配。
  /// 用设备令牌连上时主机按设备名署名。
  Future<Task> createTask(String title, {String? description, String? assignee, String? role, int? dueMs}) async =>
      Task.fromJson(
        await request({
          'type': 'create_task',
          'title': title,
          'description': ?description,
          'assignee': ?assignee,
          'role': ?role,
          'due_ms': ?dueMs,
        }) as Json,
      );

  // ---------- 记忆（“管理”权限，F-18） ----------

  Future<MemoryView> _memory(Json req) async => MemoryView.fromJson(await request(req) as Json);

  Future<MemoryView> getMemory(String roleId) => _memory({'type': 'get_memory', 'role_id': roleId});

  /// 整体改写决策者笔记或岗位记忆摘要，空文本等于清空这一层。
  Future<MemoryView> saveMemoryText(String roleId, MemoryLayer layer, String text) =>
      _memory({'type': 'save_memory_text', 'role_id': roleId, 'layer': layer.name, 'text': text});

  /// 抹掉一个对话在这个岗位下的记忆。
  Future<MemoryView> forgetThreadMemory(String roleId, String threadId) =>
      _memory({'type': 'forget_thread_memory', 'role_id': roleId, 'thread_id': threadId});

  /// 删一条人要求的更正。
  Future<MemoryView> deleteMemoryNote(String roleId, int noteId) =>
      _memory({'type': 'delete_memory_note', 'role_id': roleId, 'note_id': noteId});

  /// 清空一个岗位的记忆（摘要、各对话的记忆、更正；决策者笔记不动）。
  Future<MemoryView> clearMemory(String roleId) => _memory({'type': 'clear_memory', 'role_id': roleId});

  /// 关掉了记忆的对话。
  Future<List<String>> listMemoryOffThreads() async => [
    for (final id in (await request({'type': 'list_memory_off_threads'}) as List? ?? const [])) id as String,
  ];

  /// 关掉或重新打开一个对话的记忆，回复改完后关掉了记忆的对话。
  Future<List<String>> setThreadMemory(String threadId, {required bool enabled}) async => [
    for (final id
        in (await request({'type': 'set_thread_memory', 'thread_id': threadId, 'enabled': enabled}) as List? ??
            const []))
      id as String,
  ];

  // ---------- 定时任务（“管理”权限，F-18） ----------

  Future<List<Schedule>> listSchedules() => _list({'type': 'list_schedules'}, Schedule.fromJson);

  /// 立即运行一次（不影响原定的下次运行）。
  Future<SendResult> runSchedule(String scheduleId) async =>
      SendResult.fromJson(await request({'type': 'run_schedule', 'schedule_id': scheduleId}) as Json);

  // ---------- 设备管理（“管理”权限，F-18） ----------

  Future<List<DeviceInfo>> listDevices() => _list({'type': 'list_devices'}, DeviceInfo.fromJson);

  /// 生成一个 2 分钟内有效的 6 位配对码；`linkBase` 是控制台的地址，给了就附上扫码地址和二维码。
  Future<NewPairing> createPairing(String name, {String? linkBase}) async =>
      NewPairing.fromJson(await request({'type': 'create_pairing', 'name': name, 'link_base': ?linkBase}) as Json);

  Future<List<DeviceInfo>> renameDevice(String deviceId, String name) =>
      _list({'type': 'rename_device', 'device_id': deviceId, 'name': name}, DeviceInfo.fromJson);

  Future<List<DeviceInfo>> setDeviceScope(String deviceId, Scope scope) =>
      _list({'type': 'set_device_scope', 'device_id': deviceId, 'scope': scope.name}, DeviceInfo.fromJson);

  /// 吊销一台设备，它的连接立即断开。
  Future<List<DeviceInfo>> revokeDevice(String deviceId) =>
      _list({'type': 'revoke_device', 'device_id': deviceId}, DeviceInfo.fromJson);

  /// 这台设备的通知设置（共用令牌连上时是共用的那份）。
  Future<NotifySettings> getNotifySettings() async =>
      NotifySettings.fromJson(await request({'type': 'get_notify_settings'}) as Json);

  /// 改这台设备的通知设置，回复保存后的设置。日常权限就能改（改的是自己的）。
  Future<NotifySettings> setNotifySettings(NotifySettings s) async =>
      NotifySettings.fromJson(await request({'type': 'set_notify_settings', 'settings': s.toJson()}) as Json);

  /// 告诉主机 App 在不在前台（m2-spec D-33）：在前台时主机不给这台设备推送，30 秒没报 `true` 就当不在。
  Future<void> presence(bool active) => request({'type': 'presence', 'active': active});

  /// 今天（UTC）各岗位和语义分派的模型费用。
  Future<SpendToday> spendToday() async => SpendToday.fromJson(await request({'type': 'spend_today'}) as Json);

  /// 员工写的报告，新改的在前。省略 `agentId` 时列所有员工的。
  Future<List<ReportInfo>> listReports({String? agentId, int limit = 200}) =>
      _list({'type': 'list_reports', 'agent_id': ?agentId, 'limit': limit}, ReportInfo.fromJson);

  /// 读一份报告；`path` 是 `list_reports` 给的 `reports/<文件名>.md`。
  Future<ReportText> readReport(String agentId, String path) async =>
      ReportText.fromJson(await request({'type': 'read_report', 'agent_id': agentId, 'path': path}) as Json);

  /// 省略 `agentId` 时由路由规则决定；省略 `threadId` 时开一个新对话。
  /// 用设备令牌连上时，主机按设备名署名，不用带 `author`（apps-plan 4.4）。
  Future<SendResult> send(String text, {String? agentId, String? threadId}) async => SendResult.fromJson(
    await request({'type': 'send', 'text': text, 'agent_id': ?agentId, 'thread_id': ?threadId}) as Json,
  );
}
