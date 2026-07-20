//! `message.participant-receipts` 的 typed attach/query outlet。
//!
//! read/template/urgent 三类回执使用不同 scope。权限不足时 Helix 明确发布 `forbidden` 空视图；
//! 端侧不能把空的 raw reader list 猜成“所有人未完成”。

use std::collections::BTreeMap;

use thiserror::Error;

pub use crate::interaction_frame::ParticipantReceiptsVm;
use crate::interaction_frame::{
    ParticipantReceiptRowVm, ReceiptKind, SurfaceDataVm, SurfaceId, SurfaceLoadState,
    SurfaceMutation, SurfaceScope, TypedSurfaceMutation, WindowPageVm,
};

use super::{SurfaceProjectorError, SurfaceView, TypedSurfaceProjector};

/// 每个 message + receipt kind 窗口的最大可见回执数。
pub const MAX_PARTICIPANT_RECEIPT_ROWS: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantReceiptsRequest {
    pub channel_id: String,
    pub message_id: String,
    pub kind: ReceiptKind,
    pub cursor: String,
}

impl ParticipantReceiptsRequest {
    pub fn new(
        channel_id: impl Into<String>,
        message_id: impl Into<String>,
        kind: ReceiptKind,
        cursor: impl Into<String>,
    ) -> Self {
        Self {
            channel_id: channel_id.into(),
            message_id: message_id.into(),
            kind,
            cursor: cursor.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticipantReceiptsData {
    pub summary: SurfaceDataVm,
    pub completed: Vec<ParticipantReceiptRowVm>,
    pub pending: Vec<ParticipantReceiptRowVm>,
    pub state: SurfaceLoadState,
}

impl ParticipantReceiptsData {
    pub fn forbidden() -> Self {
        Self {
            summary: SurfaceDataVm::Record(BTreeMap::new()),
            completed: Vec::new(),
            pending: Vec::new(),
            state: SurfaceLoadState::Forbidden,
        }
    }
}

impl SurfaceView for ParticipantReceiptsVm {
    const SURFACE: SurfaceId = SurfaceId::MessageParticipantReceipts;

    fn snapshot(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::ParticipantReceiptsSnapshot { view }
    }

    fn patch(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::ParticipantReceiptsPatch {
            patch: crate::interaction_frame::ReplaceView {
                kind: crate::interaction_frame::ReplaceViewKind::ReplaceView,
                view,
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct ParticipantReceiptsProjector {
    slots: TypedSurfaceProjector<ParticipantReceiptsVm>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParticipantReceiptsProjectionError {
    #[error("participant receipt window has {actual} rows; maximum is {maximum}")]
    RowLimit { actual: usize, maximum: usize },
    #[error(transparent)]
    Slot(#[from] SurfaceProjectorError),
}

impl ParticipantReceiptsProjector {
    /// attach/query 的完整回执快照。`cursor` 只存在于 VM page，不与其他 kind 共用 slot。
    pub fn snapshot(
        &mut self,
        request: ParticipantReceiptsRequest,
        data: ParticipantReceiptsData,
    ) -> Result<SurfaceMutation, ParticipantReceiptsProjectionError> {
        ensure_bounded(&data)?;
        let view = view_for(&request, data);
        self.slots
            .snapshot(scope_for(&request), view)
            .map_err(Into::into)
    }

    /// 权威 read/template/urgent 回执变化后的完整 replaceView patch。
    pub fn patch(
        &mut self,
        request: ParticipantReceiptsRequest,
        data: ParticipantReceiptsData,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, ParticipantReceiptsProjectionError> {
        ensure_bounded(&data)?;
        let view = view_for(&request, data);
        self.slots
            .patch(scope_for(&request), view, causation_id)
            .map_err(Into::into)
    }

    /// 权限不足是显式的 surface state，而不是把可见成员集合置空来暗示成功。
    pub fn snapshot_forbidden(
        &mut self,
        request: ParticipantReceiptsRequest,
    ) -> Result<SurfaceMutation, ParticipantReceiptsProjectionError> {
        self.snapshot(request, ParticipantReceiptsData::forbidden())
    }

    pub fn current_view(
        &self,
        channel_id: &str,
        message_id: &str,
        kind: ReceiptKind,
    ) -> Option<&ParticipantReceiptsVm> {
        self.slots.current_view(&SurfaceScope::ParticipantReceipts {
            channel_id: channel_id.to_string(),
            message_id: message_id.to_string(),
            receipt_kind: kind,
        })
    }

    pub fn reset(&mut self) {
        self.slots.reset();
    }
}

fn scope_for(request: &ParticipantReceiptsRequest) -> SurfaceScope {
    SurfaceScope::ParticipantReceipts {
        channel_id: request.channel_id.clone(),
        message_id: request.message_id.clone(),
        receipt_kind: request.kind,
    }
}

fn view_for(
    request: &ParticipantReceiptsRequest,
    data: ParticipantReceiptsData,
) -> ParticipantReceiptsVm {
    ParticipantReceiptsVm {
        channel_id: request.channel_id.clone(),
        message_id: request.message_id.clone(),
        kind: request.kind,
        summary: data.summary,
        completed: data.completed,
        pending: data.pending,
        page: WindowPageVm {
            window_token: request.cursor.clone(),
            has_older: false,
            has_newer: false,
            has_more: false,
        },
        state: data.state,
    }
}

fn ensure_bounded(
    data: &ParticipantReceiptsData,
) -> Result<(), ParticipantReceiptsProjectionError> {
    let actual = data.completed.len().saturating_add(data.pending.len());
    if actual > MAX_PARTICIPANT_RECEIPT_ROWS {
        return Err(ParticipantReceiptsProjectionError::RowLimit {
            actual,
            maximum: MAX_PARTICIPANT_RECEIPT_ROWS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_frame::{
        ChannelRole, ParticipantReceiptRowVm, ReceiptStatus, TypedSurfaceMutation, UserRefVm,
    };

    fn request(kind: ReceiptKind) -> ParticipantReceiptsRequest {
        ParticipantReceiptsRequest::new("channel-1", "message-1", kind, "cursor-1")
    }

    fn data() -> ParticipantReceiptsData {
        ParticipantReceiptsData {
            summary: SurfaceDataVm::Record(BTreeMap::new()),
            completed: vec![ParticipantReceiptRowVm {
                user: UserRefVm {
                    user_id: "user-1".to_string(),
                },
                member_role: ChannelRole::Member,
                status: ReceiptStatus::Completed,
                completed_at: Some(1),
            }],
            pending: vec![ParticipantReceiptRowVm {
                user: UserRefVm {
                    user_id: "user-2".to_string(),
                },
                member_role: ChannelRole::Member,
                status: ReceiptStatus::Pending,
                completed_at: None,
            }],
            state: SurfaceLoadState::Ready,
        }
    }

    #[test]
    fn kind_scoped_receipt_patch_is_replace_view() {
        let mut projector = ParticipantReceiptsProjector::default();
        projector
            .snapshot(request(ReceiptKind::Read), data())
            .unwrap();
        let patch = projector
            .patch(
                request(ReceiptKind::Read),
                data(),
                Some("mark-read:request-1".to_string()),
            )
            .unwrap();
        assert_eq!(patch.base_revision, Some(1));
        assert!(matches!(
            patch.mutation,
            TypedSurfaceMutation::ParticipantReceiptsPatch { .. }
        ));
        assert!(projector
            .current_view("channel-1", "message-1", ReceiptKind::Read)
            .is_some());
        assert!(projector
            .current_view("channel-1", "message-1", ReceiptKind::UrgentConfirmation)
            .is_none());
    }

    #[test]
    fn forbidden_receipts_are_an_explicit_typed_snapshot() {
        let mut projector = ParticipantReceiptsProjector::default();
        let snapshot = projector
            .snapshot_forbidden(request(ReceiptKind::TemplateConfirmation))
            .unwrap();
        let TypedSurfaceMutation::ParticipantReceiptsSnapshot { view } = snapshot.mutation else {
            panic!("receipt snapshot")
        };
        assert_eq!(view.state, SurfaceLoadState::Forbidden);
        assert!(view.completed.is_empty());
        assert!(view.pending.is_empty());
    }
}
