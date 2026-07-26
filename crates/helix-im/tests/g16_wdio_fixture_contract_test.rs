//! G16 WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较 actual 与 Golden；仅 `UPDATE_G16_WDIO_FIXTURES=1` 允许覆盖。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "support/g16_scenario.rs"]
mod scenario;

use scenario::{
    assert_lifecycle_contract, G16ScenarioRunner, ACTOR_ID, CHANNEL_ID, GROUP_NAME, TEAM_ID,
};

const HELIX_COMMIT: &str = "63b7eee5d356e1af08e32cc1fb92c2a466bc7841";
const GENERATED_AT: &str = "2026-07-26T21:45:00-07:00";
const PRODUCER: &str =
    "G16ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture identity 与真实 producer Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成 active、closed、left 与两种 recovery。
fn produce_cases() -> Vec<FixtureCase> {
    let closed = G16ScenarioRunner::run_closed();
    let left = G16ScenarioRunner::run_left();
    let closed_recovery = G16ScenarioRunner::run_recovery(false);
    let left_recovery = G16ScenarioRunner::run_recovery(true);
    assert_lifecycle_contract(&closed.initial, "active");
    assert_lifecycle_contract(&closed.terminal, "closed");
    assert_lifecycle_contract(&left.terminal, "left");
    assert_lifecycle_contract(&closed_recovery, "closed");
    assert_lifecycle_contract(&left_recovery, "left");
    vec![
        FixtureCase {
            fixture_id: "g16-channel-summary-active-v2",
            file_name: "channel-summary-active.frame-v2.json",
            frame: closed.initial,
        },
        FixtureCase {
            fixture_id: "g16-channel-summary-closed-v2",
            file_name: "channel-summary-closed.frame-v2.json",
            frame: closed.terminal,
        },
        FixtureCase {
            fixture_id: "g16-channel-summary-left-v2",
            file_name: "channel-summary-left.frame-v2.json",
            frame: left.terminal,
        },
        FixtureCase {
            fixture_id: "g16-channel-summary-closed-recovery-v2",
            file_name: "channel-summary-closed-recovery.frame-v2.json",
            frame: closed_recovery,
        },
        FixtureCase {
            fixture_id: "g16-channel-summary-left-recovery-v2",
            file_name: "channel-summary-left-recovery.frame-v2.json",
            frame: left_recovery,
        },
    ]
}

/// 生成稳定 pretty JSON bytes 并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G16 Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取跨端必须一致的 lifecycle 业务语义。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let lifecycle = frame.mutations.iter().find_map(|mutation| {
        let view = match &mutation.mutation.mutation {
            TypedSurfaceMutation::ChannelSummarySnapshot { view } => view,
            TypedSurfaceMutation::ChannelSummaryPatch { patch } => &patch.view,
            _ => return None,
        };
        Some(json!({
            "channelId": view.channel_id,
            "lifecycle": view.lifecycle,
            "actions": view.actions.iter().map(|action| json!({
                "kind": action.kind,
                "enabled": action.enabled,
                "disabledReason": action.disabled_reason,
            })).collect::<Vec<_>>()
        }))
    });
    serde_json::to_vec(&json!({
        "schemaVersion":frame.schema_version,
        "surfaces":surface_names(frame),
        "lifecycle":lifecycle
    }))
    .expect("G16 semantic projection serializes")
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

/// 返回 Helix canonical G16 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g16-wdio")
}

/// 为一份 Fixture 生成 producer、hash、surface 与允许动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId":case.fixture_id,
        "fileName":case.file_name,
        "gateId":"G-16",
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
            "/mutations/*/mutation/view/title/text",
            "/mutations/*/mutation/view/avatar/fallbackText",
            "/mutations/*/mutation/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/title/text",
            "/mutations/*/mutation/patch/view/avatar/fallbackText",
            "/mutations/*/mutation/patch/view/actions/*/surfaceEpoch"
        ],
        "rebaseFields":[
            "/transactionId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/actions/*/surfaceEpoch"
        ],
        "placeholderValues":{
            "actorId":ACTOR_ID,
            "teamId":TEAM_ID,
            "channelId":CHANNEL_ID,
            "groupName":GROUP_NAME,
            "transactionId":case.frame.transaction_id,
            "surfaceEpoch":case.frame.mutations[0].surface_epoch,
            "revision":case.frame.mutations[0].mutation.revision,
            "baseRevision":case.frame.mutations[0].mutation.base_revision
        }
    })
}

#[test]
/// 证明真实 producer、完整字段、Golden bytes 与 provenance 始终同源。
fn current_g16_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 5, "G16 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G16_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("G16 provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G16_WDIO_FIXTURES=1 cargo test -p helix-im --test g16_wdio_fixture_contract_test",
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
