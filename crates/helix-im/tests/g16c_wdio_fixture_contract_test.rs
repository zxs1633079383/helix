//! G16c WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较 actual 与 Golden；仅 `UPDATE_G16C_WDIO_FIXTURES=1` 允许覆盖。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "support/g16c_scenario.rs"]
mod scenario;

use scenario::{assert_settings_contract, G16cScenarioRunner, CHANNEL_ID};

const HELIX_COMMIT: &str = "32740014cf30ed92998ea89a504597fbaef6e7b5";
const GENERATED_AT: &str = "2026-07-26T18:40:00-07:00";
const PRODUCER: &str =
    "G16cScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture identity 与真实 producer Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成初态、三命令、observer 与 recovery Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let info = G16cScenarioRunner::run_info();
    let display = G16cScenarioRunner::run_display_name();
    let notice = G16cScenarioRunner::run_notice();
    let (observer, duplicate_frames) = G16cScenarioRunner::run_ws_update();
    let recovery = G16cScenarioRunner::run_recovery();
    assert_eq!(duplicate_frames, 0);
    assert_settings_contract(&info.initial, "G16c Group", "旧简介", "旧规则", 7, false);
    assert_settings_contract(
        &info.terminal,
        "G16c Renamed",
        "服务端简介",
        "服务端规则",
        8,
        true,
    );
    assert_settings_contract(
        &display.terminal,
        "G16c Display",
        "旧简介",
        "旧规则",
        9,
        true,
    );
    assert_settings_contract(
        &notice.terminal,
        "G16c Group",
        "旧简介",
        "G16c 公告",
        10,
        true,
    );
    assert_settings_contract(
        &observer.terminal,
        "G16c Observer",
        "观察者简介",
        "观察者规则",
        11,
        true,
    );
    assert_settings_contract(
        &recovery,
        "G16c Recovered",
        "恢复简介",
        "恢复规则",
        10,
        false,
    );
    vec![
        FixtureCase {
            fixture_id: "g16c-channel-summary-active-v2",
            file_name: "channel-summary-active.frame-v2.json",
            frame: info.initial,
        },
        FixtureCase {
            fixture_id: "g16c-channel-summary-info-v2",
            file_name: "channel-summary-info.frame-v2.json",
            frame: info.terminal,
        },
        FixtureCase {
            fixture_id: "g16c-channel-summary-display-name-v2",
            file_name: "channel-summary-display-name.frame-v2.json",
            frame: display.terminal,
        },
        FixtureCase {
            fixture_id: "g16c-channel-summary-notice-v2",
            file_name: "channel-summary-notice.frame-v2.json",
            frame: notice.terminal,
        },
        FixtureCase {
            fixture_id: "g16c-channel-summary-observer-v2",
            file_name: "channel-summary-observer.frame-v2.json",
            frame: observer.terminal,
        },
        FixtureCase {
            fixture_id: "g16c-channel-summary-recovery-v2",
            file_name: "channel-summary-recovery.frame-v2.json",
            frame: recovery,
        },
    ]
}

/// 生成稳定 pretty JSON bytes 并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("G16c Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取跨端必须一致的 settings 业务语义。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let summary = frame.mutations.iter().find_map(|mutation| {
        let view = match &mutation.mutation.mutation {
            TypedSurfaceMutation::ChannelSummarySnapshot { view } => view,
            TypedSurfaceMutation::ChannelSummaryPatch { patch } => &patch.view,
            _ => return None,
        };
        Some(json!({
            "channelId":view.channel_id,
            "displayName":view.title.text,
            "purpose":view.purpose,
            "header":view.notice_summary,
            "settingsVersion":view.settings_version,
            "actions":view.actions.iter().map(|action| json!({
                "kind":action.kind,
                "enabled":action.enabled,
                "disabledReason":action.disabled_reason,
            })).collect::<Vec<_>>()
        }))
    });
    serde_json::to_vec(&json!({
        "schemaVersion":frame.schema_version,
        "surfaces":surface_names(frame),
        "summary":summary
    }))
    .expect("G16c semantic projection serializes")
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

/// 返回 Helix canonical G16c Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g16c-wdio")
}

/// 为一份 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId":case.fixture_id,
        "fileName":case.file_name,
        "gateId":"G-16c",
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
            "/mutations/*/mutation/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/actions/*/actionRef",
            "/mutations/*/mutation/patch/view/channelId",
            "/mutations/*/mutation/patch/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/actions/*/actionRef"
        ],
        "rebaseFields":[
            "/transactionId",
            "/causationId",
            "/mutations/*/causationId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision",
            "/mutations/*/mutation/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/view/actions/*/actionRef",
            "/mutations/*/mutation/patch/view/actions/*/surfaceEpoch",
            "/mutations/*/mutation/patch/view/actions/*/actionRef"
        ],
        "placeholderValues":{
            "channelId":CHANNEL_ID,
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
fn current_g16c_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 6, "G16c canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_G16C_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("G16c provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_G16C_WDIO_FIXTURES=1 cargo test -p helix-im --test g16c_wdio_fixture_contract_test",
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
