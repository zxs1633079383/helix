# Shared Host Network Client Design

## Context

Helix currently targets PC Tauri (`helix-driver-native`) and Flutter FFI (`helix-driver-ffi`). Web/wasm is archived by ADR-006 and is not part of this work. ADR-008 says HTTP and WebSocket clients are shared host I/O capabilities, not IM logic and not per-platform implementations.

Existing code has these gaps:

- `helix-driver-host` owns shared storage only.
- `helix-driver-native` owns its own `reqwest` and `tokio-tungstenite` clients.
- `helix-driver-ffi` still has callback-first transport stubs and no shared HTTP requester path.
- Header update exists only inside `NativeHttp`, so FFI cannot share the same token/header injection behavior.

## Goals

- Add one shared network implementation in `helix-driver-host`.
- Keep `helix-core` sans-IO and free of `tokio`, `reqwest`, `tokio-tungstenite`, `tauri`, and `web-sys`.
- Keep `helix-im` and future modules unaware of tokens, headers, login, and network client ownership.
- Make native and FFI thin shells over the shared host client.
- Preserve FFI host-network callbacks only as an escape hatch, not as the product default.
- Provide Rust and C ABI header update APIs.

## Non-Goals

- Do not implement login, token acquisition, token refresh, or auth state machines.
- Do not implement full reconnect/backoff policy beyond the existing `Transport` connect/send/recv/close surface.
- Do not modify `helix-core` port trait signatures.
- Do not reactivate or fix `helix-driver-wasm`.
- Do not move IM sync/cursor/echo/message SQL into frontend, Dart, Tauri UI, or core.

## Architecture

`helix-driver-host` gains a `network` module with these units:

- `HostNetworkConfig`: owned runtime configuration supplied by the platform shell. It contains `api_base_url`, `ws_url`, request timeout, optional user agent, and any future platform-safe network knobs.
  - Current Go backend integration convention: `api_base_url` points at the CSES HTTP root such as `http://host:port/api/cses`; `ws_url` is a complete WebSocket endpoint and, for the existing production service, uses `/api/v4/websocket`.
- `HostHeaderRegistry`: shared mutable header registry backed by an async lock. It supports `set_header`, `remove_header`, `replace_headers`, and `snapshot`. It validates header names and values using the concrete HTTP stack types.
- `SharedHttpClient`: implements `helix_core::ports::HttpRequester` using `reqwest`. It combines the latest header registry snapshot with request-level headers. Request-level headers override registry headers. Relative HTTP URLs are resolved against `api_base_url`.
- `SharedWsClient`: implements `helix_core::ports::Transport` using `tokio-tungstenite`. It reads the latest header registry snapshot at `connect()` time so reconnects use refreshed headers. It sends binary frames, converts text frames to bytes, and returns `None` on close.
- `PreparedHttpRequest` test helper: crate-public or public test-friendly structure used by tests to verify URL/header merging without making network calls.

Dependency direction remains:

```text
helix-driver-native ─┐
                     ├─> helix-driver-host ─> helix-core
helix-driver-ffi ────┘
```

`helix-driver-host` may depend on `tokio`, `reqwest`, and `tokio-tungstenite` because it is a driver-layer crate. `helix-core` must not gain any of those dependencies.

## Native Changes

`NativeHttp` becomes a compatibility wrapper around `SharedHttpClient`:

- `NativeHttp::new()` and `NativeHttp::with_timeout()` remain only as legacy compatibility methods and return an error without external config.
- `NativeHttp::with_config(config)` and `NativeHttp::from_shared(shared)` are added.
- `set_global_header` and `remove_global_header` delegate to the shared registry.
- `HttpRequester for NativeHttp` delegates to `SharedHttpClient`.

`NativeTransport` becomes a wrapper around `SharedWsClient` while preserving native-specific lifecycle ticks:

- Constructor stays `NativeTransport::new(url, id, tick_tx)`.
- Its `url` argument is the external WS URL; it must not derive or hardcode a backend URL.
- New constructors allow passing a `HostNetworkConfig` or `SharedWsClient`.
- `connect()` delegates to shared WS, then emits `Tick::Connected(id)`.
- `recv()` delegates to shared WS; when it observes close, it emits `Tick::Disconnected(id)` once for that close.
- `send()` and `close()` delegate to shared WS.

