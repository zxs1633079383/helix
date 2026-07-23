use super::command::send_command_tick;
use super::handle::HelixHandle;
use super::shared::{ffi_payload_to_vec, FFI_INVALID, FFI_NULL};

/// 提交一个严格版本化的 SurfaceRuntime action envelope（UI → engine）。
///
/// 与 `helix_command` 不同，这个 ABI 没有可伪造的 business command name：payload 必须是完整
/// `ActionInvocationEnvelope`，由 helix-im 在入队前校验 schema/unknown fields，并在 module 中再与
/// host 注入的 runtime actor 和 opaque lease 比对。返回 `FFI_OK` 仅代表 Tick 已入队；业务完成仍需
/// 等待同 requestId causation 的 `im:interaction:frame`。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `envelope`：指向至少 `envelope_len` 字节、并在本调用期间保持有效的内存；函数返回前会复制。
#[no_mangle]
pub unsafe extern "C" fn helix_invoke_action(
    handle: *mut HelixHandle,
    envelope: *const u8,
    envelope_len: usize,
) -> i32 {
    if handle.is_null() {
        return FFI_NULL;
    }
    // SAFETY: 调用方契约保证 envelope 非 null 时指向至少 envelope_len 字节；helper 在返回前复制。
    let payload = unsafe { ffi_payload_to_vec(envelope, envelope_len) };
    let command = match helix_im::client_api::build_action_invocation_command(&payload) {
        Ok(command) => command,
        Err(error) => {
            tracing::warn!(
                ?error,
                "action invocation envelope rejected at FFI boundary"
            );
            return FFI_INVALID;
        }
    };
    // SAFETY: handle 非 null 已检查；来自任一 Helix create 入口。
    let handle_ref = unsafe { &*handle };
    send_command_tick(handle_ref, command, None)
}
