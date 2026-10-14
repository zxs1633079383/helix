// CTest runner: real localhost HTTP + real C++ callback bytes, without GUI/dependencies.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import crypto from 'node:crypto';
import http from 'node:http';
import net from 'node:net';

const CHANNEL = '01hq0000000000000000000000';
const SERVER_POST_ID = '01hq0000000000000000000001';
const FIXED_NOW_MS = 1700000000000;
const MAX_BATCH_BYTES = 1024 * 1024;
const WS_GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const WS_DEADLINE_MS = 25000;
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

function websocketAccept(key) {
  return crypto.createHash('sha1').update(`${key}${WS_GUID}`).digest('base64');
}

function websocketTextFrame(text) {
  const payload = Buffer.from(text, 'utf8');
  assert(payload.length <= 65535, 'owned WS echo fixture keeps frames bounded');
  if (payload.length < 126) return Buffer.concat([Buffer.from([0x81, payload.length]), payload]);
  const header = Buffer.from([0x81, 126, payload.length >> 8, payload.length & 0xff]);
  return Buffer.concat([header, payload]);
}

function createWebSocketFixture(mode) {
  const state = { handshake: false, handshakeFailed: false, echoSent: false, socket: null };
  let resolveConnection;
  let rejectConnection;
  const connection = new Promise((resolve, reject) => {
    resolveConnection = resolve;
    rejectConnection = reject;
  });
  // The promise is consumed by the HTTP post path; keep a rejected handshake from
  // becoming an unrelated process-level unhandled rejection in negative tests.
  connection.catch(() => {});
  const sockets = new Set();
  const server = net.createServer(socket => {
    sockets.add(socket);
    socket.setTimeout(WS_DEADLINE_MS, () => socket.destroy());
    let pending = Buffer.alloc(0);
    let upgraded = false;
    const failHandshake = error => {
      if (!upgraded && !state.handshakeFailed) {
        state.handshakeFailed = true;
        rejectConnection(error);
      }
      socket.destroy();
    };
    socket.on('error', error => failHandshake(error));
    socket.on('close', () => {
      sockets.delete(socket);
      if (!upgraded && !state.handshakeFailed) {
        failHandshake(new Error('owned WS socket closed before handshake'));
      }
    });
    socket.on('data', chunk => {
      if (upgraded) return;
      pending = Buffer.concat([pending, chunk]);
      const headerEnd = pending.indexOf('\r\n\r\n');
      if (headerEnd < 0) {
        assert(pending.length <= 65536, 'bounded WS handshake request');
        return;
      }
      const header = pending.subarray(0, headerEnd).toString('utf8');
      pending = pending.subarray(headerEnd + 4);
      const lines = header.split('\r\n');
      const [method, path] = lines.shift().split(' ');
      const headers = new Map(lines.map(line => {
        const index = line.indexOf(':');
        return [line.slice(0, index).toLowerCase(), line.slice(index + 1).trim()];
      }));
      try {
        assert.equal(method, 'GET');
        assert.equal(path, '/api/v4/websocket');
        const key = headers.get('sec-websocket-key');
        assert(key, 'WS handshake has Sec-WebSocket-Key');
        if (mode === 'ws-failure') {
          state.handshakeFailed = true;
          socket.end('HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nContent-Length: 0\r\n\r\n');
          rejectConnection(new Error('owned WS fixture rejected the handshake'));
          return;
        }
        socket.write(
          'HTTP/1.1 101 Switching Protocols\r\n' +
          'Upgrade: websocket\r\n' +
          'Connection: Upgrade\r\n' +
          `Sec-WebSocket-Accept: ${websocketAccept(key)}\r\n\r\n`,
        );
        upgraded = true;
        state.handshake = true;
        state.socket = socket;
        resolveConnection(socket);
      } catch (error) {
        failHandshake(error);
      }
    });
  });
  return {
    server,
    sockets,
    state,
    connection,
    sendEcho(temporaryId, postId) {
      assert(state.handshake && state.socket, 'WS echo requires a completed handshake');
      assert(!state.echoSent, 'owned WS fixture emits exactly one echo');
      assert.equal(typeof postId, 'string', 'WS echo uses the HTTP server post id');
      const frame = {
        v: 1,
        action: 'post',
        data: {
          id: postId,
          temporaryId,
          channelId: CHANNEL,
          event_seq: 1,
          userId: 'viewer-1',
          teamId: 'team-1',
          type: 'TEXT',
          message: 'hi',
          simpleMessage: 'hi',
          props: {},
          viewers: ['all'],
          createAt: FIXED_NOW_MS,
          updateAt: FIXED_NOW_MS,
        },
      };
      state.socket.write(websocketTextFrame(JSON.stringify(frame)));
      state.echoSent = true;
    },
  };
}

