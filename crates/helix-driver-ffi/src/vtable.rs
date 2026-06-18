//! CppHostVtable — C++ host 回调表（C ABI，repr(C)）。
//!
//! ## 设计（C++ 直连模型）
//!
//! 本模型下 **Rust 持有网络**（SharedWsClient / SharedHttpClient），
//! C++ host 不再负责喂帧/喂响应，只需要：
//!   1. 接收 Rust pump 吐出的领域事件批（`on_domain_events_batch`）
//!   2. 提供 host 壁钟时间（`now_ms`）
//!
//! ## ctx 生命周期契约
//!
//! - `ctx` 由 C++ host 自管，Rust 不解引用、不拥有，只原样回传给回调函数。
//! - **C++ 不得在 `helix_destroy` 返回前释放 ctx**。
//!   `helix_destroy` 会关闭 tick channel，但 pump 线程（M3-F4 引入）在 join 完成前
//!   仍可能调用 `on_domain_events_batch`——提前释放 ctx 是 use-after-free。
//!
//! ## M3-F4 TODO（pump 建好后接入这两个字段）
//!
//! - `on_domain_events_batch`：pump 产出 `Vec<Effect::Emit>` → 序列化整批
//!   → 单次调用此函数（HX-C007 批处理铁律，禁逐条）。
//! - `now_ms`：pump 每次调用 `ExecutionShell::step()` 前经此获取壁钟（HX-C002 确定性注入）。

use std::ffi::c_void;

/// C++ host 回调表。
///
/// `repr(C)` 保证 ABI 稳定，cbindgen 自动扫描到 helix.hpp。
///
/// # Safety
///
/// - 所有函数指针若非 None，在 helix_destroy 返回前必须保持有效（C++ 管理其生命周期）。
/// - `ctx` 指向的对象由 host 自行保证跨线程安全；Rust 不解引用 `ctx`，只原样回传。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CppHostVtable {
    /// 领域事件批回调。
    ///
    /// pump（M3-F4）在一个 step 产出的所有 `Effect::Emit` 序列化为单个 byte buffer 后，
    /// **整批一次**调用此函数（HX-C007）。本轮 pump 未建，此字段仅声明契约。
    ///
    /// - `ctx`：透传 host 自定义 context，Rust 不解引用。
    /// - `ptr`：指向序列化后的 DomainEventBytes 批数据；仅在本次回调期间有效。
    /// - `len`：批数据字节长度。
    ///
    /// # Safety（C++ 侧）
    ///
    /// 函数必须在返回前拷贝或消费 `ptr..ptr+len` 的数据，不得在回调返回后持有指针。
    pub on_domain_events_batch:
        Option<unsafe extern "C" fn(ctx: *mut c_void, ptr: *const u8, len: usize)>,

    /// 壁钟时间（Unix epoch 毫秒）。
    ///
    /// pump（M3-F4）在每次 `ExecutionShell::step()` 前调用，注入确定性时间（HX-C002）。
    /// 本轮 pump 未建，此字段仅声明契约。
    pub now_ms: Option<extern "C" fn(ctx: *mut c_void) -> u64>,

    /// 用户自定义 context，Rust 不解引用、不拥有，仅原样回传给回调函数。
    pub ctx: *mut c_void,
}

// SAFETY: CppHostVtable 持有两类值：
//   1. `extern "C" fn` 函数指针——线程无关，只是代码地址。
//   2. `ctx: *mut c_void`——不透明句柄，Rust 不解引用。
//      host 侧契约保证 ctx 对应的对象在所有回调线程间安全共享（host 自管同步）。
// 因此跨线程传递 / 共享此结构安全。
unsafe impl Send for CppHostVtable {}
// SAFETY: 同上——函数指针 + 不透明 ctx，Rust 侧无内部可变状态。
unsafe impl Sync for CppHostVtable {}
