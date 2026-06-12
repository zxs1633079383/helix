//! HttpRequester port — HTTP 客户端抽象
//!
//! ## 为什么 Transport 和 HttpRequester 分开
//!
//! IM 发送主路径走 HTTP POST（/posts/create 直发），
//! gate fire 走 HTTP（/sync/notify），WS 是另一路。
//! 两路并存，不能合并为单一传输 port。
//!
//! ## 三端实现
//! - native: reqwest（tokio reactor backed）
//! - wasm:   gloo-net/fetch（或 reqwest wasm 后端，无流式，无自定义 TLS）
//! - ffi:    host http 回调 + futures-channel oneshot 包装

use crate::effect::{HttpRequest, HttpResponse};
use crate::error::PortError;
use crate::platform::{MaybeSend, MaybeSync};

/// HTTP 客户端的 core 抽象。
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait HttpRequester: MaybeSend + MaybeSync + 'static {
    /// 发送 HTTP 请求并返回响应。
    ///
    /// driver 负责连接复用、重试策略（helix-im 不重试，失败标 unSend）。
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError>;
}
