//! `message.participant-receipts` 的 typed attach/query outlet。
//!
//! read/template/urgent 三类回执使用不同 scope。权限不足时 Helix 明确发布 `forbidden` 空视图；
//! 端侧不能把空的 raw reader list 猜成“所有人未完成”。

use std::collections::BTreeMap;

use thiserror::Error;

pub use crate::interaction_frame::ParticipantReceiptsVm;
use crate::interaction_frame::{
    ChannelRole, ParticipantReceiptRowVm, ReceiptKind, ReceiptStatus, SurfaceDataVm, SurfaceId,
    SurfaceLoadState, SurfaceMutation, SurfaceScope, TypedSurfaceMutation, UserRefVm, WindowPageVm,
};
use serde_json::Value;

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
    /// 构造权限不足的显式空视图，禁止端侧把空集合解释成业务成功。
    pub fn forbidden() -> Self {
        Self {
            summary: SurfaceDataVm::Record(BTreeMap::new()),
            completed: Vec::new(),
            pending: Vec::new(),
            state: SurfaceLoadState::Forbidden,
        }
    }

    /// 从持久 message.expedite_map 建立加急确认的权威 completed/pending 视图。
    pub(crate) fn from_urgent_message_row(row: Option<&Value>) -> Self {
        let Some(row) = row else {
            return receipt_state(
                "failed",
                "urgent message row is unavailable",
                SurfaceLoadState::Failed,
            );
        };
        let expedite_map = row
            .get("expedite_map")
            .or_else(|| row.get("expediteMap"))
            .and_then(|value| match value {
                Value::String(raw) => serde_json::from_str::<Value>(raw).ok(),
                Value::Object(_) => Some(value.clone()),
                _ => None,
            });
        let Some(expedite_map) = expedite_map else {
            return receipt_state(
                "failed",
                "urgent authority is unavailable",
                SurfaceLoadState::Failed,
            );
        };
        let requester_id = expedite_map
            .get("sender")
            .and_then(Value::as_object)
            .and_then(|sender| sender.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .unwrap_or_default();
        let Some(recipients) = expedite_map.get("recipients").and_then(Value::as_object) else {
            return receipt_state(
                "failed",
                "urgent recipients are unavailable",
                SurfaceLoadState::Failed,
            );
        };
        if recipients.len() > MAX_PARTICIPANT_RECEIPT_ROWS {
            return receipt_state(
                "failed",
                "urgent receipt window exceeds the bounded surface contract",
                SurfaceLoadState::Failed,
            );
        }
        let mut completed = Vec::with_capacity(recipients.len());
        let mut pending = Vec::with_capacity(recipients.len());
        for (user_id, recipient) in recipients {
            if user_id.is_empty() {
                return receipt_state(
                    "failed",
                    "urgent recipients contain an empty user id",
                    SurfaceLoadState::Failed,
                );
            }
            let is_completed = recipient
                .get("status")
                .and_then(Value::as_i64)
                .unwrap_or_default()
                > 0;
            let row = ParticipantReceiptRowVm {
                user: UserRefVm {
                    user_id: user_id.clone(),
                },
                member_role: ChannelRole::Member,
                status: if is_completed {
                    ReceiptStatus::Completed
                } else {
                    ReceiptStatus::Pending
                },
                completed_at: if is_completed {
                    recipient
                        .get("confirmedAt")
                        .or_else(|| recipient.get("confirmAt"))
                        .or_else(|| recipient.get("updateAt"))
                        .and_then(Value::as_i64)
                } else {
                    None
                },
            };
            if is_completed {
                completed.push(row);
            } else {
                pending.push(row);
            }
        }
        let mut summary = BTreeMap::new();
        summary.insert(
            "state".to_string(),
            SurfaceDataVm::String(if pending.is_empty() {
                "acknowledged".to_string()
            } else {
                "active".to_string()
            }),
        );
        summary.insert(
            "requesterId".to_string(),
            SurfaceDataVm::String(requester_id.to_string()),
        );
        summary.insert(
            "requiredCount".to_string(),
            SurfaceDataVm::Number(serde_json::Number::from(recipients.len() as u64)),
        );
        summary.insert(
            "completedCount".to_string(),
            SurfaceDataVm::Number(serde_json::Number::from(completed.len() as u64)),
        );
        Self {
            summary: SurfaceDataVm::Record(summary),
            completed,
            pending,
            state: SurfaceLoadState::Ready,
        }
    }
}

/// 构造加急回执解析失败时的显式 surface state。
fn receipt_state(
    state: &str,
    reason: &str,
    load_state: SurfaceLoadState,
) -> ParticipantReceiptsData {
    let mut summary = BTreeMap::new();
    summary.insert(
        "state".to_string(),
        SurfaceDataVm::String(state.to_string()),
    );
    summary.insert(
        "reason".to_string(),
        SurfaceDataVm::String(reason.to_string()),
    );
    ParticipantReceiptsData {
        summary: SurfaceDataVm::Record(summary),
        completed: Vec::new(),
        pending: Vec::new(),
        state: load_state,
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
    #[error(
        "template confirmation authority unavailable; only an explicit forbidden state is valid"
    )]
    TemplateConfirmationAuthorityUnavailable,
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
        self.snapshot_with_causation(request, data, None)
    }

    /// action 触发的首次加载仍以完整 snapshot 发布，但将 invoke requestId 作为 causation 写入
    /// 权威 Frame；renderer 只能在该 Frame 原子 apply 后完成 pending action。
    pub fn snapshot_with_causation(
        &mut self,
        request: ParticipantReceiptsRequest,
        data: ParticipantReceiptsData,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, ParticipantReceiptsProjectionError> {
        ensure_authority(&request, &data)?;
        ensure_bounded(&data)?;
        let view = view_for(&request, data);
        self.slots
            .snapshot_with_causation(scope_for(&request), view, causation_id)
            .map_err(Into::into)
    }

    /// 权威 read/template/urgent 回执变化后的完整 replaceView patch。
    pub fn patch(
        &mut self,
        request: ParticipantReceiptsRequest,
        data: ParticipantReceiptsData,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, ParticipantReceiptsProjectionError> {
        ensure_authority(&request, &data)?;
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

fn ensure_authority(
    request: &ParticipantReceiptsRequest,
    data: &ParticipantReceiptsData,
) -> Result<(), ParticipantReceiptsProjectionError> {
    if request.kind == ReceiptKind::TemplateConfirmation
        && data.state != SurfaceLoadState::Forbidden
    {
        return Err(ParticipantReceiptsProjectionError::TemplateConfirmationAuthorityUnavailable);
    }
    Ok(())
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

    #[test]
    fn read_receipts_can_preserve_completed_status_when_the_authority_omits_timestamp() {
        let mut projector = ParticipantReceiptsProjector::default();
        let mut read_data = data();
        read_data.completed[0].completed_at = None;
        projector
            .snapshot(request(ReceiptKind::Read), read_data)
            .expect(
                "post/read/list proves read state but does not expose a per-user read timestamp",
            );

        let mut template_data = data();
        template_data.completed[0].completed_at = None;
        assert!(projector
            .snapshot(request(ReceiptKind::TemplateConfirmation), template_data)
            .is_err());
    }
}
