import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { snapshot, formatSnapshot, PANEL_IDS } from '../src/snapshot.js';
import { handleMessage, handleRaw } from '../src/mcp.js';
import { createHttpServer } from '../src/http.js';
import { clearCache } from '../src/upstream.js';
import { defaultRoutes, mockFetch } from './fixtures.js';

beforeEach(() => clearCache());
// FRED panels go through the (mocked) WhichAPI service, which needs a key.
process.env.WHICHAPI_API_KEY = 'k-test';
delete process.env.WHICHAPI_URL;
const now = new Date('2026-11-06T12:00:00Z');

test('snapshot fills every panel from the official sources', async () => {
  const m = mockFetch();
  try {
    const s = await snapshot({ now });
    assert.equal(s.status, 'ok');
    assert.deepEqual(s.panels.map((p) => p.id), PANEL_IDS);
    const p = Object.fromEntries(s.panels.map((x) => [x.id, x]));
    assert.deepEqual([p.sofr.value, p.sofr.as_of, p.sofr.change], [3.94, '2026-11-04', -0.02]);
    assert.deepEqual(p.effr.extras.target_range, [3.75, 4]);
    assert.deepEqual([p.broad_dollar.value, p.broad_dollar.previous, p.broad_dollar.series.length], [121.67, { date: '2026-10-31', value: 121.5 }, 2]);
    assert.equal(p.usd_cny.value, 7.1032);
    assert.equal(PANEL_IDS.some((id) => ['ig_oas', 'hy_oas', 'ig_yield'].includes(id)), false);
    assert.deepEqual([p.ust_10y.value, p.ust_10y.change, p.ust_10y.extras.slope_2s10s_pct_pts, p.ust_10y.extras.curve['30Y']], [4.84, 0.04, 1.24, 5]);
    assert.deepEqual([p.tic_china.value, p.tic_china.as_of, p.tic_china.change], [555800, '2026-08', -4300]);
    assert.equal(p.tic_japan.value, 1150000);
    assert.deepEqual([p.debt_limit.value, p.debt_limit.as_of], [750105, '2026-11-04']);
    assert.equal(p.section_232.documents.length, 2);
    assert.equal(s.warnings.length, 2);
    const fredStart = m.calls.find((c) => c.href.includes('/v1/tools/get_fred_series')).href;
    assert.match(fredStart, /start_date=2026-07-09/);
    assert.equal(m.calls.some((c) => new URL(c.href).host === 'fred.stlouisfed.org'), false);
    const text = formatSnapshot(s);
    assert.match(text, /SOFR: 3\.94% as of 2026-11-04 \(-0\.02 pts vs 2026-11-03\)/);
    assert.match(text, /Room under the statutory debt limit: 750\.1 bn USD/);
    assert.match(text, /2s10s 1\.24 pts/);
  } finally { m.restore(); }
});

test('one failing source makes the snapshot partial, not failed', async () => {
  const m = mockFetch((url) => (new URL(url).host === 'ticdata.treasury.gov' ? new Response('down', { status: 503 }) : defaultRoutes(url)));
  try {
    const s = await snapshot({ now, panels: ['sofr', 'tic_china', 'tic_japan'] });
    assert.equal(s.status, 'partial');
    assert.deepEqual(s.panels.map((p) => p.status), ['ok', 'error', 'error']);
    assert.equal(s.panels[1].error.code, 'upstream_error');
    assert.match(formatSnapshot(s), /error upstream_error/);
    await assert.rejects(snapshot({ panels: ['nope'] }), /Unknown panel/);
  } finally { m.restore(); }
});

