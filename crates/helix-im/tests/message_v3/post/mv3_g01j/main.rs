//! MV3-G01j｜AVERAGE_SCORE 创建合同 Gap。
//!
//! Phase 1 不提供评分活动创建链；本测试固定“零入口、零 Effect、零事件”的真实边界。

use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, Harness};

const GATE_ID: &str = "MV3-G01j";
const GATE_PATH: &str = "post/mv3-g01j";
const RUST_COMMAND: &str = "im_create_average_score";

/// 读取本 Gate 的目标参考文件。
fn reference_file(name: &str) -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root()
            .join(GATE_PATH)
            .join(name),
    )
}

/// 校验 deferred gap 的生产表面必须全部为空。
fn check_gap_surfaces(
    inbound: &Value,
    sample: &Value,
    effects: &Value,
    outbound: &Value,
    outbound_sample: &Value,
) -> Result<(), String> {
    if inbound["rust_entry"] != json!("NONE") || inbound["rust_commands"] != json!([]) {
        return Err("gap must expose no Rust command entry".to_string());
    }
    if sample["rust_entry_inputs"] != json!([]) {
        return Err("gap must contain no synthetic Rust input".to_string());
    }
    if effects["allowed_effect_tags"] != json!([]) || effects["reference_sequence"] != json!([]) {
        return Err("gap must declare zero effects".to_string());
    }
    if outbound["channels"] != json!([]) || outbound["command_results"] != json!([]) {
        return Err("gap must expose no outbound surface".to_string());
    }
    if outbound_sample["samples"] != json!([]) || outbound_sample["command_results"] != json!([]) {
        return Err("gap must contain no fabricated outbound sample".to_string());
    }
    Ok(())
}

/// 从磁盘读取六份参考文件并校验 Gap 一致性。
fn check_deferred_gap() -> Result<(), String> {
    for file in [
        "inbound-contract.json",
        "inbound-sample.json",
        "effects-contract.json",
        "outbound-contract.json",
        "outbound-sample.json",
        "provenance.json",
    ] {
        let value = reference_file(file);
        if value["gate_id"] != json!(GATE_ID) || value["status"] != json!("DEFERRED_GAP") {
            return Err(format!("{file} must preserve the registered gap"));
        }
    }

    let inbound = reference_file("inbound-contract.json");
    let sample = reference_file("inbound-sample.json");
    let effects = reference_file("effects-contract.json");
    let outbound = reference_file("outbound-contract.json");
    let outbound_sample = reference_file("outbound-sample.json");
    check_gap_surfaces(&inbound, &sample, &effects, &outbound, &outbound_sample)
}

/// 证明目标参考包完整且只属于参考证据层。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate(GATE_ID, GATE_PATH);
}

/// 证明 AVERAGE_SCORE 创建在 Phase 1 中是显式零入口 Gap。
#[test]
fn deferred_gap_has_zero_runtime_surface() {
    check_deferred_gap().expect("AVERAGE_SCORE gap contract must remain empty");
    assert_eq!(
        reference_file("inbound-contract.json")["angular_port_method"],
        "NONE"
    );
}

/// 在内存中伪造命令、Effect 或事件时，Gap 边界必须能够识别漂移。
#[test]
fn deferred_gap_contract_rejects_fabricated_surfaces() {
    let root = phase1_reference::reference_root().join(GATE_PATH);
    let sample = phase1_reference::read_json(&root.join("inbound-sample.json"));
    let pristine_effects = phase1_reference::read_json(&root.join("effects-contract.json"));
    let pristine_outbound = phase1_reference::read_json(&root.join("outbound-contract.json"));
    let outbound_sample = phase1_reference::read_json(&root.join("outbound-sample.json"));
    let mut inbound = phase1_reference::read_json(&root.join("inbound-contract.json"));
    inbound["rust_commands"] = json!([{"name": RUST_COMMAND}]);
    assert!(check_gap_surfaces(
        &inbound,
        &sample,
        &pristine_effects,
        &pristine_outbound,
        &outbound_sample
    )
    .is_err());

    let pristine_inbound = phase1_reference::read_json(&root.join("inbound-contract.json"));
    let mut effects = pristine_effects.clone();
    effects["reference_sequence"] = json!([{"sequence":1,"tag":"Http"}]);
    assert!(check_gap_surfaces(
        &pristine_inbound,
        &sample,
        &effects,
        &pristine_outbound,
        &outbound_sample
    )
    .is_err());

    let mut outbound = pristine_outbound.clone();
    outbound["channels"] = json!([{"rust_event":"im:post:received"}]);
    assert!(check_gap_surfaces(
        &pristine_inbound,
        &sample,
        &pristine_effects,
        &outbound,
        &outbound_sample
    )
    .is_err());
}

/// 证明未登记命令不会进入 Helix 公开 command 面。
#[test]
fn average_score_creation_command_is_not_publicly_registered() {
    let capabilities: Value = serde_json::from_slice(&helix_im::client_api::capabilities_json())
        .expect("capabilities JSON must parse");
    let commands = capabilities["commands"].as_array().expect("commands array");
    assert!(commands.iter().any(|name| name == "im_send_message"));
    assert!(!commands.iter().any(|name| name == RUST_COMMAND));
}

/// 直接注入未登记命令也必须保持零 Effect，不能静默形成隐藏实现。
#[test]
fn average_score_creation_command_produces_no_effect() {
    let intent = reference_file("inbound-sample.json")["angular_client_intent"].clone();
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let effects = harness.step(command_json(RUST_COMMAND, intent), 1_700_000_000_000);
    assert!(
        effects.is_empty(),
        "unregistered AVERAGE_SCORE command emitted {effects:?}"
    );
}
