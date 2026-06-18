# audit-B — Phase B re-port enumeration (members + channel three-path write + member_count)

> READ-ONLY audit. Compares FEAT (`feat/sync-v2-full-replacement`, real-machine verified) vs DEV-ZLC
> (`dev-zlc`, ADR-007 arch). Diff base: `git diff dev-zlc...feat/sync-v2-full-replacement`.
> Scope = #2 members (memberChange.join) · channel three-path write layer (`channel_write/`) · member_count.
> Behavior source-of-truth = 现网 cses-client `src-tauri/src/features/im/`; column contract =
> `crates/helix-im/docs/csesapi-migration/channel-52col-mapping.md` (NEW on feat, 218 lines).

---

## 0. ⚠️ HARD PREREQUISITE — Phase B does NOT compile on dev-zlc as-is (core/driver substrate missing)

`channel_write/` + its ACL effect factories depend on **helix-core + helix-driver** primitives that
dev-zlc lacks. These are NOT in the Phase B file list but are **blocking prerequisites** (likely Phase A).
Re-porting channel_write before these land = compile failure.

| Prerequisite (feat has, dev-zlc lacks) | Where | Used by Phase B |
|---|---|---|
| `StorageOp::GuardedBump(GuardedBumpSpec)` variant + struct | `helix-core/src/effect.rs` (~L92, ~L201) | path3 未读 +1 SQL 自增 (`bump_channel_unread_op`) |
| `UpsertSpec.exclude_from_update: Vec<&'static str>` field + `UpsertSpec::new()` | `helix-core/src/effect.rs` (~L124) | path1 守卫/本地列 ON CONFLICT 排除 (`upsert_channel_full`); member upsert |
| `MonotonicUpsertSpec` extra fields `table`/`key_col`/`value_col`/`touch_col` | `helix-core/src/effect.rs` (~L121) | `advance_cursor` → `channel_event_cursor` |
| `Storage::guarded_bump()` port method | `helix-core/src/ports/storage.rs` (~L68) | driver-side execution of GuardedBump |
| host/native driver impls of `guarded_bump` + `exclude_from_update` SQL + composite `conflict_key` ("channel_id,user_id") | `helix-driver-host/src/{ops,storage}.rs`, `helix-driver-native/src/{storage,engine_loop}.rs` | all three paths land via driver |

dev-zlc verification: `grep guarded_bump|exclude_from_update|GuardedBump` over `helix-core/src/ports/`
and `helix-driver-host/src/` → **0 hits**. dev-zlc `MonotonicUpsertSpec` = `{scope_key, value}` only.

**Action**: confirm these landed (Phase A) before/with Phase B, OR include them in the Phase B re-port batch.

---

## 1. Per-item re-port checklist

### 1a. NEW module `crates/helix-im/src/channel_write/` (5 files — ALL NEW on dev-zlc)

dev-zlc has NO `channel_write/` dir. Create all 5. Each ≤300-line hard cap (code-structure §1).

| File | Lines (feat) | What it does | Re-port note |
|---|---|---|---|
| **`mod.rs`** 🆕 | 81 | Module doc (3 paths + 2 intentional deviations), `pub type ChannelCol = (&'static str, SqlValue)`, `pub const ALLOWED_COLUMNS` (path2/3 white-list, **42 cols**; excludes member_count/subtopics_loaded_at), re-exports. | Pure types + whitelist. |
| **`program.rs`** 🆕 | 231 | **path1** `program_full(data, auth_user_id, now_ms) -> Option<(Vec<ChannelCol>, Vec<&'static str> omit)>`: full-52-col upsert row + JSON rename + guard/local/placeholder cols → `omit` (ON CONFLICT exclude). id-invalid → None. | Returns `(row, omit)`; omit drives `exclude_from_update`. |
| **`patch.rs`** 🆕 | 165 | **path2** `collect_present(data) -> Vec<ChannelCol>`: `update_channel` PATCH — only `Some` whitelist fields (STR/INT/BOOL/JSON maps); members/owner/admin/boss excluded. | Empty cols → upper layer no-op. |
| **`post.rs`** 🆕 | 257 | **path3** `PostChannelUpdate` struct + `post_updates(...)` (online WS, eats `serde_json::Value`) + `post_updates_from_fields(...)` (offline sync, eats `PostFields`). visible + should_increment predicates → unread_delta {0,1}. | Has inline `#[cfg(test)] mod tests` (5 tests). |
| **`members.rs`** 🆕 | 217 | **#2 members** `collect_members(data) -> Vec<MemberRow>` (4-source members/adminUsers/boss/owner + `memberChange.join`, role normalize, dedup keep-last) + `collect_member_leaves(data) -> Vec<String>` (parsed but NO consumer — core lacks Delete). | `MemberRow` is from `acl::to_effect_s1` (see 1c). |

