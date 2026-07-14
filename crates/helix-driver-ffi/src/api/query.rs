use std::time::Duration;

use helix_core::ports::Storage;

use super::handle::HelixHandle;
use super::shared::{ffi_str, FFI_BUSY, FFI_INVALID, FFI_NULL, FFI_OK, FFI_UNIMPLEMENTED};

// ── helix_query / helix_query_free ───────────────────────────────────────────

/// 同步读取 engine 状态。
///
/// ## 查询边界
///
/// 元信息 query 由 FFI 直接返回：
/// - `helix_capabilities`：返回移动端公开契约能力集 JSON
/// - `helix_version`：返回 `{"abi":1}`
///
/// IM 本地同步 query 由 `helix-im` 负责 `query_name → ScanSpec + response shape`：
/// - `im_query_dialog_list`
/// - `im_query_messages_by_channel`
///
/// `driver-ffi` 只执行 helix-im 产出的 Storage scan 计划并封装返回 buffer，不在 driver 层硬编码
/// IM 表名/列名/WHERE 语义，避免业务 schema 泄漏进平台驱动。
///
/// ## 配对约定
///
/// 成功（返回 0）时，`*out_ptr` 指向 Rust 分配的 boxed slice；
/// C++ 侧**必须**调用 `helix_query_free(*out_ptr, *out_len)` 释放，禁止 free() / delete[]。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `query_name` / `query_payload`：各自指向对应 `_len` 字节有效内存，在本调用期间有效。
/// - `out_ptr` / `out_len`：非 null，指向调用方分配的存储位置。
#[no_mangle]
pub unsafe extern "C" fn helix_query(
    handle: *mut HelixHandle,
    query_name: *const u8,
    query_name_len: usize,
    query_payload: *const u8,
    query_payload_len: usize,
    out_ptr: *mut *mut u8,
    out_len: *mut usize,
) -> i32 {
    if handle.is_null() || out_ptr.is_null() || out_len.is_null() {
        return FFI_NULL;
    }
    // G-10：返回 FFI_UNIMPLEMENTED 前先清零 out 参数——C++ 侧若误读 *out_ptr/*out_len（无视返回码）
    // 得到 (null, 0) 而非未初始化栈垃圾，杜绝对随机指针 helix_query_free（UB）。out_ptr/out_len 非 null
    // 已确认。
    // SAFETY: out_ptr/out_len 非 null 已检查；调用方契约保证各指向其分配的可写存储位置。
    unsafe {
        *out_ptr = std::ptr::null_mut();
        *out_len = 0;
    }
    // SAFETY: query_name 指针/长度由调用方保证在本调用期间有效，且 name 必须是 UTF-8。
    let query_name = match unsafe { ffi_str(query_name, query_name_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };
    let query_payload = if query_payload.is_null() || query_payload_len == 0 {
        &[][..]
    } else {
        // SAFETY: query_payload 指针/长度由调用方保证在本调用期间有效。
        unsafe { std::slice::from_raw_parts(query_payload, query_payload_len) }
    };

    match query_name.as_str() {
        // SAFETY: out_ptr/out_len 非 null 已检查；write_query_bytes 只写回新分配的 boxed slice 指针与长度。
        "helix_capabilities" => unsafe {
            write_query_bytes(
                helix_im::mobile_contract::capabilities_json(),
                out_ptr,
                out_len,
            )
        },
        // SAFETY: out_ptr/out_len 非 null 已检查；write_query_bytes 只写回新分配的 boxed slice 指针与长度。
        "helix_version" => unsafe { write_query_bytes(br#"{"abi":1}"#.to_vec(), out_ptr, out_len) },
        _ => {
            let plan = match helix_im::mobile_query::plan(query_name.as_str(), query_payload) {
                Ok(plan) => plan,
                Err(_) => return FFI_UNIMPLEMENTED,
            };
            // SAFETY: handle 非 null 已检查；调用方契约保证来自 helix_create_with_network。
            let handle_ref = unsafe { &*handle };
            let runtime = handle_ref.query.runtime.clone();
            let storage = handle_ref.query.storage.clone();
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            runtime.spawn(async move {
                let result = match plan {
                    helix_im::mobile_query::MobileQueryPlan::Scan { spec, shape } => storage
                        .scan(spec)
                        .await
                        .map(|rows| helix_im::mobile_query::shape_json(shape, rows)),
                };
                let _ = tx.send(result);
            });

            match rx.recv_timeout(Duration::from_secs(3)) {
                // SAFETY: out_ptr/out_len 非 null 已检查；bytes 所有权在此转交给 write_query_bytes 封装成返回缓冲区。
                Ok(Ok(bytes)) => unsafe { write_query_bytes(bytes, out_ptr, out_len) },
                Ok(Err(error)) => {
                    tracing::warn!(
                        query = query_name.as_str(),
                        error = ?error,
                        "mobile sync query storage failed"
                    );
                    FFI_INVALID
                }
                Err(_) => FFI_BUSY,
            }
        }
    }
}

/// 释放 `helix_query` 返回的 boxed slice。
///
/// 仅用于释放 `helix_query` 成功时通过 out_ptr/out_len 返回的内存，
/// 禁止用于任何其他 Rust 分配的内存（UB）。
///
/// # Safety
///
/// - `ptr`：必须是 `helix_query` 通过 `out_ptr` 写入的值，或 null（null 时无操作）。
/// - `len`：必须与 `helix_query` 写入 `out_len` 的值完全一致。
/// - 调用后 `ptr` 失效，不可再访问（禁 double-free）。
#[no_mangle]
pub unsafe extern "C" fn helix_query_free(ptr: *mut u8, len: usize) {
    if ptr.is_null() {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(ptr, len);
    // SAFETY: ptr 由 Box::<[u8]>::into_raw(boxed_slice) 产生（helix_query 约定）；
    // len 由调用方通过 out_len 取得，与 boxed_slice 长度完全一致。
    // slice_from_raw_parts_mut + Box::from_raw 重建 Box<[u8]> 后 drop 正确释放。
    drop(unsafe { Box::from_raw(slice) });
}

unsafe fn write_query_bytes(bytes: Vec<u8>, out_ptr: *mut *mut u8, out_len: *mut usize) -> i32 {
    let mut boxed = bytes.into_boxed_slice();
    let len = boxed.len();
    let ptr = boxed.as_mut_ptr();
    std::mem::forget(boxed);
    // SAFETY: out_ptr/out_len 已由调用者提供且在 helix_query 中检查非 null；这里仅写回 boxed slice 元信息。
    unsafe {
        *out_ptr = ptr;
        *out_len = len;
    }
    FFI_OK
}
