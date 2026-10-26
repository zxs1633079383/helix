use std::collections::HashMap;
use std::sync::Arc;

use helix_core::effect::TransportId;
use helix_driver_host::engine::{
    register_transport, TransportLifecycleEvent, TransportRegistration, TransportTraceEvent,
    TransportTraceSink, TransportTraceStats,
};
use helix_driver_host::{spawn_reconnect_supervisor, ReconnectPolicy, ReconnectTraceSink};
use tokio::sync::{mpsc, watch};

use crate::event_sink::{bus_event_name, NativeEventSink, RecvOutcome};
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

/// 连接控制事件名（与 `helix_im::EVENT_NET_RECONNECT_REQUESTED` 同一合同）。
///
/// `helix-im` 只是本 crate 的 dev-dependency，生产代码不得引用 IM 域类型；此处复述字面量，
/// 由本文件测试对照 `helix_im` 常量断言一致，防止两侧漂移。
const RECONNECT_REQUESTED_EVENT: &str = "im:net:reconnect_requested";

/// 引擎判定半开连接后会 emit `im:net:reconnect_requested`；driver 必须把它翻译成一次断开事件，
/// 交给既有 reconnect supervisor，从而复用「断开 → 重建」这一条路径，不新增第二条重连状态机。
///
/// supervisor 自身对恢复期间的重复断开做合并（恢复中只 drain），因此这里逐 transport 转交即可。
fn spawn_reconnect_request_observer(
    event_sink: &NativeEventSink,
    lifecycle_tx: mpsc::UnboundedSender<TransportLifecycleEvent>,
    transport_ids: Vec<TransportId>,
) -> tokio::task::JoinHandle<()> {
    let mut events = event_sink.subscribe_observed_as("im:net:reconnect");
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                RecvOutcome::Event(event) => event,
                RecvOutcome::Lagged(_) => continue,
                RecvOutcome::Closed => return,
            };
            if bus_event_name(&event).as_deref() != Some(RECONNECT_REQUESTED_EVENT) {
                continue;
            }
            tracing::info!(
                transports = transport_ids.len(),
                "helix-native: 收到 im:net:reconnect_requested，交给 reconnect supervisor 重建连接"
            );
            for &transport_id in &transport_ids {
                let _ = lifecycle_tx.send(TransportLifecycleEvent::Disconnected {
                    transport_id,
                    reason: "reconnect_requested",
                });
            }
        }
    })
}

pub(super) fn start_native_reconnect(
    transports: &TransportTable,
    event_sink: &NativeEventSink,
) -> NativeReconnectRuntime {
    let (register_tx, register_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut lifecycle_routes = HashMap::new();
    let mut reconnect_ids: Vec<TransportId> = Vec::new();
    let (trace_sink, trace_rx) = TransportTraceSink::channel();
    let trace_stats = trace_sink.stats();
    let mut tasks = Vec::new();

    for (&transport_id, transport) in transports {
        let Some(factory) = transport.reconnect_factory() else {
            continue;
        };
        let (route_tx, route_rx) = mpsc::unbounded_channel();
        lifecycle_routes.insert(transport_id, route_tx);
        reconnect_ids.push(transport_id);
        let register_tx = register_tx.clone();
        let shutdown_rx = shutdown_rx.clone();
        let trace_sink = ReconnectTraceSink::new(Some(trace_sink.clone()));
        let initial = transport.needs_initial_connection();
        let network_recovery = factory.recovery.clone();
        let current = Arc::new(tokio::sync::Mutex::new(Arc::clone(transport)));
        let current_for_connect = Arc::clone(&current);
        let connect = move || {
            let current = Arc::clone(&current_for_connect);
            let factory = factory.clone();
            let register_tx = register_tx.clone();
            async move {
                let mut transport = factory.build()?;
                // Allocate the new epoch before closing the previous connection.
                let previous = Arc::clone(&*current.lock().await);
                previous.close().await.ok();
                let activation = transport.connect().await?;
                let transport = Arc::new(transport);
                register_transport(&register_tx, transport_id, Arc::clone(&transport)).await?;
                activation.activate().await?;
                *current.lock().await = transport;
                Ok(())
            }
        };
        tasks.push(match network_recovery {
            Some(recovery) => crate::network_recovery::spawn_network_recovery(
                transport_id,
                recovery,
                initial,
                route_rx,
                shutdown_rx,
                trace_sink,
                connect,
                move || {
                    let current = Arc::clone(&current);
                    async move {
                        let transport = Arc::clone(&*current.lock().await);
                        let _ = transport.close().await;
                    }
                },
            ),
            None => spawn_reconnect_supervisor(
                transport_id,
                ReconnectPolicy::default(),
                route_rx,
                shutdown_rx,
                trace_sink,
                connect,
            ),
        });
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
    // 确定性顺序：重建请求按 transport id 升序转交，回放断言不依赖 HashMap 迭代序。
    reconnect_ids.sort_unstable_by_key(|id| id.raw());
    tasks.push(spawn_reconnect_request_observer(
        event_sink,
        lifecycle_tx.clone(),
        reconnect_ids,
    ));

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

#[cfg(test)]
mod reconnect_request_tests {
    use super::*;
    use bytes::Bytes;
    use helix_core::effect::DomainEventBytes;
    use helix_core::ports::EventSink;
    use std::time::Duration;

    fn domain_event(name: &str) -> DomainEventBytes {
        let payload = serde_json::json!({ "event": name, "data": {} });
        DomainEventBytes(Bytes::from(
            serde_json::to_vec(&payload).expect("event fixture must serialize"),
        ))
    }

    /// 引擎的心跳逾期判定必须真正驱动重连：`im:net:reconnect_requested` → 断开事件；
    /// 其他事件不得触发重建（否则正常事件流会引发无谓重连）。
    #[tokio::test]
    async fn reconnect_requested_event_translates_to_disconnected_once() {
        let (sink, _rx) = NativeEventSink::new();
        let (lifecycle_tx, mut lifecycle_rx) = mpsc::unbounded_channel();
        let handle = spawn_reconnect_request_observer(
            &sink,
            lifecycle_tx,
            vec![TransportId::from_raw(0u32), TransportId::from_raw(1u32)],
        );

        sink.emit(domain_event("im:health"));
        assert!(
            tokio::time::timeout(Duration::from_millis(50), lifecycle_rx.recv())
                .await
                .is_err(),
            "非重建事件不得产生断开事件"
        );

        // 跨 crate 漂移守卫：driver 侧复述的字面量必须与 helix-im 的常量一致。
        assert_eq!(
            RECONNECT_REQUESTED_EVENT,
            helix_im::EVENT_NET_RECONNECT_REQUESTED
        );
        sink.emit(domain_event(helix_im::EVENT_NET_RECONNECT_REQUESTED));
        for expected in [0u32, 1u32] {
            let event = tokio::time::timeout(Duration::from_secs(1), lifecycle_rx.recv())
                .await
                .expect("重建信号必须转交 reconnect supervisor")
                .expect("lifecycle 通道保持打开");
            match event {
                TransportLifecycleEvent::Disconnected {
                    transport_id,
                    reason,
                } => {
                    assert_eq!(transport_id, TransportId::from_raw(expected));
                    assert_eq!(reason, "reconnect_requested");
                }
            }
        }
        handle.abort();
    }
}
