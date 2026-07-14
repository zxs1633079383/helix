//! Browser platform adapter for the shared Helix business core.
//!
//! This crate owns browser Fetch, a bounded page-session store, browser timers
//! and the bridge to a JavaScript-owned physical WebSocket. It intentionally
//! contains no frontend rendering and no IM frame/cursor/sync implementation.

pub mod api;
pub mod browser_http;
pub mod event_sink;
pub mod executor;
pub mod external_ws;
pub mod session_store;
pub mod timer;

pub use browser_http::{BrowserHttpRequester, WebAuthRegistry};
pub use event_sink::{EventBatch, WebEventSink, EVENT_BATCH_VERSION};
pub use executor::{WebExecutor, WebOwnedEffect};
pub use external_ws::{
    BridgeError, ConnectionTicks, ExternalWsBridge, PlatformAction, WebSocketToken,
};
pub use session_store::{SessionStoreLimits, WebSessionStore};
