//! Host-side local store locator.
//!
//! This module owns the platform path decision for per-account SQLite stores.
//! It deliberately stays in driver-host: core and business modules receive a
//! Storage port and never learn about filesystem layout or tenant identity.

use serde::Serialize;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendScope {
    pub environment: String,
    pub api_base_url: String,
    pub ws_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStoreIdentity {
    pub app_namespace: String,
    pub backend: BackendScope,
    pub company_id: String,
    pub account_id: String,
    pub tenant_hint: Option<String>,
    pub schema_family: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStorePaths {
    pub key: String,
    pub dir: PathBuf,
    pub db_path: PathBuf,
    pub db_url: String,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStoreLocator {
    pub base_dir: PathBuf,
}

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalStoreManifest<'a> {
    version: u32,
    local_store_key: &'a str,
    app_namespace: &'a str,
    environment: &'a str,
    api_base_url: &'a str,
    ws_url: &'a str,
    company_id: &'a str,
    account_id: &'a str,
    tenant_hint: &'a str,
    schema_family: &'a str,
    created_at_ms: i64,
    last_opened_at_ms: i64,
}

impl LocalStoreLocator {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn resolve(
        &self,
        identity: &LocalStoreIdentity,
    ) -> Result<LocalStorePaths, LocalStoreError> {
        validate_identity(identity)?;

        let key = local_store_key(identity);
        let dir = self.base_dir.join("data").join(&key);
        std::fs::create_dir_all(&dir).map_err(LocalStoreError::CreateDir)?;

        let db_path = dir.join("tenant.db");
        let db_url = format!("sqlite:file:{}", db_path.to_string_lossy());
        let manifest_path = dir.join("manifest.json");
        let now = now_ms();
        let created_at_ms = existing_created_at_ms(&manifest_path).unwrap_or(now);
        let tenant_hint = identity.tenant_hint.as_deref().unwrap_or("");
        let manifest = LocalStoreManifest {
            version: 1,
            local_store_key: &key,
            app_namespace: identity.app_namespace.trim(),
            environment: identity.backend.environment.trim(),
            api_base_url: identity.backend.api_base_url.trim(),
            ws_url: identity.backend.ws_url.trim(),
            company_id: identity.company_id.trim(),
            account_id: identity.account_id.trim(),
            tenant_hint: tenant_hint.trim(),
            schema_family: identity.schema_family.trim(),
            created_at_ms,
            last_opened_at_ms: now,
        };
        let raw =
            serde_json::to_vec_pretty(&manifest).map_err(LocalStoreError::SerializeManifest)?;
        std::fs::write(&manifest_path, raw).map_err(LocalStoreError::WriteManifest)?;

        Ok(LocalStorePaths {
            key,
            dir,
            db_path,
            db_url,
            manifest_path,
        })
    }
}

fn validate_identity(identity: &LocalStoreIdentity) -> Result<(), LocalStoreError> {
    validate_non_empty(&identity.app_namespace, "app_namespace")?;
    validate_non_empty(&identity.backend.environment, "environment")?;
    validate_non_empty(&identity.backend.api_base_url, "api_base_url")?;
    validate_non_empty(&identity.backend.ws_url, "ws_url")?;
    validate_non_empty(&identity.company_id, "company_id")?;
    validate_non_empty(&identity.account_id, "account_id")?;
    validate_non_empty(&identity.schema_family, "schema_family")?;
    Ok(())
}

fn validate_non_empty(value: &str, field: &'static str) -> Result<(), LocalStoreError> {
    if value.trim().is_empty() {
        Err(LocalStoreError::MissingField(field))
    } else {
        Ok(())
    }
}

fn local_store_key(identity: &LocalStoreIdentity) -> String {
    format!("s_{:016x}", fnv1a64(&canonical_identity(identity)))
}

fn canonical_identity(identity: &LocalStoreIdentity) -> String {
    let mut out = String::new();
    push_canonical_field(&mut out, "app_namespace", identity.app_namespace.trim());
    push_canonical_field(&mut out, "environment", identity.backend.environment.trim());
    push_canonical_field(
        &mut out,
        "api_base_url",
        identity.backend.api_base_url.trim(),
    );
    push_canonical_field(&mut out, "ws_url", identity.backend.ws_url.trim());
    push_canonical_field(&mut out, "company_id", identity.company_id.trim());
    push_canonical_field(&mut out, "account_id", identity.account_id.trim());
    push_canonical_field(
        &mut out,
        "tenant_hint",
        identity.tenant_hint.as_deref().unwrap_or("").trim(),
    );
    push_canonical_field(&mut out, "schema_family", identity.schema_family.trim());
    out
}

fn push_canonical_field(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push(':');
    out.push_str(&value.len().to_string());
    out.push(':');
    out.push_str(value);
    out.push('\n');
}

fn fnv1a64(input: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for b in input.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn existing_created_at_ms(path: &PathBuf) -> Option<i64> {
    let raw = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
    json.get("createdAtMs").and_then(serde_json::Value::as_i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static DIR_SEQ: AtomicUsize = AtomicUsize::new(0);

    fn temp_base(label: &str) -> PathBuf {
        let seq = DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "helix_local_store_{label}_{}_{}",
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn identity(account_id: &str) -> LocalStoreIdentity {
        LocalStoreIdentity {
            app_namespace: "com.jinqidongli.cses.dev".to_string(),
            backend: BackendScope {
                environment: "dev".to_string(),
                api_base_url: "http://127.0.0.1:8066/api/cses".to_string(),
                ws_url: "ws://127.0.0.1:8066/api/cses/websocket".to_string(),
            },
            company_id: "64118".to_string(),
            account_id: account_id.to_string(),
            tenant_hint: Some(account_id.to_string()),
            schema_family: "helix-im-v1".to_string(),
        }
    }

    #[test]
    fn same_identity_produces_stable_key() {
        let locator = LocalStoreLocator::new(temp_base("stable"));
        let first = locator.resolve(&identity("444")).expect("first resolve");
        let second = locator.resolve(&identity("444")).expect("second resolve");

        assert_eq!(first.key, second.key);
        assert_eq!(first.db_path, second.db_path);
        assert_eq!(first.key.len(), 18);
        assert!(first.key.starts_with("s_"));
    }

    #[test]
    fn different_accounts_produce_different_paths() {
        let locator = LocalStoreLocator::new(temp_base("different"));
        let user_444 = locator.resolve(&identity("444")).expect("resolve 444");
        let user_678 = locator.resolve(&identity("678")).expect("resolve 678");

        assert_ne!(user_444.key, user_678.key);
        assert_ne!(user_444.db_path, user_678.db_path);
        assert!(user_444.db_path.ends_with("tenant.db"));
        assert!(user_678.db_path.ends_with("tenant.db"));
    }

    #[test]
    fn missing_required_fields_are_rejected() {
        let locator = LocalStoreLocator::new(temp_base("missing"));
        let mut broken = identity("444");
        broken.company_id.clear();

        let err = locator.resolve(&broken).expect_err("missing company_id");
        assert!(matches!(err, LocalStoreError::MissingField("company_id")));
    }

    #[test]
    fn manifest_contains_identity_but_no_secret_fields() {
        let locator = LocalStoreLocator::new(temp_base("manifest"));
        let paths = locator.resolve(&identity("444")).expect("resolve");
        let raw = std::fs::read_to_string(paths.manifest_path).expect("read manifest");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("manifest json");

        assert_eq!(json["accountId"], "444");
        assert_eq!(json["companyId"], "64118");
        assert_eq!(json["localStoreKey"], paths.key);
        assert!(json.get("cookie").is_none());
        assert!(json.get("token").is_none());
        assert!(json.get("authorization").is_none());
    }

    #[test]
    fn existing_manifest_preserves_created_at_ms() {
        let locator = LocalStoreLocator::new(temp_base("created_at"));
        let paths = locator.resolve(&identity("444")).expect("resolve");
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&paths.manifest_path).unwrap()).unwrap();
        json["createdAtMs"] = serde_json::json!(123_i64);
        std::fs::write(
            &paths.manifest_path,
            serde_json::to_vec_pretty(&json).unwrap(),
        )
        .unwrap();

        let paths = locator.resolve(&identity("444")).expect("resolve again");
        let raw = std::fs::read_to_string(paths.manifest_path).expect("read manifest");
        let json: serde_json::Value = serde_json::from_str(&raw).expect("manifest json");
        assert_eq!(json["createdAtMs"], 123_i64);
        assert!(json["lastOpenedAtMs"].as_i64().unwrap_or(0) >= 123_i64);
    }
}
