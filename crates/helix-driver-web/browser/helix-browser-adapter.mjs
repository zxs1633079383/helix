const DEFAULT_RECONNECT = Object.freeze({
  initialDelayMs: 250,
  maxDelayMs: 10_000,
  multiplier: 2,
  stableConnectionMs: 10_000,
});

const DEFAULT_MAX_DRAIN_PASSES = 8;
const DEFAULT_MAX_ADAPTER_ISSUES = 32;

/**
 * Append browser WebSocket authentication without exposing it as a custom
 * handshake header. Existing query parameters are preserved.
 */
export function buildWebSocketUrl(rawUrl, cookieId, baseUrl = globalThis.location?.href) {
  if (typeof rawUrl !== "string" || rawUrl.length === 0) {
    throw new TypeError("config.wsUrl must be a non-empty string");
  }
  if (typeof cookieId !== "string" || cookieId.trim().length === 0) {
    throw new TypeError("WebSocket cookieId must be a non-empty string");
  }
  if (cookieId !== cookieId.trim()) {
    throw new TypeError("WebSocket cookieId must not contain leading or trailing whitespace");
  }

  const url = baseUrl === undefined ? new URL(rawUrl) : new URL(rawUrl, baseUrl);
  if (url.protocol !== "ws:" && url.protocol !== "wss:") {
    throw new TypeError("config.wsUrl must use ws: or wss:");
  }
  // Go resolves query userId before cookieId. Remove a stale userId from a
  // configured base URL so authUserId remains the single browser identity.
  url.searchParams.delete("userId");
  url.searchParams.set("cookieId", cookieId);
  return url.toString();
}

/**
 * Construct the generated wasm-bindgen client and connect the browser-owned
 * physical WebSocket. The generated module can be wired directly:
 *
 *   createHelixBrowserAdapter({ initWasm: init, HelixWebClient, config, ... })
 */
export async function createHelixBrowserAdapter({
  initWasm,
  wasmInput,
  HelixWebClient,
  config,
  autoStart = true,
  ...adapterOptions
}) {
  const parsedConfig = parseConfig(config);
  const connection = parseConnection(adapterOptions.connection);
  // Validate browser-only connection input before allocating the Rust client.
  buildWebSocketUrl(
    connection.wsUrl ?? parsedConfig.wsUrl,
    parsedConfig.authUserId,
    connection.baseUrl,
  );

  if (initWasm !== undefined) {
    if (typeof initWasm !== "function") {
      throw new TypeError("initWasm must be a function when supplied");
    }
    await initWasm(wasmInput);
  }
  if (typeof HelixWebClient !== "function") {
    throw new TypeError("HelixWebClient must be the wasm-bindgen constructor");
  }

  const client = new HelixWebClient(JSON.stringify(parsedConfig));
  try {
    const adapter = new HelixBrowserAdapter({
      ...adapterOptions,
      client,
      config: parsedConfig,
    });
    if (autoStart) {
      adapter.start();
    }
    return adapter;
  } catch (error) {
    try {
      client.shutdown();
    } catch {
      // Preserve the construction failure; shutdown is best-effort here.
    }
    throw error;
  }
}

/**
 * Browser infrastructure adapter only. It owns physical socket lifecycle and
 * forwards opaque bytes; all IM parsing/query/sync/cursor behavior stays Rust.
 */
