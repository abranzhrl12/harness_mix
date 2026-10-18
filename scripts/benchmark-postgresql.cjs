// Compare frozen executables against the same dedicated PostgreSQL fixture over MCP.
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const readline = require('node:readline');
const assert = require('node:assert/strict');
const { createHash } = require('node:crypto');
async function main() {
  const [executable, root, output, countText = '20'] = process.argv.slice(2);
  assert(executable && root && output, 'Usage: node benchmark-postgresql.cjs executable fixtureRoot output.json [iterations]');
  const count = Number(countText);
  assert(Number.isInteger(count) && count >= 5 && count <= 100);
  const binarySha256 = createHash('sha256').update(fs.readFileSync(path.resolve(executable))).digest('hex');
  const child = spawn(path.resolve(executable), ['--root', path.resolve(root)], {
    env: { ...process.env, HARNESS_INDEX_WATCH: '0' }, stdio: ['pipe', 'pipe', 'inherit'], windowsHide: true
  });
  const pending = new Map(); let seq = 0;
  const lines = readline.createInterface({ input: child.stdout });
  lines.on('line', line => { const message = JSON.parse(line); pending.get(message.id)?.(message); pending.delete(message.id); });
  const send = value => child.stdin.write(JSON.stringify({ jsonrpc: '2.0', ...value }) + '\n');
  const rpc = (method, params) => new Promise((resolve, reject) => {
    const id = ++seq;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Timeout: ${method}`)); }, 30000);
    pending.set(id, value => { clearTimeout(timer); resolve(value); }); send({ id, method, params });
  });
  const call = async (name, args) => {
    const response = await rpc('tools/call', { name, arguments: args });
    assert(!response.error && !response.result?.isError, JSON.stringify(response));
    const envelope = response.result.structuredContent || JSON.parse(response.result.content.find(c => c.type === 'text').text);
    assert.equal(envelope.state, 'ok', JSON.stringify(envelope));
    return envelope.data;
  };
  try {
    await rpc('initialize', { protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'postgres-benchmark', version: '1' } });
    send({ method: 'notifications/initialized' });
    const boot = await call('harness_bootstrap', { includeRules: false });
    const session = boot.session;
    const measures = {};
    const requests = {
      search: { action: 'search', engine: 'postgresql', query: 'harness_catalog_fixture', objectType: 'table' },
      describe: { action: 'describe', engine: 'postgresql', name: 'public.harness_catalog_fixture' }
    };
    for (const [operation, args] of Object.entries(requests)) {
      const times = []; let reused = 0; let preparedReused = 0;
      for (let i = 0; i < count; i++) {
        const start = performance.now();
        const data = await call('harness_sql', { ...args, session });
        const elapsed = performance.now() - start;
        assert(data.count > 0, 'Fixture must be visible to the test reader');
        times.push(elapsed); reused += Number(data.connectionReused === true); preparedReused += Number(data.statementReused === true);
      }
      const sorted = times.slice(1).sort((a, b) => a - b);
      measures[operation] = { iterations: count, coldMs: times[0], warmMedianMs: sorted[Math.floor(sorted.length / 2)],
        warmP95Ms: sorted[Math.min(sorted.length - 1, Math.ceil(sorted.length * .95) - 1)],
        totalMs: times.reduce((a, b) => a + b, 0), connectionsReused: reused, statementsReused: preparedReused, samplesMs: times };
    }
    const result = { executable: path.resolve(executable), binarySha256, iterations: count, transport: 'MCP stdio', timestamp: new Date().toISOString(), measures };
    fs.writeFileSync(path.resolve(output), JSON.stringify(result, null, 2) + '\n');
    console.log(JSON.stringify({ ...result, measures: Object.fromEntries(Object.entries(measures).map(([k, v]) => [k, { ...v, samplesMs: undefined }])) }, null, 2));
  } finally {
    const exited = new Promise(resolve => child.once('exit', resolve));
    child.stdin.end(); const timer = setTimeout(() => child.kill(), 5000);
    const code = await exited; clearTimeout(timer); lines.close();
    assert.equal(code, 0, 'MCP must exit cleanly with pooled sessions');
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
