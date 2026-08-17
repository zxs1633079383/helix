/**
 * Web/WASM lifecycle seam only. It records the shape of a MessageLifecycle
 * tree for WebDriver; it never creates spans, exports OTLP, or inspects payloads.
 */
export const WEB_LIFECYCLE_STAGES = Object.freeze(["T1", "T2", "T3", "T4", "T5"]);

/** Stable statuses shared by browser adapter and deterministic test recorder. */
export const WEB_LIFECYCLE_STATUSES = Object.freeze([
  "started",
  "ok",
  "error",
  "skipped",
  "not_applicable",
]);

/** Optional capability labels; absent HTTP/WS must remain explicit. */
export const WEB_LIFECYCLE_CAPABILITIES = Object.freeze(["http", "ws", "persist", "effect"]);

const CHILD_STAGES = new Set(WEB_LIFECYCLE_STAGES.slice(1));
const STATUS_SET = new Set(WEB_LIFECYCLE_STATUSES);
const CAPABILITY_SET = new Set(WEB_LIFECYCLE_CAPABILITIES);

/**
 * Bounded, deterministic recorder for the Web/WASM Adapter contract.
 *
 * A recorder entry is deliberately metadata-only. The host may feed real
 * capability checkpoints, but the seam cannot turn a missing channel into a
 * synthetic HTTP/WS span and cannot send anything to a collector.
 */
export class MessageLifecycleRecorder {
  constructor({ maxLifecycles = 64 } = {}) {
    positiveSafeInteger(maxLifecycles, "maxLifecycles");
    this._maxLifecycles = maxLifecycles;
    this._trees = new Map();
    this._dropped = 0;
  }

  /** Start one isolated T1 root; duplicate lifecycle IDs fail closed. */
  start({ lifecycleId, operationId, tickId }) {
    const id = nonEmptyString(lifecycleId, "lifecycleId");
    const operation = nonEmptyString(operationId, "operationId");
    const rootTick = nonEmptyString(tickId, "tickId");
    if (this._trees.has(id)) {
      throw new Error(`duplicate lifecycleId ${id}`);
    }
    if (this._trees.size >= this._maxLifecycles) {
      this._dropped += 1;
      return false;
    }
    this._trees.set(id, {
      lifecycleId: id,
      operationId: operation,
      rootTickId: rootTick,
      tickIds: new Set([rootTick]),
      stages: new Set(["T1"]),
      entries: [
        Object.freeze({
          lifecycleId: id,
          operationId: operation,
          tickId: rootTick,
          parentTickId: null,
          stage: "T1",
          status: "started",
          capability: undefined,
          reason: undefined,
        }),
      ],
    });
    return true;
  }

  /** Record one T2/T3/T4/T5 sibling under T1; payloads never cross this seam. */
  record({ lifecycleId, tickId, stage, status, capability, reason }) {
    const id = nonEmptyString(lifecycleId, "lifecycleId");
    const tree = this._trees.get(id);
    if (tree === undefined) {
      throw new Error(`unknown lifecycleId ${id}`);
    }
    const childTick = nonEmptyString(tickId, "tickId");
    if (!CHILD_STAGES.has(stage)) {
      throw new TypeError("lifecycle child stage must be T2, T3, T4, or T5");
    }
    if (!STATUS_SET.has(status)) {
      throw new TypeError(`unsupported lifecycle status ${String(status)}`);
    }
    if (capability !== undefined && !CAPABILITY_SET.has(capability)) {
      throw new TypeError(`unsupported lifecycle capability ${String(capability)}`);
    }
    if (capability !== undefined && capability !== expectedCapability(stage)) {
      throw new TypeError(`lifecycle ${stage} requires capability ${expectedCapability(stage)}`);
    }
    if (tree.stages.has(stage)) {
      throw new Error(`lifecycle stage ${stage} was already recorded`);
    }
    if (tree.tickIds.has(childTick)) {
      throw new Error(`duplicate tickId ${childTick}`);
    }
    if (reason !== undefined) {
      nonEmptyString(reason, "reason");
    }
    tree.entries.push(
      Object.freeze({
        lifecycleId: id,
        operationId: tree.operationId,
        tickId: childTick,
        parentTickId: tree.rootTickId,
        stage,
        status,
        capability,
        reason,
      }),
    );
    tree.tickIds.add(childTick);
    tree.stages.add(stage);
    return true;
  }

  /** Mark the T1 root terminal without inventing a child network operation. */
  finish({ lifecycleId, status = "ok" }) {
    const id = nonEmptyString(lifecycleId, "lifecycleId");
    const tree = this._trees.get(id);
    if (tree === undefined) {
      throw new Error(`unknown lifecycleId ${id}`);
    }
    if (!STATUS_SET.has(status) || status === "started" || status === "not_applicable") {
      throw new TypeError("lifecycle finish status must be ok, error, or skipped");
    }
    const root = tree.entries[0];
    tree.entries[0] = Object.freeze({ ...root, status });
  }

  /** Return an immutable snapshot for WebDriver assertions; no exporter is involved. */
  snapshot() {
    return Object.freeze({
      trees: Object.freeze(
        [...this._trees.values()].map((tree) =>
          Object.freeze({
            lifecycleId: tree.lifecycleId,
            operationId: tree.operationId,
            rootTickId: tree.rootTickId,
            entries: Object.freeze([...tree.entries]),
          }),
        ),
      ),
      dropped: this._dropped,
    });
  }

  /** Drain completed or in-flight trees in insertion order and reset the bound. */
  drain() {
    const snapshot = this.snapshot();
    this._trees.clear();
    this._dropped = 0;
    return snapshot;
  }
}

/** Validate a positive bounded capacity without relying on browser globals. */
function positiveSafeInteger(value, name) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new TypeError(`${name} must be a positive safe integer`);
  }
  return value;
}

/** Validate IDs and reasons without normalizing user data into a payload. */
function nonEmptyString(value, name) {
  if (typeof value !== "string" || value.length === 0) {
    throw new TypeError(`${name} must be a non-empty string`);
  }
  return value;
}

/** Keep the four fixed child slots aligned with their real transport/effect capability. */
function expectedCapability(stage) {
  return {
    T2: "http",
    T3: "ws",
    T4: "persist",
    T5: "effect",
  }[stage];
}
