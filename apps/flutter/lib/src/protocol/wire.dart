// 由 tool/gen_protocol.dart 从 rust/web/src/types/*.ts（ts-rs 从 Rust 导出的）生成，不要手改。
// 改了 Rust 的协议类型：先在 rust/ 下 cargo test 重新导出 .ts，再在 apps/flutter 下
// dart run tool/gen_protocol.dart。`number` 一律是 `num`，整数还是小数由用的地方决定。

// ignore_for_file: unnecessary_cast

/// 一个数字员工：某个岗位的一个实例。
final class Agent {
  const Agent({required this.id, required this.role, required this.name});

  factory Agent.fromJson(Map<String, Object?> j) =>
      Agent(id: j['id'] as String, role: j['role'] as String, name: j['name'] as String);

  final AgentId id;

  final RoleId role;

  final String name;

  Map<String, Object?> toJson() => {'id': id, 'role': role, 'name': name};
}

/// 数字员工 ID。
typedef AgentId = String;

/// 推给设备的一条通知。
final class Alert {
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

  factory Alert.fromJson(Map<String, Object?> j) => Alert(
    kind: AlertKind.fromJson(j['kind']),
    level: AlertLevel.fromJson(j['level']),
    title: j['title'] as String,
    body: j['body'] as String,
    tag: j['tag'] as String,
    threadId: j['thread_id'] == null ? null : j['thread_id'] as String,
    roleId: j['role_id'] == null ? null : j['role_id'] as String,
    atMs: j['at_ms'] as num,
  );

  final AlertKind kind;

  final AlertLevel level;

  final String title;

  /// 预览：去掉表格、代码块和 Markdown 记号，密钥打码，最多两句。设备关了预览时为空。
  final String body;

  /// 同一件事的通知用同一个标签，后来的盖掉先来的；处理过后按标签撤掉。
  final String tag;

  /// 点开去哪个对话；没有时去“待办”。
  final ThreadId? threadId;

  /// 哪个岗位的事，按岗位静音用。
  final RoleId? roleId;

  final num atMs;

  Map<String, Object?> toJson() => {
    'kind': kind.toJson(),
    'level': level.toJson(),
    'title': title,
    'body': body,
    'tag': tag,
    'thread_id': ?threadId,
    'role_id': ?roleId,
    'at_ms': atMs,
  };
}

enum AlertKind {
  approval('approval'),
  question('question'),
  failed('failed'),
  blocked('blocked'),
  reply('reply'),
  taskDone('task_done'),

  /// 这个版本还不认识的值（主机比 App 新）。
  unknown('');

  const AlertKind(this.wire);

  /// JSON 里的写法。
  final String wire;

  static AlertKind fromJson(Object? v) => values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);

  String toJson() => wire;
}

/// 一条通知有多要紧（`apps-plan.md` 5.1）。
enum AlertLevel {
  needsMe('needs_me'),
  all('all'),

  /// 这个版本还不认识的值（主机比 App 新）。
  unknown('');

  const AlertLevel(this.wire);

  /// JSON 里的写法。
  final String wire;

  static AlertLevel fromJson(Object? v) => values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);

  String toJson() => wire;
}

final class Approval {
  const Approval({
    required this.id,
    required this.question,
    required this.options,
    this.recommended,
    required this.reason,
    required this.subject,
    required this.state,
    required this.createdAtMs,
  });

  factory Approval.fromJson(Map<String, Object?> j) => Approval(
    id: j['id'] as String,
    question: j['question'] as String,
    options: [for (final x in j['options'] as List<Object?>) ApprovalOption.fromJson(x as Map<String, Object?>)],
    recommended: j['recommended'] == null ? null : j['recommended'] as num,
    reason: j['reason'] as String,
    subject: ApprovalSubject.fromJson(j['subject'] as Map<String, Object?>),
    state: ApprovalState.fromJson(j['state'] as Map<String, Object?>),
    createdAtMs: j['created_at_ms'] as num,
  );

  final ApprovalId id;

  final String question;

  final List<ApprovalOption> options;

  /// 推荐的选项序号。
  final num? recommended;

  /// 推荐理由或背景。
  final String reason;

  final ApprovalSubject subject;

  final ApprovalState state;

  final num createdAtMs;

  Map<String, Object?> toJson() => {
    'id': id,
    'question': question,
    'options': [for (final x in options) x.toJson()],
    'recommended': recommended,
    'reason': reason,
    'subject': subject.toJson(),
    'state': state.toJson(),
    'created_at_ms': createdAtMs,
  };
}

/// 审批（决策卡片）ID。
typedef ApprovalId = String;

/// 决策卡片上的一个选项。
final class ApprovalOption {
  const ApprovalOption({required this.label, required this.consequence});

  factory ApprovalOption.fromJson(Map<String, Object?> j) =>
      ApprovalOption(label: j['label'] as String, consequence: j['consequence'] as String);

  final String label;

  /// 选了会发生什么。
  final String consequence;

  Map<String, Object?> toJson() => {'label': label, 'consequence': consequence};
}

sealed class ApprovalState {
  const ApprovalState();

  factory ApprovalState.fromJson(Map<String, Object?> j) {
    return switch (j['state']) {
      'pending' => ApprovalStatePending.fromJson(j),
      'resolved' => ApprovalStateResolved.fromJson(j),
      _ => ApprovalStateUnknown(j),
    };
  }

  Object? toJson();
}

final class ApprovalStatePending extends ApprovalState {
  const ApprovalStatePending();

  factory ApprovalStatePending.fromJson(Map<String, Object?> j) => const ApprovalStatePending();

  @override
  Map<String, Object?> toJson() => {'state': 'pending'};
}

final class ApprovalStateResolved extends ApprovalState {
  const ApprovalStateResolved({required this.choice, required this.by, required this.atMs});

  factory ApprovalStateResolved.fromJson(Map<String, Object?> j) =>
      ApprovalStateResolved(choice: j['choice'] as num, by: j['by'] as String, atMs: j['at_ms'] as num);

  final num choice;

  final String by;

  final num atMs;

  @override
  Map<String, Object?> toJson() => {'state': 'resolved', 'choice': choice, 'by': by, 'at_ms': atMs};
}

/// 这个版本还不认识的ApprovalState（主机比 App 新），原样留着。
final class ApprovalStateUnknown extends ApprovalState {
  const ApprovalStateUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// 审批因何而起；决定选项被选中后系统做什么。
sealed class ApprovalSubject {
  const ApprovalSubject();

  factory ApprovalSubject.fromJson(Map<String, Object?> j) {
    return switch (j['kind']) {
      'route_message' => ApprovalSubjectRouteMessage.fromJson(j),
      'agent_question' => ApprovalSubjectAgentQuestion.fromJson(j),
      'question' => ApprovalSubjectQuestion.fromJson(j),
      _ => ApprovalSubjectUnknown(j),
    };
  }

  Object? toJson();
}

final class ApprovalSubjectRouteMessage extends ApprovalSubject {
  const ApprovalSubjectRouteMessage({
    required this.messageId,
    required this.threadId,
    required this.text,
    required this.roles,
  });

  factory ApprovalSubjectRouteMessage.fromJson(Map<String, Object?> j) => ApprovalSubjectRouteMessage(
    messageId: j['message_id'] as String,
    threadId: j['thread_id'] as String,
    text: j['text'] as String,
    roles: [for (final x in j['roles'] as List<Object?>) x as String],
  );

  final MessageId messageId;

  final ThreadId threadId;

  final String text;

  final List<RoleId> roles;

  @override
  Map<String, Object?> toJson() => {
    'kind': 'route_message',
    'message_id': messageId,
    'thread_id': threadId,
    'text': text,
    'roles': roles,
  };
}

final class ApprovalSubjectAgentQuestion extends ApprovalSubject {
  const ApprovalSubjectAgentQuestion({required this.agentId, required this.threadId});

  factory ApprovalSubjectAgentQuestion.fromJson(Map<String, Object?> j) =>
      ApprovalSubjectAgentQuestion(agentId: j['agent_id'] as String, threadId: j['thread_id'] as String);

  final AgentId agentId;

  final ThreadId threadId;

  @override
  Map<String, Object?> toJson() => {'kind': 'agent_question', 'agent_id': agentId, 'thread_id': threadId};
}

final class ApprovalSubjectQuestion extends ApprovalSubject {
  const ApprovalSubjectQuestion();

  factory ApprovalSubjectQuestion.fromJson(Map<String, Object?> j) => const ApprovalSubjectQuestion();

  @override
  Map<String, Object?> toJson() => {'kind': 'question'};
}

/// 这个版本还不认识的ApprovalSubject（主机比 App 新），原样留着。
final class ApprovalSubjectUnknown extends ApprovalSubject {
  const ApprovalSubjectUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// 消息由谁发出。
sealed class Author {
  const Author();

  factory Author.fromJson(Map<String, Object?> j) {
    return switch (j['kind']) {
      'human' => AuthorHuman.fromJson(j),
      'agent' => AuthorAgent.fromJson(j),
      'system' => AuthorSystem.fromJson(j),
      _ => AuthorUnknown(j),
    };
  }

  Object? toJson();
}

final class AuthorHuman extends Author {
  const AuthorHuman({required this.name, this.via});

  factory AuthorHuman.fromJson(Map<String, Object?> j) => AuthorHuman(
    name: j['name'] as String,
    via: j['via'] == null ? null : Via.fromJson(j['via'] as Map<String, Object?>),
  );

  final String name;

  /// 从外部渠道来的人才有：哪个渠道、渠道里的用户 id。控制台用户没有这一项。
  final Via? via;

  @override
  Map<String, Object?> toJson() => {'kind': 'human', 'name': name, 'via': ?via?.toJson()};
}

final class AuthorAgent extends Author {
  const AuthorAgent({required this.agentId});

  factory AuthorAgent.fromJson(Map<String, Object?> j) => AuthorAgent(agentId: j['agent_id'] as String);

  final AgentId agentId;

  @override
  Map<String, Object?> toJson() => {'kind': 'agent', 'agent_id': agentId};
}

final class AuthorSystem extends Author {
  const AuthorSystem();

