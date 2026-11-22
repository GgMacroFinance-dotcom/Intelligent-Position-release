import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import * as china from '../src/china.js';
import { callTool } from '../src/tools.js';
import { clearCache, SourceError } from '../src/upstream.js';
import { mockFetch } from './fixtures.js';

// Layouts follow data.stats.gov.cn as seen on 2026-11-15; values are illustrative.
beforeEach(() => clearCache());
const rejectsWith = (promise, code) => assert.rejects(promise, (e) => e instanceof SourceError && e.code === code);
const nov15 = { now: new Date('2026-11-15T12:00:00Z') };
const BASE = 'https://data.stats.gov.cn/dg/website/publicrelease/web/external';
const ok = (data) => new Response(JSON.stringify({ data, success: true, state: 20000, message: '成功' }), { headers: { 'content-type': 'text/plain;charset=UTF-8' } });
const hex = (c) => c.repeat(32);
const CPI = china.CHINA_SERIES.cpi_yoy.parts;
const cell = (id, name, value, unit = '%') => ({ i_showname: `${name} `, _id: id, du_name: unit, value, da: '000000000000', da_name: '全国' });
const periods = (id, name, rows, suffix = 'MM') => rows.map(([p, v]) => ({ code: `${p}${suffix}`, name: p, values: [cell(id, name, v)] }));

/** A fake NBS site: catalog tree, indicator lists, search and esData for the CPI catalogs and one quarterly series. */
function site({ storedIdsGone = false } = {}) {
  const tree = {
    '': [{ _id: hex('a'), name: '月度数据', isLeaf: false }],
    [hex('a')]: [{ _id: hex('b'), name: '价格指数', isLeaf: false }, { _id: hex('c'), name: '金融', isLeaf: false }],
    [hex('b')]: [{ _id: hex('d'), name: '居民消费价格分类指数 (上年同月=100) ', isLeaf: false }],
    [hex('d')]: [
      { _id: hex('e'), name: '全国居民消费价格分类指数 (上年同月=100) (2026-)', isLeaf: true, sdate: '2026' },
      { _id: hex('f'), name: '全国居民消费价格分类指数 (上年同月=100) (2021-2025)', isLeaf: true, sdate: '2021', edate: '2025' },
      { _id: hex('9'), name: '城市居民消费价格分类指数 (上年同月=100) (2026-)', isLeaf: true, sdate: '2026' },
    ],
  };
  const lists = { [hex('e')]: [{ _id: hex('1'), i_showname: '居民消费价格指数 (上年同月=100) ', du_name: '%' }, { _id: hex('2'), i_showname: '食品烟酒类居民消费价格指数 (上年同月=100)', du_name: '%' }], [hex('f')]: [{ _id: hex('3'), i_showname: '居民消费价格指数 (上年同月=100)', du_name: '%' }] };
  const name = '居民消费价格指数 (上年同月=100)';
  const series = {
    [CPI[1][1]]: periods(CPI[1][1], name, [['202512', '100.8'], ['202511', '100.7']]),
    [CPI[2][1]]: periods(CPI[2][1], name, [['202611', ''], ['202610', '100.7'], ['202609', '100.9'], ['202512', '100.9']]),
    [hex('1')]: periods(hex('1'), name, [['202611', ''], ['202610', '100.7']]),
    [hex('3')]: periods(hex('3'), name, [['202512', '100.8']]),
    [hex('4')]: periods(hex('4'), '国内生产总值当季值 (亿元)', [['202604', ''], ['202603', '375367.2'], ['202602', '361511.1']], 'SS'),
  };
  return (href, init) => {
    const u = new URL(href);
    if (u.pathname.endsWith('/new/queryIndexTreeAsync')) return ok(tree[u.searchParams.get('pid')] || []);
    if (u.pathname.endsWith('/new/queryIndicatorsByCid')) return ok({ total: 0, list: lists[u.searchParams.get('cid')] || [] });
    if (u.pathname.endsWith('/query')) return ok({ count: 3, data: [
      { cid: hex('e'), indic_id: hex('2'), show_name: '食品烟酒类居民消费价格指数 (上年同月=100)', dt: '202610', value: '98.4', da: '000000000000', explain: '' },
      { cid: hex('e'), indic_id: hex('1'), show_name: '居民消费价格指数 (上年同月=100) ', dt: '202610', value: '100.7', da: '000000000000', explain: '居民消费价格指数是…' },
      { cid: hex('e'), indic_id: hex('1'), show_name: '居民消费价格指数 (上年同月=100) ', dt: '202609', value: '100.9', da: '000000000000' },
      { cid: hex('e'), indic_id: hex('1'), show_name: '居民消费价格指数 (上年同月=100) ', dt: '202610', value: '100.1', da: '110000000000' },
    ] });
    if (u.pathname.endsWith('/stream/esData')) {
      const id = JSON.parse(init.body).indicatorIds[0];
      return ok(storedIdsGone && CPI.some(([, x]) => x === id) ? [] : series[id] || []);
    }
    return new Response('not found', { status: 404 });
  };
}

