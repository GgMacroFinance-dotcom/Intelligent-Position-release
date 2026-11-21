// 看员工写的报告（backlog F-14，和控制台的 F-4 一样）：消息、任务结果、报告里提到的 `reports/<文件名>.md`
// 都能点开，在一个页面（窄屏）或浮层（宽屏）里按 Markdown 显示。

import 'package:flutter/material.dart';
import 'package:markdown/markdown.dart' as md;

import '../protocol/client.dart';
import '../protocol/models.dart';
import 'common.dart';

/// 链接里表示“打开这份报告”的前缀，后面是 `reports/<文件名>.md`。
const reportScheme = 'ipreport:';

/// 文字里提到的报告路径。和控制台 markdown.js 的 `REPORT_REF` 一样：文件名里不能有空白、斜杠、引号、括号和中文标点；
/// 前面不能紧跟字母数字（`myreports/x.md` 不算），`.md` 后面不能紧跟文件名字符。
const _reportRef = r'''(?<![A-Za-z0-9_/.\-])reports/[^\s/\\`'"<>()（）\[\]{}|*，。；：、！？]+?\.md(?![A-Za-z0-9_.\-])''';

/// 正文里的 `reports/<文件名>.md` 变成能点的链接。
class ReportRefSyntax extends md.InlineSyntax {
  ReportRefSyntax() : super(_reportRef, startCharacter: 0x72 /* r */);

  @override
  bool onMatch(md.InlineParser parser, Match match) {
    final path = match[0]!;
    parser.addNode(md.Element('a', [md.Text(path)])..attributes['href'] = '$reportScheme$path');
    return true;
  }
}

/// 行内代码里只有一个报告路径时（`` `reports/x.md` ``），也变成链接。显示成普通链接的颜色，不保留代码的灰底：
/// 灰底的看不出能点（在真 App 里试过）。
class ReportCodeRefSyntax extends md.InlineSyntax {
  ReportCodeRefSyntax() : super('`($_reportRef)`', startCharacter: 0x60 /* ` */);

  @override
  bool onMatch(md.InlineParser parser, Match match) {
    final path = match[1]!;
    parser.addNode(md.Element('a', [md.Text(path)])..attributes['href'] = '$reportScheme$path');
    return true;
  }
}

/// 链接是不是指向一份报告：`ipreport:` 链接，或者写成 Markdown 链接的相对路径 `reports/x.md`。返回报告路径。
String? reportPathOf(String? href) {
  if (href == null) return null;
  final h = href.startsWith(reportScheme) ? href.substring(reportScheme.length) : href;
  return RegExp('^$_reportRef\$').hasMatch(h) ? h : null;
}

/// 打开一份报告。`hint` 是可能写了它的员工（比如写这条消息的员工）。
Future<void> openReport(BuildContext context, String path, {String? hint}) {
  final page = ReportView(path: path, hint: hint);
  if (MediaQuery.sizeOf(context).width < 840) {
    // 用最外层的导航器：对话页里嵌着自己的导航器（窗口缩放时保留会话），报告要盖住整个窗口。
    return Navigator.of(context, rootNavigator: true).push(MaterialPageRoute<void>(builder: (_) => page));
  }
  return showDialog<void>(
    context: context,
    builder: (context) => Dialog(
      clipBehavior: Clip.antiAlias,
      child: ConstrainedBox(
        constraints: BoxConstraints(maxWidth: 860, maxHeight: MediaQuery.sizeOf(context).height * 0.85),
        child: page,
      ),
    ),
  );
}

class ReportView extends StatefulWidget {
  const ReportView({super.key, required this.path, this.hint});

  final String path;
  final String? hint;

  @override
  State<ReportView> createState() => _ReportViewState();
}

class _ReportViewState extends State<ReportView> {
  late final Future<ReportText> _report = AppScope.read(context).readReport(widget.path, hint: widget.hint);

  @override
  Widget build(BuildContext context) {
    final state = AppScope.of(context);
    final theme = Theme.of(context);
    final name = widget.path.replaceFirst(RegExp(r'^reports/'), '');
    return FutureBuilder<ReportText>(
      future: _report,
      builder: (context, snap) {
        final r = snap.data;
        return Scaffold(
          appBar: AppBar(
            title: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(name, overflow: TextOverflow.ellipsis),
                if (r != null)
                  Text(
                    '${state.agentName(r.agentId)} · ${formatTime(r.modifiedMs)}',
                    style: theme.textTheme.bodySmall?.copyWith(color: theme.hintColor),
                  ),
              ],
            ),
            actions: [
              IconButton(tooltip: '关闭', onPressed: () => Navigator.of(context).pop(), icon: const Icon(Icons.close)),
            ],
            automaticallyImplyLeading: false,
          ),
          body: switch (snap) {
            AsyncSnapshot(hasError: true, :final error) => EmptyHint(
              error is RequestError ? '打不开报告：${error.message}' : '打不开报告：$error',
            ),
            AsyncSnapshot(:final data?) => SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(20, 12, 20, 32),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  if (data.truncated)
                    Padding(
                      padding: const EdgeInsets.only(bottom: 12),
                      child: Text('报告太长，只显示前 1 MB。', style: TextStyle(color: theme.colorScheme.error)),
                    ),
                  MessageMarkdown(data.text, reportHint: data.agentId),
                ],
              ),
            ),
            _ => const Center(child: CircularProgressIndicator()),
          },
        );
      },
    );
  }
}
