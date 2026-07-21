//! `message.member-roster` 的有界 typed outlet。
//!
//! member role、presence 与 membership lifecycle 均是 Helix 已裁决的 VM 字段。这里不从
//! `im:channel:members` 或 raw HTTP row 推导它们，只保存 attach cursor 对应的完整窗口。

use thiserror::Error;

pub use crate::interaction_frame::MemberRosterVm;
use crate::interaction_frame::{SurfaceId, SurfaceMutation, SurfaceScope, TypedSurfaceMutation};

use super::{SurfaceProjectorError, SurfaceView, TypedSurfaceProjector};

/// 一个 roster attach cursor 最多保留的成员行，防止常驻频道面把无界成员表放进 FrameCache。
pub const MAX_MEMBER_ROSTER_ITEMS: usize = 200;

impl SurfaceView for MemberRosterVm {
    const SURFACE: SurfaceId = SurfaceId::MessageMemberRoster;

    fn snapshot(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::MemberRosterSnapshot { view }
    }

    fn patch(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::MemberRosterPatch {
            patch: crate::interaction_frame::ReplaceView {
                kind: crate::interaction_frame::ReplaceViewKind::ReplaceView,
                view,
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct MemberRosterProjector {
    slots: TypedSurfaceProjector<MemberRosterVm>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MemberRosterProjectionError {
    #[error("member roster has {actual} items; maximum is {maximum}")]
    ItemLimit { actual: usize, maximum: usize },
    #[error(transparent)]
    Slot(#[from] SurfaceProjectorError),
}

impl MemberRosterProjector {
    /// attach/query 的成员窗口快照。cursor 来自 `view.page.windowToken`，并进入 scope。
    pub fn snapshot(
        &mut self,
        view: MemberRosterVm,
    ) -> Result<SurfaceMutation, MemberRosterProjectionError> {
        ensure_bounded(&view)?;
        self.slots
            .snapshot(scope_for(&view), view)
            .map_err(Into::into)
    }

    /// 成员加入/离开、角色或 presence 变化后的完整 replaceView patch。
    pub fn patch_membership_lifecycle(
        &mut self,
        view: MemberRosterVm,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, MemberRosterProjectionError> {
        ensure_bounded(&view)?;
        self.slots
            .patch(scope_for(&view), view, causation_id)
            .map_err(Into::into)
    }

    pub fn current_view(&self, channel_id: &str, cursor: &str) -> Option<&MemberRosterVm> {
        self.slots.current_view(&SurfaceScope::MemberRoster {
            channel_id: channel_id.to_string(),
            cursor: cursor.to_string(),
        })
    }

    pub fn reset(&mut self) {
        self.slots.reset();
    }
}

fn scope_for(view: &MemberRosterVm) -> SurfaceScope {
    SurfaceScope::MemberRoster {
        channel_id: view.channel_id.clone(),
        cursor: view.page.window_token.clone(),
    }
}

fn ensure_bounded(view: &MemberRosterVm) -> Result<(), MemberRosterProjectionError> {
    if view.items.len() > MAX_MEMBER_ROSTER_ITEMS {
        return Err(MemberRosterProjectionError::ItemLimit {
            actual: view.items.len(),
            maximum: MAX_MEMBER_ROSTER_ITEMS,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_frame::{
        ChannelRole, MemberRowVm, MembershipState, PresenceState, SurfaceLoadState,
        TypedSurfaceMutation, UserRefVm, WindowPageVm,
    };

    fn roster(
        role: ChannelRole,
        presence: PresenceState,
        membership: MembershipState,
    ) -> MemberRosterVm {
        MemberRosterVm {
            channel_id: "channel-1".to_string(),
            items: vec![MemberRowVm {
                user: UserRefVm {
                    user_id: "user-1".to_string(),
                },
                nickname: Some("Ada".to_string()),
                role,
                presence,
                membership_state: membership,
                actions: Vec::new(),
            }],
            page: WindowPageVm {
                window_token: "cursor-1".to_string(),
                has_older: false,
                has_newer: false,
                has_more: false,
            },
            member_count: 1,
            online_count: Some(1),
            state: SurfaceLoadState::Ready,
        }
    }

    #[test]
    fn cursor_scoped_role_presence_and_lifecycle_patch_is_full_replace_view() {
        let mut projector = MemberRosterProjector::default();
        projector
            .snapshot(roster(
                ChannelRole::Member,
                PresenceState::Away,
                MembershipState::Active,
            ))
            .unwrap();
        let patch = projector
            .patch_membership_lifecycle(
                roster(
                    ChannelRole::Admin,
                    PresenceState::Online,
                    MembershipState::Removed,
                ),
                Some("member-change-1".to_string()),
            )
            .unwrap();
        assert_eq!(patch.base_revision, Some(1));
        assert!(matches!(
            patch.mutation,
            TypedSurfaceMutation::MemberRosterPatch { .. }
        ));
        let item = &projector
            .current_view("channel-1", "cursor-1")
            .unwrap()
            .items[0];
        assert_eq!(item.role, ChannelRole::Admin);
        assert_eq!(item.presence, PresenceState::Online);
        assert_eq!(item.membership_state, MembershipState::Removed);
    }

    #[test]
    fn refuses_unbounded_member_window() {
        let mut view = roster(
            ChannelRole::Member,
            PresenceState::Offline,
            MembershipState::Active,
        );
        view.items = (0..=MAX_MEMBER_ROSTER_ITEMS)
            .map(|index| MemberRowVm {
                user: UserRefVm {
                    user_id: format!("user-{index}"),
                },
                nickname: None,
                role: ChannelRole::Member,
                presence: PresenceState::Offline,
                membership_state: MembershipState::Active,
                actions: Vec::new(),
            })
            .collect();
        assert!(matches!(
            MemberRosterProjector::default().snapshot(view),
            Err(MemberRosterProjectionError::ItemLimit {
                actual,
                maximum,
            }) if actual == MAX_MEMBER_ROSTER_ITEMS + 1 && maximum == MAX_MEMBER_ROSTER_ITEMS
        ));
    }
}