test('search maps short names, keeps one national row per indicator, ranks name matches first', async () => {
  const m = mockFetch(site());
  try {
    const r = await china.searchChinaIndicators({ query: 'CPI', limit: 5 });
    const u = new URL(m.calls[0].href);
    assert.equal(u.origin + u.pathname, `${BASE}/query`);
    assert.deepEqual([u.searchParams.get('search'), u.searchParams.get('code'), u.searchParams.get('pageSize')], ['居民消费价格指数', '1', '100']);
    assert.equal(r.total, 2);
    assert.deepEqual(r.results.map((x) => [x.name, x.latest_period, x.latest_value, x.name_contains_query]), [
      ['居民消费价格指数 (上年同月=100)', '2026-10', 100.7, true],
      ['食品烟酒类居民消费价格指数 (上年同月=100)', '2026-10', 98.4, true],
    ]);
    assert.deepEqual([r.results[0].catalog_id, r.results[0].indicator_id, r.results[0].unit], [hex('e'), hex('1'), null]);
    await rejectsWith(china.searchChinaIndicators({ query: '' }), 'invalid_request');
    await rejectsWith(china.searchChinaIndicators({ query: 'x', frequency: 'weekly' }), 'invalid_request');
  } finally { m.restore(); }
});

test('browse lists the root, sub-catalogs, then the indicators of a leaf', async () => {
  const m = mockFetch(site());
  try {
    const root = await china.browseChinaCatalog({});
    assert.deepEqual([root.kind, root.catalog_id, root.catalogs.map((x) => x.name)], ['catalogs', hex('a'), ['价格指数', '金融']]);
    const leaves = await china.browseChinaCatalog({ catalog_id: hex('d') });
    assert.deepEqual(leaves.catalogs[1], { catalog_id: hex('f'), name: '全国居民消费价格分类指数 (上年同月=100) (2021-2025)', is_leaf: true, from_year: 2021, to_year: 2025 });
    const leaf = await china.browseChinaCatalog({ catalog_id: hex('e') });
    assert.equal(leaf.kind, 'indicators');
    assert.deepEqual(leaf.indicators[0], { indicator_id: hex('1'), name: '居民消费价格指数 (上年同月=100)', unit: '%' });
    await rejectsWith(china.browseChinaCatalog({ catalog_id: hex('0') }), 'not_found');
    await rejectsWith(china.browseChinaCatalog({ catalog_id: 'A01' }), 'invalid_request');
  } finally { m.restore(); }
});

test('indicator values: POST body, raw values, quarterly periods, unpublished periods skipped', async () => {
  const m = mockFetch(site());
  try {
    const q = await china.chinaIndicator({ catalog_id: hex('5'), indicator_id: hex('4'), frequency: 'quarterly', observations: 2 }, nov15);
    const call = m.calls[0];
    assert.deepEqual([call.href, call.init.method, call.init.headers['Content-Type']], [`${BASE}/stream/esData`, 'POST', 'application/json']);
    assert.deepEqual(JSON.parse(call.init.body), { cid: hex('5'), indicatorIds: [hex('4')], daCatalogId: '', das: [{ value: '000000000000', text: '全国' }], dts: ['202504SS-202604SS'], showType: '1' });
    assert.deepEqual(q.records, [{ period: '2026-Q3', value: 375367.2 }, { period: '2026-Q2', value: 361511.1 }]);
    assert.deepEqual([q.upstream_name, q.unit, q.area], ['国内生产总值当季值 (亿元)', '%', '全国']);
    await rejectsWith(china.chinaIndicator({ catalog_id: hex('5'), indicator_id: hex('6') }, nov15), 'not_found');
    await rejectsWith(china.chinaIndicator({ catalog_id: 'x', indicator_id: hex('6') }), 'invalid_request');
  } finally { m.restore(); }
});

test('shortcut merges base-period catalogs, newer catalog wins, offset applied, cached by body', async () => {
  const m = mockFetch(site());
  try {
    const r = await china.chinaSeries({ indicator: 'cpi_yoy', observations: 3 }, nov15);
    assert.equal(m.calls.length, 3);
    assert.equal(JSON.parse(m.calls[2].init.body).dts[0], '202606MM-202611MM');
    assert.deepEqual(r.records, [{ month: '2026-10', value: 0.7 }, { month: '2026-09', value: 0.9 }, { month: '2025-12', value: 0.9 }]);
    assert.deepEqual([r.resolved, r.ids.length, r.upstream_name, r.cached], ['stored_ids', 3, '居民消费价格指数 (上年同月=100) (%)', false]);
    assert.equal((await china.chinaSeries({ indicator: 'cpi_yoy', observations: 3 }, nov15)).cached, true);
    assert.equal(m.calls.length, 3);
    await rejectsWith(china.chinaSeries({ indicator: 'gdp' }), 'invalid_request');
  } finally { m.restore(); }
});

