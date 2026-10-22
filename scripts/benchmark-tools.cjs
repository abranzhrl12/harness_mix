// Documentation benchmark: real MCP stdio, temporary fixture, no user project/DB.
// Usage: node scripts/benchmark-tools.cjs EXE [REPORT.json] [SAMPLES]
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const { performance } = require('node:perf_hooks');

async function main() {
  assert(process.argv[2], 'Usage: benchmark-tools.cjs EXE [REPORT.json] [SAMPLES]');
  const executable = path.resolve(process.argv[2]);
  const samples = Number(process.argv[4] || 7);
  assert(Number.isInteger(samples) && samples >= 3 && samples <= 30, 'SAMPLES: 3..30');
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-doc-bench-'));
  const fixture = relative => path.join(root, relative);
  const env = { ...process.env, HARNESS_TOOL_PROFILE: 'full', HARNESS_INDEX_WATCH: '0' };
  // Explicitly disabled DBs: the benchmark never consumes real connection values.
  fs.mkdirSync(fixture('.agents'));
  fs.writeFileSync(fixture('.agents/harness.config.json'), JSON.stringify({ sql: { enabled: false }, postgresql: { enabled: false } }));
  fs.writeFileSync(fixture('sample.ts'), 'export function answer(): number { return 42; }\n');
  fs.writeFileSync(fixture('tsconfig.json'), '{"compilerOptions":{"baseUrl":"."}}');
  fs.writeFileSync(fixture('settings.config'), '<configuration><appSettings><add key="mode" value="demo"/></appSettings></configuration>');
  fs.writeFileSync(fixture('sample.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="8" height="8"/></svg>');
  fs.copyFileSync(path.join(__dirname, '..', 'tests', 'sample.xlsx'), fixture('sample.xlsx'));
  const rows = [];
  function stats(times) {
    const ordered = [...times].sort((a, b) => a - b);
    const round = n => Math.round(n * 1000) / 1000;
    const middle = Math.floor(ordered.length / 2);
    const median = ordered.length % 2 ? ordered[middle] : (ordered[middle - 1] + ordered[middle]) / 2;
    return { samplesMs: times.map(round), medianMs: round(median), minMs: round(ordered[0]), maxMs: round(ordered.at(-1)) };
  }
  function cli(args, accepted = [0]) {
    const result = spawnSync(executable, args, { env, encoding: 'utf8', timeout: 35000 });
    assert(!result.error, result.error?.message);
    assert(accepted.includes(result.status), `${args.join(' ')}: ${result.stderr}`);
    return result.stdout;
  }
  function cliMeasure(label, args, prepare = () => {}) {
    const times = [];
    for (let i = 0; i < samples; i++) {
      const arguments_ = prepare(i) || args;
      const start = performance.now();
      cli(arguments_);
      times.push(performance.now() - start);
    }
    rows.push({ kind: 'cli', operation: label, ...stats(times) });
  }
  let child;
  let lines;
  let exitPromise;
  const pending = new Map();
  let sequence = 0;
  let session;
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`Timeout ${method}`)); }, 30000);
    pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
  });
  async function call(name, args = {}, attachSession = true) {
    const response = await request('tools/call', { name, arguments: attachSession ? { ...args, session } : args });
    assert(!response.error, JSON.stringify(response.error));
    const envelope = response.result?.structuredContent;
    assert.equal(envelope?.state, 'ok', `${name}: ${JSON.stringify(response.result)}`);
    return envelope;
  }
  async function measure(name, label, args, prepare, attachSession = true) {
    const times = [];
    // One unreported warm-up, then independent measured calls.
    for (let i = -1; i < samples; i++) {
      const actual = prepare ? await prepare(i) : args;
      const start = performance.now();
      await call(name, actual, attachSession);
      if (i >= 0) times.push(performance.now() - start);
    }
    rows.push({ kind: 'mcp', tool: name, operation: label, ...stats(times) });
  }
  try {
    assert.equal(spawnSync('git', ['-C', root, 'init', '-q'], { encoding: 'utf8' }).status, 0, 'git required for fixture');
    cliMeasure('--version', ['--version']);
    cliMeasure('--help', ['--help']);
    cliMeasure('--init-config', null, i => {
      const directory = fixture(`config-${i}`);
      fs.mkdirSync(directory);
      return ['--root', directory, '--init-config'];
    });
    cliMeasure('--index init', null, i => {
      const directory = fixture(`index-${i}`);
      fs.mkdirSync(directory);
      return ['--root', directory, '--index', 'init'];
    });
    cli(['--root', root, '--index', 'init']);
    cli(['--root', root, '--index', 'sync']);
    cliMeasure('--index plan', ['--root', root, '--index', 'plan']);
    cliMeasure('--index sync (unchanged)', ['--root', root, '--index', 'sync']);
    cliMeasure('--index rebuild', ['--root', root, '--index', 'rebuild']);
    cliMeasure('--sync-file (unchanged)', ['--root', root, '--sync-file', 'sample.ts']);
    cliMeasure('--sync-file --force', ['--root', root, '--sync-file', 'sample.ts', '--force']);
    cliMeasure('--sync-file --tsconfig', ['--root', root, '--sync-file', 'sample.ts', '--tsconfig', 'tsconfig.json']);
    const start = performance.now();
    child = spawn(executable, ['--root', root], { env, stdio: ['pipe', 'pipe', 'pipe'] });
    exitPromise = new Promise(resolve => child.once('exit', resolve));
    const fail = error => { for (const item of pending.values()) item.reject(error); pending.clear(); };
    child.on('error', fail);
    child.on('exit', code => fail(new Error(`Server exited ${code}`)));
    child.stderr.on('data', () => {});
    lines = readline.createInterface({ input: child.stdout });
    lines.on('line', line => {
      try { const message = JSON.parse(line); const waiter = pending.get(message.id); if (waiter) { pending.delete(message.id); waiter.resolve(message); } }
      catch (error) { fail(error); }
    });
    const initialized = await request('initialize', { protocolVersion: '2025-11-25', capabilities: {}, clientInfo: { name: 'documentation-benchmark', version: '1' } });
    assert(initialized.result?.capabilities?.tools);
    rows.push({ kind: 'startup', operation: 'spawn + initialize (single observation)', ...stats([performance.now() - start]) });
    child.stdin.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
    const listed = await request('tools/list', {});
    const bootstrapStart = performance.now();
    const bootstrap = await call('harness_bootstrap', { compact: true }, false);
    rows.push({ kind: 'startup', operation: 'initial compact bootstrap (single observation)', ...stats([performance.now() - bootstrapStart]) });
    session = bootstrap.data.session;
    assert(session);
    const detailRef = bootstrap.detail?.ref;
    assert(detailRef, 'compact bootstrap should expose its catalog detail');
    await measure('harness_bootstrap', 'reuse existing session', { compact: true });
    await measure('harness_read', 'read one TypeScript line', { path: 'sample.ts', start: 1, end: 1 });
    await measure('harness_search', 'text search, small fixture', { kind: 'text', query: 'answer', scope: '.' });
    await measure('harness_search', 'file search, small fixture', { kind: 'files', query: 'sample.ts' });
    await measure('harness_symbols', 'outline', { action: 'outline', path: 'sample.ts' });
    await measure('harness_symbols', 'read full symbol', { action: 'read', path: 'sample.ts', symbol: 'answer', view: 'full' });
    await measure('harness_explore', 'indexed symbol, existing index', { query: 'answer', filePath: 'sample.ts', symbol: 'answer' });
    await measure('harness_detail', 'read stored bootstrap catalog', { ref: detailRef, maxChars: 2000 });
    await measure('harness_data_query', 'in-memory JMESPath', { data: { rows: [{ name: 'demo' }] }, query: 'rows[].name' });
    await measure('harness_git', 'status, uncommitted temporary repository', { action: 'status' });
    await measure('harness_config', 'read XML appSettings', { path: 'settings.config', section: 'appSettings' });
    await measure('harness_doctor', 'environment, versions disabled, no SQL connections', { action: 'env', versions: false });
    await measure('harness_doctor', 'TypeScript syntax only', { action: 'syntax', files: ['sample.ts'] });
    await measure('harness_ignore', 'list ignore policy', { action: 'list' });
    await measure('harness_external_scope', 'list (no external path granted)', { action: 'list' });
    await measure('harness_session', 'read session status', { action: 'status' });
    await measure('harness_sql', 'PostgreSQL status, disabled, no connection', { action: 'status', engine: 'postgresql', connect: false });
    await measure('harness_xlsx', 'inspect bundled sample workbook', { path: 'sample.xlsx' });
    await measure('harness_svg', 'inspect 16x16 static SVG', { path: 'sample.svg' });
    await measure('harness_verify', 'native TypeScript syntax build, no code execution', { language: 'typescript', action: 'build', scope: 'sample.ts' });
    await measure('harness_jobs', 'list empty background-job registry', { action: 'list' }, null, false);
    await measure('harness_batch', 'two independent reads, maxConcurrent=2', { jobs: [1, 2].map(() => ({ tool: 'harness_read', arguments: { path: 'sample.ts', start: 1, end: 1 } })), maxConcurrent: 2 });
    let counter = 0;
    await measure('harness_write', 'create new TypeScript file', {}, () => ({ path: `created-${counter++}.ts`, content: 'export const created = 1;\n' }));
    await measure('harness_move', 'rename existing TypeScript file', {}, () => {
      const index = counter++;
      fs.writeFileSync(fixture(`move-${index}.ts`), 'export const moved = 1;\n');
      return { from: `move-${index}.ts`, to: `moved-${index}.ts` };
    });
    await measure('harness_replace', 'preview one replacement', {}, async () => {
      const read = await call('harness_read', { path: 'sample.ts', start: 1, end: 1 });
      return { path: 'sample.ts', hash: read.data.hash, oldText: '42', newText: '43', preview: true };
    });
    let value = '42';
    await measure('harness_replace', 'publish one replacement', {}, async () => {
      const read = await call('harness_read', { path: 'sample.ts', start: 1, end: 1 });
      const next = value === '42' ? '43' : '42';
      const args = { path: 'sample.ts', hash: read.data.hash, oldText: value, newText: next };
      value = next;
      return args;
    });
    const measured = new Set(rows.filter(row => row.tool).map(row => row.tool));
    const advertised = listed.result.tools.map(tool => tool.name).sort();
    assert.deepEqual([...measured].sort(), advertised, 'every advertised tool must have a measured representative case');
    const report = {
      recordedAt: new Date().toISOString(), version: cli(['--version']).trim(),
      executableSha256: crypto.createHash('sha256').update(fs.readFileSync(executable)).digest('hex'),
      executableBuildTime: fs.statSync(executable).mtime.toISOString(),
      os: `${os.platform()} ${os.release()} ${os.arch()}`, cpu: os.cpus()[0]?.model,
      logicalProcessors: os.cpus().length, memoryGiB: Math.round(os.totalmem() / 2 ** 30),
      fixture: 'one TS function, XML, SVG and tests/sample.xlsx; CLI-created empty index directories; temporary uncommitted Git repository',
      method: 'wall time; sequential requests; one MCP warm-up per case excluded; CLI includes process startup; setup/read hashes excluded; no OS cache flush; index watcher disabled; no external DB/network/model latency',
      samplesPerCase: samples, advertisedTools: advertised, results: rows,
    };
    if (process.argv[3]) {
      const output = path.resolve(process.argv[3]);
      fs.mkdirSync(path.dirname(output), { recursive: true });
      fs.writeFileSync(output, JSON.stringify(report, null, 2) + '\n');
    }
    console.log(JSON.stringify(report, null, 2));
  } finally {
    if (child && child.exitCode === null) {
      child.stdin.end();
      const timer = setTimeout(() => child.kill(), 10000);
      const code = await exitPromise;
      clearTimeout(timer);
      lines?.close();
      assert.equal(code, 0, 'server must exit cleanly');
    }
    // Only the directory created by mkdtemp above; never a project supplied by a user.
    assert(path.dirname(root) === os.tmpdir() && path.basename(root).startsWith('harness-doc-bench-'));
    fs.rmSync(root, { recursive: true, force: true });
  }
}
main().catch(error => { console.error(error); process.exitCode = 1; });
