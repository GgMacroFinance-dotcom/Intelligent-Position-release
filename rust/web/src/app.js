// @ts-check
// 最小 Web 控制台：对话、岗位与员工；任务、审批、路由规则三页随后端功能逐步接上。

import { Client } from './client.js';
import { h, formatTime } from './dom.js';

/** @typedef {import('./types/Role').Role} Role */
/** @typedef {import('./types/Agent').Agent} Agent */
/** @typedef {import('./types/Message').Message} Message */
/** @typedef {import('./types/ThreadSummary').ThreadSummary} ThreadSummary */
/** @typedef {import('./types/SearchHit').SearchHit} SearchHit */
/** @typedef {import('./types/RuntimeEvent').RuntimeEvent} RuntimeEvent */
/** @typedef {import('./types/RouterSpec').RouterSpec} RouterSpec */
/** @typedef {import('./types/Rule').Rule} Rule */
/** @typedef {import('./types/RulesView').RulesView} RulesView */
/** @typedef {import('./types/RuleDraft').RuleDraft} RuleDraft */
/** @typedef {import('./types/Decision').Decision} Decision */
/** @typedef {import('./types/SendResult').SendResult} SendResult */
/** @typedef {import('./types/Task').Task} Task */
/** @typedef {import('./types/TaskStatus').TaskStatus} TaskStatus */
/** @typedef {import('./types/Approval').Approval} Approval */
/** @typedef {import('./types/SpendToday').SpendToday} SpendToday */
/** @typedef {import('./types/ScheduleView').ScheduleView} ScheduleView */
/** @typedef {import('./types/MemoryView').MemoryView} MemoryView */
/** @typedef {import('./types/MemoryLayer').MemoryLayer} MemoryLayer */

/** @type {Record<TaskStatus, string>} */
const STATUS_TEXT = { open: '待开始', in_progress: '进行中', blocked: '受阻', done: '已完成', cancelled: '已取消' };

/** 员工下拉框里“自动分派”的取值。 */
const AUTO = '__auto';

const TOKEN_KEY = 'ip.token';
const NAME_KEY = 'ip.name';

/** 读写浏览器存储；隐私模式下可能抛错，失败就当没有。 */
const store = {
  /** @param {string} k */
  get(k) {
    try {
      return localStorage.getItem(k);
    } catch {
      return null;
    }
  },
  /** @param {string} k @param {string} v */
  set(k, v) {
    try {
      localStorage.setItem(k, v);
    } catch {
      /* 忽略 */
    }
  },
};

// 地址栏里的 ?token= 记下后从地址栏去掉，避免令牌留在历史记录里。
const params = new URLSearchParams(location.search);
if (params.get('token')) {
  store.set(TOKEN_KEY, params.get('token') ?? '');
  history.replaceState(null, '', location.pathname + location.hash);
}
const token = store.get(TOKEN_KEY);
const wsUrl = `${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/ws${token ? `?token=${encodeURIComponent(token)}` : ''}`;
const client = new Client(wsUrl);

const state = {
  /** @type {Role[]} */ roles: [],
  /** @type {Agent[]} */ agents: [],
  /** @type {ThreadSummary[]} */ threads: [],
  /** @type {string | null} */ threadId: null,
  /** @type {Message[]} */ messages: [],
  /** 正在生成的文字，按员工 id。 @type {Map<string, string>} */ drafts: new Map(),
  /** 每个对话最近一次和哪个员工说话。 @type {Map<string, string>} */ threadAgent: new Map(),
  /** @type {string} */ agentForNew: '',
  /** @type {SearchHit[] | null} */ searchHits: null,
  /** @type {Map<string, string>} 对话最近一轮的状态说明 */ turnNote: new Map(),
  /** @type {RulesView | null} */ rules: null,
  /** @type {RuleDraft[]} */ ruleDrafts: [],
  /** 正在编辑的规则副本；null 表示只读。 @type {{ spec: RouterSpec, acceptedDraft: string | null, error: string | null } | null} */ ruleEdit: null,
  /** @type {Task[]} */ tasks: [],
  /** @type {Approval[]} */ approvals: [],
  /** @type {{ text: string, decision: Decision } | null} */ routeTry: null,
  /** 今天的模型费用。 @type {SpendToday | null} */ spend: null,
  /** @type {ScheduleView[]} */ schedules: [],
  /** “记忆”页正在看的岗位。 @type {string | null} */ memoryRole: null,
  /** @type {MemoryView | null} */ memory: null,
  /** 关掉了记忆的对话。 @type {string[]} */ memoryOff: [],
};

const PAGES = [
  { id: 'chat', title: '对话' },
  { id: 'roles', title: '岗位与员工' },
  { id: 'tasks', title: '任务' },
  { id: 'approvals', title: '审批' },
  { id: 'router', title: '路由规则' },
  { id: 'memory', title: '记忆' },
];

const root = /** @type {HTMLElement} */ (document.getElementById('app'));

