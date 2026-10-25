'use strict';
// Development-only differential checks against the existing implementation.
// The distributed Rust executable does not load Node or this test.
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const {createRequire} = require('node:module');
const {randomUUID, createHash} = require('node:crypto');
// Support both the original native/core layout and a standalone Rust checkout.
// Resolve the SDK from the reference implementation too; this checkout needs no
// package.json or node_modules. This is a development-only comparison harness.
const nodeRoot = path.resolve(process.env.HARNESS_NODE_ROOT || path.resolve(__dirname, '../../..'));
if (!fs.existsSync(path.join(nodeRoot, 'persistence.cjs'))) {
  throw new Error('Set HARNESS_NODE_ROOT to the original .agents/harness directory before running compatibility tests');
}
const nodeRequire = createRequire(path.join(nodeRoot, 'package.json'));
const {Client} = nodeRequire('@modelcontextprotocol/sdk/client/index.js');
const {StdioClientTransport} = nodeRequire('@modelcontextprotocol/sdk/client/stdio.js');
const {SessionDisk} = nodeRequire('./persistence.cjs');
const {Sessions} = nodeRequire('./session.cjs');
const {ResultStore} = nodeRequire('./results.cjs');
const {TaskBoard} = nodeRequire('./tasks.cjs');

test('Rust and Node share session v1, read hashes and UTF-16 detail pages', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-rust-'));
  const binary = process.env.HARNESS_RUST_BINARY || path.resolve(__dirname, '../target/debug', process.platform === 'win32' ? 'agent-harness.exe' : 'agent-harness');
  const client = new Client({name:'node-compatibility-test', version:'1'});
  const transport = new StdioClientTransport({command:binary, args:['--root', root], env:{PATH:''}});
  const disk = new SessionDisk(root), details = new ResultStore();
  try {
    fs.mkdirSync(path.join(root, '.agents/rules'), {recursive:true});
    const rules = '😀'.repeat(6001);
    fs.writeFileSync(path.join(root, '.agents/rules/00-mandatory-tools.md'), rules);
    fs.writeFileSync(path.join(root, 'example.txt'), 'hola\r\nRust\r\n');
    await client.connect(transport);
    const bootstrap = await client.callTool({name:'harness_bootstrap', arguments:{}});
    assert.equal(bootstrap.isError, false);
    const sessionId = bootstrap.structuredContent.data.sessionId;
    const restored = new Sessions({disk}).get(sessionId);
    assert.equal(restored.restored, true);
    assert.equal(restored.declared.goal, '');
    assert.equal(restored.tasks.summary().total, 4);

    const nodeSession = randomUUID();
    disk.save(nodeSession, {
      declared:{goal:'continuar migración', decisions:['Rust'], pending:[]},
      phases:restored.tasks.summary().phases,
      edits:[], checks:[{language:'rust', scope:'core', time:'2026-10-16T00:00:00Z', ok:true}]
    });
    const resume = await client.callTool({name:'harness_bootstrap', arguments:{session:nodeSession}});
    assert.equal(resume.isError, false);
    assert.equal(resume.structuredContent.data.sessionId, nodeSession);

    const read = await client.callTool({name:'harness_read', arguments:{session:nodeSession, path:'example.txt', start:1, end:2}});
    assert.equal(read.isError, false);
    assert.deepEqual(read.structuredContent.scope, {tool:'harness_read', path:'example.txt'});
    assert.equal(read.structuredContent.data.hash, undefined);
    assert.equal(read.structuredContent.data.anchors, undefined);
    assert.deepEqual(read.structuredContent.data.range, [1, 2]);
    assert.equal(read.structuredContent.data.text, 'hola\r\nRust');
    assert.deepEqual(JSON.parse(read.content[0].text), read.structuredContent);

    if (process.platform === 'win32') {
      const before = Buffer.from('🙂\r\na\r\nb\r\n');
      const expected = Buffer.from('🙂\r\nc\r\nd\r\n');
      fs.writeFileSync(path.join(root, 'edit.txt'), before);
      const args = {session:nodeSession, path:'edit.txt', hash:createHash('sha256').update(before).digest('hex'), oldText:'a\nb', newText:'c\nd'};
      const edit = await client.callTool({name:'harness_replace', arguments:args});
      assert.equal(edit.isError, false);
      assert.equal(edit.structuredContent.data.changed, true);
      assert.equal(edit.structuredContent.data.verified, true);
      assert.deepEqual(fs.readFileSync(path.join(root, 'edit.txt')), expected);
      assert.equal(edit.structuredContent.data.hash, createHash('sha256').update(expected).digest('hex'));
      const retry = await client.callTool({name:'harness_replace', arguments:args});
      assert.equal(retry.isError, true);
      assert.equal(retry.structuredContent.diagnostics[0].code, 'STALE');
    }

    const expectedRef = details.save({rules});
    const rustBody = 'pub fn café() -> &\'static str { "🙂" }';
    const rustSource = '\uFEFF// 🙂 UTF-16 offsets\r\n' + rustBody + '\r\n';
    fs.writeFileSync(path.join(root, 'sample.rs'), rustSource);
    const rustHash = createHash('sha256').update(rustSource).digest('hex');
    const start = rustSource.indexOf(rustBody), end = start + rustBody.length;
    const symbolId = createHash('sha256').update(`${rustHash}:${start}:${end}`).digest('hex').slice(0,24);
    const outline = await client.callTool({name:'harness_outline', arguments:{path:'sample.rs'}});
    assert.equal(outline.isError, false);
    assert.equal(outline.structuredContent.data.items[0].symbolId, symbolId);
    const symbol = await client.callTool({name:'harness_read_symbol', arguments:{path:'sample.rs', symbol:'café', symbolId}});
    assert.equal(symbol.isError, false);
    assert.equal(symbol.structuredContent.data.text, rustBody);
    assert.equal(symbol.structuredContent.data.hash, rustHash);

    const actualRef = bootstrap.structuredContent.detail;
    assert.equal(actualRef.characters, expectedRef.characters);
    let offset = 0;
    do {
      const expected = details.get(expectedRef.ref, offset, 137);
      const actual = await client.callTool({name:'harness_detail', arguments:{ref:actualRef.ref, offset, maxChars:137}});
      assert.equal(actual.isError, false);
      assert.deepEqual(actual.structuredContent.data, {...expected, ref:actualRef.ref});
      offset = expected.nextOffset;
    } while(offset !== null);
  } finally {
    await client.close();
    details.close();
    // root was created by this test and is a known, isolated temporary directory.
    fs.rmSync(root, {recursive:true, force:true});
  }
});

