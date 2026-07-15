use std::borrow::Cow;

use bytes::Bytes;
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

    // SAFETY: handle 非 null 已检查；来自 helix_create_with_network。
    let handle_ref = unsafe { &*handle };
    send_command_tick(handle_ref, cmd, trace)
}

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

fn build_app_command(public_name: String, payload_vec: Vec<u8>) -> Result<AppCommand, i32> {
    let normalized_payload = match helix_im::mobile_contract::normalize_command_payload(
        public_name.as_str(),
        &payload_vec,
    ) {
        Ok(bytes) => bytes,
        Err(error) => {
            tracing::warn!(
                command = public_name.as_str(),
                ?error,
                "mobile command payload rejected"
            );
            return Err(FFI_INVALID);
        }
    };
    let payload_bytes = Bytes::from(normalized_payload);

    let core_name = helix_im::mobile_contract::command_alias(public_name.as_str())
        .unwrap_or(public_name.as_str());

    // G-6 / PI-1：name 命中 outbound 闭集（88 命令）→ `Cow::Borrowed` 静态名免堆分配（热路径写命令
    // 净分配仅 payload 1 次）；miss（im_send_message/im_reconnect/query 名）→ `Cow::Owned` fallback
    // （低频，仍正确）。闭集真源在 helix-im registry（driver 不复制清单，避免漂移）。
    let name: Cow<'static, str> = match helix_im::outbound::canonical_command_name(core_name) {
        Some(static_name) => Cow::Borrowed(static_name),
        None => Cow::Owned(core_name.to_string()),
    };
    Ok(AppCommand {
        name,
        payload: payload_bytes,
    })
}

fn send_command_tick(handle: &HelixHandle, cmd: AppCommand, trace: Option<TraceCarrier>) -> i32 {
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