  factory AuthorSystem.fromJson(Map<String, Object?> j) => const AuthorSystem();

  @override
  Map<String, Object?> toJson() => {'kind': 'system'};
}

/// 这个版本还不认识的Author（主机比 App 新），原样留着。
final class AuthorUnknown extends Author {
  const AuthorUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// 消息内容。
sealed class Body {
  const Body();

  factory Body.fromJson(Map<String, Object?> j) {
    return switch (j['kind']) {
      'text' => BodyText.fromJson(j),
      'tool_call' => BodyToolCall.fromJson(j),
      'tool_result' => BodyToolResult.fromJson(j),
      'summary' => BodySummary.fromJson(j),
      _ => BodyUnknown(j),
    };
  }

  Object? toJson();
}

final class BodyText extends Body {
  const BodyText({required this.text});

  factory BodyText.fromJson(Map<String, Object?> j) => BodyText(text: j['text'] as String);

  final String text;

  @override
  Map<String, Object?> toJson() => {'kind': 'text', 'text': text};
}

final class BodyToolCall extends Body {
  const BodyToolCall({required this.callId, required this.name, this.input});

  factory BodyToolCall.fromJson(Map<String, Object?> j) =>
      BodyToolCall(callId: j['call_id'] as String, name: j['name'] as String, input: j['input']);

  final String callId;

  final String name;

  final Object? input;

  @override
  Map<String, Object?> toJson() => {'kind': 'tool_call', 'call_id': callId, 'name': name, 'input': input};
}

final class BodyToolResult extends Body {
  const BodyToolResult({required this.callId, required this.ok, required this.output});

  factory BodyToolResult.fromJson(Map<String, Object?> j) =>
      BodyToolResult(callId: j['call_id'] as String, ok: j['ok'] as bool, output: j['output'] as String);

  final String callId;

  final bool ok;

  final String output;

  @override
  Map<String, Object?> toJson() => {'kind': 'tool_result', 'call_id': callId, 'ok': ok, 'output': output};
}

final class BodySummary extends Body {
  const BodySummary({required this.text, required this.coversThrough});

  factory BodySummary.fromJson(Map<String, Object?> j) =>
      BodySummary(text: j['text'] as String, coversThrough: j['covers_through'] as String);

  final String text;

  final MessageId coversThrough;

  @override
  Map<String, Object?> toJson() => {'kind': 'summary', 'text': text, 'covers_through': coversThrough};
}

/// 这个版本还不认识的Body（主机比 App 新），原样留着。
final class BodyUnknown extends Body {
  const BodyUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// `POST /pair` 成功时的回复。
final class Claimed {
  const Claimed({required this.token, required this.device});

  factory Claimed.fromJson(Map<String, Object?> j) =>
      Claimed(token: j['token'] as String, device: DeviceInfo.fromJson(j['device'] as Map<String, Object?>));

  /// 这台设备专属的令牌，只在这里出现一次。
  final String token;

  final DeviceInfo device;

  Map<String, Object?> toJson() => {'token': token, 'device': device.toJson()};
}

/// 匹配条件。各项同时满足才算命中；关键词不区分英文大小写。
final class Condition {
  const Condition({required this.any, required this.all, required this.none, this.regex, required this.from});

  factory Condition.fromJson(Map<String, Object?> j) => Condition(
    any: [for (final x in j['any'] as List<Object?>) x as String],
    all: [for (final x in j['all'] as List<Object?>) x as String],
    none: [for (final x in j['none'] as List<Object?>) x as String],
    regex: j['regex'] == null ? null : j['regex'] as String,
    from: [for (final x in j['from'] as List<Object?>) x as String],
  );

  /// 含其中任意一个。
  final List<String> any;

  /// 全部都含。
  final List<String> all;

  /// 一个都不含。
  final List<String> none;

  /// 正则表达式。
  final String? regex;

  /// 发送人是其中之一。控制台用户写名字；外部渠道的用户写 `<渠道>:<用户 id>`（如 `slack:U012ABCDEF`），
  /// 渠道用户不按名字匹配，因为渠道里的显示名谁都能改。
  final List<String> from;

  Map<String, Object?> toJson() => {'any': any, 'all': all, 'none': none, 'regex': ?regex, 'from': from};
}

final class CorrectionView {
  const CorrectionView({
    required this.noteId,
    required this.text,
    required this.createdMs,
    this.threadId,
    this.appliedMs,
  });

  factory CorrectionView.fromJson(Map<String, Object?> j) => CorrectionView(
    noteId: j['note_id'] as num,
    text: j['text'] as String,
    createdMs: j['created_ms'] as num,
    threadId: j['thread_id'] == null ? null : j['thread_id'] as String,
    appliedMs: j['applied_ms'] == null ? null : j['applied_ms'] as num,
  );

  final num noteId;

  final String text;

  final num createdMs;

  final ThreadId? threadId;

  /// 哪次合并用上了它；还没用上是空。
  final num? appliedMs;

  Map<String, Object?> toJson() => {
    'note_id': noteId,
    'text': text,
    'created_ms': createdMs,
    'thread_id': threadId,
    'applied_ms': appliedMs,
  };
}

/// 路由结果，附带写给人看的理由。
sealed class Decision {
  const Decision();

  factory Decision.fromJson(Map<String, Object?> j) {
    return switch (j['kind']) {
      'routed' => DecisionRouted.fromJson(j),
      'needs_human' => DecisionNeedsHuman.fromJson(j),
      _ => DecisionUnknown(j),
    };
  }

  Object? toJson();
}

final class DecisionRouted extends Decision {
  const DecisionRouted({
    required this.by,
    this.ruleId,
    required this.role,
    this.agent,
    required this.escalate,
    required this.reason,
  });

  factory DecisionRouted.fromJson(Map<String, Object?> j) => DecisionRouted(
    by: RouteBy.fromJson(j['by']),
    ruleId: j['rule_id'] == null ? null : j['rule_id'] as String,
    role: j['role'] as String,
    agent: j['agent'] == null ? null : j['agent'] as String,
    escalate: j['escalate'] as bool,
    reason: j['reason'] as String,
  );

  final RouteBy by;

  /// 命中的规则；不是按规则分派时为空。
  final String? ruleId;

  final RoleId role;

  final AgentId? agent;

  final bool escalate;

  final String reason;

  @override
  Map<String, Object?> toJson() => {
    'kind': 'routed',
    'by': by.toJson(),
    'rule_id': ruleId,
    'role': role,
    'agent': agent,
    'escalate': escalate,
    'reason': reason,
  };
}

final class DecisionNeedsHuman extends Decision {
  const DecisionNeedsHuman({required this.reason, this.suggested});

  factory DecisionNeedsHuman.fromJson(Map<String, Object?> j) => DecisionNeedsHuman(
    reason: j['reason'] as String,
    suggested: j['suggested'] == null ? null : j['suggested'] as String,
  );

  final String reason;

  /// 语义匹配给出、但把握不够的岗位，作为卡片上的推荐项。
  final RoleId? suggested;

  @override
  Map<String, Object?> toJson() => {'kind': 'needs_human', 'reason': reason, 'suggested': ?suggested};
}

/// 这个版本还不认识的Decision（主机比 App 新），原样留着。
final class DecisionUnknown extends Decision {
  const DecisionUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// 控制台“设备”页看到的一台设备。
final class DeviceInfo {
  const DeviceInfo({
    required this.id,
    required this.name,
    required this.scope,
    required this.pairedAtMs,
    required this.lastSeenMs,
    required this.online,
  });

  factory DeviceInfo.fromJson(Map<String, Object?> j) => DeviceInfo(
    id: j['id'] as String,
    name: j['name'] as String,
    scope: Scope.fromJson(j['scope']),
    pairedAtMs: j['paired_at_ms'] as num,
    lastSeenMs: j['last_seen_ms'] as num,
    online: j['online'] as bool,
  );

  final String id;

  final String name;

  final Scope scope;

  final num pairedAtMs;

  final num lastSeenMs;

  /// 现在有没有连着。
  final bool online;

  Map<String, Object?> toJson() => {
    'id': id,
    'name': name,
    'scope': scope.toJson(),
    'paired_at_ms': pairedAtMs,
    'last_seen_ms': lastSeenMs,
    'online': online,
  };
}

/// 没有规则命中时怎么办。
sealed class Fallback {
  const Fallback();

  factory Fallback.fromJson(Object? j) {
    if (j == 'ask') return const FallbackAsk();
    if (j is! Map<String, Object?>) return FallbackUnknown(j);
    if (j.containsKey('role')) return FallbackRole.fromJson(j);
    return FallbackUnknown(j);
  }

  Object? toJson();
}

final class FallbackAsk extends Fallback {
  const FallbackAsk();

  @override
  Object? toJson() => 'ask';
}

final class FallbackRole extends Fallback {
  const FallbackRole({required this.role});

  factory FallbackRole.fromJson(Map<String, Object?> j) => FallbackRole(role: j['role'] as String);

  final RoleId role;

  @override
  Map<String, Object?> toJson() => {'role': role};
}

/// 这个版本还不认识的Fallback（主机比 App 新），原样留着。
final class FallbackUnknown extends Fallback {
  const FallbackUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// 上次运行的情况。
final class LastRun {
  const LastRun({required this.atMs, required this.ok, required this.detail, this.threadId});

  factory LastRun.fromJson(Map<String, Object?> j) => LastRun(
    atMs: j['at_ms'] as num,
    ok: j['ok'] as bool,
    detail: j['detail'] as String,
    threadId: j['thread_id'] == null ? null : j['thread_id'] as String,
  );

  final num atMs;

  final bool ok;

  /// 成功时是新对话的 id，失败时是原因。
  final String detail;

  /// 成功时是新对话。
  final ThreadId? threadId;

  Map<String, Object?> toJson() => {'at_ms': atMs, 'ok': ok, 'detail': detail, 'thread_id': ?threadId};
}

/// 上下文里人能直接改的两层。
enum MemoryLayer {
  notes('notes'),
  summary('summary'),

