// 回归锚: Spec 047/T010，四类 generated JSON 的 schema 合同。

#[path = "../../support/artifact.rs"]
mod artifact;

use serde_json::Value;
use std::path::Path;

/// 编译并执行一个 JSON Schema，返回可审阅的全部失败原因。
fn assert_schema(schema: &Value, instance: &Value, label: &str) {
    let validator = jsonschema::validator_for(schema).expect("contract schema must compile");
    let errors = validator
        .iter_errors(instance)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "{label} schema errors: {errors:#?}");
}

/// 把 artifact schema 的相对 event 引用替换成同仓冻结 schema，避免测试访问网络。
fn resolved_artifact_schema() -> Value {
    let mut artifact_schema: Value = serde_json::from_str(include_str!(
        "../../../../../../specs/047-im-event-gate-inventory/contracts/gate-artifact.schema.json"
    ))
    .expect("artifact schema JSON");
    let event_schema: Value = serde_json::from_str(include_str!(
        "../../../../../../specs/047-im-event-gate-inventory/contracts/message-v3-event.schema.json"
    ))
    .expect("event schema JSON");
    artifact_schema["properties"]["events"]["items"] = event_schema;
    artifact_schema
}

/// sender/receiver、inbound 与 provenance 四类产物都必须通过仓内冻结 schema。
#[test]
fn four_generated_artifacts_match_contract_schemas() {
    let bundle = artifact::example_bundle();
    let artifact_schema = resolved_artifact_schema();
    let inbound_schema: Value = serde_json::from_str(include_str!(
        "../../../../../../specs/047-im-event-gate-inventory/contracts/gate-inbound.schema.json"
    ))
    .expect("inbound schema JSON");
    let provenance_schema: Value = serde_json::from_str(include_str!(
        "../../../../../../specs/047-im-event-gate-inventory/contracts/gate-provenance.schema.json"
    ))
    .expect("provenance schema JSON");

    assert_schema(&inbound_schema, &bundle.inbound, "inbound");
    assert_schema(&artifact_schema, &bundle.sender, "sender");
    assert_schema(&artifact_schema, &bundle.receiver, "receiver");
    assert_schema(&provenance_schema, &bundle.provenance, "provenance");
}

/// 破坏 event envelope 后 schema 必须拒绝，证明测试不是只走 happy path。
#[test]
fn artifact_schema_rejects_retired_wrapper_fields() {
    let mut sender = artifact::example_bundle().sender;
    sender["events"][0]["surface"] = serde_json::json!({"id": "retired"});
    let artifact_schema = resolved_artifact_schema();
    let validator = jsonschema::validator_for(&artifact_schema).expect("artifact schema compiles");
    assert!(!validator.is_valid(&sender));
}

/// 相同输入必须产生相同 bytes 与 hash，避免 HashMap 顺序污染 canonical 产物。
#[test]
fn repeated_generation_is_byte_and_hash_stable() {
    let bundle = artifact::example_bundle();
    let first = artifact::canonical_json(&bundle.sender);
    let second = artifact::canonical_json(&bundle.sender);
    assert_eq!(first, second);
    assert_eq!(artifact::sha256(&first), artifact::sha256(&second));
}

/// 任意层级的敏感字段都必须在写盘前 fail closed。
#[test]
fn sensitive_fields_are_rejected_before_artifact_write() {
    let mut sender = artifact::example_bundle().sender;
    sender["events"][0]["data"]["authorization"] = serde_json::json!("Bearer secret");
    let error = artifact::reject_sensitive_fields(&sender)
        .expect_err("sensitive artifact must be rejected");
    assert!(error.contains("authorization"));
}

/// 从磁盘读取 JSON，并把路径纳入解析失败信息。
fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

/// 生成器提供根目录时，逐 Gate 校验四类真实 Rust 产物而不是示例对象。
#[test]
fn generated_tree_matches_schemas_when_requested() {
    let Ok(root) = std::env::var("HELIX_GATE_ARTIFACT_ROOT") else {
        return;
    };
    let root = Path::new(&root);
    let artifact_schema = resolved_artifact_schema();
    let inbound_schema: Value = serde_json::from_str(include_str!(
        "../../../../../../specs/047-im-event-gate-inventory/contracts/gate-inbound.schema.json"
    ))
    .expect("inbound schema JSON");
    let provenance_schema: Value = serde_json::from_str(include_str!(
        "../../../../../../specs/047-im-event-gate-inventory/contracts/gate-provenance.schema.json"
    ))
    .expect("provenance schema JSON");

    let mut gates = std::fs::read_dir(root)
        .expect("artifact root exists")
        .map(|entry| entry.expect("gate directory").path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    gates.sort();
    assert_eq!(gates.len(), 30, "generated tree must contain 30 gates");
    for gate in gates {
        let generated = gate.join("generated");
        assert_schema(
            &inbound_schema,
            &read_json(&generated.join("inbound.json")),
            &format!("{} inbound", gate.display()),
        );
        assert_schema(
            &artifact_schema,
            &read_json(&generated.join("outbound.sender.json")),
            &format!("{} sender", gate.display()),
        );
        assert_schema(
            &artifact_schema,
            &read_json(&generated.join("outbound.receiver.json")),
            &format!("{} receiver", gate.display()),
        );
        assert_schema(
            &provenance_schema,
            &read_json(&generated.join("provenance.json")),
            &format!("{} provenance", gate.display()),
        );
    }
}
