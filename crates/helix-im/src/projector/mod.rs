//! Surface projector 的公共下行出口。
//!
//! Projector 只把已经裁决的领域/查询事实映射为强类型 ViewModel；它不执行 HTTP、WS 或
//! storage。所有 Surface 共享这一处 envelope 编码，避免某个入口意外退回 raw projection。

use std::collections::BTreeMap;

pub mod action;
pub mod channel_summary;
pub mod composer;
pub mod member_roster;
pub mod participant_receipts;
pub mod thread_detail;
pub mod timeline;
pub mod topic_list;
pub mod unread_summary;

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use thiserror::Error;

use crate::interaction_frame::{
    InteractionFrame, InteractionFrameError, InteractionFrameV2, SurfaceId, SurfaceMutation, SurfaceScope,
    TypedSurfaceMutation, INTERACTION_FRAME_EVENT,
};

/// 各强类型 ViewModel 到 closed mutation 的唯一映射。
///
/// 这个 trait 故意不暴露 raw JSON 或细粒度 patch。所有具体 surface 只提供完整快照和
/// `replaceView` 更新，客户端不会获得需要自行解释的领域 delta。
pub(crate) trait SurfaceView: Clone {
    const SURFACE: SurfaceId;

    fn snapshot(view: Self) -> TypedSurfaceMutation;
    fn patch(view: Self) -> TypedSurfaceMutation;
}

#[derive(Debug, Clone)]
struct SurfaceSlot<V> {
    revision: u64,
    view: V,
}

/// attached surface 的确定性 revision/VM 账本。
///
/// 它是纯内存 projection helper：attach/query 调用 [`Self::snapshot`]，领域事实完成后调用
/// [`Self::patch`]。两个入口都会先执行 InteractionFrame 的 scope/VM 校验，失败不写 slot。
#[derive(Debug)]
pub(crate) struct TypedSurfaceProjector<V: SurfaceView> {
    slots: BTreeMap<String, SurfaceSlot<V>>,
}

