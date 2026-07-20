//! Surface projector 的公共下行出口。
//!
//! Projector 只把已经裁决的领域/查询事实映射为强类型 ViewModel；它不执行 HTTP、WS 或
//! storage。所有 Surface 共享这一处 envelope 编码，避免某个入口意外退回 raw projection。

pub mod timeline;
pub mod unread_summary;

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};

use crate::interaction_frame::{
    InteractionFrame, InteractionFrameError, SurfaceMutation, INTERACTION_FRAME_EVENT,
};

/// 先验证整个 mutation 集，再允许任一 driver 看见它。调用方不能以多个 `Emit` 拼出一次领域
/// 变更，因此已 attach 的 slots 要么共同提交，要么共同请求 resync。
pub(crate) fn build_interaction_frame(
    transaction_id: String,
    mutations: Vec<SurfaceMutation>,
) -> Result<InteractionFrame, InteractionFrameError> {
    let frame = InteractionFrame {
        schema_version: crate::interaction_frame::INTERACTION_FRAME_SCHEMA_VERSION,
        transaction_id,
        mutations,
    };
    frame.validate()?;
    Ok(frame)
}

/// 把已校验的 Frame 封装为单个下行 effect。
///
/// 一个 `Effect::Emit` 对应一个 Frame；FFI sink 会把该 event 原样装进同一次 callback batch。
/// Projector 不拆分 mutation，也不让 driver 解码/重编码 JSON。
pub(crate) fn emit_interaction_frame(
    frame: InteractionFrame,
) -> Result<Effect, InteractionFrameError> {
    frame.validate()?;
    let payload = serde_json::json!({
        "event": INTERACTION_FRAME_EVENT,
        "data": frame,
    });
    let bytes = serde_json::to_vec(&payload)
        .expect("InteractionFrame only contains JSON-serializable typed view models");
    Ok(Effect::Emit {
        event: DomainEventBytes(Bytes::from(bytes)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_frame::{
        DialogListVm, DialogSection, DialogSectionVm, ReplaceView, ReplaceViewKind, SurfaceId,
        SurfaceLoadState, SurfaceScope, TimelineAnchorMode, TimelineAnchorVm, TimelineWindowVm,
        TypedSurfaceMutation, UnreadSummaryVm, WindowPageVm,
    };

    #[test]
    fn dialog_timeline_unread_share_one_atomic_transaction() {
        let user_scope = SurfaceScope::User {
            user_id: "viewer-1".to_string(),
            team_id: "team-1".to_string(),
        };
        let timeline_scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        let mutations = vec![
            SurfaceMutation {
                surface: SurfaceId::MessageLeftDialogList,
                scope: user_scope.clone(),
                revision: 7,
                base_revision: Some(6),
                causation_id: Some("post:message-1".to_string()),
                mutation: TypedSurfaceMutation::DialogListPatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view: DialogListVm {
                            sections: vec![DialogSectionVm {
                                kind: DialogSection::Normal,
                                items: Vec::new(),
                                page: WindowPageVm {
                                    window_token: "dialog".to_string(),
                                    has_older: false,
                                    has_newer: false,
                                    has_more: false,
                                },
                            }],
                            state: SurfaceLoadState::Ready,
                        },
                    },
                },
            },
            SurfaceMutation {
                surface: SurfaceId::MessageUnreadSummary,
                scope: user_scope,
                revision: 4,
                base_revision: Some(3),
                causation_id: Some("post:message-1".to_string()),
                mutation: TypedSurfaceMutation::UnreadSummaryPatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view: UnreadSummaryVm {
                            unread_total: 3,
                            mention_total: 1,
                            urgent_total: 0,
                            unread_channel_count: 1,
                        },
                    },
                },
            },
            SurfaceMutation {
                surface: SurfaceId::MessageTimeline,
                scope: timeline_scope,
                revision: 2,
                base_revision: Some(1),
                causation_id: Some("post:message-1".to_string()),
                mutation: TypedSurfaceMutation::TimelinePatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view: TimelineWindowVm {
                            channel_id: "channel-1".to_string(),
                            anchor: TimelineAnchorVm {
                                message_id: Some("message-1".to_string()),
                                mode: TimelineAnchorMode::Latest,
                            },
                            items: Vec::new(),
                            first_unread_message_id: None,
                            target_message_id: None,
                            page: WindowPageVm {
                                window_token: "latest".to_string(),
                                has_older: false,
                                has_newer: false,
                                has_more: false,
                            },
                            state: SurfaceLoadState::Ready,
                        },
                    },
                },
            },
        ];
        let frame = build_interaction_frame("post:message-1".to_string(), mutations)
            .expect("all three typed slots form one valid frame");
        let effect = emit_interaction_frame(frame).expect("single batch effect");
        let helix_core::effect::Effect::Emit { event } = effect else {
            panic!("surface projector must emit a single event")
        };
        let envelope: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
        assert_eq!(envelope["event"], INTERACTION_FRAME_EVENT);
        assert_eq!(envelope["data"]["mutations"].as_array().unwrap().len(), 3);
        assert!(envelope["data"]["mutations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|mutation| mutation["causationId"] == "post:message-1"));
    }
}
