//! HTTP envelope — 把 `HttpRequester::request` 的响应编码成 PortReply JSON 信封。
//!
//! ## 双端共享（ADR-007）
//!
//! HTTP 响应 → JSON 信封的编码统一在此 host 模块，native / FFI 两端复用，
//! 不各写一份（否则信封字段语义双端漂移）。仅 `HttpRequester` 边界错误经
//! `classify_port_error` 翻译为 `PortError::*`。
//!
//! ## 信封 wire 契约（逐字段，helix-im 反序列化端必须对齐）
//!
//! ```json
//! {"status": <u16, 含 4xx/5xx>, "headers": [[k, v], ...], "body": "<base64>"}
//! ```
//! - `status`：HTTP 状态码（含 4xx/5xx，不在此分类——业务层判）。
//! - `headers`：**array of 2-elem array**（非 object），保留重复 header 与顺序。
//! - `body`：标准 base64（表 `+/`，`=` padding），避免 binary 污染 JSON string。

use bytes::Bytes;
use serde_json::json;

use helix_core::effect::HttpRequest;
use helix_core::ports::HttpRequester;
use helix_core::tick::{PortOutcome, ReplyBytes};

use crate::classify_port_error;

/// 执行一条 HTTP 请求，把响应编码成 PortReply JSON 信封（Http / HttpFire worker 共用）。
///
/// 仅 `HttpRequester` 层错误 → `PortOutcome::Err(classify_port_error(e))`；
/// HTTP 4xx/5xx 仍走 Ok 路径（status 字段携带，业务层判）。
pub async fn run_http_envelope<H>(http: &H, req: HttpRequest) -> PortOutcome
where
    H: HttpRequester + ?Sized,
{
    match http.request(req).await {
        Ok(resp) => {
            let body_b64 = base64_encode(&resp.body);
            let json_bytes = serde_json::to_vec(&json!({
                "status": resp.status,
                "headers": resp.headers,
                "body": body_b64,
            }))
            .unwrap_or_default();
            PortOutcome::Ok(ReplyBytes(Bytes::from(json_bytes)))
        }
        Err(e) => PortOutcome::Err(classify_port_error(e)),
    }
}

/// 标准 base64 编码（HTTP body 放进 JSON 时用，避免 binary 污染 JSON string）。
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base64_encode() {
        assert_eq!(base64_encode(b"Man"), "TWFu");
        assert_eq!(base64_encode(b"Ma"), "TWE=");
        assert_eq!(base64_encode(b"M"), "TQ==");
        assert_eq!(base64_encode(b""), "");
    }
}
