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
///
/// ## getrandom 注意事项（MIN-5：js feature 只在 wasm32 target 段声明）
///
/// `getrandom` 的 `"js"` feature 是 wasm32 专属（依赖 wasm-bindgen），
/// 放进 workspace 级会污染 native 编译图、且 getrandom 0.3 升级时可能破坏
/// native 链接。正确做法是 **只在 `helix-driver-wasm/Cargo.toml` 的
/// wasm32 target 段单独激活**（feature unification 单调性 → 各 target 编译图隔离）：
/// ```toml
/// # helix-driver-wasm/Cargo.toml
/// [target.'cfg(target_arch = "wasm32")'.dependencies]
/// getrandom = { workspace = true, features = ["js"] }
/// ```
/// workspace 根的 `getrandom` 不带任何 feature。
pub trait IdSource: MaybeSend + 'static {
    /// 生成一个 UUID v4（16 字节）
    fn new_uuid(&self) -> Result<[u8; 16], PortError>;
}

/// 随机数生成的 core 抽象（用于退避 jitter 等场景）。
pub trait Random: MaybeSend + 'static {
    /// 生成 [0, max) 范围内的均匀随机 u64
    fn random_u64(&self, max: u64) -> u64;
}
