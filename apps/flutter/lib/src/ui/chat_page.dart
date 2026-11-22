// 对话：对话列表、消息（员工回复按 Markdown，流式显示）、输入框。
// 宽屏上列表和对话左右并排，窄屏上点开对话推进新的一屏。

import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../app_state.dart';
import '../protocol/client.dart';
import '../protocol/models.dart';
import 'common.dart';

/// 列表和对话并排的最小宽度。
const double kWideChat = 840;

class ChatPage extends StatefulWidget {
  const ChatPage({
    super.key,
    required this.active,
    required this.threadOpen,
    required this.onOpenThread,
    required this.onNewChat,
    required this.onShowDetail,
    required this.onBack,
  });

  final bool active;
  final bool threadOpen;
  final void Function(String threadId) onOpenThread;
  final VoidCallback onNewChat;
  final VoidCallback onShowDetail;
  final VoidCallback onBack;

  @override
  State<ChatPage> createState() => _ChatPageState();
}

class _ChatPageState extends State<ChatPage> {
  final _navigator = GlobalKey<NavigatorState>();
  static const _detail = ValueKey('thread-detail');

  @override
  Widget build(BuildContext context) {
    return LayoutBuilder(
      builder: (context, box) {
        final split = box.maxWidth >= kWideChat;
        final list = ThreadList(onOpen: widget.onOpenThread, onNew: widget.onNewChat);
        return Row(
          children: [
            if (split) ...[SizedBox(width: 320, child: list), const VerticalDivider(width: 1)],
            Expanded(
              child: NavigatorPopHandler<void>(
                enabled: widget.active,
                onPopWithResult: (_) {
                  if (widget.active) _navigator.currentState!.maybePop();
                },
                child: Navigator(
                  key: _navigator,
                  requestFocus: widget.active,
                  onDidRemovePage: (page) {
                    if (page.key == _detail && !split && widget.threadOpen) widget.onBack();
                  },
                  pages: [
                    if (!split)
                      MaterialPage<void>(
                        key: const ValueKey('thread-list'),
                        child: Material(child: list),
                      ),
                    if (split || widget.threadOpen)
                      MaterialPage<void>(
                        key: _detail,
                        child: Focus(
                          canRequestFocus: false,
                          onFocusChange: (focused) {
                            if (focused && widget.active && !widget.threadOpen) widget.onShowDetail();
                          },
                          child: Material(child: ThreadScreen(compact: !split)),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}

class ThreadList extends StatelessWidget {
  const ThreadList({super.key, required this.onOpen, required this.onNew});

  final void Function(String threadId) onOpen;
  final VoidCallback onNew;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    return ListView.builder(
      itemCount: state.threads.length + 1,
      itemBuilder: (context, i) {
        if (i == 0) {
          return ListTile(
            leading: const Icon(Icons.add),
            title: const Text('新对话'),
            selected: state.threadId == null,
            onTap: onNew,
          );
        }
        final t = state.threads[i - 1];
        return ListTile(
          title: Text(t.preview ?? '（无文字）', maxLines: 2, overflow: TextOverflow.ellipsis),
          subtitle: Text('${formatTime(t.lastAtMs)} · ${t.messageCount} 条', style: TextStyle(color: theme.hintColor)),
          selected: t.threadId == state.threadId,
          onTap: () => onOpen(t.threadId),
        );
      },
    );
  }
}

/// 分栏只改变标题和列表，同一条详情路由保留滚动位置、输入法和草稿。
class ThreadScreen extends StatelessWidget {
  const ThreadScreen({super.key, required this.compact});

  final bool compact;

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final id = state.threadId;
    final agent = id == null ? null : state.threadAgent[id];
    return Column(
      children: [
        if (compact)
          AppBar(primary: false, title: Text(id == null ? '新对话' : (agent == null ? '对话' : state.agentName(agent)))),
        const Expanded(child: ThreadView()),
      ],
    );
  }
}

class ThreadView extends StatefulWidget {
  const ThreadView({super.key});

  @override
  State<ThreadView> createState() => _ThreadViewState();
}

class _ThreadViewState extends State<ThreadView> {
  final _scroll = ScrollController();

  /// 渲染过的消息按 id 缓存：消息写进存储后不再变，流式输出时每来一段文字就重建一次，
  /// 同一个 Widget 实例 Flutter 会跳过，不用把整段对话的 Markdown 重新解析（和网页控制台一样）。
  final _cache = <String, Widget>{};

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  Widget _message(AppState state, Message m) {
    final who = switch (m.author) {
      HumanAuthor(:final name) => name,
      AgentAuthor(:final agentId) => state.agentName(agentId),
      SystemAuthor() => '系统',
    };
    // 员工名单晚于消息加载时显示名会变，所以名字也算进缓存的键。
    final key = '${m.id}\u0000$who';
    if (_cache.length > 2000) _cache.clear();
    return _cache[key] ??= MessageTile(key: ValueKey(m.id), message: m, who: who);
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final id = state.threadId;
    final items = <Widget>[
      for (final m in state.messages) _message(state, m),
      for (final MapEntry(key: agentId, value: text) in state.drafts.entries)
        if (id == null || state.threadAgent[id] == null || state.threadAgent[id] == agentId)
          _Bubble(
            who: state.agentName(agentId),
            note: '正在输入…',
            child: MessageMarkdown(text, reportHint: agentId),
          ),
      if (id != null && state.turnNote[id] != null)
        Padding(
          padding: const EdgeInsets.symmetric(vertical: 8),
          child: Text(state.turnNote[id]!, style: TextStyle(color: Theme.of(context).hintColor, fontSize: 13)),
        ),
    ];
    return Column(
      children: [
        if (id != null && state.memoryOff != null) ThreadMemoryBar(threadId: id, off: state.memoryOff!.contains(id)),
        Expanded(
          child: items.isEmpty
              ? EmptyHint(id == null ? (state.agents.isEmpty ? '主机上没有数字员工。' : '选一位员工，或者交给自动分派，开始新对话。') : '')
              // 倒过来排：最新的在底部，来了新内容自动停在底部，往上翻时不会被拽下来。
              : ListView.builder(
                  controller: _scroll,
                  reverse: true,
                  padding: const EdgeInsets.fromLTRB(12, 12, 12, 4),
                  itemCount: items.length,
                  itemBuilder: (context, i) => items[items.length - 1 - i],
                ),
        ),
        const Composer(),
      ],
    );
  }
}

/// 对话顶上的一行：这个对话进不进岗位记忆；“管理”权限的设备能关掉、重新打开（和控制台一样）。
class ThreadMemoryBar extends StatefulWidget {
  const ThreadMemoryBar({super.key, required this.threadId, required this.off});

  final String threadId;
  final bool off;

  @override
  State<ThreadMemoryBar> createState() => _ThreadMemoryBarState();
}

class _ThreadMemoryBarState extends State<ThreadMemoryBar> {
  bool _busy = false;

  Future<void> _toggle() async {
    final state = AppScope.read(context);
    if (!widget.off) {
      final ok = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('不让这个对话进记忆？'),
          content: const Text('这个对话已经提炼进岗位记忆的内容会被抹掉，以后也不再提炼。随时可以重新打开。'),
          actions: [
            TextButton(onPressed: () => Navigator.pop(context, false), child: const Text('取消')),
            FilledButton(onPressed: () => Navigator.pop(context, true), child: const Text('不让它进记忆')),
          ],
        ),
      );
      if (ok != true || !mounted) return;
    }
    setState(() => _busy = true);
    try {
      await state.setThreadMemory(widget.threadId, enabled: widget.off);
    } on RequestError catch (e) {
      if (mounted) showError(context, '操作失败：$e');
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 4, 8, 0),
      child: Row(
        children: [
          Icon(widget.off ? Icons.memory_outlined : Icons.memory, size: 16, color: theme.hintColor),
          const SizedBox(width: 8),
          Expanded(
            child: Text(
              widget.off ? '这个对话不进岗位记忆。' : '岗位开了记忆时，这个对话闲置后会被提炼进岗位记忆。',
              style: TextStyle(color: theme.hintColor, fontSize: 13),
            ),
          ),
          // 开关记忆要“管理”权限。
          if (state.isAdmin)
            TextButton(
              onPressed: _busy || !state.connected ? null : _toggle,
              child: Text(widget.off ? '重新打开' : '不让它进记忆'),
            ),
        ],
      ),
    );
  }
}

class MessageTile extends StatelessWidget {
  const MessageTile({super.key, required this.message, required this.who});

  final Message message;
  final String who;

  @override
  Widget build(BuildContext context) {
    final m = message;
    final time = formatTime(m.atMs);
    return switch (m.body) {
      // 人写的消息照原样显示；员工和系统的消息按 Markdown 渲染。
      TextBody(:final text) => _Bubble(
        who: who,
        note: time,
        human: m.author is HumanAuthor,
        child: switch (m.author) {
          HumanAuthor() => SelectableText(text),
          AgentAuthor(:final agentId) => MessageMarkdown(text, reportHint: agentId),
          SystemAuthor() => MessageMarkdown(text),
        },
      ),
      ToolCallBody(:final name, :final input) => _Folded(title: '调用工具 $name', text: _pretty(input)),
      ToolResultBody(:final ok, :final output) => _Folded(title: ok ? '工具结果' : '工具失败', text: output, failed: !ok),
      SummaryBody(:final text) => _Folded(title: '较早的对话已压缩成摘要', text: text),
      UnknownBody(:final kind) => _Folded(title: '（这个 App 还不认识的消息：$kind）', text: ''),
    };
  }

  static String _pretty(Object? input) {
    if (input is String) return input;
    try {
      return const JsonEncoder.withIndent('  ').convert(input);
    } catch (_) {
      return '$input';
    }
  }
}

class _Bubble extends StatelessWidget {
  const _Bubble({required this.who, required this.note, required this.child, this.human = false});

  final String who;
  final String note;
  final Widget child;
  final bool human;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    return Align(
      alignment: human ? Alignment.centerRight : Alignment.centerLeft,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 720),
        child: Container(
          margin: const EdgeInsets.symmetric(vertical: 4),
          padding: const EdgeInsets.fromLTRB(12, 8, 12, 10),
          decoration: BoxDecoration(
            color: human ? scheme.primaryContainer : scheme.surfaceContainerHigh,
            borderRadius: BorderRadius.circular(12),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text.rich(
                TextSpan(
                  children: [
                    TextSpan(
                      text: who,
                      style: const TextStyle(fontWeight: FontWeight.w600),
                    ),
                    TextSpan(
                      text: '  $note',
                      style: TextStyle(color: theme.hintColor),
                    ),
                  ],
                ),
                style: theme.textTheme.bodySmall,
              ),
              const SizedBox(height: 4),
              child,
            ],
          ),
        ),
      ),
    );
  }
}

class _Folded extends StatelessWidget {
  const _Folded({required this.title, required this.text, this.failed = false});

  final String title;
  final String text;
  final bool failed;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Align(
      alignment: Alignment.centerLeft,
      child: ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 720),
        child: ExpansionTile(
          dense: true,
          tilePadding: const EdgeInsets.symmetric(horizontal: 8),
          childrenPadding: const EdgeInsets.fromLTRB(8, 0, 8, 8),
          expandedAlignment: Alignment.centerLeft,
          title: Text(
            title,
            style: theme.textTheme.bodySmall?.copyWith(color: failed ? theme.colorScheme.error : theme.hintColor),
          ),
          children: [
            SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: SelectableText(text, style: const TextStyle(fontFamily: 'monospace', fontSize: 12)),
            ),
          ],
        ),
      ),
    );
  }
}

