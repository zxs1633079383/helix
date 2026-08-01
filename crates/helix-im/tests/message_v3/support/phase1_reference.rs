//! visual MessageV3 Phase 1 目标参考 JSON 的只读测试支持。

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

const GENERATED_FILES: [&str; 6] = [
    "inbound-contract.json",
    "inbound-sample.json",
    "effects-contract.json",
    "outbound-contract.json",
    "outbound-sample.json",
    "provenance.json",
];

/// 返回 Helix Phase 1 registry 的稳定路径。
pub fn registry_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../specs/047-im-event-gate-inventory/contracts")
        .join("message-v3-phase1-send.registry.json")
}

/// 从 registry 读取 visual 参考根，允许 CI 用环境变量覆盖机器绝对路径。
pub fn reference_root() -> PathBuf {
    if let Some(root) = std::env::var_os("MESSAGE_V3_PHASE1_REFERENCE_ROOT") {
        return PathBuf::from(root);
    }
    let registry = read_json(&registry_path());
    PathBuf::from(
        registry["reference_root"]
            .as_str()
            .expect("registry reference_root must be a string"),
    )
}

/// 读取 JSON 并在解析失败时附带文件路径。
pub fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    });
    serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!("failed to parse {}: {error}", path.display());
    })
}

/// 校验单 Gate 七文件包、证据边界与 Gate identity。
pub fn assert_reference_gate(expected_gate_id: &str, relative_path: &str) {
    let gate_root = reference_root().join(relative_path);
    let contract_path = gate_root.join("contract.json");
    let contract = read_json(&contract_path);
    let contract_gate_id = contract["gate_id"]
        .as_str()
        .or_else(|| contract["gateId"].as_str())
        .expect("contract must declare gate_id or gateId");
    assert_eq!(contract_gate_id, expected_gate_id);

    let mut discovered = BTreeSet::new();
    for file_name in GENERATED_FILES {
        let file_path = gate_root.join(file_name);
        let artifact = read_json(&file_path);
        assert_eq!(
            artifact["gate_id"].as_str(),
            Some(expected_gate_id),
            "{} has a mismatched gate_id",
            file_path.display()
        );
        discovered.insert(file_name);
    }
    assert_eq!(discovered.len(), GENERATED_FILES.len());

    for file_name in ["inbound-sample.json", "outbound-sample.json"] {
        let file_path = gate_root.join(file_name);
        let sample = read_json(&file_path);
        assert_eq!(
            sample["evidence_level"].as_str(),
            Some("TARGET_REFERENCE_SAMPLE")
        );
        assert_eq!(sample["runtime_capture"].as_bool(), Some(false));
    }
}
