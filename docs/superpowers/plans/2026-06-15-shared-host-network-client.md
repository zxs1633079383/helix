# Shared Host Network Client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move HTTP and WebSocket client ownership into `helix-driver-host` so native and FFI reuse one shared network implementation.

**Architecture:** Add a shared `network` capsule in `helix-driver-host` containing config, header registry, HTTP requester, and WS transport. Keep `helix-driver-native` and `helix-driver-ffi` as thin shell wrappers. Do not change `helix-core` port traits or introduce runtime dependencies into `helix-core`.

**Tech Stack:** Rust, `tokio`, `reqwest`, `tokio-tungstenite`, `helix-core` ports, C ABI for FFI.

---

## File Structure

- Create `crates/helix-driver-host/src/network.rs`
  - Owns `HostNetworkConfig`, `HostHeaderRegistry`, `SharedHttpClient`, `SharedWsClient`, and `PreparedHttpRequest`.
- Modify `crates/helix-driver-host/src/lib.rs`
  - Re-export shared network types.
- Modify `crates/helix-driver-host/Cargo.toml`
  - Add network runtime dependencies in driver layer only.
- Create or update `crates/helix-driver-host/tests/network_shared.rs`
  - Tests host shared header and client behavior.
- Modify `crates/helix-driver-native/src/http.rs`
  - Convert `NativeHttp` to a wrapper around `SharedHttpClient`.
- Modify `crates/helix-driver-native/src/transport.rs`
  - Convert `NativeTransport` to a wrapper around `SharedWsClient`, preserving lifecycle ticks.
- Modify `crates/helix-driver-native/Cargo.toml`
  - Remove direct network deps if no longer needed by native.
- Modify `crates/helix-driver-ffi/src/transport.rs`
  - Make shared WS the default path and callback fallback explicit.
- Create `crates/helix-driver-ffi/src/http.rs`
  - Add `FfiHttpRequester` wrapper around `SharedHttpClient`.
- Modify `crates/helix-driver-ffi/src/api.rs`
  - Add opaque `HelixHandle` and header update C ABI.
- Modify `crates/helix-driver-ffi/src/callbacks.rs`
  - Make host-network callbacks optional fallback fields.
- Modify `crates/helix-driver-ffi/src/lib.rs`
  - Export new `http` module.
- Modify `crates/helix-driver-ffi/Cargo.toml`
  - Add `tokio` if required by header API tests or shared wrapper tests.

## Task 1: Shared Host Network Capsule

**Files:**
- Create: `crates/helix-driver-host/src/network.rs`
- Modify: `crates/helix-driver-host/src/lib.rs`
- Modify: `crates/helix-driver-host/Cargo.toml`
- Test: `crates/helix-driver-host/tests/network_shared.rs`

- [ ] **Step 1: Write failing host network tests**

Create tests covering:

```rust
#[tokio::test]
async fn header_registry_updates_and_removes_headers() { /* set, overwrite, remove, snapshot */ }

#[tokio::test]
async fn header_registry_rejects_invalid_header_names() { /* "bad header" -> PortError::Http */ }

#[tokio::test]
async fn shared_http_resolves_relative_urls_and_keeps_request_headers_explicit() { /* /messages + override */ }

#[tokio::test]
async fn shared_ws_send_before_connect_reports_transport_error() { /* disconnected send -> Transport */ }
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test -p helix-driver-host --test network_shared
```

Expected: compile failure because `HostNetworkConfig`, `HostHeaderRegistry`, `SharedHttpClient`, and `SharedWsClient` do not exist.

- [ ] **Step 3: Implement minimal shared network types**

Implement:

```rust
pub struct HostNetworkConfig {
    pub api_base_url: String,
    pub ws_url: String,
    pub timeout: Duration,
    pub user_agent: Option<String>,
}

#[derive(Clone, Default)]
pub struct HostHeaderRegistry { /* Arc<RwLock<HeaderMap>> */ }

#[derive(Clone)]
pub struct SharedHttpClient { /* reqwest client + config + registry */ }

pub struct SharedWsClient { /* config + registry + ConnState */ }
```

Backend integration note: for `cses-im-server`, platform shells should pass an HTTP base like `http://host:port/api/cses` and a complete WS URL like `ws://host:port/api/v4/websocket`. The shared client must not derive one from the other.

Required methods:

```rust
impl HostNetworkConfig {
    pub fn new(api_base_url: impl Into<String>, ws_url: impl Into<String>) -> Self;
    pub fn with_timeout(mut self, timeout: Duration) -> Self;
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self;
}

impl HostHeaderRegistry {
    pub async fn set_header(&self, name: &str, value: &str) -> Result<(), PortError>;
    pub async fn remove_header(&self, name: &str) -> Result<(), PortError>;
    pub async fn replace_headers<I, K, V>(&self, headers: I) -> Result<(), PortError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>;
    pub async fn snapshot(&self) -> HeaderMap;
}

impl SharedHttpClient {
    pub fn new(config: HostNetworkConfig) -> Result<Self, PortError>;
    pub fn with_registry(config: HostNetworkConfig, headers: HostHeaderRegistry) -> Result<Self, PortError>;
    pub fn headers(&self) -> HostHeaderRegistry;
    pub async fn prepare_for_test(&self, req: HttpRequest) -> Result<PreparedHttpRequest, PortError>;
}

impl SharedWsClient {
    pub fn new(config: HostNetworkConfig) -> Result<Self, PortError>;
    pub fn with_registry(config: HostNetworkConfig, headers: HostHeaderRegistry) -> Result<Self, PortError>;
    pub fn headers(&self) -> HostHeaderRegistry;
}
```

`SharedHttpClient` implements `HttpRequester`. `SharedWsClient` implements `Transport`.

- [ ] **Step 4: Verify GREEN**

Run:

```bash
cargo test -p helix-driver-host --test network_shared
cargo check -p helix-driver-host
```

Expected: tests pass and host crate checks.

## Task 2: Native Thin Wrappers

**Files:**
- Modify: `crates/helix-driver-native/src/http.rs`
- Modify: `crates/helix-driver-native/src/transport.rs`
- Modify: `crates/helix-driver-native/src/lib.rs` only if exports need adjustment
- Modify: `crates/helix-driver-native/Cargo.toml`

- [ ] **Step 1: Run GitNexus impact**

Run impact analysis for:

- `NativeHttp`
- `NativeTransport`

Stop and report if any result is HIGH or CRITICAL.

- [ ] **Step 2: Preserve existing native tests as RED/GREEN guard**

Run:

```bash
cargo test -p helix-driver-native test_send_on_disconnected_returns_err test_recv_on_disconnected_returns_none test_close_on_disconnected_is_ok
```

Expected before changes: pass. After wrapper changes: still pass.

- [ ] **Step 3: Convert `NativeHttp`**

Replace direct reqwest/header ownership with:

```rust
#[derive(Clone)]
pub struct NativeHttp {
    inner: SharedHttpClient,
}
```

Keep compatibility methods:

```rust
pub fn new() -> Result<Self, PortError>;
pub fn with_timeout(timeout: Duration) -> Result<Self, PortError>;
pub fn with_config(config: HostNetworkConfig) -> Result<Self, PortError>;
pub fn from_shared(inner: SharedHttpClient) -> Self;
pub async fn set_global_header(&self, name: &str, value: &str) -> Result<(), PortError>;
pub async fn remove_global_header(&self, name: &str) -> Result<(), PortError>;
```

Delegate `HttpRequester::request` to `inner.request(req).await`.
`new()` and `with_timeout()` must not hardcode backend URLs; they return a clear `PortError::Http` and callers should use `with_config`.

- [ ] **Step 4: Convert `NativeTransport`**

Replace direct `tokio_tungstenite` ownership with `SharedWsClient`.

Keep constructor:

```rust
pub fn new(url: impl Into<String>, id: TransportId, tick_tx: Option<mpsc::Sender<Tick>>) -> Self;
```

The `url` parameter is supplied by the platform shell/login module. Do not hardcode an HTTP or WS backend URL inside this constructor.

Add constructors:

```rust
pub fn with_config(config: HostNetworkConfig, id: TransportId, tick_tx: Option<mpsc::Sender<Tick>>) -> Result<Self, PortError>;
pub fn from_shared(inner: SharedWsClient, id: TransportId, tick_tx: Option<mpsc::Sender<Tick>>) -> Self;
```

Preserve lifecycle tick behavior:

- `connect()` emits `Tick::Connected(id)` after shared connect succeeds.
- `close()` emits `Tick::Disconnected(id)`.
- `recv()` emits `Tick::Disconnected(id)` when shared recv returns `Ok(None)` after a connection was open.

- [ ] **Step 5: Verify native**

Run:

```bash
cargo test -p helix-driver-native test_send_on_disconnected_returns_err test_recv_on_disconnected_returns_none test_close_on_disconnected_is_ok
cargo check -p helix-driver-native
```