/// 输入框和“发给谁”。输入框在重建时保留内容，事件推送不会冲掉正在打的字。
class Composer extends StatefulWidget {
  const Composer({super.key});

  @override
  State<Composer> createState() => _ComposerState();
}

class _ComposerState extends State<Composer> {
  final _text = TextEditingController();
  final _focus = FocusNode();
  bool _sending = false;

  @override
  void dispose() {
    _text.dispose();
    _focus.dispose();
    super.dispose();
  }

  Future<void> _send() async {
    final text = _text.text.trim();
    if (text.isEmpty || _sending) return;
    final state = AppScope.read(context);
    setState(() => _sending = true);
    _text.clear();
    try {
      await state.send(text);
    } on RequestError catch (e) {
      _text.text = text;
      if (mounted) showError(context, '发送失败：$e');
    } finally {
      if (mounted) setState(() => _sending = false);
    }
  }

  /// 桌面上 Enter 发送，Shift+Enter 换行；手机上点发送按钮。
  KeyEventResult _onKey(FocusNode node, KeyEvent e) {
    if (e is KeyDownEvent &&
        e.logicalKey == LogicalKeyboardKey.enter &&
        !HardwareKeyboard.instance.isShiftPressed &&
        !_text.value.isComposingRangeValid) {
      _send();
      return KeyEventResult.handled;
    }
    return KeyEventResult.ignored;
  }

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final current = state.currentAgent;
    // 当前选的员工不在名单里（名单还没加载）时先显示自动分派。
    final value = state.agents.any((a) => a.id == current) ? current : null;
    return Material(
      color: theme.colorScheme.surfaceContainerLow,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 8, 12, 8),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            DropdownButtonHideUnderline(
              child: DropdownButton<String?>(
                isExpanded: true,
                isDense: true,
                value: value,
                hint: const Text('自动分派（按路由规则）'),
                items: [
                  const DropdownMenuItem<String?>(value: null, child: Text('自动分派（按路由规则）')),
                  for (final a in state.agents)
                    DropdownMenuItem<String?>(value: a.id, child: Text('${a.name}（${a.role}）')),
                ],
                onChanged: state.connected ? state.chooseAgent : null,
              ),
            ),
            const SizedBox(height: 4),
            Row(
              crossAxisAlignment: CrossAxisAlignment.end,
              children: [
                Expanded(
                  child: Focus(
                    onKeyEvent: _onKey,
                    child: TextField(
                      controller: _text,
                      focusNode: _focus,
                      minLines: 1,
                      maxLines: 6,
                      textInputAction: TextInputAction.newline,
                      keyboardType: TextInputType.multiline,
                      decoration: const InputDecoration(
                        hintText: '写给员工……',
                        border: OutlineInputBorder(),
                        isDense: true,
                      ),
                    ),
                  ),
                ),
                const SizedBox(width: 8),
                IconButton.filled(
                  tooltip: '发送',
                  onPressed: state.connected && !_sending ? _send : null,
                  icon: const Icon(Icons.send),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
