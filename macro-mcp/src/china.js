// China: National Bureau of Statistics database (data.stats.gov.cn), read through the endpoints its own website uses.
// NBS publishes no formal open API. The site was redesigned in 2026: the old easyquery.htm and codes such as A01010101
// now answer 403 from mainland networks too. Endpoints used here (checked against the live site on 2026-11-15):
//   GET  .../query?search=&code=1|2|3       site search; one row per indicator and period, with catalog id and indicator id
//   GET  .../new/queryIndexTreeAsync?code=&pid=   catalog tree, one level at a time
//   GET  .../new/queryIndicatorsByCid?cid=        indicators of a leaf catalog
//   POST .../stream/esData                  values of one indicator for a period range (area dimension is required)
// An indicator is addressed by its catalog id (a leaf of the site's catalog tree) and its indicator id, both 32-hex.
// Some series are split by base period into one catalog per span of years (CPI: 2016-2020, 2021-2025, 2026-).
// The ids are not meaningful and change with a redesign, so agents find them with searchChinaIndicators or
// browseChinaCatalog, and the named shortcuts in CHINA_SERIES find their ids again by name when the stored ones stop working.
import { getText, SourceError } from './upstream.js';

const BASE = 'https://data.stats.gov.cn/dg/website/publicrelease/web/external';
const PAGE_URL = 'https://data.stats.gov.cn/dg/website/page.html';
const NATIONAL = { value: '000000000000', text: '全国' };
const HEX32 = /^[0-9a-f]{32}$/;
export const NBS_SOURCE = 'National Bureau of Statistics of China';
// NBS's terms (www.stats.gov.cn 服务条款) require reposted or quoted content to name the NBS website and its address
// in a prominent place. Every NBS result carries this line so callers can quote it as is.
export const NBS_ATTRIBUTION = '数据来自国家统计局网站（www.stats.gov.cn）';

/** Site search `code`, esData period suffix, and the period label we return. */
export const CHINA_FREQUENCIES = {
  monthly: { code: '1', suffix: 'MM', page: 'monthData', example: '2026-10' },
  quarterly: { code: '2', suffix: 'SS', page: 'quarterData', example: '2026-Q3' },
  annual: { code: '3', suffix: 'YY', page: 'yearData', example: '2025' },
};

// The site search is fuzzy, knows only Chinese names, and only covers indicators whose latest period is already
// published (M2 is missing in the days before the PBoC release). Common short names are mapped; browsing is complete.
const ALIASES = {
  cpi: '居民消费价格指数', ppi: '工业生产者出厂价格指数', pmi: '采购经理指数', gdp: '国内生产总值',
  社零: '社会消费品零售总额', 固投: '固定资产投资',
};

const invalid = (message) => new SourceError('invalid_request', message);
const int = (value, name, min, max, fallback) => {
  const v = value === undefined || value === null ? fallback : Number(value);
  if (!Number.isInteger(v) || v < min || v > max) throw invalid(`${name} must be an integer from ${min} to ${max}.`);
  return v;
};
const squash = (s) => String(s ?? '').replace(/\s+/g, '').replace(/（/g, '(').replace(/）/g, ')');
const frequencyOf = (f) => {
  const key = String(f || 'monthly');
  if (!CHINA_FREQUENCIES[key]) throw invalid(`frequency must be one of ${Object.keys(CHINA_FREQUENCIES).join(', ')}.`);
  return key;
};

/** GET or POST one NBS endpoint and return its parsed `{success:true,data}` body. */
async function nbs(url, init, o) {
  let res;
  try {
    res = await getText(url, { accept: 'application/json, text/plain, */*', timeoutMs: 60000, ...o, ...init });
  } catch (error) {
    if (error instanceof SourceError && /HTTP 403/.test(error.message)) throw new SourceError('upstream_error', 'data.stats.gov.cn refused the request (HTTP 403). Its firewall blocks some networks outside mainland China; from a mainland network it means the site has moved its query endpoints again.');
    throw error;
  }
  let body;
  try { body = JSON.parse(res.text); }
  catch { throw new SourceError('mapping_failed', 'data.stats.gov.cn did not return JSON. It refuses some networks outside mainland China.'); }
  if (body?.success !== true) throw new SourceError('upstream_error', `data.stats.gov.cn returned an error: ${String(body?.message ?? '').slice(0, 200)}`);
  return { data: body.data, cached: res.cached };
}

