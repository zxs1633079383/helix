//! host 环境派生（DB 路径 + 设备 id）—— 从 `main.rs` 抽出，守 src ≤300 行（structure-gate §1）。
//!
//! 配置去 env 化后（PRD §4.2）这里**不再读 `HELIX_*` 配置 env**：网关拓扑 / DB / tenant
//! 由调用方从 `Config` 派生后传入。仅保留 `HOME`（OS 级 env，非 HELIX 配置）+ 设备 id 探测。
//! 全是 ambient API（`std::env`/`std::fs`/`std::process`）。host 是其合法落点
//! （本 crate clippy `disallowed_methods=allow`）——core/im 才 deny。

use helix_driver_host::{BackendScope, LocalStoreIdentity, LocalStoreLocator};
use std::path::{Path, PathBuf};

const APP_NAMESPACE: &str = "com.jinqidongli.cses.dev";
const TMP_STORE_BASE: &str = "/tmp/helix_host_cli";
const MISSING_COMPANY_SENTINEL: &str = "__missing_company__";

/// 读取平台显式注入的 Java/default 业务网关 base。
///
/// 空串保持为空，让依赖 Java 的能力明确失败；禁止从 Go `api_base` 裁路径猜测另一个服务地址。
pub fn default_api_base(configured: &str) -> String {
    configured.to_string()
}

/// 派生 SQLite DB 绝对路径：runtime.db 显式覆盖优先；否则按 app/backend/company/account/schema
/// 生成 `<store-key>`，落到 `.../data/<store-key>/tenant.db`。无 `$HOME` 或主目录失败时，
/// 也退到 `/tmp/helix_host_cli/data/<store-key>/tenant.db`，避免 444/678 兜底时重新合库。
pub struct DbPathInputs<'a> {
    pub cookie_id: &'a str,
    pub company_id: &'a str,
    pub configured_db: &'a str,
    pub configured_tenant: &'a str,
    pub api_base_url: &'a str,
    pub ws_url: &'a str,
}

pub fn resolve_db_path(inputs: DbPathInputs<'_>) -> String {
    // ① 显式 db 覆盖（铁律：设置即用，跳过派生）。
    if !inputs.configured_db.is_empty() {
        return inputs.configured_db.to_string();
    }

    // ② 派生 base_dir = ~/Library/Application Support/com.jinqidongli.cses.dev
    let account_id = if !inputs.configured_tenant.is_empty() {
        inputs.configured_tenant.to_string()
    } else if !inputs.cookie_id.is_empty() {
        inputs.cookie_id.to_string()
    } else {
        "default".to_string()
    };

    let identity = local_store_identity(inputs, &account_id);
    let primary_base = match std::env::var("HOME").ok().filter(|s| !s.is_empty()) {
        Some(home) => Path::new(&home)
            .join("Library")
            .join("Application Support")
            .join(APP_NAMESPACE),
        None => {
            tracing::warn!("$HOME 未设置，DB 退回 /tmp 多租户目录兜底");
            PathBuf::from(TMP_STORE_BASE)
        }
    };

    match LocalStoreLocator::new(&primary_base).resolve(&identity) {
        Ok(paths) => paths.db_path.to_string_lossy().into_owned(),
        Err(e) => {
            tracing::warn!(error = %e, dir = %primary_base.display(), "派生 DB 路径失败，退回 /tmp 多租户目录兜底");
            match LocalStoreLocator::new(TMP_STORE_BASE).resolve(&identity) {
                Ok(paths) => paths.db_path.to_string_lossy().into_owned(),
                Err(e) => {
                    tracing::warn!(error = %e, "派生 /tmp DB 路径仍失败，退回账号隔离路径");
                    isolated_tmp_fallback_path(&account_id)
                        .to_string_lossy()
                        .into_owned()
                }
            }
        }
    }
}

fn local_store_identity(inputs: DbPathInputs<'_>, account_id: &str) -> LocalStoreIdentity {
    let company_id = if inputs.company_id.trim().is_empty() {
        MISSING_COMPANY_SENTINEL
    } else {
        inputs.company_id.trim()
    };
    LocalStoreIdentity {
        app_namespace: APP_NAMESPACE.to_string(),
        backend: BackendScope {
            environment: "dev".to_string(),
            api_base_url: inputs.api_base_url.to_string(),
            ws_url: inputs.ws_url.to_string(),
        },
        company_id: company_id.to_string(),
        account_id: account_id.to_string(),
        tenant_hint: Some(account_id.to_string()),
        schema_family: "helix-im-v1".to_string(),
    }
}

fn isolated_tmp_fallback_path(account_id: &str) -> PathBuf {
    Path::new(TMP_STORE_BASE)
        .join("data")
        .join(format!("fallback_{}", safe_path_segment(account_id)))
        .join("tenant.db")
}

fn safe_path_segment(input: &str) -> String {
    let out: String = input
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() {
        "default".to_string()
    } else {
        out
    }
}