  /// 这个版本还不认识的值（主机比 App 新）。
  unknown('');

  const MemoryLayer(this.wire);

  /// JSON 里的写法。
  final String wire;

  static MemoryLayer fromJson(Object? v) =>
      values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);

  String toJson() => wire;
}

final class MemoryText {
  const MemoryText({required this.text, required this.updatedMs});

  factory MemoryText.fromJson(Map<String, Object?> j) =>
      MemoryText(text: j['text'] as String, updatedMs: j['updated_ms'] as num);

  final String text;

  final num updatedMs;

  Map<String, Object?> toJson() => {'text': text, 'updated_ms': updatedMs};
}

/// `get_memory` 等请求的回复：一个岗位的记忆全貌。
final class MemoryView {
  const MemoryView({
    required this.roleId,
    required this.enabled,
    this.notes,
    this.summary,
    required this.summaryFromThreads,
    required this.threads,
    required this.corrections,
  });

  factory MemoryView.fromJson(Map<String, Object?> j) => MemoryView(
    roleId: j['role_id'] as String,
    enabled: j['enabled'] as bool,
    notes: j['notes'] == null ? null : MemoryText.fromJson(j['notes'] as Map<String, Object?>),
    summary: j['summary'] == null ? null : MemoryText.fromJson(j['summary'] as Map<String, Object?>),
    summaryFromThreads: j['summary_from_threads'] as num,
    threads: [for (final x in j['threads'] as List<Object?>) ThreadMemoryView.fromJson(x as Map<String, Object?>)],
    corrections: [
      for (final x in j['corrections'] as List<Object?>) CorrectionView.fromJson(x as Map<String, Object?>),
    ],
  );

  final RoleId roleId;

  /// 这个岗位开没开记忆（`memory.enabled` 和 `memory.roles`）。没开时摘要不进提示词，也不提炼。
  final bool enabled;

  final MemoryText? notes;

  final MemoryText? summary;

  /// 岗位记忆摘要这一版由多少份对话摘要合并而成；人手写的是 0。
  final num summaryFromThreads;

  /// 各对话的记忆，最近生成的在前。
  final List<ThreadMemoryView> threads;

  /// 人要求的更正，旧的在前。
  final List<CorrectionView> corrections;

  Map<String, Object?> toJson() => {
    'role_id': roleId,
    'enabled': enabled,
    'notes': notes?.toJson(),
    'summary': summary?.toJson(),
    'summary_from_threads': summaryFromThreads,
    'threads': [for (final x in threads) x.toJson()],
    'corrections': [for (final x in corrections) x.toJson()],
  };
}

/// 一条消息。写入后不再修改；更正用新消息表达。
final class Message {
  const Message({
    required this.id,
    required this.threadId,
    required this.atMs,
    required this.author,
    required this.body,
    this.notice,
  });

  factory Message.fromJson(Map<String, Object?> j) => Message(
    id: j['id'] as String,
    threadId: j['thread_id'] as String,
    atMs: j['at_ms'] as num,
    author: Author.fromJson(j['author'] as Map<String, Object?>),
    body: Body.fromJson(j['body'] as Map<String, Object?>),
    notice: j['notice'] == null ? null : Notice.fromJson(j['notice']),
  );

  final MessageId id;

  final ThreadId threadId;

  /// Unix 时间，毫秒。
  final num atMs;

  final Author author;

  final Body body;

  /// 系统消息的类型；人和员工的消息没有这一项。外部渠道按它决定发不发（`docs/ip/m2-spec.md` D-26）。
  final Notice? notice;

  Map<String, Object?> toJson() => {
    'id': id,
    'thread_id': threadId,
    'at_ms': atMs,
    'author': author.toJson(),
    'body': body.toJson(),
    'notice': ?notice?.toJson(),
  };
}

/// 消息 ID。
typedef MessageId = String;

/// 岗位使用的模型。名字的格式是 `<端点>/<模型>`，本地岗位模型写成 `local/<基座>+<LoRA>`。
final class ModelSlot {
  const ModelSlot({required this.defaultValue, this.escalate});

  factory ModelSlot.fromJson(Map<String, Object?> j) =>
      ModelSlot(defaultValue: j['default'] as String, escalate: j['escalate'] == null ? null : j['escalate'] as String);

  final String defaultValue;

  /// 高难度或高风险任务升级到的模型。
  final String? escalate;

  Map<String, Object?> toJson() => {'default': defaultValue, 'escalate': escalate};
}

/// `create_pairing` 的回复：配对码只在这里出现一次。
final class NewPairing {
  const NewPairing({required this.code, required this.name, required this.expiresMs, this.link, this.qr});

  factory NewPairing.fromJson(Map<String, Object?> j) => NewPairing(
    code: j['code'] as String,
    name: j['name'] as String,
    expiresMs: j['expires_ms'] as num,
    link: j['link'] == null ? null : j['link'] as String,
    qr: j['qr'] == null ? null : [for (final x in j['qr'] as List<Object?>) x as String],
  );

  /// 6 位数字。
  final String code;

  /// 配对成功后设备叫什么。
  final String name;

  final num expiresMs;

  /// 扫码打开的地址（`<控制台地址>/#/pair/<配对码>`）；没给控制台地址时没有。
  final String? link;

  /// `link` 的二维码，每行一串 `1`（黑）和 `0`（白），不含四周留白。
  final List<String>? qr;

  Map<String, Object?> toJson() => {'code': code, 'name': name, 'expires_ms': expiresMs, 'link': ?link, 'qr': ?qr};
}

/// 系统消息的类型。
enum Notice {
  taskBrief('task_brief'),
  taskReport('task_report'),
  approvalAnswer('approval_answer'),
  turnFailed('turn_failed'),

  /// 这个版本还不认识的值（主机比 App 新）。
  unknown('');

  const Notice(this.wire);

  /// JSON 里的写法。
  final String wire;

  static Notice fromJson(Object? v) => values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);

  String toJson() => wire;
}

/// 一台设备要哪些通知。
enum NotifyLevel {
  off('off'),
  needsMe('needs_me'),
  all('all'),

  /// 这个版本还不认识的值（主机比 App 新）。
  unknown('');

  const NotifyLevel(this.wire);

  /// JSON 里的写法。
  final String wire;

  static NotifyLevel fromJson(Object? v) =>
      values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);

  String toJson() => wire;
}

/// 一台设备自己的通知设置（`apps-plan.md` 5.1 最后一段）。
final class NotifySettings {
  const NotifySettings({required this.level, required this.mutedRoles, this.muteUntilMs, required this.preview});

  factory NotifySettings.fromJson(Map<String, Object?> j) => NotifySettings(
    level: NotifyLevel.fromJson(j['level']),
    mutedRoles: [for (final x in j['muted_roles'] as List<Object?>) x as String],
    muteUntilMs: j['mute_until_ms'] == null ? null : j['mute_until_ms'] as num,
    preview: j['preview'] as bool,
  );

  final NotifyLevel level;

  /// 这些岗位的事不通知。
  final List<RoleId> mutedRoles;

  /// 这个时间之前都不通知。
  final num? muteUntilMs;

  /// 通知里带不带预览文字。
  final bool preview;

  Map<String, Object?> toJson() => {
    'level': level.toJson(),
    'muted_roles': mutedRoles,
    'mute_until_ms': muteUntilMs,
    'preview': preview,
  };
}

/// 一个组织的全部岗位定义。
final class OrgSpec {
  const OrgSpec({required this.roles});

  factory OrgSpec.fromJson(Map<String, Object?> j) =>
      OrgSpec(roles: [for (final x in j['roles'] as List<Object?>) Role.fromJson(x as Map<String, Object?>)]);

  final List<Role> roles;

  Map<String, Object?> toJson() => {
    'roles': [for (final x in roles) x.toJson()],
  };
}

/// `list_reports` 的一项。
final class ReportInfo {
  const ReportInfo({required this.agentId, required this.path, required this.bytes, required this.modifiedMs});

  factory ReportInfo.fromJson(Map<String, Object?> j) => ReportInfo(
    agentId: j['agent_id'] as String,
    path: j['path'] as String,
    bytes: j['bytes'] as num,
    modifiedMs: j['modified_ms'] as num,
  );

  final AgentId agentId;

  /// 相对员工工作目录的路径，形如 `reports/2026-11-16-cpi.md`。
  final String path;

  final num bytes;

  final num modifiedMs;

  Map<String, Object?> toJson() => {'agent_id': agentId, 'path': path, 'bytes': bytes, 'modified_ms': modifiedMs};
}

/// `read_report` 的回复。
final class ReportText {
  const ReportText({
    required this.agentId,
    required this.path,
    required this.text,
    required this.modifiedMs,
    required this.truncated,
  });

  factory ReportText.fromJson(Map<String, Object?> j) => ReportText(
    agentId: j['agent_id'] as String,
    path: j['path'] as String,
    text: j['text'] as String,
    modifiedMs: j['modified_ms'] as num,
    truncated: j['truncated'] as bool,
  );

  final AgentId agentId;

  final String path;

  final String text;

  final num modifiedMs;

  /// 文件超过 1 MB 时只给前 1 MB。
  final bool truncated;

  Map<String, Object?> toJson() => {
    'agent_id': agentId,
    'path': path,
    'text': text,
    'modified_ms': modifiedMs,
    'truncated': truncated,
  };
}

sealed class Request {
  const Request();

