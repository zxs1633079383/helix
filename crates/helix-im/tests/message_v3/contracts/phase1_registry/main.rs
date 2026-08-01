#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// 读取 Phase 1 registry entries 并保持失败信息集中。
fn entries() -> Vec<Value> {
    phase1_reference::read_json(&phase1_reference::registry_path())["entries"]
        .as_array()
        .expect("registry entries must be an array")
        .clone()
}

/// 证明 Phase 1 registry 精确覆盖 33 Gate 且不存在重复 identity。
#[test]
fn registry_has_exact_denominator_and_unique_gate_ids() {
    let entries = entries();
    assert_eq!(entries.len(), 33);
    let ids = entries
        .iter()
        .map(|entry| {
            entry["gate_id"]
                .as_str()
                .expect("entry gate_id must be a string")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 33);
}

/// 证明 30 个真实接入、2 个 deferred Gap 和 1 个 partial Gap 保持 fail-closed。
#[test]
fn registry_preserves_real_and_gap_denominators() {
    let mut counts = BTreeMap::new();
    for entry in entries() {
        let state = entry["state"]
            .as_str()
            .expect("entry state must be a string")
            .to_owned();
        *counts.entry(state).or_insert(0usize) += 1;
    }
    assert_eq!(counts.get("DEFERRED_GAP"), Some(&2));
    assert_eq!(counts.get("PARTIAL_DEFERRED_GAP"), Some(&1));
    assert_eq!(counts.get("TARGET_REQUIRED"), Some(&29));
    assert_eq!(counts.get("NOT_APPLICABLE"), Some(&1));
}

/// 证明每个 registry entry 都能只读加载 visual 七文件参考包。
#[test]
fn registry_reference_paths_are_complete() {
    for entry in entries() {
        phase1_reference::assert_reference_gate(
            entry["gate_id"]
                .as_str()
                .expect("entry gate_id must be a string"),
            entry["reference_path"]
                .as_str()
                .expect("entry reference_path must be a string"),
        );
    }
}

/// 证明 target 和测试路径都是独立且命名一致的机器入口。
#[test]
fn registry_test_targets_are_unique_and_well_formed() {
    let entries = entries();
    let targets = entries
        .iter()
        .map(|entry| {
            entry["test_target"]
                .as_str()
                .expect("test_target must be a string")
        })
        .collect::<BTreeSet<_>>();
    let paths = entries
        .iter()
        .map(|entry| {
            entry["test_path"]
                .as_str()
                .expect("test_path must be a string")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(targets.len(), 33);
    assert_eq!(paths.len(), 33);
    assert!(targets
        .iter()
        .all(|target| target.starts_with("message_v3_mv3_")));
}
