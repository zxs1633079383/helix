//! NativeEventSink — tokio broadcast channel
//!
//! emit 是同步非阻塞的：try_send 入队即返回（lagged 接收方自行处理）。
//! 容量 1024：足以缓冲 1 秒的突发事件（IM 消息推送通常 < 100/s）。
//!
//! ## 订阅方式
//!
//! ```rust,ignore
//! let (sink, rx) = NativeEventSink::new();
//! // helix-tauri 的 event_bus.rs 调用 sink.subscribe() 拿到 broadcast::Receiver
//! // 然后在 tokio 任务里 loop { rx.recv().await }，转发给 Tauri AppHandle.emit
//! ```

use tokio::sync::broadcast;
use helix_core::ports::EventSink;
use helix_core::effect::DomainEventBytes;

/// broadcast channel 容量：缓冲突发事件，lagged 接收方跳过过旧消息。
const BROADCAST_CAPACITY: usize = 1024;

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
        let (sender, receiver) = broadcast::channel(BROADCAST_CAPACITY);
        (Self { sender }, receiver)
    }

    /// 订阅新的接收端（允许多个订阅者，如 Tauri 事件总线 + 测试 spy）。
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEventBytes> {
        self.sender.subscribe()
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
