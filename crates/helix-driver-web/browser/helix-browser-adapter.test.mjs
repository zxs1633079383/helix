import assert from "node:assert/strict";
import test from "node:test";

import {
  HelixBrowserAdapter,
  buildWebSocketUrl,
  createHelixBrowserAdapter,
} from "./helix-browser-adapter.mjs";

const encoder = new TextEncoder();

function webConfig(overrides = {}) {
  return {
    wsUrl: "wss://im.example.test/ws?existing=kept",
    apiBaseUrl: "https://im.example.test/api",
    defaultApiBaseUrl: "https://im.example.test/api",
    authUserId: "user-1",
    companyId: "company-must-not-enter-ws-url",
    userName: "user",
    orgName: "org",
    deptName: "dept",
    pingIntervalMs: 30_000,
    sendTimeoutMs: 5_000,
    httpTimeoutMs: 10_000,
    localStoreMode: "session",
    sessionStore: {
      maxRowsPerTable: 100,
      maxRowsPerScope: 50,
      maxScanRows: 100,
    },
    maxPendingActions: 4,
    maxPendingEventBatches: 4,
    maxEventsPerBatch: 4,
    maxRuntimeIssues: 4,
    sessionAuth: null,
    botAuth: null,
    ...overrides,
  };
}

class FakeClient {
  constructor() {
    this.calls = [];
    this.frames = [];
    this.disconnected = [];
    this.actions = [];
    this.batches = [];
    this.issues = [];
    this.nextToken = 1;
    this.currentToken = undefined;
    this.connectError = undefined;
    this.acceptFrames = true;
    this.disconnectResult = undefined;
    this.shutdownCalled = 0;
  }

  invoke(name, payload) {
    this.calls.push(["invoke", name, [...payload]]);
  }

  capabilities() {
    return encoder.encode("{}");
  }

  ws_connected() {
    this.calls.push(["connected"]);
    if (this.connectError !== undefined) {
      throw this.connectError;
    }
    const previous = this.currentToken;
    const token = this.nextToken++;
    this.currentToken = token;
    if (previous !== undefined) {
      this.actions.push({ kind: "closeSocket", token: previous });
    }
    return token;
  }

  ws_frame(token, frame) {
    this.calls.push(["frame", token]);
    this.frames.push({ token, text: new TextDecoder().decode(frame) });
    return this.acceptFrames;
  }

  ws_disconnected(token) {
    this.calls.push(["disconnected", token]);
    this.disconnected.push(token);
    if (this.disconnectResult !== undefined) {
      return this.disconnectResult;
    }
    if (this.currentToken === token) {
      this.currentToken = undefined;
      return true;
    }
    return false;
  }

  take_platform_actions() {
    return this.actions.splice(0);
  }

  take_event_batches() {
    return this.batches.splice(0);
  }

  take_runtime_issues() {
    return this.issues.splice(0);
  }

  shutdown() {
    this.calls.push(["shutdown"]);
    this.shutdownCalled += 1;
    if (this.currentToken !== undefined) {
      this.actions.push({ kind: "closeSocket", token: this.currentToken });
    }
  }
}

class FakeWebSocket {
  static instances = [];
  static constructorCalls = [];

  static reset() {
    this.instances = [];
    this.constructorCalls = [];
  }

  constructor(...args) {
    FakeWebSocket.constructorCalls.push(args);
    this.url = args[0];
    this.binaryType = undefined;
    this.sent = [];
    this.closeCalls = 0;
    this.closed = false;
    FakeWebSocket.instances.push(this);
  }

  open() {
    this.onopen?.({ target: this });
  }

  message(data) {
    this.onmessage?.({ data, target: this });
  }

  fail() {
    this.onerror?.(new Error("fake socket error"));
  }

  send(value) {
    if (this.closed) {
      throw new Error("send on closed socket");
    }
    this.sent.push(value);
  }

  close() {
    this.closeCalls += 1;
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.onclose?.({ target: this });
  }
}

class ManualScheduler {
  constructor() {
    this.nextId = 1;
    this.timeouts = new Map();
    this.intervals = new Map();
  }