function page() {
  const id = location.hash.replace(/^#\/?/, '').split('/')[0];
  return PAGES.some((p) => p.id === id) ? id : 'chat';
}

/** @param {string} id */
function agentName(id) {
  return state.agents.find((a) => a.id === id)?.name ?? id;
}

function render() {
  const current = page();
  root.replaceChildren(renderHeader(current), h('main', null, renderPage(current)));
  if (current === 'chat') scrollMessages();
}

/** @param {string} current */
function renderHeader(current) {
  return (
    h(
      'header.top',
      null,
      h('div.brand', null, 'intelligent-position'),
      h(
        'nav',
        null,
        ...PAGES.map((p) => {
          const pending = p.id === 'approvals' ? state.approvals.filter((a) => a.state.state === 'pending').length : 0;
          return h(
            'a' + (p.id === current ? '.active' : ''),
            { href: `#/${p.id}` },
            p.title,
            pending ? h('span.badge', null, pending) : null,
          );
        }),
      ),
      h('div.status.' + client.status, { title: '与服务的连接' }, { open: '已连接', connecting: '连接中', closed: '已断开' }[client.status]),
      h('button.ghost', { onclick: askToken, title: '设置访问令牌' }, '令牌'),
    )
  );
}

/** @param {string} id */
function renderPage(id) {
  switch (id) {
    case 'roles':
      return renderRoles();
    case 'tasks':
      return renderTasks();
    case 'approvals':
      return renderApprovals();
    case 'router':
      return renderRouter();
    case 'memory':
      return renderMemory();
    default:
      return renderChat();
  }
}

/** @param {string} title @param {string} text */
function placeholder(title, text) {
  return h('section.placeholder', null, h('h2', null, title), h('p', null, text), h('p.muted', null, '这一页还在开发中。'));
}

// ---------- 岗位与员工 ----------

/**
 * 提示词缓存命中率：命中缓存的输入 token 占全部输入 token 的比例。
 * 输入太短（低于服务商的最小可缓存长度）的调用也算在分母里。
 * @param {import('./types/Usage').Usage | undefined} u
 */
function cacheHit(u) {
  if (!u || !u.prompt_tokens) return '还没有调用';
  const rate = Math.round((100 * u.cached_tokens) / u.prompt_tokens);
  return `${rate}%（输入 ${u.prompt_tokens.toLocaleString('en-US')} token，其中命中 ${u.cached_tokens.toLocaleString('en-US')}）`;
}

/** 全部岗位加自动分派合计的缓存命中率，放在费用那一行后面。 @param {SpendToday} spend */
function cacheSummary(spend) {
  let input = spend.routing_tokens.prompt_tokens;
  let cached = spend.routing_tokens.cached_tokens;
  for (const u of Object.values(spend.tokens)) {
    input += u?.prompt_tokens ?? 0;
    cached += u?.cached_tokens ?? 0;
  }
  return input ? `提示词缓存命中 ${Math.round((100 * cached) / input)}%。` : '';
}

function renderRoles() {
  if (!state.roles.length) return h('section.placeholder', null, h('p.muted', null, '还没有岗位。在 ip.yaml 的 roles 里定义。'));
  const spend = state.spend;
  return h(
    'section.roles',
    null,
    spend &&
      h(
        'p.muted.spend',
        null,
        `今天（UTC ${spend.date}）模型费用合计 ${usd(Object.values(spend.roles).reduce((a, b) => a + (b ?? 0), 0) + spend.routing_usd)}`,
        spend.routing_usd > 0 ? `，其中自动分派 ${usd(spend.routing_usd)}` : '',
        '。没在端点 prices 里登记价格的模型不计费。',
        cacheSummary(spend),
      ),
    ...state.roles.map((role) => {
      const agents = state.agents.filter((a) => a.role === role.id);
      const spent = spend?.roles[role.id] ?? 0;
      const limit = role.budget_per_day_usd;
      const over = limit !== null && spent >= limit;
      return h(
        'article.card',
        null,
        h('h3', null, role.name ?? role.id, h('span.muted.mono', null, ` ${role.id}`)),
        h('p', null, role.description),
        h(
          'dl',
          null,
          h('dt', null, '模型'),
          h('dd.mono', null, role.model.default + (role.model.escalate ? ` → ${role.model.escalate}` : '')),
          h('dt', null, '今日费用'),
          h(
            over ? 'dd.over' : 'dd',
            null,
            limit === null ? usd(spent) : `${usd(spent)} / $${limit}`,
            over ? '（已用满，UTC 0 点后恢复）' : '',
          ),
          h('dt', null, '今日缓存命中'),
          h('dd', null, cacheHit(spend?.tokens[role.id])),
        ),
        h(
          'div.agents',
          null,
          agents.length ? null : h('span.muted', null, '这个岗位还没有员工'),
          ...agents.map((a) =>
            h('button', { onclick: () => startChat(a.id) }, `和 ${a.name} 对话`),
          ),
        ),
      );
    }),
  );
}

/** @param {number} n 美元 */
function usd(n) {
  return n > 0 && n < 0.01 ? `$${n.toFixed(4)}` : `$${n.toFixed(2)}`;
}

/** @param {string} agentId */
function startChat(agentId) {
  state.threadId = null;
  state.messages = [];
  state.agentForNew = agentId;
  composerEl = null;
  location.hash = '#/chat';
  render();
}

/**
 * 按 id 替换或插入。
 * @template {{ id: string }} T
 * @param {T[]} list
 * @param {T} item
 */
function upsert(list, item) {
  const i = list.findIndex((x) => x.id === item.id);
  if (i >= 0) list[i] = item;
  else list.unshift(item);
}

/** 只重绘顶栏（对话页正在输入时不打断）。 */
function renderHeaderOnly() {
  const header = root.querySelector('header.top');
  if (header) header.replaceWith(renderHeader(page()));
  else render();
}

// ---------- 任务 ----------

function renderTasks() {
  const title = /** @type {HTMLInputElement} */ (h('input', { placeholder: '新任务标题，例如：写 10 月 CPI 点评' }));
  const desc = /** @type {HTMLTextAreaElement} */ (h('textarea', { placeholder: '说明（可选）', rows: 2 }));
  const who = /** @type {HTMLSelectElement} */ (
    h(
      'select',
      null,
      h('option', { value: '' }, '暂不指定负责人'),
      ...state.agents.map((a) => h('option', { value: a.id }, `${a.name}（${a.role}）`)),
    )
  );
  const due = /** @type {HTMLInputElement} */ (h('input', { type: 'date' }));
  const create = async () => {
    if (!title.value.trim()) return title.focus();
    try {
      await client.request({
        type: 'create_task',
        title: title.value,
        description: desc.value || undefined,
        assignee: who.value || undefined,
        due_ms: due.value ? new Date(`${due.value}T23:59:59`).getTime() : undefined,
        author: store.get(NAME_KEY) ?? undefined,
      });
    } catch (e) {
      alert(`新建失败：${/** @type {Error} */ (e).message}`);
    }
  };
  const open = state.tasks.filter((t) => t.status !== 'done' && t.status !== 'cancelled');
  const closed = state.tasks.filter((t) => t.status === 'done' || t.status === 'cancelled');
  return h(
    'section.tasks',
    null,
    h('div.card', null, h('h3', null, '新任务'), title, desc, h('div.row', null, who, due, h('button.primary', { onclick: create }, '新建'))),
    h('div.card', null, h('h3', null, `未完成（${open.length}）`), open.length ? taskTable(open) : h('p.muted', null, '没有未完成的任务。')),
    closed.length ? h('div.card', null, h('h3', null, `已结束（${closed.length}）`), taskTable(closed)) : null,
    state.schedules.length ? h('div.card', null, h('h3', null, `定时任务（${state.schedules.length}）`), scheduleTable()) : null,
  );
}

function scheduleTable() {
  /** @param {string} id */
  const runNow = async (id) => {
    try {
      const r = /** @type {SendResult} */ (await client.request({ type: 'run_schedule', schedule_id: id }));
      state.schedules = await client.request({ type: 'list_schedules' });
      render();
      if (confirm('已发出。打开这段对话？')) openThread(r.message.thread_id);
    } catch (e) {
      alert(`运行失败：${/** @type {Error} */ (e).message}`);
    }
  };
  return h(
    'table',
    null,
    h(
      'thead',
      null,
      h('tr', null, h('th', null, '定时任务'), h('th', null, '时间'), h('th', null, '发给'), h('th', null, '上次运行'), h('th', null, '')),
    ),
    h(
      'tbody',
      null,
      ...state.schedules.map(({ spec, next_run_ms, last_run }) => {
        const to = spec.to ? roleName(spec.to.role) + (spec.to.agent ? ` · ${agentName(spec.to.agent)}` : '') : '按路由规则';
        const last = last_run;
        return h(
          'tr',
          null,
          h('td', null, h('div', null, spec.description || spec.id), h('div.muted', null, spec.text)),
          h(
            'td',
            null,
            h('div.mono', null, `${spec.cron}（UTC${spec.utc_offset}）`),
            h('div.muted', null, spec.enabled ? (next_run_ms ? `下次 ${formatTime(next_run_ms)}` : '不会再运行') : '已停用'),
          ),
          h('td', null, to),
          h(
            'td',
            null,
            last
              ? h(
                  last.ok ? 'div' : 'div.error',
                  null,
                  `${formatTime(last.at_ms)} ${last.ok ? '' : '失败：'}${last.detail}`,
                  ...(last.thread_id
                    ? [' · ', h('a', { href: '#/chat', onclick: () => openThread(/** @type {string} */ (last.thread_id)) }, '对话')]
                    : []),
                )
              : h('span.muted', null, '还没运行过'),
          ),
          h('td', null, h('button', { onclick: () => runNow(spec.id) }, '立即运行')),
        );
      }),
    ),
  );
}

/**
 * 任务的执行对话和派活对话链接。
 * @param {Task} t
 */
function threadLinks(t) {
  /** @type {(Node | string)[]} */
  const out = [];
  for (const [id, label] of /** @type {[string | null, string][]} */ ([
    [t.thread_id, '执行对话'],
    [t.origin_thread_id, '派活对话'],
  ])) {
    if (!id) continue;
    out.push(' · ', h('a', { href: '#/chat', onclick: () => openThread(id) }, label));
  }
  return out;
}

/** @param {Task[]} tasks */
function taskTable(tasks) {
  return h(
    'table',
    null,
    h('thead', null, h('tr', null, h('th', null, '任务'), h('th', null, '负责人'), h('th', null, '截止'), h('th', null, '状态'))),
    h(
      'tbody',
      null,
      ...tasks.map((t) =>
        h(
          'tr',
          null,
          h(
            'td',
            null,
            h('div', null, t.title),
            t.description ? h('div.muted', null, t.description) : null,
            h(
              'div.muted.small',
              null,
              `由 ${t.created_by.kind === 'human' ? t.created_by.name : t.created_by.kind === 'agent' ? agentName(t.created_by.agent_id) : '系统'} 创建`,
              ...threadLinks(t),
            ),
            t.result ? h('div.task-result', null, `结果：${t.result}`) : null,
          ),
          h('td', null, t.assignee ? agentName(t.assignee) : t.role ?? '—'),
          h('td', null, t.due_ms ? formatTime(t.due_ms) : '—'),
          h(
            'td',
            null,
            h(
              'select',
              {
                onchange: (/** @type {Event} */ e) =>
                  client
                    .request({
                      type: 'update_task',
                      task_id: t.id,
                      author: store.get(NAME_KEY) ?? undefined,
                      status: /** @type {TaskStatus} */ (/** @type {HTMLSelectElement} */ (e.target).value),
                    })
                    .catch((err) => alert(err.message)),
              },
              ...Object.entries(STATUS_TEXT).map(([v, label]) => h('option', { value: v, selected: v === t.status }, label)),
            ),
          ),
        ),
      ),
    ),
  );
}

// ---------- 审批 ----------

function renderApprovals() {
  const pending = state.approvals.filter((a) => a.state.state === 'pending');
  const done = state.approvals.filter((a) => a.state.state !== 'pending');
  return h(
    'section.approvals',
    null,
    h('h2', null, pending.length ? `待你决定（${pending.length}）` : '没有待决定的事'),
    ...pending.map(approvalCard),
    done.length ? h('h2', null, '已决定') : null,
    ...done.map(approvalCard),
  );
}

/** @param {Approval} a */
function approvalCard(a) {
  const st = a.state;
  const choose = async (/** @type {number} */ i) => {
    try {
      await client.request({ type: 'resolve_approval', approval_id: a.id, choice: i, author: store.get(NAME_KEY) ?? undefined });
      // 人工改派可能生成了新的规则草案。
      state.ruleDrafts = await client.request({ type: 'list_rule_drafts' });
      render();
    } catch (e) {
      alert(`操作失败：${/** @type {Error} */ (e).message}`);
    }
  };
  const subject = a.subject;
  return h(
    'article.card.approval' + (st.state === 'pending' ? '' : '.resolved'),
    null,
    h('h3', null, a.question),
    a.reason ? h('p.muted', null, a.reason) : null,
    subject.kind === 'route_message' || subject.kind === 'agent_question'
      ? h(
          'p',
          null,
          subject.kind === 'agent_question' ? `${agentName(subject.agent_id)} 提问 · ` : null,
          h('a', { href: '#/chat', onclick: () => openThread(subject.thread_id) }, '查看对话'),
        )
      : null,
    h(
      'div.options',
      null,
      ...a.options.map((o, i) => {
        const chosen = st.state === 'resolved' && st.choice === i;
        return h(
          'button' + (a.recommended === i ? '.recommended' : '') + (chosen ? '.chosen' : ''),
          { onclick: st.state === 'pending' ? () => choose(i) : undefined, disabled: st.state !== 'pending', title: o.consequence },
          h('strong', null, o.label),
          h('span.muted', null, o.consequence),
        );
      }),
    ),
    st.state === 'resolved' ? h('p.muted', null, `${st.by} 于 ${formatTime(st.at_ms)} 选了「${a.options[st.choice]?.label ?? st.choice}」`) : null,
    h('p.muted.small', null, `发起于 ${formatTime(a.created_at_ms)}`),
  );
}

// ---------- 路由规则 ----------

function renderRouter() {
  const view = state.rules;
  if (!view) return placeholder('路由规则', '加载中…');
  const spec = view.spec;
  const input = /** @type {HTMLInputElement} */ (h('input', { placeholder: '输入一句话，看看会分给谁', value: state.routeTry?.text ?? '' }));
  const tryIt = async () => {
    const text = input.value.trim();
    if (!text) return;
    /** @type {Decision} */
    const decision = await client.request({ type: 'route', text, author: store.get(NAME_KEY) ?? undefined });
    state.routeTry = { text, decision };
    render();
  };
  input.addEventListener('keydown', (e) => e.key === 'Enter' && tryIt());
  const d = state.routeTry?.decision;
  return h(
    'section.router',
    null,
    h(
      'div.card',
      null,
      h('h3', null, '试一试'),
      h('div.row', null, input, h('button.primary', { onclick: tryIt }, '路由')),
      d &&
        h(
          'p.decision' + (d.kind === 'routed' ? '' : '.ask'),
          null,
          d.kind === 'routed'
            ? `→ ${roleName(d.role)}${d.agent ? `（${agentName(d.agent)}）` : ''}${d.escalate ? '，用升级模型' : ''}。${d.reason}`
            : `→ 交给人决定${d.suggested ? `（推荐 ${roleName(d.suggested)}）` : ''}。${d.reason}`,
        ),
    ),
    state.ruleDrafts.length && !state.ruleEdit ? renderDrafts() : null,
    state.ruleEdit ? ruleEditor() : renderRules(spec, view.from_console),
  );
}

/** @param {string} id */
function roleName(id) {
  return state.roles.find((r) => r.id === id)?.name ?? id;
}

/** @param {Rule['to']} to */
function describeTarget(to) {
  return `→ ${roleName(to.role)}${to.agent ? `（${agentName(to.agent)}）` : ''}${to.escalate ? '，用升级模型' : ''}`;
}

/**
 * @param {RouterSpec} spec
 * @param {boolean} fromConsole
 */
function renderRules(spec, fromConsole) {
  return h(
    'div.card',
    null,
    h(
      'div.card-head',
      null,
      h('h3', null, `规则（${spec.rules.length} 条，按顺序匹配，第一条命中的生效）`),
      h('button', { onclick: () => startEdit(null) }, '编辑'),
    ),
    spec.rules.length ? null : h('p.muted', null, '还没有规则。'),
    h(
      'ol.rules',
      null,
      ...spec.rules.map((r) =>
        h(
          'li',
          null,
          h('div', null, h('strong', null, r.id), r.description ? h('span.muted', null, ` ${r.description}`) : null),
          h('div.mono', null, describeCondition(r)),
          h('div', null, describeTarget(r.to)),
        ),
      ),
    ),
    spec.semantic
      ? h(
          'p',
          null,
          `规则都没命中时：先用 ${spec.semantic.model} 按岗位职责做语义匹配，把握不低于 ${spec.semantic.min_confidence} 就直接分派，否则把它的判断作为推荐交给兜底。`,
        )
      : h('p.muted', null, '没有开启语义匹配。'),
    h('p', null, '兜底：', spec.fallback === 'ask' ? '交给人决定' : `交给 ${roleName(spec.fallback.role)}`),
    h('p.muted', null, fromConsole ? '当前规则是在控制台保存的，重启后仍然生效（优先于 ip.yaml）。' : '当前规则来自 ip.yaml。在这里保存后，以控制台保存的为准。'),
  );
}

function renderDrafts() {
  return h(
    'div.card.drafts',
    null,
    h('h3', null, `规则草案（${state.ruleDrafts.length}）`),
    h('p.muted', null, '你在审批卡片上给消息选岗位时，系统按消息里的关键词起草规则。草案不会自动生效，采纳前可以修改。'),
    ...state.ruleDrafts.map((dr) =>
      h(
        'div.draft',
        null,
        h('div', null, h('strong', null, dr.rule.id), h('span.muted', null, ` ${dr.source}`)),
        h('div.mono', null, describeCondition(dr.rule)),
        h('div', null, describeTarget(dr.rule.to)),
        h(
          'div.row',
          null,
          h('button.primary', { onclick: () => startEdit(dr) }, '采纳（先编辑）'),
          h(
            'button',
            {
              onclick: async () => {
                state.ruleDrafts = await client.request({ type: 'dismiss_rule_draft', draft_id: dr.id });
                render();
              },
            },
            '忽略',
          ),
        ),
      ),
    ),
  );
}

/** @param {RuleDraft | null} draft */
function startEdit(draft) {
  if (!state.rules) return;
  /** @type {RouterSpec} */
  const spec = JSON.parse(JSON.stringify(state.rules.spec));
  if (draft) spec.rules.push(JSON.parse(JSON.stringify(draft.rule)));
  state.ruleEdit = { spec, acceptedDraft: draft?.id ?? null, error: null };
  editorEl = null;
  render();
}

/** 编辑器的节点跨重绘保留，事件推送时不丢正在输入的内容；增删、移动规则时才重建。 @type {HTMLElement | null} */
let editorEl = null;

/** @param {string} v */
function splitList(v) {
  return v
    .split(/[,，、\n]/)
    .map((x) => x.trim())
    .filter(Boolean);
}

function ruleEditor() {
  const edit = state.ruleEdit;
  if (!edit) return null;
  if (editorEl) return editorEl;
  const spec = edit.spec;
  const rebuild = () => {
    editorEl = null;
    render();
  };
  /**
   * @param {string} value
   * @param {(v: string) => void} set
   * @param {string} [placeholder]
   */
  const field = (value, set, placeholder) => {
    const el = /** @type {HTMLInputElement} */ (h('input', { value, placeholder: placeholder ?? '' }));
    el.addEventListener('input', () => set(el.value));
    return el;
  };
  /** @param {Rule} r */
  const roleSelect = (r) => {
    const el = /** @type {HTMLSelectElement} */ (
      h('select', null, ...state.roles.map((ro) => h('option', { value: ro.id, selected: ro.id === r.to.role }, ro.name ?? ro.id)))
    );
    el.addEventListener('change', () => {
      r.to.role = el.value;
      delete r.to.agent;
      rebuild();
    });
    return el;
  };
  /** @param {Rule} r */
  const agentSelect = (r) => {
    const el = /** @type {HTMLSelectElement} */ (
      h(
        'select',
        null,
        h('option', { value: '' }, '最空闲的员工'),
        ...state.agents.filter((a) => a.role === r.to.role).map((a) => h('option', { value: a.id, selected: a.id === r.to.agent }, a.name)),
      )
    );
    el.addEventListener('change', () => {
      if (el.value) r.to.agent = el.value;
      else delete r.to.agent;
    });
    return el;
  };
  /** @param {Rule} r */
  const escalateBox = (r) => {
    const el = /** @type {HTMLInputElement} */ (h('input', { type: 'checkbox', checked: r.to.escalate }));
    el.addEventListener('change', () => (r.to.escalate = el.checked));
    return h('label.check', null, el, ' 用升级模型');
  };
  /** @param {number} i @param {number} delta */
  const move = (i, delta) => {
    const j = i + delta;
    if (j < 0 || j >= spec.rules.length) return;
    [spec.rules[i], spec.rules[j]] = [spec.rules[j], spec.rules[i]];
    rebuild();
  };
  const fallback = /** @type {HTMLSelectElement} */ (
    h(
      'select',
      null,
      h('option', { value: '', selected: spec.fallback === 'ask' }, '交给人决定'),
      ...state.roles.map((ro) =>
        h('option', { value: ro.id, selected: spec.fallback !== 'ask' && spec.fallback.role === ro.id }, `交给 ${ro.name ?? ro.id}`),
      ),
    )
  );
  fallback.addEventListener('change', () => (spec.fallback = fallback.value ? { role: fallback.value } : 'ask'));
  const semanticOn = /** @type {HTMLInputElement} */ (h('input', { type: 'checkbox', checked: !!spec.semantic }));
  semanticOn.addEventListener('change', () => {
    spec.semantic = semanticOn.checked ? { model: state.roles[0]?.model.default ?? '', min_confidence: 0.7 } : undefined;
    if (!spec.semantic) delete spec.semantic;
    rebuild();
  });
  const sem = spec.semantic;
  const confidence = /** @type {HTMLInputElement} */ (
    h('input', { type: 'number', min: '0', max: '1', step: '0.05', value: String(sem?.min_confidence ?? 0.7) })
  );
  confidence.addEventListener('input', () => sem && (sem.min_confidence = Number(confidence.value)));
  const semanticRow = h(
    'div.row',
    null,
    h('label.check', null, semanticOn, ' 规则都没命中时做语义匹配'),
    sem ? h('label', null, '模型 ', field(sem.model, (v) => (sem.model = v.trim()), '端点/模型')) : null,
    sem ? h('label', null, '把握至少 ', confidence) : null,
  );
  const errorEl = h('p.error', null, edit.error ?? '');
  const save = async () => {
    try {
      const saved = await client.request({ type: 'save_rules', spec, accepted_draft: edit.acceptedDraft ?? undefined });
      state.rules = { spec: saved, from_console: true };
      state.ruleDrafts = await client.request({ type: 'list_rule_drafts' });
      state.ruleEdit = null;
      editorEl = null;
      render();
    } catch (e) {
      edit.error = `没有保存：${/** @type {Error} */ (e).message}`;
      errorEl.textContent = edit.error;
    }
  };
  editorEl = h(
    'div.card.rule-editor',
    null,
    h('h3', null, edit.acceptedDraft ? '采纳草案：改好后保存' : '编辑规则'),
    h('p.muted', null, '关键词用逗号或顿号分开，英文不区分大小写。规则按顺序匹配，第一条命中的生效。'),
    ...spec.rules.map((r, i) =>
      h(
        'div.rule-form',
        null,
        h(
          'div.row',
          null,
          field(r.id, (v) => (r.id = v.trim()), '规则名'),
          field(r.description, (v) => (r.description = v), '说明（可选）'),
          h('button', { onclick: () => move(i, -1), disabled: i === 0, title: '上移' }, '↑'),
          h('button', { onclick: () => move(i, 1), disabled: i === spec.rules.length - 1, title: '下移' }, '↓'),
          h('button.danger', { onclick: () => (spec.rules.splice(i, 1), rebuild()) }, '删除'),
        ),
        h(
          'div.grid',
          null,
          h('label', null, '含任一', field(r.when.any.join('、'), (v) => (r.when.any = splitList(v)))),
          h('label', null, '全部含', field(r.when.all.join('、'), (v) => (r.when.all = splitList(v)))),
          h('label', null, '不含', field(r.when.none.join('、'), (v) => (r.when.none = splitList(v)))),
          h('label', null, '正则', field(r.when.regex ?? '', (v) => (v.trim() ? (r.when.regex = v) : delete r.when.regex))),
          h('label', null, '发送人', field(r.when.from.join('、'), (v) => (r.when.from = splitList(v)))),
        ),
        h('div.row', null, '交给', roleSelect(r), agentSelect(r), escalateBox(r)),
      ),
    ),
    h(
      'div.row',
      null,
      h(
        'button',
        {
          onclick: () => {
            const role = state.roles[0]?.id ?? '';
            spec.rules.push({ id: `规则${spec.rules.length + 1}`, description: '', when: { any: [], all: [], none: [], from: [] }, to: { role, escalate: false } });
            rebuild();
          },
        },
        '+ 新规则',
      ),
    ),
    semanticRow,
    h('div.row', null, h('span.muted', null, '兜底：'), fallback),
    errorEl,
    h(
      'div.row',
      null,
      h('button.primary', { onclick: save }, '保存并生效'),
      h(
        'button',
        {
          onclick: () => {
            state.ruleEdit = null;
            editorEl = null;
            render();
          },
        },
        '取消',
      ),
    ),
  );
  return editorEl;
}

/** @param {Rule} r */
function describeCondition(r) {
  const w = r.when;
  /** @type {string[]} */
  const parts = [];
  if (w.any.length) parts.push(`含任一：${w.any.join('、')}`);
  if (w.all.length) parts.push(`全部含：${w.all.join('、')}`);
  if (w.none.length) parts.push(`不含：${w.none.join('、')}`);
  if (w.regex) parts.push(`正则：${w.regex}`);
  if (w.from.length) parts.push(`发送人：${w.from.join('、')}`);
  return parts.join('；');
}

// ---------- 记忆 ----------

/** 记忆页整块缓存：别的事件触发重绘时不丢正在改的文字。数据变了才重建。 @type {HTMLElement | null} */
let memoryEl = null;
/** 正在加载哪个岗位，防止重复请求。 @type {string | null} */
let memoryLoading = null;
/** 加载失败的岗位和原因。失败后不自动重试，免得断线时反复请求。 @type {{ role: string, error: string } | null} */
let memoryFailed = null;

/** @param {string} role */
async function loadMemory(role) {
  if (memoryLoading === role) return;
  memoryLoading = role;
  try {
    const [view, off] = await Promise.all([
      client.request({ type: 'get_memory', role_id: role }),
      client.request({ type: 'list_memory_off_threads' }),
    ]);
    if (state.memoryRole !== role) return;
    state.memory = view;
    state.memoryOff = off;
    memoryFailed = null;
  } catch (e) {
    memoryFailed = { role, error: /** @type {Error} */ (e).message };
  } finally {
    memoryLoading = null;
  }
  memoryEl = null;
  if (page() === 'memory') render();
}

/**
 * 发一个改记忆的请求，用回复里的新视图重画。
 * @param {import('./types/Request').Request} request
 */
async function memoryAction(request) {
  try {
    state.memory = await client.request(request);
    memoryEl = null;
    render();
  } catch (e) {
    alert(`操作失败：${/** @type {Error} */ (e).message}`);
  }
}

/** @param {string} threadId @param {boolean} enabled */
async function setThreadMemory(threadId, enabled) {
  try {
    state.memoryOff = await client.request({ type: 'set_thread_memory', thread_id: threadId, enabled });
    // 关掉时这个对话已提炼的内容被抹掉了，记忆页要重新取。
    state.memory = null;
    memoryEl = null;
    render();
  } catch (e) {
    alert(`操作失败：${/** @type {Error} */ (e).message}`);
  }
}

/** @param {string} threadId */
function threadLabel(threadId) {
  return state.threads.find((t) => t.thread_id === threadId)?.preview ?? threadId;
}

function renderMemory() {
  if (!state.roles.length) return h('section.placeholder', null, h('p.muted', null, '还没有岗位。在 ip.yaml 的 roles 里定义。'));
  if (!state.memoryRole || !state.roles.some((r) => r.id === state.memoryRole)) state.memoryRole = state.roles[0].id;
  const role = state.memoryRole;
  if (state.memory?.role_id !== role) {
    if (memoryFailed?.role === role) {
      return h(
        'section.placeholder',
        null,
        h('h2', null, '记忆'),
        h('p.error', null, `加载失败：${memoryFailed.error}`),
        h('button', { onclick: () => ((memoryFailed = null), render()) }, '重试'),
      );
    }
    loadMemory(role);
    return h('section.placeholder', null, h('h2', null, '记忆'), h('p.muted', null, '加载中…'));
  }
  memoryEl ??= buildMemory(state.memory);
  return memoryEl;
}

/** @param {MemoryView} view */
function buildMemory(view) {
  const role = view.role_id;
  const picker = h(
    'select',
    {
      onchange: (/** @type {Event} */ e) => {
        state.memoryRole = /** @type {HTMLSelectElement} */ (e.target).value;
        state.memory = null;
        memoryEl = null;
        render();
      },
    },
    ...state.roles.map((r) => h('option', { value: r.id, selected: r.id === role }, `${r.name ?? r.id}（${r.id}）`)),
  );

  /** @param {MemoryLayer} layer @param {string} title @param {string} hint @param {import('./types/MemoryText').MemoryText | null} current */
  const editor = (layer, title, hint, current) => {
    const area = /** @type {HTMLTextAreaElement} */ (h('textarea', { rows: layer === 'notes' ? 5 : 12, value: current?.text ?? '' }));
    return h(
      'div.card',
      null,
      h('h3', null, title),
      h('p.muted', null, hint),
      area,
      h(
        'div.row',
        null,
        h('button.primary', { onclick: () => memoryAction({ type: 'save_memory_text', role_id: role, layer, text: area.value }) }, '保存'),
        h('span.muted.small', null, current ? `上次改动 ${formatTime(current.updated_ms)}` : '还没有内容'),
      ),
    );
  };

  const off = state.memoryOff;
  return h(
    'section.memory',
    null,
    h('div.row', null, h('label', null, '岗位 ', picker), h('button', { onclick: () => loadMemory(role) }, '刷新')),
    view.enabled
      ? h('p.muted', null, '这个岗位开了记忆：闲置下来的对话会被提炼，再合并成岗位记忆摘要，放进这个岗位每位员工的提示词。')
      : h(
          'p.notice',
          null,
          '这个岗位没开记忆（ip.yaml 的 memory.enabled 和 memory.roles）。决策者笔记照常生效；岗位记忆摘要不进提示词，对话也不会被提炼。',
        ),
    editor('notes', '决策者笔记', '你写给这个岗位的话，每位员工每轮都会带上。只有人能改。', view.notes),
    editor(
      'summary',
      '岗位记忆摘要',
      view.summary_from_threads
        ? `这一版基于 ${view.summary_from_threads} 份对话记忆。你可以直接改，下次合并会在你改过的基础上接着改。清空后下次合并会从对话记忆重新生成，想彻底清掉请用页面底部的“清空”。`
        : '还没有合并过，或是人手写的一版。你可以直接改，下次合并会在这一版的基础上接着改。',
      view.summary,
    ),
    h(
      'div.card',
      null,
      h('h3', null, `人要求的更正（${view.corrections.length}）`),
      h('p.muted', null, '有人在对话里明确说“记住……”“以后别……”“忘掉……”时，员工记在这里，下次合并时生效。'),
      view.corrections.length
        ? h(
            'table',
            null,
            h('tbody', null, ...view.corrections.map((n) =>
              h(
                'tr',
                null,
                h('td', null, n.text, h('div.muted', null, `${formatTime(n.created_ms)} · ${n.applied_ms ? '已合并进摘要' : '等下次合并'}`)),
                h(
                  'td.actions',
                  null,
                  n.thread_id ? h('a', { href: '#/chat', onclick: () => openThread(/** @type {string} */ (n.thread_id)) }, '看对话') : null,
                  h('button.danger', { onclick: () => memoryAction({ type: 'delete_memory_note', role_id: role, note_id: n.note_id }) }, '删除'),
                ),
              ),
            )),
          )
        : h('p.muted', null, '没有。'),
    ),
    h(
      'div.card',
      null,
      h('h3', null, `各对话的记忆（${view.threads.length}）`),
      h('p.muted', null, '每个对话提炼出的一段。抹掉以后，下次合并会把摘要里只来自它的内容拿掉；这个对话有新消息后会重新提炼，不想让它再进记忆请同时关掉它的记忆。'),
      view.threads.length
        ? h(
            'table',
            null,
            h('tbody', null, ...view.threads.map((t) =>
              h(
                'tr',
                null,
                h(
                  'td',
                  null,
                  h('strong', null, t.title ?? '（没有标题）'),
                  h('div.memory-text', null, t.summary),
                  h(
                    'div.muted',
                    null,
                    `提炼于 ${formatTime(t.generated_ms)} · 被员工读过 ${t.usage_count} 次${t.last_usage_ms ? `，最近 ${formatTime(t.last_usage_ms)}` : ''}`,
                  ),
                ),
                h(
                  'td.actions',
                  null,
                  h('a', { href: '#/chat', onclick: () => openThread(t.thread_id) }, '看对话'),
                  h('button.danger', { onclick: () => memoryAction({ type: 'forget_thread_memory', role_id: role, thread_id: t.thread_id }) }, '抹掉'),
                  off.includes(t.thread_id) ? null : h('button', { onclick: () => setThreadMemory(t.thread_id, false) }, '抹掉并关掉记忆'),
                ),
              ),
            )),
          )
        : h('p.muted', null, '还没有。'),
    ),
    h(
      'div.card',
      null,
      h('h3', null, `关掉了记忆的对话（${off.length}）`),
      h('p.muted', null, '这些对话不会被提炼，员工也不能在里面记更正。对所有岗位生效。在对话页顶部可以关掉或重新打开。'),
      ...off.map((id) =>
        h(
          'div.row',
          null,
          h('a', { href: '#/chat', onclick: () => openThread(id) }, threadLabel(id)),
          h('button', { onclick: () => setThreadMemory(id, true) }, '重新打开'),
        ),
      ),
    ),
    h(
      'div.card',
      null,
      h('h3', null, '清空这个岗位的记忆'),
      h('p.muted', null, '岗位记忆摘要、各对话的记忆、人要求的更正都清掉，不能恢复。决策者笔记不动。之后的对话照常提炼，记忆从头积累。'),
      h(
        'button.danger',
        {
          onclick: () => {
            if (confirm(`清空「${roleName(role)}」的全部记忆？不能恢复。`)) memoryAction({ type: 'clear_memory', role_id: role });
          },
        },
        '清空',
      ),
    ),
  );
}

// ---------- 对话 ----------

function renderChat() {
  return h('section.chat', null, renderSidebar(), renderThread());
}

/** 搜索框和输入框跨重绘保留，事件推送重绘时不丢正在输入的内容。 @type {HTMLElement | null} */
let searchEl = null;
/** @type {HTMLElement | null} */
let composerEl = null;
/** @type {string | null | undefined} */
let composerFor = undefined;

function renderSidebar() {
  searchEl ??= h('input', {
    type: 'search',
    placeholder: '搜索全部消息',
    onkeydown: (/** @type {KeyboardEvent} */ e) => {
      if (e.key === 'Enter') runSearch(/** @type {HTMLInputElement} */ (e.target).value);
      if (e.key === 'Escape') {
        state.searchHits = null;
        render();
      }
    },
  });
  return h('aside', null, searchEl, renderList());
}

function renderList() {
  const list = state.searchHits
    ? h(
        'div.list',
        null,
        h('div.list-head', null, `找到 ${state.searchHits.length} 条`, h('button.link', { onclick: () => ((state.searchHits = null), render()) }, '清除')),
        ...state.searchHits.map((hit) =>
          h(
            'button.item',
            { onclick: () => openThread(hit.thread_id) },
            h('div.preview', null, hit.text),
            h('div.meta', null, formatTime(hit.at_ms)),
          ),
        ),
      )
    : h(
        'div.list',
        null,
        h('button.item.new' + (state.threadId === null ? '.active' : ''), { onclick: () => startChat(state.agentForNew) }, '+ 新对话'),
        ...state.threads.map((t) =>
          h(
            'button.item' + (t.thread_id === state.threadId ? '.active' : ''),
            { onclick: () => openThread(t.thread_id) },
            h('div.preview', null, t.preview ?? '（无文字）'),
            h('div.meta', null, `${formatTime(t.last_at_ms)} · ${t.message_count} 条`),
          ),
        ),
      );
  list.id = 'thread-list';
  return list;
}

function renderThread() {
  const msgs = h('div.messages', { id: 'messages' }, ...messageNodes());
  if (!composerEl || composerFor !== state.threadId) {
    composerEl = renderComposer();
    composerFor = state.threadId;
  }
  return h('div.thread', null, threadMemoryBar(), msgs, composerEl);
}

/** 对话页顶部的一行：这个对话进不进岗位记忆。 */
function threadMemoryBar() {
  const id = state.threadId;
  if (!id) return null;
  const off = state.memoryOff.includes(id);
  return h(
    'div.thread-memory',
    null,
    off ? '这个对话不进岗位记忆。' : '岗位开了记忆时，这个对话闲置后会被提炼进岗位记忆。',
    h('button.link', { onclick: () => setThreadMemory(id, off) }, off ? '重新打开' : '不让它进记忆'),
  );
}

/** @returns {HTMLElement[]} */
function messageNodes() {
  /** @type {HTMLElement[]} */
  const msgs = [];
  if (!state.threadId && !state.messages.length) {
    msgs.push(h('div.empty', null, state.agents.length ? '选一位员工，开始新对话。' : '没有数字员工。在 ip.yaml 的 agents 里添加。'));
  }
  for (const m of state.messages) msgs.push(renderMessage(m));
  for (const [id, text] of state.drafts) {
    if (state.threadId && state.threadAgent.get(state.threadId) !== id) continue;
    msgs.push(h('div.msg.agent.draft', null, h('div.who', null, agentName(id), h('span.muted', null, ' 正在输入…')), h('div.text', null, text)));
  }
  const note = state.threadId ? state.turnNote.get(state.threadId) : undefined;
  if (note) msgs.push(h('div.turn-note', null, note));
  return msgs;
}

function renderComposer() {
  const agentId = currentAgent();

  const input = /** @type {HTMLTextAreaElement} */ (
    h('textarea', { placeholder: 'Enter 发送，Shift+Enter 换行', rows: 3 })
  );
  const select = /** @type {HTMLSelectElement} */ (
    h(
      'select',
      { onchange: (/** @type {Event} */ e) => (state.agentForNew = /** @type {HTMLSelectElement} */ (e.target).value) },
      h('option', { value: AUTO, selected: agentId === AUTO }, '自动分派（按路由规则）'),
      ...state.agents.map((a) => h('option', { value: a.id, selected: a.id === agentId }, `${a.name}（${a.role}）`)),
    )
  );
  const send = async () => {
    const text = input.value.trim();
    if (!text || !select.value) return;
    input.value = '';
    try {
      /** @type {SendResult} */
      const res = await client.request({
        type: 'send',
        agent_id: select.value === AUTO ? undefined : select.value,
        thread_id: state.threadId ?? undefined,
        text,
        author: store.get(NAME_KEY) ?? undefined,
      });
      const m = res.message;
      if (res.agent_id) {
        state.threadAgent.set(m.thread_id, res.agent_id);
        // 自动分派后，这个对话后续默认继续找同一位员工。
        select.value = res.agent_id;
      }
      if (res.decision?.kind === 'routed' && res.agent_id) {
        state.turnNote.set(m.thread_id, `自动分派给 ${agentName(res.agent_id)}。${res.decision.reason}`);
      } else if (res.approval_id) {
        state.turnNote.set(m.thread_id, '路由拿不准，已在“审批”页发起决策卡片，选定岗位后员工会接着处理。');
      }
      if (!state.threadId) {
        state.threadId = m.thread_id;
        composerFor = m.thread_id;
        if (!state.messages.some((x) => x.id === m.id)) state.messages.push(m);
        render();
      }
    } catch (e) {
      input.value = text;
      alert(`发送失败：${/** @type {Error} */ (e).message}`);
    }
  };
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
      e.preventDefault();
      send();
    }
  });
  return h('div.composer', null, select, input, h('button.primary', { onclick: send }, '发送'));
}

