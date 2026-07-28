//! G07 WDIO Frame V2/V3 canonical Fixture 真源。
//!
//! 所有文件来自共享 `G07ScenarioRunner` 的 production `ImModule::handle` 回放；普通测试
//! 只比较 actual Frame 与 checked-in Golden，只有 `UPDATE_G07_WDIO_FIXTURES=1` 才写盘。

#![recursion_limit = "256"]

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceScope, TimelineDeltaOperationVm, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "support/g07_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_urgent_receipts_contract,
    assert_urgent_timeline_contract, assert_urgentable_contract, first_message, g07_scenario_spec,
    sha256_hex, surface_names, G07ScenarioRunner, ACTOR_ID, CHANNEL_ID, CREATED_AT, GROUP_NAME,
    MARKER, OBSERVER_ID, REQUEST_ID, SERVER_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "b89c043899d2d137306c59b2f77dd87ec300c9c4";
const GENERATED_AT: &str = "2026-07-28T00:35:28-07:00";
const PRODUCER: &str =
    "G07ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一份 G07 canonical Frame 的稳定 ID、文件名与 producer 输出。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成建群、成员、加急、确认及两类 receipt 共七份 Frame。
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
    assert_member_roster_contract(&post.member_roster);
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
    assert_eq!(
        timeline_operation_kinds(&post.urgent_active),
        Some(vec!["upsertMessage"])
    );
    assert_eq!(
        timeline_operation_kinds(&confirm.urgent_confirmed),
        Some(vec!["upsertMessage"])
    );
    assert_urgent_receipts_contract(&confirm.receipts_completed, true);
    vec![
        FixtureCase {
            fixture_id: "g07-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: post.group_created,
        },
        FixtureCase {
            fixture_id: "g07-member-roster-v2",
            file_name: "member-roster.frame-v2.json",
            frame: post.member_roster,
        },
        FixtureCase {
            fixture_id: "g07-urgentable-v3",
            file_name: "urgentable.frame-v3.json",
            frame: post.urgentable,
        },
        FixtureCase {
            fixture_id: "g07-urgent-active-v3",
            file_name: "urgent-active.frame-v3.json",
            frame: post.urgent_active,
        },
        FixtureCase {
            fixture_id: "g07-receipts-pending-v2",
            file_name: "receipts-pending.frame-v2.json",
            frame: post.receipts_pending,
        },
        FixtureCase {
            fixture_id: "g07-urgent-confirmed-v3",
            file_name: "urgent-confirmed.frame-v3.json",
            frame: confirm.urgent_confirmed,
        },
        FixtureCase {
            fixture_id: "g07-receipts-completed-v2",
            file_name: "receipts-completed.frame-v2.json",
            frame: confirm.receipts_completed,
        },
    ]
}

/// 断言加急选择器消费的成员 roster 完整包含 Actor 与 Observer。
fn assert_member_roster_contract(frame: &InteractionFrameV2) {
    let view = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::MemberRosterSnapshot { view } => Some(view),
            TypedSurfaceMutation::MemberRosterPatch { patch } => Some(&patch.view),
            _ => None,
        })
        .expect("G07 member roster Frame must contain a typed roster view");
    assert_eq!(view.channel_id, CHANNEL_ID);
    assert_eq!(view.member_count, 2);
    assert_eq!(
        view.items
            .iter()
            .map(|item| item.user.user_id.as_str())
            .collect::<Vec<_>>(),
        [ACTOR_ID, OBSERVER_ID]
    );
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 生成覆盖 operation/key/range/bounds/revision 的跨仓语义摘要。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "mutations": frame.mutations.iter().map(|mutation| json!({
            "surface": mutation.mutation.surface,
            "surfaceSchemaVersion": mutation.surface_schema_version,
            "surfaceEpoch": mutation.surface_epoch,
            "scope": mutation.mutation.scope,
            "revision": mutation.mutation.revision,
            "baseRevision": mutation.mutation.base_revision,
            "mutation": mutation.mutation.mutation,
        })).collect::<Vec<_>>(),
    }))
    .expect("semantic projection serializes")
}

/// 返回 TimelineDelta 的 closed-union operation 顺序；Snapshot 与非 Timeline 返回 None。
fn timeline_operation_kinds(frame: &InteractionFrameV2) -> Option<Vec<&'static str>> {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineDelta { operations } => Some(
                operations
                    .iter()
                    .map(|operation| match operation {
                        TimelineDeltaOperationVm::UpsertMessage { .. } => "upsertMessage",
                        TimelineDeltaOperationVm::InstallSegment { .. } => "installSegment",
                        TimelineDeltaOperationVm::InsertKeys { .. } => "insertKeys",
                        TimelineDeltaOperationVm::MergeRanges { .. } => "mergeRanges",
                        TimelineDeltaOperationVm::SetRangeBounds { .. } => "setRangeBounds",
                        TimelineDeltaOperationVm::ActivateWindow { .. } => "activateWindow",
                    })
                    .collect(),
            ),
            _ => None,
        })
}

