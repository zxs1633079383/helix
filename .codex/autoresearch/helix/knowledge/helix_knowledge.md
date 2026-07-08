# Helix Knowledge Base

## 0. Provenance And Scope

This knowledge base was distilled on 2026-07-08 from the live repository, with a founder/core-maintainer lens. Primary evidence came from repo-local sources: `README.md`, `Cargo.toml`, `docs/GOAL.md`, `docs/ROADMAP.md`, `docs/harness/README.md`, `docs/harness/OPERATING.md`, `docs/harness/decisions.md`, crate `src/lib.rs` files, `scripts/gate.sh`, and the code graph for the indexed project `System-Volumes-Data-workspace-rust-helix`.

This is a project repository distillation, not a trained AutoReSearch bundle. Score: unscored. Memory was used only as routing context and was not treated as current proof.

Time-sensitive status is marked as current-at-distillation. Re-check `docs/ROADMAP.md`, `SESSION.md`, `git status`, and `scripts/gate.sh` before making claims about current completion.

2026-07-08 doc-audit status: `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi` passed locally, but `scripts/structure-gate.sh` was red, `scripts/review-gap.sh` reported gap 92/10, `.git/hooks/pre-push` was missing, and local `cargo-public-api` was not installed. Treat those as live verification boundaries until refreshed.

## 1. Founder Thesis

Helix is strategically important because it attacks the expensive part of IM product maintenance: business behavior drifting across PC, mobile, server migration stages, and test harnesses. Its bet is not "Rust for Rust's sake"; the bet is one deterministic, replayable business kernel that can be driven by multiple hosts without duplicating sync, send, storage, and projection logic.

The winning wedge is IM, but the platform thesis is broader:

- One core state machine can serve PC Tauri and Flutter FFI without owning either runtime.
- Business modules can be added by implementing `Module` and emitting `Effect`, not by teaching the core every domain.
- Host I/O can be shared where it is truly cross-platform, especially SQLite storage, HTTP, WS, event sinks, trace hooks, and batching.
- Replay, golden fixtures, and structured hop observability can turn integration failures from "guess through four repos" into mechanical evidence.

What not to chase:

- Do not restart web/wasm as a delivery target without a new ADR. `helix-driver-wasm` is archived/experimental.
- Do not make Helix a UI framework. Direction A can emit data ViewModel shapes from business modules later; view trees, widgets, and pixels are out of scope.
- Do not make `helix-core` a generic plugin framework before a second real module forces the shape.
- Do not solve product gaps by putting HTTP, SQL, login, thread models, or domain words into `helix-core`.

The founder-level opportunity is a "runtime-agnostic business OS" for high-integrity client products: deterministic business logic, replayable effects, host-owned I/O, and exact contract gates. The current product opportunity is finishing the Loopforge Tauri validation path, then letting mobile use the same kernel through FFI without reimplementing IM behavior.

## 2. Core Maintainer Mental Model

Think in three layers:

```text
business modules  helix-im first, future presence/notify/etc.
core layer        helix-core ports, Module host, Tick, Effect, deterministic shell
platform drivers  helix-driver-host shared I/O, helix-driver-native, helix-driver-ffi
```

`helix-core` is the smallest trusted kernel. It owns protocol for stepping modules and expressing I/O intent, but it does not do I/O. A maintainer should be able to prove that the same input ticks produce the same output effects.

`helix-im` is a guest module. It owns IM concepts: channel, message, cursor, temporary id, read bits, projection payloads, outbound commands, and sync semantics. Those words must not migrate into `helix-core`.

`helix-driver-host` is the shared host I/O layer. It is where real SQLite storage, HTTP/WS clients, event sink instrumentation, bounded spawners, local store identity, trace hooks, and shared codecs live. Native and FFI are shells around this host layer, not parallel implementations of IM business logic.

`helix-driver-native` is the PC/Tauri-facing runtime shell. It wires tokio, native transport, event bus, and shared host dependencies.

`helix-driver-ffi` is the mobile/Flutter FFI shell. It exposes a small C ABI, owns no business behavior, and must obey FFI ownership/no-unwind/batch rules.

The maintainer move is to ask: "Which boundary owns this fact?"

- Business state or wire domain rule: `helix-im`.
- Runtime-neutral orchestration shape: `helix-core`.
- SQLite/HTTP/WS/event/trace execution: `helix-driver-host`.
- Tauri/desktop surface: `helix-driver-native`.
- Flutter/C ABI/mobile surface: `helix-driver-ffi`.
- Existing production behavior truth: current cses-client / Go wire docs, not imagined APIs.