impl<V: SurfaceView> Default for TypedSurfaceProjector<V> {
    fn default() -> Self {
        Self {
            slots: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SurfaceProjectorError {
    #[error("surface {surface} requires an existing snapshot for slot {slot}")]
    MissingSnapshot { surface: &'static str, slot: String },
    #[error("surface {surface} revision reached u64::MAX")]
    RevisionOverflow { surface: &'static str },
    #[error(transparent)]
    Invalid(#[from] InteractionFrameError),
}

impl<V: SurfaceView> TypedSurfaceProjector<V> {
    pub(crate) fn reset(&mut self) {
        self.slots.clear();
    }

    pub(crate) fn snapshot(
        &mut self,
        scope: SurfaceScope,
        view: V,
    ) -> Result<SurfaceMutation, SurfaceProjectorError> {
        self.snapshot_with_causation(scope, view, None)
    }

    /// 有些权威 HTTP 回包首次建立一个 surface 时也必须携带 action causation。这个仍是完整
    /// snapshot，而不是把 receipt/accepted 当作 VM 完成信号。
    pub(crate) fn snapshot_with_causation(
        &mut self,
        scope: SurfaceScope,
        view: V,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, SurfaceProjectorError> {
        let slot_key = scope.canonical_key();
        let revision = self
            .slots
            .get(&slot_key)
            .map_or(Ok(1), |slot| next_revision(slot.revision, V::SURFACE))?;
        let mutation = SurfaceMutation {
            surface: V::SURFACE,
            scope,
            revision,
            base_revision: None,
            causation_id,
            mutation: V::snapshot(view.clone()),
        };
        mutation.validate()?;
        self.slots.insert(slot_key, SurfaceSlot { revision, view });
        Ok(mutation)
    }

    pub(crate) fn patch(
        &mut self,
        scope: SurfaceScope,
        view: V,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, SurfaceProjectorError> {
        let slot_key = scope.canonical_key();
        let Some(slot) = self.slots.get(&slot_key) else {
            return Err(SurfaceProjectorError::MissingSnapshot {
                surface: V::SURFACE.as_str(),
                slot: slot_key,
            });
        };
        let base_revision = slot.revision;
        let revision = next_revision(base_revision, V::SURFACE)?;
        let mutation = SurfaceMutation {
            surface: V::SURFACE,
            scope,
            revision,
            base_revision: Some(base_revision),
            causation_id,
            mutation: V::patch(view.clone()),
        };
        mutation.validate()?;
        self.slots.insert(slot_key, SurfaceSlot { revision, view });
        Ok(mutation)
    }

    pub(crate) fn current_view(&self, scope: &SurfaceScope) -> Option<&V> {
        self.slots
            .get(&scope.canonical_key())
            .map(|slot| &slot.view)
    }

    pub(crate) fn current_revision(&self, scope: &SurfaceScope) -> Option<u64> {
        self.slots
            .get(&scope.canonical_key())
            .map(|slot| slot.revision)
    }
}

fn next_revision(current: u64, surface: SurfaceId) -> Result<u64, SurfaceProjectorError> {
    current
        .checked_add(1)
        .ok_or(SurfaceProjectorError::RevisionOverflow {
            surface: surface.as_str(),
        })
}

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

/// V2 has a distinct producer until the raw V1 producer is separately removed.
/// This encoder still emits one atomic DomainEvent and never exposes individual
/// surface mutations to a driver.
pub(crate) fn emit_interaction_frame_v2(
    frame: InteractionFrameV2,
) -> Result<Effect, InteractionFrameError> {
    frame.validate()?;
    let payload = serde_json::json!({
        "event": INTERACTION_FRAME_EVENT,
        "data": frame,
    });
    let bytes = serde_json::to_vec(&payload)
        .expect("InteractionFrameV2 only contains JSON-serializable typed view models");
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

    impl SurfaceView for UnreadSummaryVm {
        const SURFACE: SurfaceId = SurfaceId::MessageUnreadSummary;

        fn snapshot(view: Self) -> TypedSurfaceMutation {
            TypedSurfaceMutation::UnreadSummarySnapshot { view }
        }

        fn patch(view: Self) -> TypedSurfaceMutation {
            TypedSurfaceMutation::UnreadSummaryPatch {
                patch: ReplaceView {
                    kind: ReplaceViewKind::ReplaceView,
                    view,
                },
            }
        }
    }

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

    #[test]
    fn revision_overflow_never_wraps_to_a_stale_snapshot() {
        let scope = SurfaceScope::User {
            user_id: "viewer-1".to_string(),
            team_id: "team-1".to_string(),
        };
        let mut projector = TypedSurfaceProjector::<UnreadSummaryVm>::default();
        projector.slots.insert(
            scope.canonical_key(),
            SurfaceSlot {
                revision: u64::MAX,
                view: UnreadSummaryVm {
                    unread_total: 0,
                    mention_total: 0,
                    urgent_total: 0,
                    unread_channel_count: 0,
                },
            },
        );
        assert!(matches!(
            projector.snapshot(
                scope,
                UnreadSummaryVm {
                    unread_total: 0,
                    mention_total: 0,
                    urgent_total: 0,
                    unread_channel_count: 0,
                },
            ),
            Err(SurfaceProjectorError::RevisionOverflow {
                surface: "message.unread-summary"
            })
        ));
    }

    #[test]
    fn create_topic_channel_member_change_commits_all_attached_surfaces_atomically() {
        let canonical = crate::interaction_frame::InteractionFrame::decode(include_bytes!(
            "../../tests/fixtures/uc01-16-surface-runtime/snapshots.json"
        ))
        .expect("canonical fixture");
        let mutations = canonical
            .mutations
            .into_iter()
            .filter(|mutation| {
                matches!(
                    mutation.surface,
                    SurfaceId::MessageLeftDialogList
                        | SurfaceId::MessageTimeline
                        | SurfaceId::MessageChannelSummary
                        | SurfaceId::MessageMemberRoster
                        | SurfaceId::MessageTopicList
                )
            })
            .map(patch_from_snapshot)
            .collect::<Vec<_>>();
        let frame = build_interaction_frame("create-topic:request-1".to_string(), mutations)
            .expect("all attached surface views form one transaction");
        assert_eq!(frame.mutations.len(), 5);
        assert!(frame
            .mutations
            .iter()
            .all(|mutation| mutation.causation_id.as_deref() == Some("create-topic:request-1")));
        assert!(frame
            .mutations
            .iter()
            .all(|mutation| mutation.mutation.is_patch()));
    }

    fn patch_from_snapshot(mut mutation: SurfaceMutation) -> SurfaceMutation {
        mutation.base_revision = Some(mutation.revision);
        mutation.revision = mutation
            .revision
            .checked_add(1)
            .expect("fixture revision is below u64::MAX");
        mutation.causation_id = Some("create-topic:request-1".to_string());
        mutation.mutation = match mutation.mutation {
            TypedSurfaceMutation::DialogListSnapshot { view } => {
                TypedSurfaceMutation::DialogListPatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view,
                    },
                }
            }
            TypedSurfaceMutation::TimelineSnapshot { view } => {
                TypedSurfaceMutation::TimelinePatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view,
                    },
                }
            }
            TypedSurfaceMutation::ChannelSummarySnapshot { view } => {
                TypedSurfaceMutation::ChannelSummaryPatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view,
                    },
                }
            }
            TypedSurfaceMutation::MemberRosterSnapshot { view } => {
                TypedSurfaceMutation::MemberRosterPatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view,
                    },
                }
            }
            TypedSurfaceMutation::TopicListSnapshot { view } => {
                TypedSurfaceMutation::TopicListPatch {
                    patch: ReplaceView {
                        kind: ReplaceViewKind::ReplaceView,
                        view,
                    },
                }
            }
            other => panic!("unexpected fixture mutation in T041 test: {other:?}"),
        };
        mutation
    }
}
