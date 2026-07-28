//! G11 WDIO/Mobile Frame V2/V3 canonical Fixture 真源。
//!
//! 普通测试只比较 production producer 与 checked-in Golden；仅
//! `UPDATE_G11_WDIO_FIXTURES=1` 允许覆盖 canonical JSON。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{
    InteractionFrameV2, TimelineDeltaOperationVm, TypedSurfaceMutation,
};
use serde_json::{json, Value};

#[path = "support/g11_scenario.rs"]
#[allow(dead_code)]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_located_contract, g11_scenario_spec, sha256_hex,
    surface_names, timeline_messages, G11ScenarioRunner, ACTOR_ID, CHANNEL_ID, CREATED_AT,
    GROUP_NAME, OBSERVER_ID, TARGET_ID, TEAM_ID, WINDOW_TOKEN,
};

const HELIX_COMMIT: &str = "b89c043899d2d137306c59b2f77dd87ec300c9c4";
const GENERATED_AT: &str = "2026-07-28T00:35:28-07:00";
const PRODUCER: &str =
    "G11ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture id、文件名与 production 回放得到的完整 Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 G11 ScenarioRunner 生成建群、bounded timeline 与 located 三份完整 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let success = G11ScenarioRunner::run_success();
    assert_group_created_contract_for(&success.group_created, &g11_scenario_spec());
    assert_located_contract(&success.located);
    assert_eq!(timeline_operation_kinds(&success.initial), None);
    let located_operations =
        timeline_operation_kinds(&success.located).expect("locate must use TimelineDelta");
    assert_eq!(located_operations.last(), Some(&"activateWindow"));
    assert_eq!(
        located_operations
            .iter()
            .filter(|kind| **kind == "activateWindow")
            .count(),
        1
    );
    vec![
        FixtureCase {
            fixture_id: "g11-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: success.group_created,
        },
        FixtureCase {
            fixture_id: "g11-timeline-initial-v3",
            file_name: "timeline-initial.frame-v3.json",
            frame: success.initial,
        },
        FixtureCase {
            fixture_id: "g11-timeline-located-v3",
            file_name: "timeline-located.frame-v3.json",
            frame: success.located,
        },
    ]
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G11 Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取排除运行期 identity 后仍须跨端一致的 G11 业务字段。
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

/// 提取 timeline 的 anchor、target、items、page、state 与 action 能力。
fn timeline_contract(frame: &InteractionFrameV2) -> Option<Value> {
    let mutation = frame
        .mutations
        .iter()
        .find(|mutation| mutation.mutation.surface.as_str() == "message.timeline")?;
    let messages = timeline_messages(frame);
    let items = messages
        .iter()
        .map(|item| {
            json!({
                "id": item.id,
                "channelId": item.channel_id,
                "author": item.author,
                "content": item.content,
                "delivery": item.delivery,
                "actions": item.actions.iter().map(|action| json!({
                    "kind": action.kind,
                    "inputShape": action.input_shape,
                    "enabled": action.enabled,
                    "disabledReason": action.disabled_reason,
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    match &mutation.mutation.mutation {
        TypedSurfaceMutation::TimelineSnapshot { view } => Some(json!({
            "kind": "snapshot",
            "channelId": view.channel_id,
            "anchor": view.anchor,
            "targetMessageId": view.target_message_id,
            "items": items,
            "page": view.page,
            "state": view.state,
        })),
        TypedSurfaceMutation::TimelineDelta { operations } => Some(json!({
            "kind": "delta",
            "operationKinds": operations.iter().map(timeline_operation_kind).collect::<Vec<_>>(),
            "items": items,
            "activeWindow": operations.iter().find_map(|operation| match operation {
                TimelineDeltaOperationVm::ActivateWindow { window, navigation_token } =>
                    Some(json!({"window": window, "navigationToken": navigation_token})),
                _ => None,
            }),
        })),
        _ => None,
    }
}

/// 将 Timeline V3 操作投影为跨端稳定的 closed-union 名称。
fn timeline_operation_kind(operation: &TimelineDeltaOperationVm) -> &'static str {
    match operation {
        TimelineDeltaOperationVm::UpsertMessage { .. } => "upsertMessage",
        TimelineDeltaOperationVm::InstallSegment { .. } => "installSegment",
        TimelineDeltaOperationVm::InsertKeys { .. } => "insertKeys",
        TimelineDeltaOperationVm::MergeRanges { .. } => "mergeRanges",
        TimelineDeltaOperationVm::SetRangeBounds { .. } => "setRangeBounds",
        TimelineDeltaOperationVm::ActivateWindow { .. } => "activateWindow",
    }
}

/// 返回 Helix canonical G11 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g11-wdio")
}

/// 为一份 G11 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    let mutation = &case.frame.mutations[0];
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
            "/directives/*/directiveId",
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
            "/mutations/*/scope/windowToken",
            "/mutations/*/scope/userId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/page/windowToken",
            "/mutations/*/mutation/view/entities/*/message/id",
            "/mutations/*/mutation/view/entities/*/message/channelId",
            "/mutations/*/mutation/view/entities/*/message/author/userId",
            "/mutations/*/mutation/view/entities/*/message/createdAt",
            "/mutations/*/mutation/view/entities/*/message/content/text",
            "/mutations/*/mutation/view/entities/*/message/actions/*/actionRef",
            "/mutations/*/mutation/view/entities/*/message/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/fallbackText",
            "/mutations/*/mutation/operations/*/entity/message/channelId",
            "/mutations/*/mutation/operations/*/window/anchor/messageId",
            "/mutations/*/mutation/patch/view/targetMessageId",
            "/mutations/*/mutation/operations/*/window/windowToken",
            "/mutations/*/mutation/operations/*/entity/message/id",
            "/mutations/*/mutation/operations/*/entity/message/channelId",
            "/mutations/*/mutation/operations/*/entity/message/author/userId",
            "/mutations/*/mutation/operations/*/entity/message/createdAt",
            "/mutations/*/mutation/operations/*/entity/message/content/text",
            "/mutations/*/mutation/operations/*/entity/message/actions/*/actionRef",
            "/mutations/*/mutation/operations/*/entity/message/actions/*/surfaceEpoch"
        ],
        "rebaseFields": [
            "/transactionId",
            "/directives/*/directiveId",
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
            "windowToken": WINDOW_TOKEN,
            "targetMessageId": TARGET_ID,
            "groupName": GROUP_NAME,
            "createdAt": CREATED_AT,
            "transactionId": case.frame.transaction_id,
            "surfaceEpoch": mutation.surface_epoch,
            "revision": mutation.mutation.revision,
            "baseRevision": mutation.mutation.base_revision
        }
    })
}

/// 证明 actual producer、字段语义、Golden bytes 与 provenance 始终同源。
#[test]
fn current_g11_producer_matches_canonical_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 3, "G11 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G11_WDIO_FIXTURES").is_some();
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
        let bytes = canonical_bytes(&case.frame);
        provenance.push(provenance_entry(case, &bytes));
        if update {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(case.file_name), &bytes).expect("fixture export succeeds");
        }
    }
    let mut provenance_bytes =
        serde_json::to_vec_pretty(&provenance).expect("G11 provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G11_WDIO_FIXTURES=1 cargo test -p helix-im --test g11_wdio_fixture_contract_test",
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
        "G11 provenance hashes or producer identity drifted"
    );
}
