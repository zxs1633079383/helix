# Outbound Contracts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix the confirmed ZLC review issues 1/2/3: outbound HTTP method fidelity, default session auth intent, and query projection documentation drift.

**Architecture:** Keep HTTP request assembly centralized in `helix-im/src/outbound/registry.rs`. Commands only declare endpoint/body/method/auth intent; drivers still inject real credentials and perform I/O. Do not touch `im_delete_all_dialogs`, storage ports, or `helix-core` runtime boundaries in this plan.

**Tech Stack:** Rust 2021 workspace, `helix-im`, `helix-driver-host`, `helix-driver-native`, `helix-driver-ffi`, Superpowers TDD workflow.

## Global Constraints

- 使用中文汇报。
- `helix-core` 不新增运行时/平台依赖。
- 不改 `im_delete_all_dialogs` 行为，不新增 truncate/delete-all storage 原语。
- 修改符号前跑 GitNexus impact；提交前跑 detect_changes。
- TDD：先写失败测试并确认 RED，再写最小实现。

---

## File Structure

- Modify `crates/helix-im/src/outbound/registry.rs`
  - Add command-level HTTP method declaration and emit explicit session auth intent.
- Modify command modules under `crates/helix-im/src/outbound/`
  - Override method only for known GET/DELETE commands.
- Modify `crates/helix-im/tests/p5a_user_misc_test.rs`
  - Assert `im_team_quit` emits DELETE.
- Modify `crates/helix-im/tests/p5b_bot_agent_test.rs`
  - Assert selected bot-agent GET/DELETE commands emit correct methods.
- Modify `crates/helix-driver-host/tests/p1_5_http_crosscut_test.rs`
  - Add or adjust coverage proving session auth intent injects and strips correctly.
- Modify `crates/helix-im/src/query.rs` and docs if needed
  - Align query result comments with `{channel_id,messages}` implementation.

### Task 1: HTTP Method Fidelity

**Files:**
- Modify: `crates/helix-im/tests/p5a_user_misc_test.rs`
- Modify: `crates/helix-im/tests/p5b_bot_agent_test.rs`
- Modify: `crates/helix-im/src/outbound/registry.rs`
- Modify: selected `crates/helix-im/src/outbound/*.rs`

**Interfaces:**
- Consumes: `OutboundCommand::build(&Value) -> Result<(&'static str, Value), ImError>`.
- Produces: `OutboundCommand::method(&self) -> &'static str`, default `"POST"`.

- [ ] **Step 1: Write failing tests**
  - Add method extraction assertions for `im_team_quit`, one bot-agent GET command, and one bot-agent DELETE command.

- [ ] **Step 2: Run RED**
  - Run: `cargo test -p helix-im p5a_user_misc_test::team_commands_match_existing_contracts p5b_bot_agent_test::bot_agent_methods_match_existing_contracts`
  - Expected: FAIL because all commands currently emit POST.

- [ ] **Step 3: Implement minimal method support**
  - Add `fn method(&self) -> &'static str { "POST" }` to `OutboundCommand`.
  - Rename `http_post` to neutral request builder and pass `command.method()`.
  - Override only known GET/DELETE commands.

- [ ] **Step 4: Run GREEN**
  - Run: `cargo test -p helix-im p5a_user_misc_test p5b_bot_agent_test`
  - Expected: PASS.

### Task 2: Default Session Auth Intent

**Files:**
- Modify: `crates/helix-im/src/outbound/registry.rs`
- Modify: `crates/helix-im/tests/p5a_user_misc_test.rs` or another outbound test helper
- Modify: `crates/helix-driver-host/tests/p1_5_http_crosscut_test.rs`

**Interfaces:**
- Consumes: `AUTH_KIND_HEADER`, `AuthKind::Session`, `AuthKind::Bot`.
- Produces: outbound session commands explicitly include `X-Auth-Kind: session`; host crosscut injects token and strips the intent header.

- [ ] **Step 1: Write failing tests**
  - Assert a default session outbound includes `X-Auth-Kind: session`.
  - Add an end-to-end host crosscut assertion that session intent injects `cookieId` and strips `X-Auth-Kind`.

- [ ] **Step 2: Run RED**
  - Run: `cargo test -p helix-im p5a_user_misc_test && cargo test -p helix-driver-host --test p1_5_http_crosscut_test`
  - Expected: helix-im assertion fails before implementation.

- [ ] **Step 3: Implement minimal header change**
  - Always push `AUTH_KIND_HEADER` with `auth_kind.as_str()` in outbound request assembly.
  - Preserve bot behavior and driver stripping semantics.

- [ ] **Step 4: Run GREEN**
  - Run: `cargo test -p helix-im p5a_user_misc_test && cargo test -p helix-driver-host --test p1_5_http_crosscut_test`
  - Expected: PASS.

### Task 3: Query Projection Documentation Alignment

**Files:**
- Modify: `crates/helix-im/src/query.rs`
- Inspect/modify: `crates/helix-im/docs/projection-schema.md`
- Inspect/modify: `crates/helix-im/docs/takeover-plan/04-cses-client接入方案.md`

**Interfaces:**
- Consumes: existing implementation `emit_message_query_result(channel_id, rows)` emits `data: { channel_id, messages }`.
- Produces: docs/comments no longer claim this event uses array payload wrapping.

- [ ] **Step 1: Write/confirm doc drift target**
  - Search: `rg -n 'messages:query_result|items' crates/helix-im/src/query.rs crates/helix-im/docs`

- [ ] **Step 2: Edit comments/docs only**
  - Update query result language to `{channel_id,messages}`.
  - Keep array `{items:[]}` language only for events whose `data` is itself an array.

- [ ] **Step 3: Verify schema tests**
  - Run: `cargo test -p helix-im projection_schema_test::im_query_messages_result_freezes_shape`
  - Expected: PASS with existing `{channel_id,messages}` contract.

### Task 4: Final Verification

**Files:**
- All files modified by Tasks 1-3.

- [ ] **Step 1: Run focused tests**
  - Run: `cargo test -p helix-im p5a_user_misc_test p5b_bot_agent_test`
  - Run: `cargo test -p helix-driver-host --test p1_5_http_crosscut_test`

- [ ] **Step 2: Run broader checks**
  - Run: `cargo test -p helix-im`
  - Run: `cargo check -p helix-core -p helix-im -p helix-driver-native -p helix-driver-ffi`
  - Run: `cargo tree -p helix-core | rg -i 'tokio|sqlx|reqwest|web-sys|tauri'`
  - Run: `git diff --check`

- [ ] **Step 3: Run GitNexus detect_changes**
  - Run: `detect_changes({scope:"all", repo:"helix"})`
  - Expected: affected scope matches outbound registry/tests/docs and host crosscut tests.

- [ ] **Step 4: Session closeout**
  - Add `✅ DONE` or `⚡ DRIFT-PRED` in `SESSION.md §0` with exact verification evidence.
