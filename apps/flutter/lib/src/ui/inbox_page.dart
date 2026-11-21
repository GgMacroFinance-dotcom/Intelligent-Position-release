// “待办”页（apps-plan 第 6 节，手机首页）：员工提问、等你决定的审批卡片、受阻的任务。
// 最近完成的任务和新报告在 backlog F-14。

import 'package:flutter/material.dart';

import '../app_state.dart';
import '../protocol/client.dart';
import '../protocol/models.dart';
import 'common.dart';

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
    if (pending.isEmpty && blocked.isEmpty) {
      return EmptyHint(state.connected ? '没有等你处理的事。' : '连上主机后，等你处理的事会显示在这里。');
    }
    final theme = Theme.of(context);
    Widget heading(String text) => Padding(
      padding: const EdgeInsets.fromLTRB(4, 16, 4, 8),
      child: Text(text, style: theme.textTheme.titleMedium),
    );
    return ListView(
      padding: const EdgeInsets.fromLTRB(12, 0, 12, 24),
      children: [
        if (questions.isNotEmpty) heading('员工在问你（${questions.length}）'),
        for (final a in questions) ApprovalCard(approval: a, onOpenThread: onOpenThread),
        if (decisions.isNotEmpty) heading('等你决定（${decisions.length}）'),
        for (final a in decisions) ApprovalCard(approval: a, onOpenThread: onOpenThread),
        if (blocked.isNotEmpty) heading('受阻的任务（${blocked.length}）'),
        for (final t in blocked) _BlockedTask(task: t, state: state, onOpenThread: onOpenThread),
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

class _BlockedTask extends StatelessWidget {
  const _BlockedTask({required this.task, required this.state, required this.onOpenThread});

  final Task task;
  final AppState state;
  final void Function(String threadId) onOpenThread;

  @override
  Widget build(BuildContext context) {
    final who = task.assignee != null ? state.agentName(task.assignee!) : (task.role ?? '—');
    final thread = task.threadId;
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: ListTile(
        title: Text(task.title),
        subtitle: Text([who, formatTime(task.updatedAtMs), if (task.result != null) task.result!].join(' · ')),
        trailing: thread == null ? null : const Icon(Icons.chevron_right),
        onTap: thread == null ? null : () => onOpenThread(thread),
      ),
    );
  }
}
