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

/// IM 内部总线 event name（真源 cses-client event_bus.rs `BUS_CHANNEL`）。
///
/// 前端 `tauri.service.ts` 唯一订阅入口——所有 `im:*` 业务事件统一经此一个 Tauri
/// event 下发，前端 dispatcher 按 payload 内的 event 名分发（N 次 IPC 注册压成 1 次）。
pub const BUS_CHANNEL: &str = "im:__bus__";

/// C3-core：把 helix-im 吐出的 `DomainEventBytes`（`{event, data}` JSON）转成 `im:__bus__`
/// 总线信封 `{channel, payload}`（真源 cses-client event_bus.rs `BusEvent`）。
///
/// helix-im 的 emit_*（acl/to_effect.rs）已把事件序列化成 `{"event":"im:post:read","data":{...}}`；
/// 此函数读出 `event` 字段作 `channel`、整个对象作 `payload`，组装成前端 dispatcher 期望的
/// 信封形态。**单一转换点**——未来 helix-tauri 的 event_bus loop 用它把 DomainEvent → app.emit
/// 的 payload，避免转换逻辑散落。
///
/// ## M2 Tauri 接缝（host-cli 无窗口）
///
/// host-cli 无 Tauri `AppHandle` → **不调** `app.emit(BUS_CHANNEL, envelope)`；C3 验收以
/// 「EventReceiver 收到 DomainEvent + [`bus_event_name`] 类型计数正确」为准（见 host-cli
/// 消费 loop）。真 `app.emit` 信封在 M2 Tauri 装配层接 `AppHandle` 时落地：
/// ```rust,ignore
/// // helix-tauri event_bus loop（M2 接缝，本 crate 不依赖 tauri）：
/// RecvOutcome::Event(ev) => {
///     let envelope = to_bus_envelope(&ev);              // 本函数
///     let _ = app.emit(BUS_CHANNEL, &envelope);          // 真 Tauri emit
/// }
/// ```
/// 边界零信任：payload 非法 JSON / 缺 `event` 字段 → 回退 `channel=""`（不 panic）。
pub fn to_bus_envelope(ev: &DomainEventBytes) -> serde_json::Value {
    let payload: serde_json::Value =
        serde_json::from_slice(ev.0.as_ref()).unwrap_or(serde_json::Value::Null);
    let channel = payload
        .get("event")
        .and_then(|e| e.as_str())
        .unwrap_or("")
        .to_string();
    serde_json::json!({
        "channel": channel,
        "payload": payload,
    })
}

/// C3-core：从 `DomainEventBytes` 提取 `event` 名（如 `"im:post:read"`），供 host-cli 按
/// 类型计数验证（无窗口 → 不真 emit，靠计数证接线）。非法/缺字段 → None（零信任）。
pub fn bus_event_name(ev: &DomainEventBytes) -> Option<String> {
    let payload: serde_json::Value = serde_json::from_slice(ev.0.as_ref()).ok()?;
    payload
        .get("event")
        .and_then(|e| e.as_str())
        .map(str::to_string)
}

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

    // ─── C3-core：im:__bus__ 信封转换 + 事件名提取 ─────────────────────────────

    fn domain_event(json: serde_json::Value) -> DomainEventBytes {
        DomainEventBytes(Bytes::from(serde_json::to_vec(&json).unwrap()))
    }

    /// 回归锚: C3 —— DomainEvent（{event,data}）→ im:__bus__ 信封 {channel,payload}。
    #[test]
    fn to_bus_envelope_wraps_event_as_channel() {
        let ev = domain_event(serde_json::json!({
            "event": "im:post:read",
            "data": { "channel_id": "ch_1", "msg_id": "m_1" }
        }));
        let envelope = to_bus_envelope(&ev);
        assert_eq!(
            envelope.get("channel").and_then(|c| c.as_str()),
            Some("im:post:read"),
            "channel 取自 DomainEvent 的 event 字段"
        );
        // payload 完整保留原 {event,data} 对象（前端 dispatcher 消费）。
        assert_eq!(
            envelope.get("payload"),
            Some(&serde_json::json!({
                "event": "im:post:read",
                "data": { "channel_id": "ch_1", "msg_id": "m_1" }
            }))
        );
    }

    /// 边界零信任：非法 JSON payload → channel 回退空串，不 panic。
    #[test]
    fn to_bus_envelope_invalid_json_falls_back_empty_channel() {
        let ev = DomainEventBytes(Bytes::from_static(b"not json{"));
        let envelope = to_bus_envelope(&ev);
        assert_eq!(envelope.get("channel").and_then(|c| c.as_str()), Some(""));
    }

    /// host-cli 计数验证路径：从 DomainEvent 提取 event 名。
    #[test]
    fn bus_event_name_extracts_event_field() {
        let ev = domain_event(serde_json::json!({
            "event": "im:post:deleted",
            "data": {}
        }));
        assert_eq!(bus_event_name(&ev).as_deref(), Some("im:post:deleted"));
    }

    #[test]
    fn bus_event_name_none_on_missing_or_invalid() {
        assert_eq!(
            bus_event_name(&domain_event(serde_json::json!({"x": 1}))),
            None
        );
        assert_eq!(
            bus_event_name(&DomainEventBytes(Bytes::from_static(b"{["))),
            None
        );
    }

    #[test]
    fn bus_channel_constant_is_internal_namespace() {
        assert!(BUS_CHANNEL.starts_with("im:__"));
    }
}
