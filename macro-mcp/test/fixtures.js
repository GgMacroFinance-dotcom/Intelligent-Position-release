// Response layouts follow the public endpoints as seen on 2026-11-06; values and titles are illustrative.
export const nyfed = (type, rows) => ({ refRates: rows.map(([date, rate, extra = {}]) => ({ effectiveDate: date, type, percentRate: rate, percentPercentile1: rate - 0.03, percentPercentile25: rate - 0.01, percentPercentile75: rate + 0.01, percentPercentile99: rate + 0.07, volumeInBillions: 2900, revisionIndicator: '', ...extra })) });

/** A WhichAPI REST reply for get_fred_series: the hosted service's FRED result, newest first. */
export const whichapiFred = (id, rows, units) => ({ source: 'Federal Reserve Bank of St. Louis (FRED)', url: `https://fred.stlouisfed.org/series/${id}`, series_id: id, order: 'newest first', ...(units ? { units } : {}), cached: false, records: rows.map(([d, v]) => ({ observation_date: d, value: v })) });

const curveHead = 'Date,"1 Mo","1.5 Month","2 Mo","3 Mo","4 Mo","6 Mo","1 Yr","2 Yr","3 Yr","5 Yr","7 Yr","10 Yr","20 Yr","30 Yr"';
export const treasuryCsv = (rows) => curveHead + '\n' + rows.map(([d, ten, two]) => `${d},3.95,3.94,3.93,3.90,3.88,3.80,3.70,${two},3.62,3.80,4.10,${ten},5.05,5.00`).join('\n') + '\n';

const ticHead = ['country', 'country_code', 'date', 'for_lt_total_pos', 'for_lt_total_net', 'for_lt_total_valchg', 'for_lt_treas_pos', 'for_lt_treas_net', 'for_lt_treas_valchg', 'for_lt_agcy_pos', 'for_lt_agcy_net', 'for_lt_agcy_valchg', 'for_lt_corp_pos', 'for_lt_corp_net', 'for_lt_corp_valchg', 'for_lt_eqty_pos', 'for_lt_eqty_net', 'for_lt_eqty_valchg'];
export const ticTxt = (rows) => [
  'Table 1: Foreign Holdings of, and Transactions in, Long-Term US Securities',
  'Millions of US dollars',
  '',
  ticHead.join('\t'),
  'Country\tCode\tDate\tTotal\t\t\t\t\t\t\t\t\t\t\t\t\t\t',
  ...rows.map(([c, code, m, treas, net]) => [c, code, m, 1000000, 100, -50, treas, net, -1200, 50000, 10, 0, 20000, -5, 1, 300000, 200, 3000].join('\t')),
].join('\r\n') + '\r\n';

export const dts = (rows) => ({ data: rows.map(([date, cat, bal]) => ({ record_date: date, debt_catg: cat, close_today_bal: String(bal) })), meta: { count: rows.length } });
export const dtsDay = (date, scale = 1) => [
  [date, 'Debt Held by the Public', 31000000 * scale],
  [date, 'Intragovernmental Holdings', 7400000 * scale],
  [date, 'Debt Not Subject to Limit', 30000 * scale],
  [date, 'Debt Not Subject to Limit', 16109 * scale],
  [date, 'Other Debt Subject to Limit', 2000000 * scale],
  [date, 'Statutory Debt Limit', 41103996],
];

export const fr = (docs, count = docs.length) => ({ count, total_pages: 1, results: docs.map(([title, date, n]) => ({ title, publication_date: date, document_number: n, type: 'Notice', agencies: [{ name: 'Commerce Department' }], abstract: null, html_url: `https://www.federalregister.gov/d/${n}`, pdf_url: null })) });

export const sec = { cik: 1652044, entityName: 'Alphabet Inc.', label: 'Payments to Acquire Property, Plant, and Equipment', units: { USD: [
  { start: '2026-01-01', end: '2026-06-30', val: 85000000000, fy: 2026, fp: 'Q2', form: '10-Q', filed: '2026-07-28', accn: 'a' },
  { start: '2026-01-01', end: '2026-09-30', val: 131953000000, fy: 2026, fp: 'Q3', form: '10-Q', filed: '2026-10-29', accn: 'b' },
  { start: '2025-01-01', end: '2025-12-31', val: 91000000000, fy: 2025, fp: 'FY', form: '10-K', filed: '2026-02-04', accn: 'c' },
  { start: '2025-01-01', end: '2025-12-31', val: 91000000000, fy: 2026, fp: 'Q3', form: '10-Q', filed: '2026-10-29', accn: 'b' },
] } };

const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });
const text = (body, type = 'text/csv') => new Response(body, { headers: { 'content-type': type } });

/** Default responder covering every snapshot panel. */
export function defaultRoutes(url) {
  const u = new URL(url);
  if (u.host === 'markets.newyorkfed.org') return u.pathname.includes('/effr/') ? json(nyfed('EFFR', [['2026-11-04', 3.88, { targetRateFrom: 3.75, targetRateTo: 4 }], ['2026-11-03', 3.87, { targetRateFrom: 3.75, targetRateTo: 4 }]])) : json(nyfed('SOFR', [['2026-11-04', 3.94], ['2026-11-03', 3.96]]));
  if (u.host === 'whichapi-macro.onrender.com' && u.pathname === '/v1/tools/get_fred_series') {
    const id = u.searchParams.get('series_id');
    const v = { DTWEXBGS: [121.5, 121.67], DEXCHUS: [7.12, 7.1032], DGS10: [4.10, 4.15] }[id] || [1, 1];
    const units = u.searchParams.get('units') || undefined;
    return json(whichapiFred(id, [['2026-11-04', v[1]], ['2026-11-03', null], ['2026-10-31', v[0]]], units));
  }
  if (u.host === 'home.treasury.gov') return text(treasuryCsv([['11/04/2026', 4.80, 3.62], ['11/05/2026', 4.84, 3.60]]));
  if (u.host === 'ticdata.treasury.gov') return text(ticTxt([['China, Mainland', '41408', '2026-07', 560100, -4100], ['China, Mainland', '41408', '2026-08', 555800, -3200], ['Japan', '42609', '2026-08', 1150000, 2500], ['Japan', '42609', '2026-07', 1148000, 900]]), 'text/plain');
  if (u.host === 'api.fiscaldata.treasury.gov') return json(dts([...dtsDay('2026-11-04'), ...dtsDay('2026-11-03', 0.99)]));
  if (u.host === 'www.federalregister.gov') return json(fr([['Section 232 inclusions process for semiconductors', '2026-11-03', '2026-21001'], ['Section 232 national security investigation of imports of drones', '2026-10-20', '2026-20111']], 44));
  if (u.host === 'data.sec.gov') return json(sec);
  return new Response('not found', { status: 404 });
}

/** Install a fake fetch; returns the list of requested URLs and a restore function. */
export function mockFetch(routes = defaultRoutes) {
  const original = globalThis.fetch;
  const calls = [];
  globalThis.fetch = async (input, init) => { const href = String(input); calls.push({ href, init }); return routes(href, init); };
  return { calls, restore: () => { globalThis.fetch = original; } };
}
