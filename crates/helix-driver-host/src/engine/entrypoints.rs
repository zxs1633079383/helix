use helix_core::ports::{Clock, FileUploader, FrameSender, HttpRequester, Storage};
use helix_core::{ExecutionShell, Tick};
use tokio::sync::{mpsc, oneshot};

use super::{
    run_engine_loop_inner, BatchSink, EngineDeps, TransportRegistration, TransportTable,
};
use crate::tick_ingress::{
    EngineTickReceiver, EngineTickSender, TickIngressReceiver, TickIngressSender,
};

#[allow(clippy::too_many_arguments)]
pub async fn run_engine_loop<S, H, U, E, Tr, C>(
    shell: ExecutionShell,
    tick_rx: mpsc::Receiver<Tick>,
    tick_tx: mpsc::Sender<Tick>,
    deps: EngineDeps<S, H, U, E, C>,
    shutdown_rx: oneshot::Receiver<()>,
    transports: TransportTable<Tr>,
    transport_rx: mpsc::UnboundedReceiver<TransportRegistration<Tr>>,
) where
    S: Storage + Send + Sync + 'static,
    H: HttpRequester + Send + Sync + 'static,
    U: FileUploader + Send + Sync + 'static,
    E: BatchSink + Send + Sync + 'static,
    Tr: FrameSender + Send + Sync + 'static,
    C: Clock,
{
    run_engine_loop_inner(
        shell,
        EngineTickReceiver::Raw(tick_rx),
        EngineTickSender::Raw(tick_tx),
        deps,
        shutdown_rx,
        transports,
        transport_rx,
    )
    .await;
}

/// 生产入口：Tick 与入队时间戳同处一个有界 channel 元素，容量与背压语义不变。
#[allow(clippy::too_many_arguments)]
pub async fn run_engine_loop_stamped<S, H, U, E, Tr, C>(
    shell: ExecutionShell,
    tick_rx: TickIngressReceiver,
    tick_tx: TickIngressSender,
    deps: EngineDeps<S, H, U, E, C>,
    shutdown_rx: oneshot::Receiver<()>,
    transports: TransportTable<Tr>,
    transport_rx: mpsc::UnboundedReceiver<TransportRegistration<Tr>>,
) where
    S: Storage + Send + Sync + 'static,
    H: HttpRequester + Send + Sync + 'static,
    U: FileUploader + Send + Sync + 'static,
    E: BatchSink + Send + Sync + 'static,
    Tr: FrameSender + Send + Sync + 'static,
    C: Clock,
{
    run_engine_loop_inner(
        shell,
        EngineTickReceiver::Stamped(tick_rx),
        EngineTickSender::Stamped(tick_tx),
        deps,
        shutdown_rx,
        transports,
        transport_rx,
    )
    .await;
}