/** Period code `yyyyNN<suffix>` for `back` periods before `now` (UTC). */
function periodCode(freq, now, back) {
  const y = now.getUTCFullYear(), m = now.getUTCMonth();
  if (freq === 'annual') return `${y - back}YY`;
  if (freq === 'quarterly') { const q = y * 4 + Math.floor(m / 3) - back; return `${Math.floor(q / 4)}${String((q % 4) + 1).padStart(2, '0')}SS`; }
  const d = new Date(Date.UTC(y, m - back, 1));
  return `${d.getUTCFullYear()}${String(d.getUTCMonth() + 1).padStart(2, '0')}MM`;
}

/** `202610MM` → `2026-10`, `202603SS` → `2026-Q3`, `2025YY` → `2025`; null if the code does not fit. */
function periodLabel(freq, code) {
  const s = String(code);
  if (freq === 'annual') { const m = /^(\d{4})YY$/.exec(s); return m ? m[1] : null; }
  const m = /^(\d{4})(\d{2})(MM|SS)$/.exec(s);
  if (!m || m[3] !== CHINA_FREQUENCIES[freq].suffix) return null;
  const n = Number(m[2]);
  if (freq === 'quarterly') return n >= 1 && n <= 4 ? `${m[1]}-Q${n}` : null;
  return n >= 1 && n <= 12 ? `${m[1]}-${m[2]}` : null;
}

const nameAndUnit = (cell) => {
  const name = String(cell?.i_showname ?? cell?.show_name ?? '').trim();
  const unit = String(cell?.du_name ?? '').trim();
  return { name, unit: unit && unit !== '无' ? unit : null };
};

/**
 * Values for one or more parts (catalog id + indicator id) of the same indicator, oldest part first; a later part
 * wins a period both cover. Returns periods newest first.
 */
async function readParts(parts, freq, observations, o) {
  const { now = new Date(), ...http } = o;
  // A few spare periods: the latest one or two are usually not published yet.
  const dts = `${periodCode(freq, now, observations + 2)}-${periodCode(freq, now, 0)}`;
  const values = new Map();
  let name = null, unit = null, cached = true;
  for (const [cid, id] of parts) {
    const { data, cached: hit } = await nbs(`${BASE}/stream/esData`, {
      ttlSeconds: 6 * 3600, jsonBody: { cid, indicatorIds: [id], daCatalogId: '', das: [NATIONAL], dts: [dts], showType: '1' },
    }, http);
    cached &&= hit;
    if (!Array.isArray(data)) throw new SourceError('mapping_failed', 'Unexpected data.stats.gov.cn response.');
    for (const period of data) {
      const cell = (period?.values || []).find((v) => v?._id === id && v.da === NATIONAL.value);
      if (!cell) continue;
      ({ name, unit } = nameAndUnit(cell));
      const raw = String(cell.value ?? '').trim();
      if (!raw) continue;
      const label = periodLabel(freq, period.code);
      if (!label) throw new SourceError('mapping_failed', `Unrecognised data.stats.gov.cn period code ${period.code}.`);
      const value = Number(raw.replace(/,/g, ''));
      if (!Number.isFinite(value)) throw new SourceError('mapping_failed', `data.stats.gov.cn returned a non-numeric value for ${label}.`);
      values.set(label, value);
    }
  }
  const records = [...values].map(([period, value]) => ({ period, value })).sort((a, b) => b.period.localeCompare(a.period)).slice(0, observations);
  return { name, unit, cached, records };
}