/** 只更新消息区和对话列表，不动输入框。 */
function updateChat() {
  const msgs = document.getElementById('messages');
  const list = document.getElementById('thread-list');
  if (page() !== 'chat' || !msgs || !list) return render();
  const atBottom = msgs.scrollHeight - msgs.scrollTop - msgs.clientHeight < 40;
  msgs.replaceChildren(...messageNodes());
  list.replaceWith(renderList());
  if (atBottom) scrollMessages();
}

function currentAgent() {
  if (state.threadId) {
    const a = state.threadAgent.get(state.threadId);
    if (a) return a;
  }
  return state.agentForNew || state.agents[0]?.id || '';
}

/** @param {Message} m */
function renderMessage(m) {
  const who =
    m.author.kind === 'human' ? m.author.name : m.author.kind === 'agent' ? agentName(m.author.agent_id) : '系统';
  const cls = m.author.kind === 'human' ? '.human' : m.author.kind === 'agent' ? '.agent' : '.system';
  const head = h('div.who', null, who, h('span.muted', null, ` ${formatTime(m.at_ms)}`));
  switch (m.body.kind) {
    case 'text':
      return h('div.msg' + cls, null, head, h('div.text', null, m.body.text));
    case 'tool_call':
      return h(
        'details.tool',
        null,
        h('summary', null, `调用工具 ${m.body.name}`),
        h('pre', null, typeof m.body.input === 'string' ? m.body.input : JSON.stringify(m.body.input, null, 2)),
      );
    case 'tool_result':
      return h(
        'details.tool' + (m.body.ok ? '' : '.failed'),
        null,
        h('summary', null, m.body.ok ? '工具结果' : '工具失败'),
        h('pre', null, m.body.output),
      );
    case 'summary':
      return h(
        'details.tool.summary',
        null,
        h('summary', null, '较早的对话已压缩成摘要'),
        h('div.text', null, m.body.text),
      );
  }
}