test('national CPI returns the latest three published months when regional cells precede the national total', async () => {
  const [cid, id] = CPI.at(-1);
  const rows = periods(id, '居民消费价格指数 (上年同月=100)', [
    ['202611', ''], ['202610', '100.7'], ['202609', '100.9'], ['202608', '100.8'], ['202607', '100.5'],
  ]);
  for (const period of rows) period.values.unshift({ ...period.values[0], da: '110000000000', value: '140' });
  const mock = mockFetch((url, init) => {
    assert.equal(url, BASE + '/stream/esData');
    assert.equal(init.method, 'POST');
    const query = JSON.parse(init.body);
    if (query.cid !== cid) return ok([]);
    assert.deepEqual(query.indicatorIds, [id]);
    assert.equal(query.das[0].value, '000000000000');
    return ok(rows);
  });
  try {
    const result = await china.chinaSeries({ indicator: 'cpi_yoy', observations: 3 }, nov15);
    assert.deepEqual(result.records, [
      { month: '2026-10', value: 0.7 }, { month: '2026-09', value: 0.9 }, { month: '2026-08', value: 0.8 },
    ]);
  } finally { mock.restore(); }
});

test('shortcut finds its ids again by catalog and indicator names when the stored ones return nothing', async () => {
  const m = mockFetch(site({ storedIdsGone: true }));
  try {
    const r = await china.chinaSeries({ indicator: 'cpi_yoy', observations: 2 }, nov15);
    assert.equal(r.resolved, 'by_name');
    assert.deepEqual(r.ids, [{ catalog_id: hex('f'), indicator_id: hex('3') }, { catalog_id: hex('e'), indicator_id: hex('1') }], 'oldest base period first, other areas skipped');
    assert.deepEqual(r.records, [{ month: '2026-10', value: 0.7 }, { month: '2025-12', value: 0.8 }]);
  } finally { m.restore(); }
});

test('NBS errors are mapped; a 403 is reported, not retried by name', async () => {
  const cases = [
    [() => new Response('<html>403 Forbidden reason:UrlACL</html>', { status: 403, headers: { 'content-type': 'text/html' } }), 'upstream_error'],
    [() => new Response('<html>访问受限</html>', { headers: { 'content-type': 'text/html' } }), 'mapping_failed'],
    [() => new Response(JSON.stringify({ success: false, state: 50000, message: '参数错误' })), 'upstream_error'],
    [() => ok([{ code: '2026SS', values: [cell(CPI[2][1], 'CPI', '1')] }]), 'mapping_failed'],
  ];
  for (const [route, code] of cases) {
    clearCache();
    const m = mockFetch(route);
    try { await rejectsWith(china.chinaIndicator({ catalog_id: CPI[2][0], indicator_id: CPI[2][1] }, nov15), code); } finally { m.restore(); }
  }
  const m = mockFetch(cases[0][0]);
  try { await rejectsWith(china.chinaSeries({ indicator: 'pmi_mfg' }, nov15), 'upstream_error'); assert.equal(m.calls.length, 1); } finally { m.restore(); }
});

test('China tools run through callTool and validate arguments', async () => {
  const m = mockFetch(site());
  try {
    const r = await callTool('search_china_indicators', { query: 'CPI', limit: 1 });
    assert.equal(r.isError, false);
    assert.match(r.content[0].text, /居民消费价格指数 \(上年同月=100\)\s+latest 2026-10 = 100\.7/);
    const bad = await callTool('get_china_indicator', { catalog_id: hex('5') });
    assert.equal(bad.isError, true);
  } finally { m.restore(); }
});

test('every NBS result and summary carries the attribution the NBS terms ask for', async () => {
  const m = mockFetch(site());
  try {
    assert.equal(china.NBS_ATTRIBUTION, '数据来自国家统计局网站（www.stats.gov.cn）');
    const results = [
      await china.searchChinaIndicators({ query: 'CPI' }),
      await china.browseChinaCatalog({}),
      await china.chinaIndicator({ catalog_id: CPI[2][0], indicator_id: CPI[2][1] }, nov15),
      await china.chinaSeries({ indicator: 'cpi_yoy' }, nov15),
    ];
    for (const r of results) assert.equal(r.attribution, china.NBS_ATTRIBUTION);
    for (const [name, args] of [['search_china_indicators', { query: 'CPI' }], ['get_china_indicator', { catalog_id: CPI[2][0], indicator_id: CPI[2][1] }], ['get_china_series', { indicator: 'cpi_yoy' }]]) {
      const r = await callTool(name, args);
      assert.equal(r.isError, false, name);
      assert.ok(r.content[0].text.trimEnd().endsWith(china.NBS_ATTRIBUTION), name);
    }
  } finally { m.restore(); }
});
