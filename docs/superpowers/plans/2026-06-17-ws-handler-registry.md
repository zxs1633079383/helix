# WS Handler Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `helix-im` WebSocket inbound `action` dispatch with a `linkme`-backed `action -> handler` registry so new WS message types can be added without changing the central dispatcher.

**Architecture:** Keep the registry entirely inside `helix-im`; `helix-core` and drivers remain unchanged. Use `linkme::distributed_slice` for distributed static registration, a `OnceLock<HashMap<...>>` lookup for dispatch, and a thin `ImWsContext` for handler state access.

**Tech Stack:** Rust 2021, `linkme` 0.3.35, `serde_json`, existing `helix-core` `Tick`/`Effect` model, existing `helix-im` replay tests.

## Current Interface Truth After Task 2 Review

The original planning snippets below still contain some early function-pointer
sketches. The implemented and reviewed Task 2 interface is authoritative for
Tasks 2 through 7:

- `WsMessageHandler` is the strategy interface:
  - `action(&self) -> &'static str`
  - `handle(&self, ctx: &mut ImWsContext<'_>, frame: &WsFrame, out: &mut EffectSink) -> Result<(), ImError>`
- `WsHandlerRegistration` stores `action` plus `&'static dyn WsMessageHandler`.
  `build_registry` must reject duplicate actions and reject
  `registration.action != handler.action()`.
- `dispatch_ws(ctx, frame, out)` receives a parsed `WsFrame` and writes effects
  into the same deterministic `EffectSink` used by `Module::handle`.
- `EffectSink` is allowed in handler signatures because it only collects
  `Effect` values for replay and driver batching; it is not the `EventSink`
  port and must not be confused with direct event emission.
- `ImWsContext` must not expose `EventSink`; handlers emit by pushing
  `Effect::Emit` into `EffectSink`, never by calling `EventSink::emit`.
- `ImWsContext` currently holds `&mut ImState` plus deterministic `now_ms`.
- `WsFrame::action()` reads only top-level `action`; frames without top-level
  `action` map to `__legacy_event__`.
- `WsFrame::event_seq()` keeps the existing real-wire priority first:
  `data.event_seq` -> `data.props.channel_event_seq` ->
  top-level `channel_event_seq` -> `data.seq`, then compatibility fallbacks
  `eventSeq` -> `event.seq` -> `payload.eventSeq`.

---

## File Structure

- Modify `crates/helix-im/Cargo.toml`
  - Add `linkme = "0.3.35"`.
- Modify `crates/helix-im/src/lib.rs`
  - Add `pub(crate) mod ws;`.
- Modify `crates/helix-im/src/error.rs`
  - Add explicit WS dispatch errors.
- Create `crates/helix-im/src/ws/mod.rs`
  - Re-export frame, registry, context, handlers, and expose `dispatch_ws`.
- Create `crates/helix-im/src/ws/frame.rs`
  - Own raw inbound bytes and parsed JSON envelope.
- Create `crates/helix-im/src/ws/registry.rs`
  - Declare `WS_HANDLERS`, build lookup map, enforce unique action keys.
- Create `crates/helix-im/src/ws/context.rs`
  - Provide handler-facing context over `ImModule` and `now_ms`.
- Create `crates/helix-im/src/ws/handlers/mod.rs`
  - Declare handler modules.
- Create `crates/helix-im/src/ws/handlers/ignored.rs`
  - Explicitly consume known operational/noise actions.
- Create `crates/helix-im/src/ws/handlers/hello.rs`
  - Register and handle `hello`.
- Create `crates/helix-im/src/ws/handlers/increment_channel.rs`
  - Register and handle `increment_channel`.
- Create `crates/helix-im/src/ws/handlers/increment_channel_end.rs`
  - Register and handle `increment_channel_end`.
- Create `crates/helix-im/src/ws/handlers/posted.rs`
  - Register and handle `posted` echo only.
- Create `crates/helix-im/src/ws/handlers/legacy_event.rs`
  - Register and preserve legacy no-action frames.
- Modify `crates/helix-im/src/module.rs`
  - Route `Tick::Inbound` through `ws::dispatch_ws`.
  - Make current private helpers available to `ImWsContext`.
- Modify `crates/helix-im/src/parser.rs`
  - Keep sync/field parsing helpers; shrink or remove `FrameClass`/`classify_frame` after migration.
