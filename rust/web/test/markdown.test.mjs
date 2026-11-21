// Markdown 解析的单元测试（backlog F-3）。在 rust/web 下运行：node --test test/*.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';

import { parseBlocks, parseInline, splitReportRefs, splitRow } from '../src/markdown.js';

test('标题、段落和换行', () => {
  assert.deepEqual(parseBlocks('# 结论\n第一行\n第二行\n\n另一段'), [
    { type: 'heading', level: 1, text: '结论' },
    { type: 'paragraph', text: '第一行\n第二行' },
    { type: 'paragraph', text: '另一段' },
  ]);
  assert.deepEqual(parseInline('a\nb'), [{ t: 'text', v: 'a' }, { t: 'br' }, { t: 'text', v: 'b' }]);
  // 没有空格的 # 不是标题
  assert.equal(parseBlocks('#话题')[0].type, 'paragraph');
});

test('员工报告里常见的表格', () => {
  const md = [
    '| 指标 | 数值 | 观测日期 |',
    '| --- | ---: | :---: |',
    '| CPI 同比 | 2.9% | 2026-10-31 |',
    '| 核心 CPI | 3.1% |',
    '',
    '解读',
  ].join('\n');
  const [table, para] = parseBlocks(md);
  assert.deepEqual(table, {
    type: 'table',
    align: [null, 'right', 'center'],
    header: ['指标', '数值', '观测日期'],
    rows: [
      ['CPI 同比', '2.9%', '2026-10-31'],
      ['核心 CPI', '3.1%', ''],
    ],
  });
  assert.deepEqual(para, { type: 'paragraph', text: '解读' });
});

test('分隔行列数对不上时不当表格', () => {
  assert.equal(parseBlocks('a | b\n| --- |')[0].type, 'paragraph');
});

test('表格单元格里转义的竖线和代码里的竖线不切开', () => {
  assert.deepEqual(splitRow('| a \\| b | `x|y` |'), ['a | b', '`x|y`']);
});

test('列表：有序起始号、嵌套、复选框、续行', () => {
  const [ol] = parseBlocks('3. 第三\n4. 第四');
  assert.equal(ol.type, 'list');
  assert.equal(ol.type === 'list' && ol.start, 3);

  const [ul] = parseBlocks('- 外层\n  - 内层\n- [x] 已完成\n- [ ] 没完成\n续行');
  assert.ok(ul.type === 'list');
  assert.equal(ul.items.length, 3);
  assert.deepEqual(ul.items[0].children, [
    { type: 'paragraph', text: '外层' },
    { type: 'list', ordered: false, start: 1, items: [{ checked: null, children: [{ type: 'paragraph', text: '内层' }] }] },
  ]);
  assert.equal(ul.items[1].checked, true);
  assert.deepEqual(ul.items[2], { checked: false, children: [{ type: 'paragraph', text: '没完成\n续行' }] });
});

test('列表项之间有空行仍是同一个列表，换了种类就分开', () => {
  const out = parseBlocks('- a\n\n- b\n\n1. c');
  assert.equal(out.length, 2);
  assert.ok(out[0].type === 'list' && out[0].items.length === 2);
  assert.ok(out[1].type === 'list' && out[1].ordered);
});

test('代码块、引用、分隔线', () => {
  assert.deepEqual(parseBlocks('```sql\nselect 1;\n| 不是表格 |\n```\n> 引用\n> 第二行\n\n---'), [
    { type: 'code', lang: 'sql', text: 'select 1;\n| 不是表格 |' },
    { type: 'quote', children: [{ type: 'paragraph', text: '引用\n第二行' }] },
    { type: 'hr' },
  ]);
  // 没有收尾的代码块一直到文末（流式输出时常见）
  assert.deepEqual(parseBlocks('```\na'), [{ type: 'code', lang: '', text: 'a' }]);
});

test('行内：粗体、斜体、删除线、代码', () => {
  assert.deepEqual(parseInline('**结论**：*注意* ~~旧~~ `DGS10`'), [
    { t: 'strong', c: [{ t: 'text', v: '结论' }] },
    { t: 'text', v: '：' },
    { t: 'em', c: [{ t: 'text', v: '注意' }] },
    { t: 'text', v: ' ' },
    { t: 'del', c: [{ t: 'text', v: '旧' }] },
    { t: 'text', v: ' ' },
    { t: 'code', v: 'DGS10' },
  ]);
});

test('乘号、snake_case 和没收尾的星号按原文显示', () => {
  assert.deepEqual(parseInline('2 * 3 * 4'), [{ t: 'text', v: '2 * 3 * 4' }]);
  assert.deepEqual(parseInline('memory_search 和 get_fred_series'), [{ t: 'text', v: 'memory_search 和 get_fred_series' }]);
  assert.deepEqual(parseInline('**没收尾'), [{ t: 'text', v: '**没收尾' }]);
});

test('链接只认 http、https、mailto', () => {
  assert.deepEqual(parseInline('[FRED](https://fred.stlouisfed.org)'), [
    { t: 'link', href: 'https://fred.stlouisfed.org', c: [{ t: 'text', v: 'FRED' }] },
  ]);
  assert.deepEqual(parseInline('[点我](javascript:alert(1))'), [{ t: 'text', v: '[点我](javascript:alert(1))' }]);
  assert.deepEqual(parseInline('[报告](reports/2026-11-16-cpi.md)'), [{ t: 'text', v: '[报告](reports/2026-11-16-cpi.md)' }]);
});

test('裸网址去掉句末标点', () => {
  assert.deepEqual(parseInline('来源：https://www.stats.gov.cn。'), [
    { t: 'text', v: '来源：' },
    { t: 'link', href: 'https://www.stats.gov.cn', c: [{ t: 'text', v: 'https://www.stats.gov.cn' }] },
    { t: 'text', v: '。' },
  ]);
});

test('HTML 标签按文字保留', () => {
  assert.deepEqual(parseInline('<img src=x onerror=alert(1)>'), [{ t: 'text', v: '<img src=x onerror=alert(1)>' }]);
});

test('文字里提到的报告路径（backlog F-4）', () => {
  assert.deepEqual(splitReportRefs('报告已写好：reports/2026-11-16-cpi.md。请看'), [
    { t: 'text', v: '报告已写好：' },
    { t: 'report', path: 'reports/2026-11-16-cpi.md' },
    { t: 'text', v: '。请看' },
  ]);
  // 中文文件名、紧跟中文
  assert.deepEqual(splitReportRefs('见reports/美国融资环境.md里的表'), [
    { t: 'text', v: '见' },
    { t: 'report', path: 'reports/美国融资环境.md' },
    { t: 'text', v: '里的表' },
  ]);
  for (const plain of [
    '/tmp/ops-demo/reports/a.md', // 别的目录下的
    'reports/sub/a.md', // 子目录
    'reports/../ip.db.md',
    'reports/.hidden.md',
    'reports/a.mdx',
    'reports/a.md.bak',
    'myreports/a.md',
    'reports/',
  ]) {
    assert.deepEqual(splitReportRefs(plain), [{ t: 'text', v: plain }], plain);
  }
  assert.deepEqual(
    splitReportRefs('reports/a.md 和 reports/b.md').filter((s) => s.t === 'report').map((s) => s.t === 'report' && s.path),
    ['reports/a.md', 'reports/b.md'],
  );
});
