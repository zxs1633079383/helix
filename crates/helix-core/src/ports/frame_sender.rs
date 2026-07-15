//! Outbound WebSocket frame port.
//!
//! Core only asks a platform to send an already-built frame. Connection setup,
//! inbound reads, close and reconnect are platform lifecycle concerns; drivers
//! feed their outcomes back through `Tick::{Connected, Inbound, Disconnected}`.

use bytes::Bytes;

use crate::error::PortError;
use crate::platform::{MaybeSend, MaybeSync};

/// Send-only boundary consumed by the sans-I/O execution host.
///
/// The trait deliberately has no connect/receive/close methods. This keeps
/// browser-owned WebSocket lifecycles and native managed connections outside
/// core while all platforms continue to share the same inbound Tick semantics.
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
pub trait FrameSender: MaybeSend + MaybeSync + 'static {
    /// Send one frame; fragmentation, masking and the physical socket belong to
    /// the implementing platform adapter.
    async fn send(&self, frame: Bytes) -> Result<(), PortError>;
}
