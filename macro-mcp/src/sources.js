// One function per official source. Each validates its inputs, checks the upstream layout it expects,
// and returns plain records with documented units. Missing upstream values become null, never guesses.
import { getDelimited, getJson, SourceError } from './upstream.js';

const DATE = /^\d{4}-\d{2}-\d{2}$/;
const num = (v) => {
  if (v === null || v === undefined || v === '' || v === '.' || v === 'N/A') return null;
  const n = typeof v === 'number' ? v : Number(String(v).replace(/,/g, ''));
  return Number.isFinite(n) ? n : null;
};
const invalid = (message) => new SourceError('invalid_request', message);
const int = (value, name, min, max, fallback) => {
  const v = value === undefined || value === null ? fallback : Number(value);
  if (!Number.isInteger(v) || v < min || v > max) throw invalid(`${name} must be an integer from ${min} to ${max}.`);
  return v;
};

// ---------- New York Fed reference rates ----------
export const NYFED_RATES = { SOFR: ['secured', 'sofr'], TGCR: ['secured', 'tgcr'], BGCR: ['secured', 'bgcr'], EFFR: ['unsecured', 'effr'], OBFR: ['unsecured', 'obfr'] };
/** @param {{rate?:string,observations?:number}} p @param {{maxAgeSeconds?:number}} [o] */
export async function referenceRates(p = {}, o = {}) {
  const rate = String(p.rate || 'SOFR').toUpperCase();
  const kind = NYFED_RATES[rate];
  if (!kind) throw invalid(`rate must be one of ${Object.keys(NYFED_RATES).join(', ')}.`);
  const n = int(p.observations, 'observations', 1, 250, 10);
  const { data, cached } = await getJson(`https://markets.newyorkfed.org/api/rates/${kind[0]}/${kind[1]}/last/${n}.json`, { ttlSeconds: 900, ...o });
  if (!Array.isArray(data?.refRates)) throw new SourceError('mapping_failed', 'Unexpected New York Fed response.');
  const records = data.refRates.map((r) => ({
    effective_date: r.effectiveDate ?? null, rate_type: r.type ?? rate, rate_percent: num(r.percentRate),
    percentile_1: num(r.percentPercentile1), percentile_25: num(r.percentPercentile25), percentile_75: num(r.percentPercentile75), percentile_99: num(r.percentPercentile99),
    target_rate_from: num(r.targetRateFrom), target_rate_to: num(r.targetRateTo), volume_billions_usd: num(r.volumeInBillions), revised: Boolean(r.revisionIndicator),
  }));
  return { source: 'Federal Reserve Bank of New York', url: 'https://www.newyorkfed.org/markets/reference-rates', unit: 'percent', order: 'newest first', cached, records };
}

// ---------- FRED, through the WhichAPI hosted service ----------
// This package does not call FRED itself. FRED's terms ask for its registered API instead of scraping, so FRED
// series are read through the WhichAPI hosted service, which holds the FRED key. Users need a WhichAPI key.
export const WHICHAPI_DEFAULT_URL = 'https://whichapi-macro.onrender.com';
// ICE BofA series (ids starting with BAML) are ICE Data Indices' copyright and need ICE's written permission.
const ICE_SERIES = /^BAML/;
// FRED-side transformations the WhichAPI service passes to the FRED API (which-api PR #13, 2026-11-14).
export const FRED_UNITS = {
  lin: 'levels (no transformation)', chg: 'change', ch1: 'change from a year ago', pch: 'percent change',
  pc1: 'percent change from a year ago', pca: 'compounded annual rate of change', cch: 'continuously compounded rate of change',
  cca: 'continuously compounded annual rate of change', log: 'natural log',
};
export const FRED_NOTICE = 'This product uses the FRED® API but is not endorsed or certified by the Federal Reserve Bank of St. Louis.';

/** The WhichAPI service to read FRED through: `WHICHAPI_URL` (default the hosted service) and `WHICHAPI_API_KEY`. */
export function whichapiConfig(env = process.env) {
  const raw = (env.WHICHAPI_URL || WHICHAPI_DEFAULT_URL).replace(/\/+$/, '');
  let origin;
  try { origin = new URL(raw).origin; } catch { throw new SourceError('requires_configuration', `WHICHAPI_URL is not a valid URL: ${raw}`); }
  return { base: raw, origin, key: env.WHICHAPI_API_KEY || '' };
}

