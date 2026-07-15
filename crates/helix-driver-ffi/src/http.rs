//! FFI HTTP requester — thin wrapper over shared host HTTP crosscut semantics.

use helix_core::effect::{HttpRequest, HttpResponse};
use helix_core::ports::HttpRequester;
use helix_core::PortError;
use helix_driver_host::{
    AuthTokenRegistry, CrossCuttingHttp, HostOtelRuntime, HttpOutcomeObserver, SharedHttpClient,
};

use crate::event_sink::FfiEventSink;

pub struct FfiHttpRequester {
    inner: CrossCuttingHttp<SharedHttpClient, FfiEventSink>,
}

impl FfiHttpRequester {
    pub fn new(
        inner: SharedHttpClient,
        event_sink: std::sync::Arc<FfiEventSink>,
        auth: AuthTokenRegistry,
        outcome_observer: HttpOutcomeObserver,
    ) -> Self {
        Self {
            inner: CrossCuttingHttp::new(inner, event_sink, auth, outcome_observer),
        }
    }

    pub fn with_otel(mut self, runtime: HostOtelRuntime) -> Self {
        self.inner = self.inner.with_otel(runtime);
        self
    }
}

#[async_trait::async_trait]
impl HttpRequester for FfiHttpRequester {
    async fn request(&self, req: HttpRequest) -> Result<HttpResponse, PortError> {
        self.inner.request(req).await
    }
}
