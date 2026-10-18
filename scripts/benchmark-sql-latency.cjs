'use strict';
// Dedicated loopback PostgreSQL fixture only. Artificial chunk delay is not a production RTT measurement.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const { Transform } = require('node:stream');
const { spawn } = require('node:child_process');

async function main() {
  const [before, after, caFile, outputDirectory, delayText = '20', countText = '10'] = process.argv.slice(2);
  assert(before && after && caFile && outputDirectory, 'Usage: node benchmark-sql-latency.cjs before.exe after.exe fixture-ca.pem output-directory [per-chunk-delay-ms] [iterations]');
  const delay = Number(delayText); const count = Number(countText);
  assert(Number.isInteger(delay) && delay >= 0 && delay <= 100);
  assert(Number.isInteger(count) && count >= 5 && count <= 100);
  const output = path.resolve(outputDirectory); fs.mkdirSync(output, { recursive: true });
  const fixture = fs.mkdtempSync(path.join(output, 'fixture-'));
  fs.mkdirSync(path.join(fixture, '.agents')); fs.copyFileSync(caFile, path.join(fixture, 'pg-ca.pem'));
  const sockets = new Set();
  class Delay extends Transform {
    _transform(chunk, encoding, callback) {
      if (delay === 0) { this.push(chunk); callback(); }
      else setTimeout(() => { this.push(chunk); callback(); }, delay);
    }
  }
  const server = net.createServer(front => {
    const upstream = net.connect({ host: '127.0.0.1', port: 54389 });
    for (const socket of [front, upstream]) {
      sockets.add(socket); socket.setNoDelay(true); socket.once('close', () => sockets.delete(socket));
      socket.on('error', () => { front.destroy(); upstream.destroy(); });
    }
    front.pipe(new Delay()).pipe(upstream); upstream.pipe(new Delay()).pipe(front);
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
  const env = { ...process.env, HARNESS_PG_BENCH_DSN: `host=localhost port=${server.address().port} user=harness_reader dbname=postgres` };
  const summaries = {};
  try {
    for (const [label, executable, ttl] of [['before', before, 0], ['after-live', after, 0], ['after-cache', after, 60]]) {
      fs.writeFileSync(path.join(fixture, '.agents', 'harness.config.json'), JSON.stringify({ postgresql: {
        enabled: true, schemaCacheSeconds: ttl, connection: { source: 'environment', name: 'HARNESS_PG_BENCH_DSN' },
        tls: { mode: 'verify-full', caFile: 'pg-ca.pem' },
      }}));
      const artifact = path.join(output, `${label}.json`);
      await new Promise((resolve, reject) => {
        const child = spawn(process.execPath, [path.join(__dirname, 'benchmark-postgresql.cjs'), path.resolve(executable), fixture, artifact, String(count)], { env, stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true });
        let error = ''; child.stdout.resume(); child.stderr.on('data', chunk => { if (error.length < 10000) error += chunk; });
        child.once('error', reject); child.once('exit', code => code === 0 ? resolve() : reject(new Error(`${label} failed (${code}): ${error}`)));
      });
      summaries[label] = JSON.parse(fs.readFileSync(artifact, 'utf8')).measures;
      for (const measure of Object.values(summaries[label])) delete measure.samplesMs;
    }
    const result = { mode: delay > 0 ? 'dedicated-loopback-TLS-with-artificial-chunk-delay' : 'dedicated-loopback-TLS-proxy-no-added-delay', perChunkDelayMs: delay, iterations: count, fixture, summaries };
    fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result, null, 2));
  } finally {
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => server.close(resolve));
  }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
