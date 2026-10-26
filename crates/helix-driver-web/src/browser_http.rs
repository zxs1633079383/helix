//! Browser Fetch implementation of the shared HTTP port.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::collections::HashMap;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

#[cfg(any(target_arch = "wasm32", test))]
use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, HttpRequest, HttpResponse};
#[cfg(any(target_arch = "wasm32", test))]
use helix_core::ports::EventSink;
use helix_core::ports::HttpRequester;
use helix_core::{apply_auth_intent, AuthKind, PortError, AUTH_KIND_HEADER};

use crate::event_sink::WebEventSink;

const TRANSLATION_SERVICE_URL: &str = "service:translation";

#[derive(Clone, Default)]
struct RequestLifecycle {
    stopped: Arc<AtomicBool>,
    #[cfg(target_arch = "wasm32")]
    active: Rc<RefCell<ActiveRequests>>,
}

#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct ActiveRequests {
    next_id: u64,
    controllers: HashMap<u64, web_sys::AbortController>,
}

impl RequestLifecycle {
    fn ensure_active(&self) -> Result<(), PortError> {
        if self.stopped.load(Ordering::Acquire) {
            Err(PortError::Other(
                "Browser HTTP requester is shut down".into(),
            ))
        } else {
            Ok(())
        }
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn is_active(&self) -> bool {
        !self.stopped.load(Ordering::Acquire)
    }

    #[cfg(target_arch = "wasm32")]
    fn begin(&self) -> Result<(u64, web_sys::AbortController), PortError> {
        self.ensure_active()?;
        let controller = web_sys::AbortController::new()
            .map_err(|error| http_setup_error("abort controller", error))?;
        let mut active = self.active.borrow_mut();
        self.ensure_active()?;
        let id = active.next_id;
        active.next_id = active.next_id.wrapping_add(1);
        if active.controllers.contains_key(&id) {
            return Err(PortError::Http(
                "Browser HTTP request id space exhausted".into(),
            ));
        }
        active.controllers.insert(id, controller.clone());
        Ok((id, controller))
    }

    #[cfg(target_arch = "wasm32")]
    fn finish(&self, id: u64) {
        self.active.borrow_mut().controllers.remove(&id);
    }

    fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        #[cfg(target_arch = "wasm32")]
        {
            let mut active = self.active.borrow_mut();
            for controller in active.controllers.values() {
                controller.abort();
            }
            active.controllers.clear();
        }
    }
}

#[derive(Clone, Default)]
struct AuthSlots {
    session: Option<(String, String)>,
    bot: Option<(String, String)>,
}

/// Driver-owned credentials. The shared business core sees only AuthKind.
#[derive(Clone, Default)]
pub struct WebAuthRegistry {
    inner: Arc<Mutex<AuthSlots>>,
}

impl std::fmt::Debug for WebAuthRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebAuthRegistry")
            .finish_non_exhaustive()
    }
}

impl WebAuthRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_session(
        &self,
        header_name: impl Into<String>,
        header_value: impl Into<String>,
    ) -> Result<(), PortError> {
        self.set(AuthKind::Session, header_name.into(), header_value.into())
    }

    pub fn set_bot(
        &self,
        header_name: impl Into<String>,
        header_value: impl Into<String>,
    ) -> Result<(), PortError> {
        self.set(AuthKind::Bot, header_name.into(), header_value.into())
    }

    pub fn clear(&self, kind: AuthKind) -> Result<(), PortError> {
        let mut slots = self
            .inner
            .lock()
            .map_err(|_| PortError::Http("Web auth registry lock poisoned".into()))?;
        match kind {
            AuthKind::Session => slots.session = None,
            AuthKind::Bot => slots.bot = None,
        }
        Ok(())
    }

    fn set(
        &self,
        kind: AuthKind,
        header_name: String,
        header_value: String,
    ) -> Result<(), PortError> {
        if header_name.trim().is_empty() || header_value.is_empty() {
            return Err(PortError::Http(
                "Web credential header name and value must not be empty".into(),
            ));
        }
        if header_name.eq_ignore_ascii_case(AUTH_KIND_HEADER) {
            return Err(PortError::Http(
                "Web credential header must not use the internal auth-intent name".into(),
            ));
        }
        let mut slots = self
            .inner
            .lock()
            .map_err(|_| PortError::Http("Web auth registry lock poisoned".into()))?;
        let credential = Some((header_name, header_value));
        match kind {
            AuthKind::Session => slots.session = credential,
            AuthKind::Bot => slots.bot = credential,
        }
        Ok(())
    }

    fn snapshot(&self) -> Result<AuthSlots, PortError> {
        self.inner
            .lock()
            .map(|slots| slots.clone())
            .map_err(|_| PortError::Http("Web auth registry lock poisoned".into()))
    }
}

