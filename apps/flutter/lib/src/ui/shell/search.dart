// 搜索全部消息（requirements.txt N-1）：⌘K 或左栏顶上的放大镜打开，输入后搜主机上所有对话的消息，
// 点结果打开那个对话。主机的 search 请求中文按二字切分（m2-spec），网页控制台也用它。

import 'dart:async';

import 'package:flutter/material.dart';

import '../../protocol/client.dart';
import '../../protocol/wire.dart' as w;
import '../common.dart';
import 'tokens.dart';

Future<void> showMessageSearch(BuildContext context, {required void Function(String threadId) onOpen}) {
  return showGeneralDialog<void>(
    context: context,
    barrierDismissible: true,
    barrierLabel: '关闭搜索',
    barrierColor: Shell.of(context).scrim,
    transitionDuration: Motion.dialog,
    pageBuilder: (_, _, _) => _SearchSheet(onOpen: onOpen),
    transitionBuilder: (context, anim, _, child) {
      final t = CurvedAnimation(parent: anim, curve: Motion.curve);
      return FadeTransition(
        opacity: t,
        child: SlideTransition(
          position: Tween(begin: const Offset(0, -0.01), end: Offset.zero).animate(t),
          child: child,
        ),
      );
    },
  );
}

class _SearchSheet extends StatefulWidget {
  const _SearchSheet({required this.onOpen});

  final void Function(String threadId) onOpen;

  @override
  State<_SearchSheet> createState() => _SearchSheetState();
}

class _SearchSheetState extends State<_SearchSheet> {
  final _text = TextEditingController();
  Timer? _debounce;
  List<w.SearchHit>? _hits;
  String? _error;
  int _seq = 0;

  @override
  void dispose() {
    _debounce?.cancel();
    _text.dispose();
    super.dispose();
  }

  void _changed(String q) {
    _debounce?.cancel();
    if (q.trim().isEmpty) {
      setState(() {
        _hits = null;
        _error = null;
      });
      return;
    }
    _debounce = Timer(const Duration(milliseconds: 220), () => _run(q.trim()));
  }

  Future<void> _run(String q) async {
    final seq = ++_seq;
    try {
      final hits = await AppScope.read(context).client.search(q);
      if (mounted && seq == _seq) {
        setState(() {
          _hits = hits;
          _error = null;
        });
      }
    } on RequestError catch (e) {
      if (mounted && seq == _seq) setState(() => _error = '搜不了：$e');
    }
  }

  void _open(w.SearchHit h) {
    Navigator.of(context).pop();
    widget.onOpen(h.threadId);
  }

  @override
  Widget build(BuildContext context) {
    final s = Shell.of(context);
    final hits = _hits;
    return Align(
      alignment: const Alignment(0, -0.6),
      child: Padding(
        padding: const EdgeInsets.all(24),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 620, maxHeight: 520),
          child: Material(
            color: s.menu,
            clipBehavior: Clip.antiAlias,
            shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(Radii.x3)),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                Padding(
                  padding: const EdgeInsets.fromLTRB(16, 10, 10, 10),
                  child: Row(
                    children: [
                      Icon(Icons.search, size: 20, color: s.inkSecondary),
                      const SizedBox(width: 10),
                      Expanded(
                        child: TextField(
                          controller: _text,
                          autofocus: true,
                          onChanged: _changed,
                          onSubmitted: (_) {
                            if (hits != null && hits.isNotEmpty) _open(hits.first);
                          },
                          style: TextStyle(color: s.ink, fontSize: 16),
                          decoration: const InputDecoration(
                            hintText: '搜索全部消息',
                            filled: false,
                            border: InputBorder.none,
                            enabledBorder: InputBorder.none,
                            focusedBorder: InputBorder.none,
                          ),
                        ),
                      ),
                      Text('Esc', style: TextStyle(color: s.inkTertiary, fontSize: 12)),
                    ],
                  ),
                ),
                if (_error != null || hits != null) Divider(height: 1, color: s.edge),
                if (_error != null)
                  Padding(
                    padding: const EdgeInsets.all(16),
                    child: Text(_error!, style: TextStyle(color: s.danger)),
                  )
                else if (hits != null && hits.isEmpty)
                  Padding(
                    padding: const EdgeInsets.all(16),
                    child: Text('没有找到。', style: TextStyle(color: s.inkTertiary)),
                  )
                else if (hits != null)
                  Flexible(
                    child: ListView.builder(
                      shrinkWrap: true,
                      padding: const EdgeInsets.all(6),
                      itemCount: hits.length,
                      itemBuilder: (context, i) {
                        final h = hits[i];
                        return InkWell(
                          borderRadius: BorderRadius.circular(10),
                          onTap: () => _open(h),
                          child: Padding(
                            padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 9),
                            child: Column(
                              crossAxisAlignment: CrossAxisAlignment.start,
                              children: [
                                Text(
                                  h.text.replaceAll('\n', ' '),
                                  maxLines: 2,
                                  overflow: TextOverflow.ellipsis,
                                  style: TextStyle(color: s.ink, fontSize: 14),
                                ),
                                const SizedBox(height: 2),
                                Text(
                                  formatTime(h.atMs.toInt()),
                                  style: TextStyle(color: s.inkTertiary, fontSize: 12),
                                ),
                              ],
                            ),
                          ),
                        );
                      },
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
