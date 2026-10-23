'use strict';
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const os = require('node:os');
const { execute, median, sha256 } = require('./benchmark-index-cold.cjs');

async function main() {
  const [beforeArg, afterArg, sourceArg, outputArg, countArg = '200', trialsArg = '3', mode = 'binaries'] = process.argv.slice(2);
  assert(beforeArg && afterArg && sourceArg && outputArg,
    'Usage: node scripts/benchmark-index-resolver.cjs beforeExe afterExe frozenSource NEW_outputDirectory [sample=200|all] [oddTrials=3] [binaries|rust-pending|pinned-read]');
  assert(['binaries', 'rust-pending', 'pinned-read'].includes(mode), 'Unsupported comparison mode');
  const binaries = { before: fs.realpathSync(beforeArg), after: fs.realpathSync(afterArg) };
  if (mode !== 'binaries') {
    assert.equal(sha256(fs.readFileSync(binaries.before)), sha256(fs.readFileSync(binaries.after)),
      'Selector comparisons require identical binary contents');
  }
  const source = fs.realpathSync(sourceArg), output = path.resolve(outputArg);
  const allSources = countArg === 'all';
  const count = allSources ? 2000 : Number(countArg), trials = Number(trialsArg);
  assert(Number.isInteger(count) && count >= 1 && count <= 2000);
  assert(Number.isInteger(trials) && trials >= 3 && trials <= 9 && trials % 2 === 1);
  assert(!fs.existsSync(output), 'Use a new output directory');
  const manifest = path.join(source, 'benchmark-snapshot-manifest.json');
  assert(fs.existsSync(manifest), 'Freeze the corpus before changing production sources');
  const inputManifest = JSON.parse(fs.readFileSync(manifest, 'utf8'));
  const checkCorpus = () => {
    for (const file of inputManifest) {
      assert.equal(sha256(fs.readFileSync(path.join(source, file.path))), file.sha256, `Changed frozen source: ${file.path}`);
    }
  };
  checkCorpus();
  fs.mkdirSync(output, { recursive: true });
  const rustSettings = { before: mode === 'rust-pending' ? '0' : '1', after: '1' };
  const pinSettings = { before: '0', after: mode === 'pinned-read' ? '1' : '0' };
  const metadata = { recordedAt: new Date().toISOString(), source, sample: allSources ? 'all' : count, trials,
    comparisonMode: mode, rustPending: rustSettings, pinnedRead: pinSettings,
    corpusManifestSha256: sha256(fs.readFileSync(manifest)), workers: 4,
    binaries: Object.fromEntries(Object.entries(binaries).map(([name, file]) => [name,
      { path: file, sha256: sha256(fs.readFileSync(file)), modifiedAt: fs.statSync(file).mtime.toISOString() }])),
    cpu: os.cpus()[0]?.model, os: `${os.platform()} ${os.release()} ${os.arch()}`,
    definition: 'Empty temporary index per invocation; fixed corpus; OS caches not flushed; profiler enabled on both binaries.' };
  fs.writeFileSync(path.join(output, 'metadata.json'), JSON.stringify(metadata, null, 2));
  const checkBinaries = () => {
    for (const [variant, file] of Object.entries(binaries)) {
      assert.equal(sha256(fs.readFileSync(file)), metadata.binaries[variant].sha256,
        `Binary changed during benchmark: ${variant}`);
    }
  };
  const runs = [];
  let expected;
  for (let trial = 1; trial <= trials; trial++) {
    for (const variant of trial % 2 ? ['before', 'after'] : ['after', 'before']) {
      process.stdout.write(`resolver trial=${trial}/${trials} variant=${variant}\n`);
      checkBinaries();
      const { report, stderr } = await execute(binaries[variant], source, count, 4, rustSettings[variant], pinSettings[variant]);
      fs.writeFileSync(path.join(output, `${variant}-${trial}.json`), JSON.stringify(report, null, 2));
      checkCorpus();
      checkBinaries();
      const pass = report.passes?.[0], workspace = pass?.workspaceReport;
      assert.equal(report.copiedExistingGraph, false);
      assert.deepEqual(report.graphBefore, { files: 0, nodes: 0, edges: 0 });
      assert.equal(report.passes.length, 1);
      if (allSources) {
        assert(report.copiedSourceFiles < count, 'Full corpus reaches the probe cap; coverage is not established');
        assert.equal(report.sampleFiles, report.copiedSourceFiles, 'Full corpus was truncated');
        assert.equal(workspace.nativeFilesRemaining, 0, 'Full corpus has unprocessed native files');
      } else {
        assert.equal(report.sampleFiles, count);
      }
      assert.equal(workspace.databaseWriters, 1);
      const evidence = { sourceHash: report.sampleSourceHash, graphHash: pass.graphHash,
        filesProcessed: workspace.filesProcessed, referencesAttempted: workspace.totalReferencesAttempted,
        referencesResolved: workspace.totalReferencesResolved, nodesWritten: workspace.totalNodesWritten,
        remaining: workspace.nativeFilesRemaining, unchecked: workspace.uncheckedFilePaths,
        failures: workspace.failures?.map(({path, code, message, commitState}) => ({path, code, message, commitState})),
        skipped: workspace.skippedFileReports?.map(({path, code, message}) => ({path, code, message})) };
      if (!expected) expected = evidence;
      assert.deepEqual(evidence, expected, 'Behavior changed: inspect raw graph/results before comparing speed');
      runs.push({ variant, trial, elapsedMs: pass.elapsedMs, phases: pass.phaseTimingsMicros,
        evidence, stderr });
      fs.writeFileSync(path.join(output, 'runs.json'), JSON.stringify(runs, null, 2));
      process.stdout.write(`  ${(pass.elapsedMs / 1000).toFixed(3)} s\n`);
    }
  }
  const variants = ['before', 'after'].map(variant => {
    const selected = runs.filter(run => run.variant === variant);
    return { variant, elapsedMs: selected.map(run => run.elapsedMs),
      medianElapsedMs: median(selected.map(run => run.elapsedMs)),
      phaseMediansMicros: Object.fromEntries(Object.keys(selected[0].phases).map(key =>
        [key, median(selected.map(run => run.phases[key]))])) };
  });
  const summary = { metadata, evidence: expected, variants,
    speedup: variants[0].medianElapsedMs / variants[1].medianElapsedMs,
    limitations: ['Identify code and profiling differences for each experiment before attributing runtime changes to a component.',
      'Subphase medians are independent; nested subphase timers must not be added to their parent duration.',
      'This corpus/sample does not establish semantic equivalence for every language or workload.'] };
  fs.writeFileSync(path.join(output, 'summary.json'), JSON.stringify(summary, null, 2));
  process.stdout.write(`Summary: ${path.join(output, 'summary.json')}\n`);
}

main().catch(error => { process.stderr.write(`${error.stack}\n`); process.exitCode = 1; });
