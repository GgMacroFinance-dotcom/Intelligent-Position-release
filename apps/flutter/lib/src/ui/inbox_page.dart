// “待办”页（apps-plan 第 6 节，手机首页）：员工提问、等你决定的审批卡片、受阻的任务（能直接改状态），
// 以及最近 7 天完成的任务和新报告（backlog F-14，和控制台的 F-5 一样）。

import 'package:flutter/material.dart';

import '../app_state.dart';
import '../protocol/client.dart';
import '../protocol/models.dart';
import 'common.dart';
import 'report.dart';

/// “最近”是几天内，每类最多列几条。
const _recent = Duration(days: 7);
const _recentMax = 10;

/// 任务状态的中文名，和控制台一样。
const taskStatusText = {'open': '待开始', 'in_progress': '进行中', 'blocked': '受阻', 'done': '已完成', 'cancelled': '已取消'};

class InboxPage extends StatelessWidget {
  const InboxPage({super.key, required this.onOpenThread});

  /// 点“查看对话”时去对话页打开它。
  final void Function(String threadId) onOpenThread;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final pending = state.pendingApprovals;
    final questions = pending.where((a) => a.isAgentQuestion).toList();
    final decisions = pending.where((a) => !a.isAgentQuestion).toList();
    final blocked = state.blockedTasks;
    final since = DateTime.now().subtract(_recent).millisecondsSinceEpoch;
    final done = state.tasks.where((t) => t.status == 'done' && t.updatedAtMs >= since).take(_recentMax).toList();
    final reports = state.reports.where((r) => r.modifiedMs >= since).take(_recentMax).toList();
    if (!state.connected && pending.isEmpty && blocked.isEmpty && done.isEmpty && reports.isEmpty) {
      return const EmptyHint('连上主机后，等你处理的事会显示在这里。');
    }
    final theme = Theme.of(context);
    Widget heading(String text) => Padding(
      padding: const EdgeInsets.fromLTRB(4, 16, 4, 8),
      child: Text(text, style: theme.textTheme.titleMedium),
    );
    final muted = TextStyle(color: theme.hintColor);
    return ListView(
      padding: const EdgeInsets.fromLTRB(12, 0, 12, 24),
      children: [
        if (pending.isEmpty && blocked.isEmpty)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 20, 4, 4),
            child: Text('没有等你处理的事。', style: muted),
          ),
        if (questions.isNotEmpty) heading('员工在问你（${questions.length}）'),
        for (final a in questions) ApprovalCard(approval: a, onOpenThread: onOpenThread),
        if (decisions.isNotEmpty) heading('等你决定（${decisions.length}）'),
        for (final a in decisions) ApprovalCard(approval: a, onOpenThread: onOpenThread),
        if (blocked.isNotEmpty) heading('受阻的任务（${blocked.length}）'),
        for (final t in blocked) TaskCard(key: ValueKey(t.id), task: t, onOpenThread: onOpenThread),
        heading('最近 7 天'),
        Card(
          margin: const EdgeInsets.only(bottom: 12),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              ListTile(title: Text('完成的任务', style: theme.textTheme.titleSmall)),
              if (done.isEmpty) ListTile(dense: true, title: Text('没有。', style: muted)),
              for (final t in done) _DoneTask(task: t, state: state, onOpenThread: onOpenThread),
            ],
          ),
        ),
        Card(
          margin: const EdgeInsets.only(bottom: 12),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              ListTile(title: Text('新报告', style: theme.textTheme.titleSmall)),
              if (reports.isEmpty) ListTile(dense: true, title: Text('没有。', style: muted)),
              for (final r in reports)
                ListTile(
                  leading: const Icon(Icons.description_outlined),
                  title: Text(r.fileName),
                  subtitle: Text('${state.agentName(r.agentId)} · ${formatTime(r.modifiedMs)}', style: muted),
                  onTap: () => openReport(context, r.path, hint: r.agentId),
                ),
            ],
          ),
        ),
      ],
    );
  }
}

/// 谁负责、谁派的、什么时候更新的。
String _taskMeta(AppState state, Task t) {
  final who = t.assignee != null ? state.agentName(t.assignee!) : (t.role ?? '—');
  final by = t.createdByName ?? (t.createdByAgent != null ? state.agentName(t.createdByAgent!) : '系统');
  return '$who · 由 $by 派 · ${formatTime(t.updatedAtMs)}';
}

/// 执行对话、派活对话的跳转按钮。
List<Widget> _threadLinks(Task t, void Function(String threadId) onOpenThread) => [
  for (final (id, label) in [(t.threadId, '执行对话'), (t.originThreadId, '派活对话')])
    if (id != null)
      TextButton(
        style: TextButton.styleFrom(
          visualDensity: VisualDensity.compact,
          padding: const EdgeInsets.symmetric(horizontal: 8),
        ),
        onPressed: () => onOpenThread(id),
        child: Text(label),
      ),
];

/// 一个任务：标题、说明、谁负责、受阻原因或结果，状态能直接改（受阻的任务修好了就改回进行中，或者取消）。
class TaskCard extends StatefulWidget {
  const TaskCard({super.key, required this.task, required this.onOpenThread});

  final Task task;
  final void Function(String threadId) onOpenThread;

  @override
  State<TaskCard> createState() => _TaskCardState();
}

class _TaskCardState extends State<TaskCard> {
  bool _saving = false;

