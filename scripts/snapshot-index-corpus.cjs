'use strict';
// Freeze benchmark inputs before changing code that is itself part of the corpus.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const assert = require('node:assert/strict');
const output = path.resolve(process.argv[2] || 'target/cold-index-corpus-v1');
assert(!fs.existsSync(output), 'Use a new directory; snapshots are never overwritten');
fs.mkdirSync(output, { recursive: true });
const files = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z'],
  { encoding: 'utf8', windowsHide: true }).split('\0').filter(Boolean).sort();
const excluded = new Set(['.git', '.agents', '.codegraph', 'target', 'node_modules', 'bin', 'obj']);
const manifest = [];
for (const relative of files) {
  const parts = relative.replaceAll('\\', '/').split('/');
  if (parts.some(part => excluded.has(part) || /^(?:\.env|credential|secret|id_rsa|id_ed25519|oauth_creds|mcp_config)/i.test(part))) continue;
  if (!fs.existsSync(relative)) continue; // Deleted cache file in the worktree.
  const stat = fs.lstatSync(relative);
  if (!stat.isFile() || stat.size > 1024 * 1024) continue;
  const target = path.join(output, relative);
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.copyFileSync(relative, target, fs.constants.COPYFILE_EXCL);
  manifest.push({ path: relative, sha256: createHash('sha256').update(fs.readFileSync(target)).digest('hex') });
}
fs.writeFileSync(path.join(output, 'benchmark-snapshot-manifest.json'), JSON.stringify(manifest, null, 2));
process.stdout.write(`Frozen ${manifest.length} files at ${output}\n`);
