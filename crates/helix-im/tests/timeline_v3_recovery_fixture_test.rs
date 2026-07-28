//! Timeline V3 原子拒绝与 Snapshot 恢复负向 Fixture 合同。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::InteractionFrameV2;
use serde_json::{json, Value};

#[path = "support/timeline_formal_gate.rs"]
mod support;

use support::sha256_hex;

/// 返回 Timeline V3 recovery fixture 目录。
fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("timeline-v3-recovery")
}

/// 生成稳定 pretty JSON bytes，并固定末尾换行。
fn canonical_bytes(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value).expect("recovery fixture serializes");
    bytes.push(b'\n');
    bytes
}

/// 从合法 Snapshot 派生三个独立可证伪的无效事务。
fn invalid_cases() -> Vec<(&'static str, Value)> {
    let original: Value = serde_json::from_slice(include_bytes!(
        "fixtures/g10-wdio/timeline-ready.frame-v3.json"
    ))
    .expect("canonical snapshot parses");

    let mut identity = original.clone();
    identity["transactionId"] = json!("recovery-identity-mismatch");
    identity["mutations"][0]["mutation"]["view"]["entities"][0]["entityKey"]["temporaryId"] =
        json!("wrong-identity");

    let mut duplicate = original.clone();
    duplicate["transactionId"] = json!("recovery-duplicate-range-key");
    let first_key =
        duplicate["mutations"][0]["mutation"]["view"]["ranges"][0]["orderedKeys"][0].clone();
    duplicate["mutations"][0]["mutation"]["view"]["ranges"][0]["orderedKeys"][1] = first_key;

    let mut mixed = original.clone();
    mixed["transactionId"] = json!("recovery-mixed-invalid-transaction");
    let mut invalid_mutation = mixed["mutations"][0].clone();
    invalid_mutation["revision"] = json!(2);
    invalid_mutation["baseRevision"] = json!(1);
    invalid_mutation["mutation"] = json!({
        "kind": "timelineDelta",
        "operations": [{"kind": "replaceView", "view": {}}]
    });
    mixed["mutations"]
        .as_array_mut()
        .expect("mutations is an array")
        .push(invalid_mutation);

    vec![
        ("entity-key-mismatch.frame-v3.json", identity),
        ("duplicate-range-key.frame-v3.json", duplicate),
        ("mixed-invalid-transaction.frame-v3.json", mixed),
    ]
}

/// 校验任一无效 operation 都拒绝整个 Frame，并固定 raw hash provenance。
#[test]
fn invalid_timeline_transactions_are_rejected_atomically() {
    let directory = fixture_dir();
    let update = std::env::var_os("UPDATE_TIMELINE_V3_RECOVERY_FIXTURES").is_some();
    let mut fixtures = Vec::new();
    for (file_name, value) in invalid_cases() {
        let bytes = canonical_bytes(&value);
        assert!(
            InteractionFrameV2::decode(&bytes).is_err(),
            "{file_name} must remain invalid"
        );
        if update {
            fs::create_dir_all(&directory).expect("fixture directory is writable");
            fs::write(directory.join(file_name), &bytes).expect("fixture export succeeds");
        }
        assert_eq!(
            fs::read(directory.join(file_name)).expect("negative fixture exists"),
            bytes
        );
        fixtures.push(json!({
            "fileName": file_name,
            "fixtureSha256": sha256_hex(&bytes),
            "expected": "atomic-reject-and-request-recovery-snapshot"
        }));
    }
    let provenance = json!({
        "gateId": "Timeline-V3-Recovery",
        "testClass": "timeline_v3_recovery_fixture_test",
        "producer": "canonical V3 Snapshot -> deterministic invalid transaction derivation",
        "producerRevision": "312e331938054467f1de09d4a251fd47b12093b0",
        "fixtures": fixtures
    });
    let provenance_bytes = canonical_bytes(&provenance);
    if update {
        fs::write(directory.join("provenance.json"), &provenance_bytes)
            .expect("provenance export succeeds");
    }
    assert_eq!(
        fs::read(directory.join("provenance.json")).expect("provenance exists"),
        provenance_bytes
    );
}
