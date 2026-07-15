//! Bridge between the Rust business runtime and a browser-owned WebSocket.
//!
//! JavaScript owns physical connect/reconnect/close. Rust owns the shared IM
//! protocol state machine and emits only outbound frame actions. A connection
//! token prevents an old socket's late frame/close callback from mutating the
//! current connection generation.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use helix_core::effect::TransportId;
use helix_core::ports::FrameSender;
use helix_core::tick::InboundBytes;
use helix_core::{PortError, Tick};
use thiserror::Error;

pub const WEB_TRANSPORT_RAW: u32 = 0;

pub fn web_transport_id() -> TransportId {
    TransportId::from_raw(WEB_TRANSPORT_RAW)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WebSocketToken(u32);

impl WebSocketToken {
    pub fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone)]
pub enum PlatformAction {
    SendFrame {
        token: WebSocketToken,
        transport: TransportId,
        frame: Bytes,
    },
    CloseSocket {
        token: WebSocketToken,
    },
}

#[derive(Debug)]
pub struct ConnectionTicks {
    pub disconnected: Option<Tick>,
    pub connected: Tick,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BridgeError {
    #[error("external WebSocket bridge capacity must be greater than zero")]
    InvalidCapacity,
    #[error("external WebSocket bridge is shut down")]
    Closed,
    #[error("external WebSocket control action queue is full")]
    ControlQueueFull,
    #[error("external WebSocket bridge lock poisoned")]
    Poisoned,
}

#[derive(Debug)]
struct BridgeState {
    next_token: u32,
    active: Option<WebSocketToken>,
    closed: bool,
    control_actions: VecDeque<PlatformAction>,
    actions: VecDeque<PlatformAction>,
}

#[derive(Debug, Clone)]
pub struct ExternalWsBridge {
    state: Arc<Mutex<BridgeState>>,
    max_pending_actions: usize,
}

impl ExternalWsBridge {
    pub fn new(max_pending_actions: usize) -> Result<Self, BridgeError> {
        if max_pending_actions == 0 {
            return Err(BridgeError::InvalidCapacity);
        }
        Ok(Self {
            state: Arc::new(Mutex::new(BridgeState {
                next_token: 1,
                active: None,
                closed: false,
                control_actions: VecDeque::new(),
                actions: VecDeque::with_capacity(max_pending_actions.min(64)),
            })),
            max_pending_actions,
        })
    }

    /// Mark a newly opened physical socket active before publishing Connected.
    pub fn ws_connected(&self) -> Result<(WebSocketToken, ConnectionTicks), BridgeError> {
        let mut state = self.state.lock().map_err(|_| BridgeError::Poisoned)?;
        if state.closed {
            return Err(BridgeError::Closed);
        }
        let replaced = state.active;
        // One control slot remains reserved so shutdown can always publish the
        // active socket's close without discarding an older close request.
        if replaced.is_some()
            && state.control_actions.len() >= self.max_pending_actions.saturating_sub(1)
        {
            return Err(BridgeError::ControlQueueFull);
        }
        let token = WebSocketToken(state.next_token);
        state.next_token = state.next_token.wrapping_add(1).max(1);
        state.active = Some(token);
        // Frames queued for a previous physical socket must never be replayed
        // onto the new generation.
        state.actions.clear();
        if let Some(old) = replaced {
            state
                .control_actions
                .push_back(PlatformAction::CloseSocket { token: old });
        }
        Ok((
            token,
            ConnectionTicks {
                disconnected: replaced.map(|_| Tick::Disconnected(web_transport_id())),
                connected: Tick::Connected(web_transport_id()),
            },
        ))
    }

    pub fn ws_frame(
        &self,
        token: WebSocketToken,
        frame: Bytes,
    ) -> Result<Option<Tick>, BridgeError> {
        let state = self.state.lock().map_err(|_| BridgeError::Poisoned)?;
        Ok((state.active == Some(token)).then(|| Tick::Inbound(InboundBytes(frame))))
    }