export class HelixBrowserAdapter {
  constructor({
    client,
    config,
    connection = {},
    onEventBatch,
    onRuntimeIssue,
    onAdapterIssue,
    WebSocketImpl = globalThis.WebSocket,
    BlobImpl = globalThis.Blob,
    TextEncoderImpl = globalThis.TextEncoder,
    TextDecoderImpl = globalThis.TextDecoder,
    scheduler = browserScheduler(),
    drainIntervalMs = 16,
    reconnect = {},
    maxDrainPasses = DEFAULT_MAX_DRAIN_PASSES,
    maxAdapterIssues = DEFAULT_MAX_ADAPTER_ISSUES,
  }) {
    this._client = requireClient(client);
    this._config = parseConfig(config);
    const parsedConnection = parseConnection(connection);
    this._limits = readLimits(this._config);
    this._wsUrl = buildWebSocketUrl(
      parsedConnection.wsUrl ?? this._config.wsUrl,
      this._config.authUserId,
      parsedConnection.baseUrl,
    );

    if (typeof WebSocketImpl !== "function") {
      throw new TypeError("WebSocket is unavailable in this browser context");
    }
    if (typeof TextEncoderImpl !== "function" || typeof TextDecoderImpl !== "function") {
      throw new TypeError("TextEncoder and TextDecoder are required");
    }
    if (typeof onEventBatch !== "function") {
      throw new TypeError("onEventBatch is required so DomainEvent batches are not discarded");
    }

    this._WebSocket = WebSocketImpl;
    this._Blob = BlobImpl;
    this._encoder = new TextEncoderImpl();
    this._decoder = new TextDecoderImpl("utf-8", { fatal: true });
    this._scheduler = requireScheduler(scheduler);
    this._onEventBatch = onEventBatch;
    this._onRuntimeIssue = optionalFunction(onRuntimeIssue, "onRuntimeIssue");
    this._onAdapterIssue = optionalFunction(onAdapterIssue, "onAdapterIssue");
    this._drainIntervalMs = nonNegativeSafeInteger(drainIntervalMs, "drainIntervalMs");
    this._reconnect = normalizeReconnect(reconnect);
    this._maxDrainPasses = positiveSafeInteger(maxDrainPasses, "maxDrainPasses");
    this._adapterIssueQueue = new BoundedIssueQueue(
      positiveSafeInteger(maxAdapterIssues, "maxAdapterIssues"),
    );

    this._started = false;
    this._shutdown = false;
    this._draining = false;
    this._drainAgain = false;
    this._generation = 0;
    this._opening = undefined;
    this._active = undefined;
    this._recordsByToken = new Map();
    this._reconnectAttempt = 0;
    this._reconnectTimer = undefined;
    this._drainTimer = undefined;
  }

  get websocketUrl() {
    return this._wsUrl;
  }

  get isShutdown() {
    return this._shutdown;
  }

  start() {
    if (this._shutdown) {
      throw new Error("Helix browser adapter is shut down");
    }
    if (this._started) {
      return;
    }
    this._started = true;
    if (this._drainIntervalMs > 0) {
      this._drainTimer = this._scheduler.setInterval(
        () => this.drain(),
        this._drainIntervalMs,
      );
    }
    this._openSocket();
    this.drain();
  }

  /** Force a new physical generation; Rust decides which old token to close. */
  reconnectNow() {
    if (this._shutdown) {
      return false;
    }
    if (!this._started) {
      this.start();
      return true;
    }
    this._clearReconnectTimer();
    this._openSocket();
    return true;
  }

  invoke(publicName, payload) {
    if (this._shutdown) {
      throw new Error("Helix browser adapter is shut down");
    }
    const bytes = asUint8Array(payload, "invoke payload");
    this._client.invoke(publicName, bytes);
    this.drain();
  }

  capabilities() {
    return asUint8Array(this._client.capabilities(), "capabilities");
  }

  /**
   * Drain bounded Rust queues. Re-entrant calls request another pass; an
   * explicit pass limit prevents a consumer callback from creating an
   * unbounded synchronous loop.
   */
  drain() {
    if (this._shutdown && !this._draining) {
      // shutdown() performs its own final action drain.
      return;
    }
    if (this._draining) {
      this._drainAgain = true;
      return;
    }

    this._draining = true;
    try {
      let pass = 0;
      do {
        this._drainAgain = false;
        pass += 1;
        this._drainPlatformActions();
        this._drainEventBatches();
        this._drainRuntimeIssues();
      } while (this._drainAgain && pass < this._maxDrainPasses);
    } finally {
      this._draining = false;
    }
  }

  takeAdapterIssues() {
    return this._adapterIssueQueue.drain();
  }

