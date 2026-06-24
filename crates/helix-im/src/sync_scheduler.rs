//! SyncScheduler — 全局 sync 并发窗口（B4，接管本质差异）。
//!
//! ## 为什么（接管 vs 现网）
//!
//! 现网 cses-client 重连后逐 channel 直发 `sync/notify`（`increment_channel_end` 收尾对
//! `(increment_fetched − need_sync_skip)` 每个 channel 立即发 HTTP）。helix 接管 HTTP 后，257
//! channel 同时 needSync 会一次性吐 257 条并发 `Effect::Http` → 重连风暴打爆 driver 连接池 /
//! 后端。SyncScheduler 在 core 侧加一道**全局并发窗口**：所有待 sync channel 入
//! `pending_sync: VecDeque`，在途数 < K 才取队首 dispatch；一个 sync 回报释放 1 个窗口再 drain。
//!
//! ## 不变量（HX-C011 证伪锚点）
//!
//! - **在途 sync ≤ K**：任意时刻 `inflight` 计数 ≤ `MAX_INFLIGHT_SYNC`（破坏即 `p4_sync_scheduler_test` fail）。
//! - **不丢 channel**：入队的 channel 终将被 dispatch（队列 FIFO，每次 drain 释放后补发）。
//! - **确定性有序**：入队顺序由调用方排序锚定（HX-C010，`increment_channel_end` 已 `sort_unstable`）；
//!   VecDeque FIFO 保持有序出队，不引入 SipHash 随机。
//! - **幂等防重**：已在途（`inflight_sync.is_some()`）或已在队（`enqueued` 去重集）的 channel 不重复入队。
//!
//! ## 复杂度
//!
//! enqueue = O(1) push_back + O(1) HashSet 去重；drain 单步 = O(1) pop_front。窗口释放 O(1)。
//! 不读 / 不改 channel 消息体——纯调度元数据（HX-C005 热路径友好）。

use crate::state::{ChannelId, CorrelationContext, ImState, InflightSync};
use helix_core::{Correlation, EffectSink};
use std::collections::{HashMap, HashSet, VecDeque};

/// 全局在途 sync 上限 K（B4 风暴闸门）。
///
/// 真源风暴规模：现网实测重连 ~257 channel 同时 needSync。K 取保守小窗（驱动连接池友好），
/// 大于 1 以保留并行度。注释凭据：HX-C011 证伪测试构造 N>K（N≫8）验证在途恒 ≤ K。
pub const MAX_INFLIGHT_SYNC: usize = 8;

/// 同一 channel 两次 pong gap 补偿的最小间隔（节流防风暴），毫秒。
///
/// 对齐现网 `sync_compensation.rs::MIN_COMPENSATE_INTERVAL = 5s`：心跳 8s/次，gap 命中后服务端
/// 下个 ping 才重新评估，5s 足够覆盖单次 sync 往返去重。pong gap 与 hashMismatch 全量根群补偿
/// **共享**此节流闸门（避免「gap 已补 + hashMismatch 又补」对同 channel 双拉）。
pub const PONG_COMPENSATE_MIN_INTERVAL_MS: u64 = 5_000;

/// SyncScheduler 全局调度态（并入 ImState，纯数据，无 I/O 句柄）。
///
/// 与 per-channel `Channel::inflight_sync`（单 channel 防自踩守卫）正交：这里管**全局**窗口
/// （跨 channel 并发上限），per-channel 守卫管单 channel 不重入。
#[derive(Debug, Default)]
pub struct SyncScheduler {
    /// 待 dispatch 的 channel 队列（FIFO，入队顺序由调用方确定性排序锚定，HX-C010）。
    pending: VecDeque<ChannelId>,
    /// 已入队去重集（防同 channel 重复 push_back；与 `pending` 同生命周期）。
    enqueued: HashSet<ChannelId>,
    /// 全局在途 sync 计数（dispatch 时 +1，回报时 -1，恒 ≤ MAX_INFLIGHT_SYNC）。
    inflight: usize,
    /// pong gap 补偿节流：channel → 上次补偿的 `now_ms`（test-and-set，防 pong 风暴）。
    /// 与 `pending`/`enqueued` 正交：那两个管「待 sync 队列」，这个管「补偿冷却」。
    /// 确定性：节流判定纯比 `now_ms`（host 注入），不读墙钟（HX-C011 / sans-IO）。
    pong_throttle: HashMap<ChannelId, u64>,
}

impl SyncScheduler {
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前全局在途 sync 数（证伪测试 / 日志读）。
    pub fn inflight(&self) -> usize {
        self.inflight
    }

    /// 当前待 dispatch 队列长度（证伪测试 / 日志读）。
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// 是否还能立即 dispatch（在途未满窗）。
    pub fn has_window(&self) -> bool {
        self.inflight < MAX_INFLIGHT_SYNC
    }

    /// 入队一个待 sync channel（已在队则幂等忽略）。返回 true=新入队 / false=已在队去重。
    ///
    /// 注：调用方负责跳过「已在途」channel（`inflight_sync.is_some()`）——本结构只防**队内**重复。
    pub fn enqueue(&mut self, channel_id: ChannelId) -> bool {
        if self.enqueued.insert(channel_id) {
            self.pending.push_back(channel_id);
            true
        } else {
            false
        }
    }

    /// 取下一个可 dispatch 的 channel（在途未满窗且队非空时）。
    ///
    /// 取出即视为「即将 dispatch」：从队列 + 去重集移除，并 `inflight += 1`（窗口占用）。
    /// 调用方拿到后负责实际 push `Effect::Http` + 登记 per-channel `inflight_sync` 守卫。
    /// 返回 None = 满窗或队空（不占窗口）。
    pub fn next_dispatch(&mut self) -> Option<ChannelId> {
        if !self.has_window() {
            return None;
        }
        let channel_id = self.pending.pop_front()?;
        self.enqueued.remove(&channel_id);
        self.inflight += 1;
        Some(channel_id)
    }

