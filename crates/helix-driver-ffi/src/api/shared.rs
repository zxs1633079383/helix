// ── FFI 返回码 ──────────────────────────────────────────────────────────────
/// 操作成功
pub const FFI_OK: i32 = 0;
/// 传入 handle / 指针为 null
pub const FFI_NULL: i32 = -1;
/// 字节序列不是合法 UTF-8
pub const FFI_UTF8: i32 = -2;
/// 参数取值无效（header registry 错误等）
pub const FFI_INVALID: i32 = -3;
/// tick channel 满，调用方应稍后重试（M3-F4 pump 接入前此码不会实际出现）
pub const FFI_BUSY: i32 = -4;
/// 功能尚未实现（本轮留 stub，M3-F4 pump 接入后填充）
pub const FFI_UNIMPLEMENTED: i32 = -5;
// ── 内部工具 ─────────────────────────────────────────────────────────────────

/// 将 C 侧裸指针 + 长度转换为 Rust `String`，立即拷贝（ACL-3）。
///
/// # Safety
///
/// 调用方保证 `ptr` 非 null 时指向至少 `len` 字节有效内存，且仅在本次调用期间有效。
pub(super) unsafe fn ffi_str(ptr: *const u8, len: usize) -> Result<String, i32> {
    if ptr.is_null() {
        return Err(FFI_NULL);
    }
    // SAFETY: ptr 非 null 已确认；调用方契约保证指向至少 len 字节有效内存。
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| FFI_UTF8)
}

pub(super) unsafe fn ffi_payload_to_vec(ptr: *const u8, len: usize) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        return Vec::new();
    }
    // SAFETY: ptr 非 null 且调用方契约保证指向至少 len 字节有效内存；这里立即拷贝，不持有原指针。
    unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
}