  factory Request.fromJson(Map<String, Object?> j) {
    return switch (j['type']) {
      'list_roles' => RequestListRoles.fromJson(j),
      'list_agents' => RequestListAgents.fromJson(j),
      'spend_today' => RequestSpendToday.fromJson(j),
      'list_threads' => RequestListThreads.fromJson(j),
      'history' => RequestHistory.fromJson(j),
      'search' => RequestSearch.fromJson(j),
      'route' => RequestRoute.fromJson(j),
      'list_rules' => RequestListRules.fromJson(j),
      'save_rules' => RequestSaveRules.fromJson(j),
      'list_rule_drafts' => RequestListRuleDrafts.fromJson(j),
      'list_schedules' => RequestListSchedules.fromJson(j),
      'run_schedule' => RequestRunSchedule.fromJson(j),
      'dismiss_rule_draft' => RequestDismissRuleDraft.fromJson(j),
      'list_tasks' => RequestListTasks.fromJson(j),
      'create_task' => RequestCreateTask.fromJson(j),
      'update_task' => RequestUpdateTask.fromJson(j),
      'list_approvals' => RequestListApprovals.fromJson(j),
      'resolve_approval' => RequestResolveApproval.fromJson(j),
      'get_memory' => RequestGetMemory.fromJson(j),
      'save_memory_text' => RequestSaveMemoryText.fromJson(j),
      'forget_thread_memory' => RequestForgetThreadMemory.fromJson(j),
      'delete_memory_note' => RequestDeleteMemoryNote.fromJson(j),
      'clear_memory' => RequestClearMemory.fromJson(j),
      'list_memory_off_threads' => RequestListMemoryOffThreads.fromJson(j),
      'set_thread_memory' => RequestSetThreadMemory.fromJson(j),
      'list_reports' => RequestListReports.fromJson(j),
      'read_report' => RequestReadReport.fromJson(j),
      'who_am_i' => RequestWhoAmI.fromJson(j),
      'list_devices' => RequestListDevices.fromJson(j),
      'create_pairing' => RequestCreatePairing.fromJson(j),
      'rename_device' => RequestRenameDevice.fromJson(j),
      'set_device_scope' => RequestSetDeviceScope.fromJson(j),
      'revoke_device' => RequestRevokeDevice.fromJson(j),
      'get_notify_settings' => RequestGetNotifySettings.fromJson(j),
      'set_notify_settings' => RequestSetNotifySettings.fromJson(j),
      'push_key' => RequestPushKey.fromJson(j),
      'subscribe_push' => RequestSubscribePush.fromJson(j),
      'unsubscribe_push' => RequestUnsubscribePush.fromJson(j),
      'presence' => RequestPresence.fromJson(j),
      'send' => RequestSend.fromJson(j),
      _ => RequestUnknown(j),
    };
  }

  Object? toJson();
}

final class RequestListRoles extends Request {
  const RequestListRoles();

  factory RequestListRoles.fromJson(Map<String, Object?> j) => const RequestListRoles();

  @override
  Map<String, Object?> toJson() => {'type': 'list_roles'};
}

final class RequestListAgents extends Request {
  const RequestListAgents();

  factory RequestListAgents.fromJson(Map<String, Object?> j) => const RequestListAgents();

  @override
  Map<String, Object?> toJson() => {'type': 'list_agents'};
}

final class RequestSpendToday extends Request {
  const RequestSpendToday();

  factory RequestSpendToday.fromJson(Map<String, Object?> j) => const RequestSpendToday();

  @override
  Map<String, Object?> toJson() => {'type': 'spend_today'};
}

final class RequestListThreads extends Request {
  const RequestListThreads({this.limit});

  factory RequestListThreads.fromJson(Map<String, Object?> j) =>
      RequestListThreads(limit: j['limit'] == null ? null : j['limit'] as num);

  final num? limit;

  @override
  Map<String, Object?> toJson() => {'type': 'list_threads', 'limit': ?limit};
}

final class RequestHistory extends Request {
  const RequestHistory({required this.threadId, this.beforeMs, this.limit});

  factory RequestHistory.fromJson(Map<String, Object?> j) => RequestHistory(
    threadId: j['thread_id'] as String,
    beforeMs: j['before_ms'] == null ? null : j['before_ms'] as num,
    limit: j['limit'] == null ? null : j['limit'] as num,
  );

  final ThreadId threadId;

  final num? beforeMs;

  final num? limit;

  @override
  Map<String, Object?> toJson() => {'type': 'history', 'thread_id': threadId, 'before_ms': ?beforeMs, 'limit': ?limit};
}

final class RequestSearch extends Request {
  const RequestSearch({required this.query, this.threadId, this.limit});

  factory RequestSearch.fromJson(Map<String, Object?> j) => RequestSearch(
    query: j['query'] as String,
    threadId: j['thread_id'] == null ? null : j['thread_id'] as String,
    limit: j['limit'] == null ? null : j['limit'] as num,
  );

  final String query;

  final ThreadId? threadId;

  final num? limit;

  @override
  Map<String, Object?> toJson() => {'type': 'search', 'query': query, 'thread_id': ?threadId, 'limit': ?limit};
}

final class RequestRoute extends Request {
  const RequestRoute({required this.text, this.author});

  factory RequestRoute.fromJson(Map<String, Object?> j) =>
      RequestRoute(text: j['text'] as String, author: j['author'] == null ? null : j['author'] as String);

  final String text;

  final String? author;

  @override
  Map<String, Object?> toJson() => {'type': 'route', 'text': text, 'author': ?author};
}

final class RequestListRules extends Request {
  const RequestListRules();

  factory RequestListRules.fromJson(Map<String, Object?> j) => const RequestListRules();

  @override
  Map<String, Object?> toJson() => {'type': 'list_rules'};
}

final class RequestSaveRules extends Request {
  const RequestSaveRules({required this.spec, this.acceptedDraft});

  factory RequestSaveRules.fromJson(Map<String, Object?> j) => RequestSaveRules(
    spec: RouterSpec.fromJson(j['spec'] as Map<String, Object?>),
    acceptedDraft: j['accepted_draft'] == null ? null : j['accepted_draft'] as String,
  );

  final RouterSpec spec;

  final String? acceptedDraft;

  @override
  Map<String, Object?> toJson() => {'type': 'save_rules', 'spec': spec.toJson(), 'accepted_draft': ?acceptedDraft};
}

final class RequestListRuleDrafts extends Request {
  const RequestListRuleDrafts();

  factory RequestListRuleDrafts.fromJson(Map<String, Object?> j) => const RequestListRuleDrafts();

  @override
  Map<String, Object?> toJson() => {'type': 'list_rule_drafts'};
}

final class RequestListSchedules extends Request {
  const RequestListSchedules();

  factory RequestListSchedules.fromJson(Map<String, Object?> j) => const RequestListSchedules();

  @override
  Map<String, Object?> toJson() => {'type': 'list_schedules'};
}

final class RequestRunSchedule extends Request {
  const RequestRunSchedule({required this.scheduleId});

  factory RequestRunSchedule.fromJson(Map<String, Object?> j) =>
      RequestRunSchedule(scheduleId: j['schedule_id'] as String);

  final String scheduleId;

  @override
  Map<String, Object?> toJson() => {'type': 'run_schedule', 'schedule_id': scheduleId};
}

final class RequestDismissRuleDraft extends Request {
  const RequestDismissRuleDraft({required this.draftId});

  factory RequestDismissRuleDraft.fromJson(Map<String, Object?> j) =>
      RequestDismissRuleDraft(draftId: j['draft_id'] as String);

  final String draftId;

  @override
  Map<String, Object?> toJson() => {'type': 'dismiss_rule_draft', 'draft_id': draftId};
}

final class RequestListTasks extends Request {
  const RequestListTasks({this.openOnly, this.limit});

  factory RequestListTasks.fromJson(Map<String, Object?> j) => RequestListTasks(
    openOnly: j['open_only'] == null ? null : j['open_only'] as bool,
    limit: j['limit'] == null ? null : j['limit'] as num,
  );

  /// 只要未完成的。
  final bool? openOnly;

  final num? limit;

  @override
  Map<String, Object?> toJson() => {'type': 'list_tasks', 'open_only': ?openOnly, 'limit': ?limit};
}

final class RequestCreateTask extends Request {
  const RequestCreateTask({
    required this.title,
    this.description,
    this.role,
    this.assignee,
    this.threadId,
    this.dueMs,
    this.author,
  });

  factory RequestCreateTask.fromJson(Map<String, Object?> j) => RequestCreateTask(
    title: j['title'] as String,
    description: j['description'] == null ? null : j['description'] as String,
    role: j['role'] == null ? null : j['role'] as String,
    assignee: j['assignee'] == null ? null : j['assignee'] as String,
    threadId: j['thread_id'] == null ? null : j['thread_id'] as String,
    dueMs: j['due_ms'] == null ? null : j['due_ms'] as num,
    author: j['author'] == null ? null : j['author'] as String,
  );

  final String title;

  final String? description;

  final RoleId? role;

  final AgentId? assignee;

  final ThreadId? threadId;

  final num? dueMs;

  final String? author;

  @override
  Map<String, Object?> toJson() => {
    'type': 'create_task',
    'title': title,
    'description': ?description,
    'role': ?role,
    'assignee': ?assignee,
    'thread_id': ?threadId,
    'due_ms': ?dueMs,
    'author': ?author,
  };
}

final class RequestUpdateTask extends Request {
  const RequestUpdateTask({required this.taskId, this.status, this.assignee, this.result, this.author});

  factory RequestUpdateTask.fromJson(Map<String, Object?> j) => RequestUpdateTask(
    taskId: j['task_id'] as String,
    status: j['status'] == null ? null : TaskStatus.fromJson(j['status']),
    assignee: j['assignee'] == null ? null : j['assignee'] as String,
    result: j['result'] == null ? null : j['result'] as String,
    author: j['author'] == null ? null : j['author'] as String,
  );

  final TaskId taskId;

  final TaskStatus? status;

  final AgentId? assignee;

  /// 交回的结论或受阻原因。
  final String? result;

  final String? author;

  @override
  Map<String, Object?> toJson() => {
    'type': 'update_task',
    'task_id': taskId,
    'status': ?status?.toJson(),
    'assignee': ?assignee,
    'result': ?result,
    'author': ?author,
  };
}

final class RequestListApprovals extends Request {
  const RequestListApprovals({this.pendingOnly, this.limit});

  factory RequestListApprovals.fromJson(Map<String, Object?> j) => RequestListApprovals(
    pendingOnly: j['pending_only'] == null ? null : j['pending_only'] as bool,
    limit: j['limit'] == null ? null : j['limit'] as num,
  );

