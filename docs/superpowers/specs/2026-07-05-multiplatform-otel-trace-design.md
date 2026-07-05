# Multiplatform OpenTelemetry Trace Design

Date: 2026-07-05

## 1. Goal

Build a full-link OpenTelemetry trace system across:

- PC Tauri client in `/System/Volumes/Data/workspace/rust/loopforge-tauri-im`
- mobile QuickJS/C++ bridge in `/System/Volumes/Data/workspace/c/mobile-qucik-c++`
- Helix runtime-agnostic engine in `/System/Volumes/Data/workspace/rust/helix`
- Go IM backend in `/System/Volumes/Data/workspace/golang/cses-im-server`

The first production diagnostic target is one `im.send(...)` flow:

```text
client action
  -> Tauri invoke or QuickJS CoreBridge.call
  -> helix command
  -> optimistic storage
  -> im:post:sending
  -> helix HTTP posts/create
  -> cses-im-server HTTP handler/service/store
  -> cses-im-server WS publish/fanout/deliver
  -> helix WS recv
  -> helix gate/storage/reconcile
  -> im:post:received
  -> client render
```

The trace must make these failure classes distinguishable without guessing:

- client never sent the command
- command reached Helix but did not produce HTTP/WS/storage effects
- HTTP reached Go but handler/service/store failed
- Go wrote data but did not publish WS
- WS was published but not received by Helix
- Helix received WS but gate/storage/projection did not advance
- projection reached client bridge but UI did not render

## 2. Non-Goals

- Do not put OpenTelemetry dependencies in `helix-core`.
- Do not add trace fields to business domain models, persisted business rows, or Go business DTOs.
- Do not make environment variables the primary configuration path.
- Do not instrument every internal Go function in v1. Function-level coverage means boundary functions only.
- Do not replace existing JSONL / harness evidence in loopforge; OTel complements it.
- Do not require all projects to enable tracing together. Each project must work with tracing off or independently on.

## 3. Final Approach

Use full OpenTelemetry with a local OTel Collector and Jaeger.

```text
PC Tauri / mobile QuickJS / helix host / cses-im-server
  -> OTLP grpc/http endpoint
  -> OTel Collector
  -> Jaeger
  -> Jaeger UI
```

All projects export to the Collector. No project exports directly to Jaeger. This keeps exporter configuration uniform and lets us change the backend later without touching app code.

The selected approach is intentionally hybrid:

- OpenTelemetry is the output format and runtime trace model.
- W3C `traceparent` / `baggage` is the cross-process carrier.
- Project-specific boundary wrappers decide where spans start/end.
- `helix-core` remains sans-IO and runtime-neutral.

Rejected alternatives:

- JSONL-only tracing: too weak for cross-project distributed trace and Jaeger inspection.
- Direct Jaeger SDK per project: tighter coupling and different exporter behavior per language.
- Trace fields inside business payloads: violates business-schema separation and will pollute storage/projection contracts.

## 4. Configuration

Tracing is configured through each project's existing config system. Local scripts may choose a config file or profile, but tracing enablement, endpoint, sampler, and facets must come from that config file/profile rather than environment variables.

### 4.1 Common Config Shape

YAML form:

```yaml
observability:
  otel:
    enabled: true
    serviceName: "helix-host-cli"
    endpoint: "http://127.0.0.1:4317"
    protocol: "grpc"
    sampler:
      type: "parentbased_traceidratio"
      ratio: 1.0
    attributes:
      deployment.environment: "local"
    facets:
      command: true
      core: true
      http: true
      ws: true
      storage: true
      event: true
```

JSON form:

```json
{
  "observability": {
    "otel": {
      "enabled": true,
      "serviceName": "loopforge-tauri-im",
      "endpoint": "http://127.0.0.1:4318",
      "protocol": "http",
      "sampler": {
        "type": "parentbased_traceidratio",
        "ratio": 1.0
      },
      "facets": {
        "client": true,
        "tauri": true,
        "core": true,
        "http": true,
        "ws": true,
        "storage": true,
        "event": true,
        "render": true
      }
    }
  }
}
```

### 4.2 Project Config Locations

| Project | Primary config path | Design change |
|---|---|---|
| `helix` | `helix.config.yaml` | Add `observability.otel` for `helix-host-cli` and repo-local verification. |
| `loopforge-tauri-im` | `config/dev-local.json`, `config/pre.json`, `config/prod.json` | Add `observability.otel` to each profile. Tauri Rust config loader passes it to tracing initialization. |
| `mobile-qucik-c++` | New `config/mobile-local.json` | Replace env-first smoke config for trace settings. JS/C++ smoke reads this JSON and passes tracing config through `CoreBridgeConfig`. |
| `cses-im-server` | local `config.yaml` + Consul KV | Local dev reads `config.yaml`; production/shared deployments read `observability.otel` from Consul KV under the existing prefix. |

