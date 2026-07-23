use std::collections::{BTreeMap, BTreeSet};

use helix_im::interaction_frame::{
    ActionBinding, ActionInvocation, ActionReceipt, ActionReceiptStatus, InteractionFrameError,
    InteractionFrameV2, SurfaceDataVm, SurfaceId, TypedSurfaceMutation, MAX_SAFE_JSON_INTEGER,
    MIN_SAFE_JSON_INTEGER,
};

const CANONICAL_SNAPSHOT: &str = include_str!("fixtures/uc01-16-surface-runtime/snapshots.json");
const PATCH_TRANSACTION: &str =
    include_str!("fixtures/uc01-16-surface-runtime/patch-transaction.json");
const DUPLICATE: &str = include_str!("fixtures/uc01-16-surface-runtime/duplicate.json");
const STALE: &str = include_str!("fixtures/uc01-16-surface-runtime/stale.json");
const SCOPE_MISMATCH: &str = include_str!("fixtures/uc01-16-surface-runtime/scope-mismatch.json");
const GAP: &str = include_str!("fixtures/uc01-16-surface-runtime/gap.json");
const REJECTED_ACTION: &str = include_str!("fixtures/uc01-16-surface-runtime/rejected-action.json");
const ACCEPTED_ACTION: &str = include_str!("fixtures/uc01-16-surface-runtime/accepted-action.json");
const GS01_RECONNECT_V2: &str =
    include_str!("fixtures/uc01-16-surface-runtime/gs01-reconnect-canonical-v2.json");
const GS03_HYDRATION_V2: &str =
    include_str!("fixtures/uc01-16-surface-runtime/gs03-unknown-channel-hydration-v2.json");
const GS_DUPLICATE_V2: &str = include_str!("fixtures/uc01-16-surface-runtime/gs-duplicate-v2.json");
const RICH_V2: &str = include_str!("fixtures/uc01-16-surface-runtime/rich-v2.json");

#[test]
fn canonical_snapshot_decodes_all_eight_surface_vms() {
    let frame = decode(CANONICAL_SNAPSHOT);

    let actual: BTreeSet<_> = frame
        .mutations
        .iter()
        .map(|mutation| mutation.mutation.surface)
        .collect();
    let expected = BTreeSet::from([
        SurfaceId::MessageLeftDialogList,
        SurfaceId::MessageUnreadSummary,
        SurfaceId::MessageTimeline,
        SurfaceId::MessageThreadDetail,
        SurfaceId::MessageChannelSummary,
        SurfaceId::MessageMemberRoster,
        SurfaceId::MessageTopicList,
        SurfaceId::MessageParticipantReceipts,
    ]);

    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 8);
    assert_eq!(actual, expected);
}

#[test]
fn v2_composer_requires_runtime_epoch_and_a_channel_scoped_binding() {
    let valid = serde_json::json!({
        "schemaVersion": 2,
        "transactionId": "sync-transaction-1",
        "causationId": "sync-corr-1",
        "runtime": { "connection": "online", "initialSync": "ready", "recovery": "recovered" },
        "mutations": [{
            "surface": "message.composer",
            "scope": { "kind": "channel", "channelId": "channel-1" },
            "revision": 1,
            "surfaceEpoch": 7,
            "mutation": {
                "kind": "composerSnapshot",
                "view": {
                    "channelId": "channel-1",
                    "state": "ready",
                    "draftPolicy": "message.compose.text.v1",
                    "capabilityEpoch": 7,
                    "actions": [{
                        "actionRef": "opaque-send-1",
                        "kind": "im.send-message",
                        "inputShape": "message.compose.text.v1",
                        "surfaceEpoch": 7,
                        "enabled": true
                    }]
                }
            }
        }]
    });
    InteractionFrameV2::decode(serde_json::to_vec(&valid).unwrap().as_slice())
        .expect("v2 composer frame is valid");

    let mut missing_epoch = valid.clone();
    missing_epoch["mutations"][0]
        .as_object_mut()
        .unwrap()
        .remove("surfaceEpoch");
    assert!(matches!(
        InteractionFrameV2::decode(serde_json::to_vec(&missing_epoch).unwrap().as_slice()),
        Err(InteractionFrameError::Decode(_))
    ));

    let mut disabled_hydration = valid;
    disabled_hydration["mutations"][0]["mutation"]["view"]["state"] =
        serde_json::json!("hydrating");
    assert!(matches!(
        InteractionFrameV2::decode(serde_json::to_vec(&disabled_hydration).unwrap().as_slice()),
        Err(InteractionFrameError::InvalidView { .. })
    ));
}