async function run(executable, mode = 'echo') {
  assert(executable, 'CTest must supply the real C++ smoke executable');
  assert(['echo', 'ws-failure', 'no-echo'].includes(mode), `unknown smoke mode: ${mode}`);
  const events = [];
  let child;
  let failure;
  let hydrating = false;
  let sent = false;
  let destroyed = false;
  let hydrated;
  let sending;
  let sentTerminal;
  let callbackStart;
  let seedRequests = 0;
  let postRequest;
  let postResponseId;
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
    if (!sending || !postRequest || !sentTerminal || done || failure) return;
    assert.equal(postRequest.channelId, CHANNEL);
    assert.equal(postRequest.userId, 'viewer-1');
    assert.equal(postRequest.teamId, 'team-1');
    for (const field of ['temporaryId', 'message', 'type', 'createAt']) {
      assert.equal(postRequest[field], sending.data[field], `actual posts/create ${field}`);
    }
    assert.equal(sentTerminal.data.temporaryId, sending.data.temporaryId);
    assert.equal(sentTerminal.data.sendStatus, 'sent');
    assert.equal(sentTerminal.data.channelId, CHANNEL);
    assert.equal(sentTerminal.data.id, postResponseId);
    assert.equal(sentTerminal.data.serverId, postResponseId);
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
      if (event.event === 'im:post:sent') {
        assert(!sentTerminal, 'exactly one durable send terminal');
        assert(sending, 'durable send terminal follows optimistic sending');
        assert.equal(event.data.sendStatus, 'sent');
        assert.equal(event.data.temporaryId, sending.data.temporaryId);
        sentTerminal = event;
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
      } else if (request.url === '/api/cses/posts/create') {
        assert.equal(request.url, '/api/cses/posts/create');
        assert(callbackStart !== undefined && !postRequest, 'one real post follows hydration');
        postRequest = body;
        reply = { status: 'SUCCESS', data: { id: SERVER_POST_ID } };
        postResponseId = reply.data.id;
      } else {
        assert.equal(request.url, '/api/cses/post/clientAck');
        assert(postRequest, 'client ACK follows the correlated posts/create admission');
        reply = { status: 'SUCCESS', data: {} };
      }
      response.writeHead(200, { 'Content-Type': 'application/json', Connection: 'close' });
      response.end(JSON.stringify(reply));
      if (request.url === '/api/cses/posts/create' && mode !== 'no-echo') {
        try {
          await wsFixture.connection;
          wsFixture.sendEcho(postRequest.temporaryId, postResponseId);
        } catch (error) {
          fail(error);
        }
      }
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
  const wsFixture = createWebSocketFixture(mode);
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  await new Promise((resolve, reject) => {
    wsFixture.server.once('error', reject);
    wsFixture.server.listen(0, '127.0.0.1', resolve);
  });

  let watchdog;
  try {
    const api = `http://127.0.0.1:${server.address().port}/api/cses`;
    const ws = `ws://127.0.0.1:${wsFixture.server.address().port}/api/v4/websocket`;
    child = spawn(executable, [api, ws], { stdio: ['pipe', 'pipe', 'pipe'] });
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
          if (failure && line !== 'DESTROYED') continue; // Keep the RAII marker while draining failure output.
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
    assert.equal(exit.signal, null);
    assert.equal(pending, '', 'C++ protocol is not truncated');
    assert(destroyed, 'C++ destroyed and joined the pump before releasing callback ctx');
    if (mode !== 'echo') {
      assert.notEqual(exit.code, 0, `${mode} must fail closed without a durable send terminal`);
      assert(postRequest, `${mode} still reaches HTTP admission before failing`);
      assert(!sentTerminal, `${mode} must not fabricate im:post:sent`);
      if (mode === 'ws-failure') {
        assert(wsFixture.state.handshakeFailed, 'WS handshake failure is observed');
        assert(!wsFixture.state.handshake, 'WS failure never becomes an active session');
      } else {
        assert(wsFixture.state.handshake, 'missing-echo path completed the WS handshake');
        assert(!wsFixture.state.echoSent, 'missing-echo path emits no inbound echo');
      }
      console.log(`PASS: expected ${mode} negative path rejects HTTP-only completion`);
      return;
    }
    assert(!failure, failure?.stack);
    assert.equal(exit.code, 0, 'C++ public calls/callback waits succeeded');
    assert(hydrated && sending && sentTerminal && sendingAck && done && postRequest && postResponseId);
    assert(wsFixture.state.handshake, 'C++ completed a real localhost WS handshake');
    assert(wsFixture.state.echoSent, 'fixture delivered the correlated inbound WS echo');
    console.log(`C++ durable hydration: ${JSON.stringify(hydrated)}`);
    console.log(`C++ post-seed typed sending callback: ${JSON.stringify(sending)}`);
    console.log(`C++ correlated real posts/create: ${JSON.stringify(postRequest)}`);
    console.log(`C++ durable send terminal: ${JSON.stringify(sentTerminal)}`);
    console.log('PASS: real C++ → cdylib → C++ callback, owned scope, HTTP/WS and durable terminal verified');
  } finally {
    clearTimeout(watchdog);
    if (child && !child.stdin.destroyed && !child.stdin.writableEnded) child.stdin.end();
    // Keep the HTTP fixture alive through destroy/drain; close only after child exit.
    await new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });
    for (const socket of wsFixture.sockets) socket.destroy();
    await new Promise(resolve => {
      if (!wsFixture.server.listening) resolve();
      else wsFixture.server.close(resolve);
    });
  }
}

run(process.argv[2], process.argv[3]).catch(error => {
  console.error(`FAIL: ${error.stack}`);
  process.exitCode = 1;
});
