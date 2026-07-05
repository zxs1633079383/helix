//! host-cli 配置（去 env 化，PRD §4.1/§4.2）：单一 YAML 真源 `helix.config.yaml`。
//!
//! ~18 个 `HELIX_*` env 从主路径退役 → `Config` struct（`serde_norway` 反序列化 YAML）。
//! 覆盖链（低 → 高）：YAML 文件 < `HELIX_<FIELD>` env 逃生口（**仅 debug 构建**，不文档化）
//! < `--cookie-id`/`--company-id` CLI flag。`go_log`/`go_port`/`start_hint` 仅脚本（yq）用，
//! serde 默认忽略未知字段 → host-cli 不建模它们。

use serde::Deserialize;

/// 配置加载错误（边界类型，外部输入零 `unwrap`）。
#[derive(Debug)]
pub enum ConfigError {
    /// 读文件失败（路径不存在 / 权限）。
    Io(std::io::Error),
    /// YAML 反序列化失败。
    Parse(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "读配置文件失败: {e}"),
            ConfigError::Parse(e) => write!(f, "解析 YAML 失败: {e}"),
        }
    }
}
impl std::error::Error for ConfigError {}

/// 单一配置真源（4 分组映射 PRD §4.1）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub connection: Connection,
    #[serde(default)]
    pub identity: Identity,
    #[serde(default)]
    pub client: Client,
    #[serde(default)]
    pub runtime: Runtime,
    #[serde(default)]
    pub observability: Observability,
}

/// 后端连接（切后端 = 改这块）。
#[derive(Debug, Clone, Deserialize)]
pub struct Connection {
    #[serde(default = "d_ws_url")]
    pub ws_url: String,
    #[serde(default = "d_api_base")]
    pub api_base: String,
    /// 空 = 派生（api_base 去 `/api/cses` 尾段，vote/score 第二网关）。
    #[serde(default)]
    pub default_api_base: String,
}
impl Default for Connection {
    fn default() -> Self {
        Self {
            ws_url: d_ws_url(),
            api_base: d_api_base(),
            default_api_base: String::new(),
        }
    }
}

/// 联调身份（双账号 e2e 用 CLI flag 覆盖 cookie_id/company_id）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Identity {
    #[serde(default)]
    pub cookie_id: String,
    #[serde(default)]
    pub company_id: String,
    /// 空 = 取 cookie_id（DB 多租户目录派生）。
    #[serde(default)]
    pub tenant: String,
    /// userSnapshot（ucenter 展开，posts/create body）；空非阻断。
    #[serde(default)]
    pub user_name: String,
    #[serde(default)]
    pub org_name: String,
    #[serde(default)]
    pub dept_name: String,
}

/// 端身份头（很少改）。
#[derive(Debug, Clone, Deserialize)]
pub struct Client {
    #[serde(default = "d_app_type")]
    pub app_type: String,
    #[serde(default = "d_device")]
    pub device: String,
    /// 空 = ADR-011 固定联调 deviceId → 本机派生 fallback（解析留 main.rs）。
    #[serde(default)]
    pub device_id: String,
    #[serde(default = "d_language")]
    pub language: String,
    #[serde(default = "d_app_version")]
    pub app_version: String,
    #[serde(default = "d_os_version")]
    pub os_version: String,
}
impl Default for Client {
    fn default() -> Self {
        Self {
            app_type: d_app_type(),
            device: d_device(),
            device_id: String::new(),
            language: d_language(),
            app_version: d_app_version(),
            os_version: d_os_version(),
        }
    }
}

/// 运行时旋钮。
#[derive(Debug, Clone, Deserialize)]
pub struct Runtime {
    /// 空 = 派生多租户路径。
    #[serde(default)]
    pub db: String,
    /// 0 = 默认 8（解析留 main.rs）。
    #[serde(default)]
    pub http_max_inflight: usize,
    #[serde(default = "d_true")]
    pub dump_frames: bool,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            db: String::new(),
            http_max_inflight: 0,
            dump_frames: true,
        }
    }
}