- Add or modify unit tests in new `ws` modules.
- Run existing integration tests:
  - `cargo test -p helix-im`
  - `cargo test -p helix-im --test real_go_frames_test`
  - `cargo test -p helix-im --test echo_reconcile_test`
  - `scripts/gate.sh`

## Task 1: Add Dependency, Errors, Frame, Registry Skeleton

**Files:**
- Modify: `crates/helix-im/Cargo.toml`
- Modify: `crates/helix-im/src/lib.rs`
- Modify: `crates/helix-im/src/error.rs`
- Create: `crates/helix-im/src/ws/mod.rs`
- Create: `crates/helix-im/src/ws/frame.rs`
- Create: `crates/helix-im/src/ws/registry.rs`
- Create: `crates/helix-im/src/ws/context.rs`

- [ ] **Step 1: Add the dependency**

Edit `crates/helix-im/Cargo.toml` dependencies:

```toml
[dependencies]
helix-core = { path = "../helix-core" }
serde       = { workspace = true }
serde_json  = { workspace = true }
thiserror   = { workspace = true }
bytes       = { workspace = true }
tracing     = { workspace = true }
linkme      = "0.3.35"
```

- [ ] **Step 2: Expose the internal ws module**

Edit `crates/helix-im/src/lib.rs`:

```rust
pub mod acl;
pub mod channel;
pub mod error;
pub mod module;
pub mod parser;
pub mod pending_send;
pub mod schema;
pub mod state;
pub mod sync_session;
pub(crate) mod ws;
```

- [ ] **Step 3: Add WS errors**

Edit `crates/helix-im/src/error.rs`:

```rust
#[derive(Debug, Error)]
pub enum ImError {
    #[error("parse error: {0}")]
    Parse(String),

    #[error("invalid seq: expected {expected}, got {got}")]
    InvalidSeq { expected: u64, got: u64 },

    #[error("unknown channel: {0}")]
    UnknownChannel(u64),

    #[error("unknown correlation: {0}")]
    UnknownCorrelation(u64),

    #[error("unsupported websocket action: {0}")]
    UnsupportedWsAction(String),

    #[error("duplicate websocket action handler: {0}")]
    DuplicateWsAction(String),

    #[error("invalid websocket frame: {0}")]
    InvalidWsFrame(String),
}
```

- [ ] **Step 4: Create `ws/mod.rs`**

Create `crates/helix-im/src/ws/mod.rs`:

```rust
mod context;
mod frame;
mod registry;

pub(crate) mod handlers;

pub(crate) use context::ImWsContext;
pub(crate) use frame::{WsFrame, LEGACY_EVENT_ACTION};
pub(crate) use registry::{dispatch_ws, WsActionKind, WsHandlerEntry, WsHandlerFn, WS_HANDLERS};
```

- [ ] **Step 5: Create the frame model with tests**

Create `crates/helix-im/src/ws/frame.rs`:

```rust
use bytes::Bytes;

use crate::error::ImError;
use crate::state::Seq;

pub(crate) const LEGACY_EVENT_ACTION: &str = "__legacy_event__";

pub(crate) struct WsFrame {
    raw: Bytes,
    root: serde_json::Value,
    action: String,
}

impl WsFrame {
    pub(crate) fn parse(raw: &[u8]) -> Result<Self, ImError> {
        let root: serde_json::Value = serde_json::from_slice(raw)
            .map_err(|e| ImError::InvalidWsFrame(format!("invalid JSON: {e}")))?;
        let action = root
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or(LEGACY_EVENT_ACTION)
            .to_string();
        Ok(Self {
            raw: Bytes::copy_from_slice(raw),
            root,
            action,
        })
    }

    pub(crate) fn action(&self) -> &str {
        &self.action
    }

    pub(crate) fn raw(&self) -> &[u8] {
        self.raw.as_ref()
    }

    pub(crate) fn root(&self) -> &serde_json::Value {
        &self.root
    }

    pub(crate) fn data(&self) -> Option<&serde_json::Value> {
        self.root.get("data")
    }

    pub(crate) fn data_required(&self) -> Result<&serde_json::Value, ImError> {
        self.data()
            .ok_or_else(|| ImError::InvalidWsFrame("missing data".to_string()))
    }

    pub(crate) fn event_seq(&self) -> Option<Seq> {
        let data = self.data()?;
        data.get("event_seq")
            .and_then(|v| v.as_u64())
            .or_else(|| {
                data.get("props")
                    .and_then(|p| p.get("channel_event_seq"))
                    .and_then(|v| v.as_u64())
            })
            .or_else(|| self.root.get("channel_event_seq").and_then(|v| v.as_u64()))
            .map(Seq)
    }

    pub(crate) fn event_seq_required(&self) -> Result<Seq, ImError> {
        self.event_seq()
            .ok_or_else(|| ImError::InvalidWsFrame("missing event_seq".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_action_frame_reads_top_level_action() {
        let frame = WsFrame::parse(br#"{"action":"hello","data":{"connectionId":"c1"}}"#)
            .expect("valid frame");
        assert_eq!(frame.action(), "hello");
        assert!(frame.data().is_some());
    }

    #[test]
    fn parse_legacy_frame_uses_legacy_action_key() {
        let frame = WsFrame::parse(br#"{"event":"posted","data":{}}"#).expect("valid frame");
        assert_eq!(frame.action(), LEGACY_EVENT_ACTION);
    }

    #[test]
    fn parse_invalid_json_returns_invalid_ws_frame() {
        let err = WsFrame::parse(b"{").expect_err("invalid JSON should fail");
        assert!(matches!(err, ImError::InvalidWsFrame(_)));
    }

    #[test]
    fn event_seq_uses_data_then_props_then_root() {
        let frame = WsFrame::parse(br#"{"action":"posted","data":{"event_seq":7}}"#)
            .expect("valid frame");
        assert_eq!(frame.event_seq(), Some(Seq(7)));

        let frame = WsFrame::parse(
            br#"{"action":"posted","data":{"props":{"channel_event_seq":8}}}"#,
        )
        .expect("valid frame");
        assert_eq!(frame.event_seq(), Some(Seq(8)));

        let frame = WsFrame::parse(br#"{"action":"posted","channel_event_seq":9,"data":{}}"#)
            .expect("valid frame");
        assert_eq!(frame.event_seq(), Some(Seq(9)));
    }
}
```

- [ ] **Step 6: Create context wrapper**

Create `crates/helix-im/src/ws/context.rs`:

```rust
use crate::module::ImModule;

pub(crate) struct ImWsContext<'a> {
    pub(crate) module: &'a mut ImModule,
    pub(crate) now_ms: u64,
}

impl<'a> ImWsContext<'a> {
    pub(crate) fn new(module: &'a mut ImModule, now_ms: u64) -> Self {
        Self { module, now_ms }
    }
}
```

- [ ] **Step 7: Create registry skeleton with unit tests**

Create `crates/helix-im/src/ws/registry.rs`:

```rust
use std::collections::HashMap;
use std::sync::OnceLock;

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::context::ImWsContext;
use super::frame::WsFrame;

pub(crate) type WsHandlerFn =
    fn(&mut ImWsContext<'_>, &WsFrame, &mut EffectSink) -> Result<(), ImError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WsActionKind {
    Business,
    Ignored,
}

pub(crate) struct WsHandlerEntry {
    pub(crate) action: &'static str,
    pub(crate) kind: WsActionKind,
    pub(crate) handler: WsHandlerFn,
}

#[distributed_slice]
pub(crate) static WS_HANDLERS: [WsHandlerEntry];

static WS_HANDLER_MAP: OnceLock<Result<HashMap<&'static str, WsHandlerFn>, ImError>> =
    OnceLock::new();

pub(crate) fn registry() -> Result<&'static HashMap<&'static str, WsHandlerFn>, ImError> {
    match WS_HANDLER_MAP.get_or_init(|| build_registry(WS_HANDLERS)) {
        Ok(map) => Ok(map),
        Err(err) => Err(ImError::DuplicateWsAction(err.to_string())),
    }
}

fn build_registry(
    entries: &'static [WsHandlerEntry],
) -> Result<HashMap<&'static str, WsHandlerFn>, ImError> {
    let mut map = HashMap::with_capacity(entries.len());
    for entry in entries {
        if map.insert(entry.action, entry.handler).is_some() {
            return Err(ImError::DuplicateWsAction(entry.action.to_string()));
        }
    }
    Ok(map)
}

pub(crate) fn dispatch_ws(
    ctx: &mut ImWsContext<'_>,
    raw: &[u8],
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let frame = WsFrame::parse(raw)?;
    let handler = registry()?
        .get(frame.action())
        .copied()
        .ok_or_else(|| ImError::UnsupportedWsAction(frame.action().to_string()))?;
    handler(ctx, &frame, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop(
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        Ok(())
    }

    #[test]
    fn build_registry_rejects_duplicate_actions() {
        static ENTRIES: &[WsHandlerEntry] = &[
            WsHandlerEntry {
                action: "hello",
                kind: WsActionKind::Business,
                handler: noop,
            },
            WsHandlerEntry {
                action: "hello",
                kind: WsActionKind::Ignored,
                handler: noop,
            },
        ];

        let err = build_registry(ENTRIES).expect_err("duplicate action should fail");
        assert!(matches!(err, ImError::DuplicateWsAction(action) if action == "hello"));
    }

    #[test]
    fn distributed_registry_has_unique_actions() {
        build_registry(WS_HANDLERS).expect("registered WS actions must be unique");
    }
}
```

