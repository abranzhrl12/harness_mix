// Measure the real MCP stdio startup path, isolated from tool execution.
// Usage: node scripts/benchmark-mcp-startup.cjs EXE [REPORT.json] [SAMPLES=15] [WARMUPS=3]
const { spawn, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');

const TIMEOUT_MS = 20_000;
const STOP_TIMEOUT_MS = 5_000;

function sha256(file) {
  return crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
}

function percentile(sorted, p) {
  return sorted[Math.max(0, Math.ceil(p * sorted.length) - 1)];
}

function summarize(valuesNs) {
  const sorted = [...valuesNs].sort((a, b) => a < b ? -1 : a > b ? 1 : 0);
  const median = sorted.length % 2
    ? sorted[(sorted.length - 1) / 2]
    : (sorted[sorted.length / 2 - 1] + sorted[sorted.length / 2]) / 2n;
  const toMs = value => Math.round(Number(value) / 1e6 * 1000) / 1000;
  return {
    n: sorted.length,
    minMs: toMs(sorted[0]),
    medianMs: toMs(median),
    p95NearestRankMs: toMs(percentile(sorted, 0.95)),
    maxMs: toMs(sorted.at(-1)),
    samplesMs: valuesNs.map(toMs),
  };
}

function childEnvironment() {
  const env = {};
  // Keep only variables needed to launch a native Windows process. In
  // particular, do not pass database credentials or developer service tokens.
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

async function runOne(executable, env) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-mcp-startup-'));
  fs.mkdirSync(path.join(root, '.agents'));
  fs.writeFileSync(
    path.join(root, '.agents', 'harness.config.json'),
    JSON.stringify({ sql: { enabled: false }, postgresql: { enabled: false } }),
  );

  let child;
  let lines;
  let exitPromise;
  let stderrTail = '';
  const pending = new Map();
  let nextId = 1;

  const waitForExit = async () => {
    if (!child || child.exitCode !== null) return child?.exitCode;
    const result = await Promise.race([
      exitPromise.then(code => ({ code })),
      new Promise(resolve => setTimeout(() => resolve({ timeout: true }), STOP_TIMEOUT_MS)),
    ]);
    if (result.timeout) {
      child.kill();
      throw new Error('MCP server did not exit after stdin EOF');
    }
    return result.code;
  };

  try {
    const startedAtNs = process.hrtime.bigint();
    child = spawn(executable, ['--root', root], {
      env,
      windowsHide: true,
      stdio: ['pipe', 'pipe', 'pipe'],
    });
    exitPromise = new Promise(resolve => child.once('exit', code => resolve(code)));
    const failPending = error => {
      for (const waiter of pending.values()) waiter.reject(error);
      pending.clear();
    };
    child.on('error', failPending);
    child.on('exit', code => failPending(new Error(`MCP server exited early (${code}): ${stderrTail}`)));
    child.stderr.on('data', chunk => {
      stderrTail = (stderrTail + chunk.toString('utf8')).slice(-8_000);
    });
    lines = readline.createInterface({ input: child.stdout });
    lines.on('line', line => {
      let message;
      try {
        message = JSON.parse(line);
      } catch (error) {
        failPending(new Error(`Non-JSON stdout from MCP server: ${line.slice(0, 500)} (${error})`));
        return;
      }
      if (message.id !== undefined) {
        const waiter = pending.get(message.id);
        if (waiter) {
          pending.delete(message.id);
          waiter.resolve(message);
        }
      }
    });

    const request = (method, params) => new Promise((resolve, reject) => {
      const id = nextId++;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error(`Timeout waiting for ${method} (${TIMEOUT_MS} ms). ${stderrTail}`));
      }, TIMEOUT_MS);
      pending.set(id, {
        resolve: value => { clearTimeout(timer); resolve(value); },
        reject: error => { clearTimeout(timer); reject(error); },
      });
      child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id, method, params })}\n`);
    });

    const initialized = await request('initialize', {
      protocolVersion: '2025-11-25',
      capabilities: {},
      clientInfo: { name: 'mcp-startup-benchmark', version: '1' },
    });
    const initializeNs = process.hrtime.bigint() - startedAtNs;
    assert(initialized.result?.capabilities, `initialize failed: ${JSON.stringify(initialized)}`);

    child.stdin.write(`${JSON.stringify({
      jsonrpc: '2.0', method: 'notifications/initialized',
    })}\n`);
    const listStartedAtNs = process.hrtime.bigint();
    const listed = await request('tools/list', {});
    const toolsListNs = process.hrtime.bigint() - listStartedAtNs;
    assert(Array.isArray(listed.result?.tools), `tools/list failed: ${JSON.stringify(listed)}`);

    const bootstrapStartedAtNs = process.hrtime.bigint();
    const bootstrap = await request('tools/call', {
      name: 'harness_bootstrap', arguments: { compact: true },
    });
    const bootstrapNs = process.hrtime.bigint() - bootstrapStartedAtNs;
    assert(bootstrap.result && bootstrap.result.isError !== true,
      `harness_bootstrap failed: ${JSON.stringify(bootstrap)}`);

    child.stdin.end();
    const exitCode = await waitForExit();
    assert.equal(exitCode, 0, `MCP server exit was ${exitCode}: ${stderrTail}`);

    return {
      spawnToInitializeNs: initializeNs,
      initializedToToolsListNs: toolsListNs,
      toolsListToBootstrapNs: bootstrapNs,
      spawnToToolsListNs: initializeNs + toolsListNs,
      spawnToBootstrapNs: initializeNs + toolsListNs + bootstrapNs,
      protocolVersion: initialized.result.protocolVersion,
      serverInfo: initialized.result.serverInfo,
      advertisedToolCount: listed.result.tools.length,
      bootstrapResponseBytes: Buffer.byteLength(JSON.stringify(bootstrap)),
    };
  } finally {
    if (child && child.exitCode === null) {
      child.stdin.end();
      await waitForExit().catch(() => {});
    }
    lines?.close();
    assert(path.dirname(root) === os.tmpdir() && path.basename(root).startsWith('harness-mcp-startup-'));
    fs.rmSync(root, { recursive: true, force: true });
  }
}

async function main() {
  assert(process.argv[2], 'Usage: node scripts/benchmark-mcp-startup.cjs EXE [REPORT.json] [SAMPLES=15] [WARMUPS=3]');
  const executable = path.resolve(process.argv[2]);
  const samples = Number(process.argv[4] || 15);
  const warmups = Number(process.argv[5] || 3);
  assert(fs.statSync(executable).isFile(), `Not a file: ${executable}`);
  assert(Number.isInteger(samples) && samples >= 15 && samples <= 100, 'SAMPLES must be 15..100');
  assert(Number.isInteger(warmups) && warmups >= 1 && warmups <= 20, 'WARMUPS must be 1..20');

  const env = childEnvironment();
  const warmupResults = [];
  for (let i = 0; i < warmups; i++) warmupResults.push(await runOne(executable, env));

  const results = [];
  for (let i = 0; i < samples; i++) results.push(await runOne(executable, env));
  const fields = [
    ['Process spawn → MCP initialize response', 'spawnToInitializeNs'],
    ['notifications/initialized → tools/list response', 'initializedToToolsListNs'],
    ['tools/list response → harness_bootstrap response', 'toolsListToBootstrapNs'],
    ['Process spawn → tools/list response', 'spawnToToolsListNs'],
    ['Process spawn → harness_bootstrap response', 'spawnToBootstrapNs'],
  ];
  const summary = Object.fromEntries(fields.map(([name, key]) => [
    name,
    summarize(results.map(result => result[key])),
  ]));

  const git = spawnSync('git', ['-C', path.dirname(path.dirname(executable)), 'rev-parse', '--short', 'HEAD'], {
    encoding: 'utf8', windowsHide: true,
  });
  const projectRoot = path.resolve(__dirname, '..');
  const status = spawnSync('git', ['-C', projectRoot, 'status', '--porcelain'], {
    encoding: 'utf8', windowsHide: true,
  });
  const inputFiles = [
    'Cargo.toml', 'Cargo.lock', 'src/main.rs', 'src/cli.rs', 'src/transport/runtime/single.rs',
    'crates/mcp/src/server.rs', 'scripts/benchmark-mcp-startup.cjs',
  ];
  const report = {
    recordedAt: new Date().toISOString(),
    executable,
    executableSha256: sha256(executable),
    executableModifiedAt: fs.statSync(executable).mtime.toISOString(),
    profile: 'release; cargo build --locked --release --bin agent-harness',
    version: spawnSync(executable, ['--version'], { encoding: 'utf8', env, windowsHide: true }).stdout.trim(),
    sourceRevision: git.status === 0 ? git.stdout.trim() : null,
    workingTree: status.status === 0 ? {
      dirty: status.stdout.trim().length > 0,
      changedPathCount: status.stdout.trim() ? status.stdout.trim().split(/\r?\n/).length : 0,
      statusSha256: crypto.createHash('sha256').update(status.stdout).digest('hex'),
    } : null,
    sourceSha256: Object.fromEntries(inputFiles.map(file => [file, sha256(path.join(projectRoot, file))])),
    runtime: {
      node: process.version,
      rustc: spawnSync('rustc', ['--version'], { encoding: 'utf8', windowsHide: true }).stdout.trim(),
      os: `${os.platform()} ${os.release()} ${os.arch()}`,
      cpu: os.cpus()[0]?.model,
      logicalProcessors: os.cpus().length,
      memoryGiB: Math.round(os.totalmem() / 2 ** 30),
    },
    protocol: 'MCP stdio, JSON-RPC 2.0 newline-delimited; initialize protocolVersion 2025-11-25; then notifications/initialized, tools/list, tools/call harness_bootstrap(compact=true)',
    environment: 'Each server process receives only Windows launch variables (PATH/SystemRoot/WINDIR/TEMP/TMP/user paths); HARNESS_TOOL_PROFILE=full; HARNESS_INDEX_WATCH=0; temporary workspace; SQL and PostgreSQL disabled in its local harness.config.json; no external DB/network/model request.',
    method: 'Monotonic process.hrtime.bigint wall clock; each measured observation uses a new server process and fresh temporary workspace; warm-up processes excluded; sequential phases; no OS cache flush, so results are warm-cache observations after normal system warmup, not controlled cold starts. p95 uses nearest-rank definition.',
    samples,
    warmups,
    warmupCheck: {
      serverInfo: warmupResults.at(-1)?.serverInfo,
      advertisedToolCount: warmupResults.at(-1)?.advertisedToolCount,
    },
    measuredServerInfo: results.at(-1)?.serverInfo,
    advertisedToolCount: results.at(-1)?.advertisedToolCount,
    bootstrapResponseBytes: results.at(-1)?.bootstrapResponseBytes,
    summary,
    observations: results.map((result, index) => ({
      trial: index + 1,
      spawnToInitializeNs: result.spawnToInitializeNs.toString(),
      initializedToToolsListNs: result.initializedToToolsListNs.toString(),
      toolsListToBootstrapNs: result.toolsListToBootstrapNs.toString(),
      spawnToToolsListNs: result.spawnToToolsListNs.toString(),
      spawnToBootstrapNs: result.spawnToBootstrapNs.toString(),
      protocolVersion: result.protocolVersion,
      serverInfo: result.serverInfo,
      advertisedToolCount: result.advertisedToolCount,
      bootstrapResponseBytes: result.bootstrapResponseBytes,
    })),
  };

  if (process.argv[3]) {
    const reportPath = path.resolve(process.argv[3]);
    fs.mkdirSync(path.dirname(reportPath), { recursive: true });
    fs.writeFileSync(reportPath, `${JSON.stringify(report, null, 2)}\n`);
  }
  console.log(JSON.stringify(report, null, 2));
}

main().catch(error => {
  console.error(error);
  process.exitCode = 1;
});