`crates/helix-im/src/lib.rs`: add `pub mod channel_write;` (+1 line on feat).

### 1b. `#2 members (memberChange.join)` handling

- **Collection** (`members.rs::collect_members`): merges **4-source** (`members[]`→MEMBER, `adminUsers[]`→ADMIN,
  `boss[]`→BOSS, `owner{}`→OWNER) **+ `memberChange.join[]`** (role `?? MEMBER`), then dedup keep-last.
  Order: members→adminUsers→boss→owner→join (join after 4-source → incremental role override). O(n), cold path.
- **A1 fix root cause**: feat consolidated 3 wire paths (increment / channel_created / channel_member_update)
  onto `collect_members`. Pre-fix `apply_increment` read 4-source only → real-Go increment frames deliver
  new members via `memberChange.join` → 43-person group landed only 3 sparse `members[]` rows.
- **owner-as-MEMBER skip**: join entries where `user_id == owner_id && role==MEMBER` are skipped
  (owner already in 4-source as OWNER, not downgraded).
- **`memberChange.leave`** (`collect_member_leaves`): parses leave user_id set but **has NO consumer** —
  helix-core `StorageOp` has no `Delete` variant. Intentional deviation, tracked ROADMAP-P. Re-port the
  function (test anchor) but wire-up waits for a future core `BatchDelete`.

### 1c. ACL effect factories (consumers of channel_write — re-port these too)

| File | Status | What to add |
|---|---|---|
| `crates/helix-im/src/acl/to_effect_s1.rs` | 🆕 NEW (183 lines) | `pub struct MemberRow {user_id, team_id, role, nick_name}`, `upsert_channel_members(channel_id, Vec<MemberRow>) -> Option<Effect>` (BatchUpsert `channel_member`, conflict_key `"channel_id,user_id"`), `update_member_nickname(...)` (composite-PK upsert, exclude role/team_id), plus S1 emit_* + `channel_set_cols`. |
| `crates/helix-im/src/acl/to_effect.rs` | MODIFY (+224) | Add `upsert_channel_full(cols, exclude)` (path1, BatchUpsert `channel` conflict_key `"id"`, `exclude_from_update`), `update_channel_partial(channel_id, cols) -> Option<Effect>` (path2/3, BatchUpdate WHERE id), `bump_channel_unread(upd)` + `bump_channel_unread_op(upd)` (path3 GuardedBump), `sync_unread_bump_ops(...)` (offline sync +N to DB). Also `advance_cursor` retargeted to `channel_event_cursor` w/ table/key/value/touch cols. Also add `exclude_from_update: Vec::new()` to existing `upsert_message`. |
| `crates/helix-im/src/acl/mod.rs` | MODIFY (+1) | `pub mod to_effect_s1;` |

### 1d. Handler wiring (`apply_increment` consumes channel_write)

`crates/helix-im/src/ws/handlers/increment_channel.rs` (MODIFY +40):
1. `Channel::new(channel_id, 0)` — **seed cursor 0 NOT lastEventSeq** (HX-C008; lastEventSeq is server
   high-water, feeds `increment_target` only — critical correctness fix; pre-fix broke offline backfill + B2 heal).
2. `program_full(&data, ctx.auth_user_id, ctx.now_ms)` → `upsert_channel_full(cols, exclude)`.
3. `collect_members(&data)` → `upsert_channel_members(channel_id, members)`.