  final bool? pendingOnly;

  final num? limit;

  @override
  Map<String, Object?> toJson() => {'type': 'list_approvals', 'pending_only': ?pendingOnly, 'limit': ?limit};
}

final class RequestResolveApproval extends Request {
  const RequestResolveApproval({required this.approvalId, required this.choice, this.author});

  factory RequestResolveApproval.fromJson(Map<String, Object?> j) => RequestResolveApproval(
    approvalId: j['approval_id'] as String,
    choice: j['choice'] as num,
    author: j['author'] == null ? null : j['author'] as String,
  );

  final ApprovalId approvalId;

  final num choice;

  final String? author;

  @override
  Map<String, Object?> toJson() => {
    'type': 'resolve_approval',
    'approval_id': approvalId,
    'choice': choice,
    'author': ?author,
  };
}

final class RequestGetMemory extends Request {
  const RequestGetMemory({required this.roleId});

  factory RequestGetMemory.fromJson(Map<String, Object?> j) => RequestGetMemory(roleId: j['role_id'] as String);

  final RoleId roleId;

  @override
  Map<String, Object?> toJson() => {'type': 'get_memory', 'role_id': roleId};
}

final class RequestSaveMemoryText extends Request {
  const RequestSaveMemoryText({required this.roleId, required this.layer, required this.text});

  factory RequestSaveMemoryText.fromJson(Map<String, Object?> j) => RequestSaveMemoryText(
    roleId: j['role_id'] as String,
    layer: MemoryLayer.fromJson(j['layer']),
    text: j['text'] as String,
  );

  final RoleId roleId;

  final MemoryLayer layer;

  final String text;

  @override
  Map<String, Object?> toJson() => {
    'type': 'save_memory_text',
    'role_id': roleId,
    'layer': layer.toJson(),
    'text': text,
  };
}

final class RequestForgetThreadMemory extends Request {
  const RequestForgetThreadMemory({required this.roleId, required this.threadId});

  factory RequestForgetThreadMemory.fromJson(Map<String, Object?> j) =>
      RequestForgetThreadMemory(roleId: j['role_id'] as String, threadId: j['thread_id'] as String);

  final RoleId roleId;

  final ThreadId threadId;

  @override
  Map<String, Object?> toJson() => {'type': 'forget_thread_memory', 'role_id': roleId, 'thread_id': threadId};
}

final class RequestDeleteMemoryNote extends Request {
  const RequestDeleteMemoryNote({required this.roleId, required this.noteId});

  factory RequestDeleteMemoryNote.fromJson(Map<String, Object?> j) =>
      RequestDeleteMemoryNote(roleId: j['role_id'] as String, noteId: j['note_id'] as num);

  final RoleId roleId;

  final num noteId;

  @override
  Map<String, Object?> toJson() => {'type': 'delete_memory_note', 'role_id': roleId, 'note_id': noteId};
}

final class RequestClearMemory extends Request {
  const RequestClearMemory({required this.roleId});

  factory RequestClearMemory.fromJson(Map<String, Object?> j) => RequestClearMemory(roleId: j['role_id'] as String);

  final RoleId roleId;

  @override
  Map<String, Object?> toJson() => {'type': 'clear_memory', 'role_id': roleId};
}

final class RequestListMemoryOffThreads extends Request {
  const RequestListMemoryOffThreads();

  factory RequestListMemoryOffThreads.fromJson(Map<String, Object?> j) => const RequestListMemoryOffThreads();

  @override
  Map<String, Object?> toJson() => {'type': 'list_memory_off_threads'};
}

final class RequestSetThreadMemory extends Request {
  const RequestSetThreadMemory({required this.threadId, required this.enabled});

  factory RequestSetThreadMemory.fromJson(Map<String, Object?> j) =>
      RequestSetThreadMemory(threadId: j['thread_id'] as String, enabled: j['enabled'] as bool);

  final ThreadId threadId;

  final bool enabled;

  @override
  Map<String, Object?> toJson() => {'type': 'set_thread_memory', 'thread_id': threadId, 'enabled': enabled};
}

final class RequestListReports extends Request {
  const RequestListReports({this.agentId, this.limit});

  factory RequestListReports.fromJson(Map<String, Object?> j) => RequestListReports(
    agentId: j['agent_id'] == null ? null : j['agent_id'] as String,
    limit: j['limit'] == null ? null : j['limit'] as num,
  );

  final AgentId? agentId;

  final num? limit;

  @override
  Map<String, Object?> toJson() => {'type': 'list_reports', 'agent_id': ?agentId, 'limit': ?limit};
}

final class RequestReadReport extends Request {
  const RequestReadReport({required this.agentId, required this.path});

  factory RequestReadReport.fromJson(Map<String, Object?> j) =>
      RequestReadReport(agentId: j['agent_id'] as String, path: j['path'] as String);

  final AgentId agentId;

  final String path;

  @override
  Map<String, Object?> toJson() => {'type': 'read_report', 'agent_id': agentId, 'path': path};
}

final class RequestWhoAmI extends Request {
  const RequestWhoAmI();

  factory RequestWhoAmI.fromJson(Map<String, Object?> j) => const RequestWhoAmI();

  @override
  Map<String, Object?> toJson() => {'type': 'who_am_i'};
}

final class RequestListDevices extends Request {
  const RequestListDevices();

  factory RequestListDevices.fromJson(Map<String, Object?> j) => const RequestListDevices();

  @override
  Map<String, Object?> toJson() => {'type': 'list_devices'};
}

final class RequestCreatePairing extends Request {
  const RequestCreatePairing({required this.name, this.linkBase});

  factory RequestCreatePairing.fromJson(Map<String, Object?> j) => RequestCreatePairing(
    name: j['name'] as String,
    linkBase: j['link_base'] == null ? null : j['link_base'] as String,
  );

  final String name;

  final String? linkBase;

  @override
  Map<String, Object?> toJson() => {'type': 'create_pairing', 'name': name, 'link_base': ?linkBase};
}

final class RequestRenameDevice extends Request {
  const RequestRenameDevice({required this.deviceId, required this.name});

  factory RequestRenameDevice.fromJson(Map<String, Object?> j) =>
      RequestRenameDevice(deviceId: j['device_id'] as String, name: j['name'] as String);

  final String deviceId;

  final String name;

  @override
  Map<String, Object?> toJson() => {'type': 'rename_device', 'device_id': deviceId, 'name': name};
}

final class RequestSetDeviceScope extends Request {
  const RequestSetDeviceScope({required this.deviceId, required this.scope});

  factory RequestSetDeviceScope.fromJson(Map<String, Object?> j) =>
      RequestSetDeviceScope(deviceId: j['device_id'] as String, scope: Scope.fromJson(j['scope']));

  final String deviceId;

  final Scope scope;

  @override
  Map<String, Object?> toJson() => {'type': 'set_device_scope', 'device_id': deviceId, 'scope': scope.toJson()};
}

final class RequestRevokeDevice extends Request {
  const RequestRevokeDevice({required this.deviceId});

  factory RequestRevokeDevice.fromJson(Map<String, Object?> j) =>
      RequestRevokeDevice(deviceId: j['device_id'] as String);

  final String deviceId;

  @override
  Map<String, Object?> toJson() => {'type': 'revoke_device', 'device_id': deviceId};
}

final class RequestGetNotifySettings extends Request {
  const RequestGetNotifySettings();

  factory RequestGetNotifySettings.fromJson(Map<String, Object?> j) => const RequestGetNotifySettings();

  @override
  Map<String, Object?> toJson() => {'type': 'get_notify_settings'};
}

final class RequestSetNotifySettings extends Request {
  const RequestSetNotifySettings({required this.settings});

  factory RequestSetNotifySettings.fromJson(Map<String, Object?> j) =>
      RequestSetNotifySettings(settings: NotifySettings.fromJson(j['settings'] as Map<String, Object?>));

  final NotifySettings settings;

  @override
  Map<String, Object?> toJson() => {'type': 'set_notify_settings', 'settings': settings.toJson()};
}

final class RequestPushKey extends Request {
  const RequestPushKey();

  factory RequestPushKey.fromJson(Map<String, Object?> j) => const RequestPushKey();

  @override
  Map<String, Object?> toJson() => {'type': 'push_key'};
}

final class RequestSubscribePush extends Request {
  const RequestSubscribePush({required this.endpoint, required this.p256dh, required this.auth});

  factory RequestSubscribePush.fromJson(Map<String, Object?> j) =>
      RequestSubscribePush(endpoint: j['endpoint'] as String, p256dh: j['p256dh'] as String, auth: j['auth'] as String);

  final String endpoint;

  final String p256dh;

  final String auth;

  @override
  Map<String, Object?> toJson() => {'type': 'subscribe_push', 'endpoint': endpoint, 'p256dh': p256dh, 'auth': auth};
}

final class RequestUnsubscribePush extends Request {
  const RequestUnsubscribePush({required this.endpoint});

  factory RequestUnsubscribePush.fromJson(Map<String, Object?> j) =>
      RequestUnsubscribePush(endpoint: j['endpoint'] as String);

  final String endpoint;

  @override
  Map<String, Object?> toJson() => {'type': 'unsubscribe_push', 'endpoint': endpoint};
}

final class RequestPresence extends Request {
  const RequestPresence({required this.active});

  factory RequestPresence.fromJson(Map<String, Object?> j) => RequestPresence(active: j['active'] as bool);

  final bool active;

  @override
  Map<String, Object?> toJson() => {'type': 'presence', 'active': active};
}

final class RequestSend extends Request {
  const RequestSend({this.agentId, this.threadId, required this.text, this.author});

  factory RequestSend.fromJson(Map<String, Object?> j) => RequestSend(
    agentId: j['agent_id'] == null ? null : j['agent_id'] as String,
    threadId: j['thread_id'] == null ? null : j['thread_id'] as String,
    text: j['text'] as String,
    author: j['author'] == null ? null : j['author'] as String,
  );

  /// 省略时由路由规则决定。
  final AgentId? agentId;

  /// 省略时开一个新对话。
  final ThreadId? threadId;

