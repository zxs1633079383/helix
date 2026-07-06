# Mission: Helix System Mastery

## Why
You want to understand Helix as a working system, not as isolated Rust files: the core, IM module, host/native/FFI drivers, boundary cases, and local instruction stack all need to become navigable enough that you can steer implementation and eventually design a Loop Engineer/router layer across repos.

## Success looks like
- Read a Helix task and immediately identify which layer owns it: `helix-core`, `helix-im`, `helix-driver-host`, `helix-driver-native`, `helix-driver-ffi`, or an external host such as `loopforge-tauri-im`.
- Explain the main invariants behind core ports, effects, drivers, projection events, storage, HTTP/WS, and FFI without drifting into generic Rust talk.
- Use the repo's instruction stack, roadmap, harness cards, and validation commands to decide what to learn or change next.
- Learn the Rust syntax needed for this codebase in context: traits, enums, generics, lifetimes, cfg/features, ownership, channels, async boundaries, and unsafe FFI.

## Constraints
- Keep teaching grounded in current local repo files and project instructions.
- Prioritize practical navigation and boundary judgment over textbook completeness.
- Use short lessons with retrieval practice so the knowledge survives beyond the current session.

## Out of scope
- Web/wasm product delivery, unless a future ADR reactivates it.
- Re-teaching all of Rust from scratch before touching Helix.
- Broad multi-repo implementation work in this teaching workspace unless the user explicitly switches from learning to building.
