// MCP tool definitions. Each tool validates input in its source function and returns
// a short human summary (text) plus the full result (structuredContent).
import * as src from './sources.js';
import * as china from './china.js';
import { formatSnapshot, GROUPS, PANELS, PANEL_IDS, snapshot } from './snapshot.js';
import { SourceError } from './upstream.js';

const readOnly = { readOnlyHint: true, destructiveHint: false, idempotentHint: true, openWorldHint: true };
const maxAge = { max_age_seconds: { type: 'integer', minimum: 0, description: 'Accept cached data up to this age. 0 forces a fresh upstream request.' } };
const obj = (properties, required = []) => ({ type: 'object', properties: { ...properties, ...maxAge }, required, additionalProperties: false });

const lines = (r, pick, n = 5) => r.records.slice(0, n).map(pick).join('\n');

export const TOOLS = [
  {
    name: 'macro_snapshot', title: 'Macro snapshot',
    description: `Latest US macro dashboard in one call: SOFR, EFFR (with target range), broad dollar index and USD/CNY (these two need WHICHAPI_API_KEY), 10-year Treasury (with full curve and 2s10s), mainland China and Japan Treasury holdings, debt-limit room, latest Section 232 notices. Each panel has value, as_of (source date), change vs previous observation and a short series. Failed panels are reported individually. Panel ids: ${PANEL_IDS.join(', ')}.`,
    inputSchema: obj({ panels: { type: 'array', items: { type: 'string', enum: PANEL_IDS }, minItems: 1, description: 'Subset of panels; default all.' } }),
    run: async (a) => { const s = await snapshot({ panels: a.panels, maxAgeSeconds: a.max_age_seconds }); return { text: formatSnapshot(s), data: s, isError: s.status === 'error' }; },
  },
  {
    name: 'list_macro_panels', title: 'List macro panels',
    description: 'List the macro snapshot panels with their group, unit, source and interpretation notes. No network access.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
    run: async () => { const data = { groups: GROUPS, panels: PANELS.map(({ id, group, label, unit, source, note }) => ({ id, group, label, unit, source, note })) }; return { text: data.panels.map((p) => `${p.id} — ${p.label} [${p.unit}] (${p.source})`).join('\n'), data }; },
  },
  {
    name: 'get_reference_rates', title: 'New York Fed reference rates',
    description: 'Daily SOFR, TGCR, BGCR, EFFR or OBFR from the New York Fed with percentiles and volume; EFFR includes the FOMC target range. Newest first.',
    inputSchema: obj({ rate: { type: 'string', enum: Object.keys(src.NYFED_RATES), default: 'SOFR' }, observations: { type: 'integer', minimum: 1, maximum: 250, default: 10 } }),
    run: async (a) => { const r = await src.referenceRates(a, { maxAgeSeconds: a.max_age_seconds }); return { text: `${a.rate || 'SOFR'} (percent), newest first:\n` + lines(r, (x) => `${x.effective_date}  ${x.rate_percent}${x.target_rate_from !== null ? `  target ${x.target_rate_from}–${x.target_rate_to}` : ''}`), data: r }; },
  },
  {
    name: 'get_fred_series', title: 'FRED series',
    description: 'Observations of a public FRED series, newest first, read through the WhichAPI hosted service (needs WHICHAPI_API_KEY). units transforms on FRED\'s side, e.g. pc1 = percent change from a year ago: use it for inflation rates instead of computing them. Examples: DGS10, T10Y2Y, SOFR, CPIAUCSL, UNRATE, DTWEXBGS (broad dollar, not DXY), DEXCHUS (CNY per USD). ICE BofA series (ids starting with BAML) are not served. Missing days are null.',
    inputSchema: obj({ series_id: { type: 'string', pattern: '^[A-Za-z0-9_]{1,40}$' }, start_date: { type: 'string', pattern: '^\\d{4}-\\d{2}-\\d{2}$', description: 'Default: one year ago.' }, end_date: { type: 'string', pattern: '^\\d{4}-\\d{2}-\\d{2}$' }, limit: { type: 'integer', minimum: 1, maximum: 5000, default: 100 }, units: { type: 'string', enum: Object.keys(src.FRED_UNITS), default: 'lin', description: 'FRED transformation: lin levels, chg change, ch1 change from a year ago, pch percent change, pc1 percent change from a year ago, pca compounded annual rate, cch/cca continuously compounded, log natural log.' } }, ['series_id']),
    run: async (a) => { const r = await src.fredSeries(a, { maxAgeSeconds: a.max_age_seconds }); return { text: `${r.series_id}, newest first (${r.records.length} observations):\n` + lines(r, (x) => `${x.observation_date}  ${x.value ?? 'missing'}`, 10), data: r }; },
  },
  {
    name: 'search_china_indicators', title: 'Search China NBS indicators',
    description: 'Find National Bureau of Statistics of China indicators by Chinese name, e.g. 社会消费品零售总额, 居民消费价格指数, 国内生产总值, 规模以上工业增加值; CPI, PPI, PMI and GDP are mapped to their Chinese names. Returns catalog_id and indicator_id for get_china_indicator, NBS\'s own name and unit, and the latest national value. The NBS site search is fuzzy and skips indicators whose latest period is not yet published (M2 before the PBoC release, for example); when it misses, use browse_china_catalog.',
    inputSchema: obj({ query: { type: 'string', minLength: 1, maxLength: 50 }, frequency: { type: 'string', enum: Object.keys(china.CHINA_FREQUENCIES), default: 'monthly' }, limit: { type: 'integer', minimum: 1, maximum: 50, default: 10 } }, ['query']),
    run: async (a) => { const r = await china.searchChinaIndicators(a, { maxAgeSeconds: a.max_age_seconds }); if (!r.results.length) return { text: `No ${r.frequency} NBS indicator matches "${r.query}". Try another Chinese name or browse_china_catalog.`, data: r }; return { text: `${r.total} ${r.frequency} NBS indicators for "${r.query}"; showing ${r.results.length}:\n` + r.results.map((x) => `${x.name}  latest ${x.latest_period} = ${x.latest_value ?? 'missing'}  catalog_id ${x.catalog_id} indicator_id ${x.indicator_id}`).join('\n') + `\n${r.attribution}`, data: r }; },
  },
  {
    name: 'browse_china_catalog', title: 'Browse the China NBS catalog',
    description: 'Walk the National Bureau of Statistics of China database catalog one level at a time. Without catalog_id, lists the top-level catalogs for the frequency (monthly: 价格指数, 工业, 能源, 固定资产投资, 房地产, 国内贸易, 对外经济, 采购经理指数, 财政, 金融 …). A catalog lists its sub-catalogs; a leaf lists its indicators with indicator_id. Complete, unlike the site search. Some series are split by base period into one catalog per span of years (from_year/to_year).',
    inputSchema: obj({ frequency: { type: 'string', enum: Object.keys(china.CHINA_FREQUENCIES), default: 'monthly' }, catalog_id: { type: 'string', pattern: '^[0-9a-f]{32}$', description: 'Omit for the top level.' } }),
    run: async (a) => { const r = await china.browseChinaCatalog(a, { maxAgeSeconds: a.max_age_seconds }); return { text: r.kind === 'catalogs' ? `${r.catalogs.length} ${r.frequency} catalogs:\n` + r.catalogs.map((x) => `${x.name}${x.from_year || x.to_year ? ` (${x.from_year ?? ''}–${x.to_year ?? ''})` : ''}${x.is_leaf ? '  [leaf]' : ''}  catalog_id ${x.catalog_id}`).join('\n') : `${r.indicators.length} indicators in catalog ${r.catalog_id}:\n` + r.indicators.map((x) => `${x.name}  indicator_id ${x.indicator_id}`).join('\n'), data: r }; },
  },
  {
    name: 'get_china_indicator', title: 'China NBS indicator values',
    description: 'National values of one National Bureau of Statistics of China indicator, newest first, exactly as NBS publishes them (an index with the previous year = 100 stays an index). Get catalog_id and indicator_id from search_china_indicators or browse_china_catalog, and pass the same frequency. Periods: monthly 2026-10, quarterly 2026-Q3, annual 2025. upstream_name is NBS\'s own name; quote it with the source.',
    inputSchema: obj({ catalog_id: { type: 'string', pattern: '^[0-9a-f]{32}$' }, indicator_id: { type: 'string', pattern: '^[0-9a-f]{32}$' }, frequency: { type: 'string', enum: Object.keys(china.CHINA_FREQUENCIES), default: 'monthly' }, observations: { type: 'integer', minimum: 1, maximum: 240, default: 24 } }, ['catalog_id', 'indicator_id']),
    run: async (a) => { const r = await china.chinaIndicator(a, { maxAgeSeconds: a.max_age_seconds }); return { text: `${r.upstream_name}${r.unit && !String(r.upstream_name).includes(r.unit) ? ` [${r.unit}]` : ''} (NBS, 全国), newest first:\n` + r.records.slice(0, 12).map((x) => `${x.period}  ${x.value}`).join('\n') + `\n${r.attribution}`, data: r }; },
  },
  {
    name: 'get_china_series', title: 'China headline series (NBS)',
    description: 'Shortcut for the most used monthly China series from the National Bureau of Statistics, newest first: cpi_yoy and ppi_yoy (percent, year on year; NBS publishes them with the previous year = 100), pmi_mfg (index, 50 = no change), m2 (100 million CNY) and m2_yoy (percent). If NBS changes its ids, the tool finds them again by name (resolved = by_name). For any other NBS indicator use search_china_indicators.',
    inputSchema: obj({ indicator: { type: 'string', enum: Object.keys(china.CHINA_SERIES) }, observations: { type: 'integer', minimum: 1, maximum: 120, default: 24 } }, ['indicator']),
    run: async (a) => { const r = await china.chinaSeries(a, { maxAgeSeconds: a.max_age_seconds }); return { text: `${r.name} [${r.unit}]${r.upstream_name ? ` — NBS: ${r.upstream_name}` : ''}, newest first:\n` + lines(r, (x) => `${x.month}  ${x.value}`, 12) + `\n${r.attribution}`, data: r }; },
  },
  {
    name: 'get_treasury_curve', title: 'US Treasury par yield curve',
    description: 'Daily US Treasury par yield curve (1M to 30Y, percent), newest day first. Years 2025 and later.',
    inputSchema: obj({ year: { type: 'integer', minimum: 2025, maximum: 2100, description: 'Default: current year.' }, limit: { type: 'integer', minimum: 1, maximum: 260, default: 30 } }),
    run: async (a) => { const r = await src.treasuryCurve(a, { maxAgeSeconds: a.max_age_seconds }); const d = r.records[0]; return { text: d ? `Treasury par curve ${d.date}: ` + Object.entries(d.yields).filter(([, v]) => v !== null).map(([t, v]) => `${t} ${v}`).join(', ') : 'No curve data for that year yet.', data: r }; },
  },
  {
    name: 'get_tic_holdings', title: 'TIC foreign holdings of US securities',
    description: 'Monthly foreign holdings, net purchases and valuation changes of long-term US securities (Treasuries, agencies, corporates, equities) for one country, millions of USD, newest month first. Use exact TIC names, e.g. "China, Mainland", "Japan", "United Kingdom", "Cayman Islands".',
    inputSchema: obj({ country: { type: 'string', default: 'China, Mainland' }, limit: { type: 'integer', minimum: 1, maximum: 240, default: 24 } }),
    run: async (a) => { const r = await src.ticHoldings(a, { maxAgeSeconds: a.max_age_seconds }); return { text: `${r.records[0].country} Treasury holdings (USD mn), newest first:\n` + lines(r, (x) => `${x.month}  ${x.treas_pos}  (net ${x.treas_net})`, 6), data: r }; },
  },
  {
    name: 'get_debt_limit', title: 'US debt limit headroom',
    description: 'Latest statutory debt limit, debt subject to the limit and the remaining room, derived from the Daily Treasury Statement (millions of USD). Not an official X-date estimate.',
    inputSchema: obj({}),
    run: async (a) => { const r = await src.debtLimit({}, { maxAgeSeconds: a.max_age_seconds }); const x = r.records[0]; return { text: `${x.record_date}: limit ${x.statutory_debt_limit}, subject ${x.debt_subject_to_limit}, headroom ${x.headroom} (USD mn, derived)`, data: r }; },
  },
  {
    name: 'search_federal_register', title: 'Search the Federal Register',
    description: 'Search US Federal Register rules, proposed rules, notices and presidential documents (tariffs, Section 232, export controls, sanctions). Quote phrases to match exactly. Optional agency slug (e.g. industry-and-security-bureau, international-trade-administration), type and publication date filter.',
    inputSchema: obj({ term: { type: 'string', minLength: 2, maxLength: 200 }, order: { type: 'string', enum: ['relevance', 'newest', 'oldest'], default: 'relevance' }, limit: { type: 'integer', minimum: 1, maximum: 100, default: 10 }, agency: { type: 'string' }, type: { type: 'string', enum: ['rule', 'proposed_rule', 'notice', 'presidential'] }, published_since: { type: 'string', pattern: '^\\d{4}-\\d{2}-\\d{2}$' } }, ['term']),
    run: async (a) => { const r = await src.federalRegister(a, { maxAgeSeconds: a.max_age_seconds }); return { text: `${r.total ?? r.records.length} matches; showing ${r.records.length}:\n` + lines(r, (x) => `${x.publication_date}  ${x.type}  ${x.title}  ${x.html_url}`, 10), data: r }; },
  },
  {
    name: 'get_company_concept', title: 'SEC company financial concept',
    description: 'Reported XBRL values for one company and one US-GAAP concept, newest period first. Capex: PaymentsToAcquirePropertyPlantAndEquipment; total debt: DebtLongtermAndShorttermCombinedAmount or LongTermDebt. CIKs: Alphabet 0001652044, Microsoft 0000789019, Amazon 0001018724, Meta 0001326801, Oracle 0001341439, NVIDIA 0001045810. Requires SEC_USER_AGENT with a contact email.',
    inputSchema: obj({ cik: { type: 'string', pattern: '^(CIK)?\\d{1,10}$' }, concept: { type: 'string', pattern: '^[A-Za-z0-9_]{2,100}$' }, unit: { type: 'string', default: 'USD' }, forms: { type: 'array', items: { type: 'string' }, description: 'e.g. ["10-K"] for annual values only.' }, limit: { type: 'integer', minimum: 1, maximum: 200, default: 12 } }, ['cik', 'concept']),
    run: async (a) => { const r = await src.companyConcept(a, { maxAgeSeconds: a.max_age_seconds }); return { text: `${r.entity_name} ${r.concept} (${r.unit}), newest first:\n` + lines(r, (x) => `${x.period_start ?? ''}→${x.period_end}  ${x.value}  ${x.form} filed ${x.filed}`, 8), data: r }; },
  },
];

