//! WasmHttp — gloo-net fetch

use helix_core::effect::{HttpRequest, HttpResponse};
use helix_core::PortError;

pub struct WasmHttp;

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl helix_core::ports::HttpRequester for WasmHttp {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        // TODO: gloo_net::http::Request::new(&req.url).method(&req.method)...send().await
        todo!("WasmHttp::request")
    }
}
