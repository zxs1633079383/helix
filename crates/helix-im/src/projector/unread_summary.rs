//! 当前 viewer 的未读聚合投影。
//!
//! 这不是由客户端 DialogList reducer 推导出的派生状态。Helix 在获得 viewer-scoped
//! channel facts 时保存每频道贡献；首次 snapshot 为 `O(n)`，后续频道绝对值更新为 `O(1)`。

use std::collections::HashMap;

use thiserror::Error;

use crate::interaction_frame::UnreadSummaryVm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewerChannelUnread {
    pub unread_count: u32,
    pub mention_count: u32,
    pub urgent_count: u32,
}

impl ViewerChannelUnread {
    pub const fn new(unread_count: u32, mention_count: u32, urgent_count: u32) -> Self {
        Self {
            unread_count,
            mention_count,
            urgent_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadSummarySnapshot {
    pub revision: u64,
    pub view: UnreadSummaryVm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadSummaryPatch {
    pub base_revision: u64,
    pub revision: u64,
    pub view: UnreadSummaryVm,
}

/// 未读聚合 slot 无法继续推进时的 fail-closed 信号。
///
/// revision 是客户端应用 frame 的因果栅栏；回绕到 1 会使新 patch 看起来像陈旧快照，因此
/// 达到 `u64::MAX` 后必须保留既有 slot，等待宿主 reset/重新 attach。
#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum UnreadSummaryProjectionError {
    #[error("unread summary revision reached u64::MAX")]
    RevisionOverflow,
}

#[derive(Debug, Default)]
pub struct UnreadSummaryProjector {
    by_channel: HashMap<String, ViewerChannelUnread>,
    unread_total: u64,
    mention_total: u64,
    urgent_total: u64,
    unread_channel_count: u64,
    revision: u64,
    has_snapshot: bool,
}

impl UnreadSummaryProjector {
    /// 以权威 viewer-scoped channel 集合重建一次聚合。这个低频读路径为 `O(n)`。
    pub fn snapshot<I>(
        &mut self,
        channels: I,
    ) -> Result<UnreadSummarySnapshot, UnreadSummaryProjectionError>
    where
        I: IntoIterator<Item = (String, ViewerChannelUnread)>,
    {
        let revision = next_revision(self.revision)?;
        self.by_channel.clear();
        self.unread_total = 0;
        self.mention_total = 0;
        self.urgent_total = 0;
        self.unread_channel_count = 0;

        for (channel_id, contribution) in channels {
            if channel_id.is_empty() || self.by_channel.contains_key(&channel_id) {
                continue;
            }
            self.add(contribution);
            self.by_channel.insert(channel_id, contribution);
        }

        self.revision = revision;
        self.has_snapshot = true;
        Ok(UnreadSummarySnapshot {
            revision: self.revision,
            view: self.view(),
        })
    }

    /// 用频道的绝对 badge 值更新聚合。没有可见 aggregate 改变时不制造 patch revision。
    pub fn upsert(
        &mut self,
        channel_id: String,
        contribution: ViewerChannelUnread,
    ) -> Result<Option<UnreadSummaryPatch>, UnreadSummaryProjectionError> {
        if !self.has_snapshot || channel_id.is_empty() {
            return Ok(None);
        }
        if self.by_channel.get(&channel_id) == Some(&contribution) {
            return Ok(None);
        }

        // 预检必须在改动 channel 贡献前完成。overflow 时整个 aggregate slot 保持原样。
        let revision = next_revision(self.revision)?;

        let before = self.view();
        if let Some(previous) = self.by_channel.insert(channel_id, contribution) {
            self.remove(previous);
        }
        self.add(contribution);
        let view = self.view();
        if view == before {
            return Ok(None);
        }

        let base_revision = self.revision;
        self.revision = revision;
        Ok(Some(UnreadSummaryPatch {
            base_revision,
            revision: self.revision,
            view,
        }))
    }

    pub fn remove_channel(
        &mut self,
        channel_id: &str,
    ) -> Result<Option<UnreadSummaryPatch>, UnreadSummaryProjectionError> {
        if !self.has_snapshot {
            return Ok(None);
        }
        let Some(previous) = self.by_channel.get(channel_id).copied() else {
            return Ok(None);
        };

        // 与 upsert 相同，不能在确定可以推进 revision 前先删掉贡献。
        let revision = next_revision(self.revision)?;
        self.by_channel.remove(channel_id);
        let before = self.view();
        self.remove(previous);
        let view = self.view();
        if view == before {
            return Ok(None);
        }

        let base_revision = self.revision;
        self.revision = revision;
        Ok(Some(UnreadSummaryPatch {
            base_revision,
            revision: self.revision,
            view,
        }))
    }

    pub(crate) fn ensure_can_advance(&self) -> Result<(), UnreadSummaryProjectionError> {
        next_revision(self.revision).map(|_| ())
    }

    #[cfg(test)]
    pub(crate) fn set_revision_for_test(&mut self, revision: u64) {
        self.revision = revision;
    }

    pub fn snapshot_view(&self) -> Option<UnreadSummarySnapshot> {
        self.has_snapshot.then(|| UnreadSummarySnapshot {
            revision: self.revision,
            view: self.view(),
        })
    }

    fn add(&mut self, contribution: ViewerChannelUnread) {
        self.unread_total = self
            .unread_total
            .saturating_add(u64::from(contribution.unread_count));
        self.mention_total = self
            .mention_total
            .saturating_add(u64::from(contribution.mention_count));
        self.urgent_total = self
            .urgent_total
            .saturating_add(u64::from(contribution.urgent_count));
        if contribution.unread_count > 0 {
            self.unread_channel_count = self.unread_channel_count.saturating_add(1);
        }
    }

    fn remove(&mut self, contribution: ViewerChannelUnread) {
        self.unread_total = self
            .unread_total
            .saturating_sub(u64::from(contribution.unread_count));
        self.mention_total = self
            .mention_total
            .saturating_sub(u64::from(contribution.mention_count));
        self.urgent_total = self
            .urgent_total
            .saturating_sub(u64::from(contribution.urgent_count));
        if contribution.unread_count > 0 {
            self.unread_channel_count = self.unread_channel_count.saturating_sub(1);
        }
    }

    fn view(&self) -> UnreadSummaryVm {
        UnreadSummaryVm {
            unread_total: saturating_u32(self.unread_total),
            mention_total: saturating_u32(self.mention_total),
            urgent_total: saturating_u32(self.urgent_total),
            unread_channel_count: saturating_u32(self.unread_channel_count),
        }
    }
}

fn next_revision(current: u64) -> Result<u64, UnreadSummaryProjectionError> {
    current
        .checked_add(1)
        .ok_or(UnreadSummaryProjectionError::RevisionOverflow)
}

fn saturating_u32(value: u64) -> u32 {
    value.min(u64::from(u32::MAX)) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_then_absolute_channel_update_is_o1_aggregate_patch() {
        let mut projector = UnreadSummaryProjector::default();
        let snapshot = projector
            .snapshot([
                ("a".to_string(), ViewerChannelUnread::new(3, 1, 0)),
                ("b".to_string(), ViewerChannelUnread::new(0, 0, 2)),
            ])
            .expect("initial snapshot");
        assert_eq!(snapshot.revision, 1);
        assert_eq!(snapshot.view.unread_total, 3);
        assert_eq!(snapshot.view.unread_channel_count, 1);

        let patch = projector
            .upsert("a".to_string(), ViewerChannelUnread::new(4, 1, 1))
            .expect("revision is available")
            .expect("absolute change must advance the aggregate once");
        assert_eq!(patch.base_revision, 1);
        assert_eq!(patch.revision, 2);
        assert_eq!(patch.view.unread_total, 4);
        assert_eq!(patch.view.mention_total, 1);
        assert_eq!(patch.view.urgent_total, 3);
        assert_eq!(patch.view.unread_channel_count, 1);
    }

    #[test]
    fn overflow_keeps_the_aggregate_slot_unchanged() {
        let mut projector = UnreadSummaryProjector::default();
        projector
            .snapshot([("a".to_string(), ViewerChannelUnread::new(3, 1, 0))])
            .unwrap();
        projector.revision = u64::MAX;
        let before = projector.snapshot_view();
        let before_contribution = projector.by_channel.clone();

        assert_eq!(
            projector.upsert("a".to_string(), ViewerChannelUnread::new(4, 1, 0)),
            Err(UnreadSummaryProjectionError::RevisionOverflow)
        );
        assert_eq!(projector.snapshot_view(), before);
        assert_eq!(projector.by_channel, before_contribution);

        assert_eq!(
            projector.snapshot([("b".to_string(), ViewerChannelUnread::new(9, 0, 0))]),
            Err(UnreadSummaryProjectionError::RevisionOverflow)
        );
        assert_eq!(projector.snapshot_view(), before);
        assert_eq!(projector.by_channel, before_contribution);
    }
}
