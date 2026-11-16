// Macro snapshot: nine headline indicators from the sources in sources.js, fetched in parallel.
// The ICE BofA credit panels of the WhichAPI hosted version are not in this snapshot; read those series with get_fred_series.
// A failing source marks only its own panel as an error; the snapshot then reports "partial".
import * as src from './sources.js';
import { SourceError } from './upstream.js';

export const GROUPS = { funding: 'Funding rates', dollar: 'Dollar and FX', curve: 'Treasury curve', holdings: 'Foreign holdings', fiscal: 'Debt limit', policy: 'Trade and export notices' };

const daysBefore = (now, days) => new Date(now.getTime() - days * 86400000).toISOString().slice(0, 10);
const fredPanel = (id, label, series, group, unit, note) => ({ id, group, label, unit, note, source: 'FRED (via WhichAPI)', run: (now, o) => src.fredSeries({ series_id: series, start_date: daysBefore(now, 120), limit: 60 }, o), points: (r) => r.records.map((x) => [x.observation_date, x.value]) });
const rates = (rate) => (now, o) => src.referenceRates({ rate, observations: 30 }, o);

export const PANELS = [
  { id: 'sofr', group: 'funding', label: 'SOFR', unit: '%', source: 'New York Fed', note: 'Secured Overnight Financing Rate, published about 08:00 ET for the previous business day.', run: rates('SOFR'), points: (r) => r.records.map((x) => [x.effective_date, x.rate_percent]) },
  { id: 'effr', group: 'funding', label: 'Effective fed funds rate', unit: '%', source: 'New York Fed', note: 'Includes the FOMC target range in extras.', run: rates('EFFR'), points: (r) => r.records.map((x) => [x.effective_date, x.rate_percent]),
    extras: (r) => { const x = r.records[0]; return x && x.target_rate_from !== null ? { target_range: [x.target_rate_from, x.target_rate_to] } : {}; } },
  fredPanel('broad_dollar', 'Nominal broad US dollar index', 'DTWEXBGS', 'dollar', 'index', 'Federal Reserve trade-weighted index (Jan 2006 = 100). Not ICE DXY. Needs WHICHAPI_API_KEY.'),
  fredPanel('usd_cny', 'USD/CNY', 'DEXCHUS', 'dollar', 'CNY per USD', 'Federal Reserve H.10 noon buying rate in New York. Needs WHICHAPI_API_KEY.'),
  { id: 'ust_10y', group: 'curve', label: 'US Treasury 10-year par yield', unit: '%', source: 'US Treasury', note: 'The full latest curve and the 2s10s slope are in extras.',
    run: (now, o) => curveWithPriorYear(now, o), points: (r) => r.records.map((x) => [x.date, x.yields['10Y']]),
    extras: (r) => { const y = r.records[0]?.yields; if (!y) return {}; return { curve_date: r.records[0].date, curve: Object.fromEntries(Object.entries(y).filter(([, v]) => v !== null)), ...(y['2Y'] !== null && y['10Y'] !== null ? { slope_2s10s_pct_pts: round(y['10Y'] - y['2Y']) } : {}) }; } },
  { id: 'tic_china', group: 'holdings', label: 'Mainland China holdings of US Treasuries', unit: 'USD mn', source: 'US Treasury TIC', note: 'Month-end Treasury bonds and notes by holder location; custody can sit elsewhere.', run: (now, o) => src.ticHoldings({ country: 'China, Mainland', limit: 24 }, o), points: (r) => r.records.map((x) => [x.month, x.treas_pos]) },
  { id: 'tic_japan', group: 'holdings', label: 'Japan holdings of US Treasuries', unit: 'USD mn', source: 'US Treasury TIC', note: 'Month-end Treasury bonds and notes.', run: (now, o) => src.ticHoldings({ country: 'Japan', limit: 24 }, o), points: (r) => r.records.map((x) => [x.month, x.treas_pos]) },
  { id: 'debt_limit', group: 'fiscal', label: 'Room under the statutory debt limit', unit: 'USD mn', source: 'Daily Treasury Statement', note: 'Arithmetic on published rows, not an official X-date estimate.', run: (now, o) => src.debtLimit({}, o), points: (r) => r.records.map((x) => [x.record_date, x.headroom]),
    extras: (r) => { const x = r.records[0]; return x ? { statutory_debt_limit: x.statutory_debt_limit, debt_subject_to_limit: x.debt_subject_to_limit } : {}; } },
  { id: 'section_232', group: 'policy', label: 'Latest Section 232 notices', unit: 'documents', source: 'Federal Register', note: 'Keyword search, newest first.', run: (now, o) => src.federalRegister({ term: '"Section 232"', order: 'newest', limit: 5 }, o), documents: true },
];
export const PANEL_IDS = PANELS.map((p) => p.id);

/**
 * Early in a year the current-year file has fewer than two days (or none). Add the prior year so the panel
 * still shows the latest curve and a change versus the previous trading day.
 */
async function curveWithPriorYear(now, o) {
  const year = Math.max(2025, now.getUTCFullYear());
  let current;
  try { current = await src.treasuryCurve({ year, limit: 60 }, o); }
  catch (error) { if (year === 2025 || !(error instanceof SourceError) || error.code === 'invalid_request') throw error; current = null; }
  if (current && current.records.length >= 2) return current;
  if (year === 2025) return current;
  const prior = await src.treasuryCurve({ year: year - 1, limit: 60 }, o);
  return { ...(current || prior), records: [...(current?.records || []), ...prior.records].slice(0, 60) };
}