/// 可观测性配置（主路径仍是 helix.config.yaml，不走 env 真源）。
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Observability {
    #[serde(default)]
    pub otel: Otel,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Otel {
    #[serde(default)]
    pub enabled: bool,
    #[serde(rename = "serviceName", default = "d_helix_service_name")]
    pub service_name: String,
    #[serde(default = "d_otel_endpoint")]
    pub endpoint: String,
    #[serde(default = "d_otel_protocol")]
    pub protocol: String,
    #[serde(default)]
    pub sampler: OtelSampler,
    #[serde(default)]
    pub facets: OtelFacets,
}

impl Default for Otel {
    fn default() -> Self {
        Self {
            enabled: false,
            service_name: d_helix_service_name(),
            endpoint: d_otel_endpoint(),
            protocol: d_otel_protocol(),
            sampler: OtelSampler::default(),
            facets: OtelFacets::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct OtelSampler {
    #[serde(rename = "type", default = "d_sampler_type")]
    pub kind: String,
    #[serde(default = "d_sampler_ratio")]
    pub ratio: f64,
}

impl Default for OtelSampler {
    fn default() -> Self {
        Self {
            kind: d_sampler_type(),
            ratio: d_sampler_ratio(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct OtelFacets {
    #[serde(default)]
    pub command: bool,
    #[serde(default)]
    pub core: bool,
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub ws: bool,
    #[serde(default)]
    pub storage: bool,
    #[serde(rename = "portReply", default)]
    pub port_reply: bool,
    #[serde(default)]
    pub event: bool,
    #[serde(default)]
    pub native: bool,
    #[serde(default)]
    pub ffi: bool,
}

// serde default 工厂（旧 env fallback 同值，仅 ws/api base 8065→8066）。
fn d_ws_url() -> String {
    "ws://127.0.0.1:8066/api/v4/websocket".to_string()
}
fn d_api_base() -> String {
    "http://127.0.0.1:8066/api/cses".to_string()
}
fn d_app_type() -> String {
    "bct".to_string()
}
fn d_device() -> String {
    "IOS".to_string()
}
fn d_language() -> String {
    "zh".to_string()
}
fn d_app_version() -> String {
    "1.2.56".to_string()
}
fn d_os_version() -> String {
    "macos-14.5.0".to_string()
}
fn d_true() -> bool {
    true
}
fn d_helix_service_name() -> String {
    "helix-host-cli".to_string()
}
fn d_otel_endpoint() -> String {
    "http://127.0.0.1:4317".to_string()
}
fn d_otel_protocol() -> String {
    "grpc".to_string()
}
fn d_sampler_type() -> String {
    "parentbased_traceidratio".to_string()
}
fn d_sampler_ratio() -> f64 {
    1.0
}

/// 解析后的 CLI 参数（`--config` + 身份覆盖）。
#[derive(Debug, Clone)]
pub struct CliArgs {
    pub config_path: String,
    pub cookie_id: Option<String>,
    pub company_id: Option<String>,
}

impl Config {
    /// 从 YAML 文本反序列化。
    pub fn from_yaml_str(s: &str) -> Result<Self, ConfigError> {
        serde_norway::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// 从路径加载（默认 `./helix.config.yaml`，`--config` 覆盖）。
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
        Self::from_yaml_str(&raw)
    }

    /// debug 逃生口：仅 debug 构建，`HELIX_<FIELD>` env 非空则覆盖对应 config 值（不文档化）。
    /// 空值不覆盖（与旧 `unwrap_or_default` 分野：缺省/空 = 保留 config 值）。
    #[cfg(debug_assertions)]
    pub fn apply_env_overrides(&mut self) {
        env_str(&mut self.connection.ws_url, "HELIX_WS_URL");
        env_str(&mut self.connection.api_base, "HELIX_API_BASE");
        env_str(
            &mut self.connection.default_api_base,
            "HELIX_DEFAULT_API_BASE",
        );
        env_str(&mut self.identity.cookie_id, "HELIX_COOKIE_ID");
        env_str(&mut self.identity.company_id, "HELIX_COMPANY_ID");
        env_str(&mut self.identity.tenant, "HELIX_TENANT");
        env_str(&mut self.identity.user_name, "HELIX_USER_NAME");
        env_str(&mut self.identity.org_name, "HELIX_ORG_NAME");
        env_str(&mut self.identity.dept_name, "HELIX_DEPT_NAME");
        env_str(&mut self.client.app_type, "HELIX_APP_TYPE");
        env_str(&mut self.client.device, "HELIX_DEVICE");
        env_str(&mut self.client.device_id, "HELIX_DEVICE_ID");
        env_str(&mut self.client.language, "HELIX_LANGUAGE");
        env_str(&mut self.client.app_version, "HELIX_APP_VERSION");
        env_str(&mut self.client.os_version, "HELIX_OS_VERSION");
        env_str(&mut self.runtime.db, "HELIX_DB");
        if let Ok(v) = std::env::var("HELIX_HTTP_MAX_INFLIGHT") {
            if let Ok(n) = v.parse::<usize>() {
                self.runtime.http_max_inflight = n;
            }
        }
        if let Ok(v) = std::env::var("HELIX_DUMP_FRAMES") {
            // 显式 truthy/falsy；其它值忽略（不静默乱覆盖）。
            match v.as_str() {
                "1" | "true" | "TRUE" => self.runtime.dump_frames = true,
                "0" | "false" | "FALSE" => self.runtime.dump_frames = false,
                _ => {}
            }
        }
    }

    /// 生产构建：无 env 逃生口（彻底去 env）。
    #[cfg(not(debug_assertions))]
    pub fn apply_env_overrides(&mut self) {}

    /// CLI flag 覆盖 identity（最高优先级，双账号 e2e）。
    pub fn apply_cli_overrides(&mut self, cli: &CliArgs) {
        if let Some(c) = &cli.cookie_id {
            self.identity.cookie_id = c.clone();
        }
        if let Some(c) = &cli.company_id {
            self.identity.company_id = c.clone();
        }
    }
}

/// 仅当 env 存在且非空时覆盖（debug 逃生口共用）。
#[cfg(debug_assertions)]
fn env_str(slot: &mut String, key: &str) {
    if let Ok(v) = std::env::var(key) {
        if !v.is_empty() {
            *slot = v;
        }
    }
}

/// 手解析 args（省 clap 依赖）：`--config <path>` / `--cookie-id <id>` / `--company-id <id>`。
/// 未知参数忽略；缺值（flag 在末尾无后继）则跳过。
pub fn parse_args<I: IntoIterator<Item = String>>(args: I) -> CliArgs {
    let mut config_path = "./helix.config.yaml".to_string();
    let mut cookie_id = None;
    let mut company_id = None;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--config" => {
                if let Some(v) = it.next() {
                    config_path = v;
                }
            }
            "--cookie-id" => cookie_id = it.next(),
            "--company-id" => company_id = it.next(),
            _ => {}
        }
    }
    CliArgs {
        config_path,
        cookie_id,
        company_id,
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
