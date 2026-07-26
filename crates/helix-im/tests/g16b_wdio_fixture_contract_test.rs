//! G16b WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较 actual 与 Golden；仅 `UPDATE_G16B_WDIO_FIXTURES=1` 允许覆盖。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "support/g16b_scenario.rs"]
mod scenario;

use scenario::{
    assert_roster_contract, G16bScenarioRunner, CHANNEL_ID, LEAVING_USER_ID, REMAINING_USER_ID,
};

const HELIX_COMMIT: &str = "8c9a9706cb865d0849b17b91485f2e6f2480a4bd";
const GENERATED_AT: &str = "2026-07-26T23:40:00-07:00";
const PRODUCER: &str =
    "G16bScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture identity 与真实 producer Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成 attach、durable leave patch 与 recovery snapshot。
fn produce_cases() -> Vec<FixtureCase> {
    let leave = G16bScenarioRunner::run_leave();
    let recovery = G16bScenarioRunner::run_recovery();
    assert_roster_contract(&leave.initial, &[LEAVING_USER_ID, REMAINING_USER_ID], false);
    assert_roster_contract(&leave.terminal, &[REMAINING_USER_ID], true);
    assert_roster_contract(&recovery, &[REMAINING_USER_ID], false);
    vec![
        FixtureCase {
            fixture_id: "g16b-member-roster-active-v2",
            file_name: "member-roster-active.frame-v2.json",
            frame: leave.initial,
        },
        FixtureCase {
            fixture_id: "g16b-member-roster-left-v2",
            file_name: "member-roster-left.frame-v2.json",
            frame: leave.terminal,
        },
        FixtureCase {
            fixture_id: "g16b-member-roster-recovery-v2",
            file_name: "member-roster-recovery.frame-v2.json",
            frame: recovery,
        },
    ]
}

/// 生成稳定 pretty JSON bytes 并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G16b Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取跨端必须一致的 roster 成员与 revision 业务语义。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let roster = frame.mutations.iter().find_map(|mutation| {
        let view = match &mutation.mutation.mutation {
            TypedSurfaceMutation::MemberRosterSnapshot { view } => view,
            TypedSurfaceMutation::MemberRosterPatch { patch } => &patch.view,
            _ => return None,
        };
        Some(json!({
            "channelId":view.channel_id,
            "memberCount":view.member_count,
            "users":view.items.iter().map(|item| json!({
                "userId":item.user.user_id,
                "role":item.role,
                "membershipState":item.membership_state,
                "actions":item.actions.iter().map(|action| json!({
                    "kind":action.kind,
                    "enabled":action.enabled,
                    "disabledReason":action.disabled_reason,
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        }))
    });
    serde_json::to_vec(&json!({
        "schemaVersion":frame.schema_version,
        "surfaces":surface_names(frame),
        "roster":roster
    }))
    .expect("G16b semantic projection serializes")
}

/// 返回 Frame 的稳定 surface inventory。
fn surface_names(frame: &InteractionFrameV2) -> BTreeSet<&'static str> {
    frame
        .mutations
        .iter()
        .map(|mutation| mutation.mutation.surface.as_str())
        .collect()
}

/// 计算 provenance 使用的小写 SHA-256。
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 返回 Helix canonical G16b Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g16b-wdio")
}

/// 为一份 Fixture 生成 producer、hash、surface 与允许动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId":case.fixture_id,
        "fileName":case.file_name,
        "gateId":"G-16b",
        "schemaVersion":case.frame.schema_version,
        "helixCommit":HELIX_COMMIT,
        "producer":PRODUCER,
        "sourceKind":"rust-export",
        "semanticSha256":sha256_hex(&semantic_bytes(&case.frame)),
        "fixtureSha256":sha256_hex(bytes),
        "surfaces":surface_names(&case.frame),
        "generatedAt":GENERATED_AT,
        "parameterizableFields":[
            "/transactionId",
            "/causationId",
            "/mutations/*/causationId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/scope/channelId",
            "/mutations/*/mutation/view/channelId",
            "/mutations/*/mutation/view/items/*/user/userId",
            "/mutations/*/mutation/view/items/*/nickname",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/items/*/user/userId",
            "/mutations/*/mutation/patch/view/items/*/nickname",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch"
        ],
        "rebaseFields":[
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/items/*/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/items/*/actions/*/surfaceEpoch"
        ],
        "placeholderValues":{
            "channelId":CHANNEL_ID,
            "leavingUserId":LEAVING_USER_ID,
            "remainingUserId":REMAINING_USER_ID,
            "transactionId":case.frame.transaction_id,
            "causationId":case.frame.causation_id,
            "surfaceEpoch":case.frame.mutations[0].surface_epoch,
            "revision":case.frame.mutations[0].mutation.revision,
            "baseRevision":case.frame.mutations[0].mutation.base_revision
        }
    })
}

#[test]
/// 证明真实 producer、完整字段、Golden bytes 与 provenance 始终同源。
fn current_g16b_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 3, "G16b canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G16B_WDIO_FIXTURES").is_some();
    let mut provenance = Vec::with_capacity(cases.len());
    for case in &cases {
        case.frame.validate().expect("producer Frame remains valid");
        let bytes = canonical_bytes(&case.frame);
        provenance.push(provenance_entry(case, &bytes));
        if update {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(case.file_name), &bytes).expect("fixture export succeeds");
        }
    }
    let mut provenance_bytes =
        serde_json::to_vec_pretty(&provenance).expect("G16b provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G16B_WDIO_FIXTURES=1 cargo test -p helix-im --test g16b_wdio_fixture_contract_test",
                path.display()
            )
        });
        assert_eq!(checked_in, canonical_bytes(&case.frame));
        InteractionFrameV2::decode(&checked_in).expect("checked-in fixture remains current V2");
    }
    assert_eq!(
        fs::read(directory.join("provenance.json")).expect("provenance exists"),
        provenance_bytes
    );
}
