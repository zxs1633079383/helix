//! `message.channel-summary` 的 typed attach/query outlet。
//!
//! 频道设置和生命周期事件在进入本模块前必须已经被 Helix 裁决为完整 `ChannelSummaryVm`。
//! 本模块只维护 attach slot 的 revision，并以 `replaceView` 发布；它不接受 raw channel row。

use thiserror::Error;

pub use crate::interaction_frame::ChannelSummaryVm;
use crate::interaction_frame::{SurfaceId, SurfaceMutation, SurfaceScope, TypedSurfaceMutation};

use super::{SurfaceProjectorError, SurfaceView, TypedSurfaceProjector};

impl SurfaceView for ChannelSummaryVm {
    const SURFACE: SurfaceId = SurfaceId::MessageChannelSummary;

    fn snapshot(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::ChannelSummarySnapshot { view }
    }

    fn patch(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::ChannelSummaryPatch {
            patch: crate::interaction_frame::ReplaceView {
                kind: crate::interaction_frame::ReplaceViewKind::ReplaceView,
                view,
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct ChannelSummaryProjector {
    slots: TypedSurfaceProjector<ChannelSummaryVm>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChannelSummaryProjectionError {
    #[error("channel summary settingsVersion regressed from {current} to {incoming}")]
    SettingsVersionRegression { current: u64, incoming: u64 },
    #[error(transparent)]
    Slot(#[from] SurfaceProjectorError),
}

impl ChannelSummaryProjector {
    /// attach/query 的完整频道摘要快照。
    pub fn snapshot(
        &mut self,
        view: ChannelSummaryVm,
    ) -> Result<SurfaceMutation, ChannelSummaryProjectionError> {
        self.slots
            .snapshot(scope_for(&view), view)
            .map_err(Into::into)
    }

    /// 设置或生命周期事实变化后，以完整 `replaceView` 推进已 attach 的频道摘要。
    ///
    /// `settingsVersion` 只能单调不减：生命周期变化可以沿用当前 setting revision，但不能用旧设置
    /// 覆盖新设置。真实 WS/HTTP producer 尚未接入时，调用方可用此 typed outlet 进行 query/replay。
    pub fn patch_settings_or_lifecycle(
        &mut self,
        view: ChannelSummaryVm,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, ChannelSummaryProjectionError> {
        let scope = scope_for(&view);
        if let Some(current) = self.slots.current_view(&scope) {
            if view.settings_version < current.settings_version {
                return Err(ChannelSummaryProjectionError::SettingsVersionRegression {
                    current: current.settings_version,
                    incoming: view.settings_version,
                });
            }
        }
        self.slots
            .patch(scope, view, causation_id)
            .map_err(Into::into)
    }

    pub fn current_view(&self, channel_id: &str) -> Option<&ChannelSummaryVm> {
        self.slots.current_view(&SurfaceScope::Channel {
            channel_id: channel_id.to_string(),
        })
    }

    pub fn reset(&mut self) {
        self.slots.reset();
    }
}

fn scope_for(view: &ChannelSummaryVm) -> SurfaceScope {
    SurfaceScope::Channel {
        channel_id: view.channel_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_frame::{
        ChannelAvatarRef, ChannelLifecycleVm, ChannelTitleRef, NotificationMode,
        TypedSurfaceMutation,
    };

    fn summary(settings_version: u64) -> ChannelSummaryVm {
        ChannelSummaryVm {
            channel_id: "channel-1".to_string(),
            title: ChannelTitleRef {
                text: "Design".to_string(),
            },
            avatar: ChannelAvatarRef {
                resource: None,
                fallback_text: "Design".to_string(),
            },
            notice_summary: None,
            purpose: None,
            lifecycle: ChannelLifecycleVm {
                state: "active".to_string(),
                reason: None,
            },
            member_count: 3,
            online_count: Some(1),
            topic_count: 0,
            notification_mode: NotificationMode::All,
            actions: Vec::new(),
            settings_version,
        }
    }

    #[test]
    fn settings_and_lifecycle_patch_replaces_the_attached_summary() {
        let mut projector = ChannelSummaryProjector::default();
        let snapshot = projector.snapshot(summary(4)).expect("valid snapshot");
        assert_eq!(snapshot.revision, 1);

        let mut closed = summary(4);
        closed.lifecycle.state = "closed".to_string();
        let patch = projector
            .patch_settings_or_lifecycle(closed, Some("action-1".to_string()))
            .expect("same settings revision may carry a lifecycle change");
        assert_eq!(patch.base_revision, Some(1));
        assert_eq!(patch.revision, 2);
        assert_eq!(patch.causation_id.as_deref(), Some("action-1"));
        assert!(matches!(
            patch.mutation,
            TypedSurfaceMutation::ChannelSummaryPatch { .. }
        ));
        assert_eq!(
            projector.current_view("channel-1").unwrap().lifecycle.state,
            "closed"
        );
    }

    #[test]
    fn older_settings_cannot_replace_a_newer_snapshot() {
        let mut projector = ChannelSummaryProjector::default();
        projector.snapshot(summary(5)).unwrap();
        assert!(matches!(
            projector.patch_settings_or_lifecycle(summary(4), None),
            Err(ChannelSummaryProjectionError::SettingsVersionRegression {
                current: 5,
                incoming: 4,
            })
        ));
    }
}