### 4.3 Facets

Facets are independent switches. Turning off one facet must not disable trace propagation.

Helix facets:

- `command`
- `core`
- `http`
- `ws`
- `storage`
- `portReply`
- `event`
- `native`
- `ffi`

Go facets:

- `http`
- `handler`
- `service`
- `store`
- `ws`
- `pulsar`
- `grpc`
- `redis`
- `es`

Client facets:

- `client`
- `tauri`
- `quickjs`
- `cpp`
- `ffi`
- `render`

## 5. Trace Context Carriers

### 5.1 HTTP

HTTP uses standard W3C headers:

```text
traceparent: 00-<trace_id>-<span_id>-01
baggage: helix.facets=core,http,ws,storage
```

Helix already has `traceparent` pass-through behavior in the host HTTP crosscut layer. The OTel design keeps that as the invariant: the crosscut layer may inject missing context, but it must not rewrite an already valid inbound `traceparent` unless it is starting a child span and using standard OTel propagation.

### 5.2 WebSocket Handshake

WS connection establishment carries a session-level context in handshake headers:

```text
traceparent: 00-...
baggage: client=pc-tauri
```

This context proves connection-level ancestry but does not replace per-message tracing. A single WS connection can carry many business operations.

### 5.3 WebSocket Frames

Per-frame trace context uses the existing WS envelope's top-level tracing area:

```json
{
  "v": 1,
  "action": "post",
  "data": {},
  "broadcast": {},
  "tracing": {
    "traceparent": "00-<trace_id>-<span_id>-01",
    "baggage": "channel=im"
  },
  "seq": 40114
}
```

Rules:

- `tracing` is a system envelope field, not business `data`.
- Helix parser extracts trace context before business classification.
- Business handlers receive normal business data only.
- If `tracing` is absent or invalid, the receiving project starts or continues a local root span according to OTel defaults.

### 5.4 Tauri Invoke

Tauri commands carry trace context as an invoke sidecar, not inside business payload:

```json
{
  "payload": {
    "channelId": "ch...",
    "text": "hello"
  },
  "__trace": {
    "traceparent": "00-...",
    "baggage": "client=loopforge"
  }
}
```

The Tauri command wrapper extracts `__trace`, removes it from the business payload, and sends the context to Helix's command boundary.

### 5.5 FFI Command

The FFI boundary should remain additive. The old function stays valid:

```c
helix_command(handle, name, name_len, payload, payload_len)
```

Add a trace-aware variant:

```c
helix_command_with_trace(
  handle,
  name, name_len,
  payload, payload_len,
  trace_json, trace_json_len
)
```

`trace_json` contains W3C carrier fields. C++ and QuickJS treat it as boundary metadata. The business payload remains unchanged.

If we decide not to add a new ABI symbol in v1, the fallback is to pass trace sidecar through `CoreBridgeConfig` and let the next Rust command span start locally. That is less complete and should be treated as a temporary limitation.

## 6. Helix Instrumentation

### 6.1 Dependency Rule

`helix-core` must not depend on OTel or any runtime-specific tracing exporter. Instrumentation belongs in:

- `helix-driver-host`
- `helix-driver-native`
- `helix-driver-ffi`
- host applications such as `helix-host-cli` and loopforge Tauri backend

`helix-im` may emit normal `tracing` events or expose metadata through existing effects, but OTel span lifecycle is owned by the host layer.

### 6.2 Span Matrix

| Span | Boundary | Parent | Required attributes |
|---|---|---|---|
| `helix.command.accept` | `Tick::Command` accepted by pump | client/bridge span | `helix.command`, `trace.source`, `payload.bytes` |
| `helix.core.step` | one tick -> effect batch | command / ws recv / port reply | `tick.kind`, `effects.count`, `duration.ms` |
| `helix.http.request` | `Effect::Http` / `HttpFire` dispatch | core step | `http.method`, `url.path`, `helix.corr`, `fire_and_forget` |
| `helix.ws.send` | `Effect::Send` dispatch | core step | `ws.transport_id`, `payload.bytes` |
| `helix.ws.recv` | inbound frame to `Tick::Inbound` | frame context from WS `tracing` | `ws.action`, `ws.seq`, `payload.bytes` |
| `helix.storage.persist` | `Effect::Persist` dispatch | core step | `storage.ops.count`, `storage.tables`, `helix.corr` |
| `helix.port.reply` | `Tick::PortReply` | stored corr context | `helix.corr`, `reply.kind`, `error.class` |
| `helix.event.emit` | `Effect::Emit` / `EventSink::emit` | core step | `event.name`, `payload.bytes` |
| `helix.native.app_emit` | Tauri `app.emit("im:__bus__")` | event emit | `event.channel`, `tauri.event` |
| `helix.ffi.batch_flush` | `FfiEventSink::flush` | event emit batch | `events.count`, `batch.bytes` |

