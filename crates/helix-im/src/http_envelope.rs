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

/// 剥 HTTP 成功信封并校验 2xx；媒体 prepare/complete 等会推进状态机的写路径使用。
///
/// `HttpRequester` 会把 4xx/5xx 也编码为 `PortOutcome::Ok`，因此业务层不能只解 body；
/// 否则异常响应若碰巧带有形似成功的 JSON，就可能错误推进写状态。
pub(crate) fn unwrap_success_envelope(reply: &[u8], operation: &str) -> Result<Vec<u8>, ImError> {
    let env: serde_json::Value = serde_json::from_slice(reply)
        .map_err(|e| ImError::Parse(format!("{operation} HTTP envelope JSON: {e}")))?;
    let status = env
        .get("status")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            ImError::Parse(format!(
                "{operation} HTTP envelope missing numeric `status` field"
            ))
        })?;
    if !(200..300).contains(&status) {
        return Err(ImError::Parse(format!("{operation} HTTP status {status}")));
    }
    let body_b64 = env
        .get("body")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ImError::Parse(format!(
                "{operation} HTTP envelope missing string `body` field"
            ))
        })?;
    crate::base64::decode(body_b64)
        .map_err(|e| ImError::Parse(format!("{operation} HTTP envelope body base64: {e}")))
}

#[cfg(test)]
mod tests {
    use super::unwrap_success_envelope;
    use serde_json::json;

    #[test]
    fn success_envelope_requires_2xx_numeric_status_and_decodes_body() {
        let ok = serde_json::to_vec(&json!({
            "status": 200,
            "headers": [],
            "body": "eyJzdGF0dXMiOiJTVUNDRVNTIn0="
        }))
        .unwrap();
        assert_eq!(
            unwrap_success_envelope(&ok, "media prepare").unwrap(),
            br#"{"status":"SUCCESS"}"#
        );

        let failed = serde_json::to_vec(&json!({
            "status": 409,
            "headers": [],
            "body": "eyJzdGF0dXMiOiJTVUNDRVNTIn0="
        }))
        .unwrap();
        let error = unwrap_success_envelope(&failed, "media prepare").unwrap_err();
        assert!(error.to_string().contains("HTTP status 409"));

        let wrong_status_type = serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "headers": [],
            "body": "e30="
        }))
        .unwrap();
        let error = unwrap_success_envelope(&wrong_status_type, "media prepare").unwrap_err();
        assert!(error.to_string().contains("missing numeric `status`"));
    }
}