Sibling handlers also use channel_write (out of strict Phase-B but part of the layer — feat NEW files):
`channel_created.rs` (program_full + collect_members), `channel_member_update.rs` (program_full + collect_members),
`update_channel.rs` (collect_present), `post.rs` (post_updates). These are NEW handler files on feat;
re-port if Phase B scope includes them (they exercise the same write layer).

### 1e. `ImConfig.auth_user_id` (module.rs +113, partial)

Add `pub auth_user_id: String` to `ImConfig` (+ Default ""). Thread `ctx.auth_user_id` through every
`ImWsContext::new(...)` call (signature gains a param — `ws/context.rs` +5). Needed by path1 (channel
`user_id` col) + path3 (sender exemption + visibility gate). Empty = no identity → conservative count-in.

### 1f. `schema.rs` delta (+25) — `channel_event_cursor` table

⚠️ **NOT member/channel-write related.** dev-zlc's `channel` table (incl. `member_count` col) and
`channel_member` table (composite PK) **already exist**. The +25 delta is purely the NEW
`channel_event_cursor (channel_id PK, last_event_seq, updated_at)` table (a cursor/sync concern), replacing
the deleted-`_helix_monotonic`-as-cursor approach. Re-port the table DDL; it pairs with `advance_cursor`
retarget + `module.rs` on_start `Scan(channel_event_cursor)` + `handle_scan_reply` (channel_id/last_event_seq
cols) + `state.rs` `CorrelationContext::ScanCursors` doc.

### 1g. `parser.rs` delta (+52) — viewers + lastEventSeq optional + camel/snake

Adds `PostFields.viewers: Vec<String>` (path3 visibility); makes `lastEventSeq` **Optional** in
`parse_increment_channel` (real-Go GetMaxSeq degrade — pre-fix `?` early-return dropped 650 frames →
0 rows landed); camel/snake dual-form for `channelId`/`nextSeq`/`resetTo`/`reset_to` in `parse_sync_response`.
`sync_session.rs` (+4): add `viewers` field to `PostFields`.

---

## 2. Channel three-path write data flow (for mermaid — numbered steps)

References `channel-52col-mapping.md` (§1 path1 / §2 path2 / §3 path3).

```
WS frame ──► parser ──► handler ──► channel_write (pure calc) ──► acl/to_effect (Effect) ──► driver SQL
```

**path1 — full upsert (increment_channel / channel_created / channel_member_update)**
1. WS `increment_channel` frame → `inc.raw` bytes (parser zero-trust validated id).
2. `apply_increment`: `serde_json::from_slice(inc.raw)` → `data`.
3. `program_full(&data, auth_user_id, now_ms)` → `(row: 53 cols incl member_count, omit)`. JSON rename
   (displayName→display_name, channelIsTop→is_top, createAt→created_at…); user_id col = auth_user_id;
   mention_user = "" fixed; target_users only if type=="D"; members/owner/admin_users/boss = placeholder
   "" / "{}" / "[]" / "[]" (real values → channel_member table, mapping §0.2).
4. Guard/local cols → `omit`: unread_count, unread_post_id, purpose (server None→keep local), is_remove,
   draft (local), members/owner/admin_users/boss (placeholders), member_count (server None→keep local).
5. `upsert_channel_full(cols, omit)` → `Effect::PersistFire[ BatchUpsert{ table:"channel",
   conflict_key:"id", exclude_from_update: omit } ]`. → mapping §1 (52-col + member_count guard).
6. Driver: `INSERT … ON CONFLICT(id) DO UPDATE` excluding `omit` cols (O(1) WHERE PK).

**members sub-path (parallel with path1)**
7. `collect_members(&data)` → `Vec<MemberRow>` (4-source + memberChange.join, dedup keep-last,
   role normalize OWNER/ADMIN/BOSS/MEMBER). → mapping §0.2 (real members → channel_member table).
8. `upsert_channel_members(channel_id, members)` → `BatchUpsert{ table:"channel_member",
   conflict_key:"channel_id,user_id" }` (composite PK). Empty → None (no-op).
9. `memberChange.leave` → `collect_member_leaves` (parsed, NO consumer — core lacks Delete).