/// 提取 timeline urgency 与 urgent actions 的稳定业务字段。
fn message_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let has_timeline = frame.mutations.iter().any(|mutation| {
        matches!(
            mutation.mutation.mutation,
            TypedSurfaceMutation::TimelineSnapshot { .. }
                | TypedSurfaceMutation::TimelineDelta { .. }
        )
    });
    let message = has_timeline.then(|| first_message(frame))?;
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
    if frame.mutations.iter().any(|mutation| {
        matches!(
            mutation.mutation.mutation,
            TypedSurfaceMutation::TimelineSnapshot { .. }
                | TypedSurfaceMutation::TimelineDelta { .. }
        )
    }) {
        return Some(first_message(frame).id.as_str());
    }
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::ParticipantReceiptsSnapshot { view } => {
                Some(view.message_id.as_str())
            }
            TypedSurfaceMutation::ParticipantReceiptsPatch { patch } => {
                Some(patch.view.message_id.as_str())
            }
            _ => None,
        })
}

/// 返回成员列表 scope 的可重绑定 cursor；其他 surface 不声明该参数。
fn optional_roster_cursor(frame: &InteractionFrameV2) -> Option<&str> {
    frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.scope {
            SurfaceScope::MemberRoster { cursor, .. } => Some(cursor.as_str()),
            _ => None,
        })
}

