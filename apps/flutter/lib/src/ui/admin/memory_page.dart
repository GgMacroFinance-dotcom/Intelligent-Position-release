// “记忆”页（F-18，只给“管理”权限）：一个岗位的决策者笔记、岗位记忆摘要、人要求的更正、各对话的记忆，
// 关掉了记忆的对话，清空。和网页控制台“记忆”页一样（rust/web/src/app.js 的 renderMemory）。

import 'dart:async';

import 'package:flutter/material.dart';

import '../../protocol/client.dart';
import '../../protocol/models.dart';
import '../common.dart';

class MemoryPage extends StatefulWidget {
  const MemoryPage({super.key});

  @override
  State<MemoryPage> createState() => _MemoryPageState();
}

class _MemoryPageState extends State<MemoryPage> {
  String? _role;
  MemoryView? _view;
  List<String> _off = [];
  String? _error;
  bool _busy = false;
  final _editors = {for (final l in MemoryLayer.values) l: TextEditingController()};

  @override
  void initState() {
    super.initState();
    final roles = AppScope.read(context).roles;
    if (roles.isNotEmpty) {
      _role = roles.first.id;
      unawaited(_load());
    }
  }

  @override
  void dispose() {
    for (final c in _editors.values) {
      c.dispose();
    }
    super.dispose();
  }

  /// 读一个岗位的记忆。两个输入框换成读到的内容（没保存的改动会丢，换岗位、刷新时才这样）。
  Future<void> _load() async {
    final role = _role;
    if (role == null) return;
    final c = AppScope.read(context).client;
    setState(() => _error = null);
    try {
      final (view, off) = await (c.getMemory(role), c.listMemoryOffThreads()).wait;
      if (!mounted || _role != role) return;
      setState(() {
        _view = view;
        _off = off;
        for (final l in MemoryLayer.values) {
          _editors[l]!.text = view.layer(l)?.text ?? '';
        }
      });
    } on ParallelWaitError<(MemoryView?, List<String>?), (AsyncError?, AsyncError?)> catch (e) {
      final err = e.errors.$1?.error ?? e.errors.$2?.error;
      if (mounted && _role == role) setState(() => _error = '加载失败：$err');
    } on RequestError catch (e) {
      if (mounted && _role == role) setState(() => _error = '加载失败：$e');
    }
  }