### 6.3 Async Context Registry

Effects are asynchronous and replies come back later. Host-side instrumentation needs small trace registries:

- `Correlation -> SpanContext` for HTTP/storage effects
- `TransportId -> SpanContext` for WS connection-level spans
- `TimerId -> SpanContext` if timer traces become necessary

This registry is an observability data structure, not a business state structure. It must be bounded by existing in-flight operations and cleaned when replies, disconnects, or shutdown happen.

### 6.4 Storage Detail Policy

Storage spans must be useful without leaking message content:

Allowed:

- table name
- operation kind
- row count
- corr id
- duration
- error class

Forbidden:

- full message text
- token/header values
- full SQL with sensitive values
- raw blob payload

## 7. Go Server Instrumentation

### 7.1 Scope

V1 function-level coverage means boundary functions:

- HTTP middleware
- HTTP handlers
- application/service methods
- store methods
- WS publish/fanout/deliver
- Pulsar produce/consume
- gRPC client calls
- Redis and Elasticsearch client calls

Do not instrument arbitrary internal helper functions in v1. If a helper later becomes a stable diagnostic boundary, add it deliberately.

### 7.2 Span Matrix

| Span | Boundary | Parent | Required attributes |
|---|---|---|---|
| `cses.http.request` | gorilla/mux middleware | extracted HTTP context | `http.method`, `http.route`, `http.status_code`, `user.id?`, `company.id?` |
| `cses.handler.<name>` | handler entry | HTTP span | `handler.name`, `channel.id?`, `post.id?` |
| `cses.service.<name>` | app/service method | handler span | `service.name`, `operation` |
| `cses.store.<name>` | store method | service span | `db.system`, `db.operation`, `table`, `rows.affected` |
| `cses.grpc.<method>` | paas gRPC client | service span | `rpc.service`, `rpc.method`, `rpc.status_code` |
| `cses.pulsar.produce` | message produce | service/span | `messaging.system`, `topic`, `key` |
| `cses.pulsar.consume` | message consume | extracted carrier | `messaging.system`, `topic`, `consumer` |
| `cses.ws.publish` | event publish | service span | `ws.action`, `ws.seq`, `channel.id` |
| `cses.ws.fanout` | hub fanout | publish span | `viewer.count`, `local.count`, `remote.count` |
| `cses.ws.deliver` | per connection deliver | fanout span | `connection.id`, `user.id`, `dropped?` |

### 7.3 WS Trace Injection

When Go publishes a WS event, it injects the current context into the event envelope `tracing` field:

```json
{
  "tracing": {
    "traceparent": "00-...",
    "baggage": "..."
  }
}
```

For fanout, child spans must not accidentally become siblings' parents. The server already has documented trace parent restoration constraints around cluster publish; the OTel implementation must preserve that pattern.

### 7.4 Config Source

Local config:

```yaml
observability:
  otel:
    enabled: true
    serviceName: "cses-im-server"
    endpoint: "http://127.0.0.1:4317"
    protocol: "grpc"
    facets:
      http: true
      handler: true
      service: true
      store: true
      ws: true
      pulsar: true
      grpc: true
      redis: true
      es: true
```

Production/shared config should be stored in Consul KV under the existing config prefix. Local `config.yaml` remains the development bootstrap and local override path.

## 8. PC Tauri Instrumentation

### 8.1 Loopforge Config

Add `observability.otel` to each profile:

- `config/dev-local.json`
- `config/pre.json`
- `config/prod.json`

The existing Tauri Rust `config::load()` should parse the new optional section and initialize OTel before spawning the Helix engine.

### 8.2 Span Matrix

