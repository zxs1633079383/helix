//! NativeEventSink — tokio broadcast channel
//!
//! emit 是同步非阻塞的：send 入队即返回。容量默认 1024（缓冲约 1 秒突发）。
//!
//! ## lagged 不再静默（E7）
//!
//! tokio broadcast 的「慢消费者落后」**只在接收侧**体现为 `recv()` 返回
//! `RecvError::Lagged(n)`——发送侧 `send()` 只会在「无接收者」时报错，**感知不到
//! lagged**。所以消除「静默丢事件」必须在消费侧做：用 [`EventReceiver`] 包装
//! broadcast receiver，把 lagged 升级成显式 [`RecvOutcome::Lagged`] 信号，消费方
//! （event_bus loop）据此触发 UI 全量刷新 / 重拉，而不是无声跳过。
//!
//! ## 订阅方式
//!
//! ```rust,ignore
//! let (sink, _rx) = NativeEventSink::new();
//! // helix-tauri 的 event_bus.rs 用 sink.subscribe_observed() 拿到 EventReceiver，
//! // loop { match rx.recv().await {
//! //   RecvOutcome::Event(ev) => app.emit("im:__bus__", ev),
//! //   RecvOutcome::Lagged(n) => app.emit("im:sync:resync_needed", n), // 不再静默
//! //   RecvOutcome::Closed    => break,
//! // }}
//! ```

use helix_core::effect::DomainEventBytes;
use helix_core::ports::EventSink;
use tokio::sync::broadcast;

/// broadcast channel 默认容量：缓冲突发事件，超出后慢消费者会 lagged。
const BROADCAST_CAPACITY: usize = 1024;

/// 消费侧 recv 结果：把 broadcast 的 lagged 从「静默跳过」升级为显式信号（E7）。
#[derive(Debug)]
pub enum RecvOutcome {
    /// 正常领域事件。
    Event(DomainEventBytes),
    /// 接收方落后、被 broadcast 丢弃了 `n` 条事件——消费侧应触发 UI 全量刷新 / 重拉，
    /// 而不是当作无事发生（这正是 E7 要消除的「静默丢」）。
    Lagged(u64),
    /// 所有 sender 已 drop，事件流结束。
    Closed,
}

/// broadcast receiver 的可观测包装：lagged 不再静默，累计落后计数可读。
///
/// recv 是 `&mut self`，落后计数用普通 `u64` 即可（无需原子）。
pub struct EventReceiver {
    inner: broadcast::Receiver<DomainEventBytes>,
    lagged_total: u64,
}

impl EventReceiver {
    /// 包装一个已有的 broadcast receiver。
    pub fn new(inner: broadcast::Receiver<DomainEventBytes>) -> Self {
        Self {
            inner,
            lagged_total: 0,
        }
    }

    /// 取下一个结果。lagged 被显式上报为 [`RecvOutcome::Lagged`]（并累加计数），
    /// 不再被 broadcast 静默吞掉。
    pub async fn recv(&mut self) -> RecvOutcome {
        match self.inner.recv().await {
            Ok(ev) => RecvOutcome::Event(ev),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                self.lagged_total = self.lagged_total.saturating_add(n);
                RecvOutcome::Lagged(n)
            }
            Err(broadcast::error::RecvError::Closed) => RecvOutcome::Closed,
        }
    }

    /// 累计被丢弃的事件总数（可观测性 / 测试）。
    pub fn lagged_total(&self) -> u64 {
        self.lagged_total
    }
}

/// 领域事件发布端（同步非阻塞）。
///
/// Clone 是 O(1)：内部 `broadcast::Sender` 是 Arc 包裹，引用计数复制。
#[derive(Clone)]
pub struct NativeEventSink {
    sender: broadcast::Sender<DomainEventBytes>,
}

impl NativeEventSink {
    /// 创建 EventSink + 初始 Receiver（通常交给 event_bus 任务持有）。
    pub fn new() -> (Self, broadcast::Receiver<DomainEventBytes>) {
        Self::new_with_capacity(BROADCAST_CAPACITY)
    }

