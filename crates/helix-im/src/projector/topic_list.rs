//! `message.topic-list` 的 parent-scoped typed outlet。
//!
//! topic 与 root-message link 已在 Helix 形成完整 `TopicListVm` 后才能进入这里；客户端不根据
//! `im_make_topic`、channel row 或消息关联字段拼装话题列表。

use thiserror::Error;

pub use crate::interaction_frame::TopicListVm;
use crate::interaction_frame::{SurfaceId, SurfaceMutation, SurfaceScope, TypedSurfaceMutation};

use super::{SurfaceProjectorError, SurfaceView, TypedSurfaceProjector};

/// 一个 parent + cursor 对应的 topic 页的最大项目数。
pub const MAX_TOPIC_LIST_ITEMS: usize = 100;

impl SurfaceView for TopicListVm {
    const SURFACE: SurfaceId = SurfaceId::MessageTopicList;

    fn snapshot(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::TopicListSnapshot { view }
    }

    fn patch(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::TopicListPatch {
            patch: crate::interaction_frame::ReplaceView {
                kind: crate::interaction_frame::ReplaceViewKind::ReplaceView,
                view,
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct TopicListProjector {
    slots: TypedSurfaceProjector<TopicListVm>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TopicListProjectionError {
    #[error("topic list has {actual} items; maximum is {maximum}")]
    ItemLimit { actual: usize, maximum: usize },
    #[error(transparent)]
    Slot(#[from] SurfaceProjectorError),
}

impl TopicListProjector {
    /// attach/query 的完整 parent topic 页快照。
    pub fn snapshot(
        &mut self,
        view: TopicListVm,
    ) -> Result<SurfaceMutation, TopicListProjectionError> {
        ensure_bounded(&view)?;
        self.slots
            .snapshot(scope_for(&view), view)
            .map_err(Into::into)
    }

    /// create-topic、root link 或 lifecycle 完成后的完整 replaceView patch。
    pub fn patch_topic_link(
        &mut self,
        view: TopicListVm,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, TopicListProjectionError> {
        ensure_bounded(&view)?;
        self.slots
            .patch(scope_for(&view), view, causation_id)
            .map_err(Into::into)
    }

    pub fn current_view(&self, parent_channel_id: &str, cursor: &str) -> Option<&TopicListVm> {
        self.slots.current_view(&SurfaceScope::TopicList {
            parent_channel_id: parent_channel_id.to_string(),
            cursor: cursor.to_string(),
        })
    }

    pub fn reset(&mut self) {
        self.slots.reset();
    }
}

fn scope_for(view: &TopicListVm) -> SurfaceScope {
    SurfaceScope::TopicList {
        parent_channel_id: view.parent_channel_id.clone(),
        cursor: view.page.window_token.clone(),
    }
}

fn ensure_bounded(view: &TopicListVm) -> Result<(), TopicListProjectionError> {
    if view.items.len() > MAX_TOPIC_LIST_ITEMS {
        return Err(TopicListProjectionError::ItemLimit {
            actual: view.items.len(),
            maximum: MAX_TOPIC_LIST_ITEMS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_frame::{
        ChannelAvatarRef, ChannelLifecycleVm, ChannelTitleRef, PreviewVm, SurfaceLoadState,
        TopicRowVm, TypedSurfaceMutation, WindowPageVm,
    };

    fn topics(last_activity_at: i64) -> TopicListVm {
        TopicListVm {
            parent_channel_id: "channel-1".to_string(),
            items: vec![TopicRowVm {
                topic_channel_id: "topic-1".to_string(),
                root_message_id: "message-1".to_string(),
                title: ChannelTitleRef {
                    text: "Roadmap".to_string(),
                },
                root_message_preview: PreviewVm::System {
                    text: "created".to_string(),
                },
                last_activity_at,
                message_count: 1,
                avatar: ChannelAvatarRef {
                    resource: None,
                    user_ids: Vec::new(),
                    fallback_text: "Roadmap".to_string(),
                },
                lifecycle: ChannelLifecycleVm {
                    state: "active".to_string(),
                    reason: None,
                },
                actions: Vec::new(),
            }],
            page: WindowPageVm {
                window_token: "cursor-1".to_string(),
                has_older: false,
                has_newer: false,
                has_more: false,
            },
            topic_count: 1,
            state: SurfaceLoadState::Ready,
        }
    }

    #[test]
    fn parent_cursor_and_root_link_publish_a_replace_view_patch() {
        let mut projector = TopicListProjector::default();
        projector.snapshot(topics(1)).unwrap();
        let patch = projector
            .patch_topic_link(topics(2), Some("create-topic:request-1".to_string()))
            .unwrap();
        assert_eq!(patch.base_revision, Some(1));
        assert_eq!(
            patch.causation_id.as_deref(),
            Some("create-topic:request-1")
        );
        assert!(matches!(
            patch.mutation,
            TypedSurfaceMutation::TopicListPatch { .. }
        ));
        assert_eq!(
            projector
                .current_view("channel-1", "cursor-1")
                .unwrap()
                .items[0]
                .root_message_id,
            "message-1"
        );
    }
}
