// @ts-check
// 员工回复的 Markdown 渲染（backlog F-3）。
//
// 先把文字解析成块和行内元素（纯函数，`web/test` 里有测试），再用 `h()` 生成 DOM 节点。
// 全程不拼 HTML：文字都走 textContent，写在文字里的 HTML 标签原样显示，不会被执行。
// 链接只认 http、https 和 mailto，其他协议（javascript: 之类）按原文显示。
// 支持：标题、段落、换行、无序和有序列表（可嵌套、可带复选框）、引用、代码块、表格、分隔线，
// 行内的粗体、斜体、删除线、行内代码、链接和裸网址。不支持的写法按原文显示。

import { h } from './dom.js';

/**
 * @typedef {{ t: 'text', v: string }
 *   | { t: 'code', v: string }
 *   | { t: 'br' }
 *   | { t: 'strong' | 'em' | 'del', c: Inline[] }
 *   | { t: 'link', href: string, c: Inline[] }} Inline
 */

/** @typedef {'left' | 'center' | 'right' | null} Align */

/**
 * @typedef {{ type: 'paragraph', text: string }
 *   | { type: 'heading', level: number, text: string }
 *   | { type: 'code', lang: string, text: string }
 *   | { type: 'hr' }
 *   | { type: 'quote', children: Block[] }
 *   | { type: 'list', ordered: boolean, start: number, items: ListItem[] }
 *   | { type: 'table', align: Align[], header: string[], rows: string[][] }} Block
 */

/** @typedef {{ checked: boolean | null, children: Block[] }} ListItem */

