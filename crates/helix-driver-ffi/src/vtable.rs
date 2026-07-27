//! CppHostVtable — C++ host 回调表（C ABI，repr(C)）。
//!
//! ## 设计（C++ 直连模型）
//!
//! 本模型下 **Rust 持有网络**（SharedWsClient / SharedHttpClient），
//! C++ host 不再负责喂帧/喂响应，只需要：
//!   1. 接收 Rust pump 吐出的领域事件批（`on_domain_events_batch`）
//!   2. 提供 host 壁钟时间（`now_ms`）
//!
//! ## host = C++ shim（非 Dart 直连）
//!
//! 真 host = **C++ shim 层**（与 Rust pump 同进程、同步调用），**不是 Dart**。Dart 不能把 raw
//! `ptr` 当 C 函数指针直接消费（`NativeCallable.listener` fire-and-forget 无返回值；`.isolateLocal`
//! 从 pump 专线程调 = UB）。故 vtable 两个回调由 C++ shim 同步填，shim 再把数据投递给 Dart isolate。
//! 仓外 Dart/Flutter 工程消费本契约（`docs/flutter-demo-pointer.md`），本仓只交付到 C ABI。
//!
//! ## ctx 生命周期契约
//!
//! - `ctx` 单 owner = **C++ shim host**，Rust 不解引用、不拥有，只原样回传给回调函数。
//! - **C++ 不得在 `helix_destroy` 返回前释放 ctx**：`helix_destroy` 关 tick channel 后 join pump
//!   线程，join 完成前 pump 仍可能调 `on_domain_events_batch`——提前释放 ctx 是 use-after-free。

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
    /// pump 在一个 step 产出的所有 `Effect::Emit` 序列化为单个 byte buffer 后，**整批一次**调用
    /// 此函数（HX-C007，绝不逐条过边界）。
    ///
    /// - `ctx`：透传 host 自定义 context，Rust 不解引用。
    /// - `ptr`：指向序列化后的 DomainEventBytes 批数据；**仅在本次回调期间有效**（借出契约）。
    /// - `len`：批数据字节长度。
    ///
    /// ## buffer 所有权 = 借出契约（冻结，不新增 free 符号）
    ///
    /// `ptr..ptr+len` 由 Rust own，仅在 `cb` 回调期间有效，回调返回后 Rust 就地 drop。**这是正确
    /// 边界，无 UAF**——不改 ownership-transfer、不新增 `helix_free_batch`；本轮唯一 additive
    /// C ABI 是显式 Go/Java/WS 初始化入口，不改变批 buffer 所有权。
    /// Dart 异步如何吸收：**C++ shim 在回调线程内 `malloc + memcpy`** 到稳定堆，再投递给 Dart
    /// isolate（`docs/flutter-demo-pointer.md` ①稳定化）。稳定化是 shim 层的事，非 Rust 导出。
    ///
    /// # Safety（C++ 侧）
    ///
    /// 函数必须在返回前拷贝或消费 `ptr..ptr+len` 的数据，不得在回调返回后持有指针。
    pub on_domain_events_batch:
        Option<unsafe extern "C" fn(ctx: *mut c_void, ptr: *const u8, len: usize)>,

    /// 壁钟时间（Unix epoch 毫秒）。
    ///
    /// pump 在每次 `ExecutionShell::step()` 前同步调用，注入确定性时间（HX-C002 / GOAL §1.2）。
    /// **由 C++ shim host 同步提供**（shim 与 pump 同进程，回调线程内同步返回 epoch ms）——确定性
    /// 注入靠此字段成立。**禁移除字段让 Rust SystemTime 成正路**（那会使 FFI 端比 native 更不确定）。
    /// 测试用 mock now_ms 跑确定性回放。None 时 clock.rs fallback SystemTime（仅健壮性兜底 + warn）。
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
