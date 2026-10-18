// Development/release gate only. The packaged executable does not require Node.
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const assert = require('node:assert/strict');

async function main() {
  assert(process.argv[2], 'Usage: node scripts/smoke-mcp.cjs <executable>');
  const executable = path.resolve(process.argv[2]);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-release-smoke-'));
  fs.writeFileSync(path.join(root, 'sample.ts'), 'export const answer: number = 42;\n');
  const child = spawn(executable, ['--root', root], {
    stdio: ['pipe', 'pipe', 'inherit'], env: { ...process.env, HARNESS_TOOL_PROFILE: 'full' }
  });
  const pending = new Map();
  let next = 1;
  const failPending = error => { for (const waiter of pending.values()) waiter.reject(error); pending.clear(); };
  child.on('error', failPending);
  child.on('exit', code => failPending(new Error(`Server exited: ${code}`)));
  const lines = readline.createInterface({ input: child.stdout });
  lines.on('line', line => {
    let message;
    try { message = JSON.parse(line); } catch { failPending(new Error('Non-JSON stdout')); return; }
    const waiter = pending.get(message.id);
    if (waiter) { pending.delete(message.id); waiter.resolve(message); }
  });
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
  try {
    const init = await request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'release-smoke', version: '1' } });
    assert(init.result?.capabilities?.tools, JSON.stringify(init));
    send({ method: 'notifications/initialized' });
    const listed = await request('tools/list', {});
    assert.equal(listed.result?.tools.length, 30, JSON.stringify(listed));
    const bootstrap = await call('harness_bootstrap', {});
    assert(bootstrap.session, 'Bootstrap must provide persistent session');
    await call('harness_doctor', { session: bootstrap.session });
    const read = await call('harness_read', { path: 'sample.ts', start: 1, end: 1, session: bootstrap.session });
    assert(JSON.stringify(read).includes('answer'), JSON.stringify(read));
    console.log('PASS: packaged executable MCP initialize, 30 tools, bootstrap, doctor, read');
  } finally {
    child.kill();
    await new Promise(resolve => child.exitCode !== null ? resolve() : child.once('exit', resolve));
    lines.close();
    // Only remove the unique fixture directory created by this script.
    fs.rmSync(root, { recursive: true, force: true });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
