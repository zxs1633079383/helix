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
use helix_driver_host::{
    AsyncMetricSink, LabelKey, MetricEvent, MetricId, MetricLabels, NoopMetricSink,
};
use std::sync::Arc;
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
    metrics: Arc<dyn AsyncMetricSink>,
}

impl EventReceiver {
    /// 包装一个已有的 broadcast receiver。
    pub fn new(inner: broadcast::Receiver<DomainEventBytes>) -> Self {
        Self {
            inner,
            lagged_total: 0,
            metrics: Arc::new(NoopMetricSink),
        }
    }

    fn with_metrics(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        self.metrics = metrics;
        self
    }

    /// 取下一个结果。lagged 被显式上报为 [`RecvOutcome::Lagged`]（并累加计数），
    /// 不再被 broadcast 静默吞掉。
    pub async fn recv(&mut self) -> RecvOutcome {
        match self.inner.recv().await {
            Ok(ev) => RecvOutcome::Event(ev),
            Err(broadcast::error::RecvError::Lagged(n)) => {
                self.lagged_total = self.lagged_total.saturating_add(n);
                if self.metrics.is_enabled() {
                    let _ = self.metrics.try_record(MetricEvent::counter(
                        MetricId::EventLaggedTotal,
                        n as f64,
                        MetricLabels::one(LabelKey::Stage, "event"),
                    ));
                }
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
    metrics: Arc<dyn AsyncMetricSink>,
}

impl NativeEventSink {
    /// 创建 EventSink + 初始 Receiver（通常交给 event_bus 任务持有）。
    pub fn new() -> (Self, broadcast::Receiver<DomainEventBytes>) {
        Self::new_with_capacity(BROADCAST_CAPACITY)
    }

    /// 自定义容量创建（host 可按部署调大缓冲；测试可调小以触发 lagged）。
    pub fn new_with_capacity(capacity: usize) -> (Self, broadcast::Receiver<DomainEventBytes>) {
        let (sender, receiver) = broadcast::channel(capacity);
        (
            Self {
                sender,
                metrics: Arc::new(NoopMetricSink),
            },
            receiver,
        )
    }

    pub fn with_metric_sink(mut self, metrics: Arc<dyn AsyncMetricSink>) -> Self {
        self.metrics = metrics;
        self
    }

    /// 订阅新的接收端（裸 broadcast receiver，向后兼容）。
    pub fn subscribe(&self) -> broadcast::Receiver<DomainEventBytes> {
        self.sender.subscribe()
    }

    /// 订阅可观测接收端：lagged 不再静默（E7）。event_bus loop 应优先用这个。
    pub fn subscribe_observed(&self) -> EventReceiver {
        EventReceiver::new(self.sender.subscribe()).with_metrics(Arc::clone(&self.metrics))
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
        if self.sender.send(event).is_err() && self.metrics.is_enabled() {
            let _ = self.metrics.try_record(MetricEvent::counter(
                MetricId::EventNoReceiverTotal,
                1.0,
                MetricLabels::one(LabelKey::Stage, "event"),
            ));
        }
    }
}

// broadcast egress 是逐条即时 send，无需 step 批边界 → 用 BatchSink 默认 no-op。
// （host pump 每 step 调一次 flush()；native 这里什么都不做，行为零变。FFI 才攒批一次过边界。）
impl helix_driver_host::BatchSink for NativeEventSink {}

#[cfg(test)]
#[path = "event_sink_tests.rs"]
mod tests;
