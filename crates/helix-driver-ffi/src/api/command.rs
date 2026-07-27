use helix_core::tick::{AppCommand, Tick};
use helix_driver_host::{LabelKey, MetricEvent, MetricId, MetricLabels, TraceCarrier};

use super::handle::HelixHandle;
use super::shared::{ffi_payload_to_vec, ffi_str, FFI_BUSY, FFI_INVALID, FFI_NULL, FFI_OK};

// ── helix_command ────────────────────────────────────────────────────────────

/// 向 engine 发送应用命令（UI → engine）。
///
/// 立即将 `name` / `payload` 拷贝到 Rust 堆（ACL-3），构造 `Tick::Command`，
/// 并 **非阻塞** 尝试送入 tick channel（pump 线程 rx 端消费 → step → Effect → 批回 C++）：
///   - channel 满 → 返回 `FFI_BUSY`（C++ 侧应稍后重试，禁静默丢弃）。
///   - destroy 后 channel 已关闭 → 返回 `FFI_NULL`。
///
/// # Safety
///
/// - `handle`：有效指针或 null。
/// - `name` / `payload`：各自指向至少对应 `_len` 字节、在本调用期间有效的内存。
///   `name` 必须是合法 UTF-8；`payload` 为任意字节（不强要求 UTF-8）。
#[no_mangle]
pub unsafe extern "C" fn helix_command(
    handle: *mut HelixHandle,
    name: *const u8,
    name_len: usize,
    payload: *const u8,
    payload_len: usize,
) -> i32 {
    if handle.is_null() {
        return FFI_NULL;
    }
    // SAFETY: 调用方契约保证 name 指向至少 name_len 字节有效 UTF-8 内存。
    let public_name = match unsafe { ffi_str(name, name_len) } {
        Ok(s) => s,
        Err(code) => return code,
    };
    // SAFETY: 调用方契约保证 payload 指向至少 payload_len 字节有效内存；payload 可为任意字节。
    let payload_vec = unsafe { ffi_payload_to_vec(payload, payload_len) };
    let (payload_vec, trace) = match extract_trace_sidecar(payload_vec) {
        Ok(parts) => parts,
        Err(code) => return code,
    };
    let cmd = match build_app_command(public_name, payload_vec) {
        Ok(cmd) => cmd,
        Err(code) => return code,
    };

    // SAFETY: handle 非 null 已检查；来自任一 Helix create 入口。
    let handle_ref = unsafe { &*handle };
    send_command_tick(handle_ref, cmd, trace)
}

/// Separates transport tracing from the domain payload before command decoding.
fn extract_trace_sidecar(payload: Vec<u8>) -> Result<(Vec<u8>, Option<TraceCarrier>), i32> {
    const TRACE_KEY: &[u8] = b"\"__trace\"";
    if !payload
        .windows(TRACE_KEY.len())
        .any(|window| window == TRACE_KEY)
    {
        return Ok((payload, None));
    }

    let mut value: serde_json::Value = serde_json::from_slice(&payload).map_err(|error| {
        tracing::warn!(?error, "command with trace sidecar is invalid JSON");
        FFI_INVALID
    })?;
    let Some(trace) = value
        .as_object_mut()
        .and_then(|object| object.remove("__trace"))
    else {
        return Ok((payload, None));
    };
    let trace_json = serde_json::to_string(&trace).map_err(|error| {
        tracing::warn!(?error, "trace sidecar serialization failed");
        FFI_INVALID
    })?;
    let carrier = TraceCarrier::from_json_str(&trace_json).map_err(|error| {
        tracing::warn!(?error, "trace sidecar rejected");
        FFI_INVALID
    })?;
    let payload = serde_json::to_vec(&value).map_err(|error| {
        tracing::warn!(?error, "command serialization failed");
        FFI_INVALID
    })?;
    Ok((payload, Some(carrier)))
}

/// Routes the one typed private surface request without widening the public API.
fn build_app_command(public_name: String, payload_vec: Vec<u8>) -> Result<AppCommand, i32> {
    let decoded = if public_name == helix_im::client_api::SURFACE_RESYNC_COMMAND {
        helix_im::client_api::build_surface_resync_command(&payload_vec)
    } else {
        helix_im::client_api::build_command(public_name.as_str(), &payload_vec)
    };
    match decoded {
        Ok(command) => Ok(command),
        Err(error) => {
            tracing::warn!(
                command = public_name.as_str(),
                ?error,
                "mobile command payload rejected"
            );
            Err(FFI_INVALID)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves Mobile surface attachment enters the private typed builder.
    #[test]
    fn surface_snapshot_command_uses_typed_private_builder() {
        let payload = br#"{"surface":"message.timeline","scope":{"kind":"timeline","channelId":"channel-1","windowToken":"latest"}}"#;
        let command = build_app_command(
            helix_im::client_api::SURFACE_RESYNC_COMMAND.to_string(),
            payload.to_vec(),
        )
        .expect("typed surface snapshot command");

        assert_eq!(
            command.name.as_ref(),
            helix_im::client_api::SURFACE_RESYNC_COMMAND
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&command.payload)
                .expect("canonical private payload"),
            serde_json::from_slice::<serde_json::Value>(payload).expect("input payload")
        );
    }

    /// Keeps unrelated reserved commands rejected by the generic FFI entry.
    #[test]
    fn generic_ffi_command_does_not_open_other_private_commands() {
        let result = build_app_command(
            helix_im::interaction_frame::FRAME_APPLIED_COMMAND.to_string(),
            b"{}".to_vec(),
        );

        assert_eq!(
            result.expect_err("private command must stay closed"),
            FFI_INVALID
        );
    }
}

pub(super) fn send_command_tick(
    handle: &HelixHandle,
    cmd: AppCommand,
    trace: Option<TraceCarrier>,
) -> i32 {
    let guard = handle.tick_tx.lock();
    let Some(tx) = guard.as_ref() else {
        return FFI_NULL;
    };

    // `tick_tx` 的锁把「trace slot 入队 + command try_send」序列化成同一临界区，保证 mixed
    // trace slot 入队 + command try_send 在同一临界区，FIFO 与 channel 入队顺序一致。
    handle.command_traces.push_slot(trace);
    match tx.try_send(Tick::Command(cmd)) {
        Ok(()) => FFI_OK,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            handle.command_traces.rollback_last();
            if handle.metrics.is_enabled() {
                let _ = handle.metrics.try_record(MetricEvent::counter(
                    MetricId::FfiBusyTotal,
                    1.0,
                    MetricLabels::one(LabelKey::Stage, "core"),
                ));
            }
            FFI_BUSY
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            handle.command_traces.rollback_last();
            FFI_NULL
        }
    }
}
