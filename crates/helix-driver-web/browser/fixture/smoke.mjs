import init, { HelixWebClient } from "../../pkg/helix_driver_web.js";
import { createHelixBrowserAdapter } from "../helix-browser-adapter.mjs";

let adapter;
let eventHistory = [];
let runtimeIssueHistory = [];
let adapterIssueHistory = [];
let eventHistoryDropped = 0;
let runtimeIssueHistoryDropped = 0;
let adapterIssueHistoryDropped = 0;
let eventCapacity = 1;
let runtimeIssueCapacity = 1;
let adapterIssueCapacity = 32;

/** Called by a real-browser harness; this fixture deliberately renders no UI. */
globalThis.startHelixWebSmoke = async ({ config, wasmInput } = {}) => {
  if (adapter !== undefined) {
    throw new Error("Helix Web smoke fixture is already running");
  }
  const parsedConfig = typeof config === "string" ? JSON.parse(config) : config;
  eventCapacity = positiveCapacity(parsedConfig?.maxPendingEventBatches, "maxPendingEventBatches");
  runtimeIssueCapacity = positiveCapacity(parsedConfig?.maxRuntimeIssues, "maxRuntimeIssues");
  adapterIssueCapacity = Math.max(runtimeIssueCapacity, 32);
  adapter = await createHelixBrowserAdapter({
    initWasm: init,
    wasmInput,
    HelixWebClient,
    config: parsedConfig,
    onEventBatch: (batch) => {
      eventHistoryDropped += pushBounded(eventHistory, batch, eventCapacity);
    },
    onRuntimeIssue: (issue) => {
      runtimeIssueHistoryDropped += pushBounded(
        runtimeIssueHistory,
        issue,
        runtimeIssueCapacity,
      );
    },
    onAdapterIssue: (issue) => {
      adapterIssueHistoryDropped += pushBounded(
        adapterIssueHistory,
        issue,
        adapterIssueCapacity,
      );
    },
  });
  return true;
};

globalThis.helixWebSmokeSnapshot = () => ({
  connectedUrl: adapter?.websocketUrl,
  isShutdown: adapter?.isShutdown ?? false,
  capabilities: adapter === undefined ? undefined : [...adapter.capabilities()],
  events: [...eventHistory],
  runtimeIssues: [...runtimeIssueHistory],
  adapterIssues: [...adapterIssueHistory],
  eventHistoryDropped,
  runtimeIssueHistoryDropped,
  adapterIssueHistoryDropped,
});

globalThis.helixWebSmokeDrain = () => {
  const snapshot = globalThis.helixWebSmokeSnapshot();
  eventHistory = [];
  runtimeIssueHistory = [];
  adapterIssueHistory = [];
  eventHistoryDropped = 0;
  runtimeIssueHistoryDropped = 0;
  adapterIssueHistoryDropped = 0;
  return snapshot;
};

globalThis.helixWebSmokeInvoke = (name, payload) => {
  if (adapter === undefined) {
    throw new Error("Helix Web smoke fixture is not running");
  }
  const bytes = payload instanceof Uint8Array ? payload : new Uint8Array(payload);
  adapter.invoke(name, bytes);
};

globalThis.stopHelixWebSmoke = () => {
  adapter?.shutdown();
};

globalThis.__HELIX_WEB_SMOKE_READY__ = true;

function pushBounded(history, value, capacity) {
  if (history.length === capacity) {
    history.shift();
    history.push(value);
    return 1;
  }
  history.push(value);
  return 0;
}

function positiveCapacity(value, name) {
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new TypeError(`${name} must be a positive safe integer`);
  }
  return value;
}