function round(v) { return Math.round(v * 1e6) / 1e6; }

/** Reduce a source result to the panel shape. Exported for tests. */
export function summarize(panel, result) {
  const base = { id: panel.id, group: panel.group, label: panel.label, unit: panel.unit, source: panel.source, source_url: result.url, note: panel.note, cached: Boolean(result.cached) };
  if (panel.documents) {
    const documents = result.records.map((d) => ({ title: d.title, date: d.publication_date, type: d.type, url: d.html_url }));
    return { ...base, status: documents.length ? 'ok' : 'empty', value: null, as_of: documents[0]?.date ?? null, previous: null, change: null, series: [], documents, error: null };
  }
  const byDate = new Map();
  for (const [date, value] of panel.points(result)) if (typeof date === 'string' && typeof value === 'number' && Number.isFinite(value)) byDate.set(date, value);
  const series = [...byDate].map(([date, value]) => ({ date, value })).sort((a, b) => a.date.localeCompare(b.date));
  const last = series.at(-1) ?? null, prev = series.length > 1 ? series.at(-2) : null;
  const extras = panel.extras ? panel.extras(result) : {};
  return { ...base, status: last ? 'ok' : 'empty', value: last?.value ?? null, as_of: last?.date ?? null, previous: prev, change: last && prev ? round(last.value - prev.value) : null, series, ...(Object.keys(extras).length ? { extras } : {}), error: null };
}

/**
 * @param {{panels?:string[],maxAgeSeconds?:number,now?:Date}} [options]
 */
export async function snapshot(options = {}) {
  const now = options.now ?? new Date();
  if (options.panels !== undefined) {
    if (!Array.isArray(options.panels) || !options.panels.length) throw new SourceError('invalid_request', 'panels must be a non-empty array.');
    const bad = options.panels.find((p) => !PANEL_IDS.includes(p));
    if (bad) throw new SourceError('invalid_request', `Unknown panel "${bad}". Valid panels: ${PANEL_IDS.join(', ')}.`);
  }
  const selected = PANELS.filter((p) => !options.panels || options.panels.includes(p.id));
  const o = options.maxAgeSeconds !== undefined ? { maxAgeSeconds: options.maxAgeSeconds } : {};
  const panels = await Promise.all(selected.map(async (panel) => {
    try { return summarize(panel, await panel.run(now, o)); }
    catch (error) {
      const e = error instanceof SourceError ? { code: error.code, message: error.message, retryable: error.retryable } : { code: 'internal_error', message: String(error?.message || error), retryable: false };
      return { id: panel.id, group: panel.group, label: panel.label, unit: panel.unit, source: panel.source, note: panel.note, status: 'error', value: null, as_of: null, previous: null, change: null, series: [], error: e };
    }
  }));
  const failed = panels.filter((p) => p.status === 'error').length;
  const ids = new Set(panels.map((p) => p.id));
  const warnings = [
    ...(ids.has('broad_dollar') ? ['broad_dollar is the Federal Reserve nominal broad dollar index, not ICE DXY.'] : []),
    ...(ids.has('debt_limit') ? ['debt_limit is derived from Daily Treasury Statement rows and is not an official X-date estimate.'] : []),
  ];
  return { status: failed === panels.length ? 'error' : failed ? 'partial' : 'ok', generated_at: now.toISOString(), panels, warnings };
}

/** Plain-text table for terminals and for MCP text content. */
export function formatSnapshot(s) {
  const fmt = (v, unit) => {
    if (v === null || v === undefined) return '—';
    if (unit === 'USD mn') return `${(v / 1000).toLocaleString('en-US', { maximumFractionDigits: 1, minimumFractionDigits: 1 })} bn USD`;
    const digits = unit === 'CNY per USD' ? 4 : 2;
    return `${v.toLocaleString('en-US', { maximumFractionDigits: digits, minimumFractionDigits: digits })}${unit === '%' ? '%' : unit === '% pts' ? ' pts' : ''}`;
  };
  const lines = [`Macro snapshot ${s.generated_at} — ${s.status}`];
  for (const p of s.panels) {
    if (p.status === 'error') { lines.push(`- ${p.label}: error ${p.error.code} — ${p.error.message}`); continue; }
    if (p.documents) { lines.push(`- ${p.label}:`); for (const d of p.documents) lines.push(`    ${d.date}  ${d.title}${d.url ? `  ${d.url}` : ''}`); continue; }
    const change = p.change === null ? '' : ` (${p.change > 0 ? '+' : ''}${p.unit === 'USD mn' ? fmt(p.change, p.unit) : fmt(p.change, p.unit === '%' ? '% pts' : p.unit).replace(/%$/, '')} vs ${p.previous.date})`;
    let extra = '';
    if (p.extras?.target_range) extra = `; target ${p.extras.target_range[0]}–${p.extras.target_range[1]}%`;
    if (p.extras?.slope_2s10s_pct_pts !== undefined) extra = `; 2s10s ${p.extras.slope_2s10s_pct_pts.toFixed(2)} pts`;
    lines.push(`- ${p.label}: ${p.status === 'ok' ? fmt(p.value, p.unit) : 'no data'}${p.as_of ? ` as of ${p.as_of}` : ''}${change}${extra}`);
  }
  for (const w of s.warnings) lines.push(`Note: ${w}`);
  return lines.join('\n');
}