const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([^`\s]*).*$/;
const HEADING = /^ {0,3}(#{1,6})(?:\s+(.*?))?\s*#*\s*$/;
const HR = /^ {0,3}([-*_])(?:\s*\1){2,}\s*$/;
const QUOTE = /^ {0,3}>/;
const LIST = /^( {0,3})([-*+]|(\d{1,9})[.)])(?: +(.*)|$)/;
const TASK = /^\[([ xX])\]\s+/;

/**
 * 把 Markdown 文字解析成块。
 * @param {string} src
 * @returns {Block[]}
 */
export function parseBlocks(src) {
  return blocks(src.replace(/\r\n?/g, '\n').split('\n'));
}

/**
 * @param {string[]} lines
 * @returns {Block[]}
 */
function blocks(lines) {
  /** @type {Block[]} */
  const out = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (!line.trim()) {
      i++;
      continue;
    }
    const fence = FENCE.exec(line);
    if (fence) {
      const mark = fence[1];
      const close = new RegExp(`^ {0,3}${mark[0] === '`' ? '`' : '~'}{${mark.length},}\\s*$`);
      /** @type {string[]} */
      const body = [];
      i++;
      while (i < lines.length && !close.test(lines[i])) body.push(lines[i++]);
      i++; // 跳过收尾的围栏；没有收尾时代码块一直到文末
      out.push({ type: 'code', lang: fence[2], text: body.join('\n') });
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      out.push({ type: 'heading', level: heading[1].length, text: heading[2] ?? '' });
      i++;
      continue;
    }
    if (HR.test(line)) {
      out.push({ type: 'hr' });
      i++;
      continue;
    }
    if (isTableStart(lines, i)) {
      const header = splitRow(lines[i]);
      const align = splitRow(lines[i + 1]).map(alignOf);
      /** @type {string[][]} */
      const rows = [];
      i += 2;
      while (i < lines.length && lines[i].includes('|') && lines[i].trim() && !startsBlock(lines[i])) {
        const cells = splitRow(lines[i++]);
        rows.push(header.map((_, k) => cells[k] ?? ''));
      }
      out.push({ type: 'table', align, header, rows });
      continue;
    }
    if (QUOTE.test(line)) {
      /** @type {string[]} */
      const inner = [];
      while (i < lines.length && QUOTE.test(lines[i])) inner.push(lines[i++].replace(/^ {0,3}> ?/, ''));
      out.push({ type: 'quote', children: blocks(inner) });
      continue;
    }
    if (LIST.test(line)) {
      i = list(lines, i, out);
      continue;
    }
    /** @type {string[]} */
    const para = [line.trim()];
    i++;
    while (i < lines.length && lines[i].trim() && !startsBlock(lines[i]) && !isTableStart(lines, i)) {
      para.push(lines[i++].trim());
    }
    out.push({ type: 'paragraph', text: para.join('\n') });
  }
  return out;
}

/**
 * 从第 i 行起读一个列表，追加到 out，返回列表之后的行号。
 * @param {string[]} lines
 * @param {number} i
 * @param {Block[]} out
 */
function list(lines, i, out) {
  const first = /** @type {RegExpExecArray} */ (LIST.exec(lines[i]));
  const ordered = first[3] !== undefined;
  /** @type {ListItem[]} */
  const items = [];
  while (i < lines.length) {
    const m = LIST.exec(lines[i]);
    if (!m || (m[3] !== undefined) !== ordered) break;
    const rest = m[4] ?? '';
    // 内容缩进：标记后面的空格超过 4 个时按 1 个算（CommonMark 的规则）。
    const spaces = lines[i].length - m[1].length - m[2].length - rest.length;
    const indent = m[1].length + m[2].length + (spaces >= 1 && spaces <= 4 ? spaces : 1);
    /** @type {string[]} */
    const body = [rest];
    i++;
    while (i < lines.length) {
      const line = lines[i];
      if (!line.trim()) {
        // 空行之后还有缩进够的内容才算这一项的；否则这一项（也可能整个列表）到此为止。
        let j = i + 1;
        while (j < lines.length && !lines[j].trim()) j++;
        if (j < lines.length && leading(lines[j]) >= indent) {
          while (i < j) body.push(lines[i++].slice(indent));
          continue;
        }
        break;
      }
      if (leading(line) >= indent) {
        body.push(line.slice(indent));
        i++;
        continue;
      }
      if (LIST.test(line) || startsBlock(line)) break;
      body.push(line.trim()); // 没缩进的续行接在这一项的段落后面
      i++;
    }
    let checked = null;
    const task = TASK.exec(body[0]);
    if (task) {
      checked = task[1] !== ' ';
      body[0] = body[0].slice(task[0].length);
    }
    items.push({ checked, children: blocks(body) });
    // 列表项之间隔着空行时，下一行如果还是同一种列表就接着读。
    let j = i;
    while (j < lines.length && !lines[j].trim()) j++;
    const next = j < lines.length ? LIST.exec(lines[j]) : null;
    if (!next || (next[3] !== undefined) !== ordered) break;
    i = j;
  }
  out.push({ type: 'list', ordered, start: ordered ? Number(first[3]) : 1, items });
  return i;
}

/** @param {string} line */
function leading(line) {
  return line.length - line.trimStart().length;
}

/** 这一行是不是会打断段落的块的开头。 @param {string} line */
function startsBlock(line) {
  return FENCE.test(line) || HEADING.test(line) || HR.test(line) || QUOTE.test(line) || LIST.test(line);
}

/**
 * @param {string[]} lines
 * @param {number} i
 */
function isTableStart(lines, i) {
  if (!lines[i].includes('|') || i + 1 >= lines.length) return false;
  const delim = lines[i + 1];
  if (!delim.includes('-')) return false;
  const cells = splitRow(delim);
  return cells.length > 0 && cells.every((c) => /^:?-+:?$/.test(c)) && cells.length === splitRow(lines[i]).length;
}

/**
 * 按 `|` 切开一行表格，去掉两头的竖线。反引号里和转义的竖线不切。
 * @param {string} line
 * @returns {string[]}
 */
export function splitRow(line) {
  let s = line.trim();
  if (s.startsWith('|')) s = s.slice(1);
  if (s.endsWith('|') && !s.endsWith('\\|')) s = s.slice(0, -1);
  /** @type {string[]} */
  const cells = [];
  let cell = '';
  let inCode = false;
  for (let k = 0; k < s.length; k++) {
    const c = s[k];
    if (c === '\\' && s[k + 1] === '|') {
      cell += '|';
      k++;
    } else if (c === '`') {
      inCode = !inCode;
      cell += c;
    } else if (c === '|' && !inCode) {
      cells.push(cell.trim());
      cell = '';
    } else {
      cell += c;
    }
  }
  cells.push(cell.trim());
  return cells;
}

