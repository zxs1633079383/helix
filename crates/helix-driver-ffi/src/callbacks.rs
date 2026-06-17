//! HelixCallbacks — C ABI callback table.
//!
//! Flutter product path uses Rust shared HTTP/WS clients. Network callbacks are
//! retained only as a host-network escape hatch.

use std::ffi::c_void;

/// C ABI 回调集合（repr(C) 保证跨 FFI ABI 稳定）
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HelixCallbacks {
    /// 可选逃生口：宿主实现 WS send。Flutter 主路径应为空。
    pub send_ws_frame: Option<extern "C" fn(ctx: *mut c_void, data: *const u8, len: usize) -> i32>,

    /// 可选逃生口：宿主实现 HTTP。Flutter 主路径应为空。
    pub send_http: Option<
        extern "C" fn(
            ctx: *mut c_void,
            req_id: u64,
            url: *const u8,
            url_len: usize,
            method: *const u8,
            method_len: usize,
            body: *const u8,
            body_len: usize,
        ),
    >,

    /// 领域事件回调：宿主接收已序列化的 DomainEventBytes
    pub on_domain_event: Option<extern "C" fn(ctx: *mut c_void, data: *const u8, len: usize)>,

    /// 获取当前时间（Unix epoch 毫秒）
    pub now_ms: Option<extern "C" fn(ctx: *mut c_void) -> u64>,

    /// 用户自定义 context（host 自己管理，Rust 不解释）。
    pub ctx: *mut c_void,
}

// SAFETY: HelixCallbacks 仅持有 `extern "C" fn` 函数指针（线程无关）+ `ctx: *mut c_void`
// 不透明句柄。host 侧契约保证 ctx 指向的对象在所有回调线程间安全共享（host 自管同步，
// Rust 不解引用、不拥有 ctx——只原样回传给 host 的 extern "C" fn）。故跨线程传递/共享安全。
unsafe impl Send for HelixCallbacks {}
// SAFETY: 见上——函数指针 + 不透明 ctx（host 自管线程安全），无内部可变状态被 Rust 触碰。
unsafe impl Sync for HelixCallbacks {}
