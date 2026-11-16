// Live acceptance: calls every tool once against the real upstreams (no mocks, cache bypassed).
// Usage: node test/live.js
//   set WHICHAPI_API_KEY to include FRED (get_fred_series and the dollar and USD/CNY snapshot panels),
//   set SEC_USER_AGENT to include get_company_concept.
import { callTool } from '../src/tools.js';

const hasKey = Boolean(process.env.WHICHAPI_API_KEY);
const cases = [
  // Without a WhichAPI key the two FRED panels fail and the snapshot is partial.
  ['macro_snapshot', { max_age_seconds: 0 }, (d) => d.status === (hasKey ? 'ok' : 'partial')],
  ['get_reference_rates', { rate: 'SOFR', observations: 3 }, (d) => d.records.length === 3 && typeof d.records[0].rate_percent === 'number'],
  ['get_treasury_curve', { limit: 2 }, (d) => d.records.length > 0 && typeof d.records[0].yields['10Y'] === 'number'],
  ['get_tic_holdings', { country: 'Japan', limit: 2 }, (d) => d.records.length === 2],
  ['get_debt_limit', {}, (d) => typeof d.records[0].headroom === 'number'],
  ['search_federal_register', { term: '"Section 232"', order: 'newest', limit: 3 }, (d) => d.records.length > 0],
  // NBS may refuse some networks outside mainland China; a failure here can be the network, not the code.
  ['get_china_series', { indicator: 'cpi_yoy', observations: 3 }, (d) => d.records.length > 0 && d.resolved === 'stored_ids'],
  ['search_china_indicators', { query: '社会消费品零售总额', frequency: 'annual', limit: 3 }, (d) => d.results.length > 0],
  ['browse_china_catalog', { catalog_id: '82130c6621a745cda3d64b090e733383' }, (d) => d.kind === 'indicators' && d.indicators.length > 0],
  ['get_china_indicator', { catalog_id: '28d936104e304aa191e338eb82b6dc09', indicator_id: 'd22612f09aeb4241bc557ef0ac61b3ba', frequency: 'quarterly', observations: 4 }, (d) => d.records.length > 0],
  ...(hasKey ? [['get_fred_series', { series_id: 'DGS10', limit: 5 }, (d) => d.records.length > 0]] : []),
  ...(process.env.SEC_USER_AGENT ? [['get_company_concept', { cik: '0001652044', concept: 'PaymentsToAcquirePropertyPlantAndEquipment', limit: 3 }, (d) => d.records.length > 0]] : []),
];

let failed = 0;
for (const [name, args, check] of cases) {
  const started = Date.now();
  const r = await callTool(name, args);
  const ok = !r.isError && check(r.structuredContent);
  if (!ok) failed++;
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name} ${Date.now() - started}ms\n${r.content[0].text.split('\n').slice(0, 14).map((l) => '  ' + l).join('\n')}`);
}
if (!hasKey) console.log('SKIP get_fred_series (set WHICHAPI_API_KEY)');
if (!process.env.SEC_USER_AGENT) console.log('SKIP get_company_concept (set SEC_USER_AGENT)');
console.log(`${cases.length - failed}/${cases.length} passed`);
process.exitCode = failed ? 1 : 0;