test('Rust task transitions match Node and checkpoint updates remain session-v1 compatible', async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-workflow-'));
  const binary = process.env.HARNESS_RUST_BINARY || path.resolve(__dirname, '../target/debug', process.platform === 'win32' ? 'agent-harness.exe' : 'agent-harness');
  const client = new Client({name:'workflow-compatibility-test',version:'1'});
  const transport = new StdioClientTransport({command:binary,args:['--root',root],env:{PATH:''}});
  const board = new TaskBoard({clock:()=>0});
  try {
    await client.connect(transport);
    const bootstrap = await client.callTool({name:'harness_bootstrap',arguments:{}});
    const session = bootstrap.structuredContent.data.sessionId;
    for (const [phase,status,note] of [
      ['analysis','in_progress',''],['analysis','blocked','esperando'],
      ['implementation','in_progress',''],['implementation','completed',''],
      ['review','completed','revisado'],['review','pending','otra revisión']
    ]) {
      const expected = board.update(phase,status,note);
      const actual = await client.callTool({name:'harness_tasks',arguments:{session,action:'update',phase,status,note}});
      assert.equal(actual.isError,false);
      const {persisted,...summary} = actual.structuredContent.data;
      assert.equal(persisted,true);
      assert.deepEqual(summary,expected);
    }
    assert.throws(()=>board.update('analysis','blocked','\uFEFF'),error=>error.code==='TASK');
    const invalid = await client.callTool({name:'harness_tasks',arguments:{session,action:'update',phase:'analysis',status:'blocked',note:'\uFEFF'}});
    assert.equal(invalid.structuredContent.diagnostics[0].code,'TASK');
    const actual = await client.callTool({name:'harness_tasks',arguments:{session,action:'timeline',limit:20}});
    const stripTime = timeline=>({...timeline,events:timeline.events.map(({time,...event})=>event)});
    assert.deepEqual(stripTime(actual.structuredContent.data),stripTime(board.timeline(0,20)));
    const checkpoint = await client.callTool({name:'harness_checkpoint',arguments:{session,goal:'migrar Rust',decisions:['sin Node'],pending:['vigencia']}});
    assert.equal(checkpoint.isError,false);
    assert.equal(checkpoint.structuredContent.data.freshness.available,false);
    const disk = new SessionDisk(root);
    assert.deepEqual(disk.load(session).declared,{goal:'migrar Rust',decisions:['sin Node'],pending:['vigencia']});
    const restored = new Sessions({disk}).get(session);
    assert.equal(restored.declared.goal,'migrar Rust');
    assert.deepEqual(restored.tasks.summary().phases,board.summary().phases);
  } finally {
    await client.close();
    fs.rmSync(root,{recursive:true,force:true});
  }
});
