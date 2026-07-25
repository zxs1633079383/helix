//! G07 WDIO Frame V2 canonical Fixture 真源。
//!
//! 所有文件来自共享 `G07ScenarioRunner` 的 production `ImModule::handle` 回放；普通测试
//! 只比较 actual Frame 与 checked-in Golden，只有 `UPDATE_G07_WDIO_FIXTURES=1` 才写盘。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};

#[path = "support/g07_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_urgent_receipts_contract,
    assert_urgent_timeline_contract, assert_urgentable_contract, g07_scenario_spec, sha256_hex,
    surface_names, G07ScenarioRunner, ACTOR_ID, CHANNEL_ID, CREATED_AT, GROUP_NAME, MARKER,
    OBSERVER_ID, REQUEST_ID, SERVER_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "768dc9b3597523c6c40279e387d85e9f240fde8a";
const GENERATED_AT: &str = "2026-07-25T22:45:00-07:00";
const PRODUCER: &str =
    "G07ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一份 G07 canonical Frame 的稳定 ID、文件名与 producer 输出。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成建群、加急、确认及两类 receipt 共六份 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let post = G07ScenarioRunner::run_urgent_post();
    let confirm = G07ScenarioRunner::run_urgent_confirm();
    let failed = G07ScenarioRunner::run_http_failure();
    let replayed = G07ScenarioRunner::run_persist_failure_replay();
    let recovered = G07ScenarioRunner::run_recovery();
    assert_eq!(post.group_created, failed.group_created);
    assert_eq!(post.urgentable, failed.urgentable);
    assert_eq!(post.urgent_active, replayed.urgent_active_after_replay);
    assert_group_created_contract_for(&post.group_created, &g07_scenario_spec());
    assert_urgentable_contract(&post.urgentable);
    assert_urgent_timeline_contract(&post.urgent_active, false, Some(REQUEST_ID));
    assert_urgent_receipts_contract(&post.receipts_pending, false);
    assert_urgent_timeline_contract(&recovered.recovered_timeline, false, None);
    assert_urgent_receipts_contract(&recovered.recovered_receipts, false);
    assert_urgent_timeline_contract(
        &confirm.urgent_confirmed,
        true,
        Some("g07-wdio-confirm-request-1"),
    );
    assert_urgent_receipts_contract(&confirm.receipts_completed, true);
    vec![
        FixtureCase {
            fixture_id: "g07-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: post.group_created,
        },
        FixtureCase {
            fixture_id: "g07-urgentable-v2",
            file_name: "urgentable.frame-v2.json",
            frame: post.urgentable,
        },
        FixtureCase {
            fixture_id: "g07-urgent-active-v2",
            file_name: "urgent-active.frame-v2.json",
            frame: post.urgent_active,
        },
        FixtureCase {
            fixture_id: "g07-receipts-pending-v2",
            file_name: "receipts-pending.frame-v2.json",
            frame: post.receipts_pending,
        },
        FixtureCase {
            fixture_id: "g07-urgent-confirmed-v2",
            file_name: "urgent-confirmed.frame-v2.json",
            frame: confirm.urgent_confirmed,
        },
        FixtureCase {
            fixture_id: "g07-receipts-completed-v2",
            file_name: "receipts-completed.frame-v2.json",
            frame: confirm.receipts_completed,
        },
    ]
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 生成排除动态 transaction 身份但保留 G07 业务语义的摘要。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mutation_kinds: Vec<&str> = frame
        .mutations
        .iter()
        .map(|mutation| match mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { .. } => "timelineSnapshot",
            TypedSurfaceMutation::TimelinePatch { .. } => "timelinePatch",
            TypedSurfaceMutation::ParticipantReceiptsSnapshot { .. } => {
                "participantReceiptsSnapshot"
            }
            TypedSurfaceMutation::ParticipantReceiptsPatch { .. } => "participantReceiptsPatch",
            TypedSurfaceMutation::DialogListSnapshot { .. } => "dialogListSnapshot",
            TypedSurfaceMutation::DialogListPatch { .. } => "dialogListPatch",
            _ => "other",
        })
        .collect();
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "mutationKinds": mutation_kinds,
        "message": message_contract(frame),
        "receipts": receipt_contract(frame),
    }))
    .expect("G07 semantic projection serializes")
}

/// 提取 timeline urgency 与 urgent actions 的稳定业务字段。
fn message_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let message =
        frame
            .mutations
            .iter()
            .find_map(|mutation| match &mutation.mutation.mutation {
                TypedSurfaceMutation::TimelineSnapshot { view } => view.items.first(),
                TypedSurfaceMutation::TimelinePatch { patch } => patch.view.items.first(),
                _ => None,
            })?;
    Some(json!({
        "id": message.id,
        "delivery": message.delivery,
        "urgency": message.urgency,
        "urgentActions": message.actions.iter()
            .filter(|action| action.kind.starts_with("urgent"))
            .map(|action| json!({
                "kind": action.kind,
                "enabled": action.enabled,
                "disabledReason": action.disabled_reason,
            }))
            .collect::<Vec<_>>(),
    }))
}

/// 提取 participant receipt summary、completed 与 pending 的稳定字段。
fn receipt_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let view = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::ParticipantReceiptsSnapshot { view } => Some(view),
            TypedSurfaceMutation::ParticipantReceiptsPatch { patch } => Some(&patch.view),
            _ => None,
        })?;
    Some(json!({
        "channelId": view.channel_id,
        "messageId": view.message_id,
        "kind": view.kind,
        "summary": view.summary,
        "completed": view.completed,
        "pending": view.pending,
        "state": view.state,
    }))
}