    pub fn ws_disconnected(&self, token: WebSocketToken) -> Result<Option<Tick>, BridgeError> {
        let mut state = self.state.lock().map_err(|_| BridgeError::Poisoned)?;
        if state.active != Some(token) {
            return Ok(None);
        }
        state.active = None;
        state.actions.retain(|action| match action {
            PlatformAction::SendFrame { token: queued, .. } => *queued != token,
            PlatformAction::CloseSocket { token: queued } => *queued != token,
        });
        state.control_actions.retain(|action| match action {
            PlatformAction::CloseSocket { token: queued } => *queued != token,
            PlatformAction::SendFrame { .. } => true,
        });
        Ok(Some(Tick::Disconnected(web_transport_id())))
    }

    pub fn active_token(&self) -> Result<Option<WebSocketToken>, BridgeError> {
        self.state
            .lock()
            .map(|state| state.active)
            .map_err(|_| BridgeError::Poisoned)
    }

    pub fn take_platform_actions(&self) -> Result<Vec<PlatformAction>, BridgeError> {
        let mut state = self.state.lock().map_err(|_| BridgeError::Poisoned)?;
        let mut actions = Vec::with_capacity(
            state
                .control_actions
                .len()
                .saturating_add(state.actions.len()),
        );
        actions.extend(state.control_actions.drain(..));
        actions.extend(state.actions.drain(..));
        Ok(actions)
    }

    /// Stop accepting sends and ask the browser bootstrap to close the active
    /// physical socket. Pending frames are discarded before the close action.
    pub fn shutdown(&self) -> Result<(), BridgeError> {
        let mut state = self.state.lock().map_err(|_| BridgeError::Poisoned)?;
        if state.closed {
            return Ok(());
        }
        if state.active.is_some() && state.control_actions.len() >= self.max_pending_actions {
            return Err(BridgeError::ControlQueueFull);
        }
        state.closed = true;
        let active = state.active.take();
        state.actions.clear();
        if let Some(token) = active {
            state
                .control_actions
                .push_back(PlatformAction::CloseSocket { token });
        }
        Ok(())
    }

