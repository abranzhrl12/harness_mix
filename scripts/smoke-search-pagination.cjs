// Release gate: exercise the real MCP wire and immutable search pages.
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const assert = require('node:assert/strict');

async function main() {
  assert(process.argv[2], 'Usage: node scripts/smoke-search-pagination.cjs <executable>');
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-search-pages-'));
  const file = path.join(root, 'sample.txt');
  fs.writeFileSync(file, Array.from({ length: 650 }, (_, n) => `needle ${n}\n`).join(''));
  const child = spawn(path.resolve(process.argv[2]), ['--root', root], {
    stdio: ['pipe', 'pipe', 'inherit'],
    env: { ...process.env, HARNESS_INDEX_WATCH: '0', HARNESS_TOOL_PROFILE: 'full' },
  });
  const pending = new Map();
  let nextId = 1;
  const lines = readline.createInterface({ input: child.stdout });
  const rejectPending = error => {
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
  };
  child.on('error', rejectPending);
  child.on('exit', code => rejectPending(new Error(`Server exited: ${code}`)));
  lines.on('line', line => {
    const message = JSON.parse(line);
    const waiter = pending.get(message.id);
    if (waiter) { pending.delete(message.id); waiter.resolve(message); }
  });
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = nextId++;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Timeout: ${method}`)); }, 30000);
    pending.set(id, {
      resolve: value => { clearTimeout(timer); resolve(value); },
      reject: error => { clearTimeout(timer); reject(error); },
    });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  const call = async (name, args) => {
    const response = await request('tools/call', { name, arguments: args });
    assert(!response.error, JSON.stringify(response));
    const envelope = response.result.structuredContent;
    assert.equal(envelope.state, 'ok', JSON.stringify(response));
    return envelope;
  };
  try {
    await request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'search-pagination-smoke', version: '1' } });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
    const bootstrap = await call('harness_bootstrap', {});
    const session = bootstrap.data.session;
    let page = await call('harness_search', { query: 'needle', limit: 40, session });
    assert.equal(page.data.scanComplete, true);
    assert.equal(page.omitted.matches, 610);
    assert(!(page.diagnostics || []).some(item => item.code === 'SEARCH_INCOMPLETE'));
    const reference = page.detail.ref;
    const captured = [...page.data.matches];
    // Continuation must return the original results even after the source changes.
    fs.writeFileSync(file, 'replacement without the search term\n');
    while (page.data.hasMore) {
      assert.equal(page.data.nextCall.tool, 'harness_search');
      page = await call('harness_search', page.data.nextCall.arguments);
      assert.equal(page.data.scanComplete, true);
      assert.equal(page.detail.ref, reference);
      captured.push(...page.data.matches);
    }
    assert.equal(captured.length, 650);
    assert.equal(new Set(captured.map(row => row.line)).size, 650);
    assert.equal(captured[649].text.trimEnd(), 'needle 649');
    assert.equal(page.data.nextCall, null);
    fs.writeFileSync(file, 'fn cursorPagination() {}\nfn pagination() {}\n');
    const ranked = await call('harness_search', { query: 'cursor pagination', limit: 1, contextLines: 0, session });
    assert.equal(ranked.data.retrieval.ranking, 'bm25');
    assert.equal(ranked.data.matches[0].line, 1);
    assert.equal(ranked.data.matches[0].context.text, 'fn cursorPagination() {}\n');
    assert(ranked.data.matches[0].score > 0);
    const rankedNext = await call('harness_search', ranked.data.nextCall.arguments);
    assert.equal(rankedNext.data.retrieval.ranking, 'bm25');
    assert.equal(rankedNext.data.matches[0].line, 2);
    console.log('PASS: 650 MCP matches paged once, no duplicates, snapshot stable after file change.');
    console.log('PASS: automatic BM25 term recovery, ranking, source context and cursor metadata.');
  } finally {
    if (child.exitCode === null && child.pid) {
      const closed = new Promise(resolve => child.once('exit', resolve));
      child.stdin.end();
      const timer = setTimeout(() => child.kill(), 5000);
      await closed;
      clearTimeout(timer);
    }
    lines.close();
    fs.rmSync(root, { recursive: true, force: true });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
