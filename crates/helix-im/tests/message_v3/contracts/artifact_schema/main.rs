// 回归锚: Spec 047/T010，四类 generated JSON 的 schema 合同。

#[path = "../../support/artifact.rs"]
mod artifact;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use serde_json::{json, Value};
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

/// 生成器提供根目录时校验真实 Rust 产物；迁移期可显式允许非空子集。
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

    let allow_partial = std::env::var("HELIX_GATE_ARTIFACT_ALLOW_PARTIAL").as_deref() == Ok("1");
    let mut gates = std::fs::read_dir(root)
        .expect("artifact root exists")
        .map(|entry| entry.expect("gate directory").path())
        .filter(|path| path.is_dir() && (!allow_partial || path.join("generated").is_dir()))
        .collect::<Vec<_>>();
    gates.sort();
    if allow_partial {
        assert!(
            !gates.is_empty(),
            "partial generated tree must not be empty"
        );
    } else {
        assert_eq!(gates.len(), 30, "generated tree must contain 30 gates");
    }
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

// ═══════════════════════ T148 · Phase 1 33 Gate 产物 schema ═══════════════════════
//
// 上面的 047 schema 只覆盖 helix 自产的四类 artifact（`gate_id` 形如 `G-01`）。
// MessageV3 Phase 1 的 33 个 Gate 另有一套由 visual 侧生成器产出的**六份**生成物
// （`gate_id` 形如 `MV3-G01a`），它们此前**没有任何机器可执行的 schema**——只有生成器
// 自己的口头约定。本节把这套形状固定成仓内 schema 并对全部 33 个 Gate 逐一执行。
//
// 这些 schema 写在测试里而不是 `specs/` 下，是因为本批次禁改 `specs/003-*`；它们校验的
// 对象在 visual 仓，schema 的宿主放在消费侧（Helix 测试）不影响权威性。
//
// 证伪：`phase1_schemas_reject_*` 系列在内存里把真实产物改坏，schema 必须拒绝；
// 若把某条 schema 放宽（例如去掉 `additionalProperties:false` 或去掉 enum），
// 对应的负向用例立刻变红。

/// Phase 1 Gate id 的机器形态（`MV3-G` + 两位序号 + 一位子序）。
const PHASE1_GATE_ID_PATTERN: &str = "^MV3-G[0-9]{2}[a-z]$";

/// 33 Gate 允许出现的生命周期状态。分母语义见 spec 004 §2.3。
const PHASE1_STATUSES: [&str; 4] = [
    "TARGET_REQUIRED",
    "DEFERRED_GAP",
    "PARTIAL_DEFERRED_GAP",
    "NOT_APPLICABLE",
];

/// Phase 1 只允许这六种 Effect tag（spec 004 §2.1 的 Effect 序列列）。
const PHASE1_EFFECT_TAGS: [&str; 6] = [
    "Http",
    "Persist",
    "PersistAtomic",
    "UploadFile",
    "ScheduleTimer",
    "Emit",
];

/// 六份生成物的文件名，与 `phase1_reference::GENERATED_FILES` 同集合。
const PHASE1_GENERATED_FILES: [&str; 6] = [
    "inbound-contract.json",
    "inbound-sample.json",
    "effects-contract.json",
    "outbound-contract.json",
    "outbound-sample.json",
    "provenance.json",
];

/// 公共头：每份生成物都必须自报 schema 版本、Gate identity 与状态。
fn phase1_common_properties(schema_version: &str) -> Value {
    json!({
        "schema_version": {"const": schema_version},
        "gate_id": {"type": "string", "pattern": PHASE1_GATE_ID_PATTERN},
        "status": {"enum": PHASE1_STATUSES},
        "contract_decision_ref": {
            "const": "specs/004-messagev3-helix-phase1-send-domain/Phase1合同决议.json"
        },
    })
}