/** @param {string} cell @returns {Align} */
function alignOf(cell) {
  const left = cell.startsWith(':');
  const right = cell.endsWith(':');
  return left && right ? 'center' : right ? 'right' : left ? 'left' : null;
}

const ESCAPABLE = /[\\`*_{}[\]()#+\-.!|~>]/;
const URL_START = /^https?:\/\/[^\s<>()（）「」《》]+/;
/** 网址末尾的这些标点算句子的，不算网址的。 */
const URL_TRAIL = /[.,;:!?'"。，；：！？、）]+$/;

/**
 * 解析一段行内文字。
 * @param {string} src
 * @returns {Inline[]}
 */
export function parseInline(src) {
  /** @type {Inline[]} */
  const out = [];
  let text = '';
  const flush = () => {
    if (text) out.push({ t: 'text', v: text });
    text = '';
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === '\\' && i + 1 < src.length && ESCAPABLE.test(src[i + 1])) {
      text += src[i + 1];
      i += 2;
      continue;
    }
    if (c === '\n') {
      flush();
      out.push({ t: 'br' });
      i++;
      continue;
    }
    if (c === '`') {
      let n = 1;
      while (src[i + n] === '`') n++;
      const run = '`'.repeat(n);
      let j = src.indexOf(run, i + n);
      while (j !== -1 && src[j + n] === '`') j = src.indexOf(run, j + n + 1);
      if (j === -1) {
        text += run;
        i += n;
        continue;
      }
      let code = src.slice(i + n, j).replace(/\n/g, ' ');
      if (code.length > 2 && code.startsWith(' ') && code.endsWith(' ')) code = code.slice(1, -1);
      flush();
      out.push({ t: 'code', v: code });
      i = j + n;
      continue;
    }
    const pair = c === '*' && src[i + 1] === '*' ? '**' : c === '~' && src[i + 1] === '~' ? '~~' : null;
    if (pair) {
      const j = src.indexOf(pair, i + 2);
      if (j > i + 2 && src[i + 2] !== ' ' && src[j - 1] !== ' ') {
        flush();
        out.push({ t: pair === '**' ? 'strong' : 'del', c: parseInline(src.slice(i + 2, j)) });
        i = j + 2;
        continue;
      }
      text += pair;
      i += 2;
      continue;
    }
    if (c === '*' || (c === '_' && !isWordChar(src[i - 1]))) {
      const j = findEmClose(src, i, c);
      if (j !== -1) {
        flush();
        out.push({ t: 'em', c: parseInline(src.slice(i + 1, j)) });
        i = j + 1;
        continue;
      }
    }
    if (c === '[') {
      const link = parseLink(src, i);
      if (link) {
        flush();
        out.push(link.node);
        i = link.end;
        continue;
      }
    }
    if ((c === 'h' || c === 'H') && src.slice(i, i + 4).toLowerCase() === 'http' && !isWordChar(src[i - 1])) {
      const m = URL_START.exec(src.slice(i));
      if (m) {
        const url = m[0].replace(URL_TRAIL, '');
        if (url.length > 'https://'.length) {
          flush();
          out.push({ t: 'link', href: url, c: [{ t: 'text', v: url }] });
          i += url.length;
          continue;
        }
      }
    }
    text += c;
    i++;
  }
  flush();
  return out;
}

/** @param {string | undefined} c */
function isWordChar(c) {
  return c !== undefined && /[\p{L}\p{N}]/u.test(c);
}

/**
 * 找单个 `*` 或 `_` 的收尾位置；找不到返回 -1。
 * 开头后面和收尾前面不能是空格；`_` 的收尾后面不能紧跟字母数字（免得 snake_case 被当成斜体）。
 * @param {string} src
 * @param {number} i
 * @param {string} c
 */