/** Run a tool by name. Returns an MCP CallToolResult. */
export async function callTool(name, args = {}) {
  const tool = TOOLS.find((t) => t.name === name);
  if (!tool) return null;
  if (args === null || typeof args !== 'object' || Array.isArray(args)) args = {};
  const allowed = Object.keys(tool.inputSchema.properties);
  const extra = Object.keys(args).find((k) => !allowed.includes(k));
  try {
    if (extra) throw new SourceError('invalid_request', `Unknown argument "${extra}". Accepted: ${allowed.join(', ') || 'none'}.`);
    if (args.max_age_seconds !== undefined && (!Number.isInteger(args.max_age_seconds) || args.max_age_seconds < 0)) throw new SourceError('invalid_request', 'max_age_seconds must be a non-negative integer.');
    const out = await tool.run(args);
    return { content: [{ type: 'text', text: out.text }], structuredContent: out.data, isError: Boolean(out.isError) };
  } catch (error) {
    const e = error instanceof SourceError ? { code: error.code, message: error.message, retryable: error.retryable, ...(error.retryAfterMs !== undefined ? { retry_after_ms: error.retryAfterMs } : {}) } : { code: 'internal_error', message: String(error?.message || error), retryable: false };
    return { content: [{ type: 'text', text: `${e.code}: ${e.message}` }], structuredContent: { error: e }, isError: true };
  }
}
