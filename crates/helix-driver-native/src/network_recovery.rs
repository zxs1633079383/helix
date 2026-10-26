//! Opt-in native connection recovery. Never owns or replays application commands.

use helix_core::{effect::TransportId, PortError};
use helix_driver_host::{engine::TransportLifecycleEvent, ReconnectPolicy, ReconnectTraceSink};
use std::{future::Future, time::Duration};
use tokio::sync::{mpsc, watch};

/// Local link availability is a hint; unknown still permits a bounded handshake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkAvailability {
    Unknown,
    Offline,
    Online,
}

/// Three fast retries followed by indefinite, capped background recovery.
#[derive(Clone, Copy, Debug)]
pub struct NetworkRecoveryPolicy {
    pub fast: ReconnectPolicy,
    pub background: ReconnectPolicy,
}

impl Default for NetworkRecoveryPolicy {
    /// Defaults apply only to transports explicitly opting into network recovery.
    fn default() -> Self {
        Self {
            fast: ReconnectPolicy {
                initial_delay: Duration::from_secs(1),
                max_delay: Duration::from_secs(4),
                multiplier: 2,
                max_attempts: Some(3),
            },
            background: ReconnectPolicy {
                initial_delay: Duration::from_secs(15),
                max_delay: Duration::from_secs(60),
                multiplier: 2,
                max_attempts: None,
            },
        }
    }
}

impl NetworkRecoveryPolicy {
    /// Failure zero is the immediate initial attempt; subsequent failures select a phase.
    pub fn delay_after_failures(&self, failures: u32) -> Duration {
        if failures == 0 {
            return Duration::ZERO;
        }
        let fast_count = self.fast.max_attempts.unwrap_or(3);
        if failures <= fast_count {
            self.fast.delay_for_attempt(failures.min(128))
        } else {
            self.background
                .delay_for_attempt(failures.saturating_sub(fast_count).min(128))
        }
    }
}

#[derive(Clone)]
pub(crate) struct NetworkRecovery {
    pub availability: watch::Receiver<NetworkAvailability>,
    pub policy: NetworkRecoveryPolicy,
}

/// Owns one cancellable connect/register/activate attempt at a time, including cold startup.
pub(crate) fn spawn_network_recovery<F, Fut, Close, CloseFut>(
    id: TransportId,
    recovery: NetworkRecovery,
    initial: bool,
    mut lifecycle: mpsc::UnboundedReceiver<TransportLifecycleEvent>,
    mut shutdown: watch::Receiver<bool>,
    trace: ReconnectTraceSink,
    mut connect: F,
    mut disconnect: Close,
) -> tokio::task::JoinHandle<()>
where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), PortError>> + Send + 'static,
    Close: FnMut() -> CloseFut + Send + 'static,
    CloseFut: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let mut network = Some(recovery.availability);
        let mut recovering = initial;
        let mut failures = 0u32;
        let mut connected_since = None::<tokio::time::Instant>;
        loop {
            if *shutdown.borrow() {
                return;
            }
            if !recovering {
                tokio::select! {
                    event = lifecycle.recv() => match event {
                        Some(TransportLifecycleEvent::Disconnected { transport_id, .. }) if transport_id == id => {
                            if connected_since.is_some_and(|at| at.elapsed() >= Duration::from_secs(30)) { failures = 0; }
                            recovering = true;
                        }
                        Some(_) => continue,
                        None => return,
                    },
                    _ = stopped(&mut shutdown) => return,
                    changed = network_changed(&mut network) => {
                        if !changed { network = None; }
                        if network.as_ref().is_some_and(|rx| *rx.borrow() == NetworkAvailability::Offline) {
                            tokio::select! { _ = disconnect() => {}, _ = stopped(&mut shutdown) => return }
                            recovering = true;
                            failures = 0;
                        }
                        continue;
                    }
                }
            }
            let offline = network
                .as_ref()
                .is_some_and(|rx| *rx.borrow() == NetworkAvailability::Offline);
            if offline {
                tokio::select! {
                    _ = stopped(&mut shutdown) => return,
                    changed = network_changed(&mut network) => { if !changed { network = None; } failures = 0; }
                }
                continue;
            }
            let base = recovery.policy.delay_after_failures(failures);
            // Platform-only jitter spreads reconnects after a shared outage; phase policy remains deterministic.
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();
            let delay = if base >= Duration::from_secs(1) {
                base.mul_f64(0.8 + f64::from(seed % 401) / 1000.0)
                    .min(recovery.policy.background.max_delay)
            } else {
                base
            };
            trace.emit(helix_driver_host::engine::TransportTraceEvent {
                transport_id: id,
                name: "helix.ws.reconnect.schedule",
                action: "reconnect_schedule",
                attempt: Some(failures.saturating_add(1)),
                delay_ms: Some(delay.as_millis() as u64),
                next_delay_ms: None,
                reason: Some(if failures <= 3 {
                    "fast_recovery"
                } else {
                    "background_recovery"
                }),
                error_class: None,
            });
            tokio::select! {
                _ = tokio::time::sleep(delay) => {},
                _ = stopped(&mut shutdown) => return,
                changed = network_changed(&mut network) => { if !changed { network = None; } failures = 0; continue; }
            }
            // Reader notifications accumulated while disconnected are one recovery episode.
            while lifecycle.try_recv().is_ok() {}
            let attempt = connect();
            tokio::pin!(attempt);
            let result = loop {
                tokio::select! {
                    result = &mut attempt => break Some(result),
                    _ = stopped(&mut shutdown) => return,
                    changed = network_changed(&mut network) => {
                        if !changed { network = None; }
                        if network.as_ref().is_some_and(|rx| *rx.borrow() == NetworkAvailability::Offline) { break None; }
                    }
                }
            };
            match result {
                Some(Ok(())) => {
                    recovering = false;
                    connected_since = Some(tokio::time::Instant::now()); /* Preserve backoff across flapping handshakes. */
                }
                Some(Err(_)) => {
                    failures = failures.saturating_add(1);
                }
                None => {
                    failures = 0;
                }
            }
        }
    })
}

/// A closed network source degrades to unknown rather than spinning or disabling recovery.
async fn network_changed(network: &mut Option<watch::Receiver<NetworkAvailability>>) -> bool {
    match network {
        Some(rx) => rx.changed().await.is_ok(),
        None => std::future::pending().await,
    }
}

/// Shutdown cancels both backoff and an in-flight handshake.
async fn stopped(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow() {
        if rx.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "network_recovery/tests.rs"]
mod tests;
