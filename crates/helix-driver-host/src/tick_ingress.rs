use std::sync::Arc;
use std::time::{Duration, Instant};

use helix_core::Tick;
use tokio::sync::mpsc;

use crate::metrics::{
    AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels, NoopMetricSink,
};

struct StampedTick {
    tick: Tick,
    enqueued_at: Instant,
}

/// 生产 tick 入站发送端：时间戳和 Tick 同在一个有界 mpsc 元素内。
///
/// `reserve` 成功后才取时间戳，因此等待值严格表示“实际入队 → engine 消费”；
/// 不增加旁路队列，也不改变原 channel 容量和背压点。
#[derive(Clone)]
pub struct TickIngressSender {
    tx: mpsc::Sender<StampedTick>,
    metrics: Arc<dyn AsyncMetricSink>,
}

pub struct TickIngressReceiver {
    rx: mpsc::Receiver<StampedTick>,
}

pub fn tick_ingress_channel(capacity: usize) -> (TickIngressSender, TickIngressReceiver) {
    tick_ingress_channel_observed(capacity, Arc::new(NoopMetricSink))
}

/// 创建带 command admission 指标的生产 channel，容量与原背压点完全一致。
pub fn tick_ingress_channel_observed(
    capacity: usize,
    metrics: Arc<dyn AsyncMetricSink>,
) -> (TickIngressSender, TickIngressReceiver) {
    let (tx, rx) = mpsc::channel(capacity.max(1));
    (
        TickIngressSender { tx, metrics },
        TickIngressReceiver { rx },
    )
}

impl TickIngressSender {
    pub async fn send(&self, tick: Tick) -> Result<(), Tick> {
        let started = self.metrics.is_enabled().then(Instant::now);
        let kind = tick_kind(&tick);
        let is_command = matches!(tick, Tick::Command(_));
        let permit = match self.tx.reserve().await {
            Ok(permit) => permit,
            Err(_) => {
                self.record_rejected(kind, is_command, "closed");
                return Err(tick);
            }
        };
        permit.send(StampedTick {
            tick,
            enqueued_at: Instant::now(),
        });
        self.record_accepted(kind, is_command, started);
        Ok(())
    }

    pub fn try_send(&self, tick: Tick) -> Result<(), mpsc::error::TrySendError<Tick>> {
        let started = self.metrics.is_enabled().then(Instant::now);
        let kind = tick_kind(&tick);
        let is_command = matches!(tick, Tick::Command(_));
        let permit = match self.tx.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(())) => {
                self.record_rejected(kind, is_command, "full");
                return Err(mpsc::error::TrySendError::Full(tick));
            }
            Err(mpsc::error::TrySendError::Closed(())) => {
                self.record_rejected(kind, is_command, "closed");
                return Err(mpsc::error::TrySendError::Closed(tick));
            }
        };
        permit.send(StampedTick {
            tick,
            enqueued_at: Instant::now(),
        });
        self.record_accepted(kind, is_command, started);
        Ok(())
    }

    /// 记录真正成功 reserve+send 的 admission，不把 engine 消费误当接纳。
    fn record_accepted(&self, kind: &'static str, is_command: bool, started: Option<Instant>) {
        if !self.metrics.is_enabled() {
            return;
        }
        let labels = MetricLabels::one(LabelKey::Stage, "ingress")
            .with(LabelKey::TickKind, kind)
            .with(LabelKey::Status, "accepted");
        if is_command {
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::CommandAcceptedTotal,
                1.0,
                labels,
            ));
            if let Some(started) = started {
                let _ = self.metrics.try_record(MetricEvent::histogram(
                    MetricId::CommandAdmissionDurationSeconds,
                    started.elapsed().as_secs_f64(),
                    labels,
                ));
            }
        }
    }

    /// 记录 full/closed 拒绝和队列满分母，保持 try_send 非阻塞。
    fn record_rejected(&self, kind: &'static str, is_command: bool, reason: &'static str) {
        if !self.metrics.is_enabled() {
            return;
        }
        let labels = MetricLabels::one(LabelKey::Stage, "ingress")
            .with(LabelKey::TickKind, kind)
            .with(LabelKey::Status, reason);
        if reason == "full" {
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::TickQueueFullTotal,
                1.0,
                labels,
            ));
        }
        if is_command {
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::CommandRejectedTotal,
                1.0,
                labels,
            ));
        }
    }
}

/// 将 Tick 映射为固定低基数 admission 标签。
fn tick_kind(tick: &Tick) -> &'static str {
    match tick {
        Tick::Inbound(_) => "inbound",
        Tick::PortReply { .. } => "port_reply",
        Tick::PortProgress { .. } => "port_progress",
        Tick::Timer(_) => "timer",
        Tick::Command(_) => "command",
        Tick::Connected(_) => "connected",
        Tick::Disconnected(_) => "disconnected",
    }
}