  Future<void> _setStatus(String status) async {
    if (status == widget.task.status) return;
    setState(() => _saving = true);
    try {
      await AppScope.read(context).updateTask(widget.task, status);
    } on RequestError catch (e) {
      if (mounted) showError(context, '改状态失败：$e');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final t = widget.task;
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final muted = TextStyle(color: theme.hintColor);
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 12, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(t.title, style: theme.textTheme.titleMedium),
            if (t.description.isNotEmpty) ...[const SizedBox(height: 2), Text(t.description, style: muted)],
            const SizedBox(height: 4),
            Text(_taskMeta(state, t), style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor)),
            if (t.result case final r? when r.isNotEmpty) ...[
              const SizedBox(height: 8),
              Text(t.status == 'blocked' ? '受阻原因' : '结果', style: theme.textTheme.labelMedium),
              MessageMarkdown(r, reportHint: t.assignee),
            ],
            const SizedBox(height: 4),
            Wrap(
              crossAxisAlignment: WrapCrossAlignment.center,
              spacing: 4,
              children: [
                Text('状态', style: muted),
                DropdownButton<String>(
                  value: t.status,
                  isDense: true,
                  onChanged: _saving || !state.connected ? null : (v) => v == null ? null : _setStatus(v),
                  items: [
                    for (final MapEntry(key: v, value: label) in taskStatusText.entries)
                      DropdownMenuItem(value: v, child: Text(label)),
                  ],
                ),
                if (_saving) const SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2)),
                ..._threadLinks(t, widget.onOpenThread),
              ],
            ),
          ],
        ),
      ),
    );
  }
}

/// 最近完成的一项任务：标题、谁做的、什么时候，结果收起来。
class _DoneTask extends StatelessWidget {
  const _DoneTask({required this.task, required this.state, required this.onOpenThread});

  final Task task;
  final AppState state;
  final void Function(String threadId) onOpenThread;

  @override
  Widget build(BuildContext context) {
    final t = task;
    final muted = TextStyle(color: Theme.of(context).hintColor);
    final links = _threadLinks(t, onOpenThread);
    if (t.result == null || t.result!.isEmpty) {
      return ListTile(
        title: Text(t.title),
        subtitle: Text(_taskMeta(state, t), style: muted),
        trailing: links.isEmpty ? null : Row(mainAxisSize: MainAxisSize.min, children: links),
      );
    }
    return ExpansionTile(
      title: Text(t.title),
      subtitle: Text(_taskMeta(state, t), style: muted),
      expandedCrossAxisAlignment: CrossAxisAlignment.stretch,
      childrenPadding: const EdgeInsets.fromLTRB(16, 0, 16, 12),
      children: [
        MessageMarkdown(t.result!, reportHint: t.assignee),
        if (links.isNotEmpty) Wrap(children: links),
      ],
    );
  }
}

class ApprovalCard extends StatefulWidget {
  const ApprovalCard({super.key, required this.approval, required this.onOpenThread});

  final Approval approval;
  final void Function(String threadId) onOpenThread;

  @override
  State<ApprovalCard> createState() => _ApprovalCardState();
}

class _ApprovalCardState extends State<ApprovalCard> {
  int? _sending;

  Future<void> _choose(int i) async {
    setState(() => _sending = i);
    try {
      await AppScope.read(context).resolveApproval(widget.approval, i);
    } on RequestError catch (e) {
      if (mounted) showError(context, '操作失败：$e');
    } finally {
      if (mounted) setState(() => _sending = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final a = widget.approval;
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final muted = TextStyle(color: theme.hintColor);
    final thread = a.subjectThreadId;
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(a.question, style: theme.textTheme.titleMedium),
            if (a.reason.isNotEmpty) ...[const SizedBox(height: 4), Text(a.reason, style: muted)],
            if (thread != null)
              Align(
                alignment: Alignment.centerLeft,
                child: TextButton(
                  style: TextButton.styleFrom(padding: EdgeInsets.zero),
                  onPressed: () => widget.onOpenThread(thread),
                  child: Text(a.subjectAgentId != null ? '${state.agentName(a.subjectAgentId!)} 提问 · 查看对话' : '查看对话'),
                ),
              ),
            const SizedBox(height: 8),
            // 选项整行可点，手机上好按（和网页控制台 F-1 一样）。
            for (final (i, o) in a.options.indexed)
              Padding(
                padding: const EdgeInsets.only(bottom: 8),
                child: _OptionButton(
                  option: o,
                  recommended: a.recommended == i,
                  chosen: a.resolution?.choice == i,
                  busy: _sending == i,
                  onPressed: a.pending && _sending == null && state.connected ? () => _choose(i) : null,
                ),
              ),
            if (a.resolution case final r?)
              Text(
                '${r.by} 于 ${formatTime(r.atMs)} 选了「${r.choice < a.options.length ? a.options[r.choice].label : r.choice}」',
                style: muted,
              ),
            Text(
              '发起于 ${formatTime(a.createdAtMs)}',
              style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor),
            ),
          ],
        ),
      ),
    );
  }
}

class _OptionButton extends StatelessWidget {
  const _OptionButton({
    required this.option,
    required this.recommended,
    required this.chosen,
    required this.busy,
    required this.onPressed,
  });

  final ApprovalOption option;
  final bool recommended;
  final bool chosen;
  final bool busy;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final child = Padding(
      padding: const EdgeInsets.symmetric(vertical: 10),
      child: Row(
        children: [
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  recommended ? '${option.label}（推荐）' : option.label,
                  style: const TextStyle(fontWeight: FontWeight.w600),
                ),
                if (option.consequence.isNotEmpty) Text(option.consequence, style: theme.textTheme.bodySmall),
              ],
            ),
          ),
          if (busy) const SizedBox.square(dimension: 18, child: CircularProgressIndicator(strokeWidth: 2)),
          if (chosen) const Icon(Icons.check),
        ],
      ),
    );
    return recommended || chosen
        ? FilledButton.tonal(onPressed: onPressed, child: child)
        : OutlinedButton(onPressed: onPressed, child: child);
  }
}
