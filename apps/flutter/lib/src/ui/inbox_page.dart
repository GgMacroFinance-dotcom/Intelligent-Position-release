// “待办”页（apps-plan 第 6 节，手机首页）：员工提问、等你决定的审批卡片、受阻的任务（能直接改状态），
// 以及最近 7 天完成的任务和新报告（backlog F-14，和控制台的 F-5 一样）。

import 'package:flutter/material.dart';

import '../app_state.dart';
import '../protocol/client.dart';
import '../protocol/models.dart';
import 'common.dart';
import 'report.dart';
import 'shell/completion_approval.dart';
import 'task_card.dart';
import 'rework_note.dart';

/// “最近”是几天内，每类最多列几条。
const _recent = Duration(days: 7);
const _recentMax = 10;

class InboxPage extends StatelessWidget {
  const InboxPage({super.key, required this.onOpenThread});

  /// 点“查看对话”时去对话页打开它。
  final void Function(String threadId) onOpenThread;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final pending = state.pendingApprovals;
    final completion = state.pendingCompletionApprovals;
    final questions = pending.where((a) => a.isAgentQuestion).toList();
    final decisions = pending.where((a) => !a.isAgentQuestion && !a.isTaskCompletion).toList();
    final blocked = state.blockedTasks;
    final since = DateTime.now().subtract(_recent).millisecondsSinceEpoch;
    final done = state.tasks.where((t) => t.status == 'done' && t.updatedAtMs >= since).take(_recentMax).toList();
    final reports = state.reports.where((r) => r.modifiedMs >= since).take(_recentMax).toList();
    final hasAction = questions.isNotEmpty || decisions.isNotEmpty || completion.isNotEmpty || blocked.isNotEmpty;
    if (!state.connected && !hasAction && done.isEmpty && reports.isEmpty) {
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
        if (!hasAction)
          Padding(
            padding: const EdgeInsets.fromLTRB(4, 20, 4, 4),
            child: Text('没有等你处理的事。', style: muted),
          ),
        if (completion.isNotEmpty) heading('已完成 · 待审批（${completion.length}）'),
        for (final c in completion)
          _CompletionApprovalCard(key: ValueKey('page-c-${c.id}'), item: c, onOpenThread: onOpenThread),
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

/// 工作台「待办」页里的「已完成 · 待审批」卡片（与左中下停泊区、左上角待办、员工三栏窗四处同源）。
class _CompletionApprovalCard extends StatefulWidget {
  const _CompletionApprovalCard({super.key, required this.item, required this.onOpenThread});

  final CompletionApproval item;
  final void Function(String threadId) onOpenThread;

  @override
  State<_CompletionApprovalCard> createState() => _CompletionApprovalCardState();
}

class _CompletionApprovalCardState extends State<_CompletionApprovalCard> {
  CompletionDecision? _busy;

  Future<void> _decide(CompletionDecision decision) async {
    String? note;
    if (decision == CompletionDecision.reject) {
      // 先问要改什么；取消就不打回。
      note = await askReworkNote(context, widget.item);
      if (note == null || !mounted) return;
    }
    setState(() => _busy = decision);
    try {
      await AppScope.read(context)
          .decideCompletionApproval(widget.item, decision, note: (note?.isEmpty ?? true) ? null : note);
    } on RequestError catch (e) {
      if (mounted) showError(context, '审批操作失败：$e');
    } finally {
      if (mounted) setState(() => _busy = null);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final muted = TextStyle(color: theme.hintColor);
    final item = widget.item;
    final canAct = _busy == null && (item.source == CompletionSource.localDemo || state.connected);
    return Card(
      margin: const EdgeInsets.only(bottom: 12),
      child: Padding(
        padding: const EdgeInsets.all(16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                // 员工名长、时间带日期时手机宽度放不下：标签缩略，时间照常显示。
                Expanded(
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: Container(
                      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2.5),
                      decoration: BoxDecoration(
                        color: const Color(0x22D97706),
                        borderRadius: BorderRadius.circular(999),
                      ),
                      child: Text(
                        '${item.agentName} · 完成 · 待审批',
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(fontSize: 11.5, fontWeight: FontWeight.w600, color: Color(0xFFB45309)),
                      ),
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                Text(formatTime(item.createdAtMs), style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor)),
              ],
            ),
            const SizedBox(height: 8),
            Text(item.taskTitle, style: theme.textTheme.titleMedium),
            if (item.summary.isNotEmpty) ...[const SizedBox(height: 4), Text(item.summary, style: muted)],
            if (item.threadId case final tid?)
              Align(
                alignment: Alignment.centerLeft,
                child: TextButton(
                  style: TextButton.styleFrom(padding: EdgeInsets.zero),
                  onPressed: () => widget.onOpenThread(tid),
                  child: const Text('查看对话'),
                ),
              ),
            const SizedBox(height: 12),
            Row(
              children: [
                FilledButton(
                  onPressed: canAct ? () => _decide(CompletionDecision.approve) : null,
                  child: _busy == CompletionDecision.approve
                      ? const SizedBox.square(dimension: 14, child: CircularProgressIndicator(strokeWidth: 2))
                      : const Text('审批通过'),
                ),
                const SizedBox(width: 10),
                OutlinedButton(
                  onPressed: canAct ? () => _decide(CompletionDecision.reject) : null,
                  child: _busy == CompletionDecision.reject
                      ? const SizedBox.square(dimension: 14, child: CircularProgressIndicator(strokeWidth: 2))
                      : const Text('打回修改'),
                ),
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
    final links = threadLinks(t, onOpenThread);
    if (t.result == null || t.result!.isEmpty) {
      return ListTile(
        title: Text(t.title),
        subtitle: Text(taskMeta(state, t), style: muted),
        trailing: links.isEmpty ? null : Row(mainAxisSize: MainAxisSize.min, children: links),
      );
    }
    return ExpansionTile(
      title: Text(t.title),
      subtitle: Text(taskMeta(state, t), style: muted),
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
  const ApprovalCard({super.key, required this.approval, required this.onOpenThread, this.hideThreadLink = false});

  final Approval approval;
  final void Function(String threadId) onOpenThread;

  /// 嵌在对话流里时，当前看的就是这个对话，不再重复显示“查看对话”按钮。
  final bool hideThreadLink;

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
            if (thread != null && !widget.hideThreadLink)
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
    final highlighted = recommended || chosen;
    // 用按钮内部的 DefaultTextStyle 前景色，避免推荐/已选项（深底浅字）里说明文字取到全局深色而看不清。
    final child = Builder(
      builder: (context) {
        final fg = DefaultTextStyle.of(context).style.color;
        final subColor = highlighted ? fg?.withValues(alpha: 0.78) : theme.hintColor;
        return Padding(
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
                    if (option.consequence.isNotEmpty)
                      Text(option.consequence, style: theme.textTheme.bodySmall?.copyWith(color: subColor)),
                  ],
                ),
              ),
              if (busy) const SizedBox.square(dimension: 18, child: CircularProgressIndicator(strokeWidth: 2)),
              if (chosen) const Icon(Icons.check),
            ],
          ),
        );
      },
    );
    return highlighted
        ? FilledButton.tonal(onPressed: onPressed, child: child)
        : OutlinedButton(onPressed: onPressed, child: child);
  }
}