/// 返回 Helix canonical G07 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g07-wdio")
}

/// 返回 Frame 的第一条 timeline messageId；非 timeline Frame 返回 scope messageId。
fn optional_message_id(frame: &InteractionFrameV2) -> Option<&str> {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { view } => {
                view.items.first().map(|message| message.id.as_str())
            }
            TypedSurfaceMutation::TimelinePatch { patch } => {
                patch.view.items.first().map(|message| message.id.as_str())
            }
            TypedSurfaceMutation::ParticipantReceiptsSnapshot { view } => {
                Some(view.message_id.as_str())
            }
            TypedSurfaceMutation::ParticipantReceiptsPatch { patch } => {
                Some(patch.view.message_id.as_str())
            }
            _ => None,
        })
}

/// 为每份 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId": case.fixture_id,
        "fileName": case.file_name,
        "schemaVersion": case.frame.schema_version,
        "helixCommit": HELIX_COMMIT,
        "producer": PRODUCER,
        "sourceKind": "rust-export",
        "semanticSha256": sha256_hex(&semantic_bytes(&case.frame)),
        "fixtureSha256": sha256_hex(bytes),
        "surfaces": surface_names(&case.frame),
        "generatedAt": GENERATED_AT,
        "parameterizableFields": [
            "/transactionId",
            "/causationId",
            "/mutations/*/causationId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/scope/channelId",
            "/mutations/*/scope/messageId",
            "/mutations/*/scope/userId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/messageId",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/author/userId",
            "/mutations/*/mutation/view/items/*/createdAt",
            "/mutations/*/mutation/view/items/*/content/text",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/items/*/urgency/urgentRequesterId",
            "/mutations/*/mutation/view/items/*/urgency/urgentTargetIds/*",
            "/mutations/*/mutation/view/items/*/urgency/urgentConfirmedIds/*",
            "/mutations/*/mutation/view/anchor/messageId",
            "/mutations/*/mutation/view/completed/*/user/userId",
            "/mutations/*/mutation/view/completed/*/completedAt",
            "/mutations/*/mutation/view/pending/*/user/userId",
            "/mutations/*/mutation/view/summary/requesterId",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/view/sections/*/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/messageId",
            "/mutations/*/mutation/patch/view/items/*/id",
            "/mutations/*/mutation/patch/view/items/*/channelId",
            "/mutations/*/mutation/patch/view/items/*/author/userId",
            "/mutations/*/mutation/patch/view/items/*/createdAt",
            "/mutations/*/mutation/patch/view/items/*/content/text",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/items/*/urgency/urgentRequesterId",
            "/mutations/*/mutation/patch/view/items/*/urgency/urgentTargetIds/*",
            "/mutations/*/mutation/patch/view/items/*/urgency/urgentConfirmedIds/*",
            "/mutations/*/mutation/patch/view/anchor/messageId",
            "/mutations/*/mutation/patch/view/completed/*/user/userId",
            "/mutations/*/mutation/patch/view/completed/*/completedAt",
            "/mutations/*/mutation/patch/view/pending/*/user/userId",
            "/mutations/*/mutation/patch/view/summary/requesterId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/patch/view/sections/*/items/*/actions/*/surfaceEpoch"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "teamId": TEAM_ID,
            "channelId": CHANNEL_ID,
            "groupName": GROUP_NAME,
            "createdAt": CREATED_AT,
            "marker": MARKER,
            "postId": SERVER_ID,
            "reqId": case.frame.causation_id.as_deref().unwrap_or(REQUEST_ID),
            "transactionId": case.frame.transaction_id,
            "messageId": optional_message_id(&case.frame),
            "surfaceEpoch": case.frame.mutations[0].surface_epoch,
            "revision": case.frame.mutations[0].mutation.revision,
            "baseRevision": case.frame.mutations[0].mutation.base_revision
        }
    })
}

#[test]
/// 证明 actual producer、字段语义、Golden bytes 与 provenance 始终同源。
fn current_g07_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 6, "G07 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G07_WDIO_FIXTURES").is_some();
    let mut provenance = Vec::with_capacity(cases.len());
    for case in &cases {
        case.frame.validate().expect("producer Frame remains valid");
        assert_eq!(case.frame.schema_version, 2);
        assert!(!surface_names(&case.frame).is_empty());
        let bytes = canonical_bytes(&case.frame);
        provenance.push(provenance_entry(case, &bytes));
        if update {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(case.file_name), &bytes).expect("fixture export succeeds");
        }
    }
    let mut provenance_bytes =
        serde_json::to_vec_pretty(&provenance).expect("provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G07_WDIO_FIXTURES=1 cargo test -p helix-im --test g07_wdio_fixture_contract_test",
                path.display()
            )
        });
        assert_eq!(
            checked_in,
            canonical_bytes(&case.frame),
            "{} drifted from current Helix producer",
            path.display()
        );
        InteractionFrameV2::decode(&checked_in).expect("checked-in fixture remains current V2");
    }
    assert_eq!(
        fs::read(directory.join("provenance.json")).expect("provenance exists"),
        provenance_bytes,
        "provenance hashes or producer identity drifted"
    );
}
