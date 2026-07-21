//! Web business API and browser bootstrap bridge.

use helix_core::ExecutionShell;
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::ImModule;
use serde::{Deserialize, Serialize};

use crate::session_store::SessionStoreLimits;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebLocalStoreMode {
    Session,
    Disabled,
}

impl From<WebLocalStoreMode> for LocalStoreMode {
    fn from(mode: WebLocalStoreMode) -> Self {
        match mode {
            WebLocalStoreMode::Session => Self::Session,
            WebLocalStoreMode::Disabled => Self::Disabled,
        }
    }
}

/// Platform bootstrap configuration. Business calls use `invoke`; these fields
/// are supplied once by the browser integration layer.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebConfig {
    pub ws_url: String,
    pub api_base_url: String,
    pub default_api_base_url: String,
    pub auth_user_id: String,
    pub company_id: String,
    pub user_name: String,
    pub org_name: String,
    pub dept_name: String,
    pub ping_interval_ms: u64,
    pub send_timeout_ms: u64,
    pub http_timeout_ms: u64,
    pub local_store_mode: WebLocalStoreMode,
    pub session_store: SessionStoreLimits,
    pub max_pending_actions: usize,
    pub max_pending_event_batches: usize,
    pub max_events_per_batch: usize,
    pub max_runtime_issues: usize,
    pub session_auth: Option<WebCredential>,
    pub bot_auth: Option<WebCredential>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebCredential {
    pub header_name: String,
    pub header_value: String,
}

impl WebConfig {
    pub fn im_config(&self) -> ImConfig {
        ImConfig {
            ws_url: self.ws_url.clone(),
            api_base_url: self.api_base_url.clone(),
            default_api_base_url: self.default_api_base_url.clone(),
            auth_user_id: self.auth_user_id.clone(),
            company_id: self.company_id.clone(),
            user_name: self.user_name.clone(),
            org_name: self.org_name.clone(),
            dept_name: self.dept_name.clone(),
            ping_interval_ms: self.ping_interval_ms,
            send_timeout_ms: self.send_timeout_ms,
        }
    }
}

pub fn build_web_shell(config: ImConfig, local_store_mode: LocalStoreMode) -> ExecutionShell {
    let mut shell = ExecutionShell::new();
    shell.register(ImModule::new_with_local_store(config, local_store_mode));
    shell
}

#[cfg(any(target_arch = "wasm32", test))]
#[derive(Debug)]
struct BoundedIssueQueue {
    entries: std::collections::VecDeque<String>,
    capacity: usize,
    dropped: u64,
}

#[cfg(any(target_arch = "wasm32", test))]
impl BoundedIssueQueue {
    fn new(capacity: usize) -> Result<Self, String> {
        if capacity == 0 {
            return Err("Web runtime issue capacity must be greater than zero".into());
        }
        Ok(Self {
            entries: std::collections::VecDeque::with_capacity(capacity.min(64)),
            capacity,
            dropped: 0,
        })
    }