#[derive(Clone)]
pub struct BrowserHttpRequester {
    auth: WebAuthRegistry,
    #[cfg(any(target_arch = "wasm32", test))]
    events: WebEventSink,
    #[cfg(any(target_arch = "wasm32", test))]
    outcome_observer: fn(&str, &Result<HttpResponse, PortError>) -> Option<DomainEventBytes>,
    timeout_ms: u32,
    lifecycle: RequestLifecycle,
}

impl std::fmt::Debug for BrowserHttpRequester {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserHttpRequester")
            .field("timeout_ms", &self.timeout_ms)
            .finish_non_exhaustive()
    }
}

impl BrowserHttpRequester {
    pub fn new(
        auth: WebAuthRegistry,
        events: WebEventSink,
        timeout_ms: u64,
        outcome_observer: fn(&str, &Result<HttpResponse, PortError>) -> Option<DomainEventBytes>,
    ) -> Result<Self, PortError> {
        if timeout_ms == 0 || timeout_ms > i32::MAX as u64 {
            return Err(PortError::Http(format!(
                "Web HTTP timeout must be within 1..={} ms",
                i32::MAX
            )));
        }
        #[cfg(not(any(target_arch = "wasm32", test)))]
        let _ = (events, outcome_observer);
        Ok(Self {
            auth,
            #[cfg(any(target_arch = "wasm32", test))]
            events,
            #[cfg(any(target_arch = "wasm32", test))]
            outcome_observer,
            timeout_ms: timeout_ms as u32,
            lifecycle: RequestLifecycle::default(),
        })
    }

    pub fn auth_registry(&self) -> WebAuthRegistry {
        self.auth.clone()
    }

    /// Prevent new requests and abort all in-flight browser Fetch operations.
    pub fn shutdown(&self) {
        self.lifecycle.shutdown();
    }

    pub fn prepare_request(&self, request: HttpRequest) -> Result<HttpRequest, PortError> {
        let slots = self.auth.snapshot()?;
        Ok(apply_auth_intent(request, |kind| match kind {
            AuthKind::Session => slots.session.clone(),
            AuthKind::Bot => slots.bot.clone(),
        })
        .0)
    }

    #[cfg(any(target_arch = "wasm32", test))]
    fn observe_result(&self, url: &str, result: &Result<HttpResponse, PortError>) {
        if !self.lifecycle.is_active() {
            return;
        }
        if let Some(event) = (self.outcome_observer)(url, result) {
            self.events.emit(event);
            self.events.flush_step();
        }
    }
}

