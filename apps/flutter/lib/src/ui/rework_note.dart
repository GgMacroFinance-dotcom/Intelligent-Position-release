// 打回修改前问人要改什么（m2-spec D-41）：写的话作为 `note` 交给主机，主机写进负责人的对话，叫他照着改。

import 'package:flutter/material.dart';

import 'shell/completion_approval.dart';

/// 打回 [item] 前弹框问要改什么。取消返回 null（不打回）；没写返回空字符串（照样打回，主机会让负责人先问清楚）。
/// 本地演示项不连主机，不问，直接返回空字符串。
Future<String?> askReworkNote(BuildContext context, CompletionApproval item) async {
  if (item.source == CompletionSource.localDemo) return '';
  return showDialog<String>(
    context: context,
    builder: (context) => _ReworkNoteDialog(item: item),
  );
}

class _ReworkNoteDialog extends StatefulWidget {
  const _ReworkNoteDialog({required this.item});

  final CompletionApproval item;

  @override
  State<_ReworkNoteDialog> createState() => _ReworkNoteDialogState();
}

class _ReworkNoteDialogState extends State<_ReworkNoteDialog> {
  final _note = TextEditingController();

  @override
  void dispose() {
    _note.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return AlertDialog(
      title: Text('打回「${widget.item.taskTitle}」'),
      content: SizedBox(
        width: 420,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(
              '写下要改什么，会发给${widget.item.agentName}，他照着改完再交回。',
              style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor),
            ),
            const SizedBox(height: 12),
            TextField(
              key: const ValueKey('rework-note'),
              controller: _note,
              autofocus: true,
              minLines: 3,
              maxLines: 8,
              decoration: const InputDecoration(
                labelText: '要改的地方（可选）',
                hintText: '例如：补上 PPI 的同比，结论再短一点',
                border: OutlineInputBorder(),
              ),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(onPressed: () => Navigator.pop(context), child: const Text('取消')),
        FilledButton(
          key: const ValueKey('rework-confirm'),
          onPressed: () => Navigator.pop(context, _note.text.trim()),
          child: const Text('打回修改'),
        ),
      ],
    );
  }
}
