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

unsafe impl Send for HelixCallbacks {}
unsafe impl Sync for HelixCallbacks {}