**path2 — partial PATCH (update_channel)**
10. WS `update_channel` frame → `data`.
11. `collect_present(&data)` → `Vec<ChannelCol>` (only `Some` whitelist fields: STR/INT/BOOL/JSON maps;
    None/absent skipped = DB keeps existing). owner/admin/boss/member_count/subtopics excluded. → mapping §2.
12. `update_channel_partial(channel_id, cols)` → `Option<Effect>` (BatchUpdate `channel` SET cols WHERE id).
    Empty → None. Local row absent → 0 rows hit = no-op.

**path3 — new-message trigger (post handler + offline sync apply)**
13. New post (online WS via `post_updates(data, auth, msg_create_at)` OR offline sync via
    `post_updates_from_fields(channel_id, PostFields, auth)`) → `PostChannelUpdate`.
14. Predicates: **visible** = type=="NOTICE" ‖ auth empty (conservative) ‖ viewers∋"all"/auth;
    **should_increment** = sender != auth (self-message exempt). unread_delta = visible&&increment ? 1 : 0.
15. `bump_channel_unread_op(upd)` → `GuardedBump{ table:"channel", bump_col:"unread_count",
    bump_delta, set_cols:[unread_post_id?, last_post?, has_schedule_post?, last_root_post_at],
    guard_col:"last_root_post_at", guard_val:msg_create_at }`. → mapping §3.
16. Driver: `UPDATE channel SET unread_count = unread_count + delta, … WHERE id=? AND ? > last_root_post_at`
    (O(1), no read — see §3 below). Offline: `sync_unread_bump_ops` folds same op into the message-persist
    batch (same corr → unread col lands before cursor advances; CAP-9 online/offline parity).

---

## 3. member_count write strategy + HX-C005 O(1) compliance

- **member_count is NOT written by path3** (post path). It is a **path1-only guarded column**.
- **path1** (`program.rs` L220-228): `match data.get("memberCount").as_i64()`:
  - `Some(v)` → `("member_count", Integer(v))`, **NOT** in `omit` → ON CONFLICT overwrites (server authoritative).
  - `None` → `("member_count", Integer(0))` + pushed to `omit` → ON CONFLICT keeps local (degrade-frame safe).
  - This is the real-Go increment-frame value (实测 memberCount 6/9/2/1 @localhost:8065). Pre-fix the whole
    column was missing → channel.member_count stuck at 0 (real-connect Go bug, test
    `program_full_server_member_count_present_not_omitted_s3`).
- **path2** (`patch.rs`): member_count NOT in `ALLOWED_COLUMNS` → cannot be PATCHed (protected local-vs-server col).
- **HX-C005 O(1)**: member_count writes are a single `INSERT … ON CONFLICT(id) DO UPDATE SET member_count=excluded`
  (WHERE PK=id) — **NOT** read-modify-write. ✅ O(1) compliant. It rides path1's single-row upsert (cold path,
  not per-message hot path).
- **Contrast — the actual hot-path counter is `unread_count`** (path3, per-message): feat uses `GuardedBump`
  (`SET unread_count = unread_count + delta … WHERE id=? AND ? > last_root_post_at`) — single SQL self-increment,
  **no RMW**. This is the spec's ONE intentional deviation from 现网 (现网 path3 does app-layer
  `local.unread+1` RMW; mapping §3 铁律 2). ✅ HX-C005 compliant, behavior-equivalent (N msgs → +N).

---

## 4. Acceptance tests feat uses

### `tests/channel_member_collect_test.rs` (NEW, 196 lines, 7 tests — #2 members)
- `collect_members_maps_roles_and_dedups` — 4-source role map (MEMBER/ADMIN/BOSS/OWNER) + dedup keep-last
  (members+adminUsers u_dup → ADMIN, 1 row); owner team_id/nick_name passthrough.
- `collect_members_explicit_role_normalized` — MANAGER→ADMIN, CREATOR→OWNER; userId key also accepted.
- `collect_members_stringified_and_empty` — stringified array (old server tag) compat; empty sources → empty.
- `collect_members_merges_member_change_join` — **A1 core**: sparse members[]+owner + 3 join entries → 5 rows
  total, no loss; join default role MEMBER, explicit role honored, userId key works, teamId/nickName passthrough.