    /// 一个 sync 回报到达 → 释放 1 个全局窗口（`inflight` 饱和减，不下溢）。
    ///
    /// 与 per-channel `inflight_sync = None` 配套：那个清单 channel 守卫，这个清全局窗口。
    pub fn release_window(&mut self) {
        self.inflight = self.inflight.saturating_sub(1);
    }

    /// 直接占用 1 个全局窗口（不经队列）——续拉链（`maybe_continue_sync`）复用：同一 channel 刚在
    /// SyncPull 回报释放了窗口，续拉立即再起一次 sync，须重新占窗，保持 `inflight` = 真在途数。
    /// 续拉是 per-channel 串行链（非 fan-out 风暴源），故允许直接占窗不排队（仍计入全局上限可观测）。
    pub fn acquire_window(&mut self) {
        self.inflight += 1;
    }

    /// pong gap 补偿节流闸门（test-and-set）：返回 true=允许本次补偿（并记 `now_ms`），
    /// false=距上次补偿 < `PONG_COMPENSATE_MIN_INTERVAL_MS` 跳过。
    ///
    /// 对齐现网 `heartbeat.should_compensate`（test-and-set 语义）：判定即记账，调用方拿到
    /// true 后必然补偿。`now_ms` 由 host 注入（确定性，HX-C011 不读墙钟）。
    pub fn should_pong_compensate(&mut self, channel_id: ChannelId, now_ms: u64) -> bool {
        match self.pong_throttle.get(&channel_id) {
            Some(&last) if now_ms.saturating_sub(last) < PONG_COMPENSATE_MIN_INTERVAL_MS => false,
            _ => {
                self.pong_throttle.insert(channel_id, now_ms);
                true
            }
        }
    }

    /// 会话边界 reset（hello 起点）：清队列 + 去重集 + 在途计数 + pong 节流，防跨会话残留误调度。
    ///
    /// 与 `ImState::reset_increment_batch` 同生命周期（hello 窗口起点）：旧会话遗留的 pending
    /// channel / 悬挂在途计数 / 陈旧节流戳若不清，重连后会对错误 channel 调度或永久占满窗口。
    pub fn reset(&mut self) {
        self.pending.clear();
        self.enqueued.clear();
        self.inflight = 0;
        self.pong_throttle.clear();
    }
}

/// 入队一批待 sync channel（跳过已在途的），随后 drain 至满窗（B4 风暴闸门入口）。
///
/// 取代旧 `emit_proactive_resync_for` 的「逐 channel 直发」：先全部入队（确定性有序由调用方排序
/// 锚定，HX-C010），再 `drain` 至全局在途达 K。超 K 的 channel 留队列，等 sync 回报释放窗口续发。
///
/// `alloc_corr`：单调 corr 分配器（与 corr_map 不变量同源，见 state.rs CorrelationContext）。
pub fn enqueue_and_drain(
    state: &mut ImState,
    api_base_url: &str,
    targets: &[ChannelId],
    alloc_corr: &mut dyn FnMut() -> Correlation,
    out: &mut EffectSink,
) {
    for &channel_id in targets {
        // 跳过已在途（per-channel 守卫）/ 已注销 channel：与旧 emit_proactive_resync_for 同口径。
        match state.channels.get(&channel_id) {
            Some(ch) if ch.inflight_sync.is_some() => continue,
            Some(_) => {}
            None => continue,
        }
        state.sync_scheduler.enqueue(channel_id);
    }
    drain(state, api_base_url, alloc_corr, out);
}

/// drain：在途未满窗且队非空时，逐个取队首 dispatch `sync/notify`（每次占 1 窗口）。
///
/// 不变量：单次 drain 后全局在途 ≤ `MAX_INFLIGHT_SYNC`（`next_dispatch` 满窗即返回 None）。
/// 队首 channel 若在入队后被注销 / 已另起在途 → 跳过并归还窗口（不空转占窗）。
pub fn drain(
    state: &mut ImState,
    api_base_url: &str,
    alloc_corr: &mut dyn FnMut() -> Correlation,
    out: &mut EffectSink,
) {
    let conn_id = state.connection_id.clone();
    while let Some(channel_id) = state.sync_scheduler.next_dispatch() {
        let sync_corr = alloc_corr();
        // 队首在入队后可能被注销 / 已另起在途 → 跳过并归还本次占的窗口（避免窗口泄漏）。
        // 单次 get_mut 同时读 cursor + 写 inflight 守卫（消除二次 get 的不可达 None 防御分支）：
        // 守卫态 / 注销态 → 撤回本次 alloc 的 corr 路由 + 归还窗口；否则原子占位 + 发 sync。
        let from_seq = match state.channels.get_mut(&channel_id) {
            Some(ch) if ch.inflight_sync.is_none() => {
                ch.inflight_sync = Some(InflightSync(sync_corr));
                ch.cursor.value()
            }
            _ => {
                state.sync_scheduler.release_window();
                continue;
            }
        };
        state
            .corr_map
            .insert(sync_corr, CorrelationContext::SyncPull { channel_id });
        out.push(crate::acl::to_effect::sync_notify(
            api_base_url,
            channel_id,
            from_seq,
            sync_corr,
            conn_id.as_deref(),
        ));
    }
}

#[cfg(test)]
#[path = "sync_scheduler_tests.rs"]
mod tests;
