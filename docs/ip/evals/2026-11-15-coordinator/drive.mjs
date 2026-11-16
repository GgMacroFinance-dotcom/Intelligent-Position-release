#!/usr/bin/env node
// 经 /ws 给一个员工发一条消息，等整件事安静下来，把过程汇总成 JSON。
// 选择卡片（ask_human、路由拿不准）自动选推荐项，没有推荐项就选第一个。
//
//   node drive.mjs --agent coord-1 --text "……" [--thread <id>] [--url ws://127.0.0.1:7878/ws]
//                  [--idle 20] [--timeout 1500] [--max-approvals 6] [--out run.json]
//   node drive.mjs --request '{"type":"spend_today"}'        # 只发一个请求，打印回复
//
// 需要 Node.js 22（自带 WebSocket）。

const args = {};
for (let i = 2; i < process.argv.length; i += 2) args[process.argv[i].replace(/^--/, '')] = process.argv[i + 1];
const url = args.url ?? 'ws://127.0.0.1:7878/ws';
const idleMs = Number(args.idle ?? 20) * 1000;
const timeoutMs = Number(args.timeout ?? 1500) * 1000;
const maxApprovals = Number(args['max-approvals'] ?? 6);

const ws = new WebSocket(url);
let nextId = 1;
const pending = new Map();
const request = (body) =>
  new Promise((resolve, reject) => {
    const id = nextId++;
    pending.set(id, { resolve, reject });
    ws.send(JSON.stringify({ id, ...body }));
  });

const running = new Set(); // `${agent}|${thread}`
const turns = [];
const toolCalls = [];
const texts = [];
const approvals = [];
const tasks = new Map();
const callNames = new Map();
let lastActivity = Date.now();
let pendingApprovals = 0;

function onEvent(e) {
  lastActivity = Date.now();
  if (e.type === 'turn_started') running.add(`${e.agent_id}|${e.thread_id}`);
  else if (e.type === 'turn_finished') {
    running.delete(`${e.agent_id}|${e.thread_id}`);
    turns.push({ agent: e.agent_id, thread: e.thread_id, ok: true, steps: e.steps, cost_usd: e.cost_usd, usage: e.usage, elapsed_ms: e.elapsed_ms });
  } else if (e.type === 'turn_failed') {
    running.delete(`${e.agent_id}|${e.thread_id}`);
    turns.push({ agent: e.agent_id, thread: e.thread_id, ok: false, error: e.error });
  } else if (e.type === 'message') {
    const m = e.message;
    const who = m.author.kind === 'agent' ? m.author.agent_id : m.author.kind;
    if (m.body.kind === 'tool_call') {
      callNames.set(m.body.call_id, m.body.name);
      toolCalls.push({ agent: who, thread: m.thread_id, name: m.body.name, input: m.body.input, call_id: m.body.call_id });
    } else if (m.body.kind === 'tool_result') {
      const call = toolCalls.find((c) => c.call_id === m.body.call_id);
      if (call) {
        call.ok = m.body.ok;
        call.output_chars = m.body.output.length;
        if (!m.body.ok || /^(memory_|create_task|update_task|ask_human)/.test(call.name)) call.output = m.body.output.slice(0, 600);
      }
    } else if (m.body.kind === 'text') {
      texts.push({ author: who, thread: m.thread_id, notice: m.notice ?? null, text: m.body.text });
    }
  } else if (e.type === 'task_changed') {
    const t = e.task;
    tasks.set(t.id, { id: t.id, title: t.title, description: t.description, status: t.status, role: t.role, assignee: t.assignee, depth: t.depth, result: t.result, created_by: t.created_by });
  } else if (e.type === 'approval_changed') {
    const a = e.approval;
    if (a.state.state === 'pending') {
      const known = approvals.find((x) => x.id === a.id);
      if (known) return;
      const choice = a.recommended ?? 0;
      approvals.push({ id: a.id, subject: a.subject.kind, question: a.question, options: a.options.map((o) => o.label), recommended: a.recommended, reason: a.reason, chose: choice });
      if (approvals.length > maxApprovals) return; // 不再自动选，免得一圈圈花钱
      pendingApprovals += 1;
      setTimeout(async () => {
        try {
          await request({ type: 'resolve_approval', approval_id: a.id, choice, author: 'eval' });
        } catch (err) {
          approvals.find((x) => x.id === a.id).error = String(err.message ?? err);
        }
        pendingApprovals -= 1;
        lastActivity = Date.now();
      }, 1000);
    }
  }
}