  final String text;

  /// 发消息的人；登录体系上线前由客户端给出。
  final String? author;

  @override
  Map<String, Object?> toJson() => {
    'type': 'send',
    'agent_id': ?agentId,
    'thread_id': ?threadId,
    'text': text,
    'author': ?author,
  };
}

/// 这个版本还不认识的Request（主机比 App 新），原样留着。
final class RequestUnknown extends Request {
  const RequestUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// 一个岗位。
final class Role {
  const Role({
    required this.id,
    this.name,
    required this.description,
    required this.model,
    this.budgetPerDayUsd,
    this.coordinator,
  });

  factory Role.fromJson(Map<String, Object?> j) => Role(
    id: j['id'] as String,
    name: j['name'] == null ? null : j['name'] as String,
    description: j['description'] as String,
    model: ModelSlot.fromJson(j['model'] as Map<String, Object?>),
    budgetPerDayUsd: j['budget_per_day_usd'] == null ? null : j['budget_per_day_usd'] as num,
    coordinator: j['coordinator'] == null ? null : j['coordinator'] as bool,
  );

  final RoleId id;

  /// 显示名；省略时用 id。
  final String? name;

  /// 职责说明，也是语义路由比对的文本。
  final String description;

  final ModelSlot model;

  /// 每日模型费用上限（美元）。
  final num? budgetPerDayUsd;

  /// 协调层岗位：接没有明确归属的需求，拆成任务派给执行岗位（App 星图中心的太阳）。最多一个岗位标这一项。
  /// 只是标记，不改这个岗位的工具和提示词。是 false 时不出现在协议里，旧主机也一样没有。
  final bool? coordinator;

  Map<String, Object?> toJson() => {
    'id': id,
    'name': name,
    'description': description,
    'model': model.toJson(),
    'budget_per_day_usd': budgetPerDayUsd,
    'coordinator': ?coordinator,
  };
}

/// 岗位 ID，通常是人写的短名，如 `finance-ap`。
typedef RoleId = String;

/// 是怎么分派的。
enum RouteBy {
  rule('rule'),
  semantic('semantic'),
  fallback('fallback'),

  /// 这个版本还不认识的值（主机比 App 新）。
  unknown('');

  const RouteBy(this.wire);

  /// JSON 里的写法。
  final String wire;

  static RouteBy fromJson(Object? v) => values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);

  String toJson() => wire;
}

final class RouterSpec {
  const RouterSpec({required this.rules, required this.fallback, this.semantic});

  factory RouterSpec.fromJson(Map<String, Object?> j) => RouterSpec(
    rules: [for (final x in j['rules'] as List<Object?>) Rule.fromJson(x as Map<String, Object?>)],
    fallback: Fallback.fromJson(j['fallback']),
    semantic: j['semantic'] == null ? null : SemanticSpec.fromJson(j['semantic'] as Map<String, Object?>),
  );

  final List<Rule> rules;

  final Fallback fallback;

  /// 规则都没命中时用模型做语义匹配；省略表示不用。
  final SemanticSpec? semantic;

  Map<String, Object?> toJson() => {
    'rules': [for (final x in rules) x.toJson()],
    'fallback': fallback.toJson(),
    'semantic': ?semantic?.toJson(),
  };
}

/// 一条路由规则：满足 `when` 就交给 `to`。
final class Rule {
  const Rule({required this.id, required this.description, required this.when, required this.to});

  factory Rule.fromJson(Map<String, Object?> j) => Rule(
    id: j['id'] as String,
    description: j['description'] as String,
    when: Condition.fromJson(j['when'] as Map<String, Object?>),
    to: Target.fromJson(j['to'] as Map<String, Object?>),
  );

  final String id;

  /// 写给人看的说明。
  final String description;

  final Condition when;

  final Target to;

  Map<String, Object?> toJson() => {'id': id, 'description': description, 'when': when.toJson(), 'to': to.toJson()};
}

/// 一条待人审核的规则草案。
final class RuleDraft {
  const RuleDraft({required this.id, required this.rule, required this.source, required this.createdAtMs});

  factory RuleDraft.fromJson(Map<String, Object?> j) => RuleDraft(
    id: j['id'] as String,
    rule: Rule.fromJson(j['rule'] as Map<String, Object?>),
    source: j['source'] as String,
    createdAtMs: j['created_at_ms'] as num,
  );

  final String id;

  final Rule rule;

  /// 草案从哪来，例如“李四 把「服务器又挂了」交给了 运维测试”。
  final String source;

  final num createdAtMs;

  Map<String, Object?> toJson() => {'id': id, 'rule': rule.toJson(), 'source': source, 'created_at_ms': createdAtMs};
}

/// `list_rules` 的回复。
final class RulesView {
  const RulesView({required this.spec, required this.fromConsole});

  factory RulesView.fromJson(Map<String, Object?> j) =>
      RulesView(spec: RouterSpec.fromJson(j['spec'] as Map<String, Object?>), fromConsole: j['from_console'] as bool);

  final RouterSpec spec;

  /// 规则来自控制台保存（true）还是 ip.yaml（false）。
  final bool fromConsole;

  Map<String, Object?> toJson() => {'spec': spec.toJson(), 'from_console': fromConsole};
}

/// 广播给订阅者的事件。按 `type` 字段区分，直接序列化给前端。
sealed class RuntimeEvent {
  const RuntimeEvent();

  factory RuntimeEvent.fromJson(Map<String, Object?> j) {
    return switch (j['type']) {
      'turn_started' => RuntimeEventTurnStarted.fromJson(j),
      'delta' => RuntimeEventDelta.fromJson(j),
      'message' => RuntimeEventMessage.fromJson(j),
      'turn_finished' => RuntimeEventTurnFinished.fromJson(j),
      'turn_failed' => RuntimeEventTurnFailed.fromJson(j),
      'task_changed' => RuntimeEventTaskChanged.fromJson(j),
      'approval_changed' => RuntimeEventApprovalChanged.fromJson(j),
      _ => RuntimeEventUnknown(j),
    };
  }

  Object? toJson();
}

final class RuntimeEventTurnStarted extends RuntimeEvent {
  const RuntimeEventTurnStarted({required this.agentId, required this.threadId});

  factory RuntimeEventTurnStarted.fromJson(Map<String, Object?> j) =>
      RuntimeEventTurnStarted(agentId: j['agent_id'] as String, threadId: j['thread_id'] as String);

  final AgentId agentId;

  final ThreadId threadId;

  @override
  Map<String, Object?> toJson() => {'type': 'turn_started', 'agent_id': agentId, 'thread_id': threadId};
}

final class RuntimeEventDelta extends RuntimeEvent {
  const RuntimeEventDelta({required this.agentId, required this.threadId, required this.text});

  factory RuntimeEventDelta.fromJson(Map<String, Object?> j) => RuntimeEventDelta(
    agentId: j['agent_id'] as String,
    threadId: j['thread_id'] as String,
    text: j['text'] as String,
  );

  final AgentId agentId;

  final ThreadId threadId;

  final String text;

  @override
  Map<String, Object?> toJson() => {'type': 'delta', 'agent_id': agentId, 'thread_id': threadId, 'text': text};
}

final class RuntimeEventMessage extends RuntimeEvent {
  const RuntimeEventMessage({required this.message});

  factory RuntimeEventMessage.fromJson(Map<String, Object?> j) =>
      RuntimeEventMessage(message: Message.fromJson(j['message'] as Map<String, Object?>));

  final Message message;

  @override
  Map<String, Object?> toJson() => {'type': 'message', 'message': message.toJson()};
}

final class RuntimeEventTurnFinished extends RuntimeEvent {
  const RuntimeEventTurnFinished({
    required this.agentId,
    required this.threadId,
    required this.steps,
    required this.usage,
    required this.costUsd,
    required this.elapsedMs,
  });

  factory RuntimeEventTurnFinished.fromJson(Map<String, Object?> j) => RuntimeEventTurnFinished(
    agentId: j['agent_id'] as String,
    threadId: j['thread_id'] as String,
    steps: j['steps'] as num,
    usage: Usage.fromJson(j['usage'] as Map<String, Object?>),
    costUsd: j['cost_usd'] as num,
    elapsedMs: j['elapsed_ms'] as num,
  );

  final AgentId agentId;

  final ThreadId threadId;

  final num steps;

  final Usage usage;

  /// 这一轮按配置价格折算的美元费用；模型没配价格时为 0。
  final num costUsd;

  /// 从开始处理到结束，不含在收件箱里排队的时间。
  final num elapsedMs;

  @override
  Map<String, Object?> toJson() => {
    'type': 'turn_finished',
    'agent_id': agentId,
    'thread_id': threadId,
    'steps': steps,
    'usage': usage.toJson(),
    'cost_usd': costUsd,
    'elapsed_ms': elapsedMs,
  };
}

final class RuntimeEventTurnFailed extends RuntimeEvent {
  const RuntimeEventTurnFailed({required this.agentId, required this.threadId, required this.error});

  factory RuntimeEventTurnFailed.fromJson(Map<String, Object?> j) => RuntimeEventTurnFailed(
    agentId: j['agent_id'] as String,
    threadId: j['thread_id'] as String,
    error: j['error'] as String,
  );

  final AgentId agentId;

  final ThreadId threadId;

  final String error;

  @override
  Map<String, Object?> toJson() => {'type': 'turn_failed', 'agent_id': agentId, 'thread_id': threadId, 'error': error};
}

final class RuntimeEventTaskChanged extends RuntimeEvent {
  const RuntimeEventTaskChanged({required this.task});

  factory RuntimeEventTaskChanged.fromJson(Map<String, Object?> j) =>
      RuntimeEventTaskChanged(task: Task.fromJson(j['task'] as Map<String, Object?>));

  final Task task;

  @override
  Map<String, Object?> toJson() => {'type': 'task_changed', 'task': task.toJson()};
}

final class RuntimeEventApprovalChanged extends RuntimeEvent {
  const RuntimeEventApprovalChanged({required this.approval});

