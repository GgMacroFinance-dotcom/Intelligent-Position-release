import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import * as src from '../src/sources.js';
import { clearCache, parseDelimited, SourceError } from '../src/upstream.js';
import { mockFetch, defaultRoutes, ticTxt } from './fixtures.js';

beforeEach(() => clearCache());
const rejectsWith = (promise, code) => assert.rejects(promise, (e) => e instanceof SourceError && e.code === code);

test('delimited parser handles quotes, doubled quotes, CRLF, BOM and empty trailing fields', () => {
  assert.deepEqual(parseDelimited('﻿a,"b,c","d""e"\r\n1,2,3\n\n', ','), [['a', 'b,c', 'd"e'], ['1', '2', '3']]);
  assert.deepEqual(parseDelimited('x\ty\n1\t\n', '\t'), [['x', 'y'], ['1', '']]);
});

test('New York Fed rates map percentiles and the EFFR target range', async () => {
  const m = mockFetch();
  try {
    const r = await src.referenceRates({ rate: 'effr', observations: 2 });
    assert.equal(new URL(m.calls[0].href).pathname, '/api/rates/unsecured/effr/last/2.json');
    assert.deepEqual([r.records[0].effective_date, r.records[0].rate_percent, r.records[0].target_rate_from, r.records[0].target_rate_to], ['2026-11-04', 3.88, 3.75, 4]);
    await rejectsWith(src.referenceRates({ rate: 'LIBOR' }), 'invalid_request');
    await rejectsWith(src.referenceRates({ observations: 0 }), 'invalid_request');
  } finally { m.restore(); }
});

const keyed = { env: { WHICHAPI_API_KEY: 'k-test' } };
const replyJson = (body, status) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } });

test('FRED goes through the WhichAPI service with the key, newest first, nulls kept', async () => {
  const m = mockFetch();
  try {
    const r = await src.fredSeries({ series_id: 'dgs10', start_date: '2026-10-01', limit: 3 }, keyed);
    const u = new URL(m.calls[0].href);
    assert.equal(u.origin + u.pathname, 'https://whichapi-macro.onrender.com/v1/tools/get_fred_series');
    assert.deepEqual([u.searchParams.get('series_id'), u.searchParams.get('start_date'), u.searchParams.get('limit')], ['DGS10', '2026-10-01', '3']);
    assert.equal(m.calls[0].init.headers.Authorization, 'Bearer k-test');
    assert.deepEqual(r.records, [{ observation_date: '2026-11-04', value: 4.15 }, { observation_date: '2026-11-03', value: null }, { observation_date: '2026-10-31', value: 4.1 }]);
    assert.equal(r.via, 'https://whichapi-macro.onrender.com');
    assert.equal(r.url, 'https://fred.stlouisfed.org/series/DGS10');
    assert.equal((await src.fredSeries({ series_id: 'DGS10', limit: 1, start_date: '2026-10-02' }, keyed)).records.length, 1);
    await src.fredSeries({ series_id: 'DGS10', start_date: '2026-10-03' }, { env: { WHICHAPI_API_KEY: 'k', WHICHAPI_URL: 'https://whichapi-macro.onrender.com/' }, maxAgeSeconds: 0 });
    assert.equal(new URL(m.calls.at(-1).href).searchParams.get('max_age_seconds'), '0');
  } finally { m.restore(); }
});

test('ICE BofA series ids are forwarded like any other FRED series', async () => {
  const m = mockFetch();
  try {
    const r = await src.fredSeries({ series_id: 'bamlh0a0hym2', start_date: '2026-10-07' }, keyed);
    assert.equal(new URL(m.calls[0].href).searchParams.get('series_id'), 'BAMLH0A0HYM2');
    assert.equal(r.series_id, 'BAMLH0A0HYM2');
  } finally { m.restore(); }
});

test('FRED units are forwarded only when asked for, and a service that ignores them is refused', async () => {
  const m = mockFetch();
  try {
    const plain = await src.fredSeries({ series_id: 'DGS10', start_date: '2026-10-05' }, keyed);
    assert.equal(new URL(m.calls[0].href).searchParams.has('units'), false);
    assert.equal(plain.units, undefined);
    assert.match(plain.note, /uses the FRED® API but is not endorsed or certified/);
    const yoy = await src.fredSeries({ series_id: 'CPIAUCSL', units: 'pc1', start_date: '2026-10-06' }, keyed);
    assert.equal(new URL(m.calls[1].href).searchParams.get('units'), 'pc1');
    assert.deepEqual([yoy.units, yoy.transformation], ['pc1', 'percent change from a year ago']);
    await rejectsWith(src.fredSeries({ series_id: 'CPIAUCSL', units: 'yoy' }, keyed), 'invalid_request');
  } finally { m.restore(); }
  clearCache();
  // An older WhichAPI service drops units and answers levels without a units field.
  const old = mockFetch(() => replyJson({ series_id: 'CPIAUCSL', records: [{ observation_date: '2026-09-01', value: 324.1 }] }, 200));
  try { await rejectsWith(src.fredSeries({ series_id: 'CPIAUCSL', units: 'pc1' }, keyed), 'mapping_failed'); } finally { old.restore(); }
});

