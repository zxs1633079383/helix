# WS Handler Registry Design

> Date: 2026-06-17
> Scope: `helix-im` WebSocket inbound message dispatch only
> Status: Draft approved in chat, pending file review

## Goal

Make WebSocket message handling in `helix-im` easy to extend:

- Incoming backend WS frames are JSON.
- A stable dispatcher reads a message type field, currently `action`.
- The dispatcher finds a handler from an `action -> handler` registry.
- Adding a new WS message type must not require changing the dispatcher.
- New message support should require only a handler file, a registration entry, and tests.

This design intentionally does not apply the same registry pattern to HTTP requests.
HTTP remains simple helper/builder code because HTTP requests are active business
operations rather than backend-pushed message strategies.

## Non-Goals

- Do not move this registry into `helix-core`.
- Do not create a multi-business-module framework.
- Do not change frontend/Tauri/FFI command APIs in this phase.
- Do not build a custom registration mechanism.
- Do not make WebSocket handlers perform I/O or async work directly.

`helix-im` remains the only business module target for this design.

## Dependency Choice

Use `linkme` distributed slices for handler registration.

Rationale:

- It provides a proven Rust mechanism for distributed static registration.
- Each handler can register itself near its implementation.
- The central dispatcher and registry declaration stay stable as new actions are added.
- It avoids `build.rs` code generation and avoids a custom registration framework.

## Module Layout

Add a focused `ws` submodule under `crates/helix-im/src/`:

```text
crates/helix-im/src/ws/
  mod.rs
  frame.rs
  registry.rs
  context.rs
  handlers/
    mod.rs
    hello.rs
    increment_channel.rs
    increment_channel_end.rs
    posted.rs
    legacy_event.rs
    ignored.rs
```

The first implementation should move current `classify_frame + FrameClass` behavior
behind this module without changing the driver or `helix-core`.

## Registry

Declare a distributed static slice in `ws/registry.rs`:

```rust
use linkme::distributed_slice;

pub struct WsHandlerEntry {
    pub action: &'static str,
    pub kind: WsActionKind,
    pub handler: WsHandlerFn,
}

pub enum WsActionKind {
    Business,
    Ignored,
}

pub type WsHandlerFn = fn(
    ctx: &mut ImWsContext,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError>;

#[distributed_slice]
pub static WS_HANDLERS: [WsHandlerEntry];
```

Build a lookup map once:

```rust
static WS_HANDLER_MAP: OnceLock<Result<HashMap<&'static str, WsHandlerFn>, ImError>> =
    OnceLock::new();
```

The map construction must reject duplicate actions with `ImError::DuplicateWsAction`.
Tests must assert uniqueness so duplicate registration fails before runtime use.

## Handler Registration

Each action registers itself next to its handler:

```rust
use linkme::distributed_slice;

fn handle_post_read(
    ctx: &mut ImWsContext,
    frame: &WsFrame,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let data = frame.data_required()?;
    let channel_id = parse_channel_id(data)?;
    let msg_id = parse_msg_id(data)?;
    let read_bits = parse_read_bits(data)?;
    let event_seq = frame.event_seq_required()?;

    out.push(to_effect::apply_read_bits(msg_id, read_bits));
    out.push(to_effect::emit_post_read(channel_id, event_seq.0, msg_id));
    Ok(())
}

#[distributed_slice(WS_HANDLERS)]
static POST_READ: WsHandlerEntry = WsHandlerEntry {
    action: "post_read",
    kind: WsActionKind::Business,
    handler: handle_post_read,
};
```

Adding a new WS message type should not touch `dispatch_ws`.

## Frame Model

`WsFrame` owns the raw bytes and parsed JSON envelope:

```rust
pub struct WsFrame {
    raw: Bytes,
    root: serde_json::Value,
    action: String,
}
```

Required methods:

```rust
impl WsFrame {
    pub fn parse(raw: &[u8]) -> Result<Self, ImError>;
    pub fn action(&self) -> &str;
    pub fn data(&self) -> Option<&serde_json::Value>;
    pub fn data_required(&self) -> Result<&serde_json::Value, ImError>;
    pub fn root(&self) -> &serde_json::Value;
    pub fn event_seq(&self) -> Option<Seq>;
    pub fn event_seq_required(&self) -> Result<Seq, ImError>;
}
```

Parsing rules:

- Invalid JSON returns `ImError::InvalidWsFrame`.
- Frames with top-level `action` use that action string.
- Frames without top-level `action` are mapped to `__legacy_event__`.
- Top-level parsing is centralized in `WsFrame`; action-specific parsing remains in handlers.

## Dispatch

The central dispatcher is stable:

```rust
pub fn dispatch_ws(
    ctx: &mut ImWsContext,
    raw: &[u8],
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let frame = WsFrame::parse(raw)?;
    let Some(handler) = registry().get(frame.action()) else {
        return Err(ImError::UnsupportedWsAction(frame.action().to_string()));
    };

    handler(ctx, &frame, out)
}
```

Unknown actions return an error. Operational/noise frames must be explicitly registered
as ignored handlers. This keeps unsupported business frames visible during integration.

## Context

Handlers receive a thin IM context rather than the whole module where practical:

```rust
pub struct ImWsContext<'a> {
    pub state: &'a mut ImState,
    pub config: &'a ImConfig,
    pub ids: &'a mut ImIdAlloc,
}
```

`ImIdAlloc` should wrap the existing correlation and timer allocation currently stored
on `ImModule`. If this extraction becomes too large for the first change, the first
implementation may pass `&mut ImModule` internally and then narrow to `ImWsContext`
in a follow-up refactor. The final direction remains a thin handler context.

Handlers may:

- Read and mutate IM state.
- Allocate correlations and timers through the provided allocator.
- Push `Effect::Persist`, `Effect::Emit`, `Effect::Http`, `Effect::Send`,
  `Effect::ScheduleTimer`, or `Effect::CancelTimer`.

Handlers must not:

- Perform I/O.
- Await futures.
- Spawn tasks.
- Import driver/runtime crates.

## Current Action Migration

Current behavior should map to handlers as follows:

| Action | Handler | Behavior |
|---|---|---|
| `hello` | `handlers/hello.rs` | Complete handshake, store connection id, emit established, trigger increment HTTP |
| `increment_channel` | `handlers/increment_channel.rs` | Register/update channel increment state and emit increment event |
| `increment_channel_end` | `handlers/increment_channel_end.rs` | Emit channel update or trigger global-end sync/heal |
| `posted` | `handlers/posted.rs` | Handle echo only when `temporary_id` is present |
| `__legacy_event__` | `handlers/legacy_event.rs` | Preserve current legacy `{event,data}` path |
| Operational/noise actions | `handlers/ignored.rs` | Return `Ok(())` and emit no effects |

Examples of ignored actions:

- `status_change`
- `multiple_channels_viewed`
- `typing`
- `presence`

Non-echo `posted` is not ignored. It returns an unsupported-action style error until
the project deliberately implements online receive semantics for that shape.

## Error Model

Add explicit IM errors:

```rust
pub enum ImError {
    UnsupportedWsAction(String),
    DuplicateWsAction(String),
    InvalidWsFrame(String),
    // existing variants...
}
```

Runtime behavior:

- Invalid JSON: `InvalidWsFrame`.
- Missing top-level action: `__legacy_event__`.
- Registered action: call handler.
- Unknown action: `UnsupportedWsAction`.
- Explicit ignored action: `Ok(())`.
- Duplicate action: registry construction/test failure via `DuplicateWsAction`.

## HTTP Scope

HTTP does not use the WS registry pattern in this phase.

Allowed HTTP cleanup:

- Keep or add small request builder helpers in `acl/to_effect.rs`.
- Reduce repeated `Content-Type`, `connectionId`, and JSON serialization boilerplate.
- Keep response routing through the existing `corr -> state` maps.

Do not introduce an HTTP strategy registry unless HTTP actions become numerous enough
to justify it later.

## Testing

Required tests:

1. Registry uniqueness: all `WS_HANDLERS` actions are unique.
2. Dispatch lookup: `hello` resolves and calls the hello handler.
3. Unknown action returns `UnsupportedWsAction`.
4. Ignored action such as `status_change` returns `Ok(())` and emits no effects.
5. Legacy no-action frame maps to `__legacy_event__`.
6. Posted echo frame with `temporary_id` reaches echo reconciliation behavior.
7. Posted non-echo frame returns an error.
8. Existing sync-v2 replay tests continue to pass.

## Acceptance Criteria

- Adding a new WS message type does not modify `dispatch_ws`.
- Adding a new WS message type does not modify `ImModule::handle` top-level WS dispatch.
- A new type requires only a handler file, a `linkme` registration entry, and tests.
- Unknown business actions fail visibly.
- Known operational/noise actions are explicitly registered as ignored.
- `helix-core` remains unchanged and runtime-neutral.
- `scripts/gate.sh` passes.

## Implementation Boundaries

The implementation plan should be split into small commits:

1. Add `linkme` and the `ws` registry/frame/context skeleton with tests.
2. Migrate `hello`, `increment_channel`, and `increment_channel_end`.
3. Migrate `posted` echo and legacy event handling.
4. Add ignored-action handlers and unknown-action tests.
5. Remove or shrink old `FrameClass`/`classify_frame` code once all tests pass.

Each commit should preserve green tests for the migrated subset.