/// 为每份 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId": case.fixture_id,
        "fileName": case.file_name,
        "schemaVersion": case.frame.schema_version,
        "surfaceSchemaVersions": case.frame.mutations.iter().map(|mutation| json!({
            "surface": mutation.mutation.surface,
            "schemaVersion": mutation.surface_schema_version,
        })).collect::<Vec<_>>(),
        "helixCommit": HELIX_COMMIT,
        "producer": PRODUCER,
        "sourceKind": "rust-export",
        "semanticSha256": sha256_hex(&semantic_bytes(&case.frame)),
        "fixtureSha256": sha256_hex(bytes),
        "byteLength": bytes.len(),
        "surfaces": surface_names(&case.frame),
        "generatedAt": GENERATED_AT,
        "parameterizableFields": [
            "/transactionId",
            "/causationId",
            "/mutations/*/causationId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/surfaceSchemaVersion",
            "/mutations/*/mutation/view/entities/*/entityKey/createAt",
            "/mutations/*/mutation/view/entities/*/entityKey/temporaryId",
            "/mutations/*/mutation/view/ranges/*/rangeId",
            "/mutations/*/mutation/view/ranges/*/orderedKeys/*/createAt",
            "/mutations/*/mutation/view/ranges/*/orderedKeys/*/temporaryId",
            "/mutations/*/mutation/view/ranges/*/olderCursor/createAt",
            "/mutations/*/mutation/view/ranges/*/olderCursor/temporaryId",
            "/mutations/*/mutation/view/ranges/*/newerCursor/createAt",
            "/mutations/*/mutation/view/ranges/*/newerCursor/temporaryId",
            "/mutations/*/mutation/view/ranges/*/latestWatermark/createAt",
            "/mutations/*/mutation/view/ranges/*/latestWatermark/temporaryId",
            "/mutations/*/mutation/view/activeWindow/windowToken",
            "/mutations/*/mutation/view/activeWindow/rangeIds/*",
            "/mutations/*/mutation/view/persistedWatermark/createAt",
            "/mutations/*/mutation/view/persistedWatermark/temporaryId",
            "/mutations/*/mutation/operations/*/entity/entityKey/createAt",
            "/mutations/*/mutation/operations/*/entity/entityKey/temporaryId",
            "/mutations/*/mutation/operations/*/range/rangeId",
            "/mutations/*/mutation/operations/*/range/orderedKeys/*/createAt",
            "/mutations/*/mutation/operations/*/range/orderedKeys/*/temporaryId",
            "/mutations/*/mutation/operations/*/range/olderCursor/createAt",
            "/mutations/*/mutation/operations/*/range/olderCursor/temporaryId",
            "/mutations/*/mutation/operations/*/range/newerCursor/createAt",
            "/mutations/*/mutation/operations/*/range/newerCursor/temporaryId",
            "/mutations/*/mutation/operations/*/range/latestWatermark/createAt",
            "/mutations/*/mutation/operations/*/range/latestWatermark/temporaryId",
            "/mutations/*/mutation/operations/*/rangeId",
            "/mutations/*/mutation/operations/*/position/entityKey/createAt",
            "/mutations/*/mutation/operations/*/position/entityKey/temporaryId",
            "/mutations/*/mutation/operations/*/keys/*/createAt",
            "/mutations/*/mutation/operations/*/keys/*/temporaryId",
            "/mutations/*/mutation/operations/*/olderCursor/createAt",
            "/mutations/*/mutation/operations/*/olderCursor/temporaryId",
            "/mutations/*/mutation/operations/*/newerCursor/createAt",
            "/mutations/*/mutation/operations/*/newerCursor/temporaryId",
            "/mutations/*/mutation/operations/*/latestWatermark/createAt",
            "/mutations/*/mutation/operations/*/latestWatermark/temporaryId",
            "/mutations/*/mutation/operations/*/window/windowToken",
            "/mutations/*/mutation/operations/*/window/rangeIds/*",
            "/mutations/*/baseRevision",
            "/mutations/*/scope/channelId",
            "/mutations/*/scope/cursor",
            "/mutations/*/scope/messageId",
            "/mutations/*/scope/userId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/messageId",
            "/mutations/*/mutation/view/entities/*/message/id",
            "/mutations/*/mutation/view/entities/*/message/channelId",
            "/mutations/*/mutation/view/entities/*/message/author/userId",
            "/mutations/*/mutation/view/entities/*/message/user/userId",
            "/mutations/*/mutation/view/entities/*/message/createdAt",
            "/mutations/*/mutation/view/entities/*/message/content/text",
            "/mutations/*/mutation/view/entities/*/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/entities/*/message/urgency/urgentRequesterId",
            "/mutations/*/mutation/view/entities/*/message/urgency/urgentTargetIds/*",
            "/mutations/*/mutation/view/entities/*/message/urgency/urgentConfirmedIds/*",
            "/mutations/*/mutation/view/anchor/messageId",
            "/mutations/*/mutation/view/page/windowToken",
            "/mutations/*/mutation/view/completed/*/user/userId",
            "/mutations/*/mutation/view/completed/*/completedAt",
            "/mutations/*/mutation/view/pending/*/user/userId",
            "/mutations/*/mutation/view/summary/requesterId",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/view/sections/*/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/operations/*/entity/message/channelId",
            "/mutations/*/mutation/patch/view/messageId",
            "/mutations/*/mutation/operations/*/entity/message/id",
            "/mutations/*/mutation/operations/*/entity/message/channelId",
            "/mutations/*/mutation/operations/*/entity/message/author/userId",
            "/mutations/*/mutation/operations/*/entity/message/user/userId",
            "/mutations/*/mutation/operations/*/entity/message/createdAt",
            "/mutations/*/mutation/operations/*/entity/message/content/text",
            "/mutations/*/mutation/operations/*/entity/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/operations/*/entity/message/urgency/urgentRequesterId",
            "/mutations/*/mutation/operations/*/entity/message/urgency/urgentTargetIds/*",
            "/mutations/*/mutation/operations/*/entity/message/urgency/urgentConfirmedIds/*",
            "/mutations/*/mutation/operations/*/window/anchor/messageId",
            "/mutations/*/mutation/operations/*/window/windowToken",
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
            "/mutations/*/mutation/view/entities/*/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/operations/*/entity/message/actions/*/surfaceEpoch"
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
            "cursor": optional_roster_cursor(&case.frame),
            "surfaceEpoch": case.frame.mutations[0].surface_epoch,
            "revision": case.frame.mutations[0].mutation.revision,
            "baseRevision": case.frame.mutations[0].mutation.base_revision
        }
    })
}

#[test]
/// 证明 actual producer、字段语义、Golden bytes 与 provenance 始终同源。
fn current_g07_producer_matches_canonical_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 7, "G07 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G07_WDIO_FIXTURES").is_some();
    let mut provenance = Vec::with_capacity(cases.len());
    for case in &cases {
        case.frame.validate().expect("producer Frame remains valid");
        let expected_schema_version = if case
            .frame
            .mutations
            .iter()
            .any(|mutation| mutation.mutation.surface.as_str() == "message.timeline")
        {
            3
        } else {
            2
        };
        assert_eq!(case.frame.schema_version, expected_schema_version);
        let expected_suffix = if expected_schema_version == 3 {
            "v3"
        } else {
            "v2"
        };
        assert!(
            case.fixture_id.ends_with(expected_suffix),
            "fixtureId must expose the actual Frame schema"
        );
        assert!(
            case.file_name
                .ends_with(&format!(".frame-{expected_suffix}.json")),
            "file name must expose the actual Frame schema"
        );
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