  shutdown() {
    if (this._shutdown) {
      return;
    }
    this._shutdown = true;
    this._clearReconnectTimer();
    if (this._drainTimer !== undefined) {
      this._scheduler.clearInterval(this._drainTimer);
      this._drainTimer = undefined;
    }

    try {
      this._client.shutdown();
    } catch (error) {
      this._report("rust-shutdown", error);
    }

    // Rust shutdown emits the authoritative close action. Drain only actions:
    // the public drain intentionally becomes a no-op after the stopped bit.
    this._drainPlatformActions();
    for (const record of this._allRecords()) {
      this._clearStableTimer(record);
      safeClose(record.socket);
    }
    this._opening = undefined;
    this._active = undefined;
  }

  _openSocket() {
    if (this._shutdown) {
      return;
    }

    if (this._opening !== undefined) {
      const superseded = this._opening;
      // Clear ownership before close so a synchronous close callback cannot
      // schedule a second reconnect alongside the replacement being created.
      this._opening = undefined;
      safeClose(superseded.socket);
    }

    let socket;
    try {
      // Authentication is encoded in URLSearchParams. No WebSocket protocols
      // or non-standard header options are passed to the browser constructor.
      socket = new this._WebSocket(this._wsUrl);
    } catch (error) {
      this._report("ws-constructor", error);
      this._scheduleReconnect();
      return;
    }

    const record = {
      generation: ++this._generation,
      socket,
      token: undefined,
      inboundTail: Promise.resolve(),
      closeStarted: false,
      disconnectNotified: false,
      stableTimer: undefined,
    };
    this._opening = record;
    socket.binaryType = "arraybuffer";
    socket.onopen = () => this._onOpen(record);
    socket.onmessage = (event) => this._onMessage(record, event?.data);
    socket.onerror = (event) => this._onError(record, event);
    socket.onclose = () => this._onClose(record);
  }

  _onOpen(record) {
    if (
      this._shutdown ||
      record.closeStarted ||
      this._opening !== record ||
      record.generation !== this._generation
    ) {
      safeClose(record.socket);
      return;
    }

    let token;
    try {
      // Registration is synchronous. Browser message delivery is not accepted
      // until this call has published Connected into the Rust engine.
      token = positiveSafeInteger(this._client.ws_connected(), "WebSocket token");
      if (this._recordsByToken.has(token)) {
        throw new Error(`duplicate WebSocket token ${token}`);
      }
    } catch (error) {
      this._opening = undefined;
      this._report("ws-register", error);
      safeClose(record.socket);
      this._scheduleReconnect();
      return;
    }

    record.token = token;
    this._recordsByToken.set(token, record);
    this._opening = undefined;
    this._active = record;
    record.stableTimer = this._scheduler.setTimeout(() => {
      if (!this._shutdown && this._active === record && !record.closeStarted) {
        this._reconnectAttempt = 0;
      }
    }, this._reconnect.stableConnectionMs);
    this.drain();
  }

  _onMessage(record, data) {
    if (
      this._shutdown ||
      record.closeStarted ||
      record.token === undefined ||
      this._recordsByToken.get(record.token) !== record
    ) {
      safeClose(record.socket);
      return;
    }

    record.inboundTail = record.inboundTail
      .then(() => messageBytes(data, this._encoder, this._Blob))
      .then((bytes) => {
        if (
          this._shutdown ||
          this._recordsByToken.get(record.token) !== record
        ) {
          return;
        }
        const accepted = this._client.ws_frame(record.token, bytes);
        if (accepted !== true) {
          // The Rust generation is authoritative. A stale browser socket must
          // not continue delivering frames after replacement.
          safeClose(record.socket);
          return;
        }
        this.drain();
      })
      .catch((error) => {
        this._report("ws-frame", error, { token: record.token });
        safeClose(record.socket);
      });
  }

  _onError(record, event) {
    if (this._shutdown || record.closeStarted) {
      return;
    }
    this._report("ws-error", event instanceof Error ? event : new Error("WebSocket error"), {
      token: record.token,
    });
    // `close` remains the single place that publishes Disconnected.
    safeClose(record.socket);
  }