- `collect_members_join_overrides_four_source_role` — same user_id in members + join → 1 row, join role wins.
- `collect_members_join_skips_owner_as_member` — owner-as-MEMBER join entry skipped (stays OWNER), 1 row.
- `collect_member_leaves_extracts_user_ids` — leave set parse (id ?? userId, empty/missing skipped);
  no memberChange / no leave → empty.

### `tests/channel_write_program_test.rs` (NEW, 138 lines, 5 tests — path1)
- `program_full_emits_52_cols_with_guards_omitted_s3` — `row.len()==53` (52 + member_count guard);
  JSON rename asserts; mention_user "" fixed; omit contains {unread_count, unread_post_id, purpose,
  is_remove, draft, members, owner, admin_users, boss, member_count}; created_at/updated_at = server values.
- `program_full_now_fallback_and_dm_target_users_s3` — created/updated fallback to now_ms; target_users
  only for type=="D".
- `program_full_server_member_count_present_not_omitted_s3` — memberCount:9 → member_count=9 AND NOT omitted.
- `program_full_server_unread_present_not_omitted_s3` — unread/purpose server-present → not omitted.
- `program_full_invalid_id_none_s3` — missing/short id → None (zero-trust).

### Inline `#[cfg(test)]` (re-port with their files)
- `post.rs::tests` (5): other-visible→+1, self→+0, invisible→+0 (last_post empty), userSnapshot priority, NOTICE+schedule.
- `patch.rs::tests` (3): skip None fields, exclude member/owner/local cols, empty→empty.

> Also (broader Phase, same layer): `s1_ws_actions_test.rs` (576), `s4_write_order_test.rs` (183),
> `s5_post_echo_test.rs` (162), `b2_increment_heal_test.rs` (88) — exercise the wired handlers end-to-end.

---

## 5. Risks

1. **🔴 Core/driver prerequisite (§0)** — biggest risk. `GuardedBump`, `exclude_from_update`,
   `MonotonicUpsertSpec` field expansion, `guarded_bump` port + driver impls, composite `conflict_key`
   SQL support — ALL absent on dev-zlc. channel_write won't compile/run without them. Confirm Phase A
   landed or batch them in. Touches all 3 layers (core trait → host/native driver) → HX-C001 + FFI parity.
2. **Schema migration** — `channel_event_cursor` is additive (`CREATE TABLE IF NOT EXISTS`, byte-aligned to
   现网 678 migrate). LOW risk. Note: `channel`/`channel_member` tables already on dev-zlc — do NOT re-create.
   But the `advance_cursor` retarget (`_helix_monotonic` → `channel_event_cursor`) + on_start Scan + scan_reply
   parsing (scope_key/value → channel_id/last_event_seq) must move together or cursor restore breaks.
3. **300-line file limit (code-structure §1 HARD CAP for `src/**`)** — feat keeps each channel_write file
   under 300 (program 231, post 257, members 217, patch 165, mod 81) by moving program/member tests OUT to
   `tests/`. **`acl/to_effect.rs` is an existing over-limit "decrement-only" file** — adding +224 risks
   pushing it further; feat split S1 factories into the NEW `to_effect_s1.rs` precisely for this. Re-port must
   keep the to_effect_s1 split (do NOT inline back into to_effect.rs).
4. **Hot-path O(1) (HX-C005)** — path3 unread MUST be GuardedBump (SQL self-increment), never app-layer
   read-modify-write. The 现网 RMW is the explicit thing NOT to copy (mapping §3 铁律 2). member_count rides
   path1 single-row upsert (cold). ✅ if prerequisites land.
5. **Identity threading** — `auth_user_id` empty (no identity) degrades to conservative count-in; §5 acceptance
   ("self +0 / invisible +0 / echo no unread") only holds when host injects real auth (BLOCKING-1 root cause).
   ImWsContext signature change ripples to every call site.
6. **memberChange.leave gap** — leave members never deleted (core no Delete). `collect_member_leaves` ported as
   dead-but-tested anchor; documented ROADMAP-P. Acceptable known deviation, but document so it isn't "fixed" later.
