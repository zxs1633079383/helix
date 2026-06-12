//! FFI 公开 API（#[no_mangle] C ABI 导出）
//!
//! ## 内存安全约束（必须遵守）
//!
//! - 所有 *const u8 参数：**在函数返回前必须拷贝到 Rust 堆**
//! - 所有 Box<T> → raw ptr 必须用 Box::into_raw / Box::from_raw 配对
//! - panic=abort 保证不跨 FFI unwind（无需 catch_unwind，但不得 panic）

use crate::callbacks::HelixCallbacks;

/// 创建 Helix 引擎实例，返回不透明指针。
/// 宿主负责持有并传递给所有后续 API 调用。
#[no_mangle]
pub extern "C" fn helix_create(callbacks: HelixCallbacks) -> *mut std::ffi::c_void {
    // TODO: 创建 ExecutionShell + FfiTransport + FfiStorage + 注册 ImModule
    //       Box::into_raw(Box::new(engine_handle)) as *mut c_void
    std::ptr::null_mut()
}

/// 销毁 Helix 引擎实例（释放内存）。
/// 调用后 handle 不可再使用。
#[no_mangle]
pub extern "C" fn helix_destroy(handle: *mut std::ffi::c_void) {
    if handle.is_null() {
        return;
    }
    // TODO: drop(unsafe { Box::from_raw(handle as *mut EngineHandle) })
}

/// 宿主收到 WS 帧时调用：把帧注入 engine。
/// 注意：data 指向的 buffer 必须在本函数内拷贝，不得持有。
#[no_mangle]
pub extern "C" fn helix_on_ws_frame(handle: *mut std::ffi::c_void, data: *const u8, len: usize) {
    if handle.is_null() || data.is_null() {
        return;
    }
    // 立即拷贝（ACL-3：C 侧 buffer 回调返回后可能被复用）
    // SAFETY: data 非空已检查；调用方契约保证 data 指向至少 len 字节的有效内存，
    // 且仅在本回调期间有效——因此 from_raw_parts 后立即 to_vec 拷贝、不持有切片。
    let _owned: Vec<u8> = unsafe { std::slice::from_raw_parts(data, len).to_vec() };
    // TODO: 包成 Tick::Inbound 并通过 channel 发给 engine_loop
}

/// 宿主 HTTP 响应回调
#[no_mangle]
pub extern "C" fn helix_on_http_response(
    handle: *mut std::ffi::c_void,
    req_id: u64,
    status: u16,
    body: *const u8,
    body_len: usize,
) {
    if handle.is_null() {
        return;
    }
    let _body_owned: Vec<u8> = if body.is_null() {
        vec![]
    } else {
        // SAFETY: body 非空分支；调用方契约保证 body 指向至少 body_len 字节、
        // 仅在本回调期间有效——立即 to_vec 拷贝、不持有。
        unsafe { std::slice::from_raw_parts(body, body_len).to_vec() }
    };
    // TODO: 包成 PortReply{corr=req_id, Ok(body)} 发给 engine_loop
}
