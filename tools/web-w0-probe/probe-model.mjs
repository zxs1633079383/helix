const REQUIRED_ORIGINS = ["pageOrigin", "apiOrigin", "wsOrigin"];

const FORBIDDEN_EXPORT_KEYS = new Set([
  "accesstoken",
  "authorization",
  "body",
  "cookie",
  "cookieid",
  "headers",
  "message",
  "password",
  "rawframe",
  "rawframes",
  "refreshtoken",
  "requestheaders",
  "responsebody",
  "responseheaders",
  "secret",
  "token",
  "username",
]);

const SENSITIVE_QUERY_KEY = /^(?:access_?token|authorization|cookie|cookie_?id|password|secret|ticket|token)$/i;
const BEARER_VALUE = /\bBearer\s+[A-Za-z0-9._~+/-]+=*/i;
const JWT_VALUE = /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b/;
const INLINE_SECRET = /(?:access_?token|authorization|cookie_?id|password|secret|ticket|token)\s*[=:]\s*[^\s&]+/i;

export class ProbeValidationError extends Error {
  constructor(code, path, detail) {
    super(`${code} at ${path}: ${detail}`);
    this.name = "ProbeValidationError";
    this.code = code;
    this.path = path;
  }
}

function reject(code, path, detail) {
  throw new ProbeValidationError(code, path, detail);
}

function normalizedKey(key) {
  return key.toLowerCase().replaceAll(/[-_]/g, "");
}

function assertPlainOrigin(field, value) {
  if (typeof value !== "string" || value.length === 0) {
    reject("missing-origin", field, "origin is required");
  }

  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    reject("invalid-origin", field, "origin must be an absolute URL");
  }

  const allowedProtocols = field === "wsOrigin" ? ["ws:", "wss:"] : ["http:", "https:"];
  if (!allowedProtocols.includes(parsed.protocol)) {
    reject("invalid-origin", field, `protocol ${parsed.protocol} is not allowed`);
  }
  if (
    parsed.username ||
    parsed.password ||
    parsed.pathname !== "/" ||
    parsed.search ||
    parsed.hash ||
    parsed.origin !== value.replace(/\/$/, "")
  ) {
    reject("invalid-origin", field, "only scheme, host and optional port are allowed");
  }
}

export function assertValidProbeCase(probeCase) {
  if (!probeCase || typeof probeCase !== "object" || Array.isArray(probeCase)) {
    reject("invalid-case", "$", "probe case must be an object");
  }
  for (const field of REQUIRED_ORIGINS) {
    assertPlainOrigin(field, probeCase[field]);
  }
  return probeCase;
}

export function classifyProbeCase(probeCase) {
  assertValidProbeCase(probeCase);

  const http = probeCase.http ?? {};
  const websocket = probeCase.websocket ?? {};
  if (http.preflight === "rejected" || http.corsClass === "preflight-rejected") {
    return "fail-cors";
  }
  if (http.status === 401 || http.status === 403) {
    return "fail-auth";
  }
  if (websocket.closeCode === 1006) {
    return "fail-abnormal-close";
  }
  if (websocket.opened === true && websocket.helloReceived !== true) {
    return "fail-auth-or-protocol";
  }
  if (http.status >= 200 && http.status < 300 && websocket.helloReceived === true) {
    return "pass";
  }
  if (http.errorClass === "network" || websocket.errorClass === "network") {
    return "fail-network";
  }
  if (Number.isInteger(http.status)) {
    return "fail-http";
  }
  return "incomplete";
}

function assertSafeString(value, path) {
  if (BEARER_VALUE.test(value) || JWT_VALUE.test(value) || INLINE_SECRET.test(value)) {
    reject("unsafe-export", path, "value resembles a credential");
  }

  let parsed;
  try {
    parsed = new URL(value);
  } catch {
    return;
  }
  for (const key of parsed.searchParams.keys()) {
    if (SENSITIVE_QUERY_KEY.test(key)) {
      reject("unsafe-export", path, `URL contains sensitive query key ${key}`);
    }
  }
}

export function assertSafeProbeExport(value) {
  const seen = new WeakSet();

  function visit(current, path) {
    if (typeof current === "string") {
      assertSafeString(current, path);
      return;
    }
    if (current === null || typeof current !== "object") {
      return;
    }
    if (seen.has(current)) {
      reject("unsafe-export", path, "cyclic objects cannot be exported");
    }
    seen.add(current);

    for (const [key, child] of Object.entries(current)) {
      const childPath = `${path}.${key}`;
      if (FORBIDDEN_EXPORT_KEYS.has(normalizedKey(key))) {
        reject("unsafe-export", childPath, "field name is forbidden in exported evidence");
      }
      visit(child, childPath);
    }
  }

  visit(value, "$");
  return value;
}

export function prepareProbeExport(probeCase) {
  assertValidProbeCase(probeCase);
  const output = { ...probeCase, outcome: classifyProbeCase(probeCase) };
  assertSafeProbeExport(output);
  return output;
}
