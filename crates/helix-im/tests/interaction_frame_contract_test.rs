use std::collections::{BTreeMap, BTreeSet};

use helix_im::interaction_frame::{
    ActionInvocation, ActionReceipt, ActionReceiptStatus, InteractionFrame, InteractionFrameError,
    SurfaceId,
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

#[test]
fn canonical_snapshot_decodes_all_eight_surface_vms() {
    let frame = decode(CANONICAL_SNAPSHOT);

    let actual: BTreeSet<_> = frame
        .mutations
        .iter()
        .map(|mutation| mutation.surface)
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

    assert_eq!(frame.schema_version, 1);
    assert_eq!(frame.mutations.len(), 8);
    assert_eq!(actual, expected);
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
        .all(|mutation| mutation.mutation.is_patch()));
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
        InteractionFrame::decode(DUPLICATE.as_bytes()).expect_err("duplicate slot rejected");
    assert!(matches!(
        duplicate,
        InteractionFrameError::DuplicateSlot { .. }
    ));

    let mismatch =
        InteractionFrame::decode(SCOPE_MISMATCH.as_bytes()).expect_err("scope mismatch rejected");
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
fn accepted_action_fixture_is_pending_until_a_matching_frame_causation_arrives() {
    let receipt: ActionReceipt =
        serde_json::from_str(ACCEPTED_ACTION).expect("valid accepted action receipt");
    receipt.validate().expect("accepted receipt contract");
    assert_eq!(receipt.status, ActionReceiptStatus::Accepted);
    assert!(receipt.validation_errors.is_none());

    let mut frame = decode(PATCH_TRANSACTION);
    for mutation in &mut frame.mutations {
        mutation.causation_id = Some("action-accepted-1".to_string());
    }
    frame.validate().expect("matching causation frame is valid");
    assert!(frame
        .mutations
        .iter()
        .all(|mutation| mutation.causation_id.as_deref() == Some(receipt.request_id.as_str())));
}

#[test]
fn unknown_mutation_kind_and_missing_patch_baseline_are_rejected() {
    let mut unknown: serde_json::Value =
        serde_json::from_str(CANONICAL_SNAPSHOT).expect("canonical fixture is JSON");
    unknown["mutations"][0]["mutation"]["kind"] = serde_json::json!("futureDialogMutation");
    assert!(matches!(
        InteractionFrame::decode(
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
        InteractionFrame::decode(
            serde_json::to_vec(&missing_base)
                .expect("fixture remains JSON")
                .as_slice()
        ),
        Err(InteractionFrameError::MissingBaseRevision { .. })
    ));
}

fn decode(json: &str) -> InteractionFrame {
    InteractionFrame::decode(json.as_bytes()).expect("fixture must satisfy the wire contract")
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TestFrameCache {
    slots: BTreeMap<(SurfaceId, String), u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyError {
    Stale,
    ResyncRequired,
}

impl TestFrameCache {
    fn revision(&self, surface: SurfaceId, scope: &str) -> Option<u64> {
        self.slots.get(&(surface, scope.to_owned())).copied()
    }

    fn apply(&mut self, frame: &InteractionFrame) -> Result<(), ApplyError> {
        frame
            .validate()
            .expect("cache receives only schema-valid frames");
        let mut staged = self.slots.clone();

        for mutation in &frame.mutations {
            let slot = (mutation.surface, mutation.scope.canonical_key());
            let current = staged.get(&slot).copied();
            if mutation.mutation.is_patch() {
                if current != mutation.base_revision {
                    return Err(ApplyError::ResyncRequired);
                }
            } else if current.is_some_and(|revision| mutation.revision <= revision) {
                return Err(ApplyError::Stale);
            }
            staged.insert(slot, mutation.revision);
        }

        self.slots = staged;
        Ok(())
    }
}