#[cfg(target_arch = "wasm32")]
#[async_trait::async_trait(?Send)]
impl HttpRequester for BrowserHttpRequester {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, PortError> {
        self.lifecycle.ensure_active()?;
        if request.url == TRANSLATION_SERVICE_URL {
            return Err(PortError::Other(
                "translation endpoint is not configured for Web".into(),
            ));
        }
        let request = self.prepare_request(request)?;
        let url = request.url.clone();
        let (request_id, controller) = self.lifecycle.begin()?;
        let result = fetch_request(request, self.timeout_ms, controller).await;
        self.lifecycle.finish(request_id);
        if self.lifecycle.is_active() {
            self.observe_result(&url, &result);
        }
        result
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[async_trait::async_trait]
impl HttpRequester for BrowserHttpRequester {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, PortError> {
        self.lifecycle.ensure_active()?;
        if request.url == TRANSLATION_SERVICE_URL {
            return Err(PortError::Other(
                "translation endpoint is not configured for Web".into(),
            ));
        }
        Err(PortError::Other(
            "Browser Fetch is only available on wasm32".into(),
        ))
    }
}

#[cfg(target_arch = "wasm32")]
async fn fetch_request(
    request: HttpRequest,
    timeout_ms: u32,
    controller: web_sys::AbortController,
) -> Result<HttpResponse, PortError> {
    use std::cell::Cell;

    use js_sys::{Array, Function, Promise, Reflect, Uint8Array};
    use wasm_bindgen::{JsCast, JsValue};
    use wasm_bindgen_futures::JsFuture;
    use web_sys::{Request, RequestInit, Response};

    let init = RequestInit::new();
    init.set_method(&request.method);
    init.set_signal(Some(&controller.signal()));
    let body_array = request
        .body
        .as_ref()
        .map(|body| Uint8Array::from(body.as_ref()));
    if let Some(body) = &body_array {
        init.set_body(body.as_ref());
    }

    let web_request = Request::new_with_str_and_init(&request.url, &init)
        .map_err(|error| http_setup_error("request", error))?;
    for (name, value) in request.headers {
        web_request
            .headers()
            .set(&name, &value)
            .map_err(|error| http_setup_error("header", error))?;
    }

    let timed_out = Rc::new(Cell::new(false));
    let timeout_flag = Rc::clone(&timed_out);
    let timeout_controller = controller.clone();
    let timeout = gloo_timers::callback::Timeout::new(timeout_ms, move || {
        timeout_flag.set(true);
        timeout_controller.abort();
    });

    // Bind to globalThis.fetch so the adapter also works in a Worker.
    let global = js_sys::global();
    let fetch = Reflect::get(&global, &JsValue::from_str("fetch"))
        .map_err(|error| transport_error("fetch lookup", error))?
        .dyn_into::<Function>()
        .map_err(|error| transport_error("fetch binding", error))?;
    let promise = fetch
        .call1(&global, web_request.as_ref())
        .map_err(|error| transport_error("fetch call", error))?
        .dyn_into::<Promise>()
        .map_err(|error| transport_error("fetch promise", error))?;
    let response = JsFuture::from(promise).await;
    let response = response
        .map_err(|error| {
            if timed_out.get() {
                PortError::Transport(format!("timeout: browser fetch exceeded {timeout_ms} ms"))
            } else {
                transport_error("fetch rejected", error)
            }
        })?
        .dyn_into::<Response>()
        .map_err(|error| transport_error("fetch response", error))?;

    let mut headers = Vec::new();
    let iterator = js_sys::try_iter(response.headers().as_ref())
        .map_err(|error| transport_error("response headers", error))?
        .ok_or_else(|| PortError::Http("response headers are not iterable".into()))?;
    for entry in iterator {
        let entry = entry.map_err(|error| transport_error("response header", error))?;
        let pair = Array::from(&entry);
        let name = pair
            .get(0)
            .as_string()
            .ok_or_else(|| PortError::Http("response header name is not text".into()))?;
        let value = pair
            .get(1)
            .as_string()
            .ok_or_else(|| PortError::Http("response header value is not text".into()))?;
        headers.push((name, value));
    }

    let buffer = response
        .array_buffer()
        .map_err(|error| transport_error("response body", error))?;
    let buffer = JsFuture::from(buffer).await.map_err(|error| {
        if timed_out.get() {
            PortError::Transport(format!("timeout: browser fetch exceeded {timeout_ms} ms"))
        } else {
            transport_error("response body", error)
        }
    })?;
    let body = Bytes::from(Uint8Array::new(&buffer).to_vec());
    drop(timeout);

    Ok(HttpResponse {
        status: response.status(),
        headers,
        body,
    })
}

#[cfg(target_arch = "wasm32")]
fn http_setup_error(context: &str, error: wasm_bindgen::JsValue) -> PortError {
    PortError::Http(format!("browser {context}: {}", js_message(error)))
}

#[cfg(target_arch = "wasm32")]
fn transport_error(context: &str, error: wasm_bindgen::JsValue) -> PortError {
    PortError::Transport(format!("browser {context}: {}", js_message(error)))
}

#[cfg(target_arch = "wasm32")]
fn js_message(error: wasm_bindgen::JsValue) -> String {
    use js_sys::Reflect;
    use wasm_bindgen::JsValue;

    if let Some(message) = error.as_string() {
        return message;
    }
    let name = Reflect::get(&error, &JsValue::from_str("name"))
        .ok()
        .and_then(|value| value.as_string());
    let message = Reflect::get(&error, &JsValue::from_str("message"))
        .ok()
        .and_then(|value| value.as_string());
    match (name, message) {
        (Some(name), Some(message)) => format!("{name}: {message}"),
        (None, Some(message)) => message,
        (Some(name), None) => name,
        (None, None) => js_sys::JSON::stringify(&error)
            .ok()
            .and_then(|value| value.as_string())
            .filter(|value| value != "{}")
            .unwrap_or_else(|| "unknown JavaScript error".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_im::{http_outcome_event, EVENT_HTTP_UNAUTHORIZED, EVENT_NET_OFFLINE};

    fn event_sink() -> WebEventSink {
        WebEventSink::new(8, 8).expect("event sink")
    }

    fn request_with_stale_credential() -> HttpRequest {
        HttpRequest {
            method: "GET".into(),
            url: "https://example.invalid".into(),
            headers: vec![
                (AUTH_KIND_HEADER.into(), "session".into()),
                ("cookieId".into(), "stale".into()),
            ],
            body: None,
        }
    }

    #[test]
    fn browser_auth_uses_shared_intent_rewrite_and_can_rotate() {
        let auth = WebAuthRegistry::new();
        auth.set_session("cookieId", "current").expect("credential");
        let requester =
            BrowserHttpRequester::new(auth.clone(), event_sink(), 5_000, http_outcome_event)
                .expect("requester");

        let prepared = requester
            .prepare_request(request_with_stale_credential())
            .expect("prepare");
        assert!(!prepared
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(AUTH_KIND_HEADER)));
        assert!(prepared
            .headers
            .iter()
            .any(|(name, value)| name == "cookieId" && value == "current"));

        auth.clear(AuthKind::Session).expect("clear");
        let prepared = requester
            .prepare_request(HttpRequest {
                method: "GET".into(),
                url: "https://example.invalid".into(),
                headers: vec![(AUTH_KIND_HEADER.into(), "session".into())],
                body: None,
            })
            .expect("prepare cleared");
        assert!(!prepared
            .headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("cookieId")));
    }

    #[test]
    fn timeout_and_internal_header_credentials_are_rejected() {
        assert!(BrowserHttpRequester::new(
            WebAuthRegistry::new(),
            event_sink(),
            0,
            http_outcome_event,
        )
        .is_err());
        assert!(WebAuthRegistry::new()
            .set_session(AUTH_KIND_HEADER, "secret")
            .is_err());
    }

    #[test]
    fn unauthorized_and_offline_events_are_versioned_batches() {
        let events = event_sink();
        let requester = BrowserHttpRequester::new(
            WebAuthRegistry::new(),
            events.clone(),
            5_000,
            http_outcome_event,
        )
        .expect("requester");
        requester.observe_result(
            "https://example.invalid/unauthorized",
            &Ok(HttpResponse {
                status: 401,
                headers: Vec::new(),
                body: Bytes::new(),
            }),
        );
        requester.observe_result(
            "https://example.invalid/offline",
            &Err(PortError::Transport("network".into())),
        );
        let batches = events.take_batches();
        assert_eq!(batches.len(), 2);
        assert!(batches[0].events[0]
            .windows(EVENT_HTTP_UNAUTHORIZED.len())
            .any(|window| window == EVENT_HTTP_UNAUTHORIZED.as_bytes()));
        assert!(batches[1].events[0]
            .windows(EVENT_NET_OFFLINE.len())
            .any(|window| window == EVENT_NET_OFFLINE.as_bytes()));
    }

    #[test]
    fn shutdown_suppresses_late_infrastructure_events() {
        let events = event_sink();
        let requester = BrowserHttpRequester::new(
            WebAuthRegistry::new(),
            events.clone(),
            5_000,
            http_outcome_event,
        )
        .expect("requester");
        requester.shutdown();

        requester.observe_result(
            "https://example.invalid/late",
            &Ok(HttpResponse {
                status: 401,
                headers: Vec::new(),
                body: Bytes::new(),
            }),
        );
        requester.observe_result(
            "https://example.invalid/late",
            &Err(PortError::Transport("aborted".into())),
        );

        assert!(events.take_batches().is_empty());
        assert!(requester.lifecycle.ensure_active().is_err());
    }
}
