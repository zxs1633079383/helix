//! Correlated persistence commit and too-long reset.

use super::{event_to_upsert_op, Channel};
use crate::error::ImError;
use crate::state::{Cursor, Seq};
use helix_core::effect::StorageOp;
use helix_core::EffectSink;

impl Channel {
    /// peek_up_to（sync 权威路径，只读，不推 cursor）
    ///
    /// 返回 ≤ target_seq 的全部事件的落库 ops（来自 buffer），
    /// 不修改 cursor，不弹出 buffer（peek = 只读预览）。
    ///
    /// 外层对这批 ops 吐 Persist{corr, ops}，PortReply Ok 后才调 commit_up_to。
    pub fn peek_up_to(&self, target_seq: Seq) -> Vec<StorageOp> {
        self.buffer
            .range(..=target_seq)
            .map(|(_, ev)| event_to_upsert_op(ev))
            .collect()
    }

    /// commit_cursor：在相关 `PersistAtomic` 成功后推进内存 cursor
    ///
    /// 仅在 PortReply Ok 后调用（写成功才推原则）。
    pub(super) fn commit_cursor(&mut self, target_seq: Seq) -> Result<(), ImError> {
        if self.cursor.try_advance(target_seq) {
            // 清除 buffer 中 ≤ target_seq 的已处理事件（sync 已成功 commit）
            self.buffer.retain(|&seq, _| seq > target_seq);
        }
        Ok(())
    }

    /// too_long 处理：清内存 buffer + cursor 复位 + gate 清除
    ///
    /// 本方法只处理内存状态；调用方负责 reset dialog storage ops 与
    /// Emit(im:sync:too_long)。message 历史保留，后续 reload 以 upsert 覆盖最终态。
    pub fn reset_for_too_long(&mut self, reset_to: Seq, fx: &mut EffectSink) {
        self.buffer.clear();
        self.gate = None;
        // cursor 复位到 reset_to-1（下次 sync 从 reset_to 开始）
        let reset_cursor = Seq(reset_to.0.saturating_sub(1));
        self.cursor = Cursor::new(reset_cursor);
        // PersistFire advance_cursor（too_long 场景 cursor 值降低，monotonic_upsert
        // 的 MAX guard 不会实际写入，但显式调用让意图清晰）
        fx.push(crate::acl::to_effect::advance_cursor(self.id, reset_cursor));
    }
}
