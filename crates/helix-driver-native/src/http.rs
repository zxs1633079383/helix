//! NativeHttp — reqwest client

use helix_core::PortError;
use helix_core::ports::HttpRequester;
use helix_core::effect::{HttpRequest, HttpResponse};

pub struct NativeHttp {
    // TODO: client: reqwest::Client,
}

impl NativeHttp {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl HttpRequester for NativeHttp {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        // TODO: 用 reqwest::Client 发送请求，把 reqwest::Error 翻译为 PortError::Http
        todo!("NativeHttp::request")
    }
}
