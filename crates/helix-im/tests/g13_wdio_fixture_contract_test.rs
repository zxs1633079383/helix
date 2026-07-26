//! G13 WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较 production producer 与 checked-in Golden；仅
//! `UPDATE_G13_WDIO_FIXTURES=1` 允许覆盖 canonical JSON。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};

#[path = "support/g13_scenario.rs"]
#[allow(dead_code)]
mod scenario;

use scenario::{
    assert_group_created_contract_for, assert_initial_contract, assert_terminal_contract,
    g13_scenario_spec, sha256_hex, surface_names, G13ScenarioRunner, ACTOR_ID, CHANNEL_ID,
    CREATED_AT, GROUP_NAME, OBSERVER_ID, TEAM_ID,
};

const HELIX_COMMIT: &str = "a13e17151efd1e7eefc94677110598b6347b08a9";
const GENERATED_AT: &str = "2026-07-26T09:30:00-07:00";
const PRODUCER: &str =
    "G13ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture id、文件名与 production 回放得到的完整 Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 G13 ScenarioRunner 生成前置、未读、终态与恢复 Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let success = G13ScenarioRunner::run_success();
    let recovery = G13ScenarioRunner::run_recovery();
    assert_group_created_contract_for(&success.group_created, &g13_scenario_spec());
    assert_initial_contract(&success.initial);
    assert_terminal_contract(&success.terminal);
    recovery
        .validate()
        .expect("G13 recovery Fixture remains valid");
    vec![
        FixtureCase {
            fixture_id: "g13-group-created-v2",
            file_name: "group-created.frame-v2.json",
            frame: success.group_created,
        },
        FixtureCase {
            fixture_id: "g13-dialog-unread-initial-v2",
            file_name: "dialog-unread-initial.frame-v2.json",
            frame: success.initial,
        },
        FixtureCase {
            fixture_id: "g13-channel-read-v2",
            file_name: "channel-read.frame-v2.json",
            frame: success.terminal,
        },
        FixtureCase {
            fixture_id: "g13-channel-read-recovery-v2",
            file_name: "channel-read-recovery.frame-v2.json",
            frame: recovery,
        },
    ]
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G13 Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取排除运行期 identity 后仍须跨端一致的 G13 业务字段。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schemaVersion": frame.schema_version,
        "runtime": frame.runtime,
        "surfaces": surface_names(frame),
        "dialogUnread": dialog_unread_contract(frame),
    }))
    .expect("G13 semantic projection serializes")
}

/// 提取 dialog row 与 unread aggregate 的稳定绝对态。
fn dialog_unread_contract(frame: &InteractionFrameV2) -> Value {
    let mut dialog = Value::Null;
    let mut unread = Value::Null;
    for mutation in &frame.mutations {
        match &mutation.mutation.mutation {
            TypedSurfaceMutation::DialogListSnapshot { view } => {
                dialog = serde_json::to_value(view).expect("G13 dialog snapshot serializes")
            }
            TypedSurfaceMutation::DialogListPatch { patch } => {
                dialog = serde_json::to_value(&patch.view).expect("G13 dialog patch serializes")
            }
            TypedSurfaceMutation::UnreadSummarySnapshot { view } => {
                unread = serde_json::to_value(view).expect("G13 unread snapshot serializes")
            }
            TypedSurfaceMutation::UnreadSummaryPatch { patch } => {
                unread = serde_json::to_value(&patch.view).expect("G13 unread patch serializes")
            }
            _ => {}
        }
    }
    json!({"dialog": dialog, "unread": unread})
}

/// 返回 Helix canonical G13 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g13-wdio")
}

/// 为一份 G13 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    let first_mutation = case
        .frame
        .mutations
        .first()
        .expect("G13 fixture always contains at least one mutation");
    json!({
        "fixtureId": case.fixture_id,
        "fileName": case.file_name,
        "gateId": "G-13",
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
            "/mutations/*/scope/userId",
            "/mutations/*/scope/teamId",
            "/mutations/*/mutation/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/view/sections/*/items/*/avatar/fallbackText",
            "/mutations/*/mutation/view/sections/*/items/*/lastActivityAt",
            "/mutations/*/mutation/patch/view/sections/*/items/*/channelId",
            "/mutations/*/mutation/patch/view/sections/*/items/*/title/text",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/userIds/*",
            "/mutations/*/mutation/patch/view/sections/*/items/*/avatar/fallbackText",
            "/mutations/*/mutation/patch/view/sections/*/items/*/lastActivityAt"
        ],
        "rebaseFields": [
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision"
        ],
        "placeholderValues": {
            "actorId": ACTOR_ID,
            "observerId": OBSERVER_ID,
            "teamId": TEAM_ID,
            "channelId": CHANNEL_ID,
            "groupName": GROUP_NAME,
            "createdAt": CREATED_AT,
            "transactionId": case.frame.transaction_id,
            "surfaceEpoch": first_mutation.surface_epoch,
            "revision": first_mutation.mutation.revision,
            "baseRevision": first_mutation.mutation.base_revision.unwrap_or(0)
        }
    })
}

/// 证明 actual producer、字段语义、Golden bytes 与 provenance 始终同源。
#[test]
fn current_g13_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 4, "G13 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G13_WDIO_FIXTURES").is_some();
    let mut provenance = Vec::with_capacity(cases.len());
    for case in &cases {
        case.frame.validate().expect("producer Frame remains valid");
        assert_eq!(case.frame.schema_version, 2);
        let bytes = canonical_bytes(&case.frame);
        provenance.push(provenance_entry(case, &bytes));
        if update {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(case.file_name), &bytes).expect("fixture export succeeds");
        }
    }
    let mut provenance_bytes =
        serde_json::to_vec_pretty(&provenance).expect("G13 provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G13_WDIO_FIXTURES=1 cargo test -p helix-im --test g13_wdio_fixture_contract_test",
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
        "G13 provenance hashes or producer identity drifted"
    );
}