  setTimeout(callback, delay) {
    const id = this.nextId++;
    this.timeouts.set(id, { callback, delay });
    return id;
  }

  clearTimeout(id) {
    this.timeouts.delete(id);
  }

  setInterval(callback, delay) {
    const id = this.nextId++;
    this.intervals.set(id, { callback, delay });
    return id;
  }

  clearInterval(id) {
    this.intervals.delete(id);
  }

  nextTimeout() {
    return this.timeouts.entries().next().value;
  }

  runTimeout(id) {
    const entry = this.timeouts.get(id);
    assert.ok(entry, `timeout ${id} exists`);
    this.timeouts.delete(id);
    entry.callback();
  }
}

class DeferredBlob {
  constructor(promise) {
    this.promise = promise;
  }

  arrayBuffer() {
    return this.promise;
  }
}

function deferred() {
  let resolve;
  const promise = new Promise((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

function makeAdapter({ client = new FakeClient(), config = webConfig(), ...overrides } = {}) {
  const scheduler = overrides.scheduler ?? new ManualScheduler();
  const eventBatches = [];
  const runtimeIssues = [];
  const adapterIssues = [];
  const adapter = new HelixBrowserAdapter({
    client,
    config,
    WebSocketImpl: FakeWebSocket,
    BlobImpl: DeferredBlob,
    scheduler,
    drainIntervalMs: 0,
    onEventBatch: (batch) => eventBatches.push(batch),
    onRuntimeIssue: (issue) => runtimeIssues.push(issue),
    onAdapterIssue: (issue) => adapterIssues.push(issue),
    ...overrides,
  });
  return { adapter, client, scheduler, eventBatches, runtimeIssues, adapterIssues };
}

async function settle() {
  await new Promise((resolve) => setImmediate(resolve));
}

async function waitFor(predicate) {
  for (let index = 0; index < 20; index += 1) {
    if (predicate()) {
      return;
    }
    await settle();
  }
  assert.fail("condition was not reached before the bounded wait expired");
}

test.beforeEach(() => FakeWebSocket.reset());

test("URLSearchParams preserves non-auth query and makes cookieId the single identity", () => {
  const url = buildWebSocketUrl(
    "wss://im.example.test/ws?existing=a%2Bb&userId=stale&cookieId=old",
    "user ?&/= 空",
  );
  const parsed = new URL(url);
  assert.equal(parsed.searchParams.get("existing"), "a+b");
  assert.equal(parsed.searchParams.get("cookieId"), "user ?&/= 空");
  assert.equal(parsed.searchParams.has("userId"), false);
  assert.equal(parsed.searchParams.has("companyId"), false);
  assert.throws(
    () => buildWebSocketUrl("wss://im.example.test/ws", "  \t"),
    /non-empty/,
  );
  assert.throws(
    () => buildWebSocketUrl("wss://im.example.test/ws", " user-1 "),
    /leading or trailing whitespace/,
  );
  assert.throws(
    () => makeAdapter({ config: webConfig({ authUserId: " user-1" }) }),
    /leading or trailing whitespace/,
  );
  assert.throws(
    () => makeAdapter({ connection: { cookieId: "user-2" } }),
    /connection.cookieId is not supported/,
  );

  const { adapter } = makeAdapter();
  adapter.start();
  assert.equal(FakeWebSocket.constructorCalls.length, 1);
  assert.equal(FakeWebSocket.constructorCalls[0].length, 1, "no protocols/header options");
  const actual = new URL(FakeWebSocket.constructorCalls[0][0]);
  assert.equal(actual.searchParams.get("existing"), "kept");
  assert.equal(actual.searchParams.get("cookieId"), "user-1");
  assert.equal(actual.searchParams.has("companyId"), false);
  adapter.shutdown();
});

test("open registers Rust first and inbound text/ArrayBuffer/Blob finish in order before close", async () => {
  const { adapter, client } = makeAdapter();
  adapter.start();
  const socket = FakeWebSocket.instances[0];
  assert.deepEqual(client.calls, []);

  client.actions.push({
    kind: "sendFrame",
    token: 1,
    transportId: 7,
    frame: encoder.encode('{"type":"hello"}'),
  });
  socket.open();
  assert.deepEqual(client.calls[0], ["connected"]);
  assert.deepEqual(socket.sent, ['{"type":"hello"}']);
  assert.equal(typeof socket.sent[0], "string", "WebSocket text opcode is selected");

  const first = deferred();
  socket.message(new DeferredBlob(first.promise));
  socket.message(encoder.encode("second").buffer);
  socket.message("third");
  socket.close();
  await settle();
  assert.deepEqual(client.frames, [], "later frames wait for the unresolved Blob");
  assert.deepEqual(client.disconnected, [], "close waits for already delivered browser messages");

  first.resolve(encoder.encode("first").buffer);
  await waitFor(() => client.frames.length === 3 && client.disconnected.length === 1);
  assert.deepEqual(client.frames, [
    { token: 1, text: "first" },
    { token: 1, text: "second" },
    { token: 1, text: "third" },
  ]);
  assert.deepEqual(client.disconnected, [1]);
  assert.deepEqual(
    client.calls.filter(([kind]) => kind === "frame" || kind === "disconnected"),
    [
      ["frame", 1],
      ["frame", 1],
      ["frame", 1],
      ["disconnected", 1],
    ],
  );
  adapter.shutdown();
});

test("a ws_connected failure immediately closes the unowned socket and retries", async () => {
  const client = new FakeClient();
  client.connectError = new Error("bridge saturated");
  const { adapter, scheduler, adapterIssues } = makeAdapter({
    client,
    reconnect: { initialDelayMs: 25, maxDelayMs: 100, stableConnectionMs: 1_000 },
  });
  adapter.start();
  const socket = FakeWebSocket.instances[0];
  socket.open();
  await settle();

  assert.equal(socket.closed, true);
  assert.equal(client.frames.length, 0);
  assert.equal(adapterIssues.some((issue) => issue.code === "ws-register"), true);
  const [timerId, timer] = scheduler.nextTimeout();
  assert.equal(timer.delay, 25);

  client.connectError = undefined;
  scheduler.runTimeout(timerId);
  assert.equal(FakeWebSocket.instances.length, 2);
  adapter.shutdown();
});

test("replacement closes stale token, drops late frames, and close publishes at most one disconnect", async () => {
  const { adapter, client } = makeAdapter();
  adapter.start();
  const oldSocket = FakeWebSocket.instances[0];
  oldSocket.open();

  adapter.reconnectNow();
  const currentSocket = FakeWebSocket.instances[1];
  currentSocket.open();
  await settle();
  assert.equal(oldSocket.closed, true, "Rust closeSocket action owns replacement close");
  assert.deepEqual(client.disconnected, [1]);

  oldSocket.message("late-old-frame");
  oldSocket.close();
  await settle();
  assert.equal(client.frames.some((frame) => frame.text === "late-old-frame"), false);
  assert.deepEqual(client.disconnected, [1], "duplicate close cannot publish twice");

  currentSocket.message("current-frame");
  await settle();
  assert.deepEqual(client.frames, [{ token: 2, text: "current-frame" }]);
  adapter.shutdown();
});

test("reconnect backoff is capped and shutdown cancels pending reconnects", async () => {
  const { adapter, scheduler, client } = makeAdapter({
    reconnect: {
      initialDelayMs: 100,
      maxDelayMs: 250,
      multiplier: 2,
      stableConnectionMs: 5_000,
    },
  });
  adapter.start();
  const first = FakeWebSocket.instances[0];
  first.open();
  first.close();
  await settle();

  let [timerId, timer] = scheduler.nextTimeout();
  assert.equal(timer.delay, 100);
  scheduler.runTimeout(timerId);
  const second = FakeWebSocket.instances[1];
  second.close();
  await settle();

  [timerId, timer] = scheduler.nextTimeout();
  assert.equal(timer.delay, 200);
  scheduler.runTimeout(timerId);
  const third = FakeWebSocket.instances[2];
  third.close();
  await settle();

  [timerId, timer] = scheduler.nextTimeout();
  assert.equal(timer.delay, 250, "delay is bounded by maxDelayMs");
  adapter.shutdown();
  assert.equal(scheduler.timeouts.has(timerId), false);
  assert.equal(client.shutdownCalled, 1);
  const socketsBefore = FakeWebSocket.instances.length;
  await settle();
  assert.equal(FakeWebSocket.instances.length, socketsBefore, "shutdown never reconnects");
  adapter.shutdown();
  assert.equal(client.shutdownCalled, 1, "shutdown is idempotent");
});

test("Rust rejection of the active disconnect suppresses browser reconnect", async () => {
  const client = new FakeClient();
  client.disconnectResult = false;
  const { adapter, scheduler } = makeAdapter({ client });
  adapter.start();
  const socket = FakeWebSocket.instances[0];
  socket.open();
  socket.close();
  await settle();

  assert.deepEqual(client.disconnected, [1]);
  assert.equal(scheduler.nextTimeout(), undefined);
  adapter.shutdown();
});

test("bounded drains reject oversized output and unsafe drop counters", () => {
  const config = webConfig({
    maxPendingActions: 1,
    maxPendingEventBatches: 1,
    maxEventsPerBatch: 1,
    maxRuntimeIssues: 1,
  });
  const { adapter, client, eventBatches, runtimeIssues, adapterIssues } = makeAdapter({
    config,
  });
  adapter.start();
  FakeWebSocket.instances[0].open();

  client.actions.push(
    { kind: "closeSocket", token: 999 },
    { kind: "closeSocket", token: 998 },
  );
  adapter.drain();
  assert.equal(adapterIssues.some((issue) => issue.code === "action-drain"), true);

  client.batches.push({
    version: 1,
    droppedBatchesBefore: 0,
    droppedEvents: Number.MAX_SAFE_INTEGER + 1,
    events: [encoder.encode("event")],
  });
  adapter.drain();
  assert.equal(eventBatches.length, 0);
  assert.equal(adapterIssues.some((issue) => issue.code === "event-publish"), true);

  client.batches.push({
    version: 2,
    droppedBatchesBefore: 3,
    droppedEvents: 4,
    events: [encoder.encode("valid")],
  });
  client.issues.push("runtime-one", "1 additional Web runtime issues were dropped at capacity");
  adapter.drain();
  assert.equal(eventBatches.length, 1);
  assert.equal(eventBatches[0].droppedBatchesBefore, 3);
  assert.equal(eventBatches[0].droppedEvents, 4);
  assert.deepEqual(runtimeIssues, [
    "runtime-one",
    "1 additional Web runtime issues were dropped at capacity",
  ]);

  client.issues.push("too", "many", "issues");
  adapter.drain();
  assert.equal(
    adapterIssues.some((issue) => issue.code === "runtime-issue-drain"),
    true,
  );

  client.batches.push({
    version: 1,
    droppedBatchesBefore: 0,
    droppedEvents: 0,
    events: [encoder.encode("old-version")],
  });
  adapter.drain();
  assert.equal(eventBatches.length, 1, "unsupported batch versions fail closed");
  assert.equal(adapterIssues.some((issue) => issue.code === "event-publish"), true);
  adapter.shutdown();
});

test("factory initializes wasm before construction and passes the untouched WebConfig shape", async () => {
  const order = [];
  class ConstructedClient extends FakeClient {
    constructor(configJson) {
      super();
      order.push("construct");
      this.config = JSON.parse(configJson);
    }
  }
  const config = webConfig();
  const adapter = await createHelixBrowserAdapter({
    initWasm: async (input) => {
      assert.equal(input, "fixture.wasm");
      order.push("init");
    },
    wasmInput: "fixture.wasm",
    HelixWebClient: ConstructedClient,
    config,
    autoStart: false,
    WebSocketImpl: FakeWebSocket,
    BlobImpl: DeferredBlob,
    scheduler: new ManualScheduler(),
    drainIntervalMs: 0,
    onEventBatch: () => {},
  });
  assert.deepEqual(order, ["init", "construct"]);
  assert.equal(adapter.websocketUrl.includes("companyId"), false);
  adapter.shutdown();
});
