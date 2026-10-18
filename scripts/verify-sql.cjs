'use strict';
// Reproducible SQL regressions. Live databases are opt-in, never auto-discovered.
const { spawn } = require('node:child_process');
const path = require('node:path');
const root = path.resolve(__dirname, '..');

function cargo(args) {
  return new Promise((resolve, reject) => {
    const child = spawn('cargo', ['test', '--locked', '-p', 'agent-harness', '-j', '2', ...args], {
      cwd: root, stdio: 'inherit', shell: false,
    });
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0 ? resolve() : reject(new Error(`SQL regressions failed: exit=${code}, signal=${signal}`)));
  });
}

async function main() {
  await cargo(['--lib', 'adapters::sql']);
  await cargo(['--lib', 'adapters::project']);
  await cargo(['--test', 'sqlserver', '--test', 'postgresql', '--test', 'postgresql_pool', '--test', 'llm_contract', '--test', 'database_setup', '--', '--test-threads=1']);
  if (process.env.HARNESS_SQL_LIVE_TESTS === '1') {
    if (!process.env.HARNESS_SQL_TEST_DSN) throw new Error('Dedicated HARNESS_SQL_TEST_DSN is required');
    await cargo(['--test', 'sqlserver', '--', '--ignored', '--test-threads=1']);
  }
  if (process.env.HARNESS_PG_LIVE_TESTS === '1') {
    for (const name of ['HARNESS_PG_TEST_DSN', 'HARNESS_PG_TEST_ADMIN_DSN', 'HARNESS_PG_TEST_LOOPBACK_DSN', 'HARNESS_PG_TEST_CA', 'HARNESS_PG_TEST_BAD_HOST_DSN', 'HARNESS_PG_TEST_POLICY_DSN']) {
      if (!process.env[name]) throw new Error(`Dedicated ${name} is required`);
    }
    await cargo(['--test', 'postgresql', '--', '--ignored', '--test-threads=1']);
  }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
