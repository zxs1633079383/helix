# Multitenant Local DB Real Validation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add shared local store path resolution and prove real file-backed SQLite isolation for users `444` and `678`.

**Architecture:** `helix-driver-host` owns the shared local store locator and returns a file-backed `sqlite:file:<path>` URL. `HostStorage` remains the only SQLite execution layer. `helix-host-cli` keeps its public DB helper but delegates derived paths to the shared locator, while `runtime.db` explicit override still wins.

**Tech Stack:** Rust 2021, std filesystem/path APIs, serde/serde_json already present in `helix-driver-host`, rusqlite through existing `HostStorage`, `helix_im::IM_SCHEMA` for real schema setup.

## Global Constraints

- Do not modify `crates/helix-core/**`.
- Do not add a `tenant_id` column to IM tables.
- Do not add new dependencies unless the task explicitly says so; this plan uses a small deterministic FNV-1a helper instead of adding a hash crate.
- Local store directory layout is `<base_dir>/data/<local_store_key>/tenant.db`.
- `LocalStoreKey` format is `s_<16 lowercase hex chars>`.
- Required identity fields are app namespace, environment, API base URL, WS URL, company ID, account ID, and schema family.
- Manifests must not contain token, cookie, authorization, password, or secret fields.
- Real validation must use file-backed SQLite, not `mode=memory`.
- `runtime.db` explicit override remains highest priority in host-cli.
- Required verification commands: `cargo test -p helix-driver-host local_store -- --nocapture`, `cargo test -p helix-host-cli host_env -- --nocapture`, `cargo check -p helix-driver-host -p helix-host-cli`, `git diff --check`.

---

## File Structure

- Create `crates/helix-driver-host/src/local_store.rs`: local store identity, key, locator, manifest writing, deterministic hash, validation errors, unit tests.
- Modify `crates/helix-driver-host/src/lib.rs`: export local store types.
- Create `crates/helix-driver-host/tests/local_store_locator.rs`: real file-backed 444/678 SQLite isolation test.
- Modify `crates/helix-host-cli/Cargo.toml`: add direct `helix-driver-host` dependency.
- Modify `crates/helix-host-cli/src/host_env.rs`: replace local tenant-only path derivation with shared locator.
- Modify `crates/helix-host-cli/src/main.rs`: pass company and backend fields into `resolve_db_path`.
- Extend `crates/helix-host-cli/src/host_env.rs` unit tests: cover `runtime.db` override and 444/678 derived path split.
- Modify `SESSION.md`: add final DONE line after verification.

## Task 1: Add Shared Local Store Locator

**Files:**
- Create: `crates/helix-driver-host/src/local_store.rs`
- Modify: `crates/helix-driver-host/src/lib.rs`

**Interfaces:**
- Produces:
  - `BackendScope`
  - `LocalStoreIdentity`
  - `LocalStorePaths`
  - `LocalStoreLocator::new(base_dir)`
  - `LocalStoreLocator::resolve(&identity)`

- [x] **Step 1: Add failing unit tests inside `local_store.rs`**

Test names:

```rust
#[test]
fn same_identity_produces_stable_key() {}

#[test]
fn different_accounts_produce_different_paths() {}

#[test]
fn missing_required_fields_are_rejected() {}

#[test]
fn manifest_contains_identity_but_no_secret_fields() {}
```

Run:

```bash
cargo test -p helix-driver-host local_store -- --nocapture
```

Expected before implementation: compile failure because module/types do not exist.

- [x] **Step 2: Implement local store types**

Implement these public structs with `Debug, Clone` where useful:

```rust
pub struct BackendScope {
    pub environment: String,
    pub api_base_url: String,
    pub ws_url: String,
}

pub struct LocalStoreIdentity {
    pub app_namespace: String,
    pub backend: BackendScope,
    pub company_id: String,
    pub account_id: String,
    pub tenant_hint: Option<String>,
    pub schema_family: String,
}

pub struct LocalStorePaths {
    pub key: String,
    pub dir: std::path::PathBuf,
    pub db_path: std::path::PathBuf,
    pub db_url: String,
    pub manifest_path: std::path::PathBuf,
}

pub struct LocalStoreLocator {
    pub base_dir: std::path::PathBuf,
}
```

Implement `LocalStoreError` without `anyhow`:

