'use strict';

// Run the existing probe against fresh TempDir indexes. Never reindex the source.
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const assert = require('node:assert/strict');

const sha256 = value => crypto.createHash('sha256').update(value).digest('hex');
const median = values => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];

async function execute(executable, source, sample, workers) {
  const env = { ...process.env, HARNESS_INDEX_PROFILE: '1', HARNESS_INDEX_WORKERS: String(workers),
    HARNESS_INDEX_RESOLUTION_MEMO: '1', HARNESS_INDEX_RETRY_BINDINGS: '1', HARNESS_INDEX_PENDING_QUERY: 'indexed' };
  return new Promise((resolve, reject) => {
    const child = spawn(executable, [source, String(sample), '--batch', '--fresh', '--cold-only'],
      { env, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] });
    const chunks = [];
    let size = 0, stderr = '', failure;
    const timer = setTimeout(() => { failure = new Error('Probe exceeded 660 seconds'); child.kill(); }, 660000);
    child.stdout.on('data', chunk => {
      size += chunk.length;
      if (size > 16 * 1024 * 1024) { failure = new Error('Probe stdout exceeds 16 MiB'); child.kill(); }
      else chunks.push(chunk);
    });
    child.stderr.on('data', chunk => { stderr = (stderr + chunk.toString()).slice(-65536); });
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.on('close', code => {
      clearTimeout(timer);
      if (failure || code !== 0) return reject(failure || new Error(`Probe exit ${code}: ${stderr}`));
      try { resolve({ report: JSON.parse(Buffer.concat(chunks).toString('utf8')), stderr }); }
      catch (error) { reject(error); }
    });
  });
}

