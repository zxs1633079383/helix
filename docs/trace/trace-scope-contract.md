# Four-repo Trace Scope Contract

This is the canonical trace contract for the IM send-message full link across:

- PC: `/System/Volumes/Data/workspace/rust/loopforge-tauri-im`
- Mobile: `/System/Volumes/Data/workspace/c/mobile-qucik-c++`
- Helix: `/System/Volumes/Data/workspace/rust/helix`
- Server: `/System/Volumes/Data/workspace/golang/cses-im-server`

Production trace export path:

```text
App / Helix / Server span producer
  -> OTLP gRPC OpenTelemetry Collector :4317
  -> Jaeger backend
  -> Jaeger Query /api/traces/{traceId} as proof
```

Jaeger is a query/proof surface. JSONL is allowed only as local debug fallback and must not be the pass criterion for real-chain proof.

## Scope Identity

Every send-message trace must carry a stable scope identity through client, Helix, HTTP, server store, WS fanout, WS receive, and render.

Required correlation fields:

| field | source | purpose |
|---|---|---|
| `trace_id` | client root span | Jaeger full-link trace lookup |
| `corr_key` | client send action | compact match key for scope filtering |
| `channelId` | send request | reject spans from other channels |
| `temporaryId` | client-generated send id | join optimistic/client spans with server echo |
| `serverMsgId` | server ack / WS echo | join server store and final render |
| `seq` | server event sequence | order WS deliver and projection |
| `userId` | request/auth context | reject spans from other users |
| `action` | client operation | reject notify/connect/background spans |

`corr_key` format should include all available identifiers:

```text
ch=<channelId>;tmp=<temporaryId>;sid=<serverMsgId>;seq=<seq>;user=<userId>;action=send_message
```

Unknown fields may be omitted only before they exist, but downstream spans must add them when available.

## PC Required Spans

| order | span | repo | meaning |
|---:|---|---|---|
| 1 | `pc.ui.action` | loopforge | user initiated send |
| 2 | `pc.tauri.invoke.out` or `pc.tauri.invoke` | loopforge | UI to Tauri bridge |
| 3 | `pc.tauri.command.enqueue` or `pc.tauri.command` | loopforge | command accepted by native shell |
| 4 | `helix.command.accept` | helix | command entered host loop |
| 5 | `helix.core.step` | helix | deterministic core tick |
| 6 | `helix.storage.persist` | helix | local/pending persistence |
| 7 | `helix.event.emit` | helix | local event emitted |
| 8 | `helix.http.request` | helix | outbound send HTTP |
| 9 | `cses.http.request` | cses-im-server | server received request |
| 10 | `cses.handler.create_post` | cses-im-server | HTTP handler boundary |
| 11 | `cses.service.create_post` | cses-im-server | application/service boundary |
| 12 | `cses.store.create_post` | cses-im-server | DB write boundary |
| 13 | `cses.ws.publish` | cses-im-server | WS event published |
| 14 | `cses.ws.fanout` | cses-im-server | WS fanout selected recipients |
| 15 | `cses.ws.deliver` | cses-im-server | WS delivered to target connection |
| 16 | `helix.ws.recv` | helix | client inbound WS received |
| 17 | `helix.event.emit` | helix | received event emitted to shell |
| 18 | `pc.tauri.app_emit` | loopforge | Tauri emitted app event |
| 19 | `pc.ui.render` | loopforge | UI rendered final sent/received state |

## Mobile Required Spans

| order | span | repo | meaning |
|---:|---|---|---|
| 1 | `mobile.js.im_send` or `mobile.ui.action` | mobile | JS/native API send entry |
| 2 | `mobile.quickjs.call` | mobile | QuickJS bridge call |
| 3 | `mobile.cpp.call_with_trace` | mobile | C++ traced boundary |
| 4 | `mobile.ffi.command` | mobile | FFI command boundary |
| 5 | `helix.command.accept` | helix | command entered host loop |
| 6 | `helix.core.step` | helix | deterministic core tick |
| 7 | `helix.http.request` | helix | outbound send HTTP |
| 8 | `helix.http.response` | helix | outbound HTTP response captured |
| 9 | `cses.http.request` | cses-im-server | server received request |
| 10 | `cses.ws.deliver` | cses-im-server | WS delivered to client |
| 11 | `helix.ws.recv` | helix | client inbound WS received |
| 12 | `mobile.render` or `mobile.ui.render` | mobile | real drain/render path observed |

Mobile proof must use the real runtime event drain path. Synthetic waiting helpers cannot be the only render evidence.

## Forbidden Spans And Payloads

A send-message trace must fail if it contains background or unrelated entries:

- `/api/cses/channel/sync/notify`
- `sync.notify`
- join/member notice payloads not caused by the send action
- presence/reconnect/badge/connect lifecycle payloads
- any span whose `channelId`, `userId`, `temporaryId`, `serverMsgId`, or `action` contradicts the root send scope

## Capture Policy

Default policy is full capture for test/smoke proof:

| surface | default | required fields |
|---|---|---|
| HTTP request headers | capture all | traceparent, user/cookie headers, content-type |
| HTTP request body | capture all | action, channelId, temporaryId, payload |
| HTTP response | capture all | status code, response body, serverMsgId/seq when present |
| WS inbound/outbound | capture all | action/event name, target userId, channelId, payload |
| SQL statement | capture full statement | operation, table, tx id when present |
| SQL params | capture all params | redact only secrets/tokens/password-like fields |

The capture layer must support regex include/exclude and field redaction. The default smoke/proof profile is full capture with bounded payload size.

## Diagnostic Contract

Jaeger proof must not only say "missing spans". It must output:

- missing or forbidden span/payload
- likely repo
- likely boundary/entry file or function
- next command to verify

Examples:

| symptom | likely repo | next check |
|---|---|---|
| missing `helix.core.step` | helix | `HELIX_OTEL_PROTOCOL=noop cargo test -p helix-driver-host --test otel_boundary_test -- --nocapture` |
| missing `pc.tauri.app_emit` | loopforge | inspect Tauri app emit trace bridge and rerun `bash scripts/otel-pc-send-trace-smoke.sh` |
| missing `mobile.render` | mobile | inspect `im.drainEvents()` render trace correlation and rerun `REAL_CHAIN_CASE=UC-1.1 make real-chain-trace` |
| missing `cses.store.create_post` | cses-im-server | inspect create-post store instrumentation and SQL span export |
| forbidden notify/presence payload | checker/config | tighten scope filter by corr_key, action, channelId, userId |

## Helix Pin Gate

After Helix trace changes, downstream pins must be checked before declaring PC/Mobile proof current:

```bash
cd /System/Volumes/Data/workspace/rust/helix
bash scripts/check-downstream-helix-pins.sh
```

The gate must check at least:

- Loopforge `src-tauri/Cargo.toml`
- Loopforge `crates/helix-driver-instrument/Cargo.toml`
- Mobile `scripts/build-helix-ffi.sh`
- Mobile `config/helix.env.example`