  factory RuntimeEventApprovalChanged.fromJson(Map<String, Object?> j) =>
      RuntimeEventApprovalChanged(approval: Approval.fromJson(j['approval'] as Map<String, Object?>));

  final Approval approval;

  @override
  Map<String, Object?> toJson() => {'type': 'approval_changed', 'approval': approval.toJson()};
}

/// 这个版本还不认识的RuntimeEvent（主机比 App 新），原样留着。
final class RuntimeEventUnknown extends RuntimeEvent {
  const RuntimeEventUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// 一条定时任务的配置。
final class ScheduleSpec {
  const ScheduleSpec({
    required this.id,
    required this.description,
    required this.cron,
    required this.utcOffset,
    this.to,
    required this.text,
    required this.enabled,
  });

  factory ScheduleSpec.fromJson(Map<String, Object?> j) => ScheduleSpec(
    id: j['id'] as String,
    description: j['description'] as String,
    cron: j['cron'] as String,
    utcOffset: j['utc_offset'] as String,
    to: j['to'] == null ? null : Target.fromJson(j['to'] as Map<String, Object?>),
    text: j['text'] as String,
    enabled: j['enabled'] as bool,
  );

  final String id;

  final String description;

  /// 五段 cron：分 时 日 月 周（0 和 7 都是周日）。每段可写 `*`、`a`、`a-b`、逗号列表，
  /// 后面可加 `/n` 表示步长（如每 15 分钟：分钟段写星号加 `/15`）。
  final String cron;

  /// 按哪个时区解读 cron，写成相对 UTC 的偏移，如 `+08:00`。默认 `+00:00`。
  final String utcOffset;

  /// 发给哪个岗位（可指定员工、可要求升级模型）；省略时按路由规则分派。
  final Target? to;

  /// 到点发出的消息。
  final String text;

  final bool enabled;

  Map<String, Object?> toJson() => {
    'id': id,
    'description': description,
    'cron': cron,
    'utc_offset': utcOffset,
    'to': ?to?.toJson(),
    'text': text,
    'enabled': enabled,
  };
}

/// 控制台看到的一条定时任务。
final class ScheduleView {
  const ScheduleView({required this.spec, this.nextRunMs, this.lastRun});

  factory ScheduleView.fromJson(Map<String, Object?> j) => ScheduleView(
    spec: ScheduleSpec.fromJson(j['spec'] as Map<String, Object?>),
    nextRunMs: j['next_run_ms'] == null ? null : j['next_run_ms'] as num,
    lastRun: j['last_run'] == null ? null : LastRun.fromJson(j['last_run'] as Map<String, Object?>),
  );

  final ScheduleSpec spec;

  /// 下次运行的时间；停用或 cron 永远不会命中时为空。
  final num? nextRunMs;

  final LastRun? lastRun;

  Map<String, Object?> toJson() => {'spec': spec.toJson(), 'next_run_ms': nextRunMs, 'last_run': lastRun?.toJson()};
}

/// 设备权限（`docs/ip/apps-plan.md` 4.3）。哪些请求算“日常”见 `ws.rs` 的 `daily_allows`。
enum Scope {
  daily('daily'),
  admin('admin'),

  /// 这个版本还不认识的值（主机比 App 新）。
  unknown('');

  const Scope(this.wire);

  /// JSON 里的写法。
  final String wire;

  static Scope fromJson(Object? v) => values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);

  String toJson() => wire;
}

/// 一条搜索结果。
final class SearchHit {
  const SearchHit({required this.messageId, required this.threadId, required this.atMs, required this.text});

  factory SearchHit.fromJson(Map<String, Object?> j) => SearchHit(
    messageId: j['message_id'] as String,
    threadId: j['thread_id'] as String,
    atMs: j['at_ms'] as num,
    text: j['text'] as String,
  );

  final MessageId messageId;

  final ThreadId threadId;

  final num atMs;

  final String text;

  Map<String, Object?> toJson() => {'message_id': messageId, 'thread_id': threadId, 'at_ms': atMs, 'text': text};
}

/// `router.semantic` 配置。
final class SemanticSpec {
  const SemanticSpec({required this.model, required this.minConfidence});

  factory SemanticSpec.fromJson(Map<String, Object?> j) =>
      SemanticSpec(model: j['model'] as String, minConfidence: j['min_confidence'] as num);

  /// 用来判断的模型，写法同岗位模型：`<端点>/<模型>`。
  final String model;

  /// 把握不低于这个值才直接分派，否则交给人并给出推荐；默认 0.7。
  final num minConfidence;

  Map<String, Object?> toJson() => {'model': model, 'min_confidence': minConfidence};
}

/// `send` 的回复。
final class SendResult {
  const SendResult({required this.message, this.agentId, this.decision, this.approvalId});

  factory SendResult.fromJson(Map<String, Object?> j) => SendResult(
    message: Message.fromJson(j['message'] as Map<String, Object?>),
    agentId: j['agent_id'] == null ? null : j['agent_id'] as String,
    decision: j['decision'] == null ? null : Decision.fromJson(j['decision'] as Map<String, Object?>),
    approvalId: j['approval_id'] == null ? null : j['approval_id'] as String,
  );

  final Message message;

  /// 实际接手的员工；路由拿不准、等人决定时为空。
  final AgentId? agentId;

  /// 由路由决定时附上路由结果和理由。
  final Decision? decision;

  /// 路由拿不准时新建的审批卡片。
  final ApprovalId? approvalId;

  Map<String, Object?> toJson() => {
    'message': message.toJson(),
    'agent_id': agentId,
    'decision': decision?.toJson(),
    'approval_id': approvalId,
  };
}

/// 服务发给客户端的一帧。
sealed class ServerFrame {
  const ServerFrame();

  factory ServerFrame.fromJson(Map<String, Object?> j) {
    return switch (j['type']) {
      'hello' => ServerFrameHello.fromJson(j),
      'reply' => ServerFrameReply.fromJson(j),
      'event' => ServerFrameEvent.fromJson(j),
      'lagged' => ServerFrameLagged.fromJson(j),
      'alert' => ServerFrameAlert.fromJson(j),
      'dismiss' => ServerFrameDismiss.fromJson(j),
      _ => ServerFrameUnknown(j),
    };
  }

  Object? toJson();
}

final class ServerFrameHello extends ServerFrame {
  const ServerFrameHello({required this.protocol, required this.epoch, required this.seq, required this.resumed});

  factory ServerFrameHello.fromJson(Map<String, Object?> j) => ServerFrameHello(
    protocol: j['protocol'] as num,
    epoch: j['epoch'] as String,
    seq: j['seq'] as num,
    resumed: j['resumed'] as bool,
  );

  /// 协议版本号，只有不兼容的改动才加一（服务端 `events.rs` 的 `PROTOCOL_VERSION`，控制台 `client.js` 的 `PROTOCOL`）。
  final num protocol;

  /// 本次启动的标识。序号每次启动重新编号，重连补发时要和它一起用。
  final String epoch;

  /// 从哪个序号接着推：补发时是客户端给的 `since`，否则是当前最新的序号。
  final num seq;

  /// 是否补发了断开期间的事件。为 false 时客户端要重新拉列表。
  final bool resumed;

  @override
  Map<String, Object?> toJson() => {
    'type': 'hello',
    'protocol': protocol,
    'epoch': epoch,
    'seq': seq,
    'resumed': resumed,
  };
}

final class ServerFrameReply extends ServerFrame {
  const ServerFrameReply({this.id, required this.ok, this.data, this.error});

  factory ServerFrameReply.fromJson(Map<String, Object?> j) => ServerFrameReply(
    id: j['id'],
    ok: j['ok'] as bool,
    data: j['data'],
    error: j['error'] == null ? null : j['error'] as String,
  );

  final Object? id;

  final bool ok;

  final Object? data;

  final String? error;

  @override
  Map<String, Object?> toJson() => {'type': 'reply', 'id': id, 'ok': ok, 'data': ?data, 'error': ?error};
}

final class ServerFrameEvent extends ServerFrame {
  const ServerFrameEvent({required this.seq, required this.event});

  factory ServerFrameEvent.fromJson(Map<String, Object?> j) =>
      ServerFrameEvent(seq: j['seq'] as num, event: RuntimeEvent.fromJson(j['event'] as Map<String, Object?>));

  /// 事件序号，从 1 起、连续，每次启动重新编号。
  final num seq;

  final RuntimeEvent event;

  @override
  Map<String, Object?> toJson() => {'type': 'event', 'seq': seq, 'event': event.toJson()};
}

final class ServerFrameLagged extends ServerFrame {
  const ServerFrameLagged({required this.skipped});

  factory ServerFrameLagged.fromJson(Map<String, Object?> j) => ServerFrameLagged(skipped: j['skipped'] as num);

  final num skipped;

  @override
  Map<String, Object?> toJson() => {'type': 'lagged', 'skipped': skipped};
}

final class ServerFrameAlert extends ServerFrame {
  const ServerFrameAlert({required this.alert});

  factory ServerFrameAlert.fromJson(Map<String, Object?> j) =>
      ServerFrameAlert(alert: Alert.fromJson(j['alert'] as Map<String, Object?>));

  final Alert alert;

  @override
  Map<String, Object?> toJson() => {'type': 'alert', 'alert': alert.toJson()};
}

final class ServerFrameDismiss extends ServerFrame {
  const ServerFrameDismiss({required this.tag});

  factory ServerFrameDismiss.fromJson(Map<String, Object?> j) => ServerFrameDismiss(tag: j['tag'] as String);

  final String tag;

  @override
  Map<String, Object?> toJson() => {'type': 'dismiss', 'tag': tag};
}

/// 这个版本还不认识的ServerFrame（主机比 App 新），原样留着。
final class ServerFrameUnknown extends ServerFrame {
  const ServerFrameUnknown(this.json);

  final Object? json;

  @override
  Object? toJson() => json;
}

/// 今天（UTC 自然日）的模型费用。
final class SpendToday {
  const SpendToday({
    required this.date,
    required this.roles,
    required this.routingUsd,
    required this.tokens,
    required this.routingTokens,
  });

