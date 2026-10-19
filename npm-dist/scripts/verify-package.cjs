const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const root = path.join(__dirname, '..');
const manifest = require(path.join(root, 'package.json'));
const binary = path.join(root, 'bin', 'agent-harness.exe');
if (process.platform !== 'win32' || process.arch !== 'x64') {
  throw new Error('Build and verify this Windows x64 package on Windows x64.');
}
for (const file of ['bin/run.js', 'bin/agent-harness.exe', 'CodeGraph-LICENSE.txt', 'ROLLDOWN-LICENSE.txt', 'README.md']) {
  if (!fs.statSync(path.join(root, file)).isFile()) throw new Error(`Missing package file: ${file}`);
}
const actual = execFileSync(binary, ['--version'], { encoding: 'utf8', timeout: 10000, windowsHide: true }).trim();
if (!actual.startsWith('agent-harness')) {
  throw new Error(`Invalid binary version response: ${actual}`);
}
console.log(`Verified ${manifest.name}@${manifest.version}: executable and notices present.`);