/// 六份 Phase 1 生成物的 schema，按文件名索引。
fn phase1_schema(file: &str) -> Value {
    let mut properties = match file {
        "inbound-contract.json" => {
            let mut properties = phase1_common_properties("messagev3-rust-inbound-contract/v1");
            properties["gate_name"] = json!({"type": "string", "minLength": 1});
            properties["phase"] = json!({"const": 1});
            properties["direction_basis"] = json!({"type": "string", "minLength": 1});
            properties["angular_port_method"] = json!({"type": "string", "minLength": 1});
            properties["rust_entry"] = json!({
                "enum": ["Tick::Command", "Tick::Inbound", "Tick::Timer", "TauriHostCommand", "NONE"]
            });
            // 边界与载荷编码是一对绑定：走 Helix Tick::Command 的命令一律纯 JSON；
            // 只有 Tauri Host ingress（媒体暂存 `im_stage_media`）与 Raw ingress
            //（生成媒体暂存 `im_stage_generated_media`）允许非 JSON 载荷。三者交叉即为契约漂移
            //（例如把媒体流塞进纯 JSON Command 边界）。
            properties["rust_commands"] = json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "boundary", "payload_encoding", "forbid_unknown"],
                    "properties": {
                        "name": {"type": "string", "pattern": "^im_[a-z0-9_]+$"},
                        "boundary": {"enum": ["Helix Tick::Command", "Tauri Host ingress", "Tauri Host Raw ingress"]},
                        "payload_encoding": {"enum": ["JSON", "metadata JSON + chunk stream", "base64 metadata header + Raw byte body"]},
                        // fail closed 是硬约束：未知字段不得被静默接受。
                        "forbid_unknown": {"const": true}
                    },
                    "allOf": [
                        {
                            "if": {
                                "required": ["boundary"],
                                "properties": {"boundary": {"const": "Helix Tick::Command"}}
                            },
                            "then": {"properties": {"payload_encoding": {"const": "JSON"}}}
                        },
                        {
                            "if": {
                                "required": ["boundary"],
                                "properties": {"boundary": {"const": "Tauri Host ingress"}}
                            },
                            "then": {
                                "properties": {
                                    "payload_encoding": {"const": "metadata JSON + chunk stream"}
                                }
                            }
                        },
                        {
                            "if": {
                                "required": ["boundary"],
                                "properties": {"boundary": {"const": "Tauri Host Raw ingress"}}
                            },
                            "then": {
                                "properties": {
                                    "payload_encoding": {"const": "base64 metadata header + Raw byte body"}
                                }
                            }
                        }
                    ]
                }
            });
            properties["field_ownership"] = json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["angular", "helix"],
                "properties": {
                    "angular": {"type": "array", "items": {"type": "string"}},
                    "helix": {"type": "array", "items": {"type": "string"}},
                    // 转发参考生成器记录服务端权威职责；保留旧双职责材料兼容。
                    "go": {"type": "array", "items": {"type": "string"}}
                }
            });
            properties["shared_invariants"] = json!({"type": "array", "items": {"type": "string"}});
            properties["current_contract_ref"] = json!({"const": "contract.json"});
            properties
        }
        "inbound-sample.json" => {
            let mut properties = phase1_common_properties("messagev3-rust-inbound-sample/v1");
            // 参考样本永远不得自称运行时捕获（`promotion_rule` 的机器化表达）。
            properties["evidence_level"] = json!({"const": "TARGET_REFERENCE_SAMPLE"});
            properties["runtime_capture"] = json!({"const": false});
            properties["warning"] = json!({"type": "string", "minLength": 1});
            properties["angular_client_intent"] = json!({"type": ["object", "array", "null"]});
            properties["rust_entry_inputs"] = json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["tick_variant", "value"],
                    "properties": {
                        "tick_variant": {
                            "enum": ["Tick::Command", "Tick::Inbound", "Tick::Timer", "TauriHostCommand"]
                        },
                        "value": {}
                    }
                }
            });
            properties["host_stage_results"] = json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": [
                        "client_stream_id", "media_handle", "file_name",
                        "content_type", "size", "sha256", "metadata"
                    ],
                    "properties": {
                        "client_stream_id": {"type": "string", "minLength": 1},
                        "media_handle": {"type": "string", "minLength": 1},
                        "file_name": {"type": "string", "minLength": 1},
                        "content_type": {"type": "string", "minLength": 1},
                        // INV-11：单文件硬上限 1 GiB。
                        "size": {"type": "integer", "minimum": 0, "maximum": 1073741824},
                        "sha256": {"type": "string", "pattern": "^[0-9a-f]{64}$"},
                        "metadata": {"type": "object"}
                    }
                }
            });
            properties["host_stage_error"] = json!({"type": ["object", "null"]});
            properties["deterministic_ids"] = json!({"type": "object"});
            properties
        }
        "effects-contract.json" => {
            let mut properties = phase1_common_properties("messagev3-rust-effects-contract/v1");
            properties["phase"] = json!({"const": 1});
            properties["direction_basis"] = json!({"type": "string", "minLength": 1});
            properties["allowed_effect_tags"] = json!({
                "type": "array", "uniqueItems": true, "items": {"enum": PHASE1_EFFECT_TAGS}
            });
            properties["reference_sequence"] = json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["sequence", "tag", "purpose"],
                    "properties": {
                        "sequence": {"type": "integer", "minimum": 1},
                        "tag": {"enum": PHASE1_EFFECT_TAGS},
                        "purpose": {"type": "string", "minLength": 1}
                    }
                }
            });
            properties["source_state_surfaces"] =
                json!({"type": "array", "items": {"type": "string"}});
            // schedule 域（G04a-e）目前不声明 target_state_invariants；空数组是合法态，
            // 「零 Effect」的判定改由 `phase1_zero_effect_gates_are_exactly_the_declared_two`
            // 用精确 Gate 集合锁住，比一刀切的 minItems 更贴合参考包。
            properties["target_state_invariants"] =
                json!({"type": "array", "items": {"type": "string"}});
            properties["hard_rules"] = json!({"type": "array", "items": {"type": "string"}});
            properties
        }
        "outbound-contract.json" => {
            let mut properties = phase1_common_properties("messagev3-rust-outbound-contract/v1");
            properties["phase"] = json!({"const": 1});
            properties["direction_basis"] = json!({"type": "string", "minLength": 1});
            properties["channels"] = json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": [
                        "rust_event", "core_output", "envelope",
                        "angular_mapping", "angular_payload_rule"
                    ],
                    "properties": {
                        // 与 047 message-v3-event.schema.json 的事件名形态一致。
                        "rust_event": {"type": "string", "pattern": "^im:[a-z0-9_-]+:[a-z0-9_-]+$"},
                        "core_output": {"const": "Effect::Emit"},
                        "envelope": {"const": "{event,data}"},
                        "angular_mapping": {"type": "string", "minLength": 1},
                        "angular_payload_rule": {"type": "string", "minLength": 1},
                        "contract_event_kind": {"type": "string", "minLength": 1},
                        "required_payload_paths": {
                            "type": "array",
                            "items": {"type": "string", "minLength": 1}
                        }
                    }
                }
            });
            properties["command_results"] = json!({"type": "array", "items": {"type": "object"}});
            properties["delivery_rules"] = json!({"type": "array", "items": {"type": "string"}});
            // 参考包允许显式记录历史事件复用与合同扩展；它们不改变核心信道形状。
            properties["contract_extensions"] = json!({
                "type": "array",
                "items": {"type": "object"}
            });
            properties["reused_event_contracts"] = json!({
                "type": "array",
                "items": {"type": "object"}
            });
            properties
        }
        "outbound-sample.json" => {
            let mut properties = phase1_common_properties("messagev3-rust-outbound-sample/v1");
            properties["evidence_level"] = json!({"const": "TARGET_REFERENCE_SAMPLE"});
            properties["runtime_capture"] = json!({"const": false});
            properties["warning"] = json!({"type": "string", "minLength": 1});
            properties["command_results"] = json!({"type": "array", "items": {"type": "object"}});
            properties["samples"] = json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["sequence", "variant", "viewer", "rust", "angular", "sample_order"],
                    "properties": {
                        "sequence": {"type": "integer", "minimum": 1},
                        "variant": {"type": "string", "minLength": 1},
                        "viewer": {"enum": ["sender", "receiver"]},
                        "rust": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["event", "data"],
                            "properties": {
                                "event": {"type": "string", "pattern": "^im:[a-z0-9_-]+:[a-z0-9_-]+$"},
                                "data": {
                                    "oneOf": [
                                        {"type": "object"},
                                        {"type": "array", "items": {"type": "object"}}
                                    ]
                                }
                            }
                        },
                        "angular": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["kind", "payload"],
                            "properties": {
                                "kind": {"type": "string", "minLength": 1},
                                "payload": {
                                    "oneOf": [
                                        {"type": "object"},
                                        {"type": "array", "items": {"type": "object"}}
                                    ]
                                }
                            }
                        },
                        "sample_order": {"type": "integer", "minimum": 1}
                    }
                }
            });
            properties
        }
        "provenance.json" => {
            let mut properties = phase1_common_properties("messagev3-gate-reference-provenance/v1");
            properties["phase"] = json!({"const": 1});
            properties["generated_by"] = json!({"type": "string", "minLength": 1});
            properties["generated_from"] =
                json!({"type": "array", "minItems": 1, "items": {"type": "string"}});
            properties["evidence"] = json!({
                "type": "object",
                "additionalProperties": false,
                "required": [
                    "reference_sample", "scenario_runner_capture",
                    "tauri_runtime_capture", "real_http_ws_storage"
                ],
                "properties": {
                    "reference_sample": {"const": true},
                    // Phase 1 三类运行时证据一律 false：参考样本不得冒充实证（spec §0）。
                    "scenario_runner_capture": {"const": false},
                    "tauri_runtime_capture": {"const": false},
                    "real_http_ws_storage": {"const": false}
                }
            });
            properties["promotion_rule"] = json!({"type": "string", "minLength": 1});
            properties
        }
        other => panic!("unknown Phase 1 generated file `{other}`"),
    };

    let optional = if file == "outbound-contract.json" {
        ["contract_extensions", "reused_event_contracts"]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
    } else {
        std::collections::BTreeSet::new()
    };
    let required = properties
        .as_object_mut()
        .expect("properties map")
        .keys()
        .filter(|key| !optional.contains(key.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties
    });
    if file == "inbound-contract.json" {
        // Deferred/partial/N/A 参考包允许显式空命令；目标接入 Gate 仍必须声明
        // 至少一个命令、字段归属和共享不变量，避免把空合同误当成已完成实现。
        schema["allOf"] = json!([
            {
                "if": {"properties": {"status": {"const": "TARGET_REQUIRED"}}},
                "then": {
                    "properties": {
                        "rust_commands": {"minItems": 1},
                        "field_ownership": {
                            "properties": {
                                "angular": {"minItems": 1},
                                "helix": {"minItems": 1}
                            }
                        },
                        "shared_invariants": {"minItems": 1}
                    }
                }
            },
            {
                "if": {
                    "properties": {
                        "status": {
                            "enum": ["PARTIAL_DEFERRED_GAP", "NOT_APPLICABLE"]
                        }
                    }
                },
                "then": {
                    "properties": {
                        "field_ownership": {
                            "properties": {
                                "angular": {"minItems": 1},
                                "helix": {"minItems": 1}
                            }
                        },
                        "shared_invariants": {"minItems": 1}
                    }
                }
            }
        ]);
    } else if file == "inbound-sample.json" {
        schema["allOf"] = json!([{
            "if": {"properties": {"status": {"const": "TARGET_REQUIRED"}}},
            "then": {"properties": {"deterministic_ids": {"minProperties": 1}}}
        }]);
    } else if file == "effects-contract.json" {
        schema["allOf"] = json!([{
            "if": {
                "properties": {
                    "status": {
                        "enum": ["TARGET_REQUIRED", "PARTIAL_DEFERRED_GAP", "NOT_APPLICABLE"]
                    }
                }
            },
            "then": {"properties": {"hard_rules": {"minItems": 1}}}
        }]);
    } else if file == "outbound-contract.json" {
        schema["allOf"] = json!([{
            "if": {
                "properties": {
                    "status": {
                        "enum": ["TARGET_REQUIRED", "PARTIAL_DEFERRED_GAP", "NOT_APPLICABLE"]
                    }
                }
            },
            "then": {"properties": {"delivery_rules": {"minItems": 1}}}
        }]);
    }
    schema
}

