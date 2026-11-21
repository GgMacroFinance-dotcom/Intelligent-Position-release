// `/ws` 协议里 App 用到的类型。字段名和 Rust 导出的 TypeScript 类型（rust/web/src/types/）一致，
// 先手写，以后改成从 Rust 生成（backlog F-20）。
//
// 解析时宽进：认不出的事件、帧和消息内容给一个 Unknown，不抛错。协议只有不兼容的改动才升版本号，
// 服务端新加的事件类型不该让旧 App 断线。

typedef Json = Map<String, dynamic>;

int _int(Object? v) => (v as num).toInt();
int? _intOrNull(Object? v) => v == null ? null : (v as num).toInt();
double _double(Object? v) => v == null ? 0 : (v as num).toDouble();
String? _str(Object? v) => v as String?;
List<T> _list<T>(Object? v, T Function(Json) f) => [for (final e in (v as List? ?? const [])) f(e as Json)];

// ---------- 服务端发来的帧 ----------

sealed class ServerFrame {
  const ServerFrame();

  factory ServerFrame.fromJson(Json j) => switch (j['type']) {
    'hello' => HelloFrame(
      protocol: _int(j['protocol']),
      epoch: j['epoch'] as String,
      seq: _int(j['seq']),
      resumed: j['resumed'] as bool,
    ),
    'reply' => ReplyFrame(id: j['id'], ok: j['ok'] as bool, data: j['data'], error: _str(j['error'])),
    'event' => EventFrame(seq: _int(j['seq']), event: RuntimeEvent.fromJson(j['event'] as Json)),
    'lagged' => LaggedFrame(skipped: _int(j['skipped'])),
    'alert' => AlertFrame(Alert.fromJson(j['alert'] as Json)),
    'dismiss' => DismissFrame(j['tag'] as String),
    final t => UnknownFrame('$t'),
  };
}

/// 连上后的第一帧（m2-spec D-29）。收到它才算连上。
class HelloFrame extends ServerFrame {
  const HelloFrame({required this.protocol, required this.epoch, required this.seq, required this.resumed});
  final int protocol;
  final String epoch;
  final int seq;

  /// 是否补发了断开期间的事件。为 false 时要重新拉列表。
  final bool resumed;
}

class ReplyFrame extends ServerFrame {
  const ReplyFrame({required this.id, required this.ok, this.data, this.error});
  final Object? id;
  final bool ok;
  final Object? data;
  final String? error;
}

class EventFrame extends ServerFrame {
  const EventFrame({required this.seq, required this.event});
  final int seq;
  final RuntimeEvent event;
}

class LaggedFrame extends ServerFrame {
  const LaggedFrame({required this.skipped});
  final int skipped;
}

class AlertFrame extends ServerFrame {
  const AlertFrame(this.alert);
  final Alert alert;
}

class DismissFrame extends ServerFrame {
  const DismissFrame(this.tag);
  final String tag;
}

class UnknownFrame extends ServerFrame {
  const UnknownFrame(this.type);
  final String type;
}

// ---------- 运行时事件 ----------

sealed class RuntimeEvent {
  const RuntimeEvent();

  factory RuntimeEvent.fromJson(Json j) => switch (j['type']) {
    'turn_started' => TurnStarted(j['agent_id'] as String, j['thread_id'] as String),
    'delta' => Delta(j['agent_id'] as String, j['thread_id'] as String, j['text'] as String),
    'message' => MessageEvent(Message.fromJson(j['message'] as Json)),
    'turn_finished' => TurnFinished(
      agentId: j['agent_id'] as String,
      threadId: j['thread_id'] as String,
      steps: _int(j['steps']),
      usage: Usage.fromJson(j['usage'] as Json),
      costUsd: _double(j['cost_usd']),
      elapsedMs: _int(j['elapsed_ms']),
    ),
    'turn_failed' => TurnFailed(j['agent_id'] as String, j['thread_id'] as String, j['error'] as String),
    'task_changed' => TaskChanged(Task.fromJson(j['task'] as Json)),
    'approval_changed' => ApprovalChanged(Approval.fromJson(j['approval'] as Json)),
    final t => UnknownEvent('$t'),
  };
}

class TurnStarted extends RuntimeEvent {
  const TurnStarted(this.agentId, this.threadId);
  final String agentId;
  final String threadId;
}

/// 流式输出的一段文字。
class Delta extends RuntimeEvent {
  const Delta(this.agentId, this.threadId, this.text);
  final String agentId;
  final String threadId;
  final String text;
}

