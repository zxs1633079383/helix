// 回归锚: Spec 047/T011，30 Gate manifest 与显式 Rust test target 注册表。

use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const ROOT: &str = env!("CARGO_MANIFEST_DIR");

/// 读取并解析冻结 migration manifest。
fn migration_manifest() -> Value {
    let path =
        format!("{ROOT}/../../specs/047-im-event-gate-inventory/contracts/gate-migration.v1.json");
    serde_json::from_str(&std::fs::read_to_string(path).expect("migration manifest exists"))
        .expect("migration manifest is JSON")
}

/// 从 Cargo.toml 提取显式 MessageV3 Gate target 名与路径。
fn cargo_gate_targets() -> BTreeMap<String, String> {
    let cargo =
        std::fs::read_to_string(format!("{ROOT}/Cargo.toml")).expect("helix-im Cargo.toml exists");
    let mut targets = BTreeMap::new();
    let mut name = None;
    for line in cargo.lines().map(str::trim) {
        if let Some(value) = line
            .strip_prefix("name = \"message_v3_")
            .and_then(|value| value.strip_suffix('"'))
        {
            name = Some(format!("message_v3_{value}"));
        } else if let (Some(target), Some(path)) = (
            name.take(),
            line.strip_prefix("path = \"")
                .and_then(|value| value.strip_suffix('"')),
        ) {
            if ![
                "message_v3_event_contract",
                "message_v3_artifact_schema",
                "message_v3_gate_registry",
            ]
            .contains(&target.as_str())
            {
                targets.insert(target, path.to_string());
            }
        }
    }
    targets
}

/// manifest 中每个 Gate 必须有唯一显式 target，且排除项不得混入。
#[test]
fn manifest_and_explicit_gate_targets_cover_exactly_thirty_gates() {
    let manifest = migration_manifest();
    let entries = manifest["entries"]
        .as_array()
        .expect("manifest entries array");
    let ids = entries
        .iter()
        .map(|entry| entry["gate_id"].as_str().expect("gate_id"))
        .collect::<BTreeSet<_>>();
    assert_eq!(entries.len(), 30);
    assert_eq!(ids.len(), 30);

    let targets = cargo_gate_targets();
    let expected = entries
        .iter()
        .map(|entry| {
            (
                entry["test_target"].as_str().expect("test_target"),
                entry["test_path"].as_str().expect("test_path"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(targets.len(), 30, "all Gate targets must be explicit");
    for (target, path) in expected {
        assert_eq!(targets.get(target).map(String::as_str), Some(path));
        assert!(
            std::path::Path::new(ROOT).join(path).is_file(),
            "missing Gate test source: {path}"
        );
    }
    for excluded in [
        "message_v3_g03",
        "message_v3_g17",
        "message_v3_g18",
        "message_v3_g19",
        "message_v3_g20",
        "message_v3_g21",
        "message_v3_g22",
        "message_v3_g23",
        "message_v3_g24",
        "message_v3_g25",
        "message_v3_g26",
        "message_v3_g27",
    ] {
        assert!(
            !targets.contains_key(excluded),
            "excluded target leaked: {excluded}"
        );
    }
}

/// domain 到测试包映射必须同名，只有 channel-member 映射为 channel_member。
#[test]
fn domain_and_test_package_mapping_is_exact() {
    let manifest = migration_manifest();
    for entry in manifest["entries"].as_array().expect("manifest entries") {
        let domain = entry["domain"].as_str().expect("domain");
        let package = entry["test_package"].as_str().expect("test_package");
        let expected = if domain == "channel-member" {
            "channel_member"
        } else {
            domain
        };
        assert_eq!(package, expected, "wrong package for {}", entry["gate_id"]);
    }
}
