#!/usr/bin/env node
const { spawn } = require('child_process');
const path = require('path');

const exePath = path.join(__dirname, 'agent-harness.exe');

const child = spawn(exePath, process.argv.slice(2), {
  stdio: 'inherit',
  windowsHide: true
});

child.on('error', (err) => {
  console.error('Failed to start agent-harness:', err);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 0);
  }
});
