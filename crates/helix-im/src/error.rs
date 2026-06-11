//! IM 业务域错误

use thiserror::Error;

/// IM 业务域错误。
///
/// ## MIN-2 修复：删除双向 From 套娃
///
/// 原 `ImError::Core(#[from] CoreError)` + `CoreError::ModuleError{ source: Box<dyn Error> }`
/// 形成双向 From 路径（ImError → CoreError → ImError），可产生无限嵌套错误链。
///
/// 修复：删除 `ImError::Core` 及其 `#[from]` 自动实现。
/// 依赖方向应单向：helix-core 不依赖 helix-im，helix-im 的错误不应反持 CoreError。
/// IM 内部错误通过 `CoreError::ModuleError { source: Box<dyn Error> }` 向上传播，
/// 但 IM 自身不持有 CoreError（避免循环持有）。
///
/// 若需要包装外部错误，在 `module.rs` 的 `handle` 方法中显式 map_err 到 CoreError。
#[derive(Debug, Error)]
pub enum ImError {
    #[error("parse error: {0}")]
    Parse(String),

    #[error("invalid seq: expected {expected}, got {got}")]
    InvalidSeq { expected: u64, got: u64 },

    #[error("unknown channel: {0}")]
    UnknownChannel(u64),

    #[error("unknown correlation: {0}")]
    UnknownCorrelation(u64),
}