| Span | Boundary | Parent | Attributes |
|---|---|---|---|
| `pc.ui.action` | Angular UI action | root or incoming context | `ui.action`, `component`, `temporary_id?` |
| `pc.tauri.invoke` | `TauriBridgeService.invoke` | UI action | `tauri.command`, `payload.bytes` |
| `pc.tauri.command` | Rust `#[tauri::command]` entry | invoke sidecar | `tauri.command`, `accepted` |
| `pc.tauri.app_emit` | `app.emit("im:__bus__")` | Helix event span | `event.channel`, `payload.bytes` |
| `pc.ui.render` | Angular store/projection render | app emit | `event.channel`, `dom.anchor?` |

### 8.3 Business Isolation

Angular service methods should not manually add trace fields to business arguments. A bridge wrapper builds the invoke envelope and strips it before Rust sends command payloads to Helix.

## 9. Mobile QuickJS/C++ Instrumentation

### 9.1 Mobile Config

Add:

```text
/System/Volumes/Data/workspace/c/mobile-qucik-c++/config/mobile-local.json
```

Example:

```json
{
  "apiBaseUrl": "http://127.0.0.1:8066/api/cses",
  "wsUrl": "ws://127.0.0.1:8066/api/v4/websocket",
  "observability": {
    "otel": {
      "enabled": true,
      "serviceName": "mobile-quickjs",
      "endpoint": "http://127.0.0.1:4318",
      "protocol": "http",
      "facets": {
        "quickjs": true,
        "cpp": true,
        "ffi": true,
        "event": true,
        "render": true
      }
    }
  }
}
```

`config/helix.env.example` may remain for existing scripts, but trace configuration should move to JSON and the smoke runners should prefer JSON.

### 9.2 Span Matrix

| Span | Boundary | Parent | Attributes |
|---|---|---|---|
| `mobile.js.im_send` | JS facade `im.send(...)` | root/client | `command`, `temporary_id`, `channel.id` |
| `mobile.quickjs.call` | QuickJS native binding | JS span | `core.command`, `payload.bytes` |
| `mobile.cpp.call` | C++ `CoreBridge::call` | QuickJS span | `core.command`, `rc` |
| `mobile.ffi.command` | `helix_command_with_trace` | C++ span | `ffi.abi`, `command` |
| `mobile.cpp.event_batch` | C++ `onEvents` callback | Helix event context | `batch.bytes`, `dropped.count` |
| `mobile.js.event_drain` | JS `drainEvents` / `onEvents` | C++ batch | `events.count` |
| `mobile.render` | final UI/event consumption | event drain | `event.channel` |

### 9.3 C++ Boundary Safety

OTel callbacks must not throw across C ABI boundaries. C++ bridge code must keep the existing callback safety rule:

- callback is `noexcept`
- immediately copies bytes
- errors increment counters or produce local spans with error status
- no exception crosses into Rust/FFI

## 10. Local OTel Infrastructure

Add a local compose/runbook in the implementation phase, not in this design-only step.

Intended topology:

```text
otel-collector
  receivers:
    otlp:
      protocols:
        grpc: 4317
        http: 4318
  exporters:
    otlp/jaeger: jaeger:4317

jaeger
  ui: 16686
```

Required runbook behavior:

1. Start Collector + Jaeger.
2. Confirm Collector listens on `4317` and `4318`.
3. Start `cses-im-server` with config tracing enabled.
4. Start PC or mobile client with config tracing enabled.
5. Trigger `im.send(...)`.
6. Open Jaeger UI and search by service or trace id.

## 11. Attributes and Naming

### 11.1 Common Attributes

Use stable, low-cardinality attributes by default:

- `service.name`
- `deployment.environment`
- `helix.module`
- `helix.command`
- `helix.corr`
- `ws.action`
- `ws.seq`
- `event.name`
- `channel.id`
- `post.id`
- `temporary_id`
- `http.route`
- `db.operation`
- `error.type`

High-cardinality values are allowed only when they are already diagnostic anchors:

- `trace_id`
- `temporary_id`
- `post.id`
- `channel.id`

Never record:

- tokens
- cookies
- full Authorization header
- full message text by default
- raw SQL values

### 11.2 Span Naming

Span names must identify the boundary, not arbitrary payload:

Good:

- `helix.http.request`
- `cses.handler.create_post`
- `helix.storage.persist`
- `mobile.cpp.call`

Bad:

- `send hello world to channel abc`
- `POST /api/cses/posts/create?token=...`

## 12. Error Handling

All spans should set OTel status on errors:

