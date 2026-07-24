use helix_core::Correlation;
use std::collections::HashSet;

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
        if self.active {
            self.pending_persists.insert(corr);
        }
    }

    pub fn finish_persist(&mut self, corr: Correlation, committed: bool) {
        if self.pending_persists.remove(&corr) && committed {
            self.committed_change = true;
        }
    }

    pub fn take_completion(&mut self, scheduler_idle: bool) -> Option<bool> {
        if !self.active || !scheduler_idle || !self.pending_persists.is_empty() {
            return None;
        }
        self.active = false;
        let committed_change = std::mem::take(&mut self.committed_change);
        Some(committed_change)
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.pending_persists.clear();
        self.committed_change = false;
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
        assert_eq!(batch.take_completion(true), Some(true));
        assert_eq!(batch.take_completion(true), None);
    }

    #[test]
    fn single_gap_keeps_targeted_projection_path() {
        let mut batch = PongGapBatch::default();
        assert!(!batch.begin_fanout(1));
        assert!(!batch.is_active());
    }
}