class MessageEvent extends RuntimeEvent {
  const MessageEvent(this.message);
  final Message message;
}

class TurnFinished extends RuntimeEvent {
  const TurnFinished({
    required this.agentId,
    required this.threadId,
    required this.steps,
    required this.usage,
    required this.costUsd,
    required this.elapsedMs,
  });
  final String agentId;
  final String threadId;
  final int steps;
  final Usage usage;
  final double costUsd;
  final int elapsedMs;
}

class TurnFailed extends RuntimeEvent {
  const TurnFailed(this.agentId, this.threadId, this.error);
  final String agentId;
  final String threadId;
  final String error;
}

class TaskChanged extends RuntimeEvent {
  const TaskChanged(this.task);
  final Task task;
}

class ApprovalChanged extends RuntimeEvent {
  const ApprovalChanged(this.approval);
  final Approval approval;
}

class UnknownEvent extends RuntimeEvent {
  const UnknownEvent(this.type);
  final String type;
}

class Usage {
  const Usage({this.promptTokens = 0, this.completionTokens = 0, this.cachedTokens = 0, this.cacheWriteTokens = 0});

  factory Usage.fromJson(Json j) => Usage(
    promptTokens: _intOrNull(j['prompt_tokens']) ?? 0,
    completionTokens: _intOrNull(j['completion_tokens']) ?? 0,
    cachedTokens: _intOrNull(j['cached_tokens']) ?? 0,
    cacheWriteTokens: _intOrNull(j['cache_write_tokens']) ?? 0,
  );

  final int promptTokens;
  final int completionTokens;
  final int cachedTokens;
  final int cacheWriteTokens;
}

// ---------- 消息 ----------

class Message {
  const Message({
    required this.id,
    required this.threadId,
    required this.atMs,
    required this.author,
    required this.body,
    this.notice,
  });

  factory Message.fromJson(Json j) => Message(
    id: j['id'] as String,
    threadId: j['thread_id'] as String,
    atMs: _int(j['at_ms']),
    author: Author.fromJson(j['author'] as Json),
    body: Body.fromJson(j['body'] as Json),
    notice: _str(j['notice']),
  );

  final String id;
  final String threadId;
  final int atMs;
  final Author author;
  final Body body;

  /// 系统消息的类型（m2-spec D-26）；人和员工的消息没有。
  final String? notice;
}

sealed class Author {
  const Author();

  factory Author.fromJson(Json j) => switch (j['kind']) {
    'human' => HumanAuthor(j['name'] as String, channel: _str((j['via'] as Json?)?['channel'])),
    'agent' => AgentAuthor(j['agent_id'] as String),
    _ => const SystemAuthor(),
  };
}

class HumanAuthor extends Author {
  const HumanAuthor(this.name, {this.channel});
  final String name;

  /// 从外部渠道来的人才有，如 `slack`。
  final String? channel;
}

class AgentAuthor extends Author {
  const AgentAuthor(this.agentId);
  final String agentId;
}

class SystemAuthor extends Author {
  const SystemAuthor();
}

sealed class Body {
  const Body();

  factory Body.fromJson(Json j) => switch (j['kind']) {
    'text' => TextBody(j['text'] as String),
    'tool_call' => ToolCallBody(j['call_id'] as String, j['name'] as String, j['input']),
    'tool_result' => ToolResultBody(j['call_id'] as String, j['ok'] as bool, j['output'] as String),
    'summary' => SummaryBody(j['text'] as String),
    final k => UnknownBody('$k'),
  };
}

class TextBody extends Body {
  const TextBody(this.text);
  final String text;
}

class ToolCallBody extends Body {
  const ToolCallBody(this.callId, this.name, this.input);
  final String callId;
  final String name;
  final Object? input;
}

class ToolResultBody extends Body {
  const ToolResultBody(this.callId, this.ok, this.output);
  final String callId;
  final bool ok;
  final String output;
}

/// 较早的对话压缩成的摘要。
class SummaryBody extends Body {
  const SummaryBody(this.text);
  final String text;
}

class UnknownBody extends Body {
  const UnknownBody(this.kind);
  final String kind;
}

class ThreadSummary {
  const ThreadSummary({required this.threadId, required this.lastAtMs, required this.messageCount, this.preview});

  factory ThreadSummary.fromJson(Json j) => ThreadSummary(
    threadId: j['thread_id'] as String,
    lastAtMs: _int(j['last_at_ms']),
    messageCount: _int(j['message_count']),
    preview: _str(j['preview']),
  );

  final String threadId;
  final int lastAtMs;
  final int messageCount;