/** @param {{query:string,frequency?:string,limit?:number}} p @param {{maxAgeSeconds?:number}} [o] */
export async function searchChinaIndicators(p = {}, o = {}) {
  const raw = String(p.query ?? '').trim();
  if (!raw || raw.length > 50) throw invalid('query must be 1 to 50 characters, e.g. 社会消费品零售总额, 工业增加值, 居民消费价格指数.');
  const freq = frequencyOf(p.frequency);
  const limit = int(p.limit, 'limit', 1, 50, 10);
  const query = ALIASES[raw.toLowerCase()] ?? raw;
  const url = `${BASE}/query?search=${encodeURIComponent(query)}&code=${CHINA_FREQUENCIES[freq].code}&pagenum=1&pageSize=100`;
  const { data, cached } = await nbs(url, { ttlSeconds: 24 * 3600 }, o);
  if (!Array.isArray(data?.data)) throw new SourceError('mapping_failed', 'Unexpected data.stats.gov.cn search response.');
  // One row per indicator and period, newest first; keep the first (latest) national row of each indicator.
  const seen = new Map();
  for (const row of data.data) {
    if (row?.da !== NATIONAL.value || !HEX32.test(String(row.cid)) || !HEX32.test(String(row.indic_id))) continue;
    const key = `${row.cid}|${row.indic_id}`;
    if (seen.has(key)) continue;
    const { name, unit } = nameAndUnit(row);
    seen.set(key, {
      name, unit, frequency: freq, catalog_id: row.cid, indicator_id: row.indic_id,
      latest_period: periodLabel(freq, `${row.dt}${CHINA_FREQUENCIES[freq].suffix}`) ?? String(row.dt ?? ''),
      latest_value: row.value === '' || row.value === undefined ? null : Number(row.value),
      definition: row.explain ? String(row.explain).slice(0, 300) : null,
    });
  }
  // NBS ranks loosely; put names that contain the query first, exact names before them.
  const q = squash(query);
  const rank = (x) => { const n = squash(x.name).replace(/\([^)]*\)$/, ''); return n === q ? 0 : squash(x.name).includes(q) ? 1 : 2; };
  const all = [...seen.values()].map((x, i) => ({ x, i, r: rank(x) })).sort((a, b) => a.r - b.r || a.i - b.i);
  const results = all.slice(0, limit).map(({ x, r }) => ({ ...x, name_contains_query: r < 2 }));
  return {
    source: NBS_SOURCE, attribution: NBS_ATTRIBUTION, url: `${PAGE_URL}#/pc/national/search?s=${encodeURIComponent(query)}`,
    query, frequency: freq, total: all.length, cached, results,
    note: 'Pass catalog_id and indicator_id to get_china_indicator. The site search is fuzzy and skips indicators whose latest period is not yet published; when the one you want is missing, use browse_china_catalog. Names are NBS\'s own; the same sub-index name (e.g. 新订单指数) can sit in several catalogs, so check the name and latest value. Results are national figures.',
  };
}

/** @param {{catalog_id:string,indicator_id:string,frequency?:string,observations?:number}} p @param {{maxAgeSeconds?:number,now?:Date}} [o] */
export async function chinaIndicator(p = {}, o = {}) {
  const cid = String(p.catalog_id ?? ''), id = String(p.indicator_id ?? '');
  if (!HEX32.test(cid) || !HEX32.test(id)) throw invalid('catalog_id and indicator_id are the 32-character ids from search_china_indicators or browse_china_catalog.');
  const freq = frequencyOf(p.frequency);
  const n = int(p.observations, 'observations', 1, 240, 24);
  const r = await readParts([[cid, id]], freq, n, o);
  if (!r.records.length) throw new SourceError('not_found', `data.stats.gov.cn returned no ${freq} national data for this indicator in the last ${n} periods. Check the ids and the frequency with browse_china_catalog.`);
  return {
    source: NBS_SOURCE, attribution: NBS_ATTRIBUTION, url: `${PAGE_URL}#/pc/national/${CHINA_FREQUENCIES[freq].page}`, catalog_id: cid, indicator_id: id, frequency: freq,
    upstream_name: r.name, unit: r.unit, area: '全国', order: 'newest first', cached: r.cached, records: r.records,
    note: 'Values exactly as NBS publishes them. Indices with the same period of the previous year = 100 are not converted to percent changes.',
  };
}

/** Children of a catalog (or the frequency's root). Empty for a leaf. */
async function catalogChildren(freq, pid, o) {
  const { data, cached } = await nbs(`${BASE}/new/queryIndexTreeAsync?code=${CHINA_FREQUENCIES[freq].code}&pid=${pid}`, { ttlSeconds: 24 * 3600 }, o);
  if (!Array.isArray(data)) throw new SourceError('mapping_failed', 'Unexpected data.stats.gov.cn catalog response.');
  return { cached, nodes: data.filter((n) => HEX32.test(String(n?._id))).map((n) => ({ catalog_id: n._id, name: String(n.name ?? n._name ?? '').trim(), is_leaf: Boolean(n.isLeaf), from_year: n.sdate ? Number(n.sdate) : null, to_year: n.edate ? Number(n.edate) : null })) };
}

