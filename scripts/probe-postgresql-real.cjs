'use strict';
// Read-only real-endpoint diagnostics. Never print the DSN, credentials, rows or raw client errors.
const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const readline = require('node:readline');
const { spawn } = require('node:child_process');
async function main() {
  const [executable, envFile, outputDirectory] = process.argv.slice(2);
  if (!executable || !envFile || !outputDirectory) throw new Error('usage');
  const line = fs.readFileSync(envFile, 'utf8').split(/\r?\n/).find(line => /^\s*(?:export\s+)?HARNESS_SQL_CONNECTION_STRING\s*=/.test(line));
  if (!line) throw new Error('variable_missing');
  let dsn = line.slice(line.indexOf('=') + 1).trim();
  if ((dsn.startsWith('"') && dsn.endsWith('"')) || (dsn.startsWith("'") && dsn.endsWith("'"))) dsn = dsn.slice(1, -1);
  if (dsn.includes('${')) throw new Error('unresolved_variable_expression');
  let uri; try { uri = new URL(dsn); } catch { throw new Error('invalid_uri'); }
  if (!['postgres:', 'postgresql:'].includes(uri.protocol)) throw new Error('not_postgresql');
  const port = Number(uri.port || 5432); const tcp = [];
  for (let i = 0; i < 3; i++) tcp.push(await new Promise((resolve, reject) => {
    const start = performance.now(); const socket = net.connect({ host: uri.hostname, port });
    socket.setTimeout(5000); socket.once('timeout', () => { socket.destroy(); reject(new Error('tcp_timeout')); });
    socket.once('error', () => reject(new Error('tcp_connect_failed')));
    socket.once('connect', () => { socket.destroy(); resolve(performance.now() - start); });
  }));
  const output = path.resolve(outputDirectory); fs.mkdirSync(output, { recursive: true });
  const fixture = fs.mkdtempSync(path.join(output, 'readonly-probe-')); fs.mkdirSync(path.join(fixture, '.agents'));
  fs.writeFileSync(path.join(fixture, '.agents', 'harness.config.json'), JSON.stringify({ postgresql: {
    enabled: true, timeoutSeconds: 10, connection: { source: 'environment', name: 'HARNESS_SQL_CONNECTION_STRING' }, tls: { mode: 'verify-full' },
  }}));
  const child = spawn(path.resolve(executable), ['--root', fixture], { env: { ...process.env, HARNESS_SQL_CONNECTION_STRING: dsn, HARNESS_INDEX_WATCH: '0' }, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });
  child.stderr.resume(); const lines = readline.createInterface({ input: child.stdout }); let seq = 0; const pending = new Map();
  const fail = () => { for (const reject of pending.values()) reject({ error: true }); pending.clear(); };
  child.on('error', fail); child.on('exit', fail);
  lines.on('line', line => { try { const value = JSON.parse(line); const resolve = pending.get(value.id); if (resolve) { pending.delete(value.id); resolve(value); } } catch { fail(); } });
  const rpc = (method, params) => new Promise((resolve, reject) => {
    const id = ++seq; const timer = setTimeout(() => { pending.delete(id); reject(new Error('mcp_timeout')); }, 20000);
    pending.set(id, value => { clearTimeout(timer); value.error ? reject(new Error('mcp_error')) : resolve(value.result); });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  const call = async arguments_ => {
    const start = performance.now(); const response = await rpc('tools/call', { name: 'harness_sql', arguments: arguments_ });
    const envelope = response.structuredContent || JSON.parse(response.content.find(c => c.type === 'text').text);
    return { elapsedMs: performance.now() - start, envelope };
  };
  const report = { tcpConnectMs: tcp, tlsPolicy: 'verify-full-no-downgrade', harness: [] };
  try {
    await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'readonly-pg-diagnostic', version: '1' } });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
    const response = await rpc('tools/call', { name: 'harness_bootstrap', arguments: { includeRules: false } });
    const boot = response.structuredContent || JSON.parse(response.content.find(c => c.type === 'text').text);
    const session = boot.data.session;
    for (let i = 0; i < 6; i++) {
      const { elapsedMs, envelope } = await call({ action: 'status', engine: 'postgresql', connect: true, session });
      report.harness.push({ elapsedMs, state: envelope.state, codes: (envelope.diagnostics || []).map(d => d.code), connectionReused: envelope.data?.connectionReused });
      if (envelope.state !== 'ok') break;
    }
    if (report.harness.every(sample => sample.state === 'ok')) {
      // libpq receives the secret only in its private environment, never in command arguments.
      uri.searchParams.set('sslmode', 'verify-full');
      if (!uri.searchParams.has('sslrootcert')) uri.searchParams.set('sslrootcert', 'system');
      report.psql = await new Promise(resolve => {
        const start = performance.now(); let stdout = ''; let stderr = ''; let readyMs;
        const process_ = spawn('C:/Program Files/PostgreSQL/18/bin/psql.exe', ['-X', '-w', '-q', '-A', '-t', '-v', 'ON_ERROR_STOP=1'], {
          env: { ...process.env, PGDATABASE: uri.toString(), PGCONNECT_TIMEOUT: '10', PGOPTIONS: '-c default_transaction_read_only=on -c statement_timeout=5000 -c lock_timeout=1000 -c search_path=pg_catalog', PGAPPNAME: 'harness-readonly-latency-probe' }, stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true,
        });
        const timer = setTimeout(() => process_.kill(), 20000);
        process_.stdout.on('data', chunk => { stdout += chunk; if (readyMs === undefined && stdout.includes('PROBE_READY')) readyMs = performance.now() - start; });
        process_.stderr.on('data', chunk => { if (stderr.length < 10000) stderr += chunk; });
        process_.once('error', () => { clearTimeout(timer); resolve({ ok: false, cause: 'client_unavailable' }); });
        process_.once('exit', code => { clearTimeout(timer); resolve({ ok: code === 0, startupIncludingProcessMs: readyMs, queryMs: [...stdout.matchAll(/(?:Time|Tiempo):\s*([\d.,]+)\s*ms/gi)].map(m => Number(m[1].replace(',', '.'))), cause: code === 0 ? undefined : /certificate|certificado/i.test(stderr) ? 'certificate_validation' : /authentication|password|autenticaci/i.test(stderr) ? 'authentication' : 'client_connection_or_query_error' }); });
        process_.stdin.end('\\echo PROBE_READY\n\\timing on\nSELECT 1;\nSELECT 1;\nSELECT 1;\nSELECT 1;\nSELECT 1;\n\\q\n');
      });
    }
  } finally {
    const exited = new Promise(resolve => child.once('exit', resolve)); child.stdin.end(); const timer = setTimeout(() => child.kill(), 3000);
    await exited; clearTimeout(timer); lines.close();
  }
  fs.writeFileSync(path.join(output, 'real-diagnostic.json'), JSON.stringify(report, null, 2)); console.log(JSON.stringify(report, null, 2));
}
main().catch(error => { console.error(['usage','variable_missing','unresolved_variable_expression','invalid_uri','not_postgresql','tcp_timeout','tcp_connect_failed','mcp_timeout','mcp_error'].includes(error.message) ? error.message : 'diagnostic_failed_details_withheld'); process.exitCode = 1; });
