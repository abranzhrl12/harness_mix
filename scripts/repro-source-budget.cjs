// Reproduce publication failures against a private source/index snapshot.
// The original index is opened read-only and copied using SQLite backup.
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { createHash } = require('node:crypto');

const [exe, original, destination, relative, mode = 'file'] = process.argv.slice(2);
if (!exe || !original || !destination || !relative) {
  throw new Error('usage: node repro-source-budget.cjs EXE SOURCE NEW_DEST RELATIVE_FILE');
}
if (fs.existsSync(destination)) throw new Error('Destination must not exist');
if (!['file', 'workspace'].includes(mode)) throw new Error('Unknown reproduction mode');
const excluded = new Set(['.git', '.codegraph', 'node_modules', '.nuxt', '.output', 'target', 'dist', 'coverage', '.cache', '.turbo']);
let copied = 0;
function copy(source, target) {
  fs.mkdirSync(target, { recursive: true });
  for (const entry of fs.readdirSync(source, { withFileTypes: true })) {
    if (entry.isSymbolicLink() || excluded.has(entry.name) || /^(?:\.env(?:\.|$)|credential|secret|oauth_creds|id_rsa|id_ed25519)/i.test(entry.name)) continue;
    const from = path.join(source, entry.name);
    const to = path.join(target, entry.name);
    if (entry.isDirectory()) copy(from, to);
    else if (entry.isFile()) { fs.copyFileSync(from, to); copied++; }
  }
}
copy(original, destination);
if (fs.existsSync(path.join(original, '.git'))) fs.mkdirSync(path.join(destination, '.git'));
fs.mkdirSync(path.join(destination, '.codegraph'));
const backupCode = `import sqlite3,sys,json,pathlib
source=pathlib.Path(sys.argv[1]).resolve().as_uri()+'?mode=ro'
a=sqlite3.connect(source,uri=True,timeout=1)
a.execute('PRAGMA query_only=ON')
b=sqlite3.connect(sys.argv[2])
a.backup(b)
counts={t:b.execute('SELECT count(*) FROM '+t).fetchone()[0] for t in ['files','nodes','edges','unresolved_refs']}
b.close()
a.close()
print(json.dumps(counts))`;
const backup = spawnSync('python', ['-c', backupCode, path.join(original, '.codegraph', 'codegraph.db'), path.join(destination, '.codegraph', 'codegraph.db')], { encoding: 'utf8', windowsHide: true, timeout: 30000 });
if (backup.status !== 0) throw new Error('SQLite snapshot failed: ' + backup.stderr);
const started = Date.now();
const operation = mode === 'workspace' ? ['--index', 'sync'] : ['--sync-file', relative];
const run = spawnSync(exe, ['--root', destination, ...operation], {
  encoding: 'utf8', windowsHide: true, timeout: 360000, maxBuffer: 32 * 1024 * 1024,
  env: { ...process.env, HARNESS_INDEX_WATCH: '0', HARNESS_INDEX_PROFILE: '1' },
});
fs.writeFileSync(path.join(destination, 'repro-stdout.log'), run.stdout || '');
fs.writeFileSync(path.join(destination, 'repro-stderr.log'), run.stderr || '');
let diagnostic;
let nativeReport;
try { nativeReport = JSON.parse((run.stdout || '').trim()); } catch {}
const errorLine = (run.stderr || '').split(/\r?\n/).find(line => line.startsWith('Error: '));
if (errorLine) {
  let message = errorLine.slice(7);
  try { message = JSON.parse(message); } catch {}
  const marker = 'sqliteDiagnostic=';
  const at = message.indexOf(marker);
  if (at >= 0) { try { diagnostic = JSON.parse(message.slice(at + marker.length)); } catch {} }
}
diagnostic ??= nativeReport?.failure?.diagnostic;
const report = { mode, exeSha256: createHash('sha256').update(fs.readFileSync(exe)).digest('hex'), copied, snapshot: JSON.parse(backup.stdout), exitCode: run.status, durationMs: Date.now() - started, error: run.error?.message, stage: diagnostic?.stage, failureCode: diagnostic?.failureCode, profileMetrics: diagnostic?.profileMetrics, filesProcessed: nativeReport?.filesProcessed, failedFiles: nativeReport?.failedFiles, nativeFilesRemaining: nativeReport?.nativeFilesRemaining, selectedScopeApplied: nativeReport?.selectedScopeApplied, failures: nativeReport?.failures?.map(({path, code, message}) => ({path, code, message})), destination };
fs.writeFileSync(path.join(destination, 'repro-summary.json'), JSON.stringify(report, null, 2));
console.log(JSON.stringify({ ...report, profileMetrics: diagnostic?.profileMetrics ? { counters: diagnostic.profileMetrics.counters } : undefined }));
process.exitCode = run.status ?? 1;