- [ ] **Step 8: Run skeleton tests**

Run:

```bash
cargo test -p helix-im ws::
```

Expected:

```text
test result: ok
```

- [ ] **Step 9: Commit skeleton**

```bash
git add crates/helix-im/Cargo.toml crates/helix-im/src/lib.rs crates/helix-im/src/error.rs crates/helix-im/src/ws
git commit -m "feat(im): 添加 WS handler 注册表骨架"
```

## Task 2: Wire Dispatcher Into `ImModule` With Legacy Handler

**Files:**
- Modify: `crates/helix-im/src/module.rs`
- Modify: `crates/helix-im/src/ws/context.rs`
- Create: `crates/helix-im/src/ws/handlers/mod.rs`
- Create: `crates/helix-im/src/ws/handlers/legacy_event.rs`

- [ ] **Step 1: Add handler module declarations**

Create `crates/helix-im/src/ws/handlers/mod.rs`:

```rust
pub(crate) mod legacy_event;
```

- [ ] **Step 2: Move legacy inbound body behind a module method**

In `crates/helix-im/src/module.rs`, add this method near existing inbound helpers:

```rust
pub(crate) fn handle_legacy_inbound(
    &mut self,
    raw: &[u8],
    now_ms: u64,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let tick = Tick::Inbound(helix_core::tick::InboundBytes(bytes::Bytes::copy_from_slice(raw)));
    match acl::from_tick::parse_inbound_tick(&tick) {
        Ok(ev) => {
            if let Some(ch) = self.state.channels.get_mut(&ev.channel_id) {
                ch.ingest(ev, out, now_ms)?;
            }
            Ok(())
        }
        Err(ImError::Parse(msg)) => {
            tracing::debug!("im: inbound parse error: {}", msg);
            Ok(())
        }
        Err(e) => Err(e),
    }
}
```

- [ ] **Step 3: Add legacy handler**

Add this method to `crates/helix-im/src/ws/context.rs`:

```rust
use helix_core::EffectSink;

use crate::error::ImError;

impl<'a> ImWsContext<'a> {
    pub(crate) fn handle_legacy_inbound(
        &mut self,
        raw: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.module.handle_legacy_inbound(raw, self.now_ms, out)
    }
}
```

- [ ] **Step 4: Add legacy handler**

Create `crates/helix-im/src/ws/handlers/legacy_event.rs`:

```rust
use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::ws::frame::LEGACY_EVENT_ACTION;
use crate::ws::registry::{WsActionKind, WsHandlerEntry, WS_HANDLERS};
use crate::ws::{ImWsContext, WsFrame};

fn handle_legacy_event(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    ctx.handle_legacy_inbound(frame.raw(), out)
}

#[distributed_slice(WS_HANDLERS)]
static LEGACY_EVENT: WsHandlerEntry = WsHandlerEntry {
    action: LEGACY_EVENT_ACTION,
    kind: WsActionKind::Business,
    handler: handle_legacy_event,
};
```

- [ ] **Step 5: Replace only the legacy path in `Tick::Inbound`**

In `crates/helix-im/src/module.rs`, replace the current `Tick::Inbound(bytes) => { ... }`
body with dispatcher wiring:

```rust
Tick::Inbound(bytes) => {
    let mut ctx = crate::ws::ImWsContext::new(self, now_ms);
    crate::ws::dispatch_ws(&mut ctx, bytes.as_bytes(), out).map_err(|e| {
        CoreError::ModuleError {
            module: "helix-im",
            source: Box::new(e),
        }
    })?;
}
```