#[test]
fn gs_v2_fixtures_keep_composer_and_summary_in_a_valid_committed_frame() {
    for fixture in [GS01_RECONNECT_V2, GS03_HYDRATION_V2] {
        let frame = InteractionFrameV2::decode(fixture.as_bytes())
            .expect("GS V2 fixture must be a strict InteractionFrameV2");
        assert!(frame
            .mutations
            .iter()
            .any(|mutation| mutation.mutation.surface == SurfaceId::MessageComposer));
        assert_eq!(
            frame.runtime.connection,
            helix_im::interaction_frame::RuntimeConnectionState::Online
        );
    }
}

#[test]
fn gs_v2_duplicate_fixture_rejects_the_whole_frame() {
    assert!(matches!(
        InteractionFrameV2::decode(GS_DUPLICATE_V2.as_bytes()),
        Err(InteractionFrameError::DuplicateSlot { .. })
    ));
}

#[test]
fn rich_v2_fixture_preserves_reply_reaction_and_quick_action_contracts() {
    let frame = InteractionFrameV2::decode(RICH_V2.as_bytes())
        .expect("rich fixture must be a strict InteractionFrameV2");
    let value = serde_json::to_value(frame).expect("rich frame serializes");
    let items = value["mutations"][0]["mutation"]["view"]["items"]
        .as_array()
        .expect("timeline items");

    assert_eq!(
        items[0]["quickActions"],
        serde_json::json!(["react", "quoteReply", "createTopic"])
    );
    assert_eq!(items[0]["replyThread"]["replyCount"], 4);
    assert_eq!(
        items[0]["replyThread"]["items"].as_array().map(Vec::len),
        Some(3)
    );
    assert_eq!(
        items[0]["reactions"],
        serde_json::json!([{ "emoji": "😊", "count": 2 }])
    );
    assert_eq!(items[0]["topicLink"]["topicChannelId"], "topic-1");
}

#[test]
fn v2_consumer_gate_has_no_v1_or_raw_projection_fallback() {
    let v2 = InteractionFrameV2::decode(GS01_RECONNECT_V2.as_bytes())
        .expect("strict V2 decoder accepts the committed recovery fixture");
    assert_eq!(v2.schema_version, 2);
    let mut v1: serde_json::Value = serde_json::from_str(GS01_RECONNECT_V2).unwrap();
    v1["schemaVersion"] = serde_json::json!(1);
    assert!(matches!(
        InteractionFrameV2::decode(serde_json::to_vec(&v1).unwrap().as_slice()),
        Err(InteractionFrameError::SchemaVersion(1))
    ));

    let projector = include_str!("../src/projector/mod.rs");
    assert_eq!(projector.matches("fn emit_interaction_frame(").count(), 1);
    assert!(!projector.contains("emit_interaction_frame_v2"));
    assert!(
        !projector.contains("raw fallback"),
        "a rejected V2 frame must stay rejected instead of being rewritten as a raw projection"
    );
}