## 3. Architecture Map

Workspace crates current-at-distillation:

- `crates/helix-core`: ports, `Module`, `ModuleHost`, `ExecutionShell`, `Tick`, `Effect`, runtime-neutral platform traits.
- `crates/helix-im`: IM module, outbound command registry, WS parser/handlers, sync-v2, channel gate, send/reconcile, render-ready projections, SQLite schema.
- `crates/helix-driver-host`: shared host pieces for PC and Flutter FFI: storage, HTTP/WS, engine loop, batch sink, recording sink, trace hooks, local store locator, bounded spawner, timer, upload.
- `crates/helix-driver-native`: tokio/native shell and re-exports around host driver.
- `crates/helix-driver-ffi`: C ABI shell for mobile, C++ host vtable, command/query/event batch bridge.
- `crates/helix-host-cli`: local integration host used for real Go smoke/e2e; not a product runtime.
- `crates/helix-driver-wasm`: archived experiment; not a current delivery target.

Important code graph signals:

- `run_engine_loop` lives in `helix-driver-host/src/engine.rs`; native re-exports/wraps it.
- `LocalStoreLocator` resolves tenant-safe local SQLite paths in the host layer.
- `TraceHooks` lives in host driver and provides tick/storage/http/upload/ws/event trace boundaries.
- `Effect` lives in `helix-core/src/effect.rs`; host/driver code consumes it.
- `HostStorage` lives in `helix-driver-host/src/storage.rs` and implements shared SQLite behavior.

Important docs:

- `docs/GOAL.md`: north star, hard constraints, red lines.
- `docs/ROADMAP.md`: current open P0/P1/P2 live ledger.
- `docs/harness/README.md`: HX-C constraint library index.
- `docs/harness/OPERATING.md`: what event triggers what skill/workflow/gate.
- `docs/harness/decisions.md`: ADR-lite decisions.
- `docs/COMMIT_CONVENTION.md`: commit/PR/tag discipline.
- `crates/helix-im/docs/projection-schema.md`: projection payload contract source.
- `crates/helix-driver-host/docs/external-host-instrumentation-recipe.md`: external host recording/instrumentation recipe for Loopforge.

## 4. Domain And Runtime Concepts

Core concepts:

- `Tick`: the input to the deterministic shell, such as inbound frames, timer firings, command intents, or port replies.
- `Effect`: an output intent, such as persist, send, schedule timer, emit event, HTTP, upload, or transport action.
- `Module`: business guest interface. A module registers with the host and translates ticks/state into effects.
- `Port`: runtime-neutral trait boundary for storage, transport, clock, event sink, IDs, HTTP, and upload.
- `Correlation`: routing key that ties an effect to a later port reply.
- `DomainEventBytes`: opaque event bytes crossing from business module to host/UI.

IM concepts:

- `ImModule`: first real business module.
- Outbound command registry: maps public IM commands to effect plans.
- WS parser and handlers: anti-corruption layer from raw wire frames to strong inbound events.
- Channel gate: cursor monotonicity and ordered apply rules.
- Sync-v2: online push plus offline pull, anchored on real Go wire contract.
- Projection events: `im:__bus__` payloads for host/UI, covered by schema tests.
- Send reconcile: optimistic local send, server id echo, read bits, and failure handling.

Host/runtime concepts:

- `BatchSink`: batch event boundary for host projection emission and recording.
- `RecordingSink`: decorator for recording projection input/output without changing engine logic.
- `BoundedSpawner`: backpressure mechanism for Persist/HTTP/HttpFire classes.
- `HostNetworkConfig` and header registries: host-provided API/WS config and auth headers.
- `LocalStoreIdentity` -> `LocalStoreKey` -> local tenant DB path: host-owned tenant isolation.
- `TraceHooks`: sidecar trace propagation and hop probes without leaking trace into core or public IM payloads.

## 5. Invariants And Design Boundaries

Hard invariants:

- `helix-core` must not depend on tokio, sqlx, reqwest, web-sys, tauri, or any platform/runtime crate.
- `helix-core` must not contain IM domain words or business schema.
- Core does not do I/O, spawn, sleep, call wall-clock, generate random IDs, or own concurrency.
- Driver/platform code may depend on runtime crates; dependency direction is driver -> core, never core -> driver.
- Time, ID, network, storage, and event emission are injected by host/driver.
- Async port boundaries must respect native Send and FFI/single-thread `!Send` needs.
- Owned/Copy values cross async/channel/FFI boundaries; no lifetime payloads in state that needs to travel.
- Features must be additive. No mutually exclusive feature model.
- Hot paths require O(1) patterns: static names where possible, prepared/indexed operations, no read-modify-write in critical storage writes, no accidental heap churn.
- FFI must not unwind across `extern "C"`, must pair raw pointer ownership exactly once, and must batch effects/events across the boundary.
- Collection iteration that produces ordered effects must be deterministic: ordered collections or explicit sort before output.
- Tests for concurrency/stress must be falsifiable. Avoid wall-clock lower-bound assertions as proof.

Current open live constraints from `docs/ROADMAP.md` current-at-distillation:

- Tauri four seams and gray switch are still a P0 blocker, with Loopforge as the first landing repo.
- Golden frame export/replay tooling remains open, with Loopforge feeding real projection recordings back into Helix fixtures.
- AI engineering gates are closed in the repo-local evidence chain: GitLab gate/api-surface jobs, clippy deny-list, per-crate instruction mirrors, and `docs/api-baseline/helix-core.txt`. Caveat: if `cargo-public-api` is not installed locally, local public-api diff cannot be claimed from the workstation run.
- Four-capability feature split remains technical debt.
- Multi-module concurrent write pressure has not been proven with a second business module.
- Architecture drift review is currently due when `scripts/review-gap.sh` still reports gap >= 10; do not start fresh feature work on top of a red drift gate.
- Product target cargo check across core/im/native/ffi should be kept at zero errors.

## 6. Development And Verification Playbook

Start of meaningful work:

1. Read `docs/GOAL.md`.
2. Check `docs/ROADMAP.md` section two for current live blockers.
3. Use `docs/harness/OPERATING.md` to decide which gate, skill, or workflow applies.
4. Check `git status --short` and do not overwrite unrelated user changes.
5. For code discovery, prefer code graph tools for symbols and relationships; use shell search for strings, docs, configs, and scripts.

Core verification commands:

```bash
cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi
cargo test -p helix-core
cargo tree -p helix-core | grep -iE 'tokio|sqlx|reqwest|web-sys|tauri'
```

Default full gate before commit when touching behavior:

```bash
scripts/gate.sh
```

FFI-specific verification:

```bash
bash scripts/ffi-verify.sh --no-demo
bash scripts/cpp-smoke.sh
```

Structure and drift checks:

```bash
bash scripts/structure-gate.sh
bash scripts/review-gap.sh
bash scripts/branch-identity.sh --session
```

Trace/static safety:

```bash
bash scripts/trace-static-gate.sh
```

Cargo discipline for agentic Rust work:

- Do not run cargo after every small edit.
- Use type reasoning first.
- Run cargo once after a coherent module edit, then batch-fix errors, then run final verification.
- Target: two or three cargo calls for a narrow task, unless the task itself is verification.

Commit discipline:

- Use Conventional Commits with Chinese description/body.
- If touching three or more files, crossing modules, changing harness, schema, API, or more than about 50 lines, use the five-section structured body from `docs/COMMIT_CONVENTION.md`.
- For harness changes, fill the "Harness 更新" section explicitly.

Stop/go signals:

- Stop and ask if a proposed change conflicts with `docs/GOAL.md`.
- Stop before putting runtime dependencies or business concepts into `helix-core`.
- Stop before reviving wasm/web as a target.
- Ask for human review for ABI changes, public command/event schema changes, storage schema migrations, or cross-repo cutovers.
- Continue without asking for pure project-local knowledge/doc artifacts when the request is explicit and boundaries are clear.

## 7. Contribution And Product Opportunities

High-leverage contribution surfaces:

- Finish Loopforge Tauri seam integration using `BatchSink`/`RecordingSink` without changing Helix engine logic.
- Build golden frame tooling: export real projection frames from Loopforge, convert to Tick scripts, replay in Helix tests.
- Keep the public API gate baseline current for `helix-core` so domain leakage remains a structural diff, not a grep guess.
- Add the second business module or a minimal pressure harness to prove multi-module concurrent writes and fairness.
- Reduce feature debt in `helix-driver-native` and keep features additive.
- Improve FFI demo and mobile acceptance while keeping mobile bridge assets in the mobile integration repo.
- Keep projection-schema, mobile contract docs, and tests synchronized as commands/events evolve.

Founder-level product angles:

- "Deterministic client kernel" as an internal platform for complex enterprise clients.
- "Replayable IM migration harness" to move behavior from legacy clients into a shared core with proof.
- "Agent-safe development substrate" where harness cards, gates, and structured expectations make autonomous work auditable.
- "Trace-first multi-hop diagnosis" across UI -> host -> core -> storage/network -> server, without leaking business payloads into trace.

Avoid low-leverage work:

- Cosmetic architecture rewrites not tied to gates or production failures.
- Duplicating HTTP/WS/storage behavior per platform.
- Adding generic extension frameworks before another real module needs them.
- Broad documentation churn that does not update the corresponding gate, source, or README index.

## 8. Production Readiness Radar

Green or mature areas:

- North-star constraints are documented and backed by HX-C harness cards.
- Shared host storage/network boundary is established.
- FFI no-unwind/ownership/batching discipline is explicit.
- Sync-v2 has real Go wire alignment docs and tests.
- Trace sidecar patterns exist in host/Loopforge/mobile planning surfaces.
- `scripts/gate.sh` runs a serious multi-crate verification chain.

Yellow areas:

- The Tauri four-seam product landing is still open, and Loopforge is the first proof surface.
- Golden frame recording/replay is not fully closed.
- Structure-gate is the current local red signal when files exceed the 300-line policy or stored baselines; do not infer acceptance green from cargo check alone.
- Multi-module concurrent writes remain unproven with a second real module.
- Some claims depend on cross-repo evidence; re-scan Loopforge, mobile, and Go before saying "full chain green."

Red-line risks:

- Silent reintroduction of runtime dependencies into `helix-core`.
- Business schema leaking into core ports.
- Agents making cross-repo claims from memory instead of fresh collector/reviewer/gate evidence.
- FFI ABI drift without native/C++ smoke proof.
- Treating Apifox/HTTP success as proof of WS/projection/storage/DOM real-chain success.
- Recording instrumentation attached at the wrong boundary (`EventSink` instead of `BatchSink` for the external host recipe).

Observability posture:

- Every hop crossing a port/layer boundary should have a structured probe and a correlation key.
- If diagnosis feels much harder than the logic complexity, first suspect missing observability, not the business algorithm.
- Trace and recording are sidecar mechanisms; public command payloads should not absorb trace semantics.

## 9. Agent Rules

Identity:

- Act as a founder-core-maintainer hybrid: strategy-aware, boundary-protective, evidence-driven.
- Prefer fewer, stronger changes that move the product proof forward.
- Treat docs/harness/tests as engineering assets, not chat summaries.

Modification boundaries:

- Default safe write area for knowledge updates is `.codex/autoresearch/helix/`.
- Do not touch `crates/**` unless the user asks for code changes or a plan explicitly requires implementation.
- Do not touch root `AGENTS.md` or `CLAUDE.md` unless changing instructions; if one changes, mirror the other exactly.
- Do not revert unrelated dirty worktree changes.
- Do not revive `helix-driver-wasm` delivery scope without an ADR.

Discovery rules:

- Use the code graph for symbol discovery: functions, structs, methods, call paths.
- Use shell search for literal strings, config keys, docs, scripts, and test names.
- Use memory only to find likely files or prior decisions; verify repo facts locally before reporting current truth.
- Use web search only for version-sensitive external facts, ecosystem state, or community/release claims.

Verification rules:

- Before saying "done", run `python3 scripts/evaluate.py` for this bundle.
- Before saying code behavior is correct, run the narrow crate tests or gates that prove it.
- For cross-repo integration claims, require fresh repo scan, reviewer report, collector evidence, or gate output.
- Record evidence limits in reports. "Not checked" is better than false green.

Review rules:

- Review for boundary violations first: core runtime leakage, domain leakage, FFI ABI/ownership, hot path allocation, nondeterminism, unverified cross-repo claims.
- Then review tests: are they falsifiable, narrow, and tied to invariants?
- Then review maintainability: file size, module responsibility, command/schema docs, harness index synchronization.

## 10. Open Questions

- What is the exact next proof boundary for Loopforge Tauri four seams, and which gate makes it non-declarative?
- Where will golden frame fixtures live long-term: only `tests/fixtures/` or also a separate trace-export corpus?
- What minimal second business module can prove multi-module scheduling/write fairness without overbuilding a plugin framework?
- When should public API baseline become mandatory in CI, and who owns baseline updates?
- Which mobile FFI acceptance layer is the next practical green: C++ smoke, Flutter console, or device UI?
- How should the project expose founder-level strategy without bloating every agent prompt?
