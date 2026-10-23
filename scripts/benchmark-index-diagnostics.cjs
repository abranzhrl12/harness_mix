'use strict';
const fs = require('node:fs');
const path = require('node:path');
const os = require('node:os');
const assert = require('node:assert/strict');
const { execute, median, sha256 } = require('./benchmark-index-cold.cjs');

async function main() {
  const [binaryArg, sourceArg, baselineArg, outputArg, sampleArg='200', trialsArg='3'] = process.argv.slice(2);
  assert(binaryArg && sourceArg && baselineArg && outputArg,
    'Usage: node scripts/benchmark-index-diagnostics.cjs binary frozenSource previousSummary NEW_output [sample=200] [oddTrials=3]');
  const binary=fs.realpathSync(binaryArg), source=fs.realpathSync(sourceArg), output=path.resolve(outputArg);
  const sample=Number(sampleArg), trials=Number(trialsArg);
  assert(Number.isInteger(sample) && sample>=1 && sample<=2000);
  assert(Number.isInteger(trials) && trials>=3 && trials<=9 && trials%2===1);
  assert(!fs.existsSync(output),'Use a new output directory');
  const baseline=JSON.parse(fs.readFileSync(baselineArg,'utf8'));
  const manifestPath=path.join(source,'benchmark-snapshot-manifest.json');
  const manifest=JSON.parse(fs.readFileSync(manifestPath,'utf8'));
  const check=()=>{ for(const file of manifest) assert.equal(sha256(fs.readFileSync(path.join(source,file.path))),file.sha256); };
  check(); fs.mkdirSync(output,{recursive:true});
  const metadata={recordedAt:new Date().toISOString(),binary,binarySha256:sha256(fs.readFileSync(binary)),
    corpusManifestSha256:sha256(fs.readFileSync(manifestPath)),source,sample,trials,workers:4,
    cpu:os.cpus()[0]?.model,os:`${os.platform()} ${os.release()} ${os.arch()}`,
    definition:'Empty temporary index per invocation; detailed profiler enabled; counts and wall time, not CPU samples; OS caches not flushed.'};
  fs.writeFileSync(path.join(output,'metadata.json'),JSON.stringify(metadata,null,2));
  const runs=[];
  for(let trial=1;trial<=trials;trial++) {
    process.stdout.write(`diagnostic trial=${trial}/${trials}\n`);
    const {report,stderr}=await execute(binary,source,sample,4);
    fs.writeFileSync(path.join(output,`trial-${trial}.json`),JSON.stringify(report,null,2));
    check(); const pass=report.passes[0], w=pass.workspaceReport;
    assert.equal(report.copiedExistingGraph,false); assert.equal(report.sampleFiles,sample);
    assert.deepEqual(report.graphBefore,{files:0,nodes:0,edges:0});
    assert.equal(pass.graphHash,baseline.evidence.graphHash); assert.equal(report.sampleSourceHash,baseline.evidence.sourceHash);
    assert.equal(w.filesProcessed,baseline.evidence.filesProcessed);
    assert.equal(w.totalReferencesAttempted,baseline.evidence.referencesAttempted);
    assert.equal(w.totalReferencesResolved,baseline.evidence.referencesResolved);
    assert.equal(w.totalNodesWritten,baseline.evidence.nodesWritten);
    assert.equal(w.nativeFilesRemaining,0); assert.deepEqual(w.failures,[]);
    assert.deepEqual(w.uncheckedFilePaths,[]); assert.deepEqual(w.skippedFileReports,[]);
    assert(w.profileMetrics?.operations,'Detailed diagnostics missing');
    const publicationMicros=Object.entries(pass.phaseTimingsMicros)
      .filter(([name])=>!name.startsWith('resolver_')).reduce((sum,[,value])=>sum+value,0);
    const accountedMicros=publicationMicros+w.planningDurationMicros+w.preparationDurationMicros+w.finalizationDurationMicros;
    runs.push({trial,elapsedMs:pass.elapsedMs,phases:pass.phaseTimingsMicros,metrics:w.profileMetrics,
      planningMicros:w.planningDurationMicros,preparationMicros:w.preparationDurationMicros,
      finalizationMicros:w.finalizationDurationMicros,
      reconciliation:{publicationMicros,accountedMicros,remainderMicros:pass.elapsedMs*1000-accountedMicros},stderr});
    process.stdout.write(`  ${(pass.elapsedMs/1000).toFixed(3)} s; graph identical\n`);
  }
  const keys=Object.keys(runs[0].metrics.operations);
  const operations=keys.map(name=>({name,calls:runs.map(r=>r.metrics.operations[name].calls),
    medianTotalMicros:median(runs.map(r=>r.metrics.operations[name].totalMicros)),
    medianMeanMicros:median(runs.map(r=>r.metrics.operations[name].meanMicros))})).sort((a,b)=>b.medianTotalMicros-a.medianTotalMicros);
  const summary={metadata,evidence:baseline.evidence,elapsedMs:runs.map(r=>r.elapsedMs),medianElapsedMs:median(runs.map(r=>r.elapsedMs)),
    medianPlanningMicros:median(runs.map(r=>r.planningMicros)),
    medianPreparationMicros:median(runs.map(r=>r.preparationMicros)),
    medianFinalizationMicros:median(runs.map(r=>r.finalizationMicros)),
    reconciliation:runs.map(r=>({trial:r.trial,...r.reconciliation})),
    phaseMediansMicros:Object.fromEntries(Object.keys(runs[0].phases).map(name=>[name,median(runs.map(r=>r.phases[name]))])),
    operations,counters:runs.map(r=>r.metrics.counters),
    paths:runs.map(r=>({unique:r.metrics.uniqueSourcePathsRecorded,omittedAttempts:r.metrics.sourcePathsOmittedAttempts,
      totalAttempts:Object.values(r.metrics.sourceReads).reduce((a,b)=>a+b,0),
      topRepeated:Object.entries(r.metrics.sourceReads).sort((a,b)=>b[1]-a[1]).slice(0,15)})),
    coverage:runs[0].metrics.coverage,
    limitations:['Inclusive nested timers must not be summed; means are not p95.',
      'Operations instrument successful publications on the main thread; parallel preparation remains aggregate only.',
      'Detailed tracing adds overhead: use this run to locate cost, not claim a before/after speedup.',
      'Wall time alone cannot distinguish CPU from filesystem/OS wait.']};
  fs.writeFileSync(path.join(output,'summary.json'),JSON.stringify(summary,null,2));
  process.stdout.write(`Summary: ${path.join(output,'summary.json')}\n`);
}
main().catch(error=>{process.stderr.write(`${error.stack}\n`);process.exitCode=1;});
