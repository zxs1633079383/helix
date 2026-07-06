# Helix System Mastery Resources

## Knowledge

- [Local: `docs/GOAL.md`](docs/GOAL.md)
  The north-star source for Helix's runtime-agnostic, deterministic, sans-IO architecture. Use for: deciding whether a change violates the project direction.
- [Local: `docs/ROADMAP.md`](docs/ROADMAP.md)
  Current roadmap and P0/P1/P2 living account. Use for: learning priority and present blockers.
- [Local: `docs/harness/OPERATING.md`](docs/harness/OPERATING.md)
  Trigger map for harness, workflows, validation, and expert dispatch. Use for: knowing what procedure to call for a task.
- [Local: `crates/helix-core/AGENTS.md`](crates/helix-core/AGENTS.md)
  Core crate invariants: zero runtime, zero business concepts, synchronous `step()`, ports, effects, and module host.
- [Local: `crates/helix-im/AGENTS.md`](crates/helix-im/AGENTS.md)
  IM module map and behavior-truth routing: ACL, channel, send, sync, query, todo, outbound, and WS.
- [Local: `crates/helix-driver-host/AGENTS.md`](crates/helix-driver-host/AGENTS.md)
  Shared host driver truth for storage, pump core, `BatchSink`, `RecordingSink`, rows codec, HTTP envelope, and bounded workers.
- [Local: `crates/helix-driver-native/AGENTS.md`](crates/helix-driver-native/AGENTS.md)
  PC/Tauri-native shell constraints and pump/storage backpressure invariants.
- [Local: `crates/helix-driver-ffi/AGENTS.md`](crates/helix-driver-ffi/AGENTS.md)
  Mobile/Flutter FFI C ABI, vtable, batch event return, pointer ownership, and lifecycle rules.
- [Official: The Rust Book](https://doc.rust-lang.org/book/)
  Primary Rust language learning source. Use for: ownership, borrowing, enums, traits, generics, error handling, and modules.
- [Official: The Rust Reference](https://doc.rust-lang.org/reference/)
  Precise language semantics. Use for: `unsafe`, ABIs, attributes, lifetimes, and cfg/features when the book is not exact enough.
- [Official: The Rustonomicon](https://doc.rust-lang.org/nomicon/)
  Unsafe Rust guide. Use for: FFI boundary reasoning, raw pointers, ownership transfer, and panic boundaries.
- [Official: Cargo Book](https://doc.rust-lang.org/cargo/)
  Cargo features, workspaces, targets, and dependency behavior. Use for: additive features, crate boundaries, and validation commands.

## Wisdom (Communities)

- [Rust Users Forum](https://users.rust-lang.org/)
  High-signal place for practical Rust language and library questions. Use for: ownership, trait, async, and API design questions after local repo context is clear.
- [Rust Internals](https://internals.rust-lang.org/)
  Language and compiler design discussions. Use sparingly for: deep semantics or feature behavior, not day-to-day Helix tasks.

## Gaps

- Add project-local examples as lessons accumulate: one reference for `Tick -> Effect`, one for host pump dispatch, one for IM outbound/WS symmetry, and one for FFI lifecycle.