test('FRED refuses without a key and bad input, and never calls FRED directly', async () => {
  const m = mockFetch();
  try {
    await rejectsWith(src.fredSeries({ series_id: 'DGS10' }, { env: {} }), 'requires_configuration');
    await rejectsWith(src.fredSeries({ series_id: 'DGS10', start_date: '11/01/2026' }, keyed), 'invalid_request');
    await rejectsWith(src.fredSeries({ series_id: 'bad id' }, keyed), 'invalid_request');
    await rejectsWith(src.fredSeries({ series_id: 'DGS10' }, { env: { WHICHAPI_API_KEY: 'k', WHICHAPI_URL: 'not a url' } }), 'requires_configuration');
    assert.equal(m.calls.length, 0);
  } finally { m.restore(); }
  const { getText } = await import('../src/upstream.js');
  await rejectsWith(getText('https://fred.stlouisfed.org/graph/fredgraph.csv?id=DGS10'), 'internal_error');
  // A different WHICHAPI_URL is allowed for that call only, and only over https.
  await rejectsWith(src.fredSeries({ series_id: 'DGS10' }, { env: { WHICHAPI_API_KEY: 'k', WHICHAPI_URL: 'http://whichapi.example' } }), 'internal_error');
  const other = mockFetch(() => replyJson({ series_id: 'DGS10', records: [] }, 200));
  try {
    await src.fredSeries({ series_id: 'DGS10', start_date: '2026-10-04' }, { env: { WHICHAPI_API_KEY: 'k', WHICHAPI_URL: 'https://whichapi.example/macro' } });
    assert.equal(new URL(other.calls[0].href).origin + new URL(other.calls[0].href).pathname, 'https://whichapi.example/macro/v1/tools/get_fred_series');
  } finally { other.restore(); }
});

test('FRED relays WhichAPI errors: rejected key, error bodies, unexpected replies', async () => {
  const cases = [
    [() => new Response('nope', { status: 401 }), 'requires_configuration'],
    [() => replyJson({ error: { code: 'invalid_request', message: 'series_id is unknown' } }, 400), 'invalid_request'],
    [() => replyJson({ error: { code: 'mapping_failed', message: 'Unexpected FRED header', retryable: false } }, 502), 'mapping_failed'],
    [() => new Response('<html>bad gateway</html>', { status: 502, headers: { 'content-type': 'text/html' } }), 'upstream_error'],
    [() => replyJson({ series_id: 'OTHER', records: [] }, 200), 'mapping_failed'],
  ];
  for (const [route, code] of cases) {
    clearCache();
    const m = mockFetch(route);
    try { await rejectsWith(src.fredSeries({ series_id: 'DGS10' }, keyed), code); } finally { m.restore(); }
  }
});

test('Treasury curve converts dates, keeps every tenor and sorts newest first', async () => {
  const m = mockFetch();
  try {
    const r = await src.treasuryCurve({ year: 2026 });
    assert.match(m.calls[0].href, /daily-treasury-rates\.csv\/2026\/all\?type=daily_treasury_yield_curve/);
    assert.equal(r.records[0].date, '2026-11-05');
    assert.equal(r.records[0].yields['10Y'], 4.84);
    assert.equal(r.records[0].yields['1.5M'], 3.94);
    assert.equal(Object.keys(r.records[0].yields).length, 14);
    await rejectsWith(src.treasuryCurve({ year: 2024 }), 'invalid_request');
  } finally { m.restore(); }
});

test('TIC skips the preamble, matches the country exactly and suggests names on a miss', async () => {
  const m = mockFetch();
  try {
    const r = await src.ticHoldings({ country: 'china, mainland' });
    assert.deepEqual(r.records.map((x) => [x.month, x.treas_pos, x.treas_net]), [['2026-08', 555800, -3200], ['2026-07', 560100, -4100]]);
    assert.equal(r.records[0].eqty_pos, 300000);
    await assert.rejects(src.ticHoldings({ country: 'China' }), (e) => e.code === 'not_found' && e.message.includes('China, Mainland'));
  } finally { m.restore(); }
  clearCache();
  const moved = mockFetch(() => new Response(ticTxt([]).replace('for_lt_treas_pos', 'treas_pos'), { headers: { 'content-type': 'text/plain' } }));
  try { await rejectsWith(src.ticHoldings({}), 'mapping_failed'); } finally { moved.restore(); }
});

