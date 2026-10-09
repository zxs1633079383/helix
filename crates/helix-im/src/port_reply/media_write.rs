//! 按消息串行化媒体 props 写入；I/O 仍由 driver 执行，只有持久回执释放下一写。
use crate::module::ImModule;
use crate::send::upload_props::PendingMediaOp;
use crate::state::TemporaryId;
use crate::ImError;
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, EffectSink};

impl ImModule {
    /// 为实际发出的进度、完成或失败事务登记独占写屏障。
    pub(super) fn track_media_write(&mut self, id: &TemporaryId, corr: Correlation) {
        self.state
            .media_write_gates
            .entry(id.clone())
            .or_default()
            .owner = Some(corr);
        self.state.media_write_owners.insert(corr, id.clone());
    }

    /// 从已有路由取得媒体身份，不消费 correlation，也不扫描其他消息。
    fn media_reply_identity(&self, corr: Correlation) -> Option<TemporaryId> {
        let op = self
            .state
            .pending_media_ops
            .get(&corr)
            .or_else(|| self.state.pending_media_stage_persists.get(&corr));
        op.map(|op| match op {
            PendingMediaOp::Prepare(p) => p.temporary_id.clone(),
            PendingMediaOp::Put(p) | PendingMediaOp::Complete(p) => p.temporary_id.clone(),
        })
        .or_else(|| {
            self.state
                .pending_uploads
                .get(&corr)
                .map(|p| p.temporary_id.clone())
        })
    }

    /// 有 props 写在途时暂停同消息媒体终态；重复回包仅保留第一次。
    pub(crate) fn handle_port_reply(
        &mut self,
        corr: Correlation,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let owner = self.state.media_write_owners.remove(&corr);
        let identity = owner.clone().or_else(|| self.media_reply_identity(corr));
        if let Some(id) = &identity {
            if let Some(gate) = self.state.media_write_gates.get_mut(id) {
                if owner.is_some() {
                    gate.owner = None;
                } else if gate.owner.is_some() {
                    if gate.reply_keys.insert(corr) {
                        gate.replies.push_back((corr, outcome.clone(), now_ms));
                    }
                    return Ok(());
                }
            }
        }
        let result = self.dispatch_port_reply(corr, outcome, now_ms, out);
        if let Some(id) = identity {
            self.drain_media_writes(&id, out)?;
        }
        result
    }

    /// 每目标最多排队一次，最新百分比保存在既有 progress state。
    pub(super) fn enqueue_media_progress(&mut self, id: &TemporaryId, corr: Correlation) {
        let gate = self.state.media_write_gates.entry(id.clone()).or_default();
        if gate.progress_keys.insert(corr) {
            gate.progress.push_back(corr);
        }
    }

    /// 终态优先，丢弃已结束目标的过时进度；一旦下一事务发出即停止 drain。
    pub(super) fn drain_media_writes(
        &mut self,
        id: &TemporaryId,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        loop {
            let Some(gate) = self.state.media_write_gates.get_mut(id) else {
                return Ok(());
            };
            if gate.owner.is_some() {
                return Ok(());
            }
            if let Some((corr, outcome, now_ms)) = gate.replies.pop_front() {
                gate.reply_keys.remove(&corr);
                self.dispatch_port_reply(corr, &outcome, now_ms, out)?;
                continue;
            }
            if let Some(corr) = gate.progress.pop_front() {
                gate.progress_keys.remove(&corr);
                let percent = self
                    .state
                    .file_upload_progress
                    .get_mut(&corr)
                    .and_then(|state| state.queued_latest.take());
                if let Some(percent) = percent {
                    self.persist_file_upload_progress(corr, percent, out)?;
                }
                continue;
            }
            self.state.media_write_gates.remove(id);
            return Ok(());
        }
    }
}