/** Indicators in a leaf catalog. */
async function catalogIndicators(cid, o) {
  const { data, cached } = await nbs(`${BASE}/new/queryIndicatorsByCid?cid=${cid}&dt=&name=`, { ttlSeconds: 24 * 3600 }, o);
  if (!Array.isArray(data?.list)) throw new SourceError('mapping_failed', 'Unexpected data.stats.gov.cn indicator list.');
  return { cached, indicators: data.list.filter((x) => HEX32.test(String(x?._id))).map((x) => ({ indicator_id: x._id, ...nameAndUnit(x) })) };
}

/** Root catalog id of a frequency (月度数据 and so on). */
async function rootId(freq, o) {
  const { nodes } = await catalogChildren(freq, '', o);
  if (!nodes.length) throw new SourceError('mapping_failed', 'data.stats.gov.cn returned an empty catalog root.');
  return nodes[0].catalog_id;
}

/** @param {{frequency?:string,catalog_id?:string}} p @param {{maxAgeSeconds?:number}} [o] */
export async function browseChinaCatalog(p = {}, o = {}) {
  const freq = frequencyOf(p.frequency);
  const { now: _now, ...http } = o;
  const given = p.catalog_id === undefined || p.catalog_id === null || p.catalog_id === '' ? null : String(p.catalog_id);
  if (given !== null && !HEX32.test(given)) throw invalid('catalog_id is a 32-character id from an earlier browse_china_catalog or search_china_indicators result; omit it for the top level.');
  const cid = given ?? await rootId(freq, http);
  const children = await catalogChildren(freq, cid, http);
  const base = { source: NBS_SOURCE, attribution: NBS_ATTRIBUTION, url: `${PAGE_URL}#/pc/national/${CHINA_FREQUENCIES[freq].page}`, frequency: freq, catalog_id: cid };
  if (children.nodes.length) return { ...base, kind: 'catalogs', cached: children.cached, catalogs: children.nodes, note: 'Browse a catalog by its catalog_id; leaves (is_leaf true) list indicators. from_year/to_year mark catalogs that hold one base period.' };
  const list = await catalogIndicators(cid, http);
  if (!list.indicators.length) throw new SourceError('not_found', `Catalog ${cid} has no sub-catalogs or indicators for ${freq} data. Check the id and the frequency.`);
  return { ...base, kind: 'indicators', cached: list.cached, indicators: list.indicators, note: 'Pass catalog_id and indicator_id to get_china_indicator.' };
}

// ---------- Named shortcuts ----------
// `parts` are the stored ids (oldest catalog first). If they stop working, the ids are found again by name: walk
// `path` down from the monthly root, take every leaf whose name starts with `leaf` (one per base period), and pick
// the indicator named `upstream`. The site search is not used for this: it only covers indicators whose latest
// month is already out. `offset` turns "previous year = 100" into percent.
const yoyNote = 'NBS publishes the index with the same month of the previous year = 100; 100 is subtracted.';
const pbcNote = 'People\'s Bank of China data, published by NBS.';
export const CHINA_SERIES = {
  cpi_yoy: { upstream: '居民消费价格指数 (上年同月=100)', path: ['价格指数', '居民消费价格分类指数 (上年同月=100)'], leaf: '全国居民消费价格分类指数 (上年同月=100)', offset: -100, name: 'China CPI, year on year', unit: 'percent', note: yoyNote,
    parts: [['9d4eec43537742a7ab5d63db97fa2f51', 'e5c318ffdbbc4d38898e52b52267eb25'], ['809d2522b0fe4be89142650341b19083', '4ae9047687934a6390984c21d6ddab96'], ['5c7452825c7c4dcba391db5ca7f335c5', '53180dfb9c14411ba4b762307c85920c']] },
  ppi_yoy: { upstream: '工业生产者出厂价格指数 (上年同月=100)', path: ['价格指数', '工业生产者出厂价格分类指数'], leaf: '工业生产者出厂价格指数 (上年同月=100)', offset: -100, name: 'China PPI, year on year', unit: 'percent', note: yoyNote,
    parts: [['60e8b361f11c4a878c652a6487a25561', '150633e52b9a470a9a9fd1b296dd6c5b']] },
  pmi_mfg: { upstream: '制造业采购经理指数 (%)', path: ['采购经理指数'], leaf: '制造业采购经理指数', offset: 0, name: 'China manufacturing PMI', unit: 'index', note: '50 separates expansion from contraction.',
    parts: [['93ffbb1aa85740d3aa2618371508b606', 'a09aa989bdcf4cffa2021795722eb916']] },
  m2: { upstream: '货币和准货币 (M2) 供应量_期末值 (亿元)', path: ['金融'], leaf: '货币供应量', offset: 0, name: 'China M2, end of month', unit: '100 million CNY', note: pbcNote,
    parts: [['82130c6621a745cda3d64b090e733383', 'f3c0ae453a54424489af41de315ec592']] },
  m2_yoy: { upstream: '货币和准货币 (M2) 供应量_同比增长 (%)', path: ['金融'], leaf: '货币供应量', offset: 0, name: 'China M2, year on year', unit: 'percent', note: pbcNote,
    parts: [['82130c6621a745cda3d64b090e733383', 'e03f2232631f41cd9d754a7d7feb4a81']] },
};