  _onClose(record) {
    if (record.closeStarted) {
      return;
    }
    record.closeStarted = true;
    this._clearStableTimer(record);

    const wasOpening = this._opening === record;
    const wasActive = this._active === record;
    if (wasOpening) {
      this._opening = undefined;
    }
    if (wasActive) {
      this._active = undefined;
    }

    record.inboundTail.then(() => {
      const token = record.token;
      let rustAcceptedDisconnect = false;
      if (token !== undefined && this._recordsByToken.get(token) === record) {
        this._recordsByToken.delete(token);
        if (!this._shutdown && !record.disconnectNotified) {
          record.disconnectNotified = true;
          try {
            rustAcceptedDisconnect = this._client.ws_disconnected(token) === true;
            this.drain();
          } catch (error) {
            this._report("ws-disconnect", error, { token });
          }
        }
      }

      const shouldReconnect = wasOpening || (wasActive && rustAcceptedDisconnect);
      if (!this._shutdown && shouldReconnect && this._active === undefined) {
        this._scheduleReconnect();
      }
    });
  }

  _scheduleReconnect() {
    if (
      this._shutdown ||
      this._reconnectTimer !== undefined ||
      this._opening !== undefined ||
      this._active !== undefined
    ) {
      return;
    }

    const exponent = Math.min(this._reconnectAttempt, 31);
    const scaledDelay = this._reconnect.initialDelayMs === 0
      ? 0
      : this._reconnect.initialDelayMs * this._reconnect.multiplier ** exponent;
    const delay = Number.isFinite(scaledDelay)
      ? Math.min(this._reconnect.maxDelayMs, scaledDelay)
      : this._reconnect.maxDelayMs;
    this._reconnectAttempt = Math.min(this._reconnectAttempt + 1, 32);
    this._reconnectTimer = this._scheduler.setTimeout(() => {
      this._reconnectTimer = undefined;
      this._openSocket();
    }, delay);
  }

  _clearReconnectTimer() {
    if (this._reconnectTimer !== undefined) {
      this._scheduler.clearTimeout(this._reconnectTimer);
      this._reconnectTimer = undefined;
    }
  }

  _clearStableTimer(record) {
    if (record.stableTimer !== undefined) {
      this._scheduler.clearTimeout(record.stableTimer);
      record.stableTimer = undefined;
    }
  }

  _drainPlatformActions() {
    let actions;
    try {
      actions = boundedArray(
        this._client.take_platform_actions(),
        this._limits.maxDrainedPlatformActions,
        "platform actions",
      );
    } catch (error) {
      this._report("action-drain", error);
      return;
    }

    for (const action of actions) {
      try {
        if (action?.kind === "sendFrame") {
          const token = positiveSafeInteger(action.token, "sendFrame.token");
          nonNegativeSafeInteger(action.transportId, "sendFrame.transportId");
          const record = this._recordsByToken.get(token);
          if (record === undefined) {
            continue;
          }
          if (record !== this._active || record.closeStarted) {
            safeClose(record.socket);
            continue;
          }
          const text = this._decoder.decode(asUint8Array(action.frame, "sendFrame.frame"));
          record.socket.send(text);
        } else if (action?.kind === "closeSocket") {
          const token = positiveSafeInteger(action.token, "closeSocket.token");
          const record = this._recordsByToken.get(token);
          if (record !== undefined) {
            safeClose(record.socket);
          }
        } else {
          throw new TypeError(`unknown platform action ${String(action?.kind)}`);
        }
      } catch (error) {
        this._report("action-apply", error, { kind: action?.kind, token: action?.token });
        if (Number.isSafeInteger(action?.token)) {
          const record = this._recordsByToken.get(action.token);
          if (record !== undefined) {
            safeClose(record.socket);
          }
        }
      }
    }
  }

