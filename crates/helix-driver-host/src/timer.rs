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
//! - **one-shot 回收契约**：fire-once 后既不 re-arm 也不发 `CancelTimer` 的一次性
//!   timer（如 IM send-timeout），其 JoinHandle 会自然到期退出但不会自我移除——
//!   长运行会话每触发一次就漏一条已完成条目。修法：spawn 闭包在 send 完成后经
//!   `done_tx` 回灌自己的 id，`reap()` 在 schedule/cancel/pending_count 三个低频点
//!   惰性把已完成 id 从 handles 移除（不引入后台 task / 不阻塞热路径）。
//!   语义上与 core 侧 one-shot 概念对齐（core 改 `timer_map` remove-on-fire，本文件
//!   只管 host 侧 `handles` map 回收）——二者**不共享文件、不互相依赖、仅语义对齐**。

use helix_core::Tick;
use helix_core::TimerId;
use std::collections::HashMap;
use tokio::task::JoinHandle;

pub struct TimerRegistry {
    handles: HashMap<TimerId, JoinHandle<()>>,
    /// one-shot 自清理回灌通道：spawn 闭包 fire 完毕后把自身 id 送回，`reap()` 据此惰性回收。
    /// unbounded 对齐本 crate「回灌必达走 unbounded」不变量（避免回灌阻塞产生背压死锁）。
    done_tx: tokio::sync::mpsc::UnboundedSender<TimerId>,
    done_rx: tokio::sync::mpsc::UnboundedReceiver<TimerId>,
}

impl Default for TimerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerRegistry {
    pub fn new() -> Self {
        let (done_tx, done_rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            handles: HashMap::new(),
            done_tx,
            done_rx,
        }
    }

    /// 惰性回收已自然到期的 one-shot timer 条目：非阻塞排空 done 通道，把已完成 id
    /// 从 `handles` 移除。`try_recv` 空即停（O(已完成数)，不阻塞、不在热路径）。
    ///
    /// **reschedule 同 id 竞态护栏**：done 通道里可能滞留「旧 id 已 fire」的回灌，而此后
    /// schedule 用同 id 重新 arm 插入了新 handle。本函数仅做 `handles.remove(&id)` 排空，
    /// 调用约定保证 **reap 永远先于本次操作对 handles 的写入**（schedule/cancel 内 reap
    /// 在 cancel/insert 之前跑）——这样新 insert 的活 handle 不会被同批 reap 误删。
    fn reap(&mut self) {
        while let Ok(id) = self.done_rx.try_recv() {
            self.handles.remove(&id);
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
        // 先惰性回收已自然到期的 one-shot 条目（必须先于本次对 handles 的写入，
        // 防止下方 insert 的新 handle 被同批旧 done 信号误删——见 reap 护栏）。
        self.reap();
        // 先取消已有同 id timer（ScheduleTimer 可覆盖）
        self.cancel(id);

        let done_tx = self.done_tx.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(after_ms)).await;
            // send 失败（接收端已关闭）静默忽略（正常关机场景）
            tick_tx.send(Tick::Timer(id)).await.ok();
            // fire 完毕，回灌自身 id 供 registry 惰性回收；registry 已 drop 时静默忽略。
            done_tx.send(id).ok();
        });
        self.handles.insert(id, handle);
    }

    /// 取消 timer（幂等，id 不存在时静默忽略）。
    pub fn cancel(&mut self, id: TimerId) {
        // 先惰性回收（必须先于本次 remove 对 handles 的读写，与 schedule 同护栏）。
        self.reap();
        if let Some(h) = self.handles.remove(&id) {
            h.abort();
        }
    }

    /// 当前挂起的 timer 数量（测试用）
    #[cfg(test)]
    pub fn pending_count(&mut self) -> usize {
        // 先回收已自然到期的 one-shot 条目再报数，否则已 fire 的条目仍滞留计数。
        self.reap();
        self.handles.len()
    }
}

/// Drop 时 abort 所有挂起 timer 任务：把「`run_engine_loop` 返回 = 完全 drain」的
/// 保证扩展到 timer。`JoinHandle::drop` 仅 detach 不 abort——否则长 timer 任务会泄漏到
/// runtime 关停，且持 tick_tx clone 延迟 channel 关闭。与 `cancel` 的 abort 语义一致。
impl Drop for TimerRegistry {
    fn drop(&mut self) {
        for (_, h) in self.handles.drain() {
            h.abort();
        }
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

    /// one-shot timer 自然到期（收到 Tick::Timer）后，handles 不残留该 id（防缓慢内存泄漏）。
    #[tokio::test]
    async fn test_oneshot_timer_reaped_after_fire() {
        let (tx, mut rx) = mpsc::channel::<Tick>(8);
        let mut registry = TimerRegistry::new();

        let id = TimerId::from_raw(123);
        registry.schedule(id, 10, tx); // 10ms 后触发的一次性 timer

        // 等待 timer 自然触发
        let tick = tokio::time::timeout(tokio::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("timer should fire within 200ms")
            .expect("channel should not be closed");
        assert!(matches!(tick, Tick::Timer(tid) if tid == id));

        // 闭包在 send 之后才回灌 done_tx；让出执行权确保回灌已入通道，
        // 随后 reap（pending_count 内部触发）把已完成 id 从 handles 移除。
        tokio::task::yield_now().await;

        // pending_count() 内部调 reap()，应观察到该 one-shot 已被回收 → 归零。
        assert_eq!(
            registry.pending_count(),
            0,
            "one-shot timer should be reaped from handles after natural fire"
        );
    }
}
