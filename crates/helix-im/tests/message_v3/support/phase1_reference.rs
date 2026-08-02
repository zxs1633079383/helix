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

/// 返回同级 visual worktree 的根，允许 Phase 3 CI 使用显式根目录。
pub fn phase3_client_root() -> PathBuf {
    if let Some(root) = std::env::var_os("MESSAGE_V3_PHASE3_CLIENT_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../cses-client")
}

/// 返回唯一 Phase 3 manifest，禁止在 Rust 测试维护第二份 Gate 分母。
pub fn phase3_manifest_path() -> PathBuf {
    if let Some(path) = std::env::var_os("MESSAGE_V3_PHASE3_MANIFEST") {
        return PathBuf::from(path);
    }
    phase3_client_root().join("scripts/message-v3/reference/phases/phase3.manifest.json")
}

/// 返回 Phase 3 的人工 Gate 根，允许 CI 仅替换参考根而不改变 manifest。
pub fn phase3_reference_root() -> PathBuf {
    if let Some(root) = std::env::var_os("MESSAGE_V3_PHASE3_REFERENCE_ROOT") {
        return PathBuf::from(root);
    }
    phase3_client_root().join("specs/003-messagev3-gate-inventory/gates")
}

/// 从唯一 manifest 读取、去重并验证 Phase 3 的完整 Gate 分母。
pub fn phase3_gate_ids() -> Vec<String> {
    let manifest = read_json(&phase3_manifest_path());
    let expected_count = manifest["expectedGateCount"]
        .as_u64()
        .expect("phase3 manifest expectedGateCount must be an unsigned integer")
        as usize;
    let gate_ids = manifest["gates"]
        .as_array()
        .expect("phase3 manifest gates must be an array")
        .iter()
        .map(|gate| {
            gate.as_str()
                .expect("phase3 manifest gate must be a string")
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(gate_ids.len(), expected_count);
    gate_ids.into_iter().collect()
}

/// 为指定 Gate 扫描唯一人工 contract，避免在测试中复制 domain/path 映射。
pub fn phase3_gate_root(expected_gate_id: &str) -> PathBuf {
    assert!(
        phase3_gate_ids()
            .iter()
            .any(|gate_id| gate_id == expected_gate_id),
        "{expected_gate_id} is not declared by the Phase 3 manifest"
    );

    let reference_root = phase3_reference_root();
    let mut matches = Vec::new();
    for domain in fs::read_dir(&reference_root).unwrap_or_else(|error| {
        panic!(
            "failed to read Phase 3 reference root {}: {error}",
            reference_root.display()
        );
    }) {
        let domain = domain.unwrap_or_else(|error| panic!("failed to read domain entry: {error}"));
        if !domain
            .file_type()
            .unwrap_or_else(|error| panic!("failed to inspect domain entry: {error}"))
            .is_dir()
        {
            continue;
        }
        for gate in fs::read_dir(domain.path()).unwrap_or_else(|error| {
            panic!(
                "failed to read Phase 3 domain {}: {error}",
                domain.path().display()
            );
        }) {
            let gate = gate.unwrap_or_else(|error| panic!("failed to read gate entry: {error}"));
            let contract_path = gate.path().join("contract.json");
            if !contract_path.is_file() {
                continue;
            }
            let contract = read_json(&contract_path);
            let gate_id = contract["gate_id"]
                .as_str()
                .or_else(|| contract["gateId"].as_str());
            if gate_id == Some(expected_gate_id) {
                matches.push(gate.path());
            }
        }
    }
    assert_eq!(
        matches.len(),
        1,
        "{expected_gate_id} must resolve to exactly one Phase 3 contract"
    );
    matches.pop().expect("one Phase 3 contract path must exist")
}

/// 验证一个 Phase 3 Gate 的人工合同与六份派生参考均完整且未冒充运行态。
pub fn assert_phase3_reference_gate(expected_gate_id: &str) {
    let gate_root = phase3_gate_root(expected_gate_id);
    let contract = read_json(&gate_root.join("contract.json"));
    let contract_gate_id = contract["gate_id"]
        .as_str()
        .or_else(|| contract["gateId"].as_str())
        .expect("Phase 3 contract must declare gate_id or gateId");
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
        let sample = read_json(&gate_root.join(file_name));
        assert_eq!(
            sample["evidence_level"].as_str(),
            Some("TARGET_REFERENCE_SAMPLE")
        );
        assert_eq!(sample["runtime_capture"].as_bool(), Some(false));
    }
}

/// 将 manifest Gate ID 派生为唯一的 Cargo integration-test target 名称。
pub fn phase3_test_target_name(gate_id: &str) -> String {
    format!(
        "message_v3_{}",
        gate_id.to_ascii_lowercase().replace('-', "_")
    )
}

/// 验证 Cargo 只为 manifest 声明的每个 Gate 注册一次明确 test target。
pub fn assert_phase3_cargo_targets() {
    let cargo_toml_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let cargo_toml = fs::read_to_string(&cargo_toml_path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", cargo_toml_path.display());
    });
    for gate_id in phase3_gate_ids() {
        let target_name = phase3_test_target_name(&gate_id);
        let declaration = format!("name = \"{target_name}\"");
        assert_eq!(
            cargo_toml.matches(&declaration).count(),
            1,
            "{gate_id} must have exactly one explicit Cargo target {target_name}"
        );
    }
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