  /// 最后一条文字消息的开头。
  final String? preview;
}

// ---------- 岗位和员工 ----------

class Role {
  const Role({required this.id, this.name, required this.description, this.budgetPerDayUsd});

  factory Role.fromJson(Json j) => Role(
    id: j['id'] as String,
    name: _str(j['name']),
    description: _str(j['description']) ?? '',
    budgetPerDayUsd: (j['budget_per_day_usd'] as num?)?.toDouble(),
  );

  final String id;
  final String? name;
  final String description;

  /// 每日模型费用上限（美元）；没设时为空。
  final double? budgetPerDayUsd;

  String get displayName => name ?? id;
}

class Agent {
  const Agent({required this.id, required this.role, required this.name});

  factory Agent.fromJson(Json j) => Agent(id: j['id'] as String, role: j['role'] as String, name: j['name'] as String);

  final String id;
  final String role;
  final String name;
}

// ---------- 审批和任务 ----------

class ApprovalOption {
  const ApprovalOption(this.label, this.consequence);

  factory ApprovalOption.fromJson(Json j) => ApprovalOption(j['label'] as String, _str(j['consequence']) ?? '');

  final String label;

  /// 选了会发生什么。
  final String consequence;
}

class Approval {
  const Approval({
    required this.id,
    required this.question,
    required this.options,
    this.recommended,
    required this.reason,
    required this.subjectKind,
    this.subjectThreadId,
    this.subjectAgentId,
    this.resolution,
    required this.createdAtMs,
  });

  factory Approval.fromJson(Json j) {
    final subject = j['subject'] as Json;
    final state = j['state'] as Json;
    return Approval(
      id: j['id'] as String,
      question: j['question'] as String,
      options: _list(j['options'], ApprovalOption.fromJson),
      recommended: _intOrNull(j['recommended']),
      reason: _str(j['reason']) ?? '',
      subjectKind: subject['kind'] as String,
      subjectThreadId: _str(subject['thread_id']),
      subjectAgentId: _str(subject['agent_id']),
      resolution: state['state'] == 'resolved'
          ? Resolution(choice: _int(state['choice']), by: state['by'] as String, atMs: _int(state['at_ms']))
          : null,
      createdAtMs: _int(j['created_at_ms']),
    );
  }

  final String id;
  final String question;
  final List<ApprovalOption> options;

  /// 推荐的选项序号。
  final int? recommended;

  /// 推荐理由或背景。
  final String reason;

  /// `route_message`（路由拿不准）、`agent_question`（员工提问）或 `question`。
  final String subjectKind;
  final String? subjectThreadId;
  final String? subjectAgentId;

  /// 选定了才有。
  final Resolution? resolution;
  final int createdAtMs;

  bool get pending => resolution == null;
  bool get isAgentQuestion => subjectKind == 'agent_question';
}

class Resolution {
  const Resolution({required this.choice, required this.by, required this.atMs});
  final int choice;
  final String by;
  final int atMs;
}

class Task {
  const Task({
    required this.id,
    required this.title,
    required this.description,
    required this.status,
    this.role,
    this.assignee,
    this.threadId,
    this.result,
    this.createdByAgent,
    this.createdByName,
    this.originThreadId,
    this.dueMs,
    required this.createdAtMs,
    required this.updatedAtMs,
  });

  factory Task.fromJson(Json j) => Task(
    id: j['id'] as String,
    title: j['title'] as String,
    description: _str(j['description']) ?? '',
    status: j['status'] as String,
    role: _str(j['role']),
    assignee: _str(j['assignee']),
    threadId: _str(j['thread_id']),
    result: _str(j['result']),
    createdByAgent: switch (j['created_by']) {
      {'kind': 'agent', 'agent_id': final String id} => id,
      _ => null,
    },
    createdByName: switch (j['created_by']) {
      {'kind': 'human', 'name': final String name} => name,
      _ => null,
    },
    originThreadId: _str(j['origin_thread_id']),
    dueMs: _intOrNull(j['due_ms']),
    createdAtMs: _int(j['created_at_ms']),
    updatedAtMs: _int(j['updated_at_ms']),
  );

  final String id;
  final String title;
  final String description;

  /// `open`、`in_progress`、`blocked`、`done`、`cancelled`。
  final String status;
  final String? role;
  final String? assignee;
  final String? threadId;

  /// 交回的结果或受阻原因。
  final String? result;

  /// 员工派给员工的任务才有：派活的员工。人派的任务为空。
  final String? createdByAgent;

  /// 人派的任务才有：派活的人。
  final String? createdByName;