ws.addEventListener('message', (ev) => {
  const frame = JSON.parse(ev.data);
  if (frame.type === 'reply') {
    const p = pending.get(frame.id);
    if (!p) return;
    pending.delete(frame.id);
    if (frame.ok) p.resolve(frame.data);
    else p.reject(new Error(frame.error ?? '请求失败'));
  } else if (frame.type === 'event') onEvent(frame.event);
  else if (frame.type === 'lagged') console.error(`事件丢了 ${frame.skipped} 条`);
});
ws.addEventListener('error', (ev) => {
  console.error('连接出错', ev.message ?? '');
  process.exit(2);
});

ws.addEventListener('open', async () => {
  try {
    if (args.request) {
      console.log(JSON.stringify(await request(JSON.parse(args.request)), null, 2));
      ws.close();
      return;
    }
    const started = Date.now();
    const sent = await request({ type: 'send', agent_id: args.agent, thread_id: args.thread, text: args.text, author: args.author ?? 'eval' });
    const thread = sent.message.thread_id;
    let stop = 'quiet';
    for (;;) {
      await new Promise((r) => setTimeout(r, 500));
      if (Date.now() - started > timeoutMs) {
        stop = 'timeout';
        break;
      }
      if (running.size === 0 && pendingApprovals === 0 && Date.now() - lastActivity > idleMs) break;
    }
    const spend = await request({ type: 'spend_today' });
    const summary = {
      text: args.text,
      agent: sent.agent_id,
      thread,
      stop,
      elapsed_s: Math.round((Date.now() - started) / 1000),
      turns,
      approvals,
      tasks: [...tasks.values()],
      tool_calls: toolCalls.map(({ call_id, ...rest }) => rest),
      texts,
      spend,
    };
    if (args.out) (await import('node:fs')).writeFileSync(args.out, JSON.stringify(summary, null, 2));
    // 屏幕上只打要点。
    const byAgent = {};
    for (const t of turns) {
      const a = (byAgent[t.agent] ??= { turns: 0, failed: 0, cost: 0 });
      a.turns += 1;
      if (!t.ok) a.failed += 1;
      a.cost += t.cost_usd ?? 0;
    }
    console.log(`结束：${stop}，${summary.elapsed_s} 秒，对话 ${thread}`);
    for (const [a, v] of Object.entries(byAgent)) console.log(`  ${a}：${v.turns} 轮（失败 ${v.failed}），${v.cost.toFixed(4)} 美元`);
    for (const c of toolCalls.filter((c) => ['create_task', 'ask_human', 'update_task', 'memory_note', 'memory_search', 'memory_read'].includes(c.name)))
      console.log(`  ${c.agent} → ${c.name} ${JSON.stringify(c.input).slice(0, 300)}`);
    for (const t of tasks.values()) console.log(`  任务 [${t.status}] ${t.title} → ${t.assignee ?? t.role}`);
    for (const a of approvals) console.log(`  卡片（${a.subject}）：${a.question.slice(0, 120)} → 选 ${a.chose}：${a.options[a.chose]}`);
    console.log(`  今日花费：${JSON.stringify(spend.roles)}`);
    ws.close();
  } catch (err) {
    console.error('失败：', err.message ?? err);
    process.exit(1);
  }
});