  /// 发一个改记忆的请求，用回复里的新视图重画。`resetLayer` 给了时把那个输入框换成保存后的内容。
  Future<void> _act(Future<MemoryView> Function(IpClient c, String role) f, {MemoryLayer? resetLayer}) async {
    final role = _role;
    if (role == null) return;
    setState(() => _busy = true);
    try {
      final view = await f(AppScope.read(context).client, role);
      if (!mounted || _role != role) return;
      setState(() {
        _view = view;
        if (resetLayer != null) _editors[resetLayer]!.text = view.layer(resetLayer)?.text ?? '';
      });
    } on RequestError catch (e) {
      if (mounted) showError(context, '操作失败：$e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  Future<void> _setThreadMemory(String threadId, {required bool enabled}) async {
    setState(() => _busy = true);
    try {
      final off = await AppScope.read(context).client.setThreadMemory(threadId, enabled: enabled);
      if (!mounted) return;
      setState(() => _off = off);
      // 关掉时这个对话已提炼的内容被抹掉了，要重新取。
      if (!enabled) await _reloadView();
    } on RequestError catch (e) {
      if (mounted) showError(context, '操作失败：$e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  /// 只重新取记忆，不动输入框里还没保存的内容。
  Future<void> _reloadView() async {
    final role = _role;
    if (role == null) return;
    try {
      final view = await AppScope.read(context).client.getMemory(role);
      if (mounted && _role == role) setState(() => _view = view);
    } on RequestError catch (e) {
      if (mounted) showError(context, '刷新失败：$e');
    }
  }

  Future<void> _clear(String roleName) async {
    final ok = await showDialog<bool>(
      context: context,
      builder: (context) => AlertDialog(
        title: Text('清空「$roleName」的全部记忆？'),
        content: const Text('岗位记忆摘要、各对话的记忆、人要求的更正都清掉，不能恢复。决策者笔记不动。'),
        actions: [
          TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('取消')),
          FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('清空')),
        ],
      ),
    );
    if (ok == true) await _act((c, role) => c.clearMemory(role), resetLayer: MemoryLayer.summary);
  }

  String _threadLabel(String id) {
    for (final t in AppScope.read(context).threads) {
      if (t.threadId == id) {
        final p = t.preview;
        return p == null || p.isEmpty ? id : p;
      }
    }
    return id;
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final small = theme.textTheme.bodySmall;
    final roles = state.roles;
    if (roles.isEmpty) return const EmptyHint('还没有岗位。在主机的 ip.yaml 的 roles 里定义。');
    final role = _role ?? roles.first.id;
    final view = _view;

    Widget heading(String text) => Padding(
      padding: const EdgeInsets.fromLTRB(16, 16, 16, 4),
      child: Text(text, style: theme.textTheme.titleMedium),
    );
    Widget hint(String text) => Padding(
      padding: const EdgeInsets.symmetric(horizontal: 16),
      child: Text(text, style: small),
    );

    Widget editor(MemoryLayer layer, String title, String help) {
      final current = view?.layer(layer);
      return Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          heading(title),
          hint(help),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: TextField(
              key: ValueKey('memory-${layer.name}'),
              controller: _editors[layer],
              minLines: layer == MemoryLayer.notes ? 3 : 6,
              maxLines: layer == MemoryLayer.notes ? 8 : 16,
              decoration: InputDecoration(border: const OutlineInputBorder(), labelText: title),
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 4, 16, 0),
            child: Row(
              children: [
                Expanded(
                  child: Text(current == null ? '还没有内容' : '上次改动 ${formatTime(current.updatedMs)}', style: small),
                ),
                FilledButton(
                  onPressed: _busy
                      ? null
                      : () => _act((c, r) => c.saveMemoryText(r, layer, _editors[layer]!.text), resetLayer: layer),
                  child: Text('保存$title'),
                ),
              ],
            ),
          ),
        ],
      );
    }

    final roleName = state.roleName(role);
    return ListView(
      padding: const EdgeInsets.only(bottom: 32),
      children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 8, 0),
          child: Row(
            children: [
              const Text('岗位 '),
              DropdownButton<String>(
                value: role,
                onChanged: (r) {
                  if (r == null || r == _role) return;
                  setState(() {
                    _role = r;
                    _view = null;
                  });
                  unawaited(_load());
                },
                items: [
                  for (final r in roles) DropdownMenuItem(value: r.id, child: Text('${r.name ?? r.id}（${r.id}）')),
                ],
              ),
              const Spacer(),
              IconButton(tooltip: '刷新', onPressed: _busy ? null : _load, icon: const Icon(Icons.refresh)),
            ],
          ),
        ),
        if (_error != null)
          ListTile(
            title: Text(_error!, style: TextStyle(color: theme.colorScheme.error)),
            trailing: TextButton(onPressed: _load, child: const Text('重试')),
          )
        else if (view == null)
          const Padding(
            padding: EdgeInsets.all(32),
            child: Center(child: CircularProgressIndicator()),
          )
        else ...[
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: view.enabled
                ? Text('这个岗位开了记忆：闲置下来的对话会被提炼，再合并成岗位记忆摘要，放进这个岗位每位员工的提示词。', style: small)
                : Card(
                    color: theme.colorScheme.secondaryContainer,
                    margin: EdgeInsets.zero,
                    child: const Padding(
                      padding: EdgeInsets.all(12),
                      child: Text('这个岗位没开记忆（ip.yaml 的 memory.enabled 和 memory.roles）。决策者笔记照常生效；岗位记忆摘要不进提示词，对话也不会被提炼。'),
                    ),
                  ),
          ),
          editor(MemoryLayer.notes, '决策者笔记', '你写给这个岗位的话，每位员工每轮都会带上。只有人能改。'),
          editor(
            MemoryLayer.summary,
            '岗位记忆摘要',
            view.summaryFromThreads > 0
                ? '这一版基于 ${view.summaryFromThreads} 份对话记忆。你可以直接改，下次合并会在你改过的基础上接着改。清空后下次合并会从对话记忆重新生成，想彻底清掉请用页面底部的“清空”。'
                : '还没有合并过，或是人手写的一版。你可以直接改，下次合并会在这一版的基础上接着改。',
          ),
          heading('人要求的更正（${view.corrections.length}）'),
          hint('有人在对话里明确说“记住……”“以后别……”“忘掉……”时，员工记在这里，下次合并时生效。'),
          if (view.corrections.isEmpty) const ListTile(title: Text('没有。')),
          for (final n in view.corrections)
            ListTile(
              key: ValueKey('note-${n.noteId}'),
              title: Text(n.text),
              subtitle: Text('${formatTime(n.createdMs)} · ${n.appliedMs != null ? '已合并进摘要' : '等下次合并'}'),
              trailing: Wrap(
                children: [
                  if (n.threadId != null)
                    IconButton(
                      tooltip: '看对话',
                      icon: const Icon(Icons.chat_bubble_outline),
                      onPressed: () => state.openThreadFromAnywhere(n.threadId!),
                    ),
                  IconButton(
                    tooltip: '删除这条更正',
                    icon: const Icon(Icons.delete_outline),
                    onPressed: _busy ? null : () => _act((c, r) => c.deleteMemoryNote(r, n.noteId)),
                  ),
                ],
              ),
            ),
          heading('各对话的记忆（${view.threads.length}）'),
          hint('每个对话提炼出的一段。抹掉以后，下次合并会把摘要里只来自它的内容拿掉；这个对话有新消息后会重新提炼，不想让它再进记忆请用“抹掉并关掉记忆”。'),
          if (view.threads.isEmpty) const ListTile(title: Text('还没有。')),
          for (final t in view.threads)
            Padding(
              key: ValueKey('thread-${t.threadId}'),
              padding: const EdgeInsets.fromLTRB(16, 12, 16, 0),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                spacing: 4,
                children: [
                  Text(t.title ?? '（没有标题）', style: theme.textTheme.titleSmall),
                  Text(t.summary),
                  Text(
                    '提炼于 ${formatTime(t.generatedMs)} · 被员工读过 ${t.usageCount} 次'
                    '${t.lastUsageMs != null ? '，最近 ${formatTime(t.lastUsageMs!)}' : ''}',
                    style: small,
                  ),
                  Wrap(
                    spacing: 4,
                    children: [
                      TextButton(onPressed: () => state.openThreadFromAnywhere(t.threadId), child: const Text('看对话')),
                      TextButton(
                        onPressed: _busy ? null : () => _act((c, r) => c.forgetThreadMemory(r, t.threadId)),
                        child: const Text('抹掉'),
                      ),
                      if (!_off.contains(t.threadId))
                        TextButton(
                          onPressed: _busy ? null : () => _setThreadMemory(t.threadId, enabled: false),
                          child: const Text('抹掉并关掉记忆'),
                        ),
                    ],
                  ),
                ],
              ),
            ),
          heading('关掉了记忆的对话（${_off.length}）'),
          hint('这些对话不会被提炼，员工也不能在里面记更正。对所有岗位生效。'),
          for (final id in _off)
            ListTile(
              key: ValueKey('off-$id'),
              title: Text(_threadLabel(id), maxLines: 1, overflow: TextOverflow.ellipsis),
              onTap: () => state.openThreadFromAnywhere(id),
              trailing: TextButton(
                onPressed: _busy ? null : () => _setThreadMemory(id, enabled: true),
                child: const Text('重新打开'),
              ),
            ),
          heading('清空这个岗位的记忆'),
          hint('岗位记忆摘要、各对话的记忆、人要求的更正都清掉，不能恢复。决策者笔记不动。之后的对话照常提炼，记忆从头积累。'),
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
            child: Align(
              alignment: Alignment.centerLeft,
              child: OutlinedButton(
                style: OutlinedButton.styleFrom(foregroundColor: theme.colorScheme.error),
                onPressed: _busy ? null : () => _clear(roleName),
                child: const Text('清空'),
              ),
            ),
          ),
        ],
      ],
    );
  }
}
