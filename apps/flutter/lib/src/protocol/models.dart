// `/ws` 协议里 App 用到的类型：给界面用的样子（整数、默认值、方便的 getter）。
// JSON 的解析交给从 Rust 生成的 `wire.dart`（backlog F-20），这里只从生成的类型转过来，
// 所以 Rust 那边改了字段、重新生成后，这里用到的地方会编不过。
//
// 认不出的事件、帧和消息内容给一个 Unknown，不抛错（生成的类型已经这样做）。协议只有不兼容的改动才升版本号，
// 服务端新加的事件类型不该让旧 App 断线。

import 'wire.dart' as w;

typedef Json = Map<String, dynamic>;

int? _intOrNull(num? v) => v?.toInt();

// ---------- 服务端发来的帧 ----------

sealed class ServerFrame {
  const ServerFrame();

  factory ServerFrame.fromJson(Json j) => switch (w.ServerFrame.fromJson(j)) {
    w.ServerFrameHello f => HelloFrame(
      protocol: f.protocol.toInt(),
      epoch: f.epoch,
      seq: f.seq.toInt(),
      resumed: f.resumed,
    ),
    w.ServerFrameReply f => ReplyFrame(id: f.id, ok: f.ok, data: f.data, error: f.error),
    w.ServerFrameEvent f => EventFrame(seq: f.seq.toInt(), event: RuntimeEvent.fromWire(f.event)),
    w.ServerFrameLagged f => LaggedFrame(skipped: f.skipped.toInt()),
    w.ServerFrameAlert f => AlertFrame(Alert.fromWire(f.alert)),
    w.ServerFrameDismiss f => DismissFrame(f.tag),
    w.ServerFrameUnknown() => UnknownFrame('${j['type']}'),
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

  factory RuntimeEvent.fromJson(Json j) => RuntimeEvent.fromWire(w.RuntimeEvent.fromJson(j));

  factory RuntimeEvent.fromWire(w.RuntimeEvent e) => switch (e) {
    w.RuntimeEventTurnStarted e => TurnStarted(e.agentId, e.threadId),
    w.RuntimeEventDelta e => Delta(e.agentId, e.threadId, e.text),
    w.RuntimeEventMessage e => MessageEvent(Message.fromWire(e.message)),
    w.RuntimeEventTurnFinished e => TurnFinished(
      agentId: e.agentId,
      threadId: e.threadId,
      steps: e.steps.toInt(),
      usage: Usage.fromWire(e.usage),
      costUsd: e.costUsd.toDouble(),
      elapsedMs: e.elapsedMs.toInt(),
    ),
    w.RuntimeEventTurnFailed e => TurnFailed(e.agentId, e.threadId, e.error),
    w.RuntimeEventTaskChanged e => TaskChanged(Task.fromWire(e.task)),
    w.RuntimeEventApprovalChanged e => ApprovalChanged(Approval.fromWire(e.approval)),
    w.RuntimeEventOrbitPinsChanged e => OrbitPinsChanged([for (final p in e.pins) PinnedOrbit.fromWire(p)]),
    w.RuntimeEventOrgChanged e => OrgChanged(
      roles: [for (final r in e.roles) Role.fromWire(r)],
      agents: [for (final a in e.agents) Agent.fromWire(a)],
    ),
    w.RuntimeEventUnknown e => UnknownEvent('${(e.json as Map?)?['type']}'),
  };
}

/// 星图上人固定了轨道的一位员工（m2-spec D-40）。
class PinnedOrbit {
  const PinnedOrbit({required this.agentId, required this.track, required this.pinnedAtMs});

  factory PinnedOrbit.fromJson(Json j) => PinnedOrbit.fromWire(w.OrbitPin.fromJson(j));

  factory PinnedOrbit.fromWire(w.OrbitPin p) =>
      PinnedOrbit(agentId: p.agentId, track: p.orbitTrack.toInt(), pinnedAtMs: p.orbitPinnedAt.toInt());