```rust
#[derive(Debug, thiserror::Error)]
pub enum LocalStoreError {
    #[error("missing local store identity field: {0}")]
    MissingField(&'static str),
    #[error("create local store directory failed: {0}")]
    CreateDir(std::io::Error),
    #[error("write local store manifest failed: {0}")]
    WriteManifest(std::io::Error),
    #[error("serialize local store manifest failed: {0}")]
    SerializeManifest(serde_json::Error),
}
```

- [x] **Step 3: Implement deterministic key and manifest**

Use canonical JSON-like string construction in a fixed field order and FNV-1a:

```rust
fn fnv1a64(input: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for b in input.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
```

Key format:

```rust
format!("s_{:016x}", fnv1a64(&canonical))
```

Directory and DB URL:

```rust
let dir = self.base_dir.join("data").join(&key);
let db_path = dir.join("tenant.db");
let db_url = format!("sqlite:file:{}", db_path.to_string_lossy());
```

Manifest must be JSON and must include account IDs for debugging but no credential fields.

- [x] **Step 4: Re-export from lib.rs**

Add:

```rust
pub mod local_store;
pub use local_store::{BackendScope, LocalStoreIdentity, LocalStoreLocator, LocalStorePaths};
```

- [x] **Step 5: Verify Task 1**

Run:

```bash
cargo test -p helix-driver-host local_store -- --nocapture
```

Expected: all local_store unit tests pass.

## Task 2: Integrate host-cli DB Path Derivation

**Files:**
- Modify: `crates/helix-host-cli/Cargo.toml`
- Modify: `crates/helix-host-cli/src/host_env.rs`
- Modify: `crates/helix-host-cli/src/main.rs`
- Modify: `crates/helix-host-cli/src/config_tests.rs`

**Interfaces:**
- Consumes Task 1 exports from `helix_driver_host`.
- Produces host-cli `resolve_db_path(DbPathInputs)` with runtime override preserved.

- [x] **Step 1: Add host-cli direct dependency**

In `crates/helix-host-cli/Cargo.toml`, add:

```toml
helix-driver-host = { path = "../helix-driver-host" }
```

- [x] **Step 2: Change host_env API**

Replace:

```rust
pub fn resolve_db_path(cookie_id: &str, configured_db: &str, configured_tenant: &str) -> String
```

with:

```rust
pub struct DbPathInputs<'a> {
    pub cookie_id: &'a str,
    pub company_id: &'a str,
    pub configured_db: &'a str,
    pub configured_tenant: &'a str,
    pub api_base_url: &'a str,
    pub ws_url: &'a str,
}

pub fn resolve_db_path(inputs: DbPathInputs<'_>) -> String
```

Rules:

- If `configured_db` is non-empty, return it exactly.
- Otherwise use `$HOME/Library/Application Support/com.jinqidongli.cses.dev` as base dir.
- Build `LocalStoreIdentity`:
  - `app_namespace = "com.jinqidongli.cses.dev"`
  - `environment = "host-cli"`
  - `api_base_url = inputs.api_base_url`
  - `ws_url = inputs.ws_url`
  - `company_id = inputs.company_id`
  - `account_id = inputs.cookie_id`
  - `tenant_hint = configured_tenant if non-empty else cookie_id if non-empty`
  - `schema_family = "im-v1"`
- If `$HOME` is missing or directory creation fails, log warning and return a per-account `/tmp/helix_host_cli/data/<store-key>/tenant.db` path rather than a shared fallback DB.

- [x] **Step 3: Update main.rs call site**

Change DB path resolution to pass all fields:

```rust
let db_path = resolve_db_path(DbPathInputs {
    cookie_id: &cookie_id,
    company_id: &company_id,
    configured_db: &cfg.runtime.db,
    configured_tenant: &cfg.identity.tenant,
    api_base_url: &api_base_url,
    ws_url: &ws_url,
});
```

- [x] **Step 4: Add host_env tests**

Add tests near existing config/host_env tests:

```rust
#[test]
fn explicit_runtime_db_still_wins() {}

#[test]
fn users_444_and_678_resolve_to_different_tenant_db_paths() {}
```

These tests must set a temporary `HOME` and restore it after the test.

- [x] **Step 5: Verify Task 2**

Run:

```bash
cargo test -p helix-host-cli host_env -- --nocapture
```

Expected: host_env tests pass.

## Task 3: Add Real 444/678 File SQLite Isolation Test

**Files:**
- Create: `crates/helix-driver-host/tests/local_store_locator.rs`

**Interfaces:**
- Consumes Task 1 local store locator.
- Consumes `HostStorage`.
- Consumes `helix_im::IM_SCHEMA`.