#[test]
fn patch_transaction_uses_replace_view_and_advances_all_affected_slots() {
    let mut cache = TestFrameCache::default();
    cache
        .apply(&decode(CANONICAL_SNAPSHOT))
        .expect("snapshot applies");

    let patch = decode(PATCH_TRANSACTION);
    assert!(patch
        .mutations
        .iter()
        .all(|mutation| mutation.mutation.mutation.is_patch()));
    cache
        .apply(&patch)
        .expect("matching-base patch applies atomically");

    assert_eq!(
        cache.revision(SurfaceId::MessageLeftDialogList, "user|6:user-1|6:team-1"),
        Some(2)
    );
    assert_eq!(
        cache.revision(SurfaceId::MessageUnreadSummary, "user|6:user-1|6:team-1"),
        Some(2)
    );
    assert_eq!(
        cache.revision(
            SurfaceId::MessageTimeline,
            "timeline|9:channel-1|8:latest-1"
        ),
        Some(2)
    );
}

#[test]
fn duplicate_and_scope_mismatch_fixtures_reject_the_complete_frame() {
    let duplicate =
        InteractionFrameV2::decode(DUPLICATE.as_bytes()).expect_err("duplicate slot rejected");
    assert!(matches!(
        duplicate,
        InteractionFrameError::DuplicateSlot { .. }
    ));

    let mismatch =
        InteractionFrameV2::decode(SCOPE_MISMATCH.as_bytes()).expect_err("scope mismatch rejected");
    assert!(matches!(
        mismatch,
        InteractionFrameError::ScopeMismatch { .. }
    ));
}

#[test]
fn stale_and_gap_never_partially_commit_a_transaction() {
    let mut cache = TestFrameCache::default();
    cache
        .apply(&decode(CANONICAL_SNAPSHOT))
        .expect("snapshot applies");
    let before = cache.slots.clone();

    let stale = decode(STALE);
    assert_eq!(cache.apply(&stale), Err(ApplyError::Stale));
    assert_eq!(
        cache.slots, before,
        "stale slot must not replace cached state"
    );

    let gap = decode(GAP);
    assert_eq!(cache.apply(&gap), Err(ApplyError::ResyncRequired));
    assert_eq!(
        cache.slots, before,
        "a later invalid mutation rolls back earlier staged mutations"
    );
}