test('debt limit uses only the latest statement day', async () => {
  const m = mockFetch();
  try {
    const x = (await src.debtLimit()).records[0];
    assert.deepEqual([x.record_date, x.statutory_debt_limit, x.debt_subject_to_limit, x.headroom], ['2026-11-04', 41103996, 40353891, 750105]);
  } finally { m.restore(); }
});

test('Federal Register builds filters and maps documents', async () => {
  const m = mockFetch();
  try {
    const r = await src.federalRegister({ term: '"Section 232" semiconductors', order: 'newest', limit: 2, agency: 'industry-and-security-bureau', type: 'notice', published_since: '2026-01-01' });
    const u = new URL(m.calls[0].href);
    assert.equal(u.searchParams.get('conditions[term]'), '"Section 232" semiconductors');
    assert.equal(u.searchParams.get('conditions[agencies][]'), 'industry-and-security-bureau');
    assert.equal(u.searchParams.get('conditions[type][]'), 'NOTICE');
    assert.equal(u.searchParams.get('conditions[publication_date][gte]'), '2026-01-01');
    assert.equal(r.total, 44);
    assert.equal(r.records[0].html_url, 'https://www.federalregister.gov/d/2026-21001');
    await rejectsWith(src.federalRegister({ term: 'x' }), 'invalid_request');
    await rejectsWith(src.federalRegister({ term: 'tariffs', type: 'memo' }), 'invalid_request');
  } finally { m.restore(); }
});

test('SEC concept requires a contact User-Agent, sends it, and drops repeated facts', async () => {
  const m = mockFetch();
  try {
    await rejectsWith(src.companyConcept({ cik: '1652044', concept: 'PaymentsToAcquirePropertyPlantAndEquipment' }, { secUserAgent: '' }), 'requires_configuration');
    const r = await src.companyConcept({ cik: 'CIK1652044', concept: 'PaymentsToAcquirePropertyPlantAndEquipment' }, { secUserAgent: 'Test research test@example.com' });
    assert.equal(new URL(m.calls[0].href).pathname, '/api/xbrl/companyconcept/CIK0001652044/us-gaap/PaymentsToAcquirePropertyPlantAndEquipment.json');
    assert.equal(m.calls[0].init.headers['User-Agent'], 'Test research test@example.com');
    assert.deepEqual(r.records.map((x) => [x.period_end, x.value, x.form]), [['2026-09-30', 131953000000, '10-Q'], ['2026-06-30', 85000000000, '10-Q'], ['2025-12-31', 91000000000, '10-K']]);
    const annual = await src.companyConcept({ cik: '1652044', concept: 'PaymentsToAcquirePropertyPlantAndEquipment', forms: ['10-K'] }, { secUserAgent: 'a b@c.d' });
    assert.deepEqual(annual.records.map((x) => x.form), ['10-K']);
    await rejectsWith(src.companyConcept({ cik: '1652044', concept: 'PaymentsToAcquirePropertyPlantAndEquipment', unit: 'EUR' }, { secUserAgent: 'a b@c.d' }), 'not_found');
  } finally { m.restore(); }
});

test('upstream errors, caching and the origin allowlist', async () => {
  const m = mockFetch();
  try {
    await src.referenceRates({ rate: 'SOFR', observations: 3 });
    const again = await src.referenceRates({ rate: 'SOFR', observations: 3 });
    assert.equal(m.calls.length, 1); assert.equal(again.cached, true);
    await src.referenceRates({ rate: 'SOFR', observations: 3 }, { maxAgeSeconds: 0 });
    assert.equal(m.calls.length, 2);
  } finally { m.restore(); }
  clearCache();
  const limited = mockFetch(() => new Response('slow down', { status: 429 }));
  try { await rejectsWith(src.debtLimit(), 'rate_limited'); } finally { limited.restore(); }
  clearCache();
  const down = mockFetch(() => { throw new TypeError('fetch failed'); });
  try { await rejectsWith(src.debtLimit(), 'upstream_error'); } finally { down.restore(); }
  const { getText } = await import('../src/upstream.js');
  await rejectsWith(getText('https://example.com/x'), 'internal_error');
  assert.equal(typeof defaultRoutes, 'function');
});