    /// 自定义容量创建（host 可按部署调大缓冲；测试可调小以触发 lagged）。
    pub fn new_with_capacity(capacity: usize) -> (Self, broadcast::Receiver<DomainEventBytes>) {
        let (sender, receiver) = broadcast::channel(capacity);
        (Self { sender }, receiver)
    }

    /// 订阅新的接收端（裸 broadcast receiver，向后兼容）。
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEventBytes> {
        self.sender.subscribe()
    }

    /// 订阅可观测接收端：lagged 不再静默（E7）。event_bus loop 应优先用这个。
    pub fn subscribe_observed(&self) -> EventReceiver {
        EventReceiver::new(self.sender.subscribe())
    }

    /// 当前活跃订阅者数量（测试用）。
    pub fn receiver_count(&self) -> usize {
        self.sender.receiver_count()
    }
}

impl EventSink for NativeEventSink {
    /// 同步非阻塞 emit：try_send 失败（无接收者 / channel 满）静默忽略。
    ///
    /// 设计选择：EventSink 契约要求"非阻塞"，send 即使无接收者也不应 panic。
    /// broadcast::send 在无接收者时返回 Err(SendError)，此处静默 .ok()。
    fn emit(&self, event: DomainEventBytes) {
        // send() 在 broadcast 中是同步的，不需要 await
        // 错误（无接收者 = Err(SendError)）静默忽略
        self.sender.send(event).ok();
    }
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[tokio::test]
    async fn test_emit_received_by_subscriber() {
        let (sink, mut rx) = NativeEventSink::new();

        let payload = Bytes::from_static(b"hello-event");
        sink.emit(DomainEventBytes(payload.clone()));

        let received = rx.recv().await.expect("should receive event");
        assert_eq!(received.0, payload);
    }

    #[tokio::test]
    async fn test_emit_no_receiver_does_not_panic() {
        let (sink, rx) = NativeEventSink::new();
        drop(rx); // 无接收者

        // 不应 panic
        sink.emit(DomainEventBytes(Bytes::from_static(b"orphan")));
    }

    /// 回归锚: E7 —— 慢消费者落后时，lagged 被显式上报为 RecvOutcome::Lagged，
    /// 不再被 broadcast 静默吞掉。
    #[tokio::test]
    async fn test_lagged_surfaced_not_silent_e7() {
        // 容量 2 + 不消费地灌 5 条 → 接收方落后 3 条
        let (sink, rx) = NativeEventSink::new_with_capacity(2);
        let mut obs = EventReceiver::new(rx);
        for i in 0..5u8 {
            sink.emit(DomainEventBytes(Bytes::from(vec![i])));
        }

        // 第一个结果必须是 Lagged（显式信号），而非直接跳到 Event
        match obs.recv().await {
            RecvOutcome::Lagged(n) => {
                assert!(n >= 1, "lagged 数应 >= 1，实际 {n}");
                assert!(obs.lagged_total() >= 1, "lagged_total 应累加");
            }
            other => panic!("落后时首个结果必须是 Lagged，实际 {other:?}"),
        }

        // lagged 之后仍能继续取到保留窗口内的事件（流不中断）
        match obs.recv().await {
            RecvOutcome::Event(_) => {}
            other => panic!("lagged 后应继续取到 Event，实际 {other:?}"),
        }
    }

    /// 所有 sender drop 后，recv 返回 Closed（消费侧据此退出 loop）。
    #[tokio::test]
    async fn test_closed_outcome_e7() {
        let (sink, rx) = NativeEventSink::new();
        let mut obs = EventReceiver::new(rx);
        drop(sink);
        assert!(matches!(obs.recv().await, RecvOutcome::Closed));
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        let (sink, mut rx1) = NativeEventSink::new();
        let mut rx2 = sink.subscribe();

        let payload = Bytes::from_static(b"broadcast");
        sink.emit(DomainEventBytes(payload.clone()));

        assert_eq!(rx1.recv().await.expect("rx1").0, payload);
        assert_eq!(rx2.recv().await.expect("rx2").0, payload);
    }
}
