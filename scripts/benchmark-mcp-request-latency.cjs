// End-to-end request/response latency against one open local MCP stdio process.
// Usage: node scripts/benchmark-mcp-request-latency.cjs EXE [REPORT.json] [SAMPLES=100] [WARMUPS=20]
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');

const REQUEST_TIMEOUT_MS = 15_000;
const STOP_TIMEOUT_MS = 5_000;

function sha256(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function percentile(sorted, p) {
  return sorted[Math.max(0, Math.ceil(p * sorted.length) - 1)];
}

function summarize(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  const median = sorted.length % 2
    ? sorted[middle]
    : (sorted[middle - 1] + sorted[middle]) / 2;
  const round = value => Math.round(value * 1000) / 1000;
  return {
    n: sorted.length,
    min: round(sorted[0]),
    median: round(median),
    p95NearestRank: round(percentile(sorted, 0.95)),
    p99NearestRank: round(percentile(sorted, 0.99)),
    max: round(sorted.at(-1)),
  };
}

function childEnvironment() {
  const env = {};
  // Keep only native Windows launch variables. Do not inherit database
  // credentials, API keys, MCP overrides, or developer service tokens.
  for (const key of [
    'PATH', 'SystemRoot', 'WINDIR', 'TEMP', 'TMP', 'USERPROFILE',
    'HOMEDRIVE', 'HOMEPATH', 'LOCALAPPDATA', 'APPDATA', 'COMSPEC',
  ]) {
    if (process.env[key] !== undefined) env[key] = process.env[key];
  }
  env.HARNESS_TOOL_PROFILE = 'full';
  env.HARNESS_INDEX_WATCH = '0';
  return env;
}

async function main() {
  assert(process.argv[2], 'Usage: node scripts/benchmark-mcp-request-latency.cjs EXE [REPORT.json] [SAMPLES=100] [WARMUPS=20]');
  const executable = path.resolve(process.argv[2]);
  const samples = Number(process.argv[4] || 100);
  const warmups = Number(process.argv[5] || 20);
  assert(fs.statSync(executable).isFile(), `Not a file: ${executable}`);
  assert(Number.isInteger(samples) && samples >= 100 && samples <= 1000, 'SAMPLES must be 100..1000');
  assert(Number.isInteger(warmups) && warmups >= 10 && warmups <= 200, 'WARMUPS must be 10..200');

  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-mcp-request-latency-'));
  fs.mkdirSync(path.join(root, '.agents'));
  fs.writeFileSync(
    path.join(root, '.agents', 'harness.config.json'),
    JSON.stringify({ sql: { enabled: false }, postgresql: { enabled: false } }),
  );
  fs.writeFileSync(path.join(root, 'sample.ts'), [
    'export function answer(): number {',
    '  return 42;',
    '}',
  ].join('\n') + '\n');
  fs.writeFileSync(path.join(root, 'helper.rs'), 'pub fn helper() -> u32 { 7 }\n');
  fs.writeFileSync(path.join(root, 'notes.txt'), 'needle: fixed local search fixture\n');

  const env = childEnvironment();
  const child = spawn(executable, ['--root', root], {
    env,
    windowsHide: true,
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  let stderrTail = '';
  let lines;
  let exitPromise;
  let nextId = 1;
  const pending = new Map();

  const rejectPending = error => {
    for (const waiter of pending.values()) waiter.reject(error);
    pending.clear();
  };

  const request = requestObject => new Promise((resolve, reject) => {
    const id = nextId++;
    const message = { jsonrpc: '2.0', id, ...requestObject };
    const encoded = `${JSON.stringify(message)}\n`;
    const startedAt = process.hrtime.bigint();
    const timer = setTimeout(() => {
      pending.delete(id);
      reject(new Error(`Timeout ${requestObject.method} after ${REQUEST_TIMEOUT_MS} ms. ${stderrTail}`));
    }, REQUEST_TIMEOUT_MS);
    pending.set(id, {
      resolve: response => {
        clearTimeout(timer);
        resolve({
          response,
          latencyNs: process.hrtime.bigint() - startedAt,
          requestBytes: Buffer.byteLength(encoded),
        });
      },
      reject: error => { clearTimeout(timer); reject(error); },
    });
    child.stdin.write(encoded);
  });

  async function callTool(name, args) {
    const measured = await request({
      method: 'tools/call',
      params: { name, arguments: args },
    });
    assert(!measured.response.error, `${name}: ${JSON.stringify(measured.response.error)}`);
    assert(measured.response.result?.isError !== true, `${name} returned isError`);
    const envelope = measured.response.result?.structuredContent;
    assert.equal(envelope?.state, 'ok', `${name}: ${JSON.stringify(envelope)}`);
    return measured;
  }

  async function measure(name, category, execute) {
    for (let i = 0; i < warmups; i++) await execute();
    const observations = [];
    for (let i = 0; i < samples; i++) {
      const measured = await execute();
      const response = measured.response;
      const result = response.result;
      observations.push({
        latencyMs: Number(measured.latencyNs) / 1e6,
        requestBytes: measured.requestBytes,
        responseBytes: Buffer.byteLength(JSON.stringify(response)),
        resultBytes: Buffer.byteLength(JSON.stringify(result ?? null)),
        contentBytes: result?.content ? Buffer.byteLength(JSON.stringify(result.content)) : 0,
        structuredContentBytes: result?.structuredContent
          ? Buffer.byteLength(JSON.stringify(result.structuredContent))
          : 0,
      });
    }
    const bytes = field => observations.map(sample => sample[field]);
    return {
      operation: name,
      category,
      samples: observations.length,
      latencyMs: summarize(bytes('latencyMs')),
      requestBytes: summarize(bytes('requestBytes')),
      responseBytes: summarize(bytes('responseBytes')),
      resultBytes: summarize(bytes('resultBytes')),
      contentBytes: summarize(bytes('contentBytes')),
      structuredContentBytes: summarize(bytes('structuredContentBytes')),
      observations,
    };
  }

  try {
    exitPromise = new Promise(resolve => child.once('exit', code => resolve(code)));
    child.on('error', rejectPending);
    child.on('exit', code => rejectPending(new Error(`MCP process exited early (${code}): ${stderrTail}`)));
    child.stderr.on('data', chunk => {
      stderrTail = (stderrTail + chunk.toString('utf8')).slice(-8_000);
    });
    lines = readline.createInterface({ input: child.stdout });
    lines.on('line', line => {
      let response;
      try {
        response = JSON.parse(line);
      } catch (error) {
        rejectPending(new Error(`Non-JSON stdout: ${line.slice(0, 500)} (${error})`));
        return;
      }
      if (response.id !== undefined) {
        const waiter = pending.get(response.id);
        if (waiter) {
          pending.delete(response.id);
          waiter.resolve(response);
        }
      }
    });

    const initialize = await request({
      method: 'initialize',
      params: {
        protocolVersion: '2025-11-25',
        capabilities: {},
        clientInfo: { name: 'mcp-request-latency-benchmark', version: '1' },
      },
    });
    assert(initialize.response.result?.capabilities, `initialize failed: ${JSON.stringify(initialize.response)}`);
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' })}\n`);
    const initialList = await request({ method: 'tools/list', params: {} });
    const advertisedTools = initialList.response.result?.tools;
    assert(Array.isArray(advertisedTools), `tools/list failed: ${JSON.stringify(initialList.response)}`);

    const initialBootstrap = await callTool('harness_bootstrap', { compact: true });
    const session = initialBootstrap.response.result.structuredContent?.data?.session;
    assert(session, 'bootstrap did not return a session');

    const ping = await request({ method: 'ping', params: {} });
    const pingSupported = !ping.response.error;
    const results = [];
    if (pingSupported) {
      results.push(await measure('ping (JSON-RPC protocol operation)', 'protocol-plus-server',
        () => request({ method: 'ping', params: {} })));
    }
    results.push(await measure('tools/list (warm catalogue)', 'catalogue',
      () => request({ method: 'tools/list', params: {} })));
    results.push(await measure('harness_read (3-line TypeScript fixture)', 'tool-end-to-end',
      () => callTool('harness_read', { path: 'sample.ts', start: 1, end: 3, session })));
    results.push(await measure('harness_symbols read (known TypeScript function)', 'tool-end-to-end',
      () => callTool('harness_symbols', { action: 'read', path: 'sample.ts', symbol: 'answer', session })));
    results.push(await measure('harness_search (one fixed text match)', 'tool-end-to-end',
      () => callTool('harness_search', {
        kind: 'text', query: 'needle', scope: '.', contextLines: 0, session,
      })));
    results.push(await measure('harness_batch (3 independent reads, maxConcurrent=3)', 'tool-end-to-end',
      () => callTool('harness_batch', {
        jobs: [
          { tool: 'harness_read', arguments: { path: 'sample.ts', start: 1, end: 3 } },
          { tool: 'harness_read', arguments: { path: 'helper.rs', start: 1, end: 1 } },
          { tool: 'harness_read', arguments: { path: 'notes.txt', start: 1, end: 1 } },
        ],
        maxConcurrent: 3,
        session,
      })));

    const git = spawnSync('git', ['-C', path.resolve(__dirname, '..'), 'rev-parse', '--short', 'HEAD'], {
      encoding: 'utf8', windowsHide: true,
    });
    const status = spawnSync('git', ['-C', path.resolve(__dirname, '..'), 'status', '--porcelain'], {
      encoding: 'utf8', windowsHide: true,
    });
    const projectRoot = path.resolve(__dirname, '..');
    const codeFiles = [
      'Cargo.toml', 'Cargo.lock', 'src/main.rs', 'src/cli.rs',
      'src/transport/runtime/single.rs', 'crates/mcp/src/server.rs',
      'crates/mcp/src/presentation.rs', 'scripts/benchmark-mcp-request-latency.cjs',
    ];
    const report = {
      recordedAt: new Date().toISOString(),
      executable,
      executableSha256: sha256(executable),
      executableModifiedAt: fs.statSync(executable).mtime.toISOString(),
      version: spawnSync(executable, ['--version'], { encoding: 'utf8', env, windowsHide: true }).stdout.trim(),
      profile: 'release; cargo build --locked --release --bin agent-harness',
      sourceRevision: git.status === 0 ? git.stdout.trim() : null,
      workingTree: status.status === 0 ? {
        dirty: status.stdout.trim().length > 0,
        changedPathCount: status.stdout.trim() ? status.stdout.trim().split(/\r?\n/).length : 0,
        statusSha256: crypto.createHash('sha256').update(status.stdout).digest('hex'),
      } : null,
      sourceSha256: Object.fromEntries(codeFiles.map(file => [file, sha256(path.join(projectRoot, file))])),
      runtime: {
        node: process.version,
        rustc: spawnSync('rustc', ['--version'], { encoding: 'utf8', windowsHide: true }).stdout.trim(),
        os: `${os.platform()} ${os.release()} ${os.arch()}`,
        cpu: os.cpus()[0]?.model,
        logicalProcessors: os.cpus().length,
        memoryGiB: Math.round(os.totalmem() / 2 ** 30),
      },
      protocol: 'MCP stdio, newline-delimited JSON-RPC 2.0; protocol 2025-11-25; one server process stays open for all samples; bootstrap session is reused.',
      method: 'Each latency starts immediately before JSON-RPC serialization/write and stops when the complete response line has been read and parsed. It is client-observed end-to-end request/response time, not server-only CPU time. Operations run sequentially; per-operation warmups are excluded; no cache flush or concurrency load; response size includes result/content/structuredContent JSON bytes.',
      environment: 'One fresh temporary workspace and server process; full tool profile; HARNESS_INDEX_WATCH=0; SQL Server and PostgreSQL disabled in local fixture config; no index, user data, external DB, network, or model calls; tool output uses current default structured-content renderer.',
      requestsPerOperation: samples,
      warmupsPerOperation: warmups,
      sequential: true,
      concurrency: 'No concurrent client load except the three independent jobs inside each measured harness_batch call (maxConcurrent=3).',
      initializeMs: Math.round(Number(initialize.latencyNs) / 1e6 * 1000) / 1000,
      initialToolsListCount: advertisedTools.length,
      initialBootstrapContentBytes: Buffer.byteLength(JSON.stringify(initialBootstrap.response.result.content ?? null)),
      initialBootstrapStructuredContentBytes: Buffer.byteLength(JSON.stringify(initialBootstrap.response.result.structuredContent ?? null)),
      pingSupported,
      pingError: ping.response.error ?? null,
      results,
    };

    if (process.argv[3]) {
      const reportPath = path.resolve(process.argv[3]);
      fs.mkdirSync(path.dirname(reportPath), { recursive: true });
      fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
    }
    console.log(JSON.stringify(report, null, 2));
  } finally {
    if (child.exitCode === null) {
      child.stdin.end();
      const exited = await Promise.race([
        exitPromise.then(code => ({ code })),
        new Promise(resolve => setTimeout(() => resolve({ timeout: true }), STOP_TIMEOUT_MS)),
      ]);
      if (exited.timeout) child.kill();
      lines?.close();
    }
    assert(path.dirname(root) === os.tmpdir() && path.basename(root).startsWith('harness-mcp-request-latency-'));
    fs.rmSync(root, { recursive: true, force: true });
  }
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
