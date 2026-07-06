# Multitenant Local Database Real Validation Design

Date: 2026-07-06

## 1. Goal

Implement and verify Helix local database tenant isolation with real file-backed
SQLite databases for two test users: `444` and `678`.

The validation target is not a mock and not an in-memory database. It must create
two physical local store directories, two physical `tenant.db` files, and prove:

- user `444` and user `678` resolve to different local store keys
- their `tenant.db` files are different paths
- writes performed through `HostStorage` for `444` do not appear when scanning
  the `678` store
- reopening the `444` store reads the row back from disk

## 2. Non-goals

- Do not modify `helix-core`.
- Do not add a `tenant_id` column to IM tables.
- Do not implement FFI production config in this phase.
- Do not change HTTP/WS auth behavior.
- Do not hit the Go backend. This phase validates local store isolation only.
- Do not delete or migrate any existing user database on disk.

## 3. Final Approach

Use folder-per-local-store isolation.

`LocalStoreIdentity` is resolved into a stable `LocalStoreKey`. The key maps to:

```text
<base_dir>/data/<local_store_key>/
  manifest.json
  tenant.db
  tenant.db-wal
  tenant.db-shm
```

The identity includes enough information to avoid collisions between accounts,
companies, and backend scopes:

- app namespace
- environment
- API base URL
- WS URL
- company ID
- account ID
- optional tenant hint
- schema family

The directory name is hash-derived and path-safe. Raw identifiers are written
only into `manifest.json` for debugging. No token, cookie, authorization header,
or password-like field is written into the manifest.

## 4. Architecture

### 4.1 Local store module

Add a driver-host module:

```text
crates/helix-driver-host/src/local_store.rs
```

It owns:

- `BackendScope`
- `LocalStoreIdentity`
- `LocalStoreKey`
- `LocalStorePaths`
- `LocalStoreLocator`
- `LocalStoreError`

It does not open SQLite. It only resolves paths and writes manifest metadata.
`HostStorage::open_sqlite_url` remains the only SQLite opening path.

### 4.2 Public API shape

The implementation should expose this shape from `helix-driver-host`:

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

impl LocalStoreLocator {
    pub fn new(base_dir: impl Into<std::path::PathBuf>) -> Self;
    pub fn resolve(&self, identity: &LocalStoreIdentity) -> Result<LocalStorePaths, LocalStoreError>;
}
```

`LocalStoreIdentity::validate()` may exist as a private helper. The locator must
return an error if required identity fields are empty:

- app namespace
- environment
- API base URL
- WS URL
- company ID
- account ID
- schema family

### 4.3 Hash and path safety

Use a stable hash over canonical JSON. To avoid adding new dependencies, use
Rust standard library hashing only if the implementation makes the key stable
across process runs. `DefaultHasher` is not acceptable because it is not a
portable format contract.

Recommended no-new-dependency implementation:

- implement a small deterministic FNV-1a 64-bit helper over the canonical JSON
- key format: `s_<16 lowercase hex chars>`

This is not for cryptographic secrecy. It is for path safety and collision
avoidance in local app data. The manifest keeps the human-readable identity.

### 4.4 Manifest

Write `manifest.json` on every successful resolve. Existing manifests can be
overwritten with updated `lastOpenedAtMs`.

Required fields:

```json
{
  "version": 1,
  "localStoreKey": "s_...",
  "appNamespace": "...",
  "environment": "...",
  "apiBaseUrl": "...",
  "wsUrl": "...",
  "companyId": "...",
  "accountId": "...",
  "tenantHint": "...",
  "schemaFamily": "im-v1",
  "createdAtMs": 1783320000000,
  "lastOpenedAtMs": 1783320000000
}
```

The test does not need stable timestamps, only field presence and absence of
secret-like keys.

### 4.5 host-cli integration

`crates/helix-host-cli/src/host_env.rs::resolve_db_path` remains as the public
helper used by `main.rs`, but internally it should call the shared
`LocalStoreLocator`.

To keep the phase small, the helper signature may be expanded to accept the
backend and company data needed for a complete identity:

```rust
pub struct DbPathInputs<'a> {
    pub cookie_id: &'a str,
    pub company_id: &'a str,
    pub configured_db: &'a str,
    pub configured_tenant: &'a str,
    pub api_base_url: &'a str,
    pub ws_url: &'a str,
}

