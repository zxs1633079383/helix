// CTest runner: real localhost HTTP + real C++ callback bytes, without GUI/dependencies.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import http from 'node:http';

const CHANNEL = '01hq0000000000000000000000';
const FIXED_NOW_MS = 1700000000000;
const MAX_BATCH_BYTES = 1024 * 1024;
const utf8 = new TextDecoder('utf-8', { fatal: true });

function decodeBatch(hex) {
  assert.match(hex, /^(?:[0-9a-f]{2})+$/);
  const bytes = Buffer.from(hex, 'hex');
  assert(bytes.length >= 4 && bytes.length <= MAX_BATCH_BYTES);
  const count = bytes.readUInt32LE(0);
  assert(count > 0 && count <= 256, 'bounded nonempty callback batch');
  let offset = 4;
  const events = [];
  for (let i = 0; i < count; i++) {
    assert(offset + 4 <= bytes.length, 'batch length prefix is complete');
    const length = bytes.readUInt32LE(offset);
    offset += 4;
    assert(length > 0 && length <= bytes.length - offset, 'event bytes are complete');
    const event = JSON.parse(utf8.decode(bytes.subarray(offset, offset + length)));
    assert.equal(typeof event.event, 'string');
    assert(event.data && typeof event.data === 'object' && !Array.isArray(event.data));
    events.push(event);
    offset += length;
  }
  assert.equal(offset, bytes.length, 'callback has no trailing bytes');
  return events;
}