- invalid config: startup error if OTel is enabled and exporter cannot be initialized
- exporter temporarily unavailable: application continues, spans are dropped according to OTel SDK behavior
- invalid incoming traceparent: record local attribute `trace.invalid=true`, continue with local root
- disabled facet: do not create spans for that facet, but continue propagation
- missing downstream trace: create local child/root as appropriate

Tracing must never change business success/failure behavior.

## 13. Performance

Expected complexity:

- context extract/inject: O(headers) with a tiny bounded header set
- span creation per boundary: O(1)
- async registry lookup by corr/transport: O(1)
- event batch tracing: O(events in batch), no per-byte scan except size/count

Sampling:

- local dev: ratio `1.0`
- pre/prod: default ratio below `1.0`, with parent-based sampling
- request-level force tracing can be represented as baggage/configured debug headers later, but config remains the primary switch

Instrumentation must avoid:

- allocating full payload copies for spans
- serializing full business objects only for trace attributes
- adding locks on hot send/event paths beyond the existing boundary synchronization

## 14. Verification Plan

### 14.1 Static Verification

- `helix-core` has no OTel dependency.
- business DTOs do not gain trace fields.
- mobile business payloads remain unchanged.
- WS trace context is in envelope `tracing`, not `data`.
- PC invoke trace sidecar is stripped before command payload reaches Helix business handling.

### 14.2 Local End-to-End Verification

Run:

```text
Collector + Jaeger
cses-im-server with observability.otel.enabled=true
PC Tauri or mobile QuickJS with observability.otel.enabled=true
im.send(...)
```

Jaeger must show a single trace containing at least:

```text
pc.ui.action or mobile.js.im_send
pc.tauri.invoke or mobile.quickjs.call
helix.command.accept
helix.core.step
helix.storage.persist
helix.event.emit im:post:sending
helix.http.request posts/create
cses.http.request
cses.handler.create_post
cses.service.create_post
cses.store.create_post
cses.ws.publish
cses.ws.fanout
cses.ws.deliver
helix.ws.recv
helix.storage.persist
helix.event.emit im:post:received
pc.ui.render or mobile.render
```

### 14.3 Independent Enablement Verification

Each project must pass these cases:

- client tracing on, helix/go tracing off: client spans export, business still works
- helix tracing on, client/go tracing off: Helix starts local root spans, business still works
- Go tracing on, client/helix tracing off: Go HTTP/WS spans export, business still works
- all tracing off: no exporter startup required

## 15. Implementation Phasing

### Phase 0: Spec and Config Schema

- Finalize this spec.
- Add config schema sections to project docs.
- Do not add runtime dependencies yet.

### Phase 1: Local OTel Infrastructure

- Add OTel Collector + Jaeger local runbook/compose.
- Verify Collector endpoints.

### Phase 2: Go Server Boundary Tracing

- Add OTel SDK/bootstrap from `config.yaml + Consul KV`.
- Add HTTP middleware and boundary function spans.
- Add WS publish/fanout/deliver context injection.

### Phase 3: Helix Host/Driver Tracing

- Add host-layer OTel bootstrap from `helix.config.yaml`.
- Add span wrappers around pump dispatch, HTTP/WS/storage/event boundaries.
- Preserve `helix-core` zero-runtime dependency invariant.

### Phase 4: PC Tauri Trace Bridge

- Add profile JSON config fields.
- Add Angular/Tauri invoke sidecar.
- Add Tauri command/app emit/render spans.

### Phase 5: Mobile QuickJS/C++ Trace Bridge

- Add `config/mobile-local.json`.
- Add JS/C++/FFI bridge spans.
- Prefer additive `helix_command_with_trace` if ABI change is accepted.

### Phase 6: Golden E2E Trace

- Use `im.send(...)` as the first golden trace.
- Add a trace completeness gate equivalent to C015's hop coverage, but backed by Jaeger/OTel spans.

## 16. Open Decisions

1. Whether FFI v1 adds `helix_command_with_trace` immediately or uses a temporary local-root fallback.
2. Whether forced per-request debug tracing is accepted through `baggage`, custom header, or a config-side allowlist.
3. Whether production exports directly to an internal Collector per pod or through a node/local Collector.
4. Whether DB client auto-instrumentation is sufficient for some Go store spans, or whether store boundary spans remain explicit.

## 17. Design Invariants

- Trace is observability metadata, not business data.
- Config controls whether tracing is enabled; trace carriers only propagate context.
- Every async boundary that can break the chain has a span or span link.
- Missing one middle hop is a diagnostic bug, not a harmless logging gap.
- Each project can trace independently.
- OTel exporter failure must not break IM behavior.
