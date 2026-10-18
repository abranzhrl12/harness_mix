// Development/release gate only. The packaged executable does not require Node.
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const assert = require('node:assert/strict');

async function main() {
  assert(process.argv[2], 'Usage: node scripts/smoke-mcp.cjs <executable>');
  const executable = path.resolve(process.argv[2]);
  const watchIndex = process.argv.includes('--watch-index');
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-release-smoke-'));
  fs.writeFileSync(path.join(root, 'sample.ts'), 'export const answer: number = 42;\n');
  fs.writeFileSync(path.join(root, 'slow.test.cjs'), "const {test} = require('node:test'); test('slow', async () => { await new Promise(r => setTimeout(r, 30000)); });\n");
  if (watchIndex) {
    const initialized = spawnSync(executable, ['--root', root, '--init-index'], { encoding: 'utf8' });
    assert.equal(initialized.status, 0, initialized.stderr);
  }
  let child, lines;
  const pending = new Map();
  let next = 1;
  const start = () => {
    const serverEnv = { ...process.env, HARNESS_TOOL_PROFILE: 'full' };
    delete serverEnv.HARNESS_INDEX_WATCH;
    if (!watchIndex) serverEnv.HARNESS_INDEX_WATCH = '0';
    child = spawn(executable, ['--root', root], {
      stdio: ['pipe', 'pipe', 'inherit'], env: serverEnv
    });
    const failPending = error => { for (const waiter of pending.values()) waiter.reject(error); pending.clear(); };
    child.on('error', failPending);
    child.on('exit', code => failPending(new Error(`Server exited: ${code}`)));
    lines = readline.createInterface({ input: child.stdout });
    lines.on('line', line => {
      let message;
      try { message = JSON.parse(line); } catch { failPending(new Error('Non-JSON stdout')); return; }
      const waiter = pending.get(message.id);
      if (waiter) { pending.delete(message.id); waiter.resolve(message); }
    });
  };
  const stop = async () => {
    if (!child || child.exitCode !== null) return;
    const exited = new Promise(resolve => child.once('exit', resolve));
    child.stdin.end();
    const timer = setTimeout(() => child.kill(), 10000);
    const code = await exited;
    clearTimeout(timer);
    lines.close();
    assert.equal(code, 0, 'Server must close cleanly after stdin EOF');
  };
  const send = message => child.stdin.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n');
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = next++;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Timeout: ${method}`)); }, 30000);
    pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
    send({ id, method, params });
  });
  const call = async (name, args) => {
    const response = await request('tools/call', { name, arguments: args });
    assert(!response.error, JSON.stringify(response));
    assert(!response.result.isError, JSON.stringify(response));
    const envelope = response.result.structuredContent;
    assert.equal(envelope?.state, 'ok', JSON.stringify(response));
    return envelope.data;
  };
  const initialize = async () => {
    const init = await request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'release-smoke', version: '1' } });
    assert(init.result?.capabilities?.tools, JSON.stringify(init));
    send({ method: 'notifications/initialized' });
  };
  const waitFor = async (taskId, statuses) => {
    const deadline = Date.now() + 30000;
    do {
      const job = await call('harness_jobs', { action: 'status', taskId });
      if (statuses.includes(job.status)) return job;
      assert(!['failed', 'cancelled', 'timed_out', 'interrupted'].includes(job.status), JSON.stringify(job));
      await new Promise(resolve => setTimeout(resolve, 25));
    } while (Date.now() < deadline);
    throw new Error(`Job timeout: ${taskId}`);
  };
  try {
    start();
    await initialize();
    const listed = await request('tools/list', {});
    assert.equal(listed.result?.tools.length, 30, JSON.stringify(listed));
    const bootstrap = await call('harness_bootstrap', {});
    assert(bootstrap.session, 'Bootstrap must provide persistent session');
    await call('harness_doctor', { session: bootstrap.session });
    const read = await call('harness_read', { path: 'sample.ts', start: 1, end: 1, session: bootstrap.session });
    assert(JSON.stringify(read).includes('answer'), JSON.stringify(read));
    const search = await call('harness_search', { kind: 'text', query: 'answer', session: bootstrap.session });
    assert(JSON.stringify(search).includes('sample.ts'), JSON.stringify(search));
    if (watchIndex) {
      const waitForIndex = async predicate => {
        const deadline = Date.now() + 30000;
        let lastStatus;
        do {
          const report = await call('harness_doctor', { versions: false });
          const status = report.checks.find(check => check.id === 'index-auto-sync').details;
          lastStatus = status;
          assert.equal(status.enabled, true);
          if (predicate(status)) return status;
          await new Promise(resolve => setTimeout(resolve, 50));
        } while (Date.now() < deadline);
        throw new Error('Automatic index maintenance did not converge: ' + JSON.stringify(lastStatus));
      };
      await waitForIndex(status => status.state === 'idle');
      fs.writeFileSync(path.join(root, 'sample.ts'), 'export function automaticallyIndexed(): number { return 42; }\n');
      await waitForIndex(status => status.state === 'idle' && status.batches >= 1);
      const indexed = await call('harness_explore', { query: 'automaticallyIndexed', filePath: 'sample.ts' });
      assert(indexed.definitions?.some(definition => definition.symbol?.name === 'automaticallyIndexed'), JSON.stringify(indexed));
      console.log('PASS: automatic index update visible over MCP without syncIndex or --sync');
    }
    const started = await call('harness_jobs', { action: 'start', verify: { language: 'typescript', action: 'build', scope: 'sample.ts' } });
    const completed = await waitFor(started.taskId, ['succeeded']);
    assert(completed.result, JSON.stringify(completed));
    const slow = await call('harness_jobs', { action: 'start', verify: { language: 'javascript', action: 'test', scope: 'slow.test.cjs' } });
    await waitFor(slow.taskId, ['running']);
    await call('harness_jobs', { action: 'cancel', taskId: slow.taskId });
    await waitFor(slow.taskId, ['cancelled']);
    const history = await call('harness_jobs', { action: 'events', after: 0 });
    assert(history.events.length > 0, JSON.stringify(history));
    await stop();
    start();
    await initialize();
    assert.deepEqual(await call('harness_jobs', { action: 'status', taskId: started.taskId }), completed);
    assert.equal((await call('harness_jobs', { action: 'status', taskId: slow.taskId })).status, 'cancelled');
    assert.deepEqual(await call('harness_jobs', { action: 'events', after: 0 }), history);
    console.log('PASS: packaged executable MCP initialize, 30 tools, bootstrap, doctor, read, search, job result, cancellation, history after restart');
  } finally {
    await stop();
    // Only remove the unique fixture directory created by this script.
    fs.rmSync(root, { recursive: true, force: true });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
