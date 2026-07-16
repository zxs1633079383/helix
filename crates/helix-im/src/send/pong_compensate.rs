//! UC-4.4 心跳 pong gap 补偿——**消费侧**（接管 vs 现网的核心差异点）。
//!
//! ## 为什么（Round-3 偏差修复）
//!
//! 心跳 ping piggyback `{cursors, allHash}`（产侧 = `acl::to_effect::ping_frame`），服务端
//! `ReconcileCursors` 比对本端权威水位后在 **pong** 回 `{gaps[], hashMismatch}`。现网 cses-client
//! 走 `ack_observer`（pong 是 `status==OK` ACK 被 runtime 吞，不进 handler）→
//! `handle_pong_compensation` → `sync_compensation::compensate_from_pong`（逐 channel 补偿 + 5s 节流）。
//!
//! helix 此前**只有产侧无消费侧**：ping piggyback 正确发出，但 pong 回的 `{gaps, hashMismatch}`
//! 全仓无解析 / 无路由 → 在线漏推 / 非活跃群 drift **永不补齐**。本模块补齐消费侧：
//!   - pong 帧（无 `action`、`status==OK` + 带 `gaps`/`hashMismatch`）由 `WsFrame::action()` 合成
//!     `__pong__` action → `ws::handlers::pong` 路由到本模块；
//!   - `gaps[].channelId` 去重 + 5s 节流（`SyncScheduler::should_pong_compensate`）逐个补偿；
//!   - `hashMismatch==true` → 全量已知根群（`state.channels` 键）对账升级补偿（共享同节流闸门）；
//!   - 补偿复用 `sync_scheduler::enqueue_and_drain`（全局并发窗口 + per-channel 防自踩守卫，
//!     与重连 sync 风暴闸同一入口）。
//!
//! ## 不变量
//!
//! - **确定性有序**（HX-C010）：hashMismatch 全量根群补偿前对 channel 集合 `sort_unstable`，
//!   杜绝 HashMap SipHash 随机迭代序导致非确定 Effect 序。
//! - **节流去重**：同 channel 短期内多 pong 携同 gap 只补一次（5s 闸门，防 pong 风暴）。
//! - **零信任**：pong `data` 任意字段缺/坏 → 安全回退空（不 panic / 不误补），HX-C001 边界铁律。
//! - **sans-IO**：纯算定 `Effect`（sync HTTP 由 driver 发），不读墙钟（`now_ms` host 注入）。

use crate::state::{ChannelId, ImState};
use helix_core::{Correlation, EffectSink};

/// 从 pong `data` 抽 `gaps[].channelId`（去重 + 零信任，非法/缺 → 空）。
///
/// 现网 wire：`data.gaps = [{channelId, fromSeq, maxSeq}, ...]`（camelCase）。只取 `channelId`，
/// 非 26 字符 Id26 / 空 → 跳过（不补不存在的 channel）。
pub fn parse_gap_channels(data: &serde_json::Value) -> Vec<ChannelId> {
    let Some(arr) = data.get("gaps").and_then(serde_json::Value::as_array) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for g in arr {
        if let Some(id) = g
            .get("channelId")
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        {
            if seen.insert(id) {
                out.push(id);
            }
        }
    }
    out
}

/// 从 pong `data` 取 `hashMismatch`（缺/非 bool → false，零信任）。
pub fn parse_hash_mismatch(data: &serde_json::Value) -> bool {
    data.get("hashMismatch")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// 处理 pong 回带的 gap 补偿（消费侧主入口）。
///
/// - `gaps[].channelId`：去重后逐个过 5s 节流闸门 → 命中的入队 `enqueue_and_drain` 补偿 sync。
/// - `hashMismatch==true`：对全量已知根群（`state.channels` 键，**确定性升序**）逐个过同一节流
///   闸门 → 命中的一并入队补偿（与 gap 共享闸门避免对同 channel 双拉）。
///
/// 复用 `sync_scheduler::enqueue_and_drain`：自带「已在途 channel 跳过 + 全局并发窗口 K」守卫，
/// 与重连风暴闸同口径——pong 补偿不会绕过窗口打爆 driver。`now_ms` 由 host 注入（确定性）。
pub fn compensate_from_pong(
    state: &mut ImState,
    api_base_url: &str,
    now_ms: u64,
    gap_channels: &[ChannelId],
    hash_mismatch: bool,
    alloc_corr: &mut dyn FnMut() -> Correlation,
    out: &mut EffectSink,
) {
    // 收集本次「过了节流闸门」的待补偿 channel（test-and-set 即记账，确定性有序）。
    let mut targets: Vec<ChannelId> = Vec::new();

    // 1) 活跃群 gap：去重已在 parse 阶段做，这里只过节流。
    for &ch in gap_channels {
        if state.sync_scheduler.should_pong_compensate(ch, now_ms) {
            targets.push(ch);
        }
    }

    // 2) hashMismatch 全量根群对账升级：取已知 channel 键，确定性升序（HX-C010），过同一闸门。
    if hash_mismatch {
        let mut roots: Vec<ChannelId> = state.channels.keys().copied().collect();
        roots.sort_unstable();
        for ch in roots {
            // 与 gap 共享 should_pong_compensate：gap 已补的 channel 此处闸门命中跳过（防双拉）。
            if state.sync_scheduler.should_pong_compensate(ch, now_ms) {
                targets.push(ch);
            }
        }
    }

    if targets.is_empty() {
        return;
    }

    // targets 已确定性有序（gap 保持 wire 顺序 + roots 升序），enqueue_and_drain 内部跳过
    // 已在途 / 不存在 channel，并经全局窗口 K 限流 dispatch sync/notify。
    crate::sync_scheduler::enqueue_and_drain_with_trigger(
        state,
        api_base_url,
        &targets,
        crate::state::SyncTrigger::PongGap,
        alloc_corr,
        out,
    );
}

#[cfg(test)]
#[path = "pong_compensate_tests.rs"]
mod tests;