/// 33 Gate 的 registry entries（`gate_id` + `reference_path`）。
fn phase1_entries() -> Vec<Value> {
    phase1_reference::read_json(&phase1_reference::registry_path())["entries"]
        .as_array()
        .expect("registry entries must be an array")
        .clone()
}

/// 读取某 Gate 的一份生成物。
fn phase1_artifact(reference_path: &str, file: &str) -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root()
            .join(reference_path)
            .join(file),
    )
}

/// 33 个 Gate × 六份生成物（含 provenance）全部通过 Phase 1 schema。
#[test]
fn phase1_generated_artifacts_match_their_schemas() {
    let entries = phase1_entries();
    assert_eq!(entries.len(), 33, "Phase 1 分母必须是 33 Gate");

    let schemas = PHASE1_GENERATED_FILES
        .iter()
        .map(|file| (*file, phase1_schema(file)))
        .collect::<Vec<_>>();

    let mut validated = 0usize;
    for entry in &entries {
        let gate_id = entry["gate_id"].as_str().expect("gate_id must be a string");
        let reference_path = entry["reference_path"]
            .as_str()
            .expect("reference_path must be a string");
        for (file, schema) in &schemas {
            let artifact = phase1_artifact(reference_path, file);
            assert_schema(schema, &artifact, &format!("{gate_id}/{file}"));
            // schema 无法表达跨文件 identity，这里补一条：六份文件的 gate_id 必须同一。
            assert_eq!(
                artifact["gate_id"].as_str(),
                Some(gate_id),
                "{gate_id}/{file}: gate identity 漂移"
            );
            validated += 1;
        }
    }
    assert_eq!(validated, 33 * 6, "必须逐一校验 198 份生成物");
}

