//! `message.composer` 的 typed attach outlet。
//!
//! Composer 首帧只声明当前频道的输入生命周期和已接线能力。未进入 action authority 的能力必须
//! 保持 disabled；renderer 不得把存在一个 binding 误解为命令已可执行。

pub use crate::interaction_frame::ComposerVm;
use crate::interaction_frame::{SurfaceId, SurfaceMutation, SurfaceScope, TypedSurfaceMutation};

use super::{SurfaceProjectorError, SurfaceView, TypedSurfaceProjector};

impl SurfaceView for ComposerVm {
    const SURFACE: SurfaceId = SurfaceId::MessageComposer;

    fn snapshot(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::ComposerSnapshot { view }
    }

    fn patch(view: Self) -> TypedSurfaceMutation {
        TypedSurfaceMutation::ComposerPatch {
            patch: crate::interaction_frame::ReplaceView {
                kind: crate::interaction_frame::ReplaceViewKind::ReplaceView,
                view,
            },
        }
    }
}

#[derive(Debug, Default)]
pub struct ComposerProjector {
    slots: TypedSurfaceProjector<ComposerVm>,
}

impl ComposerProjector {
    pub fn snapshot(&mut self, view: ComposerVm) -> Result<SurfaceMutation, SurfaceProjectorError> {
        self.slots.snapshot(scope_for(&view), view)
    }

    pub fn reset(&mut self) {
        self.slots.reset();
    }
}

fn scope_for(view: &ComposerVm) -> SurfaceScope {
    SurfaceScope::Channel {
        channel_id: view.channel_id.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interaction_frame::{ComposerState, TypedSurfaceMutation};

    fn composer() -> ComposerVm {
        ComposerVm {
            channel_id: "channel-1".to_string(),
            state: ComposerState::Ready,
            draft_policy: "message.compose.text.v1".to_string(),
            actions: Vec::new(),
            capability_epoch: 1,
        }
    }

    #[test]
    fn repeated_snapshot_advances_revision_and_reset_restarts_the_slot() {
        let mut projector = ComposerProjector::default();
        let first = projector.snapshot(composer()).expect("first snapshot");
        let second = projector.snapshot(composer()).expect("resync snapshot");

        assert_eq!(first.revision, 1);
        assert_eq!(second.revision, 2);
        assert!(matches!(
            second.mutation,
            TypedSurfaceMutation::ComposerSnapshot { .. }
        ));

        projector.reset();
        let next_session = projector
            .snapshot(composer())
            .expect("new session snapshot");
        assert_eq!(next_session.revision, 1);
    }
}