Expected: pass.

## Task 3: FFI Shared Network API

**Files:**
- Modify: `crates/helix-driver-ffi/src/transport.rs`
- Create: `crates/helix-driver-ffi/src/http.rs`
- Modify: `crates/helix-driver-ffi/src/api.rs`
- Modify: `crates/helix-driver-ffi/src/callbacks.rs`
- Modify: `crates/helix-driver-ffi/src/lib.rs`
- Modify: `crates/helix-driver-ffi/Cargo.toml`

- [ ] **Step 1: Run GitNexus impact**

Run impact analysis for:

- `FfiTransport`
- `HelixCallbacks`
- `helix_create`

Stop and report if any result is HIGH or CRITICAL.

- [ ] **Step 2: Write failing FFI API tests**

Add tests in `api.rs` under `#[cfg(test)]`:

```rust
#[test]
fn set_header_rejects_null_handle() { /* helix_set_header(null, ...) < 0 */ }

#[test]
fn create_with_network_allows_set_and_remove_header() { /* create, set, remove, destroy */ }

#[test]
fn set_header_rejects_invalid_utf8() { /* invalid bytes -> negative */ }
```

- [ ] **Step 3: Verify RED**

Run:

```bash
cargo test -p helix-driver-ffi api::
```

Expected: compile failure because `helix_create_with_network`, `helix_set_header`, and `helix_remove_header` do not exist.

- [ ] **Step 4: Implement FFI shared wrappers**

Implement product path:

```rust
pub struct FfiTransport {
    mode: FfiTransportMode,
}

enum FfiTransportMode {
    Shared(SharedWsClient),
    Callback { callbacks: Arc<HelixCallbacks>, inbound_rx: futures_channel::mpsc::Receiver<Bytes> },
}
```

Implement `FfiHttpRequester`:

```rust
#[derive(Clone)]
pub struct FfiHttpRequester {
    inner: SharedHttpClient,
}
```

Add `HelixHandle`:

```rust
pub struct HelixHandle {
    headers: HostHeaderRegistry,
    http: SharedHttpClient,
    ws: Option<SharedWsClient>,
    callbacks: HelixCallbacks,
}
```

Add C ABI:

```rust
#[no_mangle]
pub unsafe extern "C" fn helix_create_with_network(
    api_base_url: *const u8,
    api_base_url_len: usize,
    ws_url: *const u8,
    ws_url_len: usize,
    callbacks: HelixCallbacks,
) -> *mut HelixHandle;

#[no_mangle]
pub unsafe extern "C" fn helix_set_header(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
    value: *const u8,
    value_len: usize,
) -> i32;

#[no_mangle]
pub unsafe extern "C" fn helix_remove_header(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
) -> i32;
```

Status codes:

- `0` success.
- `-1` null handle or null required pointer.
- `-2` invalid UTF-8.
- `-3` invalid network config or header validation.

- [ ] **Step 5: Verify FFI**

Run:

```bash
cargo test -p helix-driver-ffi api::
cargo check -p helix-driver-ffi
```

Expected: pass.

## Task 4: Documentation and Final Verification

**Files:**
- Modify docs only if code behavior differs from `docs/02-driver-native.md`, `docs/04-driver-ffi.md`, or `docs/ROADMAP.md`.

- [ ] **Step 1: Run formatting**

Run:

```bash
cargo fmt
```

- [ ] **Step 2: Run required cargo checks**

Run:

```bash
cargo check -p helix-driver-host
cargo check -p helix-driver-native
cargo check -p helix-driver-ffi
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo test -p helix-core
```

- [ ] **Step 3: Run core dependency gate**

Run:

```bash
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
```

Expected: stdout is empty. Exit code `1` is acceptable only with empty stdout.

- [ ] **Step 4: Run GitNexus detect_changes**

Run:

```text
detect_changes({ scope: "all", repo: "helix" })
```

Expected: changed symbols and affected flows match shared host network/client wrapper scope.

- [ ] **Step 5: Summarize completed and deferred scope**

Report:

- Shared host network API completed.
- Native wrapper completed.
- FFI header API completed.
- Any deferred work, especially full reconnect/backoff, proxy/TLS customization, or callback HTTP fallback, with reason.

## Self-Review

- The plan covers every requirement in the design spec.
- The plan has explicit files, tests, commands, and expected results.
- The plan preserves TDD red/green steps for new behavior.
- The plan includes GitNexus impact before modifying existing symbols and `detect_changes` before completion.
- The plan avoids wasm work and core port changes.