#[test]
fn rejected_action_fixture_is_not_an_interaction_frame_success_signal() {
    let receipt: ActionReceipt =
        serde_json::from_str(REJECTED_ACTION).expect("valid action receipt");
    receipt.validate().expect("rejected receipt contract");

    assert_eq!(receipt.status, ActionReceiptStatus::Rejected);
    assert_eq!(
        receipt.validation_errors.expect("rejection reason").len(),
        1
    );

    assert!(
        serde_json::from_str::<ActionInvocation>(r#"{"actionRef":"create-topic","input":[]}"#,)
            .is_err()
    );
}

#[test]
fn action_binding_carries_the_renderer_visible_safe_surface_epoch() {
    let binding = ActionBinding {
        action_ref: "act.v1.test".to_string(),
        kind: "viewReceipt".to_string(),
        input_shape: "object".to_string(),
        surface_epoch: 7,
        enabled: true,
        disabled_reason: None,
    };
    binding.validate().expect("binding carries a usable epoch");
    assert_eq!(
        serde_json::to_value(&binding).unwrap()["surfaceEpoch"],
        serde_json::json!(7)
    );

    let mut zero = binding.clone();
    zero.surface_epoch = 0;
    assert!(matches!(
        zero.validate(),
        Err(InteractionFrameError::InvalidActionBinding { .. })
    ));

    let mut unsafe_epoch = binding.clone();
    unsafe_epoch.surface_epoch = MAX_SAFE_JSON_INTEGER + 1;
    assert!(matches!(
        unsafe_epoch.validate(),
        Err(InteractionFrameError::InvalidActionBinding { .. })
    ));

    assert!(serde_json::from_str::<ActionBinding>(
        r#"{"actionRef":"act.v1.test","kind":"viewReceipt","inputShape":"object","enabled":true}"#
    )
    .is_err());
}

#[test]
fn accepted_action_fixture_is_pending_until_a_matching_frame_causation_arrives() {
    let receipt: ActionReceipt =
        serde_json::from_str(ACCEPTED_ACTION).expect("valid accepted action receipt");
    receipt.validate().expect("accepted receipt contract");
    assert_eq!(receipt.status, ActionReceiptStatus::Accepted);
    assert!(receipt.validation_errors.is_none());

    let mut frame = decode(PATCH_TRANSACTION);
    for mutation in &mut frame.mutations {
        mutation.mutation.causation_id = Some("action-accepted-1".to_string());
    }
    frame.validate().expect("matching causation frame is valid");
    assert!(frame
        .mutations
        .iter()
        .all(|mutation| mutation.mutation.causation_id.as_deref()
            == Some(receipt.request_id.as_str())));
}

#[test]
fn unknown_mutation_kind_and_missing_patch_baseline_are_rejected() {
    let mut unknown: serde_json::Value =
        serde_json::from_str(CANONICAL_SNAPSHOT).expect("canonical fixture is JSON");
    unknown["mutations"][0]["mutation"]["kind"] = serde_json::json!("futureDialogMutation");
    assert!(matches!(
        InteractionFrameV2::decode(
            serde_json::to_vec(&unknown)
                .expect("fixture remains JSON")
                .as_slice()
        ),
        Err(InteractionFrameError::Decode(_))
    ));

    let mut missing_base: serde_json::Value =
        serde_json::from_str(PATCH_TRANSACTION).expect("patch fixture is JSON");
    missing_base["mutations"][0]
        .as_object_mut()
        .expect("patch mutation object")
        .remove("baseRevision");
    assert!(matches!(
        InteractionFrameV2::decode(
            serde_json::to_vec(&missing_base)
                .expect("fixture remains JSON")
                .as_slice()
        ),
        Err(InteractionFrameError::MissingBaseRevision { .. })
    ));
}

#[test]
fn decoder_rejects_v1_wire_integers_outside_the_js_exact_safe_domain() {
    assert_unsafe_wire_number(
        CANONICAL_SNAPSHOT,
        |frame| {
            frame["mutations"][0]["revision"] = serde_json::json!(MAX_SAFE_JSON_INTEGER + 1);
        },
        "revision",
    );
    assert_unsafe_wire_number(
        PATCH_TRANSACTION,
        |frame| {
            frame["mutations"][0]["baseRevision"] = serde_json::json!(MAX_SAFE_JSON_INTEGER + 1);
        },
        "baseRevision",
    );
    assert_unsafe_wire_number(
        CANONICAL_SNAPSHOT,
        |frame| {
            frame["mutations"][0]["mutation"]["view"]["sections"][0]["items"][0]
                ["lastActivityAt"] = serde_json::json!(MAX_SAFE_JSON_INTEGER as i64 + 1);
        },
        "lastActivityAt",
    );
    assert_unsafe_wire_number(
        CANONICAL_SNAPSHOT,
        |frame| {
            frame["mutations"][2]["mutation"]["view"]["items"][0]["createdAt"] =
                serde_json::json!(MAX_SAFE_JSON_INTEGER as i64 + 1);
        },
        "createdAt",
    );
    assert_unsafe_wire_number(
        CANONICAL_SNAPSHOT,
        |frame| {
            frame["mutations"][2]["mutation"]["view"]["items"][0]["content"] = serde_json::json!({
                "kind": "media",
                "mediaType": "image",
                "resource": {},
                "fileName": "image.png",
                "size": MAX_SAFE_JSON_INTEGER + 1,
                "availability": {}
            });
        },
        "size",
    );
    assert_unsafe_wire_number(
        CANONICAL_SNAPSHOT,
        |frame| {
            frame["mutations"][4]["mutation"]["view"]["settingsVersion"] =
                serde_json::json!(MAX_SAFE_JSON_INTEGER + 1);
        },
        "settingsVersion",
    );
    assert_unsafe_wire_number(
        CANONICAL_SNAPSHOT,
        |frame| {
            frame["mutations"][6]["mutation"]["view"]["items"][0]["lastActivityAt"] =
                serde_json::json!(MIN_SAFE_JSON_INTEGER - 1);
        },
        "lastActivityAt",
    );
    assert_unsafe_wire_number(
        CANONICAL_SNAPSHOT,
        |frame| {
            frame["mutations"][7]["mutation"]["view"]["completed"][0]["completedAt"] =
                serde_json::json!(MAX_SAFE_JSON_INTEGER as i64 + 1);
        },
        "completedAt",
    );
}

#[test]
fn exact_safe_boundaries_and_finite_surface_data_decimals_remain_valid() {
    let mut snapshot = frame_value(CANONICAL_SNAPSHOT);
    snapshot["mutations"][0]["revision"] = serde_json::json!(MAX_SAFE_JSON_INTEGER);
    snapshot["mutations"][0]["mutation"]["view"]["sections"][0]["items"][0]["lastActivityAt"] =
        serde_json::json!(MIN_SAFE_JSON_INTEGER);
    snapshot["mutations"][2]["mutation"]["view"]["items"][0]["createdAt"] =
        serde_json::json!(MAX_SAFE_JSON_INTEGER as i64);
    snapshot["mutations"][2]["mutation"]["view"]["items"][0]["replySummary"] =
        serde_json::json!(0.125);
    snapshot["mutations"][2]["mutation"]["view"]["items"][0]["content"] = serde_json::json!({
        "kind": "media",
        "mediaType": "image",
        "resource": {},
        "fileName": "image.png",
        "size": MAX_SAFE_JSON_INTEGER,
        "availability": {}
    });
    snapshot["mutations"][4]["mutation"]["view"]["settingsVersion"] =
        serde_json::json!(MAX_SAFE_JSON_INTEGER);
    snapshot["mutations"][6]["mutation"]["view"]["items"][0]["lastActivityAt"] =
        serde_json::json!(MIN_SAFE_JSON_INTEGER);
    snapshot["mutations"][7]["mutation"]["view"]["completed"][0]["completedAt"] =
        serde_json::json!(MAX_SAFE_JSON_INTEGER as i64);
    decode_value(&snapshot).expect("exact-safe snapshot remains valid");

    let mut patch = frame_value(PATCH_TRANSACTION);
    patch["mutations"][0]["revision"] = serde_json::json!(MAX_SAFE_JSON_INTEGER);
    patch["mutations"][0]["baseRevision"] = serde_json::json!(MAX_SAFE_JSON_INTEGER - 1);
    decode_value(&patch).expect("exact-safe patch revision and baseline remain valid");
}

#[test]
fn producer_validation_rejects_unsafe_numbers_before_serialization() {
    let mut frame = decode(CANONICAL_SNAPSHOT);
    frame.mutations[0].mutation.revision = MAX_SAFE_JSON_INTEGER + 1;
    assert_unsafe_producer_number(frame, "revision");

    let mut frame = decode(CANONICAL_SNAPSHOT);
    let TypedSurfaceMutation::TimelineSnapshot { view } = &mut frame.mutations[2].mutation.mutation
    else {
        panic!("canonical fixture must contain a timeline snapshot");
    };
    view.items[0].created_at = MIN_SAFE_JSON_INTEGER - 1;
    assert_unsafe_producer_number(frame, "createdAt");

    let mut input = BTreeMap::new();
    input.insert(
        "unsafe".to_string(),
        SurfaceDataVm::Number(serde_json::Number::from(MAX_SAFE_JSON_INTEGER + 1)),
    );
    let error = ActionInvocation {
        action_ref: "safe-number-test".to_string(),
        input,
    }
    .validate()
    .expect_err("producer action input rejects unsafe generic numbers");
    assert!(matches!(
        error,
        InteractionFrameError::InvalidActionInvocation { ref reason }
            if reason.contains("exact-safe")
    ));
}

#[test]
fn surface_data_rejects_unsafe_integer_decimal_and_unrepresentable_numbers() {
    assert_invalid_surface_data(serde_json::json!(MAX_SAFE_JSON_INTEGER + 1), "integer");
    assert_invalid_surface_data(
        serde_json::from_str("9007199254740992.5").expect("large decimal JSON value"),
        "decimal",
    );

    let unrepresentable = CANONICAL_SNAPSHOT.replacen("\"eventSeq\": 0", "\"eventSeq\": 1e400", 1);
    assert!(matches!(
        InteractionFrameV2::decode(unrepresentable.as_bytes()),
        Err(InteractionFrameError::Decode(_))
    ));
}

fn decode(json: &str) -> InteractionFrameV2 {
    InteractionFrameV2::decode(json.as_bytes()).expect("fixture must satisfy the wire contract")
}

fn frame_value(json: &str) -> serde_json::Value {
    serde_json::from_str(json).expect("fixture must be JSON")
}

fn decode_value(value: &serde_json::Value) -> Result<InteractionFrameV2, InteractionFrameError> {
    let bytes = serde_json::to_vec(value).expect("mutated fixture remains JSON");
    InteractionFrameV2::decode(&bytes)
}

fn assert_unsafe_wire_number(
    json: &str,
    mutate: impl FnOnce(&mut serde_json::Value),
    field: &'static str,
) {
    let mut frame = frame_value(json);
    mutate(&mut frame);
    let error = decode_value(&frame).expect_err("unsafe wire number must be rejected");
    assert!(matches!(
        error,
        InteractionFrameError::UnsafeWireNumber {
            field: actual_field,
            ..
        } if actual_field == field
    ));
}

fn assert_unsafe_producer_number(frame: InteractionFrameV2, field: &'static str) {
    let error = frame
        .validate()
        .expect_err("producer cannot emit unsafe wire numbers");
    assert!(matches!(
        error,
        InteractionFrameError::UnsafeWireNumber {
            field: actual_field,
            ..
        } if actual_field == field
    ));
}

fn assert_invalid_surface_data(value: serde_json::Value, expected_reason: &str) {
    let mut frame = frame_value(CANONICAL_SNAPSHOT);
    frame["mutations"][2]["mutation"]["view"]["items"][0]["replySummary"] = value;
    let error = decode_value(&frame).expect_err("unsafe generic surface data must be rejected");
    assert!(matches!(
        error,
        InteractionFrameError::InvalidView { ref reason, .. }
            if reason.contains(expected_reason)
    ));
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TestFrameCache {
    slots: BTreeMap<(SurfaceId, String), (u64, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyError {
    Stale,
    ResyncRequired,
}

impl TestFrameCache {
    fn revision(&self, surface: SurfaceId, scope: &str) -> Option<u64> {
        self.slots
            .get(&(surface, scope.to_owned()))
            .map(|(_, revision)| *revision)
    }

    fn apply(&mut self, frame: &InteractionFrameV2) -> Result<(), ApplyError> {
        frame
            .validate()
            .expect("cache receives only schema-valid frames");
        let mut staged = self.slots.clone();

        for v2 in &frame.mutations {
            let mutation = &v2.mutation;
            let slot = (mutation.surface, mutation.scope.canonical_key());
            let current = staged.get(&slot).copied();
            if mutation.mutation.is_patch() {
                if current.map(|(epoch, revision)| (epoch, Some(revision)))
                    != Some((v2.surface_epoch, mutation.base_revision))
                {
                    return Err(ApplyError::ResyncRequired);
                }
            } else if let Some((epoch, revision)) = current {
                if v2.surface_epoch < epoch
                    || (v2.surface_epoch == epoch && mutation.revision <= revision)
                {
                    return Err(ApplyError::Stale);
                }
            }
            staged.insert(slot, (v2.surface_epoch, mutation.revision));
        }

        self.slots = staged;
        Ok(())
    }
}
