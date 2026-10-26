//! MessageV3 Gate 产物序列化、hash 与敏感字段护栏。

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 单 Gate 的四类 canonical 生成物。
pub struct ArtifactBundle {
    pub inbound: Value,
    pub sender: Value,
    pub receiver: Value,
    pub provenance: Value,
}

/// 从生产 scenario 捕获值组装四类 Gate 产物和非循环 provenance hash。
#[allow(clippy::too_many_arguments)]
pub fn gate_bundle(
    gate_id: &str,
    producer_test: &str,
    ticks: Vec<Value>,
    sender_events: Vec<Value>,
    receiver_events: Vec<Value>,
    remote_basis: Value,
    sender_storage: Value,
    receiver_storage: Value,
) -> ArtifactBundle {
    let inbound = json!({
        "schema_version": "helix-im-gate-inbound/v1",
        "gate_id": gate_id,
        "ticks": ticks,
        "remote_basis": remote_basis,
    });
    let sender = json!({
        "schema_version": "helix-im-gate-artifact/v1",
        "gate_id": gate_id,
        "perspective": "sender",
        "events": sender_events,
        "storage_basis": sender_storage,
    });
    let receiver = json!({
        "schema_version": "helix-im-gate-artifact/v1",
        "gate_id": gate_id,
        "perspective": "receiver",
        "events": receiver_events,
        "storage_basis": receiver_storage,
    });
    let inbound_bytes = canonical_json(&inbound);
    let sender_bytes = canonical_json(&sender);
    let receiver_bytes = canonical_json(&receiver);
    let storage_schema = include_bytes!(
        "../../../../../specs/047-im-event-gate-inventory/contracts/storage-schema-lock.v1.json"
    );
    let helix_head = std::env::var("HELIX_HEAD")
        .ok()
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .unwrap_or_else(|| "0000000000000000000000000000000000000000".to_string());
    let provenance = json!({
        "schema_version": "helix-im-gate-provenance/v1",
        "gate_id": gate_id,
        "producer_test": producer_test,
        "producer_entry": "ImModule::handle",
        "helix_head": helix_head,
        "matrix_epoch": 12,
        "storage_schema_sha256": sha256(storage_schema),
        "artifacts": {
            "inbound.json": sha256(&inbound_bytes),
            "outbound.sender.json": sha256(&sender_bytes),
            "outbound.receiver.json": sha256(&receiver_bytes),
        },
        "consumer_sync": "NOT_SYNCED_BY_SCOPE",
    });
    ArtifactBundle {
        inbound,
        sender,
        receiver,
        provenance,
    }
}

/// 构造只用于 schema 自证的最小产物，不作为任何 Gate 业务 fixture。
pub fn example_bundle() -> ArtifactBundle {
    let event = json!({
        "event": "im:post:received",
        "data": {
            "id": "post-schema-contract",
            "channelId": "channel-schema-contract"
        }
    });
    let storage = json!([{
        "table": "message",
        "columns": ["id", "channel_id"]
    }]);
    ArtifactBundle {
        inbound: json!({
            "schema_version": "helix-im-gate-inbound/v1",
            "gate_id": "G-01",
            "ticks": [{
                "kind": "Inbound",
                "value": {"action": "post"},
                "now_ms": 1
            }],
            "remote_basis": [{
                "table": "post",
                "columns": ["id", "channel_id"]
            }]
        }),
        sender: json!({
            "schema_version": "helix-im-gate-artifact/v1",
            "gate_id": "G-01",
            "perspective": "sender",
            "events": [event.clone()],
            "storage_basis": storage.clone()
        }),
        receiver: json!({
            "schema_version": "helix-im-gate-artifact/v1",
            "gate_id": "G-01",
            "perspective": "receiver",
            "events": [event],
            "storage_basis": storage
        }),
        provenance: json!({
            "schema_version": "helix-im-gate-provenance/v1",
            "gate_id": "G-01",
            "producer_test": "message_v3_artifact_schema",
            "producer_entry": "ImModule::handle",
            "helix_head": "0000000000000000000000000000000000000000",
            "matrix_epoch": 12,
            "storage_schema_sha256":
                "0000000000000000000000000000000000000000000000000000000000000000",
            "artifacts": {
                "inbound.json":
                    "0000000000000000000000000000000000000000000000000000000000000000",
                "outbound.sender.json":
                    "0000000000000000000000000000000000000000000000000000000000000000",
                "outbound.receiver.json":
                    "0000000000000000000000000000000000000000000000000000000000000000"
            },
            "consumer_sync": "NOT_SYNCED_BY_SCOPE"
        }),
    }
}

/// 以稳定 key 顺序和单一换行编码 JSON，保证重复生成 byte-exact。
pub fn canonical_json(value: &Value) -> Vec<u8> {
    let normalized = normalize(value);
    let mut bytes = serde_json::to_vec_pretty(&normalized).expect("JSON Value must serialize");
    bytes.push(b'\n');
    bytes
}

/// 递归把对象转换为按 key 排序的 Value，数组保持生产顺序。
fn normalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let ordered = object
                .iter()
                .map(|(key, value)| (key.clone(), normalize(value)))
                .collect::<BTreeMap<_, _>>();
            serde_json::to_value(ordered).expect("ordered JSON object serializes")
        }
        Value::Array(values) => Value::Array(values.iter().map(normalize).collect()),
        other => other.clone(),
    }
}

/// 计算 canonical artifact bytes 的小写 SHA-256。
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// 拒绝 token、password、cookie、authorization 等敏感字段进入生成物。
pub fn reject_sensitive_fields(value: &Value) -> Result<(), String> {
    const SENSITIVE: [&str; 7] = [
        "authorization",
        "cookie",
        "password",
        "secret",
        "token",
        "access_token",
        "refresh_token",
    ];

    /// 深度优先检查对象 key；值不做启发式扫描以避免误伤业务文本。
    fn visit(value: &Value, path: &str, sensitive: &[&str]) -> Result<(), String> {
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let normalized = key.to_ascii_lowercase();
                    if sensitive.contains(&normalized.as_str()) {
                        return Err(format!("sensitive field at {path}.{key}"));
                    }
                    visit(child, &format!("{path}.{key}"), sensitive)?;
                }
            }
            Value::Array(values) => {
                for (index, child) in values.iter().enumerate() {
                    visit(child, &format!("{path}[{index}]"), sensitive)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    visit(value, "$", &SENSITIVE)
}

/// 把四份已由 Rust scenario 构造的产物写入调用方指定目录。
pub fn write_bundle(output: &Path, bundle: &ArtifactBundle) -> Result<Vec<PathBuf>, String> {
    for value in [
        &bundle.inbound,
        &bundle.sender,
        &bundle.receiver,
        &bundle.provenance,
    ] {
        reject_sensitive_fields(value)?;
    }
    fs::create_dir_all(output).map_err(|error| error.to_string())?;
    let files = [
        ("inbound.json", &bundle.inbound),
        ("outbound.sender.json", &bundle.sender),
        ("outbound.receiver.json", &bundle.receiver),
        ("provenance.json", &bundle.provenance),
    ];
    files
        .iter()
        .map(|(name, value)| {
            let path = output.join(name);
            fs::write(&path, canonical_json(value)).map_err(|error| error.to_string())?;
            Ok(path)
        })
        .collect()
}
