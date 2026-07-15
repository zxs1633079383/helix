use std::collections::HashMap;
use std::sync::Arc;

use helix_driver_host::engine::{
    register_transport, TransportLifecycleEvent, TransportRegistration, TransportTraceEvent,
    TransportTraceSink, TransportTraceStats,
};
use helix_driver_host::{spawn_reconnect_supervisor, ReconnectPolicy, ReconnectTraceSink};
use tokio::sync::{mpsc, watch};

use crate::transport::NativeTransport;

use super::TransportTable;

pub(super) struct NativeReconnectRuntime {
    pub(super) transport_rx: mpsc::UnboundedReceiver<TransportRegistration<NativeTransport>>,
    pub(super) lifecycle_tx: Option<mpsc::UnboundedSender<TransportLifecycleEvent>>,
    pub(super) trace_sink: Option<TransportTraceSink>,
    pub(super) trace_stats: Option<TransportTraceStats>,
    pub(super) tasks: Vec<tokio::task::JoinHandle<()>>,
    pub(super) shutdown_tx: watch::Sender<bool>,
}

pub(super) fn start_native_reconnect(transports: &TransportTable) -> NativeReconnectRuntime {
    let (register_tx, register_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut lifecycle_routes = HashMap::new();
    let (trace_sink, trace_rx) = TransportTraceSink::channel();
    let trace_stats = trace_sink.stats();
    let mut tasks = Vec::new();

    for (&transport_id, transport) in transports {
        let Some(factory) = transport.reconnect_factory() else {
            continue;
        };
        let (route_tx, route_rx) = mpsc::unbounded_channel();
        lifecycle_routes.insert(transport_id, route_tx);
        let register_tx = register_tx.clone();
        let shutdown_rx = shutdown_rx.clone();
        let trace_sink = ReconnectTraceSink::new(Some(trace_sink.clone()));
        tasks.push(spawn_reconnect_supervisor(
            transport_id,
            ReconnectPolicy::default(),
            route_rx,
            shutdown_rx,
            trace_sink,
            move || {
                let factory = factory.clone();
                let register_tx = register_tx.clone();
                async move {
                    let mut transport = factory.build()?;
                    let activation = transport.connect().await?;
                    let transport = Arc::new(transport);
                    register_transport(
                        &register_tx,
                        transport_id,
                        Arc::clone(&transport),
                    )
                    .await?;
                    activation.activate().await
                }
            },
        ));
    }
    drop(register_tx);

    if lifecycle_routes.is_empty() {
        return NativeReconnectRuntime {
            transport_rx: register_rx,
            lifecycle_tx: None,
            trace_sink: None,
            trace_stats: None,
            tasks,
            shutdown_tx,
        };
    }

    let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
    tasks.push(tokio::spawn(async move {
        while let Some(event) = lifecycle_rx.recv().await {
            let TransportLifecycleEvent::Disconnected { transport_id, .. } = event;
            if let Some(route) = lifecycle_routes.get(&transport_id) {
                route.send(event).ok();
            }
        }
    }));
    tasks.push(spawn_transport_trace_logger(trace_rx));

    NativeReconnectRuntime {
        transport_rx: register_rx,
        lifecycle_tx: Some(lifecycle_tx),
        trace_sink: Some(trace_sink),
        trace_stats: Some(trace_stats),
        tasks,
        shutdown_tx,
    }
}

fn spawn_transport_trace_logger(
    mut rx: mpsc::Receiver<TransportTraceEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            tracing::info!(
                target: "helix_ws_lifecycle",
                transport_id = event.transport_id.raw(),
                action = event.action,
                attempt = ?event.attempt,
                delay_ms = ?event.delay_ms,
                next_delay_ms = ?event.next_delay_ms,
                reason = ?event.reason,
                error_class = ?event.error_class,
                "Helix WebSocket lifecycle"
            );
        }
    })
}
