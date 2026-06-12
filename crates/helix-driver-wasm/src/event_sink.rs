//! WasmEventSink — wasm-bindgen JS callback

use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;

pub struct WasmEventSink {
    // TODO: callback: js_sys::Function,  // JS callback(eventBytes: Uint8Array)
}

impl EventSink for WasmEventSink {
    fn emit(&self, event: DomainEventBytes) {
        // TODO: 把 event.0 转为 Uint8Array 并调用 JS callback
        let _ = event;
    }
}