- [x] **Step 1: Create integration test skeleton**

Create helper functions:

```rust
fn identity(account_id: &str) -> LocalStoreIdentity
async fn open_im_store(db_url: &str) -> HostStorage
async fn scan_messages(storage: &HostStorage) -> Vec<Row>
fn col(row: &Row, name: &str) -> Option<&SqlValue>
```

- [x] **Step 2: Write the real isolation test**

Test name:

```rust
#[tokio::test]
async fn real_file_sqlite_isolates_users_444_and_678()
```

Required assertions:

- keys differ
- dirs differ
- DB paths differ
- `paths_444.db_path.exists()`
- `paths_678.db_path.exists()`
- manifests exist
- manifest for `444` contains `"accountId":"444"` or pretty equivalent
- manifest for `678` contains `"accountId":"678"` or pretty equivalent
- scanning `678` after inserting into `444` returns zero rows
- reopening `444` sees message `"hello from 444"`

- [x] **Step 3: Insert a real message row via StorageOp**

Use `HostStorage::batch_upsert(UpsertSpec { table: "message", ... })`.

The row must include the columns required by `IM_SCHEMA`. If a required column is unclear, inspect `helix_im::IM_SCHEMA` and provide string/integer defaults. Use marker values:

```text
temporary_id = tmp-local-store-444
id = server-local-store-444
channel_id = chlocal000000000000000001
user_id = 444
message = hello from 444
```

- [x] **Step 4: Verify Task 3**

Run:

```bash
cargo test -p helix-driver-host real_file_sqlite_isolates_users_444_and_678 -- --nocapture
```

Expected: test passes and printed paths point to two different physical `tenant.db` files under `/tmp`.

## Task 4: Final Verification and Documentation Closure

**Files:**
- Modify: `SESSION.md`
- Optionally modify: `docs/过会方案/helix-多租户本地库架构设计.md` if implementation discovers a mismatch.

- [x] **Step 1: Run required verification**

Run:

```bash
cargo test -p helix-driver-host local_store -- --nocapture
cargo test -p helix-host-cli host_env -- --nocapture
cargo check -p helix-driver-host -p helix-host-cli
git diff --check
```

- [x] **Step 2: Confirm core untouched**

Run:

```bash
git diff -- crates/helix-core
```

Expected: no output.

- [x] **Step 3: Run GitNexus change detection before commit**

Run GitNexus `detect_changes({scope: "all", worktree: "/System/Volumes/Data/workspace/rust/helix/worktrees/multitenant-local-db-design"})`.

Expected: changed scope limited to driver-host, host-cli, docs, SESSION.

- [x] **Step 4: Commit in logical batches**

Suggested commits:

```bash
git add docs/过会方案/helix-多租户本地库架构设计.md docs/superpowers/specs/2026-07-06-multitenant-local-db-real-validation-design.md docs/superpowers/plans/2026-07-06-multitenant-local-db-real-validation.md SESSION.md
git commit -m "docs(storage): 沉淀多租户本地库验证方案"
```

Then implementation:

```bash
git add crates/helix-driver-host/src/local_store.rs crates/helix-driver-host/src/lib.rs crates/helix-driver-host/tests/local_store_locator.rs crates/helix-host-cli/Cargo.toml crates/helix-host-cli/src/host_env.rs crates/helix-host-cli/src/main.rs crates/helix-host-cli/src/config_tests.rs SESSION.md
git commit -m "feat(storage): 验证多租户本地库隔离"
```

If the final diff crosses 3 files or 50 lines, use the structured body required by `docs/COMMIT_CONVENTION.md`.

## Review Plan

After Task 1, dispatch a task reviewer with:

- Task scope: `local_store.rs` API, key stability, path safety, manifest safety.
- Must verify no `DefaultHasher`, no secrets in manifest, no `helix-core` diff.

After Task 2, dispatch a task reviewer with:

- Task scope: host-cli API change, `runtime.db` override, config field propagation.
- Must verify fallback behavior and no regression to tenant-only paths.

After Task 3, dispatch a task reviewer with:

- Task scope: real file SQLite validation.
- Must verify test is file-backed, uses 444/678, writes through `HostStorage`, and proves cross-read isolation.

After all tasks, dispatch a whole-branch reviewer with:

- Full diff package from merge-base to HEAD.
- Must review path security, test strength, host-cli compatibility, and Helix architecture boundaries.
