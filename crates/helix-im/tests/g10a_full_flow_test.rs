//! G10a 上拉更早历史的 Helix sans-IO 完整回放合同。

#[path = "support/g10a_scenario.rs"]
mod scenario;

use helix_core::effect::Effect;
use helix_core::tick::AppCommand;
use helix_core::{EffectSink, Module, Tick};
use helix_im::interaction_frame::{SurfaceLoadState, TypedSurfaceMutation};
use helix_im::module::ImConfig;
use helix_im::query::LocalStoreMode;
use helix_im::state::ChannelId;
use helix_im::ImModule;
use serde_json::json;

use scenario::{
    assert_group_created_contract_for, assert_timeline_contract, g10a_scenario_spec,
    G10AScenarioRunner,
};

/// Proves the native correlation reaches HTTP headers without entering the wire body.
#[test]
fn transport_correlation_is_preserved_on_post_context_http() {
    let http = G10AScenarioRunner::post_context_request();
    assert!(http
        .headers
        .iter()
        .any(|(name, value)| name == "Cses-Track-Id" && value == scenario::REQUEST_ID));
    let body: serde_json::Value = serde_json::from_slice(
        http.body
            .as_ref()
            .expect("postContext request contains a body"),
    )
    .expect("postContext body is JSON");
    assert!(body.get("req_id").is_none());
}

/// Proves every canonical run begins with the shared fresh-group prerequisite.
#[test]
fn prerequisite_creates_fresh_group_before_loading_history() {
    let trace = G10AScenarioRunner::run_success();
    assert_group_created_contract_for(&trace.group_created, &g10a_scenario_spec());
    assert_timeline_contract(&trace.initial, SurfaceLoadState::Ready, 20, true);
}

/// Proves success waits for message persistence and publishes one complete 40-row patch.
#[test]
fn happy_path_waits_for_persist_and_projects_complete_window() {
    let trace = G10AScenarioRunner::run_success();
    assert!(trace.persisted_before_frame);
    assert_timeline_contract(&trace.loaded, SurfaceLoadState::Ready, 40, true);
    assert!(matches!(
        trace.loaded.mutations[0].mutation.mutation,
        TypedSurfaceMutation::TimelinePatch { .. }
    ));
}

/// Proves detached anchors are rejected before any HTTP or persistence Effect is emitted.
#[test]
fn business_failure_rejects_anchor_without_attached_timeline() {
    let mut module = ImModule::new_with_local_store(
        ImConfig {
            auth_user_id: scenario::ACTOR_ID.to_string(),
            ..ImConfig::default()
        },
        LocalStoreMode::Session,
    );
    module.register_channel(
        ChannelId::from_str(scenario::CHANNEL_ID).expect("G10a channel id is valid"),
        0,
    );
    let command = Tick::Command(AppCommand::new(
        "im_load_older_context",
        serde_json::to_vec(&json!({
            "channel_id": scenario::CHANNEL_ID,
            "anchor_post_id": "detached-anchor",
            "before": 20,
        }))
        .expect("G10a detached command serializes"),
    ));
    let mut sink = EffectSink::new();
    let error = module
        .handle(&command, scenario::CREATED_AT as u64, &mut sink)
        .expect_err("detached anchor must be rejected");
    assert!(error.to_string().contains("requires one attached timeline"));
    assert!(sink
        .as_slice()
        .iter()
        .all(|effect| !matches!(effect, Effect::Http { .. } | Effect::Persist { .. })));
}

/// Proves HTTP failure advances to failed without erasing the initial 20 rows.
#[test]
fn infrastructure_http_failure_preserves_existing_window() {
    let trace = G10AScenarioRunner::run_http_failure();
    assert_group_created_contract_for(&trace.group_created, &g10a_scenario_spec());
    assert_timeline_contract(&trace.initial, SurfaceLoadState::Ready, 20, true);
    assert_timeline_contract(&trace.failed, SurfaceLoadState::Failed, 20, true);
    assert_eq!(trace.persisted_write_count, 0);
}

/// Proves message persistence failure cannot release a ready Frame.
#[test]
fn infrastructure_persist_failure_preserves_existing_window() {
    let trace = G10AScenarioRunner::run_persist_failure();
    assert_timeline_contract(&trace.failed, SurfaceLoadState::Failed, 20, true);
    assert_eq!(trace.persisted_write_count, 1);
}

/// Proves malformed postContext data cannot write or publish partial history.
#[test]
fn malformed_authority_response_fails_closed() {
    let trace = G10AScenarioRunner::run_malformed_response();
    assert_timeline_contract(&trace.failed, SurfaceLoadState::Failed, 20, true);
    assert_eq!(trace.persisted_write_count, 0);
}

/// Proves consumed persistence and readback correlations are idempotent no-ops.
#[test]
fn race_and_idempotency_drop_duplicate_replies() {
    let trace = G10AScenarioRunner::run_duplicate_replies();
    assert_eq!(trace.duplicate_persist_effects, 0);
    assert_eq!(trace.duplicate_readback_effects, 0);
    assert_timeline_contract(&trace.loaded, SurfaceLoadState::Ready, 40, true);
}

/// Proves a pivot-only recovery response clears Helix-owned hasOlder/hasMore.
#[test]
fn recovery_marks_the_oldest_authoritative_window_exhausted() {
    let trace = G10AScenarioRunner::run_exhaustion_recovery();
    assert_timeline_contract(&trace.loaded, SurfaceLoadState::Ready, 40, true);
    assert_timeline_contract(&trace.exhausted, SurfaceLoadState::Ready, 40, false);
    assert!(
        trace.exhausted.mutations[0].mutation.revision
            > trace.loaded.mutations[0].mutation.revision
    );
}
