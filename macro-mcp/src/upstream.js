// Read-only HTTPS requests with a timeout, a size cap, an origin allowlist and a small in-memory cache.
// Requests are GETs, except a source whose query endpoint only takes POST with a JSON body (the NBS database):
// that POST only reads, and its body is part of the cache key.
// FRED is not on the list: FRED series are read through the WhichAPI hosted service (see fredSeries in sources.js).

export const USER_AGENT = 'WhichAPI-Macro/0.1 (read-only public data)';
const MAX_BYTES = 16 * 1024 * 1024;
const ALLOWED_ORIGINS = new Set([
  'https://markets.newyorkfed.org',
  'https://home.treasury.gov',
  'https://ticdata.treasury.gov',
  'https://api.fiscaldata.treasury.gov',
  'https://www.federalregister.gov',
  'https://data.sec.gov',
  'https://data.stats.gov.cn',
]);

export class SourceError extends Error {
  /** @param {string} code @param {string} message @param {boolean} [retryable] */
  constructor(code, message, retryable = false) {
    super(message);
    this.code = code;
    this.retryable = retryable;
  }
}

/** @type {Map<string,{at:number,value:{text:string,contentType:string}}>} */
const cache = new Map();
export function clearCache() { cache.clear(); }

/**
 * Fetch text from an allowlisted origin.
 * `allowOrigin` adds one more https origin for this call (the configured WhichAPI service). With `relayErrors`,
 * a failed response whose JSON body is `{"error":{"code","message"}}` is rethrown with that code and message.
 * @param {string} href
 * With `jsonBody`, the request is a POST carrying that value as JSON.
 * @param {{accept?:string,ttlSeconds?:number,headers?:Record<string,string>,timeoutMs?:number,maxAgeSeconds?:number,allowOrigin?:string,relayErrors?:boolean,jsonBody?:unknown}} [options]
 * @returns {Promise<{text:string,contentType:string,cached:boolean}>}
 */
export async function getText(href, options = {}) {
  const url = new URL(href);
  const allowed = ALLOWED_ORIGINS.has(url.origin) || (options.allowOrigin !== undefined && options.allowOrigin === url.origin);
  if (url.protocol !== 'https:' || !allowed || url.username || url.password) {
    throw new SourceError('internal_error', `Origin not allowed: ${url.origin}`);
  }
  const ttl = Math.max(0, options.ttlSeconds ?? 300);
  const maxAge = options.maxAgeSeconds ?? ttl;
  const body = options.jsonBody === undefined ? undefined : JSON.stringify(options.jsonBody);
  const key = body === undefined ? url.href : `POST ${url.href}\n${body}`;
  const hit = cache.get(key);
  if (hit && Date.now() - hit.at < Math.min(ttl, maxAge) * 1000) return { ...hit.value, cached: true };

  let response;
  try {
    response = await fetch(url, {
      ...(body === undefined ? {} : { method: 'POST', body }),
      headers: { Accept: options.accept || 'application/json', 'User-Agent': USER_AGENT, ...(body === undefined ? {} : { 'Content-Type': 'application/json' }), ...options.headers },
      signal: AbortSignal.timeout(options.timeoutMs ?? 30000),
      redirect: 'follow',
    });
  } catch (error) {
    const name = /** @type {any} */ (error)?.name;
    if (name === 'TimeoutError' || name === 'AbortError') throw new SourceError('upstream_timeout', `${url.host} did not answer in time.`, true);
    throw new SourceError('upstream_error', `Could not reach ${url.host}.`, true);
  }
  if (!response.ok && options.relayErrors) await relayError(url, response);
  if (response.status === 404) throw new SourceError('not_found', `${url.host} returned 404 for this request.`);
  if (response.status === 429) throw new SourceError('rate_limited', `${url.host} is rate limiting requests. Try again later.`, true);
  if (!response.ok) throw new SourceError('upstream_error', `${url.host} returned HTTP ${response.status}.`, response.status >= 500);

  const reader = response.body?.getReader();
  if (!reader) throw new SourceError('upstream_error', `${url.host} returned an empty body.`, true);
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > MAX_BYTES) { await reader.cancel(); throw new SourceError('upstream_error', `${url.host} response exceeds 16 MiB.`); }
    chunks.push(value);
  }
  const value = { text: Buffer.concat(chunks).toString('utf8'), contentType: response.headers.get('content-type') || '' };
  if (ttl > 0) {
    cache.set(key, { at: Date.now(), value });
    if (cache.size > 500) cache.delete(cache.keys().next().value);
  }
  return { ...value, cached: false };
}

/**
 * Rethrow an error reply from a service that answers `{"error":{"code","message","retryable"}}` (the WhichAPI REST API).
 * 401/403 mean the key was missing or rejected. Returns normally when the body has no such error, so the caller
 * falls back to the generic status handling.
 * @param {URL} url @param {Response} response
 */
async function relayError(url, response) {
  if (response.status === 401 || response.status === 403) {
    throw new SourceError('requires_configuration', `${url.host} rejected the API key (HTTP ${response.status}). Check WHICHAPI_API_KEY.`);
  }
  let body;
  try { body = JSON.parse((await response.text()).slice(0, 65536)); } catch { return; }
  const e = body?.error;
  if (e && typeof e.code === 'string' && typeof e.message === 'string') {
    throw new SourceError(e.code, `${url.host}: ${e.message}`, Boolean(e.retryable));
  }
}

/** @param {string} href @param {Parameters<typeof getText>[1]} [options] */
export async function getJson(href, options = {}) {
  const res = await getText(href, { accept: 'application/json', ...options });
  if (!/json/i.test(res.contentType)) throw new SourceError('mapping_failed', `${new URL(href).host} did not return JSON.`);
  try { return { data: JSON.parse(res.text), cached: res.cached }; }
  catch { throw new SourceError('mapping_failed', `${new URL(href).host} returned invalid JSON.`); }
}

/**
 * RFC 4180 parser for comma or tab separated text. Handles quotes, doubled quotes, CRLF and a BOM; drops blank lines.
 * @param {string} text @param {','|'\t'} delimiter
 * @returns {string[][]}
 */
export function parseDelimited(text, delimiter) {
  const rows = [];
  let row = [], field = '', quoted = false;
  const input = text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
  for (let i = 0; i < input.length; i++) {
    const ch = input[i];
    if (quoted) {
      if (ch === '"') { if (input[i + 1] === '"') { field += '"'; i++; } else quoted = false; }
      else field += ch;
    } else if (ch === '"' && field === '') quoted = true;
    else if (ch === delimiter) { row.push(field); field = ''; }
    else if (ch === '\n' || ch === '\r') {
      if (ch === '\r' && input[i + 1] === '\n') i++;
      row.push(field); field = '';
      if (row.length > 1 || row[0] !== '') rows.push(row);
      row = [];
    } else field += ch;
  }
  row.push(field);
  if (row.length > 1 || row[0] !== '') rows.push(row);
  return rows;
}

/** @param {string} href @param {','|'\t'} delimiter @param {Parameters<typeof getText>[1]} [options] */
export async function getDelimited(href, delimiter, options = {}) {
  const res = await getText(href, { accept: 'text/csv, text/plain, */*', ...options });
  if (/text\/html/i.test(res.contentType)) throw new SourceError('mapping_failed', `${new URL(href).host} returned an HTML page instead of data.`);
  return { rows: parseDelimited(res.text, delimiter), cached: res.cached };
}
