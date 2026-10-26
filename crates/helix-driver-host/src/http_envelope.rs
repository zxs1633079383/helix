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

use helix_core::effect::HttpRequest;
use helix_core::port_codec::http_result_to_outcome;
use helix_core::ports::HttpRequester;
use helix_core::tick::PortOutcome;

pub use helix_core::port_codec::base64_encode;

/// 执行一条 HTTP 请求，把响应编码成 PortReply JSON 信封（Http / HttpFire worker 共用）。
///
/// 仅 `HttpRequester` 层错误 → `PortOutcome::Err(classify_port_error(e))`；
/// HTTP 4xx/5xx 仍走 Ok 路径（status 字段携带，业务层判）。
pub async fn run_http_envelope<H>(http: &H, req: HttpRequest) -> PortOutcome
where
    H: HttpRequester + ?Sized,
{
    http_result_to_outcome(http.request(req).await)
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
