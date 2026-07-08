use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use helix_core::effect::TransportId;
use helix_core::ports::Transport;
use helix_core::PortError;
use tokio::sync::mpsc;

use crate::engine::{TransportLifecycleEvent, TransportTraceEvent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: u32,
    pub max_attempts: Option<u32>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(10),
            multiplier: 2,
            max_attempts: None,
        }
    }
}

impl ReconnectPolicy {
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let multiplier = self.multiplier.max(1);
        let mut delay = self.initial_delay.min(self.max_delay);

        for _ in 1..attempt {
            delay = delay
                .checked_mul(multiplier)
                .unwrap_or(self.max_delay)
                .min(self.max_delay);
        }

        delay
    }
}

#[derive(Clone, Default)]
pub struct ReconnectTraceSink {
    tx: Option<mpsc::UnboundedSender<TransportTraceEvent>>,
}

impl ReconnectTraceSink {
    pub fn new(tx: Option<mpsc::UnboundedSender<TransportTraceEvent>>) -> Self {
        Self { tx }
    }

    pub fn emit(&self, event: TransportTraceEvent) {
        if let Some(tx) = &self.tx {
            tx.send(event).ok();
        }
    }
}

pub fn spawn_reconnect_supervisor<Tr, Make, Fut>(
    transport_id: TransportId,
    policy: ReconnectPolicy,
    mut lifecycle_rx: mpsc::UnboundedReceiver<TransportLifecycleEvent>,
    register_tx: mpsc::UnboundedSender<(TransportId, Arc<Tr>)>,
    trace_sink: ReconnectTraceSink,
    mut make_transport: Make,
) -> tokio::task::JoinHandle<()>
where
    Tr: Transport + Send + Sync + 'static,
    Make: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = Result<Tr, PortError>> + Send + 'static,
{
    tokio::spawn(async move {
        while let Some(event) = lifecycle_rx.recv().await {
            let TransportLifecycleEvent::Disconnected {
                transport_id: event_transport_id,
                reason,
            } = event;

            if event_transport_id != transport_id {
                continue;
            }

            let mut attempt = 1u32;
            loop {
                if policy
                    .max_attempts
                    .is_some_and(|max_attempts| attempt > max_attempts)
                {
                    break;
                }

                let delay = policy.delay_for_attempt(attempt);
                emit_schedule(&trace_sink, transport_id, attempt, delay, reason);
                tokio::time::sleep(delay).await;

                emit_attempt(&trace_sink, transport_id, attempt);
                match make_transport().await {
                    Ok(transport) => {
                        if register_tx
                            .send((transport_id, Arc::new(transport)))
                            .is_ok()
                        {
                            emit_success(&trace_sink, transport_id, attempt);
                        }
                        break;
                    }
                    Err(error) => {
                        emit_failed(&trace_sink, transport_id, attempt, &error);
                        attempt = attempt.saturating_add(1);
                    }
                }
            }
        }
    })
}

fn emit_schedule(
    trace_sink: &ReconnectTraceSink,
    transport_id: TransportId,
    attempt: u32,
    delay: Duration,
    reason: &'static str,
) {
    trace_sink.emit(TransportTraceEvent {
        transport_id,
        name: "helix.ws.reconnect.schedule",
        action: "reconnect_schedule",
        attempt: Some(attempt),
        delay_ms: Some(duration_millis(delay)),
        reason: Some(reason),
        error_class: None,
    });
}

fn emit_attempt(trace_sink: &ReconnectTraceSink, transport_id: TransportId, attempt: u32) {
    trace_sink.emit(TransportTraceEvent {
        transport_id,
        name: "helix.ws.reconnect.attempt",
        action: "reconnect_attempt",
        attempt: Some(attempt),
        delay_ms: None,
        reason: None,
        error_class: None,
    });
}

fn emit_failed(
    trace_sink: &ReconnectTraceSink,
    transport_id: TransportId,
    attempt: u32,
    error: &PortError,
) {
    trace_sink.emit(TransportTraceEvent {
        transport_id,
        name: "helix.ws.reconnect.failed",
        action: "reconnect_failed",
        attempt: Some(attempt),
        delay_ms: None,
        reason: None,
        error_class: Some(classify_port_error(error)),
    });
}

fn emit_success(trace_sink: &ReconnectTraceSink, transport_id: TransportId, attempt: u32) {
    trace_sink.emit(TransportTraceEvent {
        transport_id,
        name: "helix.ws.reconnect.success",
        action: "reconnect_success",
        attempt: Some(attempt),
        delay_ms: None,
        reason: None,
        error_class: None,
    });
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn classify_port_error(error: &PortError) -> String {
    match error {
        PortError::Transport(_) => "transport",
        PortError::Http(_) => "http",
        PortError::Storage(_) => "storage",
        PortError::Clock(_) => "clock",
        PortError::IdSource(_) => "id_source",
        PortError::Other(_) => "other",
    }
    .to_string()
}
