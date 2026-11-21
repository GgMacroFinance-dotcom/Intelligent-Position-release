// 各页共用的小东西：拿到状态、时间格式、Markdown、提示条。

import 'package:flutter/material.dart';
import 'package:flutter_markdown_plus/flutter_markdown_plus.dart';
import 'package:url_launcher/url_launcher.dart';

import '../app_state.dart';

/// 把 [AppState] 挂在树上，下面的页面用 `AppScope.of(context)` 拿，状态一变就重建。
class AppScope extends InheritedNotifier<AppState> {
  const AppScope({super.key, required AppState state, required super.child}) : super(notifier: state);

  static AppState of(BuildContext context) => context.dependOnInheritedWidgetOfExactType<AppScope>()!.notifier!;

  /// 只调方法、不随状态重建时用。
  static AppState read(BuildContext context) => context.getInheritedWidgetOfExactType<AppScope>()!.notifier!;
}

/// 和网页控制台一样：今天的只写时分，别的天加上月日。
String formatTime(int ms, {DateTime? now}) {
  final d = DateTime.fromMillisecondsSinceEpoch(ms);
  final n = now ?? DateTime.now();
  String two(int v) => v.toString().padLeft(2, '0');
  final hm = '${two(d.hour)}:${two(d.minute)}';
  if (d.year == n.year && d.month == n.month && d.day == n.day) return hm;
  if (d.year == n.year) return '${d.month}月${d.day}日 $hm';
  return '${d.year}年${d.month}月${d.day}日 $hm';
}

/// 只用系统浏览器打开 http、https、mailto 链接，别的一律不开。
Future<void> openExternal(BuildContext context, String? href) async {
  final uri = href == null ? null : Uri.tryParse(href);
  if (uri == null || !const {'http', 'https', 'mailto'}.contains(uri.scheme.toLowerCase())) return;
  final ok = await launchUrl(uri, mode: LaunchMode.externalApplication);
  if (!ok && context.mounted) showError(context, '打不开链接：$href');
}

void showError(BuildContext context, String text) {
  ScaffoldMessenger.of(context)
    ..hideCurrentSnackBar()
    ..showSnackBar(SnackBar(content: Text(text)));
}

/// 员工和系统写的文字按 Markdown 显示（和网页控制台 F-3 一样）：不执行 HTML，链接只开 http、https、mailto，
/// 不加载图片（只显示图片说明），表格在气泡里横向滚动。
class MessageMarkdown extends StatelessWidget {
  const MessageMarkdown(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return MarkdownBody(
      data: text,
      selectable: true,
      softLineBreak: true,
      onTapLink: (_, href, _) => openExternal(context, href),
      imageBuilder: (uri, title, alt) => Text('[图片：${alt ?? title ?? uri}]', style: theme.textTheme.bodySmall),
      styleSheet: MarkdownStyleSheet.fromTheme(theme).copyWith(
        tableColumnWidth: const IntrinsicColumnWidth(),
        code: theme.textTheme.bodyMedium?.copyWith(
          fontFamily: 'monospace',
          backgroundColor: theme.colorScheme.surfaceContainerHighest,
        ),
        codeblockDecoration: BoxDecoration(
          color: theme.colorScheme.surfaceContainerHighest,
          borderRadius: BorderRadius.circular(6),
        ),
      ),
    );
  }
}

/// 空列表时的一句话。
class EmptyHint extends StatelessWidget {
  const EmptyHint(this.text, {super.key});

  final String text;

  @override
  Widget build(BuildContext context) => Center(
    child: Padding(
      padding: const EdgeInsets.all(24),
      child: Text(
        text,
        textAlign: TextAlign.center,
        style: TextStyle(color: Theme.of(context).hintColor),
      ),
    ),
  );
}
