//! TimerRegistry — ScheduleTimer / CancelTimer 兑现器
//!
//! 维护 TimerId → JoinHandle 映射。
//! 到期时通过 mpsc channel 把 Tick::Timer{id} 发给 engine_loop。
//!
//! ## 设计要点
//!
//! - `schedule()` 先 cancel 同 id 的旧 timer（ScheduleTimer 可覆盖，幂等）
//! - `cancel()` abort JoinHandle + 移除（幂等，id 不存在时静默）
//! - JoinHandle 不 await：fire-and-forget spawn，timer 到期自行发送 Tick

use helix_core::Tick;
use helix_core::TimerId;
use std::collections::HashMap;
use tokio::task::JoinHandle;

pub struct TimerRegistry {
    handles: HashMap<TimerId, JoinHandle<()>>,
}

impl TimerRegistry {
    pub fn new() -> Self {
        Self {
            handles: HashMap::new(),
        }
    }

    /// 调度 timer：after_ms 毫秒后把 `Tick::Timer{id}` 发到 tick_tx。
    ///
    /// 若同 id 的 timer 已存在，先取消（ScheduleTimer 覆盖语义）。
    ///
    /// `tick_tx` 是 engine_loop 的入站通道，timer 触发与其他 Tick 源（WS/HTTP/Command）
    /// 统一汇入同一 mpsc，由串行事件泵顺序处理。
    pub fn schedule(
        &mut self,
        id: TimerId,
        after_ms: u64,
        tick_tx: tokio::sync::mpsc::Sender<Tick>,
    ) {
        // 先取消已有同 id timer（ScheduleTimer 可覆盖）
        self.cancel(id);

        let handle = tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(after_ms)).await;
            // send 失败（接收端已关闭）静默忽略（正常关机场景）
            tick_tx.send(Tick::Timer(id)).await.ok();
        });
        self.handles.insert(id, handle);
    }

    /// 取消 timer（幂等，id 不存在时静默忽略）。
    pub fn cancel(&mut self, id: TimerId) {
        if let Some(h) = self.handles.remove(&id) {
            h.abort();
        }
    }

    /// 当前挂起的 timer 数量（测试用）
    #[cfg(test)]
    pub fn pending_count(&self) -> usize {
        self.handles.len()
    }
}

// ─── 单元测试 ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::TimerId;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_timer_fires_after_delay() {
        let (tx, mut rx) = mpsc::channel::<Tick>(8);
        let mut registry = TimerRegistry::new();

        let id = TimerId::from_raw(42);
        registry.schedule(id, 10, tx); // 10ms 后触发

        let tick = tokio::time::timeout(tokio::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("timer should fire within 200ms")
            .expect("channel should not be closed");

        assert!(matches!(tick, Tick::Timer(tid) if tid == id));
    }

    #[tokio::test]
    async fn test_cancel_prevents_fire() {
        let (tx, mut rx) = mpsc::channel::<Tick>(8);
        let mut registry = TimerRegistry::new();

        let id = TimerId::from_raw(99);
        // 10s timer — 立即 cancel 后，在 50ms 窗口内不应收到 Tick::Timer
        registry.schedule(id, 10_000, tx.clone()); // clone tx 保持通道开放
        registry.cancel(id);
        // tx 仍然 alive（clone 持有），rx.recv() 会阻塞（而非立即返回 None）
        // 因此 timeout 50ms 后应超时（Err），而非收到消息（Ok）

        let result = tokio::time::timeout(tokio::time::Duration::from_millis(50), rx.recv()).await;

        // timeout elapsed = Err(Elapsed)，表示 timer 未触发（符合预期）
        assert!(
            result.is_err(),
            "cancelled timer should not fire within 50ms, result was ready"
        );

        // 显式 drop tx（避免 rx 挂起）
        drop(tx);
    }

    #[tokio::test]
    async fn test_reschedule_overwrites() {
        let (tx, mut rx) = mpsc::channel::<Tick>(8);
        let mut registry = TimerRegistry::new();

        let id = TimerId::from_raw(7);
        registry.schedule(id, 10_000, tx.clone()); // 10s timer
        registry.schedule(id, 10, tx); // 10ms 覆盖

        let tick = tokio::time::timeout(tokio::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("reschedule should fire fast")
            .expect("channel open");

        assert!(matches!(tick, Tick::Timer(tid) if tid == id));
    }

    #[tokio::test]
    async fn test_cancel_idempotent() {
        let mut registry = TimerRegistry::new();
        let id = TimerId::from_raw(1);
        // 取消不存在的 id 不 panic
        registry.cancel(id);
        registry.cancel(id);
    }
}
