//! # error.rs
//!
//! helix-core 的统一错误类型。
//!
//! - `CoreError`：ExecutionShell / ModuleHost 层错误
//! - `PortError`：所有 port trait 的统一返回错误
//!   - driver 在 ACL-2 用 thiserror `From<具体 crate 错误>` 翻译到此类型
//!   - 统一返回 PortError 保证所有 port trait 对象安全（dyn-safe）

use thiserror::Error;

/// core 层（ExecutionShell / ModuleHost）的错误。
#[derive(Debug, Error)]
pub enum CoreError {
    /// 无模块能处理该 Tick（Inbound / Command 路由未命中）
    #[error("no module accepted tick: {0}")]
    NoHandler(&'static str),

    /// corr_map 中找不到该 Correlation（PortReply 到达时 corr 已过期或从未注册）
    #[error("unknown correlation {0}")]
    UnknownCorrelation(u64),

    /// 某模块的 handle / on_start / on_stop 返回错误
    #[error("module '{module}' error: {source}")]
    ModuleError {
        module: &'static str,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

/// 所有 port trait 的统一返回错误类型。
///
/// ## driver 实现约定（ACL-2）
///
/// 各 helix-driver-* 的具体 crate 错误（sqlx::Error / reqwest::Error / 等）
/// 必须用 thiserror `From` 翻译到此类型，不得让具体 crate 类型穿透 port 边界。
///
/// ```rust,no_run
/// // 示例：helix-driver-native/src/storage.rs（需在该 crate 中编译，helix-core 不依赖 sqlx）
/// # mod example {
/// # use helix_core::PortError;
/// // impl From<sqlx::Error> for PortError {
/// //     fn from(e: sqlx::Error) -> Self {
/// //         PortError::Storage(e.to_string())
/// //     }
/// // }
/// # }
/// ```
#[derive(Debug, Error)]
pub enum PortError {
    /// Confirmed host offline before I/O; the request has never been submitted.
    #[error("NETWORK_UNAVAILABLE: not_submitted before_io")]
    NetworkUnavailable,
    /// An optimistic write lost a race; its transaction rolled back and the store remains usable.
    #[error("storage conflict")]
    StorageConflict,

    #[error("storage error: {0}")]
    Storage(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("http error: {0}")]
    Http(String),

    #[error("clock error: {0}")]
    Clock(String),

    #[error("id source error: {0}")]
    IdSource(String),

    /// 通用端口错误（用于 driver 无法分类的情况）
    #[error("port error: {0}")]
    Other(String),
}
