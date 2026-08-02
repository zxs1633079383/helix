#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

const GATE_ID: &str = "MV3-G10e";

/// 锁定 G10e 的完整只读参考包，供后续 notify 偏好测试复用。
#[test]
fn phase3_gate_reference_is_complete() {
    phase1_reference::assert_phase3_reference_gate(GATE_ID);
}
