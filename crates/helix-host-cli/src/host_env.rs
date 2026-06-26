//! host 环境派生（DB 路径 + 设备 id）—— 从 `main.rs` 抽出，守 src ≤300 行（structure-gate §1）。
//!
//! 配置去 env 化后（PRD §4.2）这里**不再读 `HELIX_*` 配置 env**：网关拓扑 / DB / tenant
//! 由调用方从 `Config` 派生后传入。仅保留 `HOME`（OS 级 env，非 HELIX 配置）+ 设备 id 探测。
//! 全是 ambient API（`std::env`/`std::fs`/`std::process`）。host 是其合法落点
//! （本 crate clippy `disallowed_methods=allow`）——core/im 才 deny。

/// 派生第二（默认业务）网关 base（spec06 方案 A）：vote/score 走 restHost（无 `/api/cses`）。
/// `configured`（= `connection.default_api_base`）非空则用之 > `api_base` 去 `/api/cses` 尾段
/// （host 注入网关拓扑，HX-C001）。
pub fn default_api_base(api_base: &str, configured: &str) -> String {
    if !configured.is_empty() {
        configured.to_string()
    } else {
        api_base.trim_end_matches("/api/cses").to_string()
    }
}

/// 派生 SQLite DB 绝对路径（多租户，对齐现网 678 真库目录）。
///
/// 优先级（高 → 低）：
/// 1. `configured_db`（= `runtime.db`）非空 → 直接返回（显式覆盖，保留旧 `/tmp` 能力）。
/// 2. 派生路径 `~/Library/Application Support/com.jinqidongli.cses.dev/data/<tenant>/tenant.db`：
///    - bundle identifier `com.jinqidongli.cses.dev` 硬编码（与 678 真库目录一致）。
///    - `<tenant>` 取值（高 → 低）：`configured_tenant`（= `identity.tenant`）>
///      `cookie_id`(=userId，对齐 cses-client tenant_key = userId) > 字面 `"default"`。
///    - 返回前 `create_dir_all` 父目录（失败则退回 `/tmp` 兜底）。
/// 3. 取不到 `$HOME` → `/tmp/helix_host_cli.db` 兜底。
pub fn resolve_db_path(cookie_id: &str, configured_db: &str, configured_tenant: &str) -> String {
    // ① 显式 db 覆盖（铁律：设置即用，跳过派生）。
    if !configured_db.is_empty() {
        return configured_db.to_string();
    }

    // ② 派生 base_dir = ~/Library/Application Support/com.jinqidongli.cses.dev
    let home = std::env::var("HOME").ok().filter(|s| !s.is_empty());
    let Some(home) = home else {
        tracing::warn!("$HOME 未设置，DB 退回 /tmp 兜底");
        return "/tmp/helix_host_cli.db".to_string();
    };

    let tenant = if !configured_tenant.is_empty() {
        configured_tenant.to_string()
    } else if !cookie_id.is_empty() {
        cookie_id.to_string()
    } else {
        "default".to_string()
    };

    let dir = std::path::Path::new(&home)
        .join("Library")
        .join("Application Support")
        .join("com.jinqidongli.cses.dev")
        .join("data")
        .join(&tenant);

    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(error = %e, dir = %dir.display(), "创建 DB 目录失败，退回 /tmp 兜底");
        return "/tmp/helix_host_cli.db".to_string();
    }

    dir.join("tenant.db").to_string_lossy().into_owned()
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
