import assert from "node:assert/strict";
import test from "node:test";

import {
  MessageLifecycleRecorder,
  WEB_LIFECYCLE_CAPABILITIES,
  WEB_LIFECYCLE_STAGES,
} from "./message-lifecycle-recorder.mjs";

test("records T1 with T2-T5 as direct siblings", () => {
  const recorder = new MessageLifecycleRecorder();
  assert.equal(
    recorder.start({ lifecycleId: "life-1", operationId: "op-1", tickId: "tick-1" }),
    true,
  );
  recorder.record({
    lifecycleId: "life-1",
    tickId: "tick-2",
    stage: "T2",
    status: "ok",
    capability: "http",
  });
  recorder.record({
    lifecycleId: "life-1",
    tickId: "tick-3",
    stage: "T3",
    status: "not_applicable",
    capability: "ws",
    reason: "transport_absent",
  });
  recorder.record({
    lifecycleId: "life-1",
    tickId: "tick-4",
    stage: "T4",
    status: "ok",
    capability: "persist",
  });
  recorder.record({
    lifecycleId: "life-1",
    tickId: "tick-5",
    stage: "T5",
    status: "ok",
    capability: "effect",
  });
  recorder.finish({ lifecycleId: "life-1" });

  const tree = recorder.snapshot().trees[0];
  assert.deepEqual(
    tree.entries.map(({ stage, parentTickId, status }) => ({ stage, parentTickId, status })),
    [
      { stage: "T1", parentTickId: null, status: "ok" },
      { stage: "T2", parentTickId: "tick-1", status: "ok" },
      { stage: "T3", parentTickId: "tick-1", status: "not_applicable" },
      { stage: "T4", parentTickId: "tick-1", status: "ok" },
      { stage: "T5", parentTickId: "tick-1", status: "ok" },
    ],
  );
  assert.deepEqual(WEB_LIFECYCLE_STAGES, ["T1", "T2", "T3", "T4", "T5"]);
  assert.deepEqual(WEB_LIFECYCLE_CAPABILITIES, ["http", "ws", "persist", "effect"]);
});

test("isolates concurrent lifecycle trees and bounds overflow", () => {
  const recorder = new MessageLifecycleRecorder({ maxLifecycles: 1 });
  assert.equal(
    recorder.start({ lifecycleId: "life-1", operationId: "op-1", tickId: "tick-1" }),
    true,
  );
  assert.equal(
    recorder.start({ lifecycleId: "life-2", operationId: "op-2", tickId: "tick-2" }),
    false,
  );
  assert.equal(recorder.snapshot().dropped, 1);
  assert.throws(
      () =>
        recorder.record({
        lifecycleId: "life-1",
        tickId: "tick-3",
        stage: "T3",
        status: "ok",
        capability: "http",
      }),
    /lifecycle T3 requires capability ws/,
  );
  recorder.record({
    lifecycleId: "life-1",
    tickId: "tick-3",
    stage: "T3",
    status: "not_applicable",
    capability: "ws",
    reason: "transport_absent",
  });
  assert.throws(
    () =>
      recorder.record({
        lifecycleId: "life-1",
        tickId: "tick-33",
        stage: "T3",
        status: "skipped",
        capability: "ws",
        reason: "already_recorded",
      }),
    /lifecycle stage T3 was already recorded/,
  );
});