function findEmClose(src, i, c) {
  if (!src[i + 1] || src[i + 1] === ' ' || src[i + 1] === c) return -1;
  for (let j = i + 2; j < src.length; j++) {
    if (src[j] === '\n') return -1;
    if (src[j] !== c || src[j - 1] === ' ' || src[j - 1] === '\\') continue;
    if (c === '*' && src[j + 1] === '*') {
      j++;
      continue;
    }
    if (c === '_' && isWordChar(src[j + 1])) continue;
    return j;
  }
  return -1;
}

/**
 * 解析 `[文字](网址)`。网址不是 http、https、mailto 时返回 null，按原文显示。
 * @param {string} src
 * @param {number} i
 * @returns {{ node: Inline, end: number } | null}
 */
function parseLink(src, i) {
  const close = src.indexOf('](', i + 1);
  if (close === -1 || src.slice(i + 1, close).includes('\n')) return null;
  const end = src.indexOf(')', close + 2);
  if (end === -1) return null;
  const href = src.slice(close + 2, end).trim().split(/\s+/)[0] ?? '';
  if (!/^(https?:\/\/|mailto:)/i.test(href)) return null;
  return { node: { t: 'link', href, c: parseInline(src.slice(i + 1, close)) }, end: end + 1 };
}

// ---------- 生成 DOM ----------

/**
 * 把 Markdown 文字渲染成一个 `div.md`。
 * @param {string} text
 * @returns {HTMLElement}
 */
export function renderMarkdown(text) {
  return h('div.md', null, ...parseBlocks(text).map(renderBlock));
}

/**
 * @param {Block} b
 * @returns {HTMLElement}
 */
function renderBlock(b) {
  switch (b.type) {
    case 'paragraph':
      return h('p', null, ...renderInline(parseInline(b.text)));
    case 'heading':
      return h(`h${b.level}`, null, ...renderInline(parseInline(b.text)));
    case 'code':
      return h('pre', null, h('code', null, b.text));
    case 'hr':
      return h('hr');
    case 'quote':
      return h('blockquote', null, ...b.children.map(renderBlock));
    case 'list':
      return h(
        b.ordered ? 'ol' : 'ul',
        { start: b.ordered && b.start !== 1 ? b.start : undefined },
        ...b.items.map((item) => {
          // 只有一段文字的列表项直接放文字，不套段落，行距紧凑些。
          const only = item.children.length === 1 && item.children[0].type === 'paragraph' ? item.children[0] : null;
          const body = only ? renderInline(parseInline(only.text)) : item.children.map(renderBlock);
          return h(
            item.checked === null ? 'li' : 'li.task',
            null,
            item.checked === null ? null : h('input', { type: 'checkbox', checked: item.checked, disabled: true }),
            ...body,
          );
        }),
      );
    case 'table': {
      /** @param {Align} a */
      const cls = (a) => (a ? `.align-${a}` : '');
      return h(
        'div.table-wrap',
        null,
        h(
          'table.md-table',
          null,
          h('thead', null, h('tr', null, ...b.header.map((c, k) => h('th' + cls(b.align[k]), null, ...renderInline(parseInline(c)))))),
          h(
            'tbody',
            null,
            ...b.rows.map((row) => h('tr', null, ...row.map((c, k) => h('td' + cls(b.align[k]), null, ...renderInline(parseInline(c)))))),
          ),
        ),
      );
    }
  }
}

/**
 * @param {Inline[]} nodes
 * @returns {(Node | string)[]}
 */
function renderInline(nodes) {
  return nodes.map((n) => {
    switch (n.t) {
      case 'text':
        return n.v;
      case 'code':
        return h('code', null, n.v);
      case 'br':
        return h('br');
      case 'strong':
      case 'em':
      case 'del':
        return h(n.t, null, ...renderInline(n.c));
      case 'link':
        return h('a', { href: n.href, target: '_blank', rel: 'noopener noreferrer' }, ...renderInline(n.c));
    }
  });
}
