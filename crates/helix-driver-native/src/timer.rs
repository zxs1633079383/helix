//! TimerRegistry — ScheduleTimer / CancelTimer 兑现器
//!
//! 维护 TimerId → JoinHandle 映射。
//! 到期时产出 Tick::Timer{id} 并通过 channel 发给 engine_loop。

use std::collections::HashMap;
use helix_core::TimerId;
use tokio::task::JoinHandle;

pub struct TimerRegistry {
    handles: HashMap<TimerId, JoinHandle<()>>,
}

impl TimerRegistry {
    pub fn new() -> Self {
        Self { handles: HashMap::new() }
    }

    /// 调度 timer：after_ms 毫秒后发送 Tick::Timer{id} 到 tick_tx
    ///
    /// BLK-2 修复：`Tick` 不再带 lifetime，满足 `'static + Send`，
    /// `tokio::sync::mpsc::Sender<Tick>` 可正常使用，timer 触发路径现已通畅。
    pub fn schedule(
        &mut self,
        id: TimerId,
        after_ms: u64,
        tick_tx: tokio::sync::mpsc::Sender<helix_core::Tick>,
    ) {
        // 先取消已有的同 id timer（ScheduleTimer 可覆盖）
        self.cancel(id);

        let handle = tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(after_ms)).await;
            // BLK-2: Tick 现在是 'static + Send，send 可正常工作
            tick_tx.send(helix_core::Tick::Timer(id)).await.ok();
        });
        self.handles.insert(id, handle);
    }

    /// 取消 timer（幂等，id 不存在时静默）
    pub fn cancel(&mut self, id: TimerId) {
        if let Some(h) = self.handles.remove(&id) {
            h.abort();
        }
    }
}