  final String agentId;

  /// 0 内环、1 中环、2 外环。
  final int track;

  /// 放上这条轨道的时间（Unix 毫秒），满环挤人时排先后。
  final int pinnedAtMs;
}

/// 主机上的星图轨道表改了（别的设备拖了员工，或者这台自己的改动生效了）。`pins` 是整张表。
class OrbitPinsChanged extends RuntimeEvent {
  const OrbitPinsChanged(this.pins);
  final List<PinnedOrbit> pins;
}

/// 主机上的组织在线改了（m2-spec D-42）：改完后的全部岗位和员工，直接换上。
class OrgChanged extends RuntimeEvent {
  const OrgChanged({required this.roles, required this.agents});
  final List<Role> roles;
  final List<Agent> agents;
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

  factory Usage.fromJson(Json j) => Usage.fromWire(w.Usage.fromJson(j));

  factory Usage.fromWire(w.Usage u) => Usage(
    promptTokens: u.promptTokens.toInt(),
    completionTokens: u.completionTokens.toInt(),
    cachedTokens: u.cachedTokens.toInt(),
    cacheWriteTokens: u.cacheWriteTokens.toInt(),
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

  factory Message.fromJson(Json j) => Message.fromWire(w.Message.fromJson(j));

  factory Message.fromWire(w.Message m) => Message(
    id: m.id,
    threadId: m.threadId,
    atMs: m.atMs.toInt(),
    author: Author.fromWire(m.author),
    body: Body.fromWire(m.body),
    notice: m.notice?.wire,
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

  factory Author.fromJson(Json j) => Author.fromWire(w.Author.fromJson(j));

  factory Author.fromWire(w.Author a) => switch (a) {
    w.AuthorHuman a => HumanAuthor(a.name, channel: a.via?.channel),
    w.AuthorAgent a => AgentAuthor(a.agentId),
    w.AuthorSystem() || w.AuthorUnknown() => const SystemAuthor(),
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

  factory Body.fromJson(Json j) => Body.fromWire(w.Body.fromJson(j));

  factory Body.fromWire(w.Body b) => switch (b) {
    w.BodyText b => TextBody(b.text),
    w.BodyToolCall b => ToolCallBody(b.callId, b.name, b.input),
    w.BodyToolResult b => ToolResultBody(b.callId, b.ok, b.output),
    w.BodySummary b => SummaryBody(b.text),
    w.BodyUnknown b => UnknownBody('${(b.json as Map?)?['kind']}'),
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
  const ThreadSummary({
    required this.threadId,
    required this.lastAtMs,
    required this.messageCount,
    this.preview,
    this.lastAgentId,
  });

  factory ThreadSummary.fromJson(Json j) {
    final t = w.ThreadSummary.fromJson(j);
    return ThreadSummary(
      threadId: t.threadId,
      lastAtMs: t.lastAtMs.toInt(),
      messageCount: t.messageCount.toInt(),
      preview: t.preview,
      lastAgentId: t.lastAgentId,
    );
  }

  final String threadId;
  final int lastAtMs;
  final int messageCount;

  /// 最后一条文字消息的开头。
  final String? preview;

  /// 最后回复的员工；员工还没说过话，或主机太旧（没有这一项）时为空。
  final String? lastAgentId;
}

// ---------- 岗位和员工 ----------

class Role {
  const Role({
    required this.id,
    this.name,
    required this.description,
    this.budgetPerDayUsd,
    this.model = '',
    this.escalateModel,
    this.coordinator = false,
  });

  factory Role.fromJson(Json j) => Role.fromWire(w.Role.fromJson(j));

  factory Role.fromWire(w.Role r) {
    return Role(
      id: r.id,
      name: r.name,
      description: r.description,
      budgetPerDayUsd: r.budgetPerDayUsd?.toDouble(),
      model: r.model.defaultValue,
      escalateModel: r.model.escalate,
      coordinator: r.coordinator ?? false,
    );
  }

  /// 主机明确标了这是协调岗位（`ip.yaml` 里 `coordinator: true`，最多一个）。旧主机没有这一项，都是 false。
  final bool coordinator;

  /// 岗位用的模型，`<端点>/<模型>`。
  final String model;

  /// 高难度或高风险任务升级到的模型；没设时为空。
  final String? escalateModel;

  final String id;
  final String? name;
  final String description;

  /// 每日模型费用上限（美元）；没设时为空。
  final double? budgetPerDayUsd;

  String get displayName => name ?? id;
}

class Agent {
  const Agent({required this.id, required this.role, required this.name, this.celestialStyle});

  factory Agent.fromJson(Json j) => Agent.fromWire(w.Agent.fromJson(j));

  factory Agent.fromWire(w.Agent a) => Agent(id: a.id, role: a.role, name: a.name, celestialStyle: a.celestialStyle);

  final String id;
  final String role;
  final String name;

  /// 星图上用哪种天体画他（`mercury`…`neptune`、`star`，m2-spec D-42）；没设时按 id 自己挑。
  final String? celestialStyle;
}

// ---------- 审批和任务 ----------

class ApprovalOption {
  const ApprovalOption(this.label, this.consequence);

  factory ApprovalOption.fromWire(w.ApprovalOption o) => ApprovalOption(o.label, o.consequence);

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
    this.subjectTaskId,
    this.resolution,
    required this.createdAtMs,
  });

  factory Approval.fromJson(Json j) => Approval.fromWire(w.Approval.fromJson(j));

  factory Approval.fromWire(w.Approval a) {
    final (kind, threadId, agentId) = switch (a.subject) {
      w.ApprovalSubjectRouteMessage s => ('route_message', s.threadId, null),
      w.ApprovalSubjectAgentQuestion s => ('agent_question', s.threadId, s.agentId),
      w.ApprovalSubjectTaskCompletion s => ('task_completion', s.threadId, s.agentId),
      w.ApprovalSubjectQuestion() => ('question', null, null),
      w.ApprovalSubjectUnknown s => ('${(s.json as Map?)?['kind']}', null, null),
    };
    return Approval(
      id: a.id,
      question: a.question,
      options: [for (final o in a.options) ApprovalOption.fromWire(o)],
      recommended: _intOrNull(a.recommended),
      reason: a.reason,
      subjectKind: kind,
      subjectThreadId: threadId,
      subjectAgentId: agentId,
      subjectTaskId: switch (a.subject) {
        w.ApprovalSubjectTaskCompletion s => s.taskId,
        _ => null,
      },
      resolution: switch (a.state) {
        w.ApprovalStateResolved s => Resolution(choice: s.choice.toInt(), by: s.by, atMs: s.atMs.toInt()),
        _ => null,
      },
      createdAtMs: a.createdAtMs.toInt(),
    );
  }

  final String id;
  final String question;
  final List<ApprovalOption> options;

  /// 推荐的选项序号。
  final int? recommended;

  /// 推荐理由或背景。
  final String reason;

  /// `route_message`（路由拿不准）、`agent_question`（员工中途提问）、`task_completion`
  /// （员工交回了要验收的任务，m2-spec D-41：选项 0 通过、1 打回修改）或 `question`。
  /// 这个版本不认识的种类原样留着，不解析员工和对话。
  final String subjectKind;
  final String? subjectThreadId;
  final String? subjectAgentId;

  /// 验收卡片（`task_completion`）验收的任务。
  final String? subjectTaskId;

  /// 选定了才有。
  final Resolution? resolution;
  final int createdAtMs;

  bool get pending => resolution == null;
  bool get isAgentQuestion => subjectKind == 'agent_question';

  /// 是否为「任务完成待审批」类型（与 `agent_question` 员工中途提问解耦，供左中下待审批停泊区消费）。
  bool get isTaskCompletion => subjectKind == 'task_completion' || subjectKind == 'task_approval';
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

  factory Task.fromJson(Json j) => Task.fromWire(w.Task.fromJson(j));

  factory Task.fromWire(w.Task t) => Task(
    id: t.id,
    title: t.title,
    description: t.description,
    status: t.status.wire,
    role: t.role,
    assignee: t.assignee,
    threadId: t.threadId,
    result: t.result,
    createdByAgent: switch (t.createdBy) {
      w.AuthorAgent a => a.agentId,
      _ => null,
    },
    createdByName: switch (t.createdBy) {
      w.AuthorHuman a => a.name,
      _ => null,
    },
    originThreadId: t.originThreadId,
    dueMs: _intOrNull(t.dueMs),
    createdAtMs: t.createdAtMs.toInt(),
    updatedAtMs: t.updatedAtMs.toInt(),
  );

  final String id;
  final String title;
  final String description;

  /// `open`、`in_progress`、`blocked`、`pending_approval`（已完成、等人验收，m2-spec D-41）、`done`、`cancelled`；
  /// 这个版本不认识的状态是空字符串。`pending_approval` 只由主机设置：改成 done 等于验收通过，改成 in_progress 等于打回。
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

  /// 是否处于「已完成、等待人工审批」状态（后端协议补齐 `pending_approval` / `awaiting_approval` 后自动命中）。
  bool get isPendingApproval => status == 'pending_approval' || status == 'awaiting_approval';
}

// ---------- 发消息 ----------

class SendResult {
  const SendResult({required this.message, this.agentId, this.routedReason, this.approvalId});

  factory SendResult.fromJson(Json j) {
    final r = w.SendResult.fromJson(j);
    return SendResult(
      message: Message.fromWire(r.message),
      agentId: r.agentId,
      routedReason: switch (r.decision) {
        w.DecisionRouted d => d.reason,
        _ => null,
      },
      approvalId: r.approvalId,
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

  static Scope fromWire(w.Scope s) => s == w.Scope.admin ? Scope.admin : Scope.daily;

  w.Scope toWire() => this == Scope.admin ? w.Scope.admin : w.Scope.daily;

  String get label => this == Scope.admin ? '管理' : '日常';
}

class DeviceInfo {
  const DeviceInfo({
    required this.id,
    required this.name,
    required this.scope,
    this.pairedAtMs,
    this.lastSeenMs,
    this.online = false,
  });

  factory DeviceInfo.fromJson(Json j) => DeviceInfo.fromWire(w.DeviceInfo.fromJson(j));

  factory DeviceInfo.fromWire(w.DeviceInfo d) => DeviceInfo(
    id: d.id,
    name: d.name,
    scope: Scope.fromWire(d.scope),
    pairedAtMs: d.pairedAtMs.toInt(),
    lastSeenMs: d.lastSeenMs.toInt(),
    online: d.online,
  );

  final String id;
  final String name;
  final Scope scope;

  /// 配对、最后在线的时间。主机总会给；可空只是为了测试里少写几个字段。
  final int? pairedAtMs;
  final int? lastSeenMs;

  /// 现在有没有连着。
  final bool online;
}

// ---------- 记忆（“管理”权限，F-18） ----------

/// 人能直接改的两层：决策者笔记、岗位记忆摘要。
enum MemoryLayer {
  notes,
  summary;

  w.MemoryLayer toWire() => this == notes ? w.MemoryLayer.notes : w.MemoryLayer.summary;
}

/// 一段记忆文字和最后改动时间。
class MemoryText {
  const MemoryText({required this.text, required this.updatedMs});

  factory MemoryText.fromWire(w.MemoryText m) => MemoryText(text: m.text, updatedMs: m.updatedMs.toInt());

  final String text;
  final int updatedMs;
}

/// 一个对话提炼出的记忆（`ThreadMemoryView`）。
class ThreadMemory {
  const ThreadMemory({
    required this.threadId,
    this.title,
    required this.summary,
    required this.generatedMs,
    required this.usageCount,
    this.lastUsageMs,
  });

  factory ThreadMemory.fromWire(w.ThreadMemoryView t) => ThreadMemory(
    threadId: t.threadId,
    title: t.title,
    summary: t.summary,
    generatedMs: t.generatedMs.toInt(),
    usageCount: t.usageCount.toInt(),
    lastUsageMs: _intOrNull(t.lastUsageMs),
  );

  final String threadId;
  final String? title;
  final String summary;
  final int generatedMs;

  /// 被员工读过几次。
  final int usageCount;
  final int? lastUsageMs;
}

/// 人要求的一条更正（`CorrectionView`）。
class MemoryCorrection {
  const MemoryCorrection({
    required this.noteId,
    required this.text,
    required this.createdMs,
    this.threadId,
    this.appliedMs,
  });

  factory MemoryCorrection.fromWire(w.CorrectionView c) => MemoryCorrection(
    noteId: c.noteId.toInt(),
    text: c.text,
    createdMs: c.createdMs.toInt(),
    threadId: c.threadId,
    appliedMs: _intOrNull(c.appliedMs),
  );

  final int noteId;
  final String text;
  final int createdMs;
  final String? threadId;

  /// 哪次合并用上了它；还没用上为空。
  final int? appliedMs;
}

/// `get_memory` 等请求的回复：一个岗位的记忆全貌（`MemoryView`）。
class MemoryView {
  const MemoryView({
    required this.roleId,
    required this.enabled,
    this.notes,
    this.summary,
    required this.summaryFromThreads,
    required this.threads,
    required this.corrections,
  });

  factory MemoryView.fromJson(Json j) {
    final m = w.MemoryView.fromJson(j);
    return MemoryView(
      roleId: m.roleId,
      enabled: m.enabled,
      notes: m.notes == null ? null : MemoryText.fromWire(m.notes!),
      summary: m.summary == null ? null : MemoryText.fromWire(m.summary!),
      summaryFromThreads: m.summaryFromThreads.toInt(),
      threads: [for (final t in m.threads) ThreadMemory.fromWire(t)],
      corrections: [for (final c in m.corrections) MemoryCorrection.fromWire(c)],
    );
  }

  final String roleId;

  /// 这个岗位开没开记忆。没开时摘要不进提示词，也不提炼。
  final bool enabled;
  final MemoryText? notes;
  final MemoryText? summary;

  /// 岗位记忆摘要这一版由多少份对话摘要合并而成；人手写的是 0。
  final int summaryFromThreads;

  /// 各对话的记忆，最近生成的在前。
  final List<ThreadMemory> threads;

  /// 人要求的更正，旧的在前。
  final List<MemoryCorrection> corrections;

  MemoryText? layer(MemoryLayer l) => l == MemoryLayer.notes ? notes : summary;
}

/// 定时任务发给谁（`Target`）。
class ScheduleTarget {
  const ScheduleTarget({required this.role, this.agent, this.escalate = false});

  factory ScheduleTarget.fromWire(w.Target t) => ScheduleTarget(role: t.role, agent: t.agent, escalate: t.escalate);

  final String role;

  /// 指定某个员工；为空时在岗位里挑当前最空闲的。
  final String? agent;

  /// 用岗位配置里的升级模型处理。
  final bool escalate;
}

/// `list_schedules` 的一项（`ScheduleView`）：配置、下次运行时间、上次运行结果。
class Schedule {
  const Schedule({
    required this.id,
    required this.description,
    required this.cron,
    required this.utcOffset,
    this.to,
    required this.text,
    required this.enabled,
    this.nextRunMs,
    this.lastRun,
  });

  factory Schedule.fromJson(Json j) {
    final v = w.ScheduleView.fromJson(j);
    final spec = v.spec;
    return Schedule(
      id: spec.id,
      description: spec.description,
      cron: spec.cron,
      utcOffset: spec.utcOffset,
      to: spec.to == null ? null : ScheduleTarget.fromWire(spec.to!),
      text: spec.text,
      enabled: spec.enabled,
      nextRunMs: _intOrNull(v.nextRunMs),
      lastRun: v.lastRun == null ? null : LastRun.fromWire(v.lastRun!),
    );
  }

  final String id;
  final String description;

  /// 五段 cron：分 时 日 月 周。
  final String cron;

  /// 按哪个时区解读 cron，如 `+08:00`。
  final String utcOffset;

  /// 为空时按路由规则分派。
  final ScheduleTarget? to;

  /// 到点发出的消息。
  final String text;
  final bool enabled;

  /// 下次运行的时间；停用或 cron 永远不会命中时为空。
  final int? nextRunMs;
  final LastRun? lastRun;
}

/// 定时任务上次运行的情况。
class LastRun {
  const LastRun({required this.atMs, required this.ok, required this.detail, this.threadId});

  factory LastRun.fromWire(w.LastRun l) =>
      LastRun(atMs: l.atMs.toInt(), ok: l.ok, detail: l.detail, threadId: l.threadId);

  final int atMs;
  final bool ok;

  /// 成功时是新对话的 id，失败时是原因。
  final String detail;
  final String? threadId;
}

/// `create_pairing` 的回复：配对码只在这里出现一次。
class NewPairing {
  const NewPairing({required this.code, required this.name, required this.expiresMs, this.link, this.qr});

  factory NewPairing.fromJson(Json j) {
    final p = w.NewPairing.fromJson(j);
    return NewPairing(code: p.code, name: p.name, expiresMs: p.expiresMs.toInt(), link: p.link, qr: p.qr);
  }

  /// 6 位数字。
  final String code;
  final String name;
  final int expiresMs;

  /// 扫码打开的地址（`<控制台地址>/#/pair/<配对码>`）。
  final String? link;

  /// `link` 的二维码，每行一串 `1`（黑）和 `0`（白），不含四周留白。
  final List<String>? qr;
}

/// `POST /pair` 成功时的回复：令牌只出现这一次。
class Claimed {
  const Claimed(this.token, this.device);

  factory Claimed.fromJson(Json j) {
    final c = w.Claimed.fromJson(j);
    return Claimed(c.token, DeviceInfo.fromWire(c.device));
  }

  final String token;
  final DeviceInfo device;
}

/// `who_am_i` 的回复。
class WhoAmI {
  const WhoAmI({this.device, required this.scope});

  factory WhoAmI.fromJson(Json j) {
    final me = w.WhoAmI.fromJson(j);
    return WhoAmI(device: me.device == null ? null : DeviceInfo.fromWire(me.device!), scope: Scope.fromWire(me.scope));
  }

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
    this.roleId,
    required this.atMs,
  });

  factory Alert.fromJson(Json j) => Alert.fromWire(w.Alert.fromJson(j));

  factory Alert.fromWire(w.Alert a) => Alert(
    kind: a.kind.wire,
    level: a.level.wire,
    title: a.title,
    body: a.body,
    tag: a.tag,
    threadId: a.threadId,
    roleId: a.roleId,
    atMs: a.atMs.toInt(),
  );

  final String kind;
  final String level;
  final String title;
  final String body;
  final String tag;

  /// 点开去哪个对话；没有时去“待办”。
  final String? threadId;
  final String? roleId;
  final int atMs;
}

/// 通知哪些事（`NotifyLevel`）。
enum NotifyLevel {
  off('off', '不通知'),
  needsMe('needs_me', '只通知要我处理的'),
  all('all', '全部');

  const NotifyLevel(this.wire, this.label);
  final String wire;
  final String label;

  static NotifyLevel parse(Object? v) => values.firstWhere((l) => l.wire == v, orElse: () => needsMe);

  w.NotifyLevel toWire() => w.NotifyLevel.fromJson(wire);
}

/// 这台设备的通知设置（m2-spec D-32 的 `NotifySettings`），服务端按它筛 `alert`。
class NotifySettings {
  const NotifySettings({
    this.level = NotifyLevel.needsMe,
    this.mutedRoles = const [],
    this.muteUntilMs,
    this.preview = true,
  });

  factory NotifySettings.fromJson(Json j) {
    final n = w.NotifySettings.fromJson(j);
    return NotifySettings(
      level: NotifyLevel.parse(n.level.wire),
      mutedRoles: n.mutedRoles,
      muteUntilMs: _intOrNull(n.muteUntilMs),
      preview: n.preview,
    );
  }

  final NotifyLevel level;

  /// 这些岗位的事不通知。
  final List<String> mutedRoles;

  /// 这个时间之前都不通知。
  final int? muteUntilMs;

  /// 通知里带不带预览文字。
  final bool preview;

  /// `clearMute` 为 true 时取消静音。
  NotifySettings copyWith({
    NotifyLevel? level,
    List<String>? mutedRoles,
    int? muteUntilMs,
    bool clearMute = false,
    bool? preview,
  }) => NotifySettings(
    level: level ?? this.level,
    mutedRoles: mutedRoles ?? this.mutedRoles,
    muteUntilMs: clearMute ? null : muteUntilMs ?? this.muteUntilMs,
    preview: preview ?? this.preview,
  );

  w.NotifySettings toWire() =>
      w.NotifySettings(level: level.toWire(), mutedRoles: mutedRoles, muteUntilMs: muteUntilMs, preview: preview);

  Json toJson() => toWire().toJson();
}

// ---------- 报告 ----------

/// `list_reports` 的一项：员工工作目录 `reports/` 下的一份报告。
class ReportInfo {
  const ReportInfo({required this.agentId, required this.path, required this.bytes, required this.modifiedMs});

  factory ReportInfo.fromJson(Json j) {
    final r = w.ReportInfo.fromJson(j);
    return ReportInfo(agentId: r.agentId, path: r.path, bytes: r.bytes.toInt(), modifiedMs: r.modifiedMs.toInt());
  }

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

  factory ReportText.fromJson(Json j) {
    final r = w.ReportText.fromJson(j);
    return ReportText(
      agentId: r.agentId,
      path: r.path,
      text: r.text,
      modifiedMs: r.modifiedMs.toInt(),
      truncated: r.truncated,
    );
  }

  final String agentId;
  final String path;
  final String text;
  final int modifiedMs;

  /// 文件超过 1 MB 时只给了前 1 MB。
  final bool truncated;
}

// ---------- 花费 ----------

/// `spend_today` 的回复：今天（UTC 自然日）的模型费用。
class SpendToday {
  const SpendToday({
    required this.date,
    required this.roles,
    required this.routingUsd,
    required this.tokens,
    required this.routingTokens,
  });

  factory SpendToday.fromJson(Json j) {
    final s = w.SpendToday.fromJson(j);
    return SpendToday(
      date: s.date,
      roles: s.roles.map((k, v) => MapEntry(k, v.toDouble())),
      routingUsd: s.routingUsd.toDouble(),
      tokens: s.tokens.map((k, v) => MapEntry(k, Usage.fromWire(v))),
      routingTokens: Usage.fromWire(s.routingTokens),
    );
  }

  /// `YYYY-MM-DD`。
  final String date;

  /// 岗位 id → 美元。今天还没花钱的岗位不在里面。
  final Map<String, double> roles;

  /// 语义分派调用模型花的美元，不占岗位预算。
  final double routingUsd;

  /// 岗位 id → 今天的 token 用量。
  final Map<String, Usage> tokens;
  final Usage routingTokens;

  double get totalUsd => roles.values.fold(routingUsd, (a, b) => a + b);

  /// 全部岗位加自动分派合计的提示词缓存命中率；还没调过模型时为空。
  double? get cacheHitRate {
    var input = routingTokens.promptTokens;
    var cached = routingTokens.cachedTokens;
    for (final u in tokens.values) {
      input += u.promptTokens;
      cached += u.cachedTokens;
    }
    return input == 0 ? null : cached / input;
  }
}
