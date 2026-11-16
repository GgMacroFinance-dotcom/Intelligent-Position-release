// MCP stdio transport: newline-delimited JSON-RPC on stdin/stdout. Logs go to stderr only.
import { createInterface } from 'node:readline';
import { handleRaw, SERVER_INFO } from './mcp.js';

export function runStdio({ input = process.stdin, output = process.stdout, log = (m) => process.stderr.write(m + '\n') } = {}) {
  const rl = createInterface({ input, crlfDelay: Infinity });
  let pending = 0, closed = false;
  const finish = () => { if (closed && pending === 0 && output !== process.stdout) output.end?.(); };
  log(`${SERVER_INFO.name} ${SERVER_INFO.version} ready on stdio`);
  rl.on('line', (line) => {
    if (!line.trim()) return;
    pending++;
    handleRaw(line)
      .then((reply) => { if (reply) output.write(reply + '\n'); })
      .catch((e) => log(`handler error: ${e?.stack || e}`))
      .finally(() => { pending--; finish(); });
  });
  rl.on('close', () => { closed = true; finish(); });
  return rl;
}