    pub fn enqueue_send(&self, transport: TransportId, frame: Bytes) -> Result<(), PortError> {
        if transport != web_transport_id() {
            return Err(PortError::Transport(format!(
                "unregistered Web transport {}",
                transport.raw()
            )));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| PortError::Transport("external WebSocket bridge poisoned".into()))?;
        if state.closed {
            return Err(PortError::Transport(
                "external WebSocket bridge is shut down".into(),
            ));
        }
        let token = state
            .active
            .ok_or_else(|| PortError::Transport("external WebSocket is not connected".into()))?;
        if state.actions.len() >= self.max_pending_actions {
            return Err(PortError::Transport(
                "external WebSocket action queue is full".into(),
            ));
        }
        state.actions.push_back(PlatformAction::SendFrame {
            token,
            transport,
            frame,
        });
        Ok(())
    }
}

#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
impl FrameSender for ExternalWsBridge {
    async fn send(&self, frame: Bytes) -> Result<(), PortError> {
        self.enqueue_send(web_transport_id(), frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_socket_callbacks_cannot_disconnect_new_generation() {
        let bridge = ExternalWsBridge::new(4).expect("bridge");
        let (old, first) = bridge.ws_connected().expect("old connect");
        assert!(first.disconnected.is_none());
        assert!(matches!(first.connected, Tick::Connected(_)));
        let (current, replacement) = bridge.ws_connected().expect("new connect");
        assert!(matches!(
            replacement.disconnected,
            Some(Tick::Disconnected(_))
        ));
        assert!(matches!(replacement.connected, Tick::Connected(_)));
        assert!(matches!(
            bridge.take_platform_actions().expect("replacement actions").as_slice(),
            [PlatformAction::CloseSocket { token }] if *token == old
        ));

        assert!(bridge
            .ws_frame(old, Bytes::from_static(b"late"))
            .expect("stale frame")
            .is_none());
        assert!(bridge.ws_disconnected(old).expect("stale close").is_none());
        assert_eq!(bridge.active_token().expect("active"), Some(current));
    }

    #[test]
    fn send_is_bounded_and_connection_scoped() {
        let bridge = ExternalWsBridge::new(1).expect("bridge");
        assert!(bridge
            .enqueue_send(web_transport_id(), Bytes::from_static(b"before"))
            .is_err());
        let (token, _) = bridge.ws_connected().expect("connect");
        bridge
            .enqueue_send(web_transport_id(), Bytes::from_static(b"one"))
            .expect("first frame");
        assert!(bridge
            .enqueue_send(web_transport_id(), Bytes::from_static(b"two"))
            .is_err());
        let actions = bridge.take_platform_actions().expect("actions");
        assert!(matches!(
            &actions[0],
            PlatformAction::SendFrame { token: actual, frame, .. }
                if *actual == token && frame.as_ref() == b"one"
        ));
    }

    #[test]
    fn close_is_exactly_once() {
        let bridge = ExternalWsBridge::new(2).expect("bridge");
        let (token, _) = bridge.ws_connected().expect("connect");
        assert!(bridge
            .ws_disconnected(token)
            .expect("first close")
            .is_some());
        assert!(bridge
            .ws_disconnected(token)
            .expect("duplicate close")
            .is_none());
    }

    #[test]
    fn shutdown_discards_frames_and_requests_one_physical_close() {
        let bridge = ExternalWsBridge::new(2).expect("bridge");
        let (token, _) = bridge.ws_connected().expect("connect");
        bridge
            .enqueue_send(web_transport_id(), Bytes::from_static(b"pending"))
            .expect("send");
        bridge.shutdown().expect("shutdown");
        let actions = bridge.take_platform_actions().expect("actions");
        assert!(matches!(
            actions.as_slice(),
            [PlatformAction::CloseSocket { token: actual }] if *actual == token
        ));
        assert!(bridge
            .enqueue_send(web_transport_id(), Bytes::from_static(b"late"))
            .is_err());
        assert!(matches!(bridge.ws_connected(), Err(BridgeError::Closed)));
        bridge.shutdown().expect("idempotent shutdown");
        assert!(bridge.take_platform_actions().expect("actions").is_empty());
    }

    #[test]
    fn replacement_fails_without_mutating_state_when_close_queue_is_saturated() {
        let bridge = ExternalWsBridge::new(2).expect("bridge");
        let (first, _) = bridge.ws_connected().expect("first connect");
        let (current, _) = bridge.ws_connected().expect("replacement");
        bridge
            .enqueue_send(web_transport_id(), Bytes::from_static(b"current"))
            .expect("current frame");

        assert!(matches!(
            bridge.ws_connected(),
            Err(BridgeError::ControlQueueFull)
        ));
        assert_eq!(bridge.active_token().expect("active"), Some(current));
        assert!(matches!(
            bridge.take_platform_actions().expect("actions").as_slice(),
            [
                PlatformAction::CloseSocket { token: closed },
                PlatformAction::SendFrame {
                    token: sent,
                    frame,
                    ..
                }
            ] if *closed == first && *sent == current && frame.as_ref() == b"current"
        ));

        let (replacement, _) = bridge.ws_connected().expect("retry replacement");
        assert_eq!(replacement.raw(), current.raw() + 1);
    }

    #[test]
    fn capacity_one_reserves_its_control_slot_for_shutdown() {
        let bridge = ExternalWsBridge::new(1).expect("bridge");
        let (token, _) = bridge.ws_connected().expect("connect");

        assert!(matches!(
            bridge.ws_connected(),
            Err(BridgeError::ControlQueueFull)
        ));
        assert_eq!(bridge.active_token().expect("active"), Some(token));

        bridge.shutdown().expect("shutdown");
        bridge.shutdown().expect("idempotent shutdown");
        assert!(matches!(
            bridge.take_platform_actions().expect("actions").as_slice(),
            [PlatformAction::CloseSocket { token: actual }] if *actual == token
        ));
    }
}
