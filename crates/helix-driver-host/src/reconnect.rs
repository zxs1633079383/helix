use std::future::Future;
use std::time::Duration;

use helix_core::effect::TransportId;
use helix_core::PortError;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::engine::{TransportLifecycleEvent, TransportTraceEvent, TransportTraceSink};

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

/// 连接成功后在窗口内又被判定断开＝「连上即断」抖动：这一轮不重置退避，继续升级。
///
/// 线上复现（pre 客户端，单日 10 小时）：2158 次 `reconnect_success` 对 2113 次
/// `disconnect`，服务端日志证明从未主动关闭（`closeCode=0`，全是 60~100s 读超时收割）。
/// 若每次抖动都从 `attempt=1` 起步，退避就永远停在 `initial_delay`（200ms），
/// 即 60~70 次/分的连接风暴。
pub const RECONNECT_FLAP_WINDOW: Duration = Duration::from_secs(5);

#[derive(Clone, Default)]
pub struct ReconnectTraceSink {
    sink: Option<TransportTraceSink>,
}

impl ReconnectTraceSink {
    pub fn new(sink: Option<TransportTraceSink>) -> Self {
        Self { sink }
    }

    pub fn emit(&self, event: TransportTraceEvent) {
        if let Some(sink) = &self.sink {
            sink.try_emit(event);
        }
    }
}

/// 在断线事件后重试一个完整的“连接 → 注册 → 激活”动作。
///
/// supervisor 不再自行注册 sender；只有完整动作返回成功，才记录 reconnect success。
pub fn spawn_reconnect_supervisor<Reconnect, Fut>(
    transport_id: TransportId,
    policy: ReconnectPolicy,
    mut lifecycle_rx: mpsc::UnboundedReceiver<TransportLifecycleEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
    trace_sink: ReconnectTraceSink,
    mut reconnect: Reconnect,
) -> tokio::task::JoinHandle<()>
where
    Reconnect: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), PortError>> + Send + 'static,
{
    tokio::spawn(async move {
        // 上一次成功的时刻与档位：用于识别「连上即断」抖动，避免退避被抖动重置。
        let mut last_success: Option<(Instant, u32)> = None;
        loop {
            let event = tokio::select! {
                event = lifecycle_rx.recv() => {
                    let Some(event) = event else { return };
                    event
                }
                _ = wait_for_shutdown(&mut shutdown_rx) => return,
            };
            let TransportLifecycleEvent::Disconnected {
                transport_id: event_transport_id,
                reason,
            } = event;

            if event_transport_id != transport_id {
                continue;
            }

            let mut attempt = 1u32;
            if let Some((succeeded_at, succeeded_attempt)) = last_success {
                if succeeded_at.elapsed() < RECONNECT_FLAP_WINDOW {
                    // 连上即断：沿用上一轮档位继续升级，不回落到首档。
                    attempt = succeeded_attempt.saturating_add(1);
                } else {
                    // 连接稳定存活过窗口，这一轮算新故障，退避从首档重新开始。
                    last_success = None;
                }
            }
            loop {
                if *shutdown_rx.borrow() {
                    return;
                }
                if policy
                    .max_attempts
                    .is_some_and(|max_attempts| attempt > max_attempts)
                {
                    break;
                }

                let delay = policy.delay_for_attempt(attempt);
                emit_schedule(&trace_sink, transport_id, attempt, delay, reason);
                let delay_elapsed = tokio::time::sleep(delay);
                tokio::pin!(delay_elapsed);
                tokio::select! {
                    _ = &mut delay_elapsed => {}
                    _ = wait_for_shutdown(&mut shutdown_rx) => return,
                }

                emit_attempt(&trace_sink, transport_id, attempt, delay);
                let reconnect_attempt = reconnect();
                tokio::pin!(reconnect_attempt);
                let reconnect_result = tokio::select! {
                    result = &mut reconnect_attempt => result,
                    _ = wait_for_shutdown(&mut shutdown_rx) => return,
                };
                match reconnect_result {
                    Ok(()) => {
                        emit_success(&trace_sink, transport_id, attempt);
                        last_success = Some((Instant::now(), attempt));
                        break;
                    }
                    Err(error) => {
                        let next_attempt = attempt.saturating_add(1);
                        let next_delay = if policy
                            .max_attempts
                            .is_some_and(|max_attempts| next_attempt > max_attempts)
                        {
                            None
                        } else {
                            Some(policy.delay_for_attempt(next_attempt))
                        };
                        emit_failed(&trace_sink, transport_id, attempt, next_delay, &error);
                        attempt = next_attempt;
                    }
                }
            }
        }
    })
}

async fn wait_for_shutdown(shutdown_rx: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown_rx.borrow() {
            return;
        }
        if shutdown_rx.changed().await.is_err() {
            return;
        }
    }
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
        next_delay_ms: None,
        reason: Some(reason),
        error_class: None,
    });
}

fn emit_attempt(
    trace_sink: &ReconnectTraceSink,
    transport_id: TransportId,
    attempt: u32,
    delay: Duration,
) {
    trace_sink.emit(TransportTraceEvent {
        transport_id,
        name: "helix.ws.reconnect.attempt",
        action: "reconnect_attempt",
        attempt: Some(attempt),
        delay_ms: Some(duration_millis(delay)),
        next_delay_ms: None,
        reason: None,
        error_class: None,
    });
}

fn emit_failed(
    trace_sink: &ReconnectTraceSink,
    transport_id: TransportId,
    attempt: u32,
    next_delay: Option<Duration>,
    error: &PortError,
) {
    trace_sink.emit(TransportTraceEvent {
        transport_id,
        name: "helix.ws.reconnect.failed",
        action: "reconnect_failed",
        attempt: Some(attempt),
        delay_ms: None,
        next_delay_ms: next_delay.map(duration_millis),
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
        next_delay_ms: None,
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
        PortError::StorageConflict => "storage_conflict",
        PortError::Clock(_) => "clock",
        PortError::IdSource(_) => "id_source",
        PortError::Other(_) => "other",
    }
    .to_string()
}
