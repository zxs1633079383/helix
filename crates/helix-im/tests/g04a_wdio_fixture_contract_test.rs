//! G04a WDIO Frame V2 canonical Fixture 真源。
//!
//! 所有文件来自共享 `G04AScenarioRunner` 的生产 `ImModule::handle` 回放；普通测试只比较
//! actual Frame 与 checked-in Golden，只有 `UPDATE_G04A_WDIO_FIXTURES=1` 才允许写盘。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};

#[path = "support/g04a_scenario.rs"]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_offline_recalled_contract, assert_revocable_contract,
    first_message, g04a_scenario_spec, sha256_hex, surface_names, G04AScenarioRunner, ACTOR_ID,
    CHANNEL_ID, CREATED_AT, GROUP_NAME, MARKER, OBSERVER_ID, REQUEST_ID, SERVER_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "c9c37ea8fcb398715cff872d4adbdc8a0e412c46";
const GENERATED_AT: &str = "2026-07-25T10:30:00-07:00";
const PRODUCER: &str =
    "G04AScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 描述一个 canonical G04a Fixture 的稳定 ID、文件名和 producer 输出。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成建群、可撤回、权威撤回与恢复四份 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let offline = G04AScenarioRunner::run_offline_revoke();
    let no_change = G04AScenarioRunner::run_no_change();
    let replayed = G04AScenarioRunner::run_persist_failure_replay();
    let recovered = G04AScenarioRunner::run_recovery();
    assert_eq!(
        offline.group_created, no_change.group_created,
        "future offline authority cannot change the fresh-group prerequisite"
    );
    assert_eq!(
        offline.revocable, no_change.revocable,
        "no-change cannot change the pre-authority timeline"
    );
    assert_eq!(
        offline.recalled, replayed.recalled_after_replay,
        "Persist retry must converge on the same recalled Frame"
    );
    let spec = g04a_scenario_spec();
    assert_group_created_contract_for(&offline.group_created, &spec);
    assert_revocable_contract(&offline.revocable);
    assert_offline_recalled_contract(&offline.recalled);
    assert_offline_recalled_contract(&recovered.recovered);
    assert_eq!(first_message(&offline.recalled).event_seq, 1);
    vec![
        FixtureCase {
            fixture_id: "g04a-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: offline.group_created,
        },
        FixtureCase {
            fixture_id: "g04a-revocable-v2",
            file_name: "revocable.frame-v2.json",
            frame: offline.revocable,
        },
        FixtureCase {
            fixture_id: "g04a-offline-recalled-v2",
            file_name: "offline-recalled.frame-v2.json",
            frame: offline.recalled,
        },
        FixtureCase {
            fixture_id: "g04a-recovered-recalled-v2",
            file_name: "recovered-recalled.frame-v2.json",
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

/// 生成排除 transaction 身份但保留 G04a 业务语义的摘要。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mutation_kinds: Vec<&str> = frame
        .mutations
        .iter()
        .map(|mutation| match mutation.mutation.mutation {
            TypedSurfaceMutation::TimelineSnapshot { .. } => "timelineSnapshot",
            TypedSurfaceMutation::TimelineDelta { .. } => "timelineDelta",
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
        "messageContract": message_contract(frame),
        "dialogContract": dialog_contract(frame),
    }))
    .expect("semantic projection serializes")
}

/// 提取 G04a content、delivery 与 revoke action 的稳定业务字段。
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

/// 返回 Helix canonical G04a Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g04a-wdio")
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
            "/mutations/*/scope/userId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/anchor/messageId",
            "/mutations/*/mutation/view/items/*/id",
            "/mutations/*/mutation/view/items/*/channelId",
            "/mutations/*/mutation/view/items/*/author/userId",
            "/mutations/*/mutation/view/items/*/createdAt",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/anchor/messageId",
            "/mutations/*/mutation/patch/view/items/*/id",
            "/mutations/*/mutation/patch/view/items/*/channelId",
            "/mutations/*/mutation/patch/view/items/*/author/userId",
            "/mutations/*/mutation/patch/view/items/*/createdAt",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch",
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
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch",
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
fn current_g04a_producer_matches_canonical_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 4, "G04a canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G04A_WDIO_FIXTURES").is_some();
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
                "{} missing ({error}); run UPDATE_G04A_WDIO_FIXTURES=1 cargo test -p helix-im --test g04a_wdio_fixture_contract_test",
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
