#![cfg(feature = "sdk-metrics")]

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::broadcast;

use super::ffi_probe::FfiProbe;
use super::l2_oracle::L2Oracle;

pub fn spawn(
    mut receiver: broadcast::Receiver<helix_core::effect::DomainEventBytes>,
    ffi: Arc<FfiProbe>,
    oracle: Arc<L2Oracle>,
    expected: u64,
) -> tokio::task::JoinHandle<Result<(), String>> {
    tokio::spawn(async move {
        while oracle.native_events.load(Ordering::Relaxed) < expected {
            let event = receiver.recv().await.map_err(|error| {
                format!("PERF_CORRECTNESS_VIOLATION: native event recv: {error}")
            })?;
            let operation = serde_json::from_slice::<serde_json::Value>(&event.0)
                .ok()
                .and_then(|value| value.get("operation").and_then(|value| value.as_u64()))
                .ok_or("PERF_CORRECTNESS_VIOLATION: event operation missing")?;
            oracle.observe_event_delivery(operation);
            oracle.native_events.fetch_add(1, Ordering::Relaxed);
            let started = Instant::now();
            ffi.forward(event);
            let elapsed = started.elapsed();
            oracle.observe_ffi_callback(elapsed);
            oracle.trace_hop(operation, "ffi.callback", elapsed);
            oracle.ffi_events.fetch_add(1, Ordering::Relaxed);
            oracle.observe_ffi_complete(operation);
        }
        Ok(())
    })
}
