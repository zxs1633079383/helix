//! Post-commit interaction composition for recovery flows.
//!
//! This module receives only committed facts supplied by the sync state machine. It has no
//! storage or transport dependency, so an uncommitted batch cannot fabricate a renderer state.

use crate::interaction_frame::{
    InteractionFrameV2, InteractionRuntimeVm, SurfaceMutationV2, UiDirective,
};

/// The bounded set of attached slots affected by one committed recovery batch.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SurfaceChangeSet {
    pub runtime: Option<InteractionRuntimeVm>,
    pub user_mutations: Vec<SurfaceMutationV2>,
    pub channel_bundles: Vec<ChannelInteractionBundle>,
    pub directives: Vec<UiDirective>,
}

/// A channel summary and composer travel together: a renderer cannot enable a send binding from
/// a summary that has not reached the same committed revision boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelInteractionBundle {
    pub channel_id: String,
    pub summary: Option<SurfaceMutationV2>,
    pub composer: SurfaceMutationV2,
}

impl SurfaceChangeSet {
    pub fn into_frame(
        self,
        transaction_id: String,
        causation_id: Option<String>,
    ) -> Result<InteractionFrameV2, &'static str> {
        let runtime = self.runtime.ok_or("committed change set requires runtime")?;
        let mut mutations = self.user_mutations;
        for bundle in self.channel_bundles {
            if bundle.channel_id.is_empty() {
                return Err("channel bundle requires channel id");
            }
            if let Some(summary) = bundle.summary {
                mutations.push(summary);
            }
            mutations.push(bundle.composer);
        }
        let frame = InteractionFrameV2 {
            schema_version: 2,
            transaction_id,
            causation_id,
            runtime,
            mutations,
            directives: self.directives,
        };
        frame.validate().map_err(|_| "invalid composed frame")?;
        Ok(frame)
    }
}