/** @param {{series_id:string,start_date?:string,end_date?:string,limit?:number,units?:string}} p @param {{maxAgeSeconds?:number,now?:Date,env?:Record<string,string|undefined>}} [o] */
export async function fredSeries(p, o = {}) {
  const id = String(p?.series_id || '').toUpperCase();
  if (!/^[A-Z0-9_]{1,40}$/.test(id)) throw invalid('series_id must be a FRED series id such as DGS10 or CPIAUCSL.');
  if (ICE_SERIES.test(id)) throw invalid(`${id} is an ICE BofA series. ICE Data Indices owns it and requires written permission, so this package does not serve it.`);
  const start = p.start_date ?? new Date((o.now ?? new Date()).getTime() - 365 * 86400000).toISOString().slice(0, 10);
  if (!DATE.test(start) || (p.end_date !== undefined && !DATE.test(p.end_date))) throw invalid('Dates must be YYYY-MM-DD.');
  const limit = int(p.limit, 'limit', 1, 5000, 100);
  const units = p.units ?? 'lin';
  if (!Object.hasOwn(FRED_UNITS, units)) throw invalid(`units must be one of ${Object.keys(FRED_UNITS).join(', ')}.`);
  const svc = whichapiConfig(o.env);
  if (!svc.key) throw new SourceError('requires_configuration', `FRED series are read through the WhichAPI hosted service. Set WHICHAPI_API_KEY to a WhichAPI key (service: ${svc.base}).`);
  const url = new URL(`${svc.base}/v1/tools/get_fred_series`);
  url.searchParams.set('series_id', id); url.searchParams.set('start_date', start); url.searchParams.set('limit', String(limit));
  if (p.end_date) url.searchParams.set('end_date', p.end_date);
  // Only sent when asked for, so default calls also work against a WhichAPI service older than PR #13.
  if (units !== 'lin') url.searchParams.set('units', units);
  if (o.maxAgeSeconds !== undefined) url.searchParams.set('max_age_seconds', String(o.maxAgeSeconds));
  // The free hosting tier sleeps when idle and takes about a minute to wake up.
  const { data, cached } = await getJson(url.href, { ttlSeconds: 3600, timeoutMs: 90000, ...o, allowOrigin: svc.origin, relayErrors: true, headers: { Authorization: `Bearer ${svc.key}` } });
  if (data?.series_id !== id || !Array.isArray(data.records)) throw new SourceError('mapping_failed', 'Unexpected reply from the WhichAPI service for this FRED series.');
  // A service that ignored units would hand back levels labelled as a transformation; refuse instead.
  if (units !== 'lin' && data.units !== units) throw new SourceError('mapping_failed', `The WhichAPI service did not apply units=${units}; it may be older than this package.`);
  const records = data.records.filter((r) => r && DATE.test(r.observation_date)).slice(0, limit).map((r) => ({ observation_date: r.observation_date, value: num(r.value) }));
  return {
    source: 'Federal Reserve Bank of St. Louis (FRED)', url: `https://fred.stlouisfed.org/series/${id}`, via: svc.base, series_id: id, order: 'newest first',
    ...(units !== 'lin' ? { units, transformation: FRED_UNITS[units] } : {}),
    note: `Values are in the series units${units !== 'lin' ? `, transformed by FRED (${FRED_UNITS[units]})` : ''}; null means FRED marks the day as missing. Read through the WhichAPI service. ${FRED_NOTICE}`,
    cached: cached || Boolean(data.cached), records,
  };
}

