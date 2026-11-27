// 一个任务的卡片：标题、说明、谁负责、受阻原因或结果，状态能直接改。待办（F-14）和任务页（F-15）共用。

import 'package:flutter/material.dart';

import '../app_state.dart';
import '../protocol/client.dart';
import '../protocol/models.dart';
import 'common.dart';

/// 任务状态的中文名，和控制台一样。
/// 人能选的任务状态。
const taskStatusText = {'open': '待开始', 'in_progress': '进行中', 'blocked': '受阻', 'done': '已完成', 'cancelled': '已取消'};

/// 任务状态的中文名，含只由主机设置的“待验收”（m2-spec D-41）。
String taskStatusLabel(String status) => taskStatusText[status] ?? (status == 'pending_approval' ? '待验收' : status);

/// 谁负责、谁派的、什么时候更新的。
String taskMeta(AppState state, Task t) {
  final who = t.assignee != null ? state.agentName(t.assignee!) : (t.role ?? '—');
  final by = t.createdByName ?? (t.createdByAgent != null ? state.agentName(t.createdByAgent!) : '系统');
  final due = t.dueMs;
  final dueText = due == null
      ? ''
      : ' · ${t.isOpen && due < DateTime.now().millisecondsSinceEpoch ? '已过期' : '截止'} ${formatTime(due)}';
  return '$who · 由 $by 派 · ${formatTime(t.updatedAtMs)}$dueText';
}

/// 执行对话、派活对话的跳转按钮。
List<Widget> threadLinks(Task t, void Function(String threadId) onOpenThread) => [
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
            Text(taskMeta(state, t), style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor)),
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
                  // 等验收（pending_approval）只由主机设置，不在可选项里：显示成提示，选“已完成”或“进行中”等于通过或打回。
                  value: taskStatusText.containsKey(t.status) ? t.status : null,
                  hint: Text(taskStatusLabel(t.status)),
                  isDense: true,
                  onChanged: _saving || !state.connected ? null : (v) => v == null ? null : _setStatus(v),
                  items: [
                    for (final MapEntry(key: v, value: label) in taskStatusText.entries)
                      DropdownMenuItem(value: v, child: Text(label)),
                  ],
                ),
                if (_saving) const SizedBox.square(dimension: 16, child: CircularProgressIndicator(strokeWidth: 2)),
                ...threadLinks(t, widget.onOpenThread),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