pub fn resolve_db_path(inputs: DbPathInputs<'_>) -> String;
```

`runtime.db` remains the highest-priority explicit override. If it is non-empty,
return it exactly.

`main.rs` must pass the values already present in `Config`:

- `cfg.identity.cookie_id`
- `cfg.identity.company_id`
- `cfg.identity.tenant`
- `cfg.runtime.db`
- `cfg.connection.api_base`
- `cfg.connection.ws_url`

### 4.6 Real local DB validation

Add a driver-host integration test that uses real file SQLite:

```text
crates/helix-driver-host/tests/local_store_locator.rs
```

Test case:

```text
real_file_sqlite_isolates_users_444_and_678
```

Required behavior:

1. Create a temp base dir under `/tmp`.
2. Resolve identity for account `444`.
3. Resolve identity for account `678`.
4. Assert keys and db paths differ.
5. Open `HostStorage` for `444` and `678`.
6. Execute `helix_im::IM_SCHEMA` on both stores.
7. Insert one row into `message` through `HostStorage::batch_upsert` for `444`.
8. Scan `message` in `678`; assert it has zero rows.
9. Reopen `444` from `paths_444.db_url`; scan `message`; assert the row exists.
10. Assert `tenant.db` exists on disk for both stores.
11. Assert both manifests exist and contain their account IDs.

The test should not rely on wall-clock ordering. It can use static values for
message columns.

## 5. Test Data

Use these test identities:

```text
user 444:
  app_namespace = "com.jinqidongli.cses.dev"
  environment = "local"
  api_base_url = "http://127.0.0.1:8066/api/cses"
  ws_url = "ws://127.0.0.1:8066/api/v4/websocket"
  company_id = "company-local"
  account_id = "444"
  tenant_hint = "444"
  schema_family = "im-v1"

user 678:
  same as above, except account_id = "678" and tenant_hint = "678"
```

The content row inserted into user `444` should contain a visible marker:

```text
temporary_id = "tmp-local-store-444"
id = "server-local-store-444"
channel_id = "chlocal000000000000000001"
user_id = "444"
message = "hello from 444"
```

The exact `message` table columns must match `helix_im::IM_SCHEMA`.

## 6. Acceptance Criteria

- `docs/superpowers/specs/2026-07-06-multitenant-local-db-real-validation-design.md` exists.
- `docs/superpowers/plans/2026-07-06-multitenant-local-db-real-validation.md` exists.
- `crates/helix-driver-host/src/local_store.rs` exists and is re-exported.
- `crates/helix-host-cli/src/host_env.rs` uses shared local store resolution.
- User `444` and user `678` produce different keys and different file-backed databases.
- `runtime.db` explicit override still wins.
- Real file-backed test proves no cross-read between `444` and `678`.
- `helix-core` has no diff.

Required verification commands:

```bash
cargo test -p helix-driver-host local_store -- --nocapture
cargo test -p helix-host-cli host_env -- --nocapture
cargo check -p helix-driver-host -p helix-host-cli
git diff --check
```

If code touches more than driver-host/host-cli, explain why before landing it.

## 7. Review Strategy

Use task-scoped review after each implementation slice:

1. Local store locator API and driver-host tests.
2. host-cli integration and host_env tests.
3. Real 444/678 SQLite isolation validation.

Reviewers must check spec compliance, path safety, no `helix-core` changes, and
whether the test uses real file-backed SQLite instead of `mode=memory`.
