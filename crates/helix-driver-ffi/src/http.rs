//! FFI HTTP requester — thin wrapper over shared host HTTP.

use helix_core::effect::{HttpRequest, HttpResponse};
use helix_core::ports::HttpRequester;
use helix_core::PortError;
use helix_driver_host::SharedHttpClient;

#[derive(Clone)]
pub struct FfiHttpRequester {
    inner: SharedHttpClient,
}

impl FfiHttpRequester {
    pub fn new(inner: SharedHttpClient) -> Self {
        Self { inner }
    }

    pub fn shared(&self) -> &SharedHttpClient {
        &self.inner
    }
}

#[async_trait::async_trait]
impl HttpRequester for FfiHttpRequester {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        self.inner.request(req).await
    }
}