  factory SpendToday.fromJson(Map<String, Object?> j) => SpendToday(
    date: j['date'] as String,
    roles: {for (final x in (j['roles'] as Map<String, Object?>).entries) x.key: x.value as num},
    routingUsd: j['routing_usd'] as num,
    tokens: {
      for (final x in (j['tokens'] as Map<String, Object?>).entries)
        x.key: Usage.fromJson(x.value as Map<String, Object?>),
    },
    routingTokens: Usage.fromJson(j['routing_tokens'] as Map<String, Object?>),
  );

  /// `YYYY-MM-DD`。
  final String date;

  /// 岗位 id → 美元。今天还没花钱的岗位不列出。
  final Map<String, num> roles;

  /// 语义分派调用模型花的美元，不占岗位预算。
  final num routingUsd;

  /// 岗位 id → 今天的 token 用量，每轮结束时累计。提示词缓存命中率 = `cached_tokens / prompt_tokens`
  /// （`prompt_tokens` 已包含命中缓存的部分）。今天还没调过模型的岗位不列出。
  final Map<String, Usage> tokens;

  /// 语义分派的 token 用量。
  final Usage routingTokens;

  Map<String, Object?> toJson() => {
    'date': date,
    'roles': roles,
    'routing_usd': routingUsd,
    'tokens': {for (final x in tokens.entries) x.key: x.value.toJson()},
    'routing_tokens': routingTokens.toJson(),
  };
}

final class Target {
  const Target({required this.role, this.agent, required this.escalate});

  factory Target.fromJson(Map<String, Object?> j) => Target(
    role: j['role'] as String,
    agent: j['agent'] == null ? null : j['agent'] as String,
    escalate: j['escalate'] as bool,
  );

  final RoleId role;

  /// 指定某个员工；省略时在岗位里挑当前最空闲的。
  final AgentId? agent;

  /// 用岗位配置里的升级模型（`model.escalate`）处理。
  final bool escalate;

  Map<String, Object?> toJson() => {'role': role, 'agent': ?agent, 'escalate': escalate};
}

final class Task {
  const Task({
    required this.id,
    required this.title,
    required this.description,
    required this.status,
    this.role,
    this.assignee,
    this.threadId,
    this.originThreadId,
    this.result,
    required this.depth,
    required this.createdBy,
    required this.createdAtMs,
    required this.updatedAtMs,
    this.dueMs,
  });

  factory Task.fromJson(Map<String, Object?> j) => Task(
    id: j['id'] as String,
    title: j['title'] as String,
    description: j['description'] as String,
    status: TaskStatus.fromJson(j['status']),
    role: j['role'] == null ? null : j['role'] as String,
    assignee: j['assignee'] == null ? null : j['assignee'] as String,
    threadId: j['thread_id'] == null ? null : j['thread_id'] as String,
    originThreadId: j['origin_thread_id'] == null ? null : j['origin_thread_id'] as String,
    result: j['result'] == null ? null : j['result'] as String,
    depth: j['depth'] as num,
    createdBy: Author.fromJson(j['created_by'] as Map<String, Object?>),
    createdAtMs: j['created_at_ms'] as num,
    updatedAtMs: j['updated_at_ms'] as num,
    dueMs: j['due_ms'] == null ? null : j['due_ms'] as num,
  );

  final TaskId id;

  final String title;

  final String description;

  final TaskStatus status;

  /// 负责的岗位；具体由谁做看 `assignee`。
  final RoleId? role;

  final AgentId? assignee;

  /// 负责人做这件事的对话。分派时运行时把任务说明写进去并通知负责人。
  final ThreadId? threadId;

  /// 派活的对话。任务完成、受阻或取消时，结果写回这里，派活的员工被叫回来接着做。
  final ThreadId? originThreadId;

  /// 负责人交回的结果或受阻原因。
  final String? result;

  /// 委派层级：人或顶层对话派的是 1，员工在执行对话里再往下派加 1。用来挡住无限转派。
  final num depth;

  final Author createdBy;

  final num createdAtMs;

  final num updatedAtMs;

  final num? dueMs;

  Map<String, Object?> toJson() => {
    'id': id,
    'title': title,
    'description': description,
    'status': status.toJson(),
    'role': role,
    'assignee': assignee,
    'thread_id': threadId,
    'origin_thread_id': originThreadId,
    'result': result,
    'depth': depth,
    'created_by': createdBy.toJson(),
    'created_at_ms': createdAtMs,
    'updated_at_ms': updatedAtMs,
    'due_ms': dueMs,
  };
}

/// 任务 ID。
typedef TaskId = String;

enum TaskStatus {
  open('open'),
  inProgress('in_progress'),
  blocked('blocked'),
  done('done'),
  cancelled('cancelled'),

  /// 这个版本还不认识的值（主机比 App 新）。
  unknown('');

  const TaskStatus(this.wire);

  /// JSON 里的写法。
  final String wire;

  static TaskStatus fromJson(Object? v) => values.firstWhere((e) => e != unknown && e.wire == v, orElse: () => unknown);

  String toJson() => wire;
}

/// 对话 ID。
typedef ThreadId = String;

final class ThreadMemoryView {
  const ThreadMemoryView({
    required this.threadId,
    this.title,
    required this.summary,
    required this.generatedMs,
    required this.usageCount,
    this.lastUsageMs,
  });

  factory ThreadMemoryView.fromJson(Map<String, Object?> j) => ThreadMemoryView(
    threadId: j['thread_id'] as String,
    title: j['title'] == null ? null : j['title'] as String,
    summary: j['summary'] as String,
    generatedMs: j['generated_ms'] as num,
    usageCount: j['usage_count'] as num,
    lastUsageMs: j['last_usage_ms'] == null ? null : j['last_usage_ms'] as num,
  );

  final ThreadId threadId;

  final String? title;

  final String summary;

  final num generatedMs;

  /// 被员工读过几次。
  final num usageCount;

  final num? lastUsageMs;

  Map<String, Object?> toJson() => {
    'thread_id': threadId,
    'title': title,
    'summary': summary,
    'generated_ms': generatedMs,
    'usage_count': usageCount,
    'last_usage_ms': lastUsageMs,
  };
}

/// 对话列表的一项。
final class ThreadSummary {
  const ThreadSummary({
    required this.threadId,
    required this.lastAtMs,
    required this.messageCount,
    this.preview,
    this.lastAgentId,
  });

  factory ThreadSummary.fromJson(Map<String, Object?> j) => ThreadSummary(
    threadId: j['thread_id'] as String,
    lastAtMs: j['last_at_ms'] as num,
    messageCount: j['message_count'] as num,
    preview: j['preview'] == null ? null : j['preview'] as String,
    lastAgentId: j['last_agent_id'] == null ? null : j['last_agent_id'] as String,
  );

  final ThreadId threadId;

  final num lastAtMs;

  final num messageCount;

  /// 最后一条文字消息的开头。
  final String? preview;

  /// 最后回复的员工；还没有员工说过话（比如刚发出、员工还没回）时为空。
  final AgentId? lastAgentId;

  Map<String, Object?> toJson() => {
    'thread_id': threadId,
    'last_at_ms': lastAtMs,
    'message_count': messageCount,
    'preview': preview,
    'last_agent_id': lastAgentId,
  };
}

final class Usage {
  const Usage({
    required this.promptTokens,
    required this.completionTokens,
    required this.cachedTokens,
    required this.cacheWriteTokens,
  });

  factory Usage.fromJson(Map<String, Object?> j) => Usage(
    promptTokens: j['prompt_tokens'] as num,
    completionTokens: j['completion_tokens'] as num,
    cachedTokens: j['cached_tokens'] as num,
    cacheWriteTokens: j['cache_write_tokens'] as num,
  );

  final num promptTokens;

  final num completionTokens;

  /// 命中提示前缀缓存的 token 数（端点支持时）。
  final num cachedTokens;

  /// 这次写进提示前缀缓存的 token 数（端点报告时，如 OpenAI 的 `cache_write_tokens`）。
  /// OpenAI 按输入价的 1.25 倍另收，见 [`crate::Price::cache_write`]。旧数据里没有这一项，读出来是 0。
  final num cacheWriteTokens;

  Map<String, Object?> toJson() => {
    'prompt_tokens': promptTokens,
    'completion_tokens': completionTokens,
    'cached_tokens': cachedTokens,
    'cache_write_tokens': cacheWriteTokens,
  };
}

/// `push_key` 的回复：浏览器订阅时要用的主机公钥（VAPID）。
final class VapidKey {
  const VapidKey({required this.publicKey});

  factory VapidKey.fromJson(Map<String, Object?> j) => VapidKey(publicKey: j['public_key'] as String);

  /// 未压缩的 P-256 公钥，base64url。
  final String publicKey;

  Map<String, Object?> toJson() => {'public_key': publicKey};
}

/// 外部渠道里的一个用户（`docs/ip/m2-spec.md` D-27）。
final class Via {
  const Via({required this.channel, required this.userId});

  factory Via.fromJson(Map<String, Object?> j) => Via(channel: j['channel'] as String, userId: j['user_id'] as String);

  /// `ip.yaml` 里的渠道名，如 `slack`。
  final String channel;

  /// 渠道里稳定不变的用户 id，如 Slack 的 `U012ABCDEF`。
  final String userId;

  Map<String, Object?> toJson() => {'channel': channel, 'user_id': userId};
}

/// `who_am_i` 的回复。
final class WhoAmI {
  const WhoAmI({this.device, required this.scope});

  factory WhoAmI.fromJson(Map<String, Object?> j) => WhoAmI(
    device: j['device'] == null ? null : DeviceInfo.fromJson(j['device'] as Map<String, Object?>),
    scope: Scope.fromJson(j['scope']),
  );

  /// 用设备令牌连上时是这台设备；共用令牌时为空。
  final DeviceInfo? device;

  /// 这个连接的权限。共用令牌（或本机不设令牌）是“管理”。
  final Scope scope;

  Map<String, Object?> toJson() => {'device': device?.toJson(), 'scope': scope.toJson()};
}
