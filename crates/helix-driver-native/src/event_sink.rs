//! NativeEventSink — tokio broadcast channel
//!
//! helix-tauri 的 event_bus.rs 订阅此 channel，
//! 把 DomainEventBytes 翻译为 `im:__bus__` Tauri 事件推给前端。

use helix_core::ports::EventSink;
use helix_core::effect::DomainEventBytes;

pub struct NativeEventSink {
    // TODO: sender: tokio::sync::broadcast::Sender<DomainEventBytes>,
}

impl EventSink for NativeEventSink {
    fn emit(&self, event: DomainEventBytes) {
        // TODO: self.sender.send(event).ok(); (lagged 接收方自行处理)
        let _ = event;
        tracing::debug!("NativeEventSink::emit (TODO: connect to tauri broadcast)");
    }
}