/// registry 声明的 `state` 必须与六份生成物里的 `status` 完全一致，且分母仍是 30 + 3。
///
/// 这是「不得混算分母」的机器化表达：任何一处状态漂移都会让 30/3 失真。
#[test]
fn phase1_status_is_consistent_across_registry_and_artifacts() {
    let mut real_integration = 0usize;
    let mut gaps = 0usize;
    for entry in phase1_entries() {
        let gate_id = entry["gate_id"].as_str().expect("gate_id");
        let state = entry["state"].as_str().expect("state");
        let reference_path = entry["reference_path"].as_str().expect("reference_path");
        for file in PHASE1_GENERATED_FILES {
            assert_eq!(
                phase1_artifact(reference_path, file)["status"].as_str(),
                Some(state),
                "{gate_id}/{file}: status 与 registry.state 不一致"
            );
        }
        match state {
            "DEFERRED_GAP" | "PARTIAL_DEFERRED_GAP" => gaps += 1,
            _ => real_integration += 1,
        }
    }
    assert_eq!(real_integration, 30, "真实接入 Gate 必须是 30");
    assert_eq!(gaps, 3, "已登记 Gap 必须是 3，且不得并入 30");
}

/// 出站信道与出站样本必须同生共死，且零事件 Gate 的验收形态按状态二分。
///
/// 当前 Phase 1 参考包中有六个零事件 Gate：
/// - `MV3-G01i/G01j`：关闭合同决议留下的 deferred Gap，不产生结果；
/// - `MV3-G02d/G02e/G04d`：零事件但**有**结构化 Command Result（错误码 / 投影结果）；
/// - `MV3-G05l`：`NOT_APPLICABLE` 的**负向** Gate，零 Effect、零事件、零 Command Result——
///   它的验收就是「什么都不发生」，一旦这里冒出任何结果就说明工具栏命令写进了 IM。
#[test]
fn phase1_zero_event_gates_match_their_declared_shape() {
    let mut zero_event_gates = Vec::new();
    for entry in phase1_entries() {
        let gate_id = entry["gate_id"].as_str().expect("gate_id").to_string();
        let state = entry["state"].as_str().expect("state");
        let reference_path = entry["reference_path"].as_str().expect("reference_path");
        let channels = phase1_artifact(reference_path, "outbound-contract.json")["channels"]
            .as_array()
            .expect("channels must be an array")
            .len();
        let sample = phase1_artifact(reference_path, "outbound-sample.json");
        let samples = sample["samples"]
            .as_array()
            .expect("samples must be an array")
            .len();
        let command_results = sample["command_results"]
            .as_array()
            .expect("command_results must be an array")
            .len();

        assert_eq!(
            channels == 0,
            samples == 0,
            "{gate_id}: 出站信道数与事件样本数必须同为零或同为非零（channels={channels} samples={samples}）"
        );
        if channels == 0 {
            if matches!(
                state,
                "NOT_APPLICABLE" | "DEFERRED_GAP" | "PARTIAL_DEFERRED_GAP"
            ) {
                assert_eq!(
                    command_results, 0,
                    "{gate_id}: deferred/负向 Gate 必须零 Command Result（未接入或零 IM 写入）"
                );
            } else {
                assert!(
                    command_results > 0,
                    "{gate_id}: 零事件但非负向的 Gate 必须以 command_results 给出结构化结果"
                );
            }
            zero_event_gates.push(gate_id);
        } else {
            // 对照：有事件的 Gate 必须给出双视角，否则 receiver-local 目标态无从对齐。
            let viewers = sample["samples"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|item| item["viewer"].as_str())
                .collect::<std::collections::BTreeSet<_>>();
            assert!(
                viewers.contains("sender"),
                "{gate_id}: 出站样本必须至少覆盖 sender 视角"
            );
        }
    }
    zero_event_gates.sort();
    assert_eq!(
        zero_event_gates,
        vec!["MV3-G01i", "MV3-G01j", "MV3-G02d", "MV3-G02e", "MV3-G04d", "MV3-G05l",],
        "零事件 Gate 集合与 Phase 1 参考包声明不符"
    );
}