// ---------- US Treasury daily par yield curve ----------
const CURVE_HEADER = ['Date', '1 Mo', '1.5 Month', '2 Mo', '3 Mo', '4 Mo', '6 Mo', '1 Yr', '2 Yr', '3 Yr', '5 Yr', '7 Yr', '10 Yr', '20 Yr', '30 Yr'];
export const CURVE_TENORS = ['1M', '1.5M', '2M', '3M', '4M', '6M', '1Y', '2Y', '3Y', '5Y', '7Y', '10Y', '20Y', '30Y'];
/** @param {{year?:number,limit?:number}} p @param {{maxAgeSeconds?:number,now?:Date}} [o] */
export async function treasuryCurve(p = {}, o = {}) {
  const year = int(p.year, 'year', 2025, 2100, (o.now ?? new Date()).getUTCFullYear());
  const limit = int(p.limit, 'limit', 1, 260, 30);
  const url = `https://home.treasury.gov/resource-center/data-chart-center/interest-rates/daily-treasury-rates.csv/${year}/all?type=daily_treasury_yield_curve&field_tdr_date_value=${year}&page=&_format=csv`;
  const { rows, cached } = await getDelimited(url, ',', { ttlSeconds: 3600, ...o });
  if (!rows[0] || CURVE_HEADER.some((h, i) => rows[0][i] !== h)) throw new SourceError('mapping_failed', 'Unexpected Treasury yield curve header.');
  const records = rows.slice(1).filter((r) => r.length === CURVE_HEADER.length).map((r) => {
    const m = /^(\d{2})\/(\d{2})\/(\d{4})$/.exec(r[0]);
    return { date: m ? `${m[3]}-${m[1]}-${m[2]}` : null, yields: Object.fromEntries(CURVE_TENORS.map((t, i) => [t, num(r[i + 1])])) };
  }).filter((r) => r.date).sort((a, b) => b.date.localeCompare(a.date)).slice(0, limit);
  return { source: 'U.S. Department of the Treasury', url: 'https://home.treasury.gov/policy-issues/financing-the-government/interest-rate-statistics', unit: 'percent (par yield)', order: 'newest first', note: 'Only 2025 and later are accepted because the 1.5-month tenor starts in 2025.', cached, records };
}