function scrollMessages() {
  const el = document.getElementById('messages');
  if (el) el.scrollTop = el.scrollHeight;
}

/** @param {string} threadId */
async function openThread(threadId) {
  state.threadId = threadId;
  state.messages = [];
  render();
  await loadThread();
}

async function loadThread() {
  const id = state.threadId;
  if (!id) return;
  /** @type {Message[]} */
  const messages = await client.request({ type: 'history', thread_id: id, limit: 200 });
  if (state.threadId !== id) return;
  state.messages = messages;
  const lastAgent = [...messages].reverse().find((m) => m.author.kind === 'agent');
  if (lastAgent && lastAgent.author.kind === 'agent' && state.threadAgent.get(id) !== lastAgent.author.agent_id) {
    state.threadAgent.set(id, lastAgent.author.agent_id);
    composerEl = null; // 员工下拉框要换成这个对话的员工
  }
  render();
}

/** @param {string} query */
async function runSearch(query) {
  if (!query.trim()) return;
  state.searchHits = await client.request({ type: 'search', query, limit: 50 });
  render();
}

let threadsTimer = 0;
let spendTimer = 0;
/** 一轮结束后刷新今天的费用（合并短时间内的多次）。 */
function refreshSpendSoon() {
  clearTimeout(spendTimer);
  spendTimer = window.setTimeout(async () => {
    state.spend = await client.request({ type: 'spend_today' });
    if (page() === 'roles') render();
  }, 500);
}