/// 当前 Phase 1 参考包声明「零 Effect」的 Gate 恰好是两个 deferred Gap 加两个负向/早退 Gate。
///
/// `allowed_effect_tags` 与 `reference_sequence` 同时为空 = 该 Gate 不允许产生任何 Effect：
/// - `MV3-G02d`（1 GiB+ 早退）：超限必须在暂存前返回结构化错误，**不创建 temporaryId、
///   不 Persist、不 Emit**（INV-11）；
/// - `MV3-G05l`（零 IM 写入工具栏命令）：整条命令不得触达 IM；
/// - `MV3-G01i/G01j`：已关闭的合同决议保持 deferred Gap，不伪造 Effect。
///
/// 这些 Gate 的正确实现方式就是「什么都不做」，所以必须用**精确集合**锁死；否则某天有人
/// 给别的 Gate 把 Effect 序列清空，schema 层面看不出任何异常。
#[test]
fn phase1_zero_effect_gates_are_exactly_the_declared_set() {
    let mut zero_effect = Vec::new();
    for entry in phase1_entries() {
        let gate_id = entry["gate_id"].as_str().expect("gate_id").to_string();
        let reference_path = entry["reference_path"].as_str().expect("reference_path");
        let effects = phase1_artifact(reference_path, "effects-contract.json");
        let tags = effects["allowed_effect_tags"]
            .as_array()
            .expect("allowed_effect_tags must be an array")
            .len();
        let sequence = effects["reference_sequence"]
            .as_array()
            .expect("reference_sequence must be an array")
            .len();
        assert_eq!(
            tags == 0,
            sequence == 0,
            "{gate_id}: Effect 白名单与参考序列必须同为空或同为非空（tags={tags} seq={sequence}）"
        );
        if tags == 0 {
            zero_effect.push(gate_id);
        } else {
            // 非零 Effect Gate：参考序列的 tag 必须恰好用满白名单（无冗余、无越界）。
            let used = effects["reference_sequence"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|step| step["tag"].as_str())
                .collect::<std::collections::BTreeSet<_>>();
            let allowed = effects["allowed_effect_tags"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|tag| tag.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            assert_eq!(
                used, allowed,
                "{gate_id}: reference_sequence 用到的 tag 必须与 allowed_effect_tags 完全相同"
            );
        }
    }
    zero_effect.sort();
    assert_eq!(
        zero_effect,
        vec!["MV3-G01i", "MV3-G01j", "MV3-G02d", "MV3-G05l"],
        "零 Effect Gate 集合与 Phase 1 参考包声明不符"
    );
}

/// 负向控制：把真实产物在内存里改坏，Phase 1 schema 必须拒绝。
///
/// 覆盖六份生成物各自最要害的一条约束；任何一条 schema 被放宽，这里立刻变红。
#[test]
fn phase1_schemas_reject_corrupted_artifacts() {
    let reference_path = "post/mv3-g01a";
    let reject = |file: &str, instance: &Value, label: &str| {
        let validator =
            jsonschema::validator_for(&phase1_schema(file)).expect("phase 1 schema compiles");
        assert!(
            !validator.is_valid(instance),
            "{label}: 腐化后的 {file} 仍被 schema 接受，说明该 schema 过松"
        );
    };

    // 先证明未腐化的原件是合法的，否则下面的 reject 全都可能是「本来就不合法」。
    for file in PHASE1_GENERATED_FILES {
        let validator =
            jsonschema::validator_for(&phase1_schema(file)).expect("phase 1 schema compiles");
        assert!(
            validator.is_valid(&phase1_artifact(reference_path, file)),
            "对照组失败：原始 {file} 必须先通过 schema"
        );
    }

    // ① inbound-contract：command 不再 fail closed。
    let mut open_command = phase1_artifact(reference_path, "inbound-contract.json");
    open_command["rust_commands"][0]["forbid_unknown"] = json!(false);
    reject(
        "inbound-contract.json",
        &open_command,
        "forbid_unknown=false",
    );

    // ② inbound-contract：入口漂移到未登记的 Tick 变体。
    let mut bogus_entry = phase1_artifact(reference_path, "inbound-contract.json");
    bogus_entry["rust_entry"] = json!("Tick::PortReply");
    reject("inbound-contract.json", &bogus_entry, "unknown rust_entry");

    // ②b inbound-contract：把媒体 chunk 流式编码挂到纯 JSON 的 Command 边界（边界×编码交叉）。
    let mut crossed = phase1_artifact(reference_path, "inbound-contract.json");
    crossed["rust_commands"][0]["payload_encoding"] = json!("metadata JSON + chunk stream");
    reject("inbound-contract.json", &crossed, "boundary/encoding cross");

    // ②c inbound-contract：把纯 JSON 编码挂到 Tauri Host ingress（反向交叉）。
    let mut media_gate = phase1_artifact("upload/mv3-g01c", "inbound-contract.json");
    let staging = media_gate["rust_commands"]
        .as_array()
        .expect("rust_commands array")
        .iter()
        .position(|command| command["boundary"] == "Tauri Host ingress")
        .expect("对照组失败：G01c 必须声明 Tauri Host ingress 的媒体暂存命令");
    media_gate["rust_commands"][staging]["payload_encoding"] = json!("JSON");
    reject(
        "inbound-contract.json",
        &media_gate,
        "host ingress downgraded to plain JSON",
    );

    // ③ inbound-sample：参考样本自称运行时捕获（冒充实证）。
    let mut forged = phase1_artifact(reference_path, "inbound-sample.json");
    forged["runtime_capture"] = json!(true);
    reject("inbound-sample.json", &forged, "runtime_capture=true");

    // ④ effects-contract：Effect tag 越出 Phase 1 白名单。
    let mut alien_tag = phase1_artifact(reference_path, "effects-contract.json");
    alien_tag["reference_sequence"][0]["tag"] = json!("Send");
    reject("effects-contract.json", &alien_tag, "alien effect tag");

    // ⑤ outbound-contract：信封被换成非合同形态。
    let mut wrong_envelope = phase1_artifact(reference_path, "outbound-contract.json");
    wrong_envelope["channels"][0]["envelope"] = json!("{channel,payload}");
    reject("outbound-contract.json", &wrong_envelope, "legacy envelope");

    // ⑥ outbound-sample：视角越界（只允许 sender / receiver）。
    let mut third_viewer = phase1_artifact(reference_path, "outbound-sample.json");
    third_viewer["samples"][0]["viewer"] = json!("both");
    reject("outbound-sample.json", &third_viewer, "unknown viewer");

    // ⑦ outbound-sample：混入未登记的顶层字段（生成器悄悄加字段）。
    let mut extra_field = phase1_artifact(reference_path, "outbound-sample.json");
    extra_field["captured_at"] = json!(1_700_000_000_000_i64);
    reject(
        "outbound-sample.json",
        &extra_field,
        "undeclared top-level field",
    );

    // ⑧ provenance：把运行时证据标成 true。
    let mut forged_provenance = phase1_artifact(reference_path, "provenance.json");
    forged_provenance["evidence"]["tauri_runtime_capture"] = json!(true);
    reject("provenance.json", &forged_provenance, "forged provenance");

    // ⑨ gate identity 形态漂移（helix 的 `G-01` 不得混进 Phase 1 命名空间）。
    let mut wrong_namespace = phase1_artifact(reference_path, "provenance.json");
    wrong_namespace["gate_id"] = json!("G-01");
    reject(
        "provenance.json",
        &wrong_namespace,
        "helix gate id namespace",
    );
}

/// 已提交的转发参考声明 Go 权威职责；它是显式可选字段，未知职责和错误类型仍须拒绝。
#[test]
fn phase1_inbound_field_ownership_accepts_go_and_rejects_unknown_roles() {
    let schema = phase1_schema("inbound-contract.json");
    let validator = jsonschema::validator_for(&schema).expect("phase 1 schema compiles");
    let mut inbound = phase1_artifact("post/mv3-g03a", "inbound-contract.json");
    assert!(
        validator.is_valid(&inbound),
        "current forward ownership must be valid"
    );
    inbound["field_ownership"]["go"] = json!("invalid scalar");
    assert!(
        !validator.is_valid(&inbound),
        "Go ownership must remain an array"
    );
    inbound["field_ownership"]["go"] = json!(["来源授权"]);
    inbound["field_ownership"]["unknown_runtime"] = json!([]);
    assert!(
        !validator.is_valid(&inbound),
        "unknown ownership must fail closed"
    );
    inbound["field_ownership"]
        .as_object_mut()
        .expect("ownership object")
        .remove("unknown_runtime");
    inbound["field_ownership"]
        .as_object_mut()
        .expect("ownership object")
        .remove("go");
    assert!(
        validator.is_valid(&inbound),
        "older two-role references remain valid"
    );
}
