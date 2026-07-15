//! IdSource + Random port — ID 和随机数注入抽象
//!
//! ## 确定性保证
//!
//! - IM 主路径的 temporaryId 由 UI 注入（Angular 前端生成），内核不调用 IdSource
//! - Correlation / TimerId 由 ExecutionShell 内部自增分配（无需注入）
//! - IdSource 用于需要内部生成 UUID 的场景（如模块的 session_id）
//! - Random 用于退避 jitter（指数退避的随机扰动）

use crate::error::PortError;
use crate::platform::MaybeSend;

/// UUID / 随机 ID 生成的 core 抽象。
pub trait IdSource: MaybeSend + 'static {
    /// 生成一个 UUID v4（16 字节）
    fn new_uuid(&self) -> Result<[u8; 16], PortError>;
}

/// 随机数生成的 core 抽象（用于退避 jitter 等场景）。
pub trait Random: MaybeSend + 'static {
    /// 生成 [0, max) 范围内的均匀随机 u64
    fn random_u64(&self, max: u64) -> u64;
}
