//! ADR-007 HTTP 信封解层（从 `module.rs` 外提，守 module.rs ≤ baseline · 多路径共享）。
//!
//! host `run_http_envelope` 对 2xx HTTP 响应回灌形态 = `{status, headers, body: base64(raw_go_body)}`；
//! SyncPull / OutboundReadReply / LoadOlderContext / F1 send 兜底对账皆须先剥此信封 + base64 decode
//! 出裸 Go body 再解析。base64 decode 在 helix-im 内联（HX-C001：不依赖 helix-driver-host）。

use crate::error::ImError;

/// 剥 ADR-007 HTTP 信封：`{status, headers, body: base64(raw_go_body)}` → 裸 Go body 字节。
///
/// 边界零信任（HX-C 不变量 4）：信封 / base64 畸形 → `Err`，不 unwrap / panic / 静默吞。
pub(crate) fn unwrap_sync_envelope(reply: &[u8]) -> Result<Vec<u8>, ImError> {
    let env: serde_json::Value = serde_json::from_slice(reply)
        .map_err(|e| ImError::Parse(format!("sync envelope JSON parse error: {}", e)))?;
    let body_b64 = env
        .get("body")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ImError::Parse("sync envelope missing string `body` field".to_string()))?;
    crate::base64::decode(body_b64)
        .map_err(|e| ImError::Parse(format!("sync envelope body base64 decode error: {}", e)))
}