Native remains responsible for Tauri shell duties, engine loop wiring, paths, lifecycle, and feature flags. It does not own a separate HTTP/WS implementation.

## FFI Changes

`FfiTransport` defaults to `SharedWsClient`:

- `FfiTransport::new(shared)` uses the product path.
- `FfiTransport::from_callbacks(callbacks)` remains as a host-network fallback.
- Fallback callbacks are optional and must return `PortError::Transport` when missing.

Add `FfiHttpRequester`:

- Product path delegates to `SharedHttpClient`.
- Optional callback fallback can be added later only if a platform network stack is required.

Add an opaque `HelixHandle` in `api.rs` for header registry ownership:

- `helix_create_with_network(api_base_url, ws_url, callbacks) -> *mut HelixHandle`
- `helix_set_header(handle, name_ptr, name_len, value_ptr, value_len) -> i32`
- `helix_remove_header(handle, name_ptr, name_len) -> i32`
- `helix_destroy(handle)`

FFI ABI rules:

- All C buffers are copied before returning.
- Null handles or invalid UTF-8 return negative status codes.
- Header validation failures return negative status codes and never panic.
- `helix_create(callbacks)` may remain as a legacy stub for now, but the documented product path is `helix_create_with_network`.

## Header Semantics

- Header names and values are supplied by login/platform code.
- Helix does not acquire, refresh, parse, or persist tokens.
- Shared HTTP request and WS connect read the latest registry snapshot.
- Request-level HTTP headers override registry headers.
- Removing a header affects future HTTP requests and future WS connects/reconnects, not already-open WS handshakes.

## Error Semantics

- Invalid config or invalid headers map to `PortError::Http` for HTTP client setup and header APIs.
- WS connection, send, receive, and close errors map to `PortError::Transport`.
- HTTP timeout maps to `PortError::Transport("timeout: ...")`.
- HTTP network errors map to `PortError::Transport("network: ...")`.
- HTTP 4xx/5xx statuses return `Ok(HttpResponse)` so business modules decide behavior.

## Tests

Host network tests:

- Header registry set/remove/replace/snapshot.
- Invalid header name/value returns `PortError::Http`.
- HTTP preparation resolves relative URLs against `api_base_url`.
- Request-level header overrides registry header.
- WS send before connect returns `PortError::Transport`.

Native tests:

- Existing disconnected send/recv/close tests still pass.
- `NativeHttp` header methods delegate to shared registry.
- `NativeTransport` disconnected behavior remains compatible.

FFI tests:

- Header API rejects null handle/null name.
- Header API accepts valid UTF-8 buffers and updates registry.
- Remove header succeeds idempotently.

## Verification

Required commands:

```bash
cargo check -p helix-driver-host
cargo check -p helix-driver-native
cargo check -p helix-driver-ffi
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo test -p helix-core
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
```

The final command must produce no output. Because `grep` exits `1` when there are no matches, that exit code is expected only when stdout is empty.

GitNexus:

- Run impact analysis before modifying existing symbols.
- Run `detect_changes()` after implementation and before any commit/PR handoff.

## Risks

- `SharedWsClient` lives in `helix-driver-host`, so that crate now owns async network runtime dependencies. This is allowed by ADR-008 but must not leak into `helix-core`.
- Existing `NativeTransport` emits lifecycle ticks; a naive wrapper could drop those events. The wrapper must preserve tick emission.
- FFI ABI additions must not panic on bad pointers or invalid UTF-8.
- `reqwest` URL joining has edge cases. The shared client should implement simple and explicit relative path resolution for `/path` and absolute URLs.

## Self-Review

- No placeholder requirements remain.
- Scope is limited to shared host network clients and shell wrappers.
- The design does not introduce login/token ownership into Helix.
- The design does not modify core port traits or core dependencies.
- The design keeps wasm archived and out of verification.
