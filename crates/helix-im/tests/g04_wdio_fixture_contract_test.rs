//! G04 WDIO Frame V2/V3 canonical Fixture 真源。
//!
//! 所有文件来自共享 `G04ScenarioRunner` 的生产 `ImModule::handle` 回放；普通测试只比较
//! actual Frame 与 checked-in Golden，只有 `UPDATE_G04_WDIO_FIXTURES=1` 才允许写盘。

#![recursion_limit = "256"]

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{
    InteractionFrameV2, TimelineDeltaOperationVm, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "support/g04_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_recalled_contract, assert_revocable_contract,
    first_message, g04_scenario_spec, sha256_hex, surface_names, G04ScenarioRunner, ACTOR_ID,
    CHANNEL_ID, CREATED_AT, GROUP_NAME, MARKER, OBSERVER_ID, REQUEST_ID, SERVER_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "b89c043899d2d137306c59b2f77dd87ec300c9c4";
const GENERATED_AT: &str = "2026-07-28T00:35:28-07:00";
const PRODUCER: &str =
    "G04ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一个 canonical G04 Fixture 的稳定 ID、文件名和 producer 输出。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成建群、可撤回、权威撤回与恢复四份 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let ws = G04ScenarioRunner::run_ws_revoke();
    let failed = G04ScenarioRunner::run_http_failure();
    let replayed = G04ScenarioRunner::run_persist_failure_replay();
    let recovered = G04ScenarioRunner::run_recovery();
    assert_eq!(
        ws.group_created, failed.group_created,
        "future revoke terminal cannot change the fresh-group prerequisite"
    );
    assert_eq!(
        ws.revocable, failed.revocable,
        "HTTP failure cannot change the pre-authority timeline"
    );
    assert_eq!(
        ws.recalled, replayed.recalled_after_replay,
        "Persist retry must converge on the same recalled Frame"
    );
    let spec = g04_scenario_spec();
    assert_group_created_contract_for(&ws.group_created, &spec);
    assert_revocable_contract(&ws.revocable);
    assert_recalled_contract(&ws.recalled, Some(REQUEST_ID));
    assert_recalled_contract(&recovered.recovered, None);
    assert_eq!(
        timeline_operation_kinds(&ws.recalled),
        Some(vec!["upsertMessage"])
    );
    assert_eq!(first_message(&ws.recalled).event_seq, 1);
    vec![
        FixtureCase {
            fixture_id: "g04-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: ws.group_created,
        },
        FixtureCase {
            fixture_id: "g04-revocable-v3",
            file_name: "revocable.frame-v3.json",
            frame: ws.revocable,
        },
        FixtureCase {
            fixture_id: "g04-recalled-v3",
            file_name: "recalled.frame-v3.json",
            frame: ws.recalled,
        },
        FixtureCase {
            fixture_id: "g04-recovered-recalled-v3",
            file_name: "recovered-recalled.frame-v3.json",
            frame: recovered.recovered,
        },
    ]
}

/// 生成仓库稳定的 pretty JSON bytes，并固定末尾换行。
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

/// 提取 G04 content、delivery 与 revoke action 的稳定业务字段。
fn message_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let has_timeline = frame.mutations.iter().any(|mutation| {
        matches!(
            mutation.mutation.mutation,
            TypedSurfaceMutation::TimelineSnapshot { .. }
                | TypedSurfaceMutation::TimelineDelta { .. }
        )
    });
    let message = has_timeline.then(|| first_message(frame))?;
    let revoke = message
        .actions
        .iter()
        .find(|action| action.kind == "revoke");
    Some(json!({
        "id": message.id,
        "content": message.content,
        "delivery": message.delivery,
        "actionsEmpty": message.actions.is_empty(),
        "quickActionsEmpty": message.quick_actions.is_empty(),
        "revokeAction": revoke.map(|action| json!({
            "kind": action.kind,
            "enabled": action.enabled,
            "disabledReason": action.disabled_reason,
        }))
    }))
}

/// 提取 fresh-group dialog 的稳定业务形状。
fn dialog_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let row = frame
        .mutations
        .iter()
        .find_map(|mutation| match &mutation.mutation.mutation {
            TypedSurfaceMutation::DialogListSnapshot { view } => view
                .sections
                .iter()
                .flat_map(|section| &section.items)
                .next(),
            TypedSurfaceMutation::DialogListPatch { patch } => patch
                .view
                .sections
                .iter()
                .flat_map(|section| &section.items)
                .next(),
            _ => None,
        })?;
    Some(json!({
        "channelId": row.channel_id,
        "memberCount": row.member_count,
        "lifecycle": row.lifecycle,
        "preview": row.preview,
    }))
}

/// 返回 Helix canonical G04 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g04-wdio")
}

/// 返回 Frame 的第一条 timeline messageId；dialog-only Frame 返回空。
fn optional_message_id(frame: &InteractionFrameV2) -> Option<&str> {
    frame
        .mutations
        .iter()
        .any(|mutation| {
            matches!(
                mutation.mutation.mutation,
                TypedSurfaceMutation::TimelineSnapshot { .. }
                    | TypedSurfaceMutation::TimelineDelta { .. }
            )
        })
        .then(|| first_message(frame).id.as_str())
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
            "/mutations/*/scope/userId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/anchor/messageId",
            "/mutations/*/mutation/view/entities/*/message/id",
            "/mutations/*/mutation/view/entities/*/message/temporaryId",
            "/mutations/*/mutation/view/entities/*/message/serverId",
            "/mutations/*/mutation/view/entities/*/message/channelId",
            "/mutations/*/mutation/view/entities/*/message/author/userId",
            "/mutations/*/mutation/view/entities/*/message/createdAt",
            "/mutations/*/mutation/view/entities/*/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/temporaryId",
            "/mutations/*/mutation/view/items/*/serverId",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/author/userId",
            "/mutations/*/mutation/view/items/*/createdAt",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/operations/*/entity/message/channelId",
            "/mutations/*/mutation/operations/*/window/anchor/messageId",
            "/mutations/*/mutation/operations/*/entity/message/id",
            "/mutations/*/mutation/operations/*/entity/message/temporaryId",
            "/mutations/*/mutation/operations/*/entity/message/serverId",
            "/mutations/*/mutation/operations/*/entity/message/channelId",
            "/mutations/*/mutation/operations/*/entity/message/author/userId",
            "/mutations/*/mutation/operations/*/entity/message/createdAt",
            "/mutations/*/mutation/operations/*/entity/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/patch/view/sections/*/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/view/sections/*/items/*/actions/*/surfaceEpoch"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/entities/*/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/operations/*/entity/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/sections/*/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/sections/*/items/*/actions/*/surfaceEpoch"
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
            "reqId": REQUEST_ID,
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
fn current_g04_producer_matches_canonical_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 4, "G04 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G04_WDIO_FIXTURES").is_some();
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
                "{} missing ({error}); run UPDATE_G04_WDIO_FIXTURES=1 cargo test -p helix-im --test g04_wdio_fixture_contract_test",
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
