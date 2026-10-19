const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { spawn } = require('node:child_process');
const readline = require('node:readline');
const root = path.join(__dirname, '..');
const manifest = require(path.join(root, 'package.json'));
const binary = path.join(root, 'bin', 'agent-harness.exe');
function validateVersion(actual, version) {
  if (actual.trim() !== `agent-harness ${version}`) {
    throw new Error(`Binary/package version mismatch: expected ${version}, received ${actual.trim()}`);
  }
}
function validateTools(tools) {
  const byName = new Map(tools.map(tool => [tool.name, tool.inputSchema]));
  for (const name of ['harness_write', 'harness_replace', 'harness_verify', 'harness_bootstrap']) {
    if (!byName.has(name)) throw new Error(`Missing MCP tool: ${name}`);
  }
  const write = byName.get('harness_write');
  if (!write.required?.includes('path') || !write.required?.includes('content')) {
    throw new Error('harness_write must require path and content');
  }
  if (!write.properties?.createParents || !write.properties?.allowUnvalidatedText) {
    throw new Error('harness_write must support createParents and allowUnvalidatedText');
  }
  if (!byName.get('harness_bootstrap').properties?.compact) {
    throw new Error('harness_bootstrap must support compact');
  }
  const replace = byName.get('harness_replace');
  if (!replace.properties?.hash || replace.required?.includes('hash') || !replace.properties?.chunks) {
    throw new Error('harness_replace must support optional hash and chunks');
  }
  if (!byName.get('harness_verify').properties?.package) {
    throw new Error('harness_verify must support package');
  }
}
function verifyMcp(executable, workspace) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, ['--root', workspace], { windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'] });
    let settled = false;
    let bytes = 0;
    const lines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
    const finish = error => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      lines.close();
      child.stdin.destroy();
      child.kill();
      if (error) reject(error); else resolve();
    };
    const timer = setTimeout(() => finish(new Error('MCP contract probe timed out')), 10000);
    const send = message => child.stdin.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n');
    child.on('error', finish);
    child.stdin.on('error', finish);
    child.on('exit', () => finish(new Error('MCP exited before contract verification')));
    child.stderr.on('data', () => {}); // Drain without printing potentially sensitive diagnostics.
    child.stdout.on('data', chunk => {
      bytes += chunk.length;
      if (bytes > 2 * 1024 * 1024) finish(new Error('MCP contract probe exceeded output budget'));
    });
    lines.on('line', line => {
      if (settled) return;
      try {
        const reply = JSON.parse(line);
        if (reply.error) throw new Error(`MCP probe failed: ${reply.error.code}`);
        if (reply.id === 1) {
          if (!reply.result?.capabilities?.tools) throw new Error('MCP tools capability missing');
          send({ method: 'notifications/initialized' });
          send({ id: 2, method: 'tools/list', params: {} });
        } else if (reply.id === 2) {
          validateTools(reply.result?.tools || []);
          finish();
        }
      } catch (error) { finish(error); }
    });
    send({ id: 1, method: 'initialize', params: {
      protocolVersion: '2024-11-05', capabilities: {}, clientInfo: { name: 'package-verifier', version: '1' }
    } });
  });
}
async function main() {
if (process.platform !== 'win32' || process.arch !== 'x64') {
  throw new Error('Build and verify this Windows x64 package on Windows x64.');
}
for (const file of ['bin/run.js', 'bin/agent-harness.exe', 'CodeGraph-LICENSE.txt', 'ROLLDOWN-LICENSE.txt', 'README.md']) {
  if (!fs.statSync(path.join(root, file)).isFile()) throw new Error(`Missing package file: ${file}`);
}
const actual = execFileSync(binary, ['--version'], { encoding: 'utf8', timeout: 10000, windowsHide: true }).trim();
validateVersion(actual, manifest.version);
await verifyMcp(binary, root);
console.log(`Verified ${manifest.name}@${manifest.version}: version, MCP capabilities and notices match.`);
}
module.exports = { validateVersion, validateTools, verifyMcp };
if (require.main === module) main().catch(error => { console.error(error.message); process.exitCode = 1; });
