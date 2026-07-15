use std::time::{Duration, Instant};

use helix_core::Tick;
use tokio::sync::mpsc;

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
}

pub struct TickIngressReceiver {
    rx: mpsc::Receiver<StampedTick>,
}

pub fn tick_ingress_channel(capacity: usize) -> (TickIngressSender, TickIngressReceiver) {
    let (tx, rx) = mpsc::channel(capacity.max(1));
    (TickIngressSender { tx }, TickIngressReceiver { rx })
}

impl TickIngressSender {
    pub async fn send(&self, tick: Tick) -> Result<(), Tick> {
        let permit = match self.tx.reserve().await {
            Ok(permit) => permit,
            Err(_) => return Err(tick),
        };
        permit.send(StampedTick {
            tick,
            enqueued_at: Instant::now(),
        });
        Ok(())
    }

    pub fn try_send(&self, tick: Tick) -> Result<(), mpsc::error::TrySendError<Tick>> {
        let permit = match self.tx.try_reserve() {
            Ok(permit) => permit,
            Err(mpsc::error::TrySendError::Full(())) => {
                return Err(mpsc::error::TrySendError::Full(tick));
            }
            Err(mpsc::error::TrySendError::Closed(())) => {
                return Err(mpsc::error::TrySendError::Closed(tick));
            }
        };
        permit.send(StampedTick {
            tick,
            enqueued_at: Instant::now(),
        });
        Ok(())
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
    use helix_core::effect::TimerId;

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
}