function refreshThreadsSoon() {
  clearTimeout(threadsTimer);
  threadsTimer = window.setTimeout(async () => {
    state.threads = await client.request({ type: 'list_threads', limit: 100 });
    if (page() === 'chat') updateChat();
  }, 300);
}

/** @param {RuntimeEvent} e */
function onEvent(e) {
  switch (e.type) {
    case 'delta': {
      if (e.thread_id !== state.threadId) return;
      state.drafts.set(e.agent_id, (state.drafts.get(e.agent_id) ?? '') + e.text);
      break;
    }
    case 'message': {
      const m = e.message;
      if (m.thread_id === state.threadId) {
        if (m.author.kind === 'agent') state.drafts.delete(m.author.agent_id);
        if (!state.messages.some((x) => x.id === m.id)) state.messages.push(m);
      }
      refreshThreadsSoon();
      break;
    }
    case 'turn_started':
      state.turnNote.set(e.thread_id, `${agentName(e.agent_id)} 开始处理…`);
      if (e.thread_id !== state.threadId) return;
      break;
    case 'turn_finished': {
      state.drafts.delete(e.agent_id);
      refreshSpendSoon();
      const tokens = e.usage.prompt_tokens + e.usage.completion_tokens;
      const cached = e.usage.prompt_tokens ? Math.round((100 * e.usage.cached_tokens) / e.usage.prompt_tokens) : 0;
      state.turnNote.set(
        e.thread_id,
        `${agentName(e.agent_id)} 完成：${e.steps} 步，${(e.elapsed_ms / 1000).toFixed(1)} 秒，${tokens} tokens（缓存命中 ${cached}%）${e.cost_usd > 0 ? `，约 $${e.cost_usd.toFixed(4)}` : ''}`,
      );
      if (e.thread_id !== state.threadId) return;
      break;
    }
    case 'turn_failed':
      state.drafts.delete(e.agent_id);
      refreshSpendSoon();
      state.turnNote.set(e.thread_id, `${agentName(e.agent_id)} 这轮失败：${e.error}`);
      if (e.thread_id !== state.threadId) return;
      break;
    case 'task_changed':
      upsert(state.tasks, e.task);
      state.tasks.sort((a, b) => b.updated_at_ms - a.updated_at_ms);
      if (page() !== 'chat') return render();
      return;
    case 'approval_changed':
      upsert(state.approvals, e.approval);
      state.approvals.sort((a, b) => b.created_at_ms - a.created_at_ms);
      // 角标在顶栏，所有页面都要更新。
      if (page() !== 'chat') return render();
      return renderHeaderOnly();
  }
  if (page() === 'chat') updateChat();
}