// ---------- TIC SLT table 1 ----------
const TIC_HEADER = ['country', 'country_code', 'date', 'for_lt_total_pos', 'for_lt_total_net', 'for_lt_total_valchg', 'for_lt_treas_pos', 'for_lt_treas_net', 'for_lt_treas_valchg', 'for_lt_agcy_pos', 'for_lt_agcy_net', 'for_lt_agcy_valchg', 'for_lt_corp_pos', 'for_lt_corp_net', 'for_lt_corp_valchg', 'for_lt_eqty_pos', 'for_lt_eqty_net', 'for_lt_eqty_valchg'];
/** @param {{country?:string,limit?:number}} p @param {{maxAgeSeconds?:number}} [o] */
export async function ticHoldings(p = {}, o = {}) {
  const country = String(p.country || 'China, Mainland').trim();
  if (!/^[A-Za-z0-9 ,.&'()/-]{2,80}$/.test(country)) throw invalid('country must be a TIC country name such as "China, Mainland" or "Japan".');
  const limit = int(p.limit, 'limit', 1, 240, 24);
  const { rows, cached } = await getDelimited('https://ticdata.treasury.gov/resource-center/data-chart-center/tic/Documents/slt_table1.txt', '\t', { ttlSeconds: 21600, ...o });
  const at = rows.findIndex((r) => TIC_HEADER.every((h, i) => r[i] === h));
  if (at < 0) throw new SourceError('mapping_failed', 'Unexpected TIC table layout.');
  const body = rows.slice(at + 1).filter((r) => r.length === TIC_HEADER.length && /^\d{4}-\d{2}$/.test(r[2]));
  const want = country.toLowerCase();
  const matches = body.filter((r) => r[0].trim().toLowerCase() === want);
  if (!matches.length) {
    const names = [...new Set(body.map((r) => r[0].trim()))];
    const near = names.filter((n) => n.toLowerCase().includes(want.split(',')[0])).slice(0, 8);
    throw new SourceError('not_found', `No TIC rows for "${country}".${near.length ? ` Did you mean: ${near.join('; ')}?` : ' Use the exact TIC spelling, e.g. "China, Mainland", "Japan", "United Kingdom".'}`);
  }
  const records = matches.sort((a, b) => b[2].localeCompare(a[2])).slice(0, limit).map((r) => ({
    country: r[0].trim(), country_code: r[1], month: r[2],
    ...Object.fromEntries(TIC_HEADER.slice(3).map((h, i) => [h.replace('for_lt_', ''), num(r[i + 3])])),
  }));
  return { source: 'U.S. Treasury International Capital (TIC), SLT table 1', url: 'https://home.treasury.gov/data/treasury-international-capital-tic-system', unit: 'millions of USD', order: 'newest month first', note: 'treas_pos = holdings of Treasury bonds and notes at month end; *_net = net foreign purchases; country is the holder location reported by custodians.', cached, records };
}

// ---------- Debt limit headroom from the Daily Treasury Statement ----------
/** @param {{}} _p @param {{maxAgeSeconds?:number}} [o] */
export async function debtLimit(_p = {}, o = {}) {
  const { data, cached } = await getJson('https://api.fiscaldata.treasury.gov/services/api/fiscal_service/v1/accounting/dts/debt_subject_to_limit?page%5Bsize%5D=30&sort=-record_date', { ttlSeconds: 3600, ...o });
  if (!Array.isArray(data?.data) || !data.data.length) throw new SourceError('mapping_failed', 'Unexpected Daily Treasury Statement response.');
  const day = data.data[0].record_date;
  const rows = data.data.filter((r) => r.record_date === day);
  const sum = (cat) => rows.filter((r) => r.debt_catg === cat).reduce((s, r) => {
    const v = num(r.close_today_bal);
    if (v === null) throw new SourceError('mapping_failed', 'Non-numeric DTS balance.');
    return s + v;
  }, 0);
  const held = sum('Debt Held by the Public'), intra = sum('Intragovernmental Holdings'), notSubject = sum('Debt Not Subject to Limit'), other = sum('Other Debt Subject to Limit'), limit = sum('Statutory Debt Limit');
  if (!limit) throw new SourceError('mapping_failed', 'The statement has no statutory debt limit row.');
  const subject = held + intra - notSubject + other;
  return {
    source: 'US Treasury Fiscal Data, Daily Treasury Statement', url: 'https://fiscaldata.treasury.gov/datasets/daily-treasury-statement/', unit: 'millions of USD', cached,
    note: 'Derived: subject debt = held by the public + intragovernmental − not subject to limit + other subject to limit. This is not an official X-date estimate and ignores extraordinary measures.',
    records: [{ record_date: day, statutory_debt_limit: limit, debt_subject_to_limit: subject, headroom: limit - subject, debt_held_by_public: held, intragovernmental_holdings: intra, debt_not_subject_to_limit: notSubject, other_debt_subject_to_limit: other }],
  };
}

// ---------- Federal Register ----------
/** @param {{term:string,order?:'relevance'|'newest',limit?:number,agency?:string,type?:string,published_since?:string}} p @param {{maxAgeSeconds?:number}} [o] */
export async function federalRegister(p, o = {}) {
  const term = String(p?.term || '').trim();
  if (!/^[\p{L}\p{N} ,.&'"()/:§-]{2,200}$/u.test(term)) throw invalid('term must be 2 to 200 characters; quote a phrase such as "Section 232" to match it exactly.');
  const order = p.order || 'relevance';
  if (!['relevance', 'newest', 'oldest'].includes(order)) throw invalid('order must be relevance, newest or oldest.');
  const limit = int(p.limit, 'limit', 1, 100, 10);
  const url = new URL('https://www.federalregister.gov/api/v1/documents.json');
  url.searchParams.set('conditions[term]', term); url.searchParams.set('order', order); url.searchParams.set('per_page', String(limit));
  if (p.agency) { if (!/^[a-z0-9-]{2,100}$/.test(p.agency)) throw invalid('agency must be a Federal Register agency slug such as industry-and-security-bureau.'); url.searchParams.set('conditions[agencies][]', p.agency); }
  if (p.type) { const t = { rule: 'RULE', proposed_rule: 'PRORULE', notice: 'NOTICE', presidential: 'PRESDOCU' }[p.type]; if (!t) throw invalid('type must be rule, proposed_rule, notice or presidential.'); url.searchParams.set('conditions[type][]', t); }
  if (p.published_since) { if (!DATE.test(p.published_since)) throw invalid('published_since must be YYYY-MM-DD.'); url.searchParams.set('conditions[publication_date][gte]', p.published_since); }
  const { data, cached } = await getJson(url.href, { ttlSeconds: 1800, ...o });
  if (!Array.isArray(data?.results)) throw new SourceError('mapping_failed', 'Unexpected Federal Register response.');
  const records = data.results.map((d) => ({ title: d.title ?? null, document_number: d.document_number ?? null, type: d.type ?? null, publication_date: d.publication_date ?? null, agencies: Array.isArray(d.agencies) ? d.agencies.map((a) => a.name || a.raw_name).filter(Boolean) : [], abstract: d.abstract ?? null, html_url: d.html_url ?? null, pdf_url: d.pdf_url ?? null }));
  return { source: 'Office of the Federal Register', url: 'https://www.federalregister.gov/', total: typeof data.count === 'number' ? data.count : null, order, note: 'Keyword search; individual export licences are usually not published.', cached, records };
}

// ---------- SEC XBRL company concept ----------
/** @param {{cik:string,concept:string,unit?:string,limit?:number,forms?:string[]}} p @param {{maxAgeSeconds?:number,secUserAgent?:string}} [o] */
export async function companyConcept(p, o = {}) {
  const ua = o.secUserAgent ?? process.env.SEC_USER_AGENT;
  if (!ua || !/@/.test(ua)) throw new SourceError('requires_configuration', 'SEC requires a User-Agent with contact details. Set SEC_USER_AGENT, e.g. "YourCompany research you@example.com".');
  const cik = String(p?.cik || '').replace(/^CIK/i, '');
  if (!/^\d{1,10}$/.test(cik)) throw invalid('cik must be 1 to 10 digits, e.g. 0001652044 for Alphabet.');
  const concept = String(p.concept || '');
  if (!/^[A-Za-z0-9_]{2,100}$/.test(concept)) throw invalid('concept must be a US-GAAP tag such as PaymentsToAcquirePropertyPlantAndEquipment.');
  const unit = String(p.unit || 'USD');
  const limit = int(p.limit, 'limit', 1, 200, 12);
  const forms = Array.isArray(p.forms) && p.forms.length ? p.forms.map(String) : null;
  const { data, cached } = await getJson(`https://data.sec.gov/api/xbrl/companyconcept/CIK${cik.padStart(10, '0')}/us-gaap/${concept}.json`, { ttlSeconds: 3600, headers: { 'User-Agent': ua }, ...o });
  const facts = data?.units?.[unit];
  if (!Array.isArray(facts)) throw new SourceError('not_found', `No ${unit} facts for ${concept}. Available units: ${Object.keys(data?.units || {}).join(', ') || 'none'}.`);
  // The same fact is repeated in later filings; keep the first filing of each (period, value) pair, newest period first.
  const seen = new Set();
  const records = facts.filter((f) => !forms || forms.includes(f.form)).sort((a, b) => String(b.end).localeCompare(String(a.end)) || String(a.filed).localeCompare(String(b.filed)))
    .filter((f) => { const k = `${f.start ?? ''}|${f.end}|${f.val}`; if (seen.has(k)) return false; seen.add(k); return true; })
    .slice(0, limit).map((f) => ({ period_start: f.start ?? null, period_end: f.end ?? null, value: num(f.val), fiscal_year: f.fy ?? null, fiscal_period: f.fp ?? null, form: f.form ?? null, filed: f.filed ?? null, accession: f.accn ?? null }));
  return { source: 'SEC EDGAR XBRL', url: `https://www.sec.gov/cgi-bin/browse-edgar?action=getcompany&CIK=${cik}`, entity_name: data.entityName ?? null, concept, label: data.label ?? null, unit, order: 'newest period first', note: 'Duration facts in 10-Q filings are often fiscal-year-to-date; check period_start.', cached, records };
}

