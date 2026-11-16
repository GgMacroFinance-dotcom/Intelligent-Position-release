#!/usr/bin/env node
// whichapi-macro: MCP server (stdio or HTTP) and command line for US macro data.
import { runStdio } from '../src/stdio.js';
import { runHttp } from '../src/http.js';
import { callTool, TOOLS } from '../src/tools.js';
import { SERVER_INFO } from '../src/mcp.js';
import { WHICHAPI_DEFAULT_URL } from '../src/sources.js';

const HELP = `whichapi-macro ${SERVER_INFO.version} — read-only US macro data from official sources

Usage:
  whichapi-macro [mcp]                 MCP server on stdio (for Claude Desktop, Claude Code, Cursor)
  whichapi-macro serve [--port 3220] [--host 127.0.0.1] [--allow-anonymous]
                                       HTTP server: MCP at POST /mcp, REST under /v1
  whichapi-macro snapshot [panel ...] [--json] [--fresh]
                                       Print the macro snapshot
  whichapi-macro call <tool> [json-args] [--json]
                                       Run one tool, e.g. call get_fred_series '{"series_id":"DGS10","limit":5}'
  whichapi-macro tools                 List tools

Environment:
  MACRO_API_KEY     Bearer key required by the HTTP server (mandatory when binding a non-loopback host)
  SEC_USER_AGENT    "Name contact@example.com", required by get_company_concept
  WHICHAPI_API_KEY  WhichAPI key. FRED series (get_fred_series, the dollar and USD/CNY snapshot panels)
                    are read through the WhichAPI hosted service and need it
  WHICHAPI_URL      WhichAPI service to use (default ${WHICHAPI_DEFAULT_URL})
`;

const argv = process.argv.slice(2);
const flag = (name) => { const i = argv.indexOf(name); if (i < 0) return undefined; argv.splice(i, 1); return true; };
const option = (name) => { const i = argv.indexOf(name); if (i < 0) return undefined; const v = argv[i + 1]; argv.splice(i, 2); return v; };

async function printResult(name, args, json) {
  const result = await callTool(name, args);
  if (!result) { process.stderr.write(`Unknown tool: ${name}\n`); process.exit(2); }
  process.stdout.write((json ? JSON.stringify(result.structuredContent, null, 2) : result.content[0].text) + '\n');
  if (result.isError) process.exitCode = 1;
}

const wantsHelp = flag('--help') || flag('-h');
const command = argv.shift() ?? 'mcp';
if (wantsHelp || command === 'help') {
  process.stdout.write(HELP);
} else if (command === 'mcp') {
  runStdio();
} else if (command === 'serve') {
  const port = Number(option('--port') ?? process.env.PORT ?? 3220);
  const host = option('--host') ?? process.env.HOST ?? '127.0.0.1';
  try { runHttp({ host, port, allowAnonymous: flag('--allow-anonymous') || process.env.MACRO_ALLOW_ANONYMOUS === '1' }); }
  catch (e) { process.stderr.write(`${e.message}\n`); process.exit(2); }
} else if (command === 'snapshot') {
  const json = flag('--json'), fresh = flag('--fresh');
  await printResult('macro_snapshot', { ...(argv.length ? { panels: argv } : {}), ...(fresh ? { max_age_seconds: 0 } : {}) }, json);
} else if (command === 'call') {
  const json = flag('--json');
  const [name, raw] = argv;
  let args = {};
  if (raw) { try { args = JSON.parse(raw); } catch { process.stderr.write('Arguments must be a JSON object.\n'); process.exit(2); } }
  await printResult(name, args, json);
} else if (command === 'tools') {
  for (const t of TOOLS) process.stdout.write(`${t.name.padEnd(24)} ${t.title}\n`);
} else {
  process.stderr.write(`Unknown command: ${command}\n\n${HELP}`);
  process.exit(2);
}
