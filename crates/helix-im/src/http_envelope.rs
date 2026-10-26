//! ADR-007 HTTP 信封解层（从 `module.rs` 外提，守 module.rs ≤ baseline · 多路径共享）。
//!
//! host `run_http_envelope` 对 HTTP 响应回灌形态 = `{status, headers, body: base64(raw_go_body)}`；
//! 消费侧须先校验 numeric `status` 为 2xx，再剥信封 + base64 decode 出裸 Go body。
//! SyncPull / OutboundReadReply / LoadOlderContext / F1 send 兜底对账皆走此边界。
//! base64 decode 在 helix-im 内联（HX-C001：不依赖 helix-driver-host）。

use crate::error::ImError;

/// 校验 ADR-007 HTTP 信封为 2xx 后解出裸 Go body 字节。
///
/// 非 2xx、信封或 base64 畸形均返回 `Err`，不让失败响应进入成功/空结果解析。
pub(crate) fn unwrap_sync_envelope(reply: &[u8]) -> Result<Vec<u8>, ImError> {
    let body = unwrap_transport_envelope(reply)?;
    validate_business_status(&body)?;
    Ok(body)
}

/// 专用业务解码器可保留原有错误码合同，传输信封仍统一校验。
pub(crate) fn unwrap_transport_envelope(reply: &[u8]) -> Result<Vec<u8>, ImError> {
    let env: serde_json::Value = serde_json::from_slice(reply)
        .map_err(|e| ImError::Parse(format!("sync envelope JSON parse error: {}", e)))?;
    let status = env
        .get("status")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| {
            ImError::Parse("sync envelope missing numeric `status` field".to_string())
        })?;
    if !(200..300).contains(&status) {
        // 非 2xx 同时保留服务端显式业务失败；畸形错误体仍以 HTTP 状态为准。
        let reason = env
            .get("body")
            .and_then(serde_json::Value::as_str)
            .and_then(|body| crate::base64::decode(body).ok())
            .and_then(|body| validate_business_status(&body).err())
            .map(|error| format!("; {error}"))
            .unwrap_or_default();
        return Err(ImError::Parse(format!("sync HTTP status {status}{reason}")));
    }
    let body_b64 = env
        .get("body")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ImError::Parse("sync envelope missing string `body` field".to_string()))?;
    let body = crate::base64::decode(body_b64)
        .map_err(|e| ImError::Parse(format!("sync envelope body base64 decode error: {}", e)))?;
    Ok(body)
}

/// 剥 HTTP 成功信封并校验 2xx；媒体 prepare/complete 等会推进状态机的写路径使用。
///
/// `HttpRequester` 会把 4xx/5xx 也编码为 `PortOutcome::Ok`，因此业务层不能只解 body；
/// 否则异常响应若碰巧带有形似成功的 JSON，就可能错误推进写状态。
pub(crate) fn unwrap_success_envelope(reply: &[u8], operation: &str) -> Result<Vec<u8>, ImError> {
    unwrap_sync_envelope(reply).map_err(|error| match error {
        ImError::RemoteBusiness { .. } => error,
        _ => ImError::Parse(format!("{operation}: {error}")),
    })
}

/// 明确失败在业务读取 data 前返回；非信封的原始成功载荷保持兼容。
fn validate_business_status(body: &[u8]) -> Result<(), ImError> {
    #[derive(serde::Deserialize)]
    struct BusinessReply<'a> {
        #[serde(borrow)]
        status: Option<std::borrow::Cow<'a, str>>,
        #[serde(borrow)]
        message: Option<&'a serde_json::value::RawValue>,
    }
    let Ok(reply) = serde_json::from_slice::<BusinessReply<'_>>(body) else {
        return Ok(());
    };
    if let Some(status) = reply.status.filter(|s| {
        ["failed", "failure", "error", "fail"]
            .iter()
            .any(|failed| s.eq_ignore_ascii_case(failed))
    }) {
        return Err(ImError::RemoteBusiness {
            status: crate::failure_context::clean_text(&status, 32),
            message: crate::failure_context::clean_text(
                reply
                    .message
                    .and_then(|raw| {
                        serde_json::from_str::<std::borrow::Cow<'_, str>>(raw.get()).ok()
                    })
                    .as_deref()
                    .unwrap_or("remote request rejected"),
                512,
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{unwrap_success_envelope, unwrap_sync_envelope};
    use serde_json::json;

    /// Sync 解层先校验 numeric status 为 2xx，再解码 body，拒绝 4xx/5xx 伪成功。
    #[test]
    fn sync_envelope_requires_2xx_numeric_status_and_decodes_body() {
        let ok = serde_json::to_vec(&json!({
            "status": 200,
            "headers": [],
            "body": "eyJzdGF0dXMiOiJTVUNDRVNTIn0="
        }))
        .unwrap();
        assert_eq!(
            unwrap_sync_envelope(&ok).unwrap(),
            br#"{"status":"SUCCESS"}"#
        );

        for status in [404, 503] {
            let failed = serde_json::to_vec(&json!({
                "status": status,
                "headers": [],
                "body": "eyJzdGF0dXMiOiJTVUNDRVNTIn0="
            }))
            .unwrap();
            let error = unwrap_sync_envelope(&failed).unwrap_err();
            assert!(error.to_string().contains(&format!("HTTP status {status}")));
        }

        let wrong_status_type = serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "headers": [],
            "body": "e30="
        }))
        .unwrap();
        let error = unwrap_sync_envelope(&wrong_status_type).unwrap_err();
        assert!(error.to_string().contains("missing numeric `status`"));
    }

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