function askToken() {
  const t = prompt('访问令牌（服务配置了 auth_token_env 时需要）', store.get(TOKEN_KEY) ?? '');
  if (t === null) return;
  store.set(TOKEN_KEY, t.trim());
  const n = prompt('你的名字（显示在消息上）', store.get(NAME_KEY) ?? '');
  if (n !== null && n.trim()) store.set(NAME_KEY, n.trim());
  location.reload();
}

async function loadAll() {
  [state.roles, state.agents, state.threads] = await Promise.all([
    client.request({ type: 'list_roles' }),
    client.request({ type: 'list_agents' }),
    client.request({ type: 'list_threads', limit: 100 }),
  ]);
  [state.rules, state.ruleDrafts, state.tasks, state.approvals] = await Promise.all([
    client.request({ type: 'list_rules' }),
    client.request({ type: 'list_rule_drafts' }),
    client.request({ type: 'list_tasks', limit: 200 }),
    client.request({ type: 'list_approvals', limit: 50 }),
  ]);
  [state.spend, state.schedules, state.memoryOff] = await Promise.all([
    client.request({ type: 'spend_today' }),
    client.request({ type: 'list_schedules' }),
    client.request({ type: 'list_memory_off_threads' }),
  ]);
  state.memory = null;
  memoryEl = null;
  memoryFailed = null;
  // 配了路由规则时，新对话默认自动分派。
  if (!state.agentForNew) state.agentForNew = state.rules?.spec.rules.length ? AUTO : (state.agents[0]?.id ?? '');
  composerEl = null;
  await loadThread();
  render();
}

client.onStatus((s) => {
  if (s === 'open') loadAll().catch((e) => console.error(e));
  render();
});
client.onEvent(onEvent);
client.onLagged(() => {
  loadThread();
  refreshThreadsSoon();
});
window.addEventListener('hashchange', render);
client.connect();
render();