async function run(executable) {
  assert(executable, 'CTest must supply the real C++ smoke executable');
  const events = [];
  let child;
  let failure;
  let hydrating = false;
  let sent = false;
  let destroyed = false;
  let hydrated;
  let sending;
  let callbackStart;
  let seedRequests = 0;
  let postRequest;
  let sendingAck = false;
  let done = false;

  function fail(error) {
    if (!failure) failure = error instanceof Error ? error : new Error(String(error));
    if (child && !child.stdin.destroyed && !child.stdin.writableEnded) child.stdin.end('FAIL\n');
  }

  function control(line) {
    assert(!failure, failure?.message);
    child.stdin.write(`${line}\n`);
  }

  function correlatePost() {
    if (!sending || !postRequest || done || failure) return;
    assert.equal(postRequest.channelId, CHANNEL);
    assert.equal(postRequest.userId, 'viewer-1');
    assert.equal(postRequest.teamId, 'team-1');
    for (const field of ['temporaryId', 'message', 'type', 'createAt']) {
      assert.equal(postRequest[field], sending.data[field], `actual posts/create ${field}`);
    }
    done = true;
    control('DONE');
  }

  function onBatch(hex) {
    for (const event of decodeBatch(hex)) {
      const index = events.length;
      assert(index < 1024, 'bounded event recorder');
      events.push(event);
      if (event.event === 'im:read:result' && event.data.req_id === 'seed-owned-channel') {
        assert(hydrating && !hydrated, 'exactly one correlated hydration terminal');
        assert(!Object.hasOwn(event.data, 'error'), 'hydration has no error');
        const body = event.data.body;
        assert.equal(body.completion, 'hydrated');
        assert.equal(body.channelId, CHANNEL);
        assert.equal(body.channel.id, CHANNEL);
        assert.equal(body.channel.type, 'O');
        assert.equal(body.channel.teamId, 'team-1');
        assert.equal(body.channel.userId, 'viewer-1');
        assert.equal(body.member.channel_id, CHANNEL);
        assert.equal(body.member.team_id, 'team-1');
        assert.equal(body.member.user_id, 'viewer-1');
        assert.equal(body.member.role, 'MEMBER');
        assert.deepEqual(body.messages, []);
        assert.equal(body.cursor, 0);
        hydrated = event;
      }
      if (event.event === 'im:post:sending') {
        assert(sent && !sending && callbackStart !== undefined && index >= callbackStart,
          'exactly one sending callback follows hydration and the original send');
        const data = event.data;
        assert.equal(data.channelId, CHANNEL);
        assert.equal(data.userId, 'viewer-1');
        assert.equal(data.message, 'hi');
        assert.equal(data.type, 'TEXT');
        assert.equal(data.sendStatus, 'sending');
        assert.equal(data.createAt, FIXED_NOW_MS);
        assert.equal(typeof data.temporaryId, 'string');
        assert(data.temporaryId.length > 0);
        assert.notEqual(data.temporaryId, 'tmp-1', 'legacy temporary_id is not authority');
        sending = event;
      }
    }
    // Freeze the cursor after the whole hydration batch, before authorizing C++ send.
    if (hydrated && callbackStart === undefined) {
      assert.equal(seedRequests, 1, 'hydration made its real HTTP request');
      callbackStart = events.length;
      control('SEND');
    }
    if (sending && !sendingAck) {
      sendingAck = true;
      control('SENDING');
    }
    correlatePost();
  }

  const server = http.createServer(async (request, response) => {
    request.setTimeout(2000, () => request.destroy(new Error('fixture HTTP timeout')));
    try {
      const chunks = [];
      let size = 0;
      for await (const chunk of request) {
        size += chunk.length;
        assert(size <= 65536, 'bounded fixture HTTP body');
        chunks.push(chunk);
      }
      assert.equal(request.method, 'POST');
      const body = JSON.parse(Buffer.concat(chunks).toString('utf8'));
      let reply;
      if (request.url === '/api/cses/channel/load/incrementByChannelId') {
        assert.equal(++seedRequests, 1);
        assert.deepEqual(body, { channelId: CHANNEL });
        reply = { data: {
          id: CHANNEL, type: 'O', teamId: 'team-1', displayName: 'scope-hydration',
          lastEventSeq: 0, unreadCount: 0, needSync: false, memberCount: 1,
          members: [{ userId: 'viewer-1', teamId: 'team-1', role: 'MEMBER' }],
        } };
      } else {
        assert.equal(request.url, '/api/cses/posts/create');
        assert(callbackStart !== undefined && !postRequest, 'one real post follows hydration');
        postRequest = body;
        reply = { status: 'SUCCESS', data: { id: '01hq0000000000000000000001' } };
      }
      response.writeHead(200, { 'Content-Type': 'application/json', Connection: 'close' });
      response.end(JSON.stringify(reply));
      correlatePost();
    } catch (error) {
      fail(error);
      if (!response.headersSent) response.writeHead(500);
      response.end();
    }
  });
  server.requestTimeout = 2000;
  server.headersTimeout = 2000;
  server.on('error', fail);
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });

  let watchdog;
  try {
    const api = `http://127.0.0.1:${server.address().port}/api/cses`;
    child = spawn(executable, [api], { stdio: ['pipe', 'pipe', 'pipe'] });
    const exited = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
    child.on('error', fail);
    child.stdin.on('error', fail);
    child.stderr.pipe(process.stderr, { end: false });
    let pending = '';
    child.stdout.setEncoding('utf8');
    child.stdout.on('data', chunk => {
      try {
        pending += chunk;
        let end;
        while ((end = pending.indexOf('\n')) !== -1) {
          const line = pending.slice(0, end);
          pending = pending.slice(end + 1);
          if (failure) continue; // Keep draining stdout while C++ takes its RAII failure exit.
          if (line.startsWith('BATCH ')) onBatch(line.slice(6));
          else if (line === 'HYDRATING') { assert(!hydrating); hydrating = true; }
          else if (line === 'SENT') { assert(callbackStart !== undefined && !sent); sent = true; }
          else if (line === 'DESTROYED') { assert(!destroyed); destroyed = true; }
          else assert.fail(`unexpected C++ protocol line: ${line}`);
        }
        assert(pending.length <= 2 * MAX_BATCH_BYTES + 6, 'bounded C++ protocol frame');
      } catch (error) { fail(error); }
    });
    watchdog = setTimeout(() => {
      fail(new Error('C++ fixture exceeded its bounded deadline'));
    }, 25000);
    const exit = await exited;
    assert(!failure, failure?.stack);
    assert.equal(exit.signal, null);
    assert.equal(exit.code, 0, 'C++ public calls/callback waits succeeded');
    assert.equal(pending, '', 'C++ protocol is not truncated');
    assert(destroyed, 'C++ destroyed and joined the pump before releasing callback ctx');
    assert(hydrated && sending && sendingAck && done && postRequest);
    console.log(`C++ durable hydration: ${JSON.stringify(hydrated)}`);
    console.log(`C++ post-seed typed sending callback: ${JSON.stringify(sending)}`);
    console.log(`C++ correlated real posts/create: ${JSON.stringify(postRequest)}`);
    console.log('PASS: real C++ → cdylib → C++ callback, owned scope and HTTP verified');
  } finally {
    clearTimeout(watchdog);
    if (child && !child.stdin.destroyed && !child.stdin.writableEnded) child.stdin.end();
    // Keep the HTTP fixture alive through destroy/drain; close only after child exit.
    await new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });
  }
}

run(process.argv[2]).catch(error => {
  console.error(`FAIL: ${error.stack}`);
  process.exitCode = 1;
});
