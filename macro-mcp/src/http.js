// HTTP transport: MCP Streamable HTTP at /mcp (JSON responses) plus a small REST API.
import { createServer } from 'node:http';
import { createHash, timingSafeEqual } from 'node:crypto';
import { handleRaw, SERVER_INFO } from './mcp.js';
import { callTool, TOOLS } from './tools.js';
import { GROUPS, PANELS } from './snapshot.js';
import { CONSOLE_CSP, CONSOLE_HTML } from './console.js';

const LOOPBACK = new Set(['127.0.0.1', '::1', 'localhost', '[::1]']);
const MAX_BODY = 64 * 1024;

function send(res, status, body, headers = {}) {
  const text = typeof body === 'string' ? body : JSON.stringify(body, null, 2);
  res.writeHead(status, { 'Content-Type': typeof body === 'string' ? 'text/plain; charset=utf-8' : 'application/json; charset=utf-8', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff', ...headers });
  res.end(text);
}
const fail = (res, status, code, message, headers) => send(res, status, { error: { code, message } }, headers);
/** Hostname of a Host header ("127.0.0.1:3220", "[::1]:3220", "localhost"), lower-cased; '' when absent or malformed. */
const hostOf = (header) => { if (!header) return ''; try { return new URL(`http://${header}`).hostname.toLowerCase(); } catch { return ''; } };
const sha = (s) => createHash('sha256').update(s).digest();

async function readBody(req) {
  const chunks = [];
  let size = 0;
  for await (const chunk of req) {
    size += chunk.length;
    if (size > MAX_BODY) throw Object.assign(new Error('Request body is too large.'), { status: 413 });
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString('utf8');
}

/** Convert query-string values to the types a tool schema declares. */
function queryArgs(tool, params) {
  const args = {};
  for (const [key, raw] of params) args[key] = typed(tool.inputSchema.properties[key]?.type, raw);
  return args;
}

/** Convert one query-string value to a JSON Schema type. */
function typed(type, raw) {
  if (type === 'integer' || type === 'number') return /^-?\d+(\.\d+)?$/.test(raw) ? Number(raw) : raw;
  if (type === 'boolean') return raw === 'true' ? true : raw === 'false' ? false : raw;
  if (type === 'array') return raw.split(',').map((s) => s.trim()).filter(Boolean);
  return raw;
}

const STATUS = { invalid_request: 400, not_found: 404, requires_configuration: 501, rate_limited: 429, upstream_timeout: 504 };
/** HTTP status for a tool result: 200, or the error code's status (502 for upstream failures). */
const statusOf = (result) => (!result.isError ? 200 : STATUS[result.structuredContent?.error?.code] ?? 502);

/**
 * @param {{host?:string,port?:number,apiKey?:string,allowAnonymous?:boolean,trustProxy?:boolean,log?:(m:string)=>void}} [options]
 */
export function createHttpServer(options = {}) {
  const host = options.host ?? '127.0.0.1';
  const apiKey = options.apiKey ?? process.env.MACRO_API_KEY ?? '';
  const log = options.log ?? ((m) => process.stderr.write(m + '\n'));
  const loopbackOnly = LOOPBACK.has(host);
  if (!loopbackOnly && !apiKey && !options.allowAnonymous) throw new Error(`Refusing to listen on ${host} without MACRO_API_KEY. Set a key, bind 127.0.0.1, or pass --allow-anonymous.`);
  const hits = new Map();
  // Behind a hosting proxy (TRUST_PROXY=1) the socket address is the proxy's; use the first X-Forwarded-For entry.
  const trustProxy = options.trustProxy ?? process.env.TRUST_PROXY === '1';
  const clientIp = (req) => (trustProxy && String(req.headers['x-forwarded-for'] || '').split(',')[0].trim()) || req.socket.remoteAddress || '?';
  const limited = (ip) => { const now = Date.now(); if (hits.size > 10000) for (const [k, v] of hits) if (now - v.start > 60000) hits.delete(k); let w = hits.get(ip); if (!w || now - w.start > 60000) { w = { start: now, n: 0 }; hits.set(ip, w); } return ++w.n > 120; };

  const server = createServer(async (req, res) => {
    try {
      let url;
      try { url = new URL(req.url || '/', 'http://localhost'); } catch { return fail(res, 400, 'invalid_request', 'Malformed request target.'); }
      const path = url.pathname.replace(/\/+$/, '') || '/';
      // DNS-rebinding protection: a loopback server only answers requests addressed to a loopback Host.
      // Same-origin browser GETs omit Origin, so the Host check is what stops a rebound hostname.
      if (loopbackOnly && !LOOPBACK.has(hostOf(req.headers.host))) return fail(res, 421, 'misdirected_request', 'This server only answers requests addressed to localhost or 127.0.0.1.');
      if (path === '/health') return send(res, 200, { status: 'ok', server: SERVER_INFO });
      // The browser test console holds no data; its calls go through the key check below.
      if (req.method === 'GET' && path === '/' && /text\/html/i.test(req.headers.accept || '')) {
        res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Content-Security-Policy': CONSOLE_CSP, 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff', 'Referrer-Policy': 'no-referrer' });
        return res.end(CONSOLE_HTML);
      }
      // DNS-rebinding protection: browsers send Origin; only loopback origins may talk to a loopback server.
      const origin = req.headers.origin;
      if (origin && loopbackOnly) { let ok = false; try { ok = LOOPBACK.has(new URL(origin).hostname); } catch {} if (!ok) return fail(res, 403, 'forbidden', 'Cross-origin requests are not allowed.'); }
      if (apiKey) { const got = req.headers.authorization || ''; if (!timingSafeEqual(sha(got), sha(`Bearer ${apiKey}`))) return fail(res, 401, 'unauthorized', 'Provide Authorization: Bearer <MACRO_API_KEY>.', { 'WWW-Authenticate': 'Bearer' }); }
      if (limited(clientIp(req))) return fail(res, 429, 'rate_limited', 'Too many requests. Try again in a minute.', { 'Retry-After': '60' });

      if (path === '/mcp') {
        if (req.method !== 'POST') return fail(res, 405, 'method_not_allowed', 'This server answers MCP over POST with JSON responses; it does not open SSE streams.', { Allow: 'POST' });
        if (!/application\/json/i.test(req.headers['content-type'] || '')) return fail(res, 415, 'unsupported_media_type', 'Send Content-Type: application/json.');
        const reply = await handleRaw(await readBody(req));
        if (!reply) { res.writeHead(202); return res.end(); }
        return send(res, 200, JSON.parse(reply));
      }
      if (req.method === 'GET' && path === '/') return send(res, 200, { server: SERVER_INFO, mcp: 'POST /mcp', rest: { panels: 'GET /v1/panels', snapshot: 'GET /v1/snapshot?panels=sofr,effr&max_age_seconds=0', tools: 'GET /v1/tools', call: 'GET or POST /v1/tools/{name}' } });
      if (req.method === 'GET' && path === '/v1/panels') return send(res, 200, { groups: GROUPS, panels: PANELS.map(({ id, group, label, unit, source, note }) => ({ id, group, label, unit, source, note })) });
      if (req.method === 'GET' && path === '/v1/tools') return send(res, 200, { tools: TOOLS.map(({ name, title, description, inputSchema }) => ({ name, title, description, inputSchema })) });
      const named = path === '/v1/snapshot' ? 'macro_snapshot' : /^\/v1\/tools\/([a-z_]+)$/.exec(path)?.[1];
      if (named) {
        const tool = TOOLS.find((t) => t.name === named);
        if (!tool) return fail(res, 404, 'not_found', `Unknown tool ${named}. See GET /v1/tools.`);
        let args;
        if (req.method === 'GET') args = queryArgs(tool, url.searchParams);
        else if (req.method === 'POST') { const text = await readBody(req); try { args = text ? JSON.parse(text) : {}; } catch { return fail(res, 400, 'invalid_request', 'Body must be a JSON object.'); } }
        else return fail(res, 405, 'method_not_allowed', 'Use GET or POST.', { Allow: 'GET, POST' });
        const result = await callTool(named, args);
        return send(res, statusOf(result), result.structuredContent);
      }
      return fail(res, 404, 'not_found', 'Not found. See GET /.');
    } catch (error) {
      log(`request error: ${error?.stack || error}`);
      if (!res.headersSent) return fail(res, error?.status || 500, error?.status === 413 ? 'payload_too_large' : 'internal_error', error?.status === 413 ? error.message : 'Internal error.');
      res.destroy();
    }
  });
  // Unparseable HTTP from a client must never take the process down.
  server.on('clientError', (_err, socket) => { if (socket.writable) socket.end('HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n'); else socket.destroy(); });
  return server;
}

export function runHttp({ host = '127.0.0.1', port = 3220, apiKey, allowAnonymous, trustProxy, log = (m) => process.stderr.write(m + '\n') } = {}) {
  const server = createHttpServer({ host, apiKey, allowAnonymous, trustProxy, log });
  server.listen(port, host, () => log(`${SERVER_INFO.name} ${SERVER_INFO.version} listening on http://${host.includes(':') ? `[${host}]` : host}:${server.address().port} (MCP at /mcp)`));
  return server;
}
