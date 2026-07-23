//! `message.thread-detail` 的 bounded replay projector。
//!
//! 回复分页在进入这里前必须已经被 Helix 解析为 `MessageVm`。snapshot、append 和 replace 都用
//! `ThreadDetailVm` 的完整 `replaceView` 下行；gap 不降级为旧 `im:channel:replies`。

use thiserror::Error;

pub use crate::interaction_frame::ThreadDetailVm;
use crate::interaction_frame::{
    MessageVm, SurfaceId, SurfaceLoadState, SurfaceMutation, SurfaceScope, TypedSurfaceMutation,
    WindowPageVm,
};

use super::{SurfaceProjectorError, SurfaceView, TypedSurfaceProjector};

/// 一个 root 的 typed thread window 最多保留的 reply node 数。
pub const MAX_THREAD_REPLY_NODES: usize = 100;

impl SurfaceView for ThreadDetailVm {
    const SURFACE: SurfaceId = SurfaceId::MessageThreadDetail;

    fn snapshot(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::ThreadDetailSnapshot { view }
    }

    fn patch(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::ThreadDetailPatch {
            patch: crate::interaction_frame::ReplaceView {
                kind: crate::interaction_frame::ReplaceViewKind::ReplaceView,
                view,
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct ThreadDetailProjector {
    slots: TypedSurfaceProjector<ThreadDetailVm>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ThreadDetailProjectionError {
    #[error("thread reply node count {actual} exceeds maximum {maximum}")]
    NodeLimit { actual: usize, maximum: usize },
    #[error("thread replay revision gap: expected {expected}, current {current}")]
    RevisionGap { expected: u64, current: u64 },
    #[error("thread reply {message_id} does not belong to channel {channel_id}")]
    ReplyChannelMismatch {
        message_id: String,
        channel_id: String,
    },
    #[error("thread root message cannot be appended as a reply")]
    RootAppendedAsReply,
    #[error(transparent)]
    Slot(#[from] SurfaceProjectorError),
}

impl ThreadDetailProjector {
    /// attach/query 的完整 thread 快照。
    pub fn snapshot(
        &mut self,
        view: ThreadDetailVm,
    ) -> Result<SurfaceMutation, ThreadDetailProjectionError> {
        ensure_bounded(&view)?;
        self.slots
            .snapshot(scope_for(&view), view)
            .map_err(Into::into)
    }

    /// 对已 attach thread 追加服务端顺序已裁决的 reply node。
    ///
    /// `expected_revision` 是请求/回复关联时观察到的 slot revision。与当前值不相等时明确返回
    /// `RevisionGap`，调用方必须请求快照而不是尝试与 legacy reply event 合并。
    pub fn append(
        &mut self,
        channel_id: &str,
        root_message_id: &str,
        expected_revision: u64,
        replies: impl IntoIterator<Item = MessageVm>,
        reply_count: u32,
        page: WindowPageVm,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, ThreadDetailProjectionError> {
        let scope = SurfaceScope::Thread {
            channel_id: channel_id.to_string(),
            root_message_id: root_message_id.to_string(),
        };
        let current = self.slots.current_view(&scope).cloned().ok_or_else(|| {
            SurfaceProjectorError::MissingSnapshot {
                surface: SurfaceId::MessageThreadDetail.as_str(),
                slot: scope.canonical_key(),
            }
        })?;
        let actual_revision = self
            .slots
            .current_revision(&scope)
            .expect("thread view and revision are stored together");
        if actual_revision != expected_revision {
            return Err(ThreadDetailProjectionError::RevisionGap {
                expected: expected_revision,
                current: actual_revision,
            });
        }

        let mut view = current;
        for reply in replies {
            if reply.channel_id != channel_id {
                return Err(ThreadDetailProjectionError::ReplyChannelMismatch {
                    message_id: reply.id,
                    channel_id: channel_id.to_string(),
                });
            }
            if reply.id == root_message_id {
                return Err(ThreadDetailProjectionError::RootAppendedAsReply);
            }
            if !view.nodes.contains_key(&reply.id) {
                view.ordered_reply_ids.push(reply.id.clone());
            }
            view.nodes.insert(reply.id.clone(), reply);
        }
        view.reply_count = reply_count.max(view.ordered_reply_ids.len() as u32);
        view.page = page;
        view.state = SurfaceLoadState::Ready;
        ensure_bounded(&view)?;
        self.slots
            .patch(scope, view, causation_id)
            .map_err(Into::into)
    }

    /// 用重拉到的完整 thread view 替换本 slot。与 append 一样，调用端先对齐观察到的 revision。
    pub fn replace(
        &mut self,
        view: ThreadDetailVm,
        expected_revision: u64,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, ThreadDetailProjectionError> {
        ensure_bounded(&view)?;
        let scope = scope_for(&view);
        let current = self.slots.current_revision(&scope).ok_or_else(|| {
            SurfaceProjectorError::MissingSnapshot {
                surface: SurfaceId::MessageThreadDetail.as_str(),
                slot: scope.canonical_key(),
            }
        })?;
        if current != expected_revision {
            return Err(ThreadDetailProjectionError::RevisionGap {
                expected: expected_revision,
                current,
            });
        }
        self.slots
            .patch(scope, view, causation_id)
            .map_err(Into::into)
    }

    pub fn current_view(&self, channel_id: &str, root_message_id: &str) -> Option<&ThreadDetailVm> {
        self.slots.current_view(&SurfaceScope::Thread {
            channel_id: channel_id.to_string(),
            root_message_id: root_message_id.to_string(),
        })
    }

    pub fn current_revision(&self, channel_id: &str, root_message_id: &str) -> Option<u64> {
        self.slots.current_revision(&SurfaceScope::Thread {
            channel_id: channel_id.to_string(),
            root_message_id: root_message_id.to_string(),
        })
    }

    pub fn reset(&mut self) {
        self.slots.reset();
    }
}

fn scope_for(view: &ThreadDetailVm) -> SurfaceScope {
    SurfaceScope::Thread {
        channel_id: view.channel_id.clone(),
        root_message_id: view.root.id.clone(),
    }
}

fn ensure_bounded(view: &ThreadDetailVm) -> Result<(), ThreadDetailProjectionError> {
    let actual = view.nodes.len();
    if actual > MAX_THREAD_REPLY_NODES {
        return Err(ThreadDetailProjectionError::NodeLimit {
            actual,
            maximum: MAX_THREAD_REPLY_NODES,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::interaction_frame::{
        DeliveryVm, MessageContentVm, ReplySummaryVm, SurfaceDataVm, TypedSurfaceMutation,
        UserRefVm,
    };

    fn message(id: &str) -> MessageVm {
        MessageVm {
            id: id.to_string(),
            channel_id: "channel-1".to_string(),
            event_seq: 1,
            author: UserRefVm {
                user_id: "user-1".to_string(),
            },
            created_at: 1,
            content: MessageContentVm::Text {
                text: id.to_string(),
            },
            delivery: DeliveryVm::Sent,
            actions: Vec::new(),
            quick_actions: Vec::new(),
            reply_id: None,
            reply_root_id: None,
            reply_first_level_id: None,
            reply_summary: ReplySummaryVm::None,
            reply_thread: None,
            reactions: Vec::new(),
            read_receipt: SurfaceDataVm::Record(BTreeMap::new()),
            template_confirmation: None,
            urgency: None,
            pinned: false,
            bookmarked: false,
            topic_link: None,
        }
    }

    fn thread() -> ThreadDetailVm {
        ThreadDetailVm {
            channel_id: "channel-1".to_string(),
            detail_context: crate::interaction_frame::DetailContextVm::Reply,
            root: message("root-1"),
            ordered_reply_ids: vec!["reply-1".to_string()],
            nodes: BTreeMap::from([("reply-1".to_string(), message("reply-1"))]),
            reply_count: 1,
            page: WindowPageVm {
                window_token: "thread-cursor-1".to_string(),
                has_older: false,
                has_newer: false,
                has_more: false,
            },
            state: SurfaceLoadState::Ready,
        }
    }

    #[test]
    fn snapshot_append_replace_and_gap_replay_are_explicit() {
        let mut projector = ThreadDetailProjector::default();
        projector.snapshot(thread()).unwrap();

        let append = projector
            .append(
                "channel-1",
                "root-1",
                1,
                [message("reply-2")],
                2,
                WindowPageVm {
                    window_token: "thread-cursor-2".to_string(),
                    has_older: true,
                    has_newer: false,
                    has_more: true,
                },
                Some("reply:request-1".to_string()),
            )
            .unwrap();
        assert_eq!(append.base_revision, Some(1));
        assert!(matches!(
            append.mutation,
            TypedSurfaceMutation::ThreadDetailPatch { .. }
        ));
        assert_eq!(
            projector
                .current_view("channel-1", "root-1")
                .unwrap()
                .ordered_reply_ids,
            vec!["reply-1", "reply-2"]
        );

        let mut replacement = thread();
        replacement.ordered_reply_ids = vec!["reply-2".to_string()];
        replacement.nodes = BTreeMap::from([("reply-2".to_string(), message("reply-2"))]);
        replacement.reply_count = 2;
        let replace = projector
            .replace(replacement, 2, Some("resync:thread-1".to_string()))
            .unwrap();
        assert_eq!(replace.base_revision, Some(2));

        assert!(matches!(
            projector.append(
                "channel-1",
                "root-1",
                1,
                [message("reply-3")],
                3,
                WindowPageVm {
                    window_token: "thread-cursor-3".to_string(),
                    has_older: false,
                    has_newer: false,
                    has_more: false,
                },
                None,
            ),
            Err(ThreadDetailProjectionError::RevisionGap {
                expected: 1,
                current: 3,
            })
        ));
    }
}
