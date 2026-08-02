#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use std::collections::BTreeSet;

/// 证明唯一 Phase 3 manifest 给出完整且无重复的 Gate 分母。
#[test]
fn phase3_manifest_has_exact_denominator_and_unique_gate_ids() {
    let gate_ids = phase1_reference::phase3_gate_ids();
    assert_eq!(gate_ids.len(), 15);
    assert_eq!(
        gate_ids.len(),
        gate_ids.iter().collect::<BTreeSet<_>>().len()
    );
}

/// 证明 manifest 每个 Gate 都映射到一个完整的只读七文件参考包。
#[test]
fn phase3_manifest_routes_every_gate_to_one_complete_reference_package() {
    for gate_id in phase1_reference::phase3_gate_ids() {
        phase1_reference::assert_phase3_reference_gate(&gate_id);
    }
}

/// 证明 Cargo 测试 target 由 manifest Gate ID 单向派生且逐一登记。
#[test]
fn phase3_manifest_has_one_registered_cargo_target_per_gate() {
    phase1_reference::assert_phase3_cargo_targets();
}