At this point, action-bearing frames will return `UnsupportedWsAction` until their
handlers are migrated in later tasks.

- [ ] **Step 6: Run legacy replay tests**

Run:

```bash
cargo test -p helix-im --test gate_test
cargo test -p helix-im --test proptest_gate
```

Expected:

```text
test result: ok
```

- [ ] **Step 7: Commit legacy dispatch wiring**

```bash
git add crates/helix-im/src/module.rs crates/helix-im/src/ws/handlers
git commit -m "feat(im): 将 legacy WS 帧接入 handler registry"
```

## Task 3: Migrate `hello` Handler

**Files:**
- Modify: `crates/helix-im/src/module.rs`
- Modify: `crates/helix-im/src/ws/context.rs`
- Create: `crates/helix-im/src/ws/handlers/hello.rs`
- Modify: `crates/helix-im/src/ws/handlers/mod.rs`

- [ ] **Step 1: Make `handle_hello` visible inside crate**

Change the method signature in `crates/helix-im/src/module.rs`:

```rust
pub(crate) fn handle_hello(&mut self, connection_id: String, out: &mut EffectSink) {
```

- [ ] **Step 2: Register hello handler module**

Add this method to `crates/helix-im/src/ws/context.rs`:

```rust
impl<'a> ImWsContext<'a> {
    pub(crate) fn handle_hello(&mut self, connection_id: String, out: &mut EffectSink) {
        self.module.handle_hello(connection_id, out);
    }
}
```

- [ ] **Step 3: Register hello handler module**

Edit `crates/helix-im/src/ws/handlers/mod.rs`:

```rust
pub(crate) mod hello;
pub(crate) mod legacy_event;
```

- [ ] **Step 4: Add hello handler**

Create `crates/helix-im/src/ws/handlers/hello.rs`:

```rust
use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::ws::registry::{WsActionKind, WsHandlerEntry, WS_HANDLERS};
use crate::ws::{ImWsContext, WsFrame};

fn handle_hello(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let connection_id = frame
        .data_required()?
        .get("connectionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ImError::InvalidWsFrame("hello missing data.connectionId".to_string()))?
        .to_string();
    ctx.handle_hello(connection_id, out);
    Ok(())
}

#[distributed_slice(WS_HANDLERS)]
static HELLO: WsHandlerEntry = WsHandlerEntry {
    action: "hello",
    kind: WsActionKind::Business,
    handler: handle_hello,
};
```

- [ ] **Step 5: Run hello lifecycle tests**

Run:

