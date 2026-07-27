//! GS1 WDIO/Mobile Frame V2 canonical Fixture 真源。
//!
//! 普通测试只比较 actual 与 Golden；仅 `UPDATE_GS1_WDIO_FIXTURES=1` 允许覆盖。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameV2, TypedSurfaceMutation};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

#[path = "support/gs1_scenario.rs"]
mod scenario;

use scenario::{assert_terminal_contract, Gs1ScenarioRunner};

const HELIX_COMMIT: &str = "98c522a3621cc599c134cb78b9e4656668eb67bb";
const GENERATED_AT: &str = "2026-07-27T00:00:00-07:00";
const PRODUCER: &str =
    "Gs1ScenarioRunner -> ImModule::handle -> Effect::Emit -> InteractionFrameV2::decode";

/// 绑定稳定 fixture identity 与真实 producer Frame。
struct FixtureCase {
    fixture_id: &'static str,
    file_name: &'static str,
    frame: InteractionFrameV2,
}

/// 从唯一 ScenarioRunner 生成 attach、durable terminal 与 cold recovery Frame。
fn produce_cases() -> Vec<FixtureCase> {
    let success = Gs1ScenarioRunner::run_success();
    let recovery = Gs1ScenarioRunner::run_recovery();
    assert_terminal_contract(&success.terminal);
    assert_terminal_contract(&recovery);
    vec![
        FixtureCase {
            fixture_id: "gs1-dialog-empty-v2",
            file_name: "dialog-empty.frame-v2.json",
            frame: success.initial,
        },
        FixtureCase {
            fixture_id: "gs1-dialog-loaded-v2",
            file_name: "dialog-loaded.frame-v2.json",
            frame: success.terminal,
        },
        FixtureCase {
            fixture_id: "gs1-dialog-recovery-v2",
            file_name: "dialog-recovery.frame-v2.json",
            frame: recovery,
        },
    ]
}

/// 生成稳定 pretty JSON bytes 并固定末尾换行。
fn canonical_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(frame).expect("GS1 Frame serializes");
    bytes.push(b'\n');
    bytes
}

/// 提取跨端必须一致的 dialog 业务语义。
fn semantic_bytes(frame: &InteractionFrameV2) -> Vec<u8> {
    let mut dialog = None;
    let mut unread = None;
    for mutation in &frame.mutations {
        match &mutation.mutation.mutation {
            TypedSurfaceMutation::DialogListSnapshot { view } => dialog = Some(json!(view)),
            TypedSurfaceMutation::DialogListPatch { patch } => dialog = Some(json!(patch.view)),
            TypedSurfaceMutation::UnreadSummarySnapshot { view } => unread = Some(json!(view)),
            TypedSurfaceMutation::UnreadSummaryPatch { patch } => unread = Some(json!(patch.view)),
            _ => {}
        }
    }
    serde_json::to_vec(&json!({
        "schemaVersion":frame.schema_version,
        "runtime":&frame.runtime,
        "surfaces":surface_names(frame),
        "dialogUnread":{"dialog":dialog,"unread":unread}
    }))
    .expect("GS1 semantic projection serializes")
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

/// 返回 Helix canonical GS1 Fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("gs1-wdio")
}

/// 为一份 Fixture 生成 producer、hash、surface 与动态字段白名单。
fn provenance_entry(case: &FixtureCase, bytes: &[u8]) -> Value {
    json!({
        "fixtureId":case.fixture_id,
        "fileName":case.file_name,
        "gateId":"G-S1",
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
            "/mutations/*/baseRevision"
        ],
        "rebaseFields":[
            "/transactionId",
            "/causationId",
            "/mutations/*/causationId",
            "/mutations/*/surfaceEpoch",
            "/mutations/*/revision",
            "/mutations/*/baseRevision"
        ],
        "placeholderValues":{
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
fn current_gs1_producer_matches_canonical_v2_fixtures_and_provenance() {
    let cases = produce_cases();
    assert_eq!(cases.len(), 3, "GS1 canonical inventory is explicit");
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_GS1_WDIO_FIXTURES").is_some();
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
        serde_json::to_vec_pretty(&provenance).expect("GS1 provenance serializes");
    provenance_bytes.push(b'\n');
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    for case in &cases {
        let path = directory.join(case.file_name);
        let checked_in = fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "{} missing ({error}); run UPDATE_GS1_WDIO_FIXTURES=1 cargo test -p helix-im --test gs1_wdio_fixture_contract_test",
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
