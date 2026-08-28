use helix_core::Correlation;
use std::collections::HashSet;

use crate::state::ChannelId;

/// One fan-out pong compensation is a single renderer authority boundary.
///
/// HTTP syncs and their storage receipts may complete out of order. This ledger
/// keeps only correlations, so registration and completion stay O(1) without
/// retaining message or projection payloads.
#[derive(Debug, Default)]
pub struct PongGapBatch {
    active: bool,
    pending_persists: HashSet<Correlation>,
    committed_change: bool,
    dialog_channels: HashSet<ChannelId>,
}

impl PongGapBatch {
    pub fn begin_fanout(&mut self, target_count: usize) -> bool {
        if target_count <= 1 {
            return false;
        }
        self.active = true;
        true
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn register_persist(&mut self, corr: Correlation) {
        // HTTP scheduler 可能先于 sync reply 编译 Persist 短暂变 idle；Persist 登记本身必须重申批次活跃。
        self.active = true;
        self.pending_persists.insert(corr);
    }

    pub fn finish_persist(&mut self, corr: Correlation, committed: bool) {
        if self.pending_persists.remove(&corr) && committed {
            self.committed_change = true;
        }
    }

    /// 记录本批已提交的 viewer dialog 频道，并按频道天然去重。
    pub fn record_dialog_channel(&mut self, channel_id: ChannelId) {
        if self.active {
            self.dialog_channels.insert(channel_id);
        }
    }

    /// 批次排空后一次性交出提交状态与确定序的 dialog 读回分母。
    pub fn take_completion(&mut self, scheduler_idle: bool) -> Option<(bool, Vec<ChannelId>)> {
        if !self.active || !scheduler_idle || !self.pending_persists.is_empty() {
            return None;
        }
        self.active = false;
        let committed_change = std::mem::take(&mut self.committed_change);
        let mut dialog_channels: Vec<_> = self.dialog_channels.drain().collect();
        dialog_channels.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        Some((committed_change, dialog_channels))
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.pending_persists.clear();
        self.committed_change = false;
        self.dialog_channels.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_thousand_channel_batch_closes_only_after_scheduler_and_persists_drain() {
        let mut batch = PongGapBatch::default();
        assert!(batch.begin_fanout(1_000));
        for raw in 1..=1_000 {
            batch.register_persist(Correlation::from_raw(raw));
        }

        assert_eq!(batch.take_completion(false), None);
        for raw in 1..1_000 {
            batch.finish_persist(Correlation::from_raw(raw), true);
        }
        assert_eq!(batch.take_completion(true), None);

        batch.finish_persist(Correlation::from_raw(1_000), true);
        assert_eq!(batch.take_completion(true), Some((true, Vec::new())));
        assert_eq!(batch.take_completion(true), None);
    }

    #[test]
    fn single_gap_keeps_targeted_projection_path() {
        let mut batch = PongGapBatch::default();
        assert!(!batch.begin_fanout(1));
        assert!(!batch.is_active());
    }

    #[test]
    fn fanout_deduplicates_dialog_readback_channels() {
        let mut batch = PongGapBatch::default();
        assert!(batch.begin_fanout(2));
        batch.register_persist(Correlation::from_raw(1));
        batch.register_persist(Correlation::from_raw(2));
        let channel = ChannelId::from_str("chfixx00000000000000000001").expect("channel id");
        batch.record_dialog_channel(channel);
        batch.record_dialog_channel(channel);
        batch.finish_persist(Correlation::from_raw(1), true);
        batch.finish_persist(Correlation::from_raw(2), true);

        assert_eq!(batch.take_completion(true), Some((true, vec![channel])));
    }
}