```bash
cargo test -p helix-im --test lifecycle_test
cargo test -p helix-im --test increment_flow_test
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit hello migration**

```bash
git add crates/helix-im/src/module.rs crates/helix-im/src/ws/handlers
git commit -m "feat(im): 迁移 hello WS handler"
```

## Task 4: Migrate Increment Handlers

**Files:**
- Modify: `crates/helix-im/src/module.rs`
- Modify: `crates/helix-im/src/ws/context.rs`
- Create: `crates/helix-im/src/ws/handlers/increment_channel.rs`
- Create: `crates/helix-im/src/ws/handlers/increment_channel_end.rs`
- Modify: `crates/helix-im/src/ws/handlers/mod.rs`

- [ ] **Step 1: Make increment helpers visible inside crate**

Change signatures in `crates/helix-im/src/module.rs`:

```rust
pub(crate) fn handle_increment(
    &mut self,
    inc: &crate::sync_session::IncrementChannel,
    out: &mut EffectSink,
) {
```

```rust
pub(crate) fn handle_increment_end(&mut self, ch: Option<ChannelId>, out: &mut EffectSink) {
```

- [ ] **Step 2: Add increment methods to context**

Add these imports and methods to `crates/helix-im/src/ws/context.rs`:

```rust
use crate::state::ChannelId;
use crate::sync_session::IncrementChannel;

impl<'a> ImWsContext<'a> {
    pub(crate) fn handle_increment(&mut self, inc: &IncrementChannel, out: &mut EffectSink) {
        self.module.handle_increment(inc, out);
    }

    pub(crate) fn handle_increment_end(&mut self, ch: Option<ChannelId>, out: &mut EffectSink) {
        self.module.handle_increment_end(ch, out);
    }
}
```

- [ ] **Step 3: Register increment handler modules**

Edit `crates/helix-im/src/ws/handlers/mod.rs`:

```rust
pub(crate) mod hello;
pub(crate) mod increment_channel;
pub(crate) mod increment_channel_end;
pub(crate) mod legacy_event;
```

- [ ] **Step 4: Add increment channel handler**

Create `crates/helix-im/src/ws/handlers/increment_channel.rs`:

```rust
use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::parser::parse_increment_channel;
use crate::ws::registry::{WsActionKind, WsHandlerEntry, WS_HANDLERS};
use crate::ws::{ImWsContext, WsFrame};

fn handle_increment_channel(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let inc = parse_increment_channel(frame.data_required()?).ok_or_else(|| {
        ImError::InvalidWsFrame("invalid increment_channel data".to_string())
    })?;
    ctx.handle_increment(&inc, out);
    Ok(())
}

#[distributed_slice(WS_HANDLERS)]
static INCREMENT_CHANNEL: WsHandlerEntry = WsHandlerEntry {
    action: "increment_channel",
    kind: WsActionKind::Business,
    handler: handle_increment_channel,
};
```

- [ ] **Step 5: Add increment end handler**

Create `crates/helix-im/src/ws/handlers/increment_channel_end.rs`:

```rust
use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::state::ChannelId;
use crate::ws::registry::{WsActionKind, WsHandlerEntry, WS_HANDLERS};
use crate::ws::{ImWsContext, WsFrame};

fn handle_increment_channel_end(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let ch = frame
        .data()
        .and_then(|d| d.get("channelId"))
        .and_then(|v| v.as_str())
        .and_then(ChannelId::from_str);
    ctx.handle_increment_end(ch, out);
    Ok(())
}

#[distributed_slice(WS_HANDLERS)]
static INCREMENT_CHANNEL_END: WsHandlerEntry = WsHandlerEntry {
    action: "increment_channel_end",
    kind: WsActionKind::Business,
    handler: handle_increment_channel_end,
};
```

- [ ] **Step 6: Run increment tests**

Run:

```bash
cargo test -p helix-im --test increment_parse_test
cargo test -p helix-im --test increment_flow_test
cargo test -p helix-im --test b2_increment_heal_test
```

Expected:

```text
test result: ok
```

- [ ] **Step 7: Commit increment migration**

```bash
git add crates/helix-im/src/module.rs crates/helix-im/src/ws/handlers
git commit -m "feat(im): 迁移 increment WS handlers"
```

## Task 5: Migrate Posted Echo Handler

**Files:**
- Modify: `crates/helix-im/src/module.rs`
- Modify: `crates/helix-im/src/ws/context.rs`
- Create: `crates/helix-im/src/ws/handlers/posted.rs`
- Modify: `crates/helix-im/src/ws/handlers/mod.rs`

- [ ] **Step 1: Make `handle_echo` visible inside crate**

Change the method signature in `crates/helix-im/src/module.rs`:

```rust
pub(crate) fn handle_echo(
    &mut self,
    channel_id: ChannelId,
    seq: crate::state::Seq,
    tmp_id: &str,
    server_id: crate::state::ServerId,
    out: &mut EffectSink,
) -> Result<(), ImError> {
```

- [ ] **Step 2: Add echo method to context**

Add these imports and method to `crates/helix-im/src/ws/context.rs`:

```rust
use crate::state::{Seq, ServerId};

impl<'a> ImWsContext<'a> {
    pub(crate) fn handle_echo(
        &mut self,
        channel_id: ChannelId,
        seq: Seq,
        temporary_id: &str,
        server_id: ServerId,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.module
            .handle_echo(channel_id, seq, temporary_id, server_id, out)
    }
}
```

- [ ] **Step 3: Register posted handler module**

Edit `crates/helix-im/src/ws/handlers/mod.rs`:

```rust
pub(crate) mod hello;
pub(crate) mod increment_channel;
pub(crate) mod increment_channel_end;
pub(crate) mod legacy_event;
pub(crate) mod posted;
```

- [ ] **Step 4: Add posted echo handler**

Create `crates/helix-im/src/ws/handlers/posted.rs`:

```rust
use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::state::{ChannelId, Seq, ServerId};
use crate::ws::registry::{WsActionKind, WsHandlerEntry, WS_HANDLERS};
use crate::ws::{ImWsContext, WsFrame};

fn handle_posted(
    ctx: &mut ImWsContext<'_>,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let data = frame.data_required()?;
    let post = data.get("post").unwrap_or(data);

    let temporary_id = post
        .get("temporary_id")
        .or_else(|| post.get("temporaryId"))
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ImError::UnsupportedWsAction("posted(non-echo)".to_string()))?;

    let server_id = post
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(ServerId::from_str)
        .ok_or_else(|| ImError::InvalidWsFrame("posted echo missing valid id".to_string()))?;

    let channel_id = post
        .get("channel_id")
        .or_else(|| post.get("channelId"))
        .and_then(|v| v.as_str())
        .and_then(ChannelId::from_str)
        .ok_or_else(|| {
            ImError::InvalidWsFrame("posted echo missing valid channel_id".to_string())
        })?;

    let seq = frame.event_seq().ok_or_else(|| {
        ImError::InvalidWsFrame("posted echo missing event_seq".to_string())
    })?;

    ctx.handle_echo(channel_id, Seq(seq.0), temporary_id, server_id, out)
}

#[distributed_slice(WS_HANDLERS)]
static POSTED: WsHandlerEntry = WsHandlerEntry {
    action: "posted",
    kind: WsActionKind::Business,
    handler: handle_posted,
};
```

- [ ] **Step 5: Run echo tests**

Run:

```bash
cargo test -p helix-im --test echo_reconcile_test
cargo test -p helix-im --test send_test
```

Expected:

```text
test result: ok
```

- [ ] **Step 6: Commit posted migration**

```bash
git add crates/helix-im/src/module.rs crates/helix-im/src/ws/handlers
git commit -m "feat(im): 迁移 posted echo WS handler"
```

## Task 6: Add Ignored Action Handlers And Unknown-Action Tests

**Files:**
- Create: `crates/helix-im/src/ws/handlers/ignored.rs`
- Modify: `crates/helix-im/src/ws/handlers/mod.rs`
- Modify: `crates/helix-im/src/ws/registry.rs`

- [ ] **Step 1: Register ignored handler module**

Edit `crates/helix-im/src/ws/handlers/mod.rs`:

```rust
pub(crate) mod hello;
pub(crate) mod ignored;
pub(crate) mod increment_channel;
pub(crate) mod increment_channel_end;
pub(crate) mod legacy_event;
pub(crate) mod posted;
```

- [ ] **Step 2: Add ignored handlers**

Create `crates/helix-im/src/ws/handlers/ignored.rs`:

```rust
use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;
use crate::ws::registry::{WsActionKind, WsHandlerEntry, WS_HANDLERS};
use crate::ws::{ImWsContext, WsFrame};

fn ignore_action(
    _ctx: &mut ImWsContext<'_>,
    _frame: &WsFrame,
    _out: &mut EffectSink,
) -> Result<(), ImError> {
    Ok(())
}

macro_rules! ignored_action {
    ($name:ident, $action:literal) => {
        #[distributed_slice(WS_HANDLERS)]
        static $name: WsHandlerEntry = WsHandlerEntry {
            action: $action,
            kind: WsActionKind::Ignored,
            handler: ignore_action,
        };
    };
}

ignored_action!(STATUS_CHANGE, "status_change");
ignored_action!(MULTIPLE_CHANNELS_VIEWED, "multiple_channels_viewed");
ignored_action!(TYPING, "typing");
ignored_action!(PRESENCE, "presence");
```

- [ ] **Step 3: Add registry tests for unknown and ignored actions**

Append to `#[cfg(test)] mod tests` in `crates/helix-im/src/ws/registry.rs`:

```rust
use crate::module::{ImConfig, ImModule};

fn empty_sink() -> EffectSink {
    EffectSink::new()
}

#[test]
fn dispatch_unknown_action_returns_error() {
    let mut module = ImModule::new(ImConfig::default());
    let mut ctx = ImWsContext::new(&mut module, 1);
    let mut out = empty_sink();
    let err = dispatch_ws(
        &mut ctx,
        br#"{"action":"new_backend_business_action","data":{}}"#,
        &mut out,
    )
    .expect_err("unknown business action should fail");

    assert!(matches!(
        err,
        ImError::UnsupportedWsAction(action) if action == "new_backend_business_action"
    ));
    assert!(out.is_empty());
}

#[test]
fn dispatch_ignored_action_returns_ok_and_no_effects() {
    let mut module = ImModule::new(ImConfig::default());
    let mut ctx = ImWsContext::new(&mut module, 1);
    let mut out = empty_sink();
    dispatch_ws(
        &mut ctx,
        br#"{"action":"status_change","data":{"user_id":"u1"}}"#,
        &mut out,
    )
    .expect("ignored action should not fail");

    assert!(out.is_empty());
}
```

- [ ] **Step 4: Run registry behavior tests**

Run:

```bash
cargo test -p helix-im ws::registry
```

Expected:

```text
test result: ok
```

- [ ] **Step 5: Commit ignored/unknown behavior**

```bash
git add crates/helix-im/src/ws/handlers/ignored.rs crates/helix-im/src/ws/handlers/mod.rs crates/helix-im/src/ws/registry.rs
git commit -m "feat(im): 显式区分未知与忽略 WS action"
```

## Task 7: Shrink Old `FrameClass` Dispatch And Run Full Verification

**Files:**
- Modify: `crates/helix-im/src/parser.rs`
- Modify: `crates/helix-im/src/module.rs`
- Modify tests if they import `FrameClass` directly.

- [ ] **Step 1: Search old dispatch users**

Run:

```bash
rg -n "FrameClass|classify_frame|parse_hello|classify_echo" crates/helix-im/src crates/helix-im/tests
```

Expected: only parser tests or obsolete code paths are listed.

- [ ] **Step 2: Remove or reduce obsolete classifier code**

Edit `crates/helix-im/src/parser.rs`:

- Keep reusable parsing helpers:
  - `extract_post_fields`
  - `parse_inbound`
  - `parse_increment_channel`
  - `parse_sync_response`
  - sync event helpers
- Remove obsolete action dispatcher items once no production code uses them:
  - `FrameClass`
  - `classify_frame`
  - `parse_hello`
  - `classify_echo`

If an existing test needs action dispatch coverage, rewrite it against `ws::dispatch_ws`
or handler-level behavior rather than `FrameClass`.

- [ ] **Step 3: Run targeted IM tests**

Run:

```bash
cargo test -p helix-im --test increment_parse_test
cargo test -p helix-im --test real_go_frames_test
cargo test -p helix-im --test echo_reconcile_test
cargo test -p helix-im --test c2_phantom_readbits_test
cargo test -p helix-im --test c3_sync_mutation_emits_test
```

Expected:

```text
test result: ok
```

- [ ] **Step 4: Run crate tests**

Run:

```bash
cargo test -p helix-im
```

Expected:

```text
test result: ok
```

- [ ] **Step 5: Run full gate**

Run:

```bash
scripts/gate.sh
```

Expected:

```text
✅ 全部闸门通过
```

If the final public-api substep is skipped because `cargo-public-api` is not installed,
record that exact skip text in the final report.

- [ ] **Step 6: Run GitNexus detect changes before final commit**

Run the GitNexus tool:

```text
detect_changes({scope: "all", repo: "helix", worktree: "/Users/mac17/workspace/rust/helix"})
```

Expected: changed symbols are confined to `helix-im` WS parsing/dispatch and Cargo metadata.

- [ ] **Step 7: Commit cleanup and verification**

```bash
git add crates/helix-im/Cargo.toml crates/helix-im/src crates/helix-im/tests
git commit -m "refactor(im): 用 WS handler registry 替换集中 action 分发"
```

## Self-Review

Spec coverage:

- `action -> handler` registry: Task 1.
- `linkme` rather than custom mechanism: Task 1.
- Stable dispatcher: Task 1 and Task 2.
- Unknown action returns error: Task 6.
- Ignored operational actions are explicit: Task 6.
- Existing `hello`/`increment`/`posted`/legacy behavior migration: Tasks 2 through 5.
- HTTP excluded from strategy registry: File structure and Task scope keep HTTP untouched.
- `helix-core` unchanged: all tasks target `helix-im` only.
- Gate verification: Task 7.

Placeholder scan:

- This plan intentionally contains no placeholder sections.

Type consistency:

- `WsHandlerFn` consistently uses `ImWsContext<'_>`, `WsFrame`, and `EffectSink`.
- `dispatch_ws` consistently returns `Result<(), ImError>`.
- `WsFrame::event_seq` consistently returns `Option<Seq>`.