test('MCP: initialize, list, call, and protocol errors', async () => {
  const init = await handleMessage({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-03-26', capabilities: {}, clientInfo: { name: 't', version: '1' } } });
  assert.equal(init.result.protocolVersion, '2025-03-26');
  assert.deepEqual(init.result.capabilities, { tools: { listChanged: false } });
  assert.equal((await handleMessage({ jsonrpc: '2.0', id: 9, method: 'initialize', params: { protocolVersion: '1999-01-01' } })).result.protocolVersion, '2025-06-18');
  assert.equal(await handleMessage({ jsonrpc: '2.0', method: 'notifications/initialized' }), null);
  const list = await handleMessage({ jsonrpc: '2.0', id: 2, method: 'tools/list' });
  assert.equal(list.result.tools.length, 13);
  assert.equal(list.result.tools.some((t) => ['discover_apis', 'get_api', 'call_api'].includes(t.name)), false);
  for (const t of list.result.tools) { assert.equal(t.inputSchema.type, 'object'); assert.equal(t.annotations.readOnlyHint, true); }
  const panels = await handleMessage({ jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'list_macro_panels', arguments: {} } });
  assert.equal(panels.result.isError, false);
  assert.equal(panels.result.structuredContent.panels.length, PANEL_IDS.length);
  const bad = await handleMessage({ jsonrpc: '2.0', id: 4, method: 'tools/call', params: { name: 'get_fred_series', arguments: { series_id: 'DGS10', surprise: 1 } } });
  assert.equal(bad.result.isError, true); assert.equal(bad.result.structuredContent.error.code, 'invalid_request');
  assert.equal((await handleMessage({ jsonrpc: '2.0', id: 5, method: 'tools/call', params: { name: 'nope' } })).error.code, -32602);
  assert.equal((await handleMessage({ jsonrpc: '2.0', id: 6, method: 'sampling/createMessage' })).error.code, -32601);
  assert.deepEqual(JSON.parse(await handleRaw('{oops')).error.code, -32700);
  const batch = JSON.parse(await handleRaw(JSON.stringify([{ jsonrpc: '2.0', id: 7, method: 'ping' }, { jsonrpc: '2.0', method: 'notifications/initialized' }])));
  assert.deepEqual(batch, [{ jsonrpc: '2.0', id: 7, result: {} }]);
});

test('MCP tools/call returns text plus structured data', async () => {
  const m = mockFetch();
  try {
    const r = (await handleMessage({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'get_debt_limit', arguments: {} } })).result;
    assert.equal(r.isError, false);
    assert.match(r.content[0].text, /headroom 750105/);
    assert.equal(r.structuredContent.records[0].headroom, 750105);
    const s = (await handleMessage({ jsonrpc: '2.0', id: 2, method: 'tools/call', params: { name: 'macro_snapshot', arguments: { panels: ['sofr'], max_age_seconds: 0 } } })).result;
    assert.equal(s.structuredContent.panels.length, 1);
  } finally { m.restore(); }
});

test('stdio transport answers line-delimited JSON-RPC from the CLI', async () => {
  const bin = fileURLToPath(new URL('../bin/whichapi-macro.js', import.meta.url));
  const child = spawn(process.execPath, [bin, 'mcp'], { stdio: ['pipe', 'pipe', 'pipe'] });
  let out = '';
  child.stdout.on('data', (d) => { out += d; });
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: { name: 't', version: '1' } } }) + '\n');
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/list' }) + '\n');
  child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id: 3, method: 'tools/call', params: { name: 'list_macro_panels', arguments: {} } }) + '\n');
  child.stdin.end();
  await new Promise((resolve) => child.on('close', resolve));
  const replies = out.trim().split('\n').map((l) => JSON.parse(l)).sort((a, b) => a.id - b.id);
  assert.deepEqual(replies.map((r) => r.id), [1, 2, 3]);
  assert.equal(replies[0].result.serverInfo.name, 'whichapi-macro');
  assert.equal(replies[1].result.tools.length, 13);
  assert.equal(replies[2].result.structuredContent.panels.length, PANEL_IDS.length);
});

async function withServer(options, run) {
  const server = createHttpServer({ log: () => {}, ...options });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  try { await run(base); } finally { await new Promise((r) => server.close(r)); }
}
const post = (base, path, body, headers = {}) => fetch(base + path, { method: 'POST', headers: { 'content-type': 'application/json', ...headers }, body: JSON.stringify(body) });

test('HTTP: MCP endpoint, REST endpoints and origin checks', async () => {
  await withServer({}, async (base) => {
    const init = await post(base, '/mcp', { jsonrpc: '2.0', id: 1, method: 'initialize', params: { protocolVersion: '2025-06-18' } });
    assert.equal(init.status, 200); assert.equal((await init.json()).result.serverInfo.name, 'whichapi-macro');
    assert.equal((await post(base, '/mcp', { jsonrpc: '2.0', method: 'notifications/initialized' })).status, 202);
    assert.equal((await fetch(base + '/mcp')).status, 405);
    assert.equal((await fetch(base + '/mcp', { method: 'POST', body: '{}' })).status, 415);
    assert.equal((await post(base, '/mcp', { jsonrpc: '2.0', id: 1, method: 'ping' }, { origin: 'https://evil.example' })).status, 403);
    assert.equal((await post(base, '/mcp', { jsonrpc: '2.0', id: 1, method: 'ping' }, { origin: 'http://localhost:5173' })).status, 200);
    assert.equal((await (await fetch(base + '/v1/panels')).json()).panels.length, PANEL_IDS.length);
    assert.equal((await fetch(base + '/v1/tools/get_fred_series?series_id=bad%20id')).status, 400);
    assert.equal((await fetch(base + '/v1/apis?query=fx')).status, 404);
    assert.equal((await fetch(base + '/v1/tools/nope')).status, 404);
    assert.equal((await fetch(base + '/health')).status, 200);
  });
});