async function main() {
  const [binaryArg, sourceArg, outputArg, sampleArg = '200', trialsArg = '3'] = process.argv.slice(2);
  assert(binaryArg && sourceArg && outputArg,
    'Usage: node scripts/benchmark-index-cold.cjs executable sourceRoot NEW_outputDirectory [sample=200] [oddTrials=3]');
  const executable = fs.realpathSync(binaryArg), source = fs.realpathSync(sourceArg), output = path.resolve(outputArg);
  const sample = Number(sampleArg), trials = Number(trialsArg);
  assert(Number.isInteger(sample) && sample >= 1 && sample <= 2000);
  assert(Number.isInteger(trials) && trials >= 3 && trials <= 9 && trials % 2 === 1);
  assert(!fs.existsSync(output), 'Use a new output directory to preserve previous evidence');
  fs.mkdirSync(output, { recursive: true });
  const git = args => execFileSync('git', args, { encoding: 'utf8', windowsHide: true }).trim();
  const sourceSnapshot = () => {
    const files = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--',
      'src', 'crates', 'examples', 'Cargo.toml', 'Cargo.lock'], { encoding: 'utf8', windowsHide: true })
      .split('\0').filter(Boolean).sort();
    return sha256(files.map(file => `${file}\0${fs.existsSync(file) ? sha256(fs.readFileSync(file)) : 'deleted'}`).join('\n'));
  };
  const metadata = {
    recordedAt: new Date().toISOString(), executable, executableSha256: sha256(fs.readFileSync(executable)),
    executableBuildTime: fs.statSync(executable).mtime.toISOString(), sourceRoot: source,
    gitHead: git(['rev-parse', 'HEAD']), gitStatus: git(['status', '--porcelain']),
    trackedDiffSha256: sha256(execFileSync('git', ['diff', 'HEAD', '--', 'src', 'crates', 'examples', 'Cargo.toml', 'Cargo.lock'])),
    sourceSnapshotSha256: sourceSnapshot(),
    os: `${os.platform()} ${os.release()} ${os.arch()}`, cpu: os.cpus()[0]?.model,
    logicalProcessors: os.cpus().length, sample, trials, profile: 'release',
    settings: { pendingQuery: 'indexed', resolutionMemo: true, retryBindings: true },
    definition: 'Fresh empty temporary index for every invocation; OS caches are not flushed. Copy/init/fingerprints excluded from elapsedMs.'
  };
  fs.writeFileSync(path.join(output, 'metadata.json'), JSON.stringify(metadata, null, 2));
  const runs = [];
  let expected;
  for (let trial = 1; trial <= trials; trial++) {
    // Alternate worker order to reduce consistent first-run/cache bias.
    for (const workers of trial % 2 ? [1, 4] : [4, 1]) {
      process.stdout.write(`cold sample=${sample} trial=${trial}/${trials} workers=${workers}\n`);
      const { report, stderr } = await execute(executable, source, sample, workers);
      fs.writeFileSync(path.join(output, `workers-${workers}-trial-${trial}.json`), JSON.stringify(report, null, 2));
      assert.equal(sourceSnapshot(), metadata.sourceSnapshotSha256, 'Source/build inputs changed during the benchmark');
      const pass = report.passes?.[0], workspace = pass?.workspaceReport;
      assert.equal(report.copiedExistingGraph, false);
      assert.deepEqual(report.graphBefore, { files: 0, nodes: 0, edges: 0 });
      assert.equal(report.passes.length, 1);
      assert.equal(report.sampleFiles, sample, 'Corpus smaller than requested sample');
      assert.equal(workspace.databaseWriters, 1);
      assert.equal(workspace.extractionWorkers, workers);
      const evidence = {
        sampleSourceHash: report.sampleSourceHash, graphHash: pass.graphHash,
        filesProcessed: workspace.filesProcessed, changedFilesCommitted: workspace.changedFilesCommitted,
        nativeCandidates: workspace.nativeCandidates, nativeFilesRemaining: workspace.nativeFilesRemaining,
        totalReferencesAttempted: workspace.totalReferencesAttempted,
        totalReferencesResolved: workspace.totalReferencesResolved, totalNodesWritten: workspace.totalNodesWritten,
        // Compare semantic failures, not variable timing/path details in diagnostics.
        failures: workspace.failures?.map(({path, code, message, commitState, stage}) => ({path, code, message, commitState, stage})),
        uncheckedFilePaths: workspace.uncheckedFilePaths,
        skippedFileReports: workspace.skippedFileReports?.map(({path, code, message}) => ({path, code, message}))
      };
      if (!expected) expected = evidence;
      assert.deepEqual(evidence, expected, 'Source/graph/results differ: do not treat timings as an equivalent optimization');
      runs.push({ trial, workers, elapsedMs: pass.elapsedMs, preparationDurationMicros: workspace.preparationDurationMicros,
        phaseTimingsMicros: pass.phaseTimingsMicros, evidence, stderr });
      fs.writeFileSync(path.join(output, 'runs.json'), JSON.stringify(runs, null, 2));
      process.stdout.write(`  ${(pass.elapsedMs / 1000).toFixed(3)} s; files=${workspace.filesProcessed}; failures=${workspace.failures?.length}\n`);
    }
  }
  const summaries = [1, 4].map(workers => {
    const selected = runs.filter(run => run.workers === workers);
    const phases = Object.keys(selected[0].phaseTimingsMicros || {});
    return { workers, elapsedMs: selected.map(run => run.elapsedMs), medianElapsedMs: median(selected.map(run => run.elapsedMs)),
      medianPreparationMicros: median(selected.map(run => run.preparationDurationMicros)),
      phaseMediansMicros: Object.fromEntries(phases.map(phase => [phase, median(selected.map(run => run.phaseTimingsMicros[phase]))])) };
  });
  const summary = { metadata, evidence: expected, summaries,
    fourWorkerSpeedup: summaries[0].medianElapsedMs / summaries[1].medianElapsedMs,
    limitations: ['Phase medians are independent and must not be summed as a measured total.',
      'Lexicographic source sample; language mix and dependency coverage may differ from a complete project.',
      'Tracks current executable identity; git status is provenance, not proof that an older binary matches source.',
      'Memory/CPU utilization and commit/fsync cost beyond existing phase counters are not measured.'] };
  fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify(summary, null, 2));
  process.stdout.write(`Summary: ${path.join(output, 'summary.json')}\n`);
}

module.exports = { execute, median, sha256 };
if (require.main === module) {
  main().catch(error => { process.stderr.write(`${error.stack}\n`); process.exitCode = 1; });
}