    fn push(&mut self, issue: String) {
        if self.entries.len() < self.capacity {
            self.entries.push_back(issue);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    fn drain(&mut self) -> Vec<String> {
        let mut issues: Vec<String> = self.entries.drain(..).collect();
        if self.dropped > 0 {
            issues.push(format!(
                "{} additional Web runtime issues were dropped at capacity",
                self.dropped
            ));
            self.dropped = 0;
        }
        issues
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::{Rc, Weak};

    use bytes::Bytes;
    use helix_core::port_codec::http_result_to_outcome;
    use helix_core::ports::{Clock, EventSink, HttpRequester};
    use helix_core::tick::{PortError as CorePortError, PortOutcome};
    use helix_core::{AuthKind, Tick};
    use js_sys::{Array, Object, Reflect, Uint8Array};
    use wasm_bindgen::prelude::*;

    use crate::browser_http::{BrowserHttpRequester, WebAuthRegistry};
    use crate::event_sink::WebEventSink;
    use crate::executor::{WebExecutor, WebOwnedEffect};
    use crate::external_ws::{ExternalWsBridge, PlatformAction, WebSocketToken};
    use crate::session_store::WebSessionStore;
    use crate::timer::{BrowserClock, BrowserTimerRegistry, TimerToken};

    use super::{build_web_shell, BoundedIssueQueue, WebConfig};

    struct RuntimeState {
        executor: WebExecutor,
        store: WebSessionStore,
        http: BrowserHttpRequester,
        bridge: ExternalWsBridge,
        events: WebEventSink,
        timers: BrowserTimerRegistry,
        clock: BrowserClock,
        priority_ticks: VecDeque<Tick>,
        ticks: VecDeque<Tick>,
        driving: bool,
        stopped: bool,
        issues: BoundedIssueQueue,
    }

    type Runtime = Rc<RefCell<RuntimeState>>;

    /// `inventory` uses Wasm constructors to assemble the shared IM registries.
    /// Running them from the final platform artifact keeps this linker detail
    /// out of `helix-im` and guarantees registration before any client exists.
    #[wasm_bindgen(start)]
    pub fn initialize_wasm_runtime() {
        unsafe extern "C" {
            fn __wasm_call_ctors();
        }

        // SAFETY: wasm-ld synthesizes this function for linked constructors.
        // wasm-bindgen invokes a start function once per module instance, before
        // exported client APIs, so all inventory submissions are initialized.
        unsafe {
            __wasm_call_ctors();
        }
    }

    #[wasm_bindgen]
    pub struct HelixWebClient {
        runtime: Runtime,
    }

    #[wasm_bindgen]
    impl HelixWebClient {
        #[wasm_bindgen(constructor)]
        pub fn new(config_json: &str) -> Result<HelixWebClient, JsValue> {
            if helix_im::outbound::outbound_command_count() == 0 {
                return Err(js_error(
                    "shared IM registry is empty; Wasm constructors were not initialized",
                ));
            }
            let config: WebConfig = serde_json::from_str(config_json)
                .map_err(|error| js_error(format!("invalid Web config: {error}")))?;
            let limits = config
                .session_store
                .validate()
                .map_err(|error| js_error(error.to_string()))?;
            let bridge = ExternalWsBridge::new(config.max_pending_actions)
                .map_err(|error| js_error(error.to_string()))?;
            let clock = BrowserClock;
            let store = WebSessionStore::new(helix_im::IM_SPARSE_TABLE_SPECS, limits, || {
                js_sys::Date::now().max(0.0) as u64
            })
            .map_err(|error| js_error(error.to_string()))?;
            let events = WebEventSink::new(
                config.max_pending_event_batches,
                config.max_events_per_batch,
            )
            .map_err(js_error)?;
            let issues = BoundedIssueQueue::new(config.max_runtime_issues).map_err(js_error)?;
            let auth = WebAuthRegistry::new();
            if let Some(credential) = &config.session_auth {
                auth.set_session(&credential.header_name, &credential.header_value)
                    .map_err(|error| js_error(error.to_string()))?;
            }
            if let Some(credential) = &config.bot_auth {
                auth.set_bot(&credential.header_name, &credential.header_value)
                    .map_err(|error| js_error(error.to_string()))?;
            }
            let http = BrowserHttpRequester::new(
                auth,
                events.clone(),
                config.http_timeout_ms,
                helix_im::http_outcome_event,
            )
            .map_err(|error| js_error(error.to_string()))?;
            let shell = build_web_shell(config.im_config(), config.local_store_mode.into());
            let runtime = Rc::new(RefCell::new(RuntimeState {
                executor: WebExecutor::new(shell),
                store,
                http,
                bridge,
                events,
                timers: BrowserTimerRegistry::new(),
                clock,
                priority_ticks: VecDeque::new(),
                ticks: VecDeque::new(),
                driving: false,
                stopped: false,
                issues,
            }));

            let initial = runtime
                .borrow_mut()
                .executor
                .start()
                .map_err(|error| js_error(format!("engine start failed: {error}")))?;
            runtime.borrow_mut().driving = true;
            dispatch_effects(&runtime, initial);
            drive(&runtime);
            Ok(Self { runtime })
        }

        /// Invoke one shared Rust business command.
        pub fn invoke(&self, public_name: &str, payload: &[u8]) -> Result<(), JsValue> {
            ensure_running(&self.runtime)?;
            let command = helix_im::client_api::build_command(public_name, payload)
                .map_err(|error| js_error(error.to_string()))?;
            enqueue_tick(&self.runtime, Tick::Command(command), false);
            Ok(())
        }

        pub fn capabilities(&self) -> Vec<u8> {
            helix_im::client_api::capabilities_json()
        }

        /// Called by the browser socket `open` callback. The returned token must
        /// be passed to subsequent frame/close callbacks for this socket.
        pub fn ws_connected(&self) -> Result<u32, JsValue> {
            ensure_running(&self.runtime)?;
            let (token, ticks) = self
                .runtime
                .borrow()
                .bridge
                .ws_connected()
                .map_err(|error| js_error(error.to_string()))?;
            tracing::debug!(token = token.raw(), hop = "web.ws.connected", "Web ingress");
            if let Some(disconnected) = ticks.disconnected {
                enqueue_tick(&self.runtime, disconnected, false);
            }
            enqueue_tick(&self.runtime, ticks.connected, false);
            Ok(token.raw())
        }

        /// Returns false for a stale connection token; stale frames are ignored.
        pub fn ws_frame(&self, token: u32, frame: &[u8]) -> Result<bool, JsValue> {
            ensure_running(&self.runtime)?;
            let token = WebSocketToken::from_raw(token);
            let tick = self
                .runtime
                .borrow()
                .bridge
                .ws_frame(token, Bytes::copy_from_slice(frame))
                .map_err(|error| js_error(error.to_string()))?;
            let Some(tick) = tick else {
                return Ok(false);
            };
            tracing::debug!(token = token.raw(), hop = "web.ws.frame", "Web ingress");
            enqueue_tick(&self.runtime, tick, false);
            Ok(true)
        }

        /// Returns false for a duplicate/stale close callback.
        pub fn ws_disconnected(&self, token: u32) -> Result<bool, JsValue> {
            ensure_running(&self.runtime)?;
            let token = WebSocketToken::from_raw(token);
            let tick = self
                .runtime
                .borrow()
                .bridge
                .ws_disconnected(token)
                .map_err(|error| js_error(error.to_string()))?;
            let Some(tick) = tick else {
                return Ok(false);
            };
            tracing::debug!(
                token = token.raw(),
                hop = "web.ws.disconnected",
                "Web ingress"
            );
            enqueue_tick(&self.runtime, tick, false);
            Ok(true)
        }

        /// Drain infrastructure actions. Frontend business code never calls this;
        /// only the WebSocket bootstrap glue does.
        pub fn take_platform_actions(&self) -> Result<Array, JsValue> {
            let actions = self
                .runtime
                .borrow()
                .bridge
                .take_platform_actions()
                .map_err(|error| js_error(error.to_string()))?;
            let output = Array::new();
            for action in actions {
                match action {
                    PlatformAction::SendFrame {
                        token,
                        transport,
                        frame,
                    } => {
                        let object = Object::new();
                        set(&object, "kind", &JsValue::from_str("sendFrame"))?;
                        set(&object, "token", &JsValue::from_f64(token.raw() as f64))?;
                        set(
                            &object,
                            "transportId",
                            &JsValue::from_f64(transport.raw() as f64),
                        )?;
                        let bytes = Uint8Array::from(frame.as_ref());
                        set(&object, "frame", bytes.as_ref())?;
                        output.push(&object);
                    }
                    PlatformAction::CloseSocket { token } => {
                        let object = Object::new();
                        set(&object, "kind", &JsValue::from_str("closeSocket"))?;
                        set(&object, "token", &JsValue::from_f64(token.raw() as f64))?;
                        output.push(&object);
                    }
                }
            }
            Ok(output)
        }

        /// Drain one object per shell step; each object has a version and an
        /// ordered array of raw DomainEvent byte arrays.
        pub fn take_event_batches(&self) -> Result<Array, JsValue> {
            let batches = self.runtime.borrow().events.take_batches();
            let output = Array::new();
            for batch in batches {
                let object = Object::new();
                set(&object, "version", &JsValue::from_f64(batch.version as f64))?;
                set(
                    &object,
                    "droppedBatchesBefore",
                    &JsValue::from_f64(batch.dropped_batches_before as f64),
                )?;
                set(
                    &object,
                    "droppedEvents",
                    &JsValue::from_f64(batch.dropped_events as f64),
                )?;
                let events = Array::new();
                for event in batch.events {
                    events.push(&Uint8Array::from(event.as_ref()));
                }
                set(&object, "events", events.as_ref())?;
                output.push(&object);
            }
            Ok(output)
        }

        pub fn take_runtime_issues(&self) -> Array {
            let mut runtime = self.runtime.borrow_mut();
            let output = Array::new();
            for issue in runtime.issues.drain() {
                output.push(&JsValue::from_str(&issue));
            }
            output
        }

        pub fn set_session_auth(
            &self,
            header_name: &str,
            header_value: &str,
        ) -> Result<(), JsValue> {
            ensure_running(&self.runtime)?;
            self.runtime
                .borrow()
                .http
                .auth_registry()
                .set_session(header_name, header_value)
                .map_err(|error| js_error(error.to_string()))
        }

        pub fn set_bot_auth(&self, header_name: &str, header_value: &str) -> Result<(), JsValue> {
            ensure_running(&self.runtime)?;
            self.runtime
                .borrow()
                .http
                .auth_registry()
                .set_bot(header_name, header_value)
                .map_err(|error| js_error(error.to_string()))
        }

        pub fn clear_session_auth(&self) -> Result<(), JsValue> {
            ensure_running(&self.runtime)?;
            self.runtime
                .borrow()
                .http
                .auth_registry()
                .clear(AuthKind::Session)
                .map_err(|error| js_error(error.to_string()))
        }

        pub fn clear_bot_auth(&self) -> Result<(), JsValue> {
            ensure_running(&self.runtime)?;
            self.runtime
                .borrow()
                .http
                .auth_registry()
                .clear(AuthKind::Bot)
                .map_err(|error| js_error(error.to_string()))
        }

        pub fn shutdown(&self) -> Result<(), JsValue> {
            if self.runtime.borrow().stopped {
                return Ok(());
            }
            {
                let mut runtime = self.runtime.borrow_mut();
                // Flip the externally visible state before any future can be
                // polled again. This also prevents queued spawn_local tasks
                // from starting a Fetch after shutdown began.
                runtime.stopped = true;
                runtime.http.shutdown();
                runtime.timers.clear();
            }
            let bridge_error = self
                .runtime
                .borrow()
                .bridge
                .shutdown()
                .err()
                .map(|error| error.to_string());
            let final_effects = {
                let mut runtime = self.runtime.borrow_mut();
                match runtime.executor.stop() {
                    Ok(effects) => effects,
                    Err(error) => {
                        runtime.issues.push(format!("engine stop failed: {error}"));
                        Vec::new()
                    }
                }
            };
            dispatch_effects(&self.runtime, final_effects);
            let mut runtime = self.runtime.borrow_mut();
            runtime.priority_ticks.clear();
            runtime.ticks.clear();
            drop(runtime);
            match bridge_error {
                Some(error) => Err(js_error(error)),
                None => Ok(()),
            }
        }
    }

    fn ensure_running(runtime: &Runtime) -> Result<(), JsValue> {
        if runtime.borrow().stopped {
            return Err(js_error("Helix Web client is shut down"));
        }
        Ok(())
    }

    fn enqueue_tick(runtime: &Runtime, tick: Tick, priority: bool) {
        let should_drive = {
            let mut state = runtime.borrow_mut();
            if state.stopped {
                return;
            }
            if priority {
                state.priority_ticks.push_back(tick);
            } else {
                state.ticks.push_back(tick);
            }
            if state.driving {
                false
            } else {
                state.driving = true;
                true
            }
        };
        if should_drive {
            drive(runtime);
        }
    }

    fn drive(runtime: &Runtime) {
        loop {
            let next = {
                let mut state = runtime.borrow_mut();
                if state.stopped {
                    state.driving = false;
                    return;
                }
                match state
                    .priority_ticks
                    .pop_front()
                    .or_else(|| state.ticks.pop_front())
                {
                    Some(tick) => {
                        let now_ms = state.clock.now_ms();
                        Some((tick, now_ms))
                    }
                    None => {
                        state.driving = false;
                        None
                    }
                }
            };
            let Some((tick, now_ms)) = next else {
                return;
            };
            tracing::debug!(now_ms, hop = "web.tick.step", "Web core step");
            let step_result = { runtime.borrow_mut().executor.step(tick, now_ms) };
            let effects = match step_result {
                Ok(effects) => effects,
                Err(error) => {
                    record_issue(runtime, format!("engine step failed: {error}"));
                    continue;
                }
            };
            dispatch_effects(runtime, effects);
        }
    }

    fn dispatch_effects(runtime: &Runtime, effects: Vec<WebOwnedEffect>) {
        let mut fire_ops = Vec::new();
        for effect in effects {
            match effect {
                WebOwnedEffect::Persist { corr, ops } => {
                    tracing::debug!(
                        corr = corr.raw(),
                        hop = "web.effect.persist",
                        "Web dispatch"
                    );
                    let outcome = runtime.borrow().store.execute_ops(&ops);
                    enqueue_tick(runtime, Tick::PortReply { corr, outcome }, true);
                }
                WebOwnedEffect::PersistAtomic { corr, ops } => {
                    tracing::debug!(
                        corr = corr.raw(),
                        hop = "web.effect.persist_atomic",
                        "Web dispatch"
                    );
                    let outcome = runtime.borrow().store.execute_atomic_write(&ops);
                    enqueue_tick(runtime, Tick::PortReply { corr, outcome }, true);
                }
                WebOwnedEffect::PersistFire { ops } => {
                    fire_ops.extend(ops);
                }
                WebOwnedEffect::Http { corr, req } => {
                    tracing::debug!(corr = corr.raw(), hop = "web.effect.http", "Web dispatch");
                    spawn_http(Rc::downgrade(runtime), Some(corr), req);
                }
                WebOwnedEffect::HttpFire { req } => {
                    spawn_http(Rc::downgrade(runtime), None, req);
                }
                WebOwnedEffect::UploadFile { corr, req } => {
                    record_issue(
                        runtime,
                        format!(
                            "browser local-path upload is unsupported: {}",
                            req.local_path
                        ),
                    );
                    enqueue_tick(
                        runtime,
                        Tick::PortReply {
                            corr,
                            outcome: PortOutcome::Err(CorePortError::Other(1)),
                        },
                        true,
                    );
                }
                WebOwnedEffect::Send { transport, frame } => {
                    tracing::debug!(
                        transport_id = transport.raw(),
                        hop = "web.effect.send",
                        "Web egress"
                    );
                    let result = runtime.borrow().bridge.enqueue_send(transport, frame);
                    if let Err(error) = result {
                        record_issue(runtime, format!("WebSocket send action failed: {error}"));
                    }
                }
                WebOwnedEffect::Request {
                    corr,
                    kind,
                    payload,
                } => {
                    record_issue(
                        runtime,
                        format!(
                            "host request unsupported on Web: kind={kind}, payload_len={}",
                            payload.len()
                        ),
                    );
                    enqueue_tick(
                        runtime,
                        Tick::PortReply {
                            corr,
                            outcome: PortOutcome::Err(CorePortError::Other(2)),
                        },
                        true,
                    );
                }
                WebOwnedEffect::Emit { event } => {
                    runtime.borrow().events.emit(event);
                }
                WebOwnedEffect::ScheduleTimer { id, after_ms } => {
                    let weak = Rc::downgrade(runtime);
                    runtime
                        .borrow_mut()
                        .timers
                        .schedule(id, after_ms, move |token| timer_elapsed(&weak, token));
                }
                WebOwnedEffect::CancelTimer { id } => {
                    runtime.borrow_mut().timers.cancel(id);
                }
            }
        }
        if !fire_ops.is_empty() {
            let outcome = runtime.borrow().store.execute_ops_atomic(&fire_ops);
            if let PortOutcome::Err(error) = outcome {
                record_issue(
                    runtime,
                    format!("PersistFire step transaction failed: {error:?}"),
                );
            }
        }
        runtime.borrow().events.flush_step();
    }

    fn spawn_http(
        runtime: Weak<RefCell<RuntimeState>>,
        corr: Option<helix_core::effect::Correlation>,
        request: helix_core::effect::HttpRequest,
    ) {
        wasm_bindgen_futures::spawn_local(async move {
            let Some(strong) = runtime.upgrade() else {
                return;
            };
            let http = {
                let state = strong.borrow();
                if state.stopped {
                    return;
                }
                state.http.clone()
            };
            drop(strong);
            let outcome = http_result_to_outcome(http.request(request).await);
            let Some(strong) = runtime.upgrade() else {
                return;
            };
            if strong.borrow().stopped {
                return;
            }
            match corr {
                Some(corr) => enqueue_tick(&strong, Tick::PortReply { corr, outcome }, true),
                None => {
                    if let PortOutcome::Err(error) = outcome {
                        record_issue(&strong, format!("HttpFire failed: {error:?}"));
                    }
                }
            }
        });
    }

    fn timer_elapsed(runtime: &Weak<RefCell<RuntimeState>>, token: TimerToken) {
        let Some(runtime) = runtime.upgrade() else {
            return;
        };
        let fired = runtime.borrow_mut().timers.mark_fired(token);
        if let Some(id) = fired {
            enqueue_tick(&runtime, Tick::Timer(id), false);
        }
    }

    fn record_issue(runtime: &Runtime, issue: String) {
        tracing::warn!(issue = %issue, hop = "web.runtime.issue", "Web runtime issue");
        runtime.borrow_mut().issues.push(issue);
    }

    fn set(object: &Object, key: &str, value: &JsValue) -> Result<(), JsValue> {
        Reflect::set(object, &JsValue::from_str(key), value).map(|_| ())
    }

    fn js_error(message: impl AsRef<str>) -> JsValue {
        JsValue::from_str(message.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::BoundedIssueQueue;

    #[test]
    fn runtime_issue_queue_is_bounded_and_reports_overflow() {
        let mut issues = BoundedIssueQueue::new(2).expect("issue queue");
        issues.push("one".into());
        issues.push("two".into());
        issues.push("three".into());
        issues.push("four".into());

        let drained = issues.drain();
        assert_eq!(drained.len(), 3);
        assert_eq!(drained[0], "one");
        assert_eq!(drained[1], "two");
        assert!(drained[2].contains("2 additional"));
        assert!(issues.drain().is_empty());
    }
}

#[cfg(target_arch = "wasm32")]
pub use wasm::HelixWebClient;