test('HTTP: query arguments are typed for REST calls', async () => {
  // The test client also uses fetch, so pass loopback requests through and fake only the upstreams.
  const original = globalThis.fetch;
  globalThis.fetch = async (input, init) => (String(input).startsWith('http://127.0.0.1') ? original(input, init) : defaultRoutes(String(input)));
  try {
    await withServer({}, async (base) => {
      const r = await fetch(base + '/v1/tools/get_reference_rates?rate=EFFR&observations=2');
      assert.equal(r.status, 200);
      assert.equal((await r.json()).records[0].target_rate_to, 4);
      const s = await (await fetch(base + '/v1/snapshot?panels=sofr,debt_limit')).json();
      assert.deepEqual(s.panels.map((p) => p.id), ['sofr', 'debt_limit']);
      const p = await post(base, '/v1/tools/get_tic_holdings', { country: 'Japan', limit: 1 });
      assert.equal((await p.json()).records[0].treas_pos, 1150000);
    });
  } finally { globalThis.fetch = original; }
});

test('HTTP: API key and public binding rules', async () => {
  await withServer({ apiKey: 'k1' }, async (base) => {
    assert.equal((await fetch(base + '/v1/panels')).status, 401);
    assert.equal((await fetch(base + '/v1/panels', { headers: { authorization: 'Bearer k1' } })).status, 200);
    assert.equal((await fetch(base + '/health')).status, 200);
  });
  assert.throws(() => createHttpServer({ host: '0.0.0.0', apiKey: '' }), /MACRO_API_KEY/);
  assert.doesNotThrow(() => createHttpServer({ host: '0.0.0.0', apiKey: 'x' }));
});

test('HTTP: malformed request targets and foreign Host headers are rejected without crashing', async () => {
  const { connect } = await import('node:net');
  await withServer({ apiKey: 'k1' }, async (base) => {
    const port = Number(new URL(base).port);
    const raw = (text) => new Promise((resolve) => { let out = ''; const s = connect(port, '127.0.0.1', () => s.write(text)); s.on('data', (d) => { out += d; }); s.on('close', () => resolve(out)); s.on('error', () => resolve(out)); });
    assert.match(await raw('GET //[/ HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n'), /^HTTP\/1\.1 400/);
    assert.match(await raw('GARBAGE\r\n\r\n'), /^HTTP\/1\.1 400/);
    assert.match(await raw('GET /v1/panels HTTP/1.1\r\nHost: attacker.example:3220\r\nConnection: close\r\n\r\n'), /^HTTP\/1\.1 421/);
    assert.equal((await fetch(base + '/health')).status, 200); // still alive
  });
});

test('Treasury panel falls back to the prior year early in January', async () => {
  const { treasuryCsv } = await import('./fixtures.js');
  const m = mockFetch((url) => {
    const u = new URL(url);
    if (u.host !== 'home.treasury.gov') return defaultRoutes(url);
    return new Response(u.pathname.includes('/2027/') ? treasuryCsv([]) : treasuryCsv([['12/30/2026', 4.70, 3.50], ['12/31/2026', 4.75, 3.55]]), { headers: { 'content-type': 'text/csv' } });
  });
  try {
    const s = await snapshot({ now: new Date('2027-01-01T12:00:00Z'), panels: ['ust_10y'] });
    const p = s.panels[0];
    assert.deepEqual([p.status, p.value, p.as_of, p.change, p.extras.curve_date], ['ok', 4.75, '2026-12-31', 0.05, '2026-12-31']);
    assert.ok(m.calls.some((c) => c.href.includes('/2027/')) && m.calls.some((c) => c.href.includes('/2026/')));
  } finally { m.restore(); }
});

test('HTTP: browser console at / is served without a key and its script parses', async () => {
  const { CONSOLE_HTML } = await import('../src/console.js');
  const script = CONSOLE_HTML.split('<script>')[1].split('</script>')[0];
  assert.doesNotThrow(() => new Function(script));
  await withServer({ apiKey: 'k1' }, async (base) => {
    const page = await fetch(base + '/', { headers: { accept: 'text/html' } });
    assert.equal(page.status, 200);
    assert.match(page.headers.get('content-security-policy'), /connect-src 'self'/);
    assert.match(await page.text(), /<title>WhichAPI Macro<\/title>/);
    assert.equal((await fetch(base + '/')).status, 401); // JSON index still needs the key
    assert.equal((await fetch(base + '/v1/snapshot', { headers: { accept: 'text/html' } })).status, 401);
  });
});

test('HTTP: TRUST_PROXY rate limits by X-Forwarded-For', async () => {
  await withServer({ trustProxy: true }, async (base) => {
    const hit = (ip) => fetch(base + '/v1/panels', { headers: { 'x-forwarded-for': `${ip}, 10.0.0.1` } }).then((r) => r.status);
    for (let i = 0; i < 120; i++) assert.equal(await hit('203.0.113.1'), 200);
    assert.equal(await hit('203.0.113.1'), 429);
    assert.equal(await hit('203.0.113.2'), 200);
  });
});
