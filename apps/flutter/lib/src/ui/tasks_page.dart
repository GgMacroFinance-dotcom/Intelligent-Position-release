// 任务页（backlog F-15，和控制台的任务页一样）：未完成和已结束的任务，状态能直接改，能新建任务。
// 定时任务只给“管理”权限（m2-spec D-31），在 F-18。

import 'package:flutter/material.dart';

import '../protocol/client.dart';
import 'common.dart';
import 'task_card.dart';

class TasksPage extends StatefulWidget {
  const TasksPage({super.key, required this.onOpenThread});

  final void Function(String threadId) onOpenThread;

  @override
  State<TasksPage> createState() => _TasksPageState();
}

class _TasksPageState extends State<TasksPage> {
  /// true 看未完成的，false 看已结束的（完成或取消）。
  bool _open = true;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final open = state.tasks.where((t) => t.isOpen).toList();
    final closed = state.tasks.where((t) => !t.isOpen).toList();
    final shown = _open ? open : closed;
    return ListView(
      padding: const EdgeInsets.fromLTRB(12, 12, 12, 24),
      children: [
        Wrap(
          spacing: 12,
          runSpacing: 8,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            SegmentedButton<bool>(
              segments: [
                ButtonSegment(value: true, label: Text('未完成（${open.length}）')),
                ButtonSegment(value: false, label: Text('已结束（${closed.length}）')),
              ],
              selected: {_open},
              onSelectionChanged: (v) => setState(() => _open = v.first),
            ),
            FilledButton.icon(
              onPressed: state.connected ? () => showNewTask(context) : null,
              icon: const Icon(Icons.add),
              label: const Text('新建任务'),
            ),
          ],
        ),
        const SizedBox(height: 12),
        if (shown.isEmpty)
          Padding(
            padding: const EdgeInsets.all(24),
            child: Text(
              _open ? '没有未完成的任务。' : '没有已结束的任务。',
              textAlign: TextAlign.center,
              style: TextStyle(color: Theme.of(context).hintColor),
            ),
          ),
        for (final t in shown) TaskCard(key: ValueKey(t.id), task: t, onOpenThread: widget.onOpenThread),
      ],
    );
  }
}

/// 新建任务：手机上从底部弹出，宽屏上是对话框。给了 [initialAssignee] 时默认选中那位员工。
Future<void> showNewTask(BuildContext context, {String? initialAssignee}) {
  if (MediaQuery.sizeOf(context).width < 720) {
    return showModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showDragHandle: true,
      builder: (context) => Padding(
        padding: EdgeInsets.only(bottom: MediaQuery.viewInsetsOf(context).bottom),
        child: NewTaskForm(initialAssignee: initialAssignee),
      ),
    );
  }
  return showDialog<void>(
    context: context,
    builder: (context) => Dialog(
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 520),
        child: NewTaskForm(initialAssignee: initialAssignee),
      ),
    ),
  );
}

class NewTaskForm extends StatefulWidget {
  const NewTaskForm({super.key, this.initialAssignee});

  final String? initialAssignee;

  @override
  State<NewTaskForm> createState() => _NewTaskFormState();
}

class _NewTaskFormState extends State<NewTaskForm> {
  final _title = TextEditingController();
  final _description = TextEditingController();
  late String? _assignee = widget.initialAssignee;
  DateTime? _due;
  bool _saving = false;
  String? _error;

  @override
  void dispose() {
    _title.dispose();
    _description.dispose();
    super.dispose();
  }

  Future<void> _pickDue() async {
    final now = DateTime.now();
    final day = await showDatePicker(
      context: context,
      firstDate: DateTime(now.year, now.month, now.day),
      lastDate: now.add(const Duration(days: 365 * 2)),
      initialDate: _due ?? now,
      helpText: '截止日期',
    );
    // 和控制台一样，截止到那天的 23:59:59（本机时区）。
    if (day != null) setState(() => _due = DateTime(day.year, day.month, day.day, 23, 59, 59));
  }

  Future<void> _create() async {
    final title = _title.text.trim();
    if (title.isEmpty) {
      setState(() => _error = '请写任务标题');
      return;
    }
    setState(() {
      _saving = true;
      _error = null;
    });
    try {
      final desc = _description.text.trim();
      await AppScope.read(context).createTask(
        title,
        description: desc.isEmpty ? null : desc,
        assignee: _assignee,
        dueMs: _due?.millisecondsSinceEpoch,
      );
      if (!mounted) return;
      final messenger = ScaffoldMessenger.of(context);
      Navigator.of(context).pop();
      messenger.showSnackBar(SnackBar(content: Text('已新建任务：$title')));
    } on RequestError catch (e) {
      if (mounted) setState(() => _error = '新建失败：$e');
    } finally {
      if (mounted) setState(() => _saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final due = _due;
    return SingleChildScrollView(
      padding: const EdgeInsets.fromLTRB(20, 4, 20, 20),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Text('新建任务', style: theme.textTheme.titleLarge),
          const SizedBox(height: 16),
          TextField(
            controller: _title,
            autofocus: true,
            decoration: const InputDecoration(
              labelText: '标题',
              hintText: '例如：写 10 月 CPI 点评',
              border: OutlineInputBorder(),
            ),
          ),
          const SizedBox(height: 12),
          TextField(
            controller: _description,
            minLines: 2,
            maxLines: 5,
            decoration: const InputDecoration(labelText: '说明（可选）', border: OutlineInputBorder()),
          ),
          const SizedBox(height: 12),
          DropdownButtonFormField<String?>(
            initialValue: _assignee,
            isExpanded: true,
            decoration: const InputDecoration(labelText: '负责人', border: OutlineInputBorder()),
            items: [
              const DropdownMenuItem<String?>(value: null, child: Text('暂不指定负责人')),
              for (final a in state.agents) DropdownMenuItem<String?>(value: a.id, child: Text('${a.name}（${a.role}）')),
            ],
            onChanged: (v) => setState(() => _assignee = v),
          ),
          const SizedBox(height: 12),
          Row(
            children: [
              Expanded(
                child: OutlinedButton.icon(
                  onPressed: _pickDue,
                  icon: const Icon(Icons.event),
                  label: Text(due == null ? '截止日期：不设' : '截止 ${due.year}-${due.month}-${due.day}'),
                ),
              ),
              if (due != null)
                IconButton(
                  tooltip: '不设截止日期',
                  onPressed: () => setState(() => _due = null),
                  icon: const Icon(Icons.close),
                ),
            ],
          ),
          if (_assignee != null) ...[
            const SizedBox(height: 8),
            Text('建好后主机会另开一个对话，把任务交给这位员工。', style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor)),
          ],
          if (_error != null) ...[
            const SizedBox(height: 8),
            Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
          ],
          const SizedBox(height: 16),
          FilledButton(
            onPressed: _saving || !state.connected ? null : _create,
            child: _saving
                ? const SizedBox.square(dimension: 18, child: CircularProgressIndicator(strokeWidth: 2))
                : const Text('新建'),
          ),
        ],
      ),
    );
  }
}