impl TickIngressReceiver {
    pub(crate) async fn recv(&mut self) -> Option<(Tick, Duration)> {
        self.rx
            .recv()
            .await
            .map(|item| (item.tick, item.enqueued_at.elapsed()))
    }

    pub(crate) fn len(&self) -> usize {
        self.rx.len()
    }

    pub(crate) fn try_recv(&mut self) -> Result<(Tick, Duration), mpsc::error::TryRecvError> {
        self.rx
            .try_recv()
            .map(|item| (item.tick, item.enqueued_at.elapsed()))
    }

    pub(crate) fn max_capacity(&self) -> usize {
        self.rx.max_capacity()
    }
}

pub(crate) enum EngineTickSender {
    Raw(mpsc::Sender<Tick>),
    Stamped(TickIngressSender),
}

impl Clone for EngineTickSender {
    fn clone(&self) -> Self {
        match self {
            Self::Raw(tx) => Self::Raw(tx.clone()),
            Self::Stamped(tx) => Self::Stamped(tx.clone()),
        }
    }
}

impl EngineTickSender {
    pub(crate) async fn send(&self, tick: Tick) -> Result<(), Tick> {
        match self {
            Self::Raw(tx) => tx.send(tick).await.map_err(|error| error.0),
            Self::Stamped(tx) => tx.send(tick).await,
        }
    }
}

pub(crate) enum EngineTickReceiver {
    Raw(mpsc::Receiver<Tick>),
    Stamped(TickIngressReceiver),
}

impl EngineTickReceiver {
    pub(crate) async fn recv(&mut self) -> Option<(Tick, Option<Duration>)> {
        match self {
            Self::Raw(rx) => rx.recv().await.map(|tick| (tick, None)),
            Self::Stamped(rx) => rx.recv().await.map(|(tick, wait)| (tick, Some(wait))),
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Self::Raw(rx) => rx.len(),
            Self::Stamped(rx) => rx.len(),
        }
    }

    pub(crate) fn try_recv(
        &mut self,
    ) -> Result<(Tick, Option<Duration>), mpsc::error::TryRecvError> {
        match self {
            Self::Raw(rx) => rx.try_recv().map(|tick| (tick, None)),
            Self::Stamped(rx) => rx.try_recv().map(|(tick, wait)| (tick, Some(wait))),
        }
    }

    pub(crate) fn max_capacity(&self) -> usize {
        match self {
            Self::Raw(rx) => rx.max_capacity(),
            Self::Stamped(rx) => rx.max_capacity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::RecordOutcome;
    use helix_core::effect::TimerId;
    use helix_core::tick::AppCommand;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingMetricSink(Mutex<Vec<MetricEvent>>);

    impl AsyncMetricSink for RecordingMetricSink {
        /// 保存 admission 指标事件，不启动真实 exporter。
        fn try_record(&self, event: MetricEvent) -> RecordOutcome {
            self.0.lock().unwrap().push(event);
            RecordOutcome::Accepted
        }
    }

    #[tokio::test]
    async fn timestamp_lives_in_same_bounded_queue_element() {
        let (tx, mut rx) = tick_ingress_channel(1);
        tx.send(Tick::Timer(TimerId::from_raw(1)))
            .await
            .expect("first tick enters queue");
        assert!(matches!(
            tx.try_send(Tick::Timer(TimerId::from_raw(2))),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        tokio::time::sleep(Duration::from_millis(5)).await;
        let (_, wait) = rx.recv().await.expect("stamped tick");
        assert!(wait >= Duration::from_millis(5));
        assert_eq!(rx.max_capacity(), 1, "stamping 不得引入第二缓冲区");
    }

    /// command 的成功接纳与队满拒绝必须在真正 channel admission 点计数。
    #[tokio::test]
    async fn observed_channel_records_command_admission_and_rejection() {
        let metrics = Arc::new(RecordingMetricSink::default());
        let (tx, _rx) = tick_ingress_channel_observed(1, metrics.clone());
        tx.try_send(Tick::Command(AppCommand::new("first", bytes::Bytes::new())))
            .expect("第一条 command 应成功入队");
        assert!(matches!(
            tx.try_send(Tick::Command(AppCommand::new(
                "second",
                bytes::Bytes::new()
            ))),
            Err(mpsc::error::TrySendError::Full(_))
        ));

        let events = metrics.0.lock().unwrap();
        for expected in [
            MetricId::CommandAcceptedTotal,
            MetricId::CommandAdmissionDurationSeconds,
            MetricId::CommandRejectedTotal,
            MetricId::TickQueueFullTotal,
        ] {
            assert!(
                events.iter().any(|event| event.id == expected),
                "admission 缺少 {:?}",
                expected
            );
        }
    }
}
