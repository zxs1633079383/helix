import assert from "node:assert/strict";
import test from "node:test";

import {
  ProbeValidationError,
  assertSafeProbeExport,
  assertValidProbeCase,
  classifyProbeCase,
  prepareProbeExport,
} from "./probe-model.mjs";

function probeCase(overrides = {}) {
  return {
    caseId: "W0-model",
    pageOrigin: "https://web.example.test",
    apiOrigin: "https://api.example.test",
    wsOrigin: "wss://ws.example.test",
    http: { status: 204, corsClass: "allowed" },
    websocket: { opened: true, helloReceived: true, closeCode: 1000 },
    secretsRedacted: true,
    ...overrides,
  };
}

function hasCode(code) {
  return (error) => error instanceof ProbeValidationError && error.code === code;
}

test("HTTP 2xx and websocket hello pass", () => {
  assert.equal(classifyProbeCase(probeCase()), "pass");
});

test("websocket open without hello fails auth or protocol", () => {
  const input = probeCase({
    websocket: { opened: true, helloReceived: false, closeCode: 1000 },
  });
  assert.equal(classifyProbeCase(input), "fail-auth-or-protocol");
});

test("rejected preflight fails CORS", () => {
  const input = probeCase({
    http: { preflight: "rejected", corsClass: "preflight-rejected" },
    websocket: { opened: false, helloReceived: false },
  });
  assert.equal(classifyProbeCase(input), "fail-cors");
});

test("websocket close 1006 is an abnormal close", () => {
  const input = probeCase({
    websocket: { opened: true, helloReceived: false, closeCode: 1006 },
  });
  assert.equal(classifyProbeCase(input), "fail-abnormal-close");
});

for (const status of [401, 403]) {
  test(`HTTP ${status} fails authentication`, () => {
    assert.equal(classifyProbeCase(probeCase({ http: { status } })), "fail-auth");
  });
}

for (const field of ["pageOrigin", "apiOrigin", "wsOrigin"]) {
  test(`missing ${field} is rejected`, () => {
    const input = probeCase();
    delete input[field];
    assert.throws(() => assertValidProbeCase(input), hasCode("missing-origin"));
  });
}

test("origins reject paths, queries and embedded credentials", () => {
  for (const pageOrigin of [
    "https://web.example.test/probe",
    "https://web.example.test?ticket=secret-value",
    "https://user:password@web.example.test",
  ]) {
    assert.throws(
      () => assertValidProbeCase(probeCase({ pageOrigin })),
      hasCode("invalid-origin"),
    );
  }
});

test("forbidden evidence field names are rejected", () => {
  for (const unsafe of [
    { headers: { authorization: "redacted" } },
    { cookieId: "redacted" },
    { response_body: "redacted" },
    { rawFrame: "redacted" },
  ]) {
    assert.throws(() => assertSafeProbeExport(unsafe), hasCode("unsafe-export"));
  }
});

test("credential-like values are rejected", () => {
  for (const note of [
    "Authorization: Bearer abc.def.ghi",
    "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.signature123",
    "wss://ws.example.test/socket?ticket=one-time-secret",
    "cookieId=actual-session-value",
  ]) {
    assert.throws(() => assertSafeProbeExport({ note }), hasCode("unsafe-export"));
  }
});

test("safe evidence is classified and prepared for export", () => {
  const output = prepareProbeExport(probeCase({
    firstAction: "hello",
    closeReasonClass: "normal",
  }));
  assert.equal(output.outcome, "pass");
  assert.equal(output.secretsRedacted, true);
  assert.doesNotThrow(() => assertSafeProbeExport(output));
});
