const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validateVersion, validateTools } = require('./verify-package.cjs');

const tools = () => [
  { name: 'harness_write', inputSchema: { required: ['path', 'content'], properties: { createParents: {}, allowUnvalidatedText: {} } } },
  { name: 'harness_replace', inputSchema: { required: ['path'], properties: { hash: {}, chunks: {} } } },
  { name: 'harness_verify', inputSchema: { properties: { package: {} } } },
  { name: 'harness_bootstrap', inputSchema: { properties: { compact: {} } } }
];
test('requires exact package and executable version equality', () => {
  assert.doesNotThrow(() => validateVersion('agent-harness 0.8.5\r\n', '0.8.5'));
  for (const actual of ['agent-harness 0.7.0', 'agent-harness 0.8.50', 'agent-harness 0.8.5 extra']) {
    assert.throws(() => validateVersion(actual, '0.8.5'), /version mismatch/);
  }
});
test('accepts the expected native MCP contract', () => {
  assert.doesNotThrow(() => validateTools(tools()));
});
test('rejects stale tools and schemas even if executable version matches', () => {
  assert.throws(() => validateTools([]), /Missing MCP tool/);
  let stale = tools();
  stale[1].inputSchema.required.push('hash');
  assert.throws(() => validateTools(stale), /optional hash/);
  stale = tools();
  delete stale[1].inputSchema.properties.chunks;
  assert.throws(() => validateTools(stale), /optional hash/);
  stale = tools();
  delete stale[2].inputSchema.properties.package;
  assert.throws(() => validateTools(stale), /support package/);
  stale = tools();
  stale[0].inputSchema.required = ['path'];
  assert.throws(() => validateTools(stale), /path and content/);
  stale = tools();
  delete stale[0].inputSchema.properties.createParents;
  assert.throws(() => validateTools(stale), /createParents/);
  stale = tools();
  delete stale[3].inputSchema.properties.compact;
  assert.throws(() => validateTools(stale), /support compact/);
});
