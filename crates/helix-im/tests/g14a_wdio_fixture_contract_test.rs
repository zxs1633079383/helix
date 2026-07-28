//! G14a WDIO/Mobile Frame V2/V3 canonical Fixture 真源。
//!
//! 所有文件来自共享 `G14AScenarioRunner` 的生产 `ImModule::handle` 回放；普通测试只比较
//! actual Frame 与 checked-in Golden，只有 `UPDATE_G14A_WDIO_FIXTURES=1` 才允许写盘。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{
    InteractionFrameV2, TimelineDeltaOperationVm, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "support/g14a_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_receivable_contract, assert_received_contract,
    first_message, g14a_scenario_spec, sha256_hex, surface_names, G14AScenarioRunner, ACTOR_ID,
    CHANNEL_ID, CREATED_AT, GROUP_NAME, MARKER, OBSERVER_ID, REQUEST_ID, SERVER_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "b89c043899d2d137306c59b2f77dd87ec300c9c4";
const GENERATED_AT: &str = "2026-07-28T00:35:28-07:00";
const PRODUCER: &str =
    "G14AScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一份 canonical G14a Fixture 的稳定 ID、文件名和 producer 输出。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成建群、可接收、权威已收到与恢复四份 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let ws = G14AScenarioRunner::run_ws_received();
    let failed = G14AScenarioRunner::run_http_failure();
    let replayed = G14AScenarioRunner::run_persist_failure_replay();
    let recovered = G14AScenarioRunner::run_recovery();
    assert_eq!(
        ws.group_created, failed.group_created,
        "future receipt outcome cannot change the fresh-group prerequisite"
    );
    assert_eq!(
        ws.receivable, failed.receivable,
        "HTTP failure cannot change the pre-authority timeline"
    );
    assert_eq!(
        ws.received, replayed.received_after_replay,
        "Persist retry must converge on the same received Frame"
    );
    let spec = g14a_scenario_spec();
    assert_group_created_contract_for(&ws.group_created, &spec);
    assert_receivable_contract(&ws.receivable);
    assert_received_contract(&ws.received);
    assert_received_contract(&recovered.recovered);
    assert_eq!(
        timeline_operation_kinds(&ws.received),
        Some(vec!["upsertMessage"])
    );
    vec![
        FixtureCase {
            fixture_id: "g14a-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: ws.group_created,
        },
        FixtureCase {
            fixture_id: "g14a-template-receivable-v3",
            file_name: "template-receivable.frame-v3.json",
            frame: ws.receivable,
        },
        FixtureCase {
            fixture_id: "g14a-template-received-v3",
            file_name: "template-received.frame-v3.json",
            frame: ws.received,
        },
        FixtureCase {
            fixture_id: "g14a-template-received-recovery-v3",
            file_name: "template-received-recovery.frame-v3.json",
            frame: recovered.recovered,
        },
    ]
}

/// 生成仓库稳定的 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G14a Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 生成排除运行期 identity 后仍须跨端一致的 G14a 业务摘要。
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

/// 提取模板内容、终态、确认字段、普通回执边界与 templateConfirm action。
fn message_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let has_timeline = frame.mutations.iter().any(|mutation| {
        matches!(
            mutation.mutation.mutation,
            TypedSurfaceMutation::TimelineSnapshot { .. }
                | TypedSurfaceMutation::TimelineDelta { .. }
        )
    });
    let message = has_timeline.then(|| first_message(frame))?;
    let confirm = message
        .actions
        .iter()
        .find(|action| action.kind == "templateConfirm");
    Some(json!({
        "id": message.id,
        "author": message.author,
        "content": message.content,
        "delivery": message.delivery,
        "readReceipt": message.read_receipt,
        "templateConfirmation": message.template_confirmation,
        "templateConfirmAction": confirm.map(|action| json!({
            "kind": action.kind,
            "inputShape": action.input_shape,
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

/// 返回 Helix canonical G14a Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g14a-wdio")
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
        "gateId": "G-14a",
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
            "/mutations/*/scope/windowToken",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/anchor/messageId",
            "/mutations/*/mutation/view/entities/*/message/id",
            "/mutations/*/mutation/view/entities/*/message/channelId",
            "/mutations/*/mutation/view/entities/*/message/author/userId",
            "/mutations/*/mutation/view/entities/*/message/createdAt",
            "/mutations/*/mutation/view/entities/*/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/entities/*/message/templateConfirmation/readerIds/*",
            "/mutations/*/mutation/operations/*/entity/message/channelId",
            "/mutations/*/mutation/operations/*/window/anchor/messageId",
            "/mutations/*/mutation/operations/*/entity/message/id",
            "/mutations/*/mutation/operations/*/entity/message/channelId",
            "/mutations/*/mutation/operations/*/entity/message/author/userId",
            "/mutations/*/mutation/operations/*/entity/message/createdAt",
            "/mutations/*/mutation/operations/*/entity/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/operations/*/entity/message/templateConfirmation/readerIds/*",
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

/// 证明 actual producer、字段语义、Golden bytes 与 provenance 始终同源。
#[test]
fn current_g14a_producer_matches_canonical_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 4, "G14a canonical inventory is explicit");
    let receivable = serde_json::to_value(
        first_message(&cases[1].frame)
            .template_confirmation
            .as_ref()
            .expect("receivable template carries confirmation fields"),
    )
    .expect("receivable template confirmation serializes");
    let received = serde_json::to_value(
        first_message(&cases[2].frame)
            .template_confirmation
            .as_ref()
            .expect("received template carries confirmation fields"),
    )
    .expect("received template confirmation serializes");
    assert_eq!(receivable["templateReceived"], false);
    assert_eq!(received["templateReceived"], true);
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G14A_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("G14a provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }

    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G14A_WDIO_FIXTURES=1 cargo test -p helix-im --test g14a_wdio_fixture_contract_test",
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
        "G14a provenance hashes or producer identity drifted"
    );
}