  /// 派活的对话：任务的结果写回这里。
  final String? originThreadId;

  /// 截止时间；没设时为空。
  final int? dueMs;
  final int createdAtMs;
  final int updatedAtMs;

  /// 还没结束（没完成也没取消）。
  bool get isOpen => status != 'done' && status != 'cancelled';
}

// ---------- 发消息 ----------

class SendResult {
  const SendResult({required this.message, this.agentId, this.routedReason, this.approvalId});

  factory SendResult.fromJson(Json j) {
    final decision = j['decision'] as Json?;
    return SendResult(
      message: Message.fromJson(j['message'] as Json),
      agentId: _str(j['agent_id']),
      routedReason: decision?['kind'] == 'routed' ? _str(decision?['reason']) : null,
      approvalId: _str(j['approval_id']),
    );
  }

  final Message message;

  /// 实际接手的员工；路由拿不准、等人决定时为空。
  final String? agentId;

  /// 由路由规则分派时的理由。
  final String? routedReason;

  /// 路由拿不准时新建的审批卡片。
  final String? approvalId;
}

// ---------- 设备 ----------

enum Scope {
  daily,
  admin;

  static Scope parse(Object? v) => v == 'admin' ? Scope.admin : Scope.daily;

  String get label => this == Scope.admin ? '管理' : '日常';
}

class DeviceInfo {
  const DeviceInfo({required this.id, required this.name, required this.scope});

  factory DeviceInfo.fromJson(Json j) =>
      DeviceInfo(id: j['id'] as String, name: j['name'] as String, scope: Scope.parse(j['scope']));

  final String id;
  final String name;
  final Scope scope;
}

/// `POST /pair` 成功时的回复：令牌只出现这一次。
class Claimed {
  const Claimed(this.token, this.device);

  factory Claimed.fromJson(Json j) => Claimed(j['token'] as String, DeviceInfo.fromJson(j['device'] as Json));

  final String token;
  final DeviceInfo device;
}

/// `who_am_i` 的回复。
class WhoAmI {
  const WhoAmI({this.device, required this.scope});

  factory WhoAmI.fromJson(Json j) => WhoAmI(
    device: j['device'] == null ? null : DeviceInfo.fromJson(j['device'] as Json),
    scope: Scope.parse(j['scope']),
  );

  /// 用设备令牌连上时是这台设备。
  final DeviceInfo? device;
  final Scope scope;
}

// ---------- 通知 ----------

/// 服务端按这台设备的通知设置筛过的一条通知（m2-spec D-32）。
class Alert {
  const Alert({
    required this.kind,
    required this.level,
    required this.title,
    required this.body,
    required this.tag,
    this.threadId,
    required this.atMs,
  });

  factory Alert.fromJson(Json j) => Alert(
    kind: j['kind'] as String,
    level: j['level'] as String,
    title: j['title'] as String,
    body: _str(j['body']) ?? '',
    tag: j['tag'] as String,
    threadId: _str(j['thread_id']),
    atMs: _int(j['at_ms']),
  );

  final String kind;
  final String level;
  final String title;
  final String body;
  final String tag;
  final String? threadId;
  final int atMs;
}

// ---------- 报告 ----------

/// `list_reports` 的一项：员工工作目录 `reports/` 下的一份报告。
class ReportInfo {
  const ReportInfo({required this.agentId, required this.path, required this.bytes, required this.modifiedMs});

  factory ReportInfo.fromJson(Json j) => ReportInfo(
    agentId: j['agent_id'] as String,
    path: j['path'] as String,
    bytes: _int(j['bytes']),
    modifiedMs: _int(j['modified_ms']),
  );

  final String agentId;

  /// 相对员工工作目录的路径，形如 `reports/2026-11-16-cpi.md`。
  final String path;
  final int bytes;
  final int modifiedMs;

  /// 去掉 `reports/` 的文件名。
  String get fileName => path.replaceFirst(RegExp(r'^reports/'), '');
}

/// `read_report` 的回复。
class ReportText {
  const ReportText({
    required this.agentId,
    required this.path,
    required this.text,
    required this.modifiedMs,
    required this.truncated,
  });

  factory ReportText.fromJson(Json j) => ReportText(
    agentId: j['agent_id'] as String,
    path: j['path'] as String,
    text: j['text'] as String,
    modifiedMs: _int(j['modified_ms']),
    truncated: j['truncated'] as bool? ?? false,
  );

  final String agentId;
  final String path;
  final String text;
  final int modifiedMs;

  /// 文件超过 1 MB 时只给了前 1 MB。
  final bool truncated;
}