  _drainEventBatches() {
    let batches;
    try {
      batches = boundedArray(
        this._client.take_event_batches(),
        this._limits.maxPendingEventBatches,
        "event batches",
      );
    } catch (error) {
      this._report("event-drain", error);
      return;
    }

    for (const batch of batches) {
      try {
        const events = boundedArray(
          batch?.events,
          this._limits.maxEventsPerBatch,
          "events in batch",
        ).map((event) => asUint8Array(event, "DomainEvent"));
        const version = nonNegativeSafeInteger(batch?.version, "event batch version");
        if (version !== 2) {
          throw new RangeError(`unsupported event batch version ${version}`);
        }
        const normalized = Object.freeze({
          version,
          droppedBatchesBefore: nonNegativeSafeInteger(
            batch?.droppedBatchesBefore,
            "event batch droppedBatchesBefore",
          ),
          droppedEvents: nonNegativeSafeInteger(
            batch?.droppedEvents,
            "event batch droppedEvents",
          ),
          events,
        });
        this._onEventBatch(normalized);
      } catch (error) {
        this._report("event-publish", error);
      }
    }
  }

  _drainRuntimeIssues() {
    let issues;
    try {
      // Rust may append one explicit overflow summary after its bounded queue.
      issues = boundedArray(
        this._client.take_runtime_issues(),
        this._limits.maxRuntimeIssues + 1,
        "runtime issues",
      );
    } catch (error) {
      this._report("runtime-issue-drain", error);
      return;
    }

    for (const issue of issues) {
      if (typeof issue !== "string") {
        this._report("runtime-issue-shape", new TypeError("runtime issue must be a string"));
        continue;
      }
      if (this._onRuntimeIssue !== undefined) {
        try {
          this._onRuntimeIssue(issue);
        } catch (error) {
          this._report("runtime-issue-publish", error);
        }
      }
    }
  }

  _allRecords() {
    const records = new Set(this._recordsByToken.values());
    if (this._opening !== undefined) {
      records.add(this._opening);
    }
    return records;
  }

  _report(code, error, context = {}) {
    const issue = Object.freeze({
      code,
      message: errorMessage(error),
      context: Object.freeze({ ...context }),
    });
    this._adapterIssueQueue.push(issue);
    if (this._onAdapterIssue !== undefined) {
      try {
        this._onAdapterIssue(issue);
      } catch {
        // The bounded queue remains the non-recursive diagnostic fallback.
      }
    }
  }
}

class BoundedIssueQueue {
  constructor(capacity) {
    this.capacity = capacity;
    this.entries = [];
    this.dropped = 0;
  }

  push(issue) {
    if (this.entries.length < this.capacity) {
      this.entries.push(issue);
    } else {
      this.dropped = Math.min(Number.MAX_SAFE_INTEGER, this.dropped + 1);
    }
  }

  drain() {
    const entries = this.entries.splice(0);
    if (this.dropped > 0) {
      entries.push(
        Object.freeze({
          code: "adapter-issue-overflow",
          message: `${this.dropped} additional adapter issues were dropped at capacity`,
          context: Object.freeze({ dropped: this.dropped }),
        }),
      );
      this.dropped = 0;
    }
    return entries;
  }
}

function parseConfig(config) {
  let parsed = config;
  if (typeof config === "string") {
    parsed = JSON.parse(config);
  }
  if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new TypeError("config must be a WebConfig object or JSON string");
  }
  return parsed;
}

function parseConnection(connection) {
  if (connection === undefined) {
    return {};
  }
  if (connection === null || typeof connection !== "object" || Array.isArray(connection)) {
    throw new TypeError("connection must be an object when supplied");
  }
  if (Object.prototype.hasOwnProperty.call(connection, "cookieId")) {
    throw new TypeError(
      "connection.cookieId is not supported; WebSocket identity comes from config.authUserId",
    );
  }
  return connection;
}

function readLimits(config) {
  const maxPendingActions = positiveSafeInteger(
    config.maxPendingActions,
    "maxPendingActions",
  );
  if (maxPendingActions > (Number.MAX_SAFE_INTEGER + 1) / 2) {
    throw new RangeError("maxPendingActions is too large to derive a safe drain bound");
  }
  return Object.freeze({
    // ExternalWsBridge bounds data and control lanes independently. At most
    // `maxPendingActions - 1` replacement closes can accompany the bounded
    // data lane in one drain.
    maxDrainedPlatformActions: maxPendingActions * 2 - 1,
    maxPendingEventBatches: positiveSafeInteger(
      config.maxPendingEventBatches,
      "maxPendingEventBatches",
    ),
    maxEventsPerBatch: positiveSafeInteger(config.maxEventsPerBatch, "maxEventsPerBatch"),
    maxRuntimeIssues: positiveSafeInteger(config.maxRuntimeIssues, "maxRuntimeIssues"),
  });
}