/** Find the ids again by catalog and indicator names; oldest base period first. */
async function resolveByName(spec, o) {
  const { now: _now, ...http } = o;
  let pid = await rootId('monthly', http);
  for (const step of spec.path) {
    const { nodes } = await catalogChildren('monthly', pid, http);
    const next = nodes.find((n) => squash(n.name) === squash(step));
    if (!next) return [];
    pid = next.catalog_id;
  }
  const { nodes } = await catalogChildren('monthly', pid, http);
  const leaves = nodes.filter((n) => n.is_leaf && squash(n.name).startsWith(squash(spec.leaf))).sort((x, y) => (x.from_year ?? 0) - (y.from_year ?? 0));
  const parts = [];
  for (const leaf of leaves) {
    const { indicators } = await catalogIndicators(leaf.catalog_id, http);
    const hit = indicators.find((x) => squash(x.name) === squash(spec.upstream) || squash(`${x.name} (${x.unit})`) === squash(spec.upstream));
    if (hit) parts.push([leaf.catalog_id, hit.indicator_id]);
  }
  return parts;
}

/** @param {{indicator:string,observations?:number}} p @param {{maxAgeSeconds?:number,now?:Date}} [o] */
export async function chinaSeries(p = {}, o = {}) {
  const key = String(p.indicator || '');
  const spec = CHINA_SERIES[key];
  if (!spec) throw invalid(`indicator must be one of ${Object.keys(CHINA_SERIES).join(', ')}. For other NBS indicators use search_china_indicators or browse_china_catalog.`);
  const n = int(p.observations, 'observations', 1, 120, 24);
  let parts = spec.parts, resolved = 'stored_ids', r;
  try {
    r = await readParts(parts, 'monthly', n, o);
    if (!r.records.length) throw new SourceError('not_found', 'no data');
  } catch (error) {
    // 403, timeouts and rate limits are about the network, not the ids: report them as they are.
    if (!(error instanceof SourceError) || error.retryable || !['not_found', 'upstream_error', 'mapping_failed'].includes(error.code) || /HTTP 403|did not return JSON/.test(error.message)) throw error;
    parts = await resolveByName(spec, o);
    if (!parts.length) throw new SourceError('not_found', `The stored NBS ids for ${key} returned no data and "${spec.upstream}" is no longer under ${spec.path.join(' > ')}. Try browse_china_catalog.`);
    resolved = 'by_name';
    r = await readParts(parts, 'monthly', n, o);
    if (!r.records.length) throw new SourceError('not_found', `data.stats.gov.cn returned no data for ${key} in the last ${n} months.`);
  }
  return {
    source: NBS_SOURCE, attribution: NBS_ATTRIBUTION, url: `${PAGE_URL}#/pc/national/monthData`, indicator: key, name: spec.name,
    upstream_name: r.name ? `${r.name}${r.unit && !r.name.includes(r.unit) ? ` (${r.unit})` : ''}` : null,
    unit: spec.unit, note: spec.note, ids: parts.map(([cid, id]) => ({ catalog_id: cid, indicator_id: id })), resolved,
    order: 'newest first', cached: r.cached,
    records: r.records.map((x) => ({ month: x.period, value: Math.round((x.value + spec.offset) * 1e6) / 1e6 })),
  };
}