/// 从本机派生稳定 deviceId；本函数自身不硬编码任何一台真实设备 id。
///
/// - macOS：`ioreg` 的 `IOPlatformUUID`
/// - Linux：`/etc/machine-id`（回退 `/var/lib/dbus/machine-id`）
/// - Windows：注册表 `HKLM\SOFTWARE\Microsoft\Cryptography\MachineGuid`
///
/// 统一归一化为**大写、去连字符**（对齐现网 deviceId 的 32 hex 形态）。
/// 取不到 → `None`（调用方告警并以空 deviceId 跑，暴露问题而非静默冒用他人 id）。
pub fn machine_device_id() -> Option<String> {
    let raw = raw_machine_id()?;
    let normalized: String = raw
        .chars()
        .filter(|c| *c != '-')
        .flat_map(|c| c.to_uppercase())
        .collect();
    let trimmed = normalized.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(target_os = "macos")]
fn raw_machine_id() -> Option<String> {
    let out = std::process::Command::new("ioreg")
        .args(["-rd1", "-c", "IOPlatformExpertDevice"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // 行形如：    "IOPlatformUUID" = "ED380414-9325-42BA-B499-3E5AC3E07C64"
    text.lines()
        .find(|l| l.contains("IOPlatformUUID"))
        .and_then(|l| l.split('"').nth(3))
        .map(str::to_string)
}

#[cfg(target_os = "linux")]
fn raw_machine_id() -> Option<String> {
    std::fs::read_to_string("/etc/machine-id")
        .or_else(|_| std::fs::read_to_string("/var/lib/dbus/machine-id"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(target_os = "windows")]
fn raw_machine_id() -> Option<String> {
    let out = std::process::Command::new("reg")
        .args([
            "query",
            r"HKLM\SOFTWARE\Microsoft\Cryptography",
            "/v",
            "MachineGuid",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // 行形如：    MachineGuid    REG_SZ    ed380414-9325-42ba-b499-3e5ac3e07c64
    text.lines()
        .find(|l| l.contains("MachineGuid"))
        .and_then(|l| l.split_whitespace().last())
        .map(str::to_string)
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn raw_machine_id() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[test]
    fn default_api_base_is_explicit_and_never_derived_from_go() {
        assert_eq!(
            default_api_base("https://java.example.test"),
            "https://java.example.test"
        );
        assert!(default_api_base("").is_empty());
    }

    fn temp_home(label: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("helix_host_env_{label}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn with_home<T>(home: Option<&Path>, f: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let old = std::env::var("HOME").ok();
        match home {
            Some(path) => std::env::set_var("HOME", path),
            None => std::env::remove_var("HOME"),
        }
        let out = f();
        match old {
            Some(value) => std::env::set_var("HOME", value),
            None => std::env::remove_var("HOME"),
        }
        out
    }

    fn inputs<'a>(
        cookie_id: &'a str,
        company_id: &'a str,
        configured_db: &'a str,
    ) -> DbPathInputs<'a> {
        DbPathInputs {
            cookie_id,
            company_id,
            configured_db,
            configured_tenant: "",
            api_base_url: "http://127.0.0.1:8066/api/cses",
            ws_url: "ws://127.0.0.1:8066/api/cses/websocket",
        }
    }

    #[test]
    fn explicit_runtime_db_still_wins() {
        let got = with_home(None, || {
            resolve_db_path(inputs("444", "64118", "/tmp/explicit-runtime.db"))
        });
        assert_eq!(got, "/tmp/explicit-runtime.db");
    }

    #[test]
    fn users_444_and_678_resolve_to_different_tenant_db_paths() {
        let home = temp_home("two_users");
        let (path_444, path_678) = with_home(Some(&home), || {
            (
                resolve_db_path(inputs("444", "64118", "")),
                resolve_db_path(inputs("678", "64118", "")),
            )
        });

        assert_ne!(path_444, path_678);
        assert!(path_444.ends_with("/tenant.db"));
        assert!(path_678.ends_with("/tenant.db"));
        assert!(path_444.contains("/data/s_"));
        assert!(path_678.contains("/data/s_"));
    }

    #[test]
    fn missing_home_falls_back_to_tmp_and_keeps_users_split() {
        let (path_444, path_678) = with_home(None, || {
            (
                resolve_db_path(inputs("444", "64118", "")),
                resolve_db_path(inputs("678", "64118", "")),
            )
        });

        assert_ne!(path_444, path_678);
        assert!(path_444.starts_with("/tmp/helix_host_cli/data/s_"));
        assert!(path_678.starts_with("/tmp/helix_host_cli/data/s_"));
        assert!(path_444.ends_with("/tenant.db"));
        assert!(path_678.ends_with("/tenant.db"));
    }

    #[test]
    fn missing_company_still_keeps_users_split() {
        let home = temp_home("missing_company");
        let (path_444, path_678) = with_home(Some(&home), || {
            (
                resolve_db_path(inputs("444", "", "")),
                resolve_db_path(inputs("678", "", "")),
            )
        });

        assert_ne!(path_444, path_678);
        assert!(path_444.ends_with("/tenant.db"));
        assert!(path_678.ends_with("/tenant.db"));
    }
}