function requireClient(client) {
  const methods = [
    "invoke",
    "capabilities",
    "ws_connected",
    "ws_frame",
    "ws_disconnected",
    "take_platform_actions",
    "take_event_batches",
    "take_runtime_issues",
    "shutdown",
  ];
  for (const method of methods) {
    if (typeof client?.[method] !== "function") {
      throw new TypeError(`HelixWebClient.${method} is required`);
    }
  }
  return client;
}

function browserScheduler() {
  return {
    setTimeout: (callback, delay) => globalThis.setTimeout(callback, delay),
    clearTimeout: (id) => globalThis.clearTimeout(id),
    setInterval: (callback, delay) => globalThis.setInterval(callback, delay),
    clearInterval: (id) => globalThis.clearInterval(id),
  };
}

function requireScheduler(scheduler) {
  for (const method of ["setTimeout", "clearTimeout", "setInterval", "clearInterval"]) {
    if (typeof scheduler?.[method] !== "function") {
      throw new TypeError(`scheduler.${method} is required`);
    }
  }
  return scheduler;
}

function normalizeReconnect(reconnect) {
  const value = { ...DEFAULT_RECONNECT, ...reconnect };
  value.initialDelayMs = nonNegativeSafeInteger(value.initialDelayMs, "initialDelayMs");
  value.maxDelayMs = nonNegativeSafeInteger(value.maxDelayMs, "maxDelayMs");
  value.stableConnectionMs = nonNegativeSafeInteger(
    value.stableConnectionMs,
    "stableConnectionMs",
  );
  if (value.maxDelayMs < value.initialDelayMs) {
    throw new RangeError("maxDelayMs must be greater than or equal to initialDelayMs");
  }
  if (typeof value.multiplier !== "number" || !Number.isFinite(value.multiplier)) {
    throw new TypeError("multiplier must be a finite number");
  }
  if (value.multiplier < 1) {
    throw new RangeError("multiplier must be at least 1");
  }
  return Object.freeze(value);
}

function optionalFunction(value, name) {
  if (value !== undefined && typeof value !== "function") {
    throw new TypeError(`${name} must be a function when supplied`);
  }
  return value;
}

function boundedArray(value, maxLength, name) {
  if (!Array.isArray(value)) {
    throw new TypeError(`${name} drain must return an array`);
  }
  if (value.length > maxLength) {
    throw new RangeError(`${name} drain exceeded configured bound ${maxLength}`);
  }
  return value;
}

function positiveSafeInteger(value, name) {
  const integer = nonNegativeSafeInteger(value, name);
  if (integer === 0) {
    throw new RangeError(`${name} must be greater than zero`);
  }
  return integer;
}

function nonNegativeSafeInteger(value, name) {
  if (!Number.isSafeInteger(value)) {
    throw new TypeError(`${name} must be a safe integer`);
  }
  if (value < 0) {
    throw new RangeError(`${name} must not be negative`);
  }
  return value;
}

function asUint8Array(value, name) {
  if (value instanceof Uint8Array) {
    return value;
  }
  if (value instanceof ArrayBuffer) {
    return new Uint8Array(value);
  }
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  throw new TypeError(`${name} must be Uint8Array, ArrayBuffer, or an ArrayBuffer view`);
}

async function messageBytes(data, encoder, BlobImpl) {
  if (typeof data === "string") {
    return encoder.encode(data);
  }
  if (data instanceof ArrayBuffer || ArrayBuffer.isView(data)) {
    return asUint8Array(data, "WebSocket message");
  }
  if (typeof BlobImpl === "function" && data instanceof BlobImpl) {
    return new Uint8Array(await data.arrayBuffer());
  }
  throw new TypeError("WebSocket message must be text, ArrayBuffer, ArrayBufferView, or Blob");
}

function safeClose(socket) {
  try {
    socket.close();
  } catch {
    // The browser close callback is the only lifecycle publication point.
  }
}

function errorMessage(error) {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}
