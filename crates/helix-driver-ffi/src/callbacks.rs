//! HelixCallbacks — C ABI 回调结构体
//!
//! 宿主（iOS/Android/C）通过此结构体注入 I/O 能力，替代 tokio/web-sys。

/// C ABI 回调集合（repr(C) 保证跨 FFI ABI 稳定）
#[repr(C)]
pub struct HelixCallbacks {
    /// 发送 WS 帧：宿主实现，Rust 调用此函数发出帧
    pub send_ws_frame: extern "C" fn(data: *const u8, len: usize),

    /// HTTP 请求：宿主实现，Rust 发起请求，宿主异步完成后调用 on_http_response
    pub send_http: extern "C" fn(
        req_id: u64,
        url: *const u8,
        url_len: usize,
        method: *const u8,
        method_len: usize,
        body: *const u8,
        body_len: usize,
    ),

    /// 领域事件回调：宿主接收已序列化的 DomainEventBytes
    pub on_domain_event: extern "C" fn(data: *const u8, len: usize),

    /// 获取当前时间（Unix epoch 毫秒）
    pub now_ms: extern "C" fn() -> u64,
}
