# Phase A re-port audit — `feat/sync-v2-full-replacement` → `dev-zlc` (ADR-007)

> READ-ONLY enumeration audit. Phase A = the two data-loss bug fixes already on feat:
> **#1 cursor seed = 0** and **#6 envelope unwrap layer**.
> FEAT (source of truth, real-machine verified): `worktrees/sync-v2-full-align/`
> DEV-ZLC (re-port target, ADR-007): `worktrees/sync-v2-report/`
>
> Key architectural fact: feat and dev-zlc solve #6 in **opposite directions**.
> feat *removed* the HTTP envelope (driver returns raw Go body).
> dev-zlc *keeps* the ADR-007 `run_http_envelope` (shared host module, native+FFI).
> Per SESSION ADR-007 ruling: **keep dev-zlc's envelope**, add the unwrap+base64 decode
> only on the SyncPull path in `module.rs handle_sync_reply`. Do NOT remove the envelope,
> do NOT touch scan path.

---

## 1. Per-item re-port checklist

### #1 — cursor seed = 0 (data-loss: offline backfill never fires)

| | detail |
|---|---|
| **What feat does** | `or_insert_with(\|\| Channel::new(channel_id, 0))` — brand-new channel's in-memory cursor seeds to **0** (= "no locally-confirmed event"). `inc.last_event_seq` (server watermark) is fed ONLY into `increment_target` (gap / B2-heal stuck detection), strictly decoupled from cursor. |
| **What dev-zlc does** | `or_insert_with(\|\| Channel::new(channel_id, inc.last_event_seq.0))` — seeds the in-memory cursor to the **server watermark `lastEventSeq`**. |
| **Why dev-zlc is a data-loss bug** | On cold start of a truly new channel (DB has no cursor → `on_start` Scan didn't load it → `or_insert_with` fires), cursor = lastEventSeq = max. Then `increment_channel_end`'s `from_seq = cursor = max` → server returns `no_change` → **offline backfill never triggers**. B2 heal's `cursor < target` is always false (cursor == target == lastEventSeq) → heal also dies. All offline events for the channel are lost. |
| **Exact change** | In `apply_increment`, change the `or_insert_with` closure from `Channel::new(channel_id, inc.last_event_seq.0)` → `Channel::new(channel_id, 0)`. The `increment_target` block below (lines ~28-35) is already correct and stays — it is the legitimate consumer of `inc.last_event_seq`. |
| **Target file:line** | `worktrees/sync-v2-report/crates/helix-im/src/ws/handlers/increment_channel.rs:22` (the `.or_insert_with(...)` line). |
| **ADR-007 seam concern** | **None.** Pure sans-IO business logic, no driver/transport touch. Clean re-port. APIs the tests need (`cursor_for`, `register_channel`, `ingest_increment`) already exist on dev-zlc (`module.rs:85/92/122`). `Channel::new(id, initial_cursor)` signature identical both sides (`channel.rs:76`). |
| **Bonus (HX-C010, same file pair)** | feat also adds `stuck.sort_unstable();` in `increment_channel_end.rs heal_incomplete_backfill_once` (after the `stuck` Vec is collected from the `increment_fetched` HashSet, before the resync loop). Reason: `stuck` is collected from a `HashSet` → iteration order is SipHash-random per process; `emit_proactive_resync_for` allocates monotonic corr + inserts into `corr_map` per channel (corr is load-bearing: routing + inflight guard + replay assertions), so ≥2 stuck channels need deterministic order. dev-zlc currently has `let stuck:` (immutable) at `increment_channel_end.rs:74`; re-port = make it `let mut stuck:` and add `stuck.sort_unstable();` before the resync. |

### #6 — envelope unwrap layer (latent data-loss: silent NoChange swallow)

| | detail |
|---|---|
| **What feat does** | Removed the envelope at the **driver**. `run_http` (native `engine_loop.rs:431`) returns `Ok(raw body)` on 2xx, `Err(Http(status))` on non-2xx. `handle_sync_reply` then feeds raw body straight to `parse_sync_response`. (feat's `run_http` head-note explicitly documents *why* the `{status,headers,body:base64}` envelope was dropped: "全仓零消费方解信封".) |
| **What dev-zlc does** | Keeps the ADR-007 envelope. `run_http_envelope` (shared host: `helix-driver-host/src/http_envelope.rs`) wraps every 2xx response as `{"status":<u16>,"headers":[[k,v]...],"body":"<base64>"}`. `pools.rs:71` drives it. `handle_sync_reply` (`module.rs:491`/`517`) feeds this **envelope JSON** directly to `parse_sync_response` → parser can't find `data.entries` → **silently returns `NoChange` → 0 persist / 0 emit**. Offline messages silently dropped. This is the latent #6 bug. |
| **Exact change (ADR-007-aligned, the agreed contract)** | In `handle_sync_reply` (dev-zlc `module.rs:491`), **before** the `parse_sync_response(reply.0.as_ref(), channel_id)` call, add an unwrap layer that runs ONLY on this SyncPull path: (1) parse `reply.0` as JSON envelope `{status, headers, body}`; (2) base64-**decode** the `body` string → raw Go body bytes; (3) pass the decoded raw bytes (not `reply.0`) to `parse_sync_response`. Keep ADR-007 envelope intact at the driver. **Do NOT touch the scan path** (`handle_scan_reply`) — scan replies are not envelope-wrapped (they're engine_loop-serialized JSON row arrays). |
| **Target file:line** | `worktrees/sync-v2-report/crates/helix-im/src/module.rs` — inside `handle_sync_reply` (fn starts at `:517`; the `parse_sync_response(reply.0.as_ref(), ...)` call to wrap is ~`:528`; SESSION cites the bug anchor as `:491`). New decode helper lives in helix-im (see seam below). |
| **ADR-007 seam concern — base64 cross-crate (the main gotcha)** | `base64_encode` lives in `helix-driver-host` (`http_envelope.rs:51`). **helix-im must NOT depend on helix-driver-host** (would invert the dependency arrow: driver → core/business, never reverse — HX-C001). So the matching **base64 *decode* cannot be imported from host.** Two ADR-007-legal options (SESSION: "im 自带 decode 或挪 core"): **(A) helix-im self-contains a small inline base64 decode** (mirror of the encode table `ABC…+/`, no new crate dep — exactly how both feat test files inline `base64_std`); **(B) move base64 encode+decode into helix-core** as a shared utility, both host (encode) and im (decode) import from core. Recommendation: **(A) inline decode in helix-im** for Phase A (smallest blast radius, zero new dep, no core API surface change); revisit (B) only if a 3rd consumer appears. Either way: **add NO `base64` crate dev-dep / dep** (the feat tests deliberately avoid it). |

---

## 2. Sync-reply data flow (numbered — ready for mermaid)

Post-fix path for a SyncPull HTTP reply on dev-zlc (ADR-007 + Phase A fixes applied):

1. **Driver HTTP**: `run_http_envelope` (host) executes the sync POST; on 2xx wraps response → `{status, headers, body: base64(raw_go_body)}` JSON, emits as `PortOutcome::Ok(ReplyBytes)`. (UNCHANGED — ADR-007 envelope kept.)
2. **Tick → handle_port_reply**: PortReply routed by corr; `corr_map` resolves this corr as a SyncPull context → dispatches to `handle_sync_reply(corr, channel_id, reply, out)`.
3. **[NEW #6] Envelope parse**: parse `reply.0` bytes as JSON envelope object; extract `body` (base64 string), `status`, `headers`.
4. **[NEW #6] base64 decode**: decode `body` → raw Go body bytes (via helix-im's self-contained inline decoder — NOT host's, cross-crate rule).
5. **parse_sync_response(raw_go_body, channel_id)**: now sees real `data.entries` → returns one of `NoChange` / `Events{events,messages,next_seq,needs_continuation}` / `TooLong{reset_to}` / `Snapshot`.
6. **[#1] cursor seed (upstream, on increment frame)**: a brand-new channel was seeded `cursor = 0` (not lastEventSeq) back in `apply_increment` → so the sync `from_seq = 0` → server actually returns offline events here (instead of `no_change`).
7. **Events branch — commit_target**: `commit_target = next_seq.max(max_event_seq)` (cursor anchors server-authoritative `next_seq`, skips trailing sparse/phantom holes).
8. **Persist (落库)**: `batch_upsert_events_with_messages(&events, &messages)` → `Effect::Persist{corr: persist_corr, ops}`; `pending_commits.insert(persist_corr, commit_target)`; `corr_map` records `ChannelPersist{channel_id, wants_continuation: needs_continuation}`.
9. **Emit (mutation events)**: `sync_mutation_emits(&events, &messages)` → per-event `im:*` emits (post:received/updated/deleted/read + channel:update-by-post). Phantom-aware (same visibility口径 as persist). emit does NOT advance cursor.
10. **cursor advance**: only when the Persist's PortReply returns Ok does cursor advance to `commit_target` (HX-C008 — persist Ok is the cursor authority, not emit).
11. **continuation**: if `needs_continuation` was true and a follow-up is warranted, next sync pull fires (`from_seq` now = advanced cursor). Backfill loop continues until server says caught up.

(Alt branches from step 5: `TooLong{reset_to}` → clear buffer + reset cursor to `reset_to-1` + emit; `Snapshot` → clear + batch upsert + cursor jump to `reset_to`. Neither is changed by Phase A.)

---

## 3. Acceptance tests feat uses for Phase A

### `http6_sync_reply_unwrap_test.rs` (#6 contract)

- **`http6_raw_go_body_drives_persist_and_emit`** — feeds the **decoded raw Go body** (3 visible events with messages map) on the sync-pull corr; asserts `persist_count > 0` AND `emit_count > 0` (data flows through).
- **`http6_legacy_envelope_is_silently_swallowed`** — feeds the **same 3 events wrapped in the legacy `{status,headers,body:base64(raw)}` envelope**; asserts `persist_count == 0` AND `emit_count == 0`. This locks the bug-in-place direction: if anyone feeds the parser an un-unwrapped envelope, data IS silently lost. Two-way assertion nails the contract.
- Note: test inlines its own `base64_std(...)` encoder "避免给 helix-im 加 base64 dev-dep" — confirms the no-new-dep constraint, and that the helix-im decode must be self-contained.

### `b2_increment_heal_test.rs` (#1 seed + B2 heal)

- **`cold_start_brand_new_channel_syncs_from_zero_not_watermark`** — no `register_channel`; ingests `inc(21, 999, true)`. Asserts `cursor_for(21) == Some(0)` (NOT 999), and after global-end the sync `from_seq == 0` (NOT 999, else Go no_change).
- **`cold_start_skipped_channel_healed_with_from_zero`** — `inc(22, 999, false)` (need_sync=false → skip set). Asserts `cursor_for(22) == Some(0)`; B2 heal detects `cursor(0) < target(999) = stuck` → emits 1 sync with `from_seq == 0`.
- **`heal_resyncs_skipped_channel_lagging_behind_increment_watermark`** — register cursor=20, watermark=999, need_sync=false → heal补发 1 POST sync.
- **`heal_skips_caughtup_channel`** — cursor==watermark==500 → no heal (not stuck).
- **`heal_runs_once_per_session_batch`** — once-only guard; second global-end same session does not re-fire.
- **`heal_resets_and_runs_again_on_new_session`** — after reconnect (`reset_increment_batch`) heal can fire again.
- (+ inflight-guard dedup test for need_sync=true lagging channel.)

### `increment_parse_test.rs` / `increment_flow_test.rs` (supporting — parser/flow contract)

- `increment_channel_real_frame_dispatches_through_registry`, `increment_channel_needsync_defaults_true_when_absent`, `increment_channel_needsync_false_respected`, `event_seq_fallback_*` — these assert the increment frame parses and dispatches correctly and that `need_sync` defaulting / `event_seq` fallback are preserved. They exercise the path #1 lives on (so the seed change must not break them). `port_reply_with_body`, `ev`, `sync_reply_events_with_messages` helpers already exist in dev-zlc `tests/common.rs` (180/301/316).

---

## 4. Risks / gotchas

1. **base64 placement (MAIN risk).** The decode must live in helix-im or helix-core — NEVER imported from helix-driver-host (HX-C001 dependency arrow). Recommended: inline self-contained decoder in helix-im, no `base64` crate dep (feat tests prove this is the intended shape). Risk if violated → `cargo check` cross-crate dep cycle / reversed dependency = architecture drift (a DRIFT-PRED trigger per SESSION §0).
2. **Scope creep — only SyncPull path.** The unwrap+decode must be gated to `handle_sync_reply` (SyncPull). Do NOT add it to `handle_scan_reply` (scan replies are NOT envelope-wrapped — they're engine_loop JSON row arrays). Mixing them breaks scan cursor loading.
3. **Do NOT remove the envelope.** feat removed it; dev-zlc's ADR-007 ruling keeps it (shared host module, native+FFI parity). Re-port direction is *additive decode on consumer side*, not *driver simplification*. Removing `run_http_envelope` would break FFI parity and contradict the ADR-007 decision.
4. **Decode robustness (零信任).** Envelope/base64 decode is a boundary input → no `unwrap`/`expect`; a malformed envelope or bad base64 must degrade to a `Result` error (logged), not panic and not silently NoChange-with-wrong-bytes. Mirror the `.map_err(ImError::Parse)` style already in `handle_sync_reply`.
5. **#1 and #6 are independent but synergistic.** Either fix alone is insufficient end-to-end: #1 ensures `from_seq=0` so the server *returns* offline events; #6 ensures those returned bytes are *actually parsed* instead of swallowed. Both required for the `http6_raw_go_body_drives_persist_and_emit` + `cold_start_*_from_zero` acceptance to pass together.
6. **HX-C010 determinism (bundled with #1's sibling file).** `stuck.sort_unstable()` in `increment_channel_end.rs` is a separate but co-located fix; easy to miss since it's not in the #1 anchor file. Re-port both together.
