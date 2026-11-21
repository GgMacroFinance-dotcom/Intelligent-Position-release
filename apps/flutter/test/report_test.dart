// 报告链接：正文和行内代码里提到的 reports/<文件名>.md 变成能点的链接，代码块里的不变。

import 'package:flutter_test/flutter_test.dart';
import 'package:ip_app/src/ui/report.dart';
import 'package:markdown/markdown.dart' as md;

String html(String text) => md.markdownToHtml(
  text,
  extensionSet: md.ExtensionSet.gitHubFlavored,
  inlineSyntaxes: [ReportCodeRefSyntax(), ReportRefSyntax()],
);

void main() {
  test('正文里的报告路径变成链接，中文标点断开', () {
    expect(
      html('报告写在 reports/2026-11-21-cpi.md。请看。'),
      contains('<a href="ipreport:reports/2026-11-21-cpi.md">reports/2026-11-21-cpi.md</a>。'),
    );
    expect(
      html('见reports/a.md，以及 reports/b.md'),
      allOf(contains('ipreport:reports/a.md'), contains('ipreport:reports/b.md')),
    );
  });

  test('行内代码里只有一个报告路径时也是链接，显示成普通链接', () {
    expect(
      html('路径：`reports/2026-11-21-cpi.md`'),
      contains('<a href="ipreport:reports/2026-11-21-cpi.md">reports/2026-11-21-cpi.md</a>'),
    );
    expect(html('`cat reports/a.md`'), isNot(contains('ipreport:')));
  });

  test('不算报告的：代码块里的、子目录、前面粘着字母的、不是 .md 的', () {
    expect(html('```\nreports/a.md\n```'), isNot(contains('ipreport:')));
    expect(html('reports/sub/a.md'), isNot(contains('ipreport:')));
    expect(html('myreports/a.md'), isNot(contains('ipreport:')));
    expect(html('reports/a.mdx 和 reports/a.txt'), isNot(contains('ipreport:')));
  });

  test('链接指向报告：ipreport: 链接和相对路径都算，网址不算', () {
    expect(reportPathOf('ipreport:reports/a.md'), 'reports/a.md');
    expect(reportPathOf('reports/a.md'), 'reports/a.md');
    expect(reportPathOf('https://example.com/reports/a.md'), isNull);
    expect(reportPathOf('reports/../secret.md'), isNull);
    expect(reportPathOf(null), isNull);
  });
}
