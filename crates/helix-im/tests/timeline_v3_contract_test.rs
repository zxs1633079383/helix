//! Timeline V3 schema、closed operation 与稳定 EntityKey 的 fail-closed 合同。

use std::fs;
use std::path::{Path, PathBuf};

use helix_im::interaction_frame::{InteractionFrameError, InteractionFrameV2};
use serde_json::Value;

/// 定位 G10 正向 Snapshot，兼容迁移事务内 v2/v3 文件名切换。
fn canonical_snapshot_path() -> PathBuf {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("g10-wdio");
    [
        "timeline-ready.frame-v3.json",
        "timeline-ready.frame-v2.json",
    ]
    .into_iter()
    .map(|name| directory.join(name))
    .find(|path| path.exists())
    .expect("G10 canonical timeline snapshot exists")
}

/// 读取并严格解码 producer-owned Timeline V3 Snapshot。
fn canonical_snapshot() -> (Value, InteractionFrameV2) {
    let bytes = fs::read(canonical_snapshot_path()).expect("canonical snapshot is readable");
    let value = serde_json::from_slice(&bytes).expect("canonical snapshot is JSON");
    let frame = InteractionFrameV2::decode(&bytes).expect("canonical snapshot is valid");
    (value, frame)
}

/// 证明 Timeline 正向资产必须同时使用 Frame schema 3 与 surface schema 3。
#[test]
fn timeline_requires_frame_and_surface_schema_three() {
    let (value, frame) = canonical_snapshot();
    assert_eq!(frame.schema_version, 3);
    assert_eq!(value["mutations"][0]["surfaceSchemaVersion"], 3);

    let mut wrong_surface = value;
    wrong_surface["mutations"][0]["surfaceSchemaVersion"] = Value::from(2);
    assert!(matches!(
        InteractionFrameV2::decode(
            serde_json::to_vec(&wrong_surface)
                .expect("invalid sample serializes")
                .as_slice()
        ),
        Err(InteractionFrameError::SurfaceSchemaVersion { .. })
    ));
}

/// 证明 EntityKey 必须逐字段等于消息的不可变 createAt + temporaryId。
#[test]
fn entity_key_must_match_the_message_identity() {
    let (mut value, _) = canonical_snapshot();
    value["mutations"][0]["mutation"]["view"]["entities"][0]["entityKey"]["temporaryId"] =
        Value::from("rewritten-identity");
    assert!(matches!(
        InteractionFrameV2::decode(
            serde_json::to_vec(&value)
                .expect("invalid identity serializes")
                .as_slice()
        ),
        Err(InteractionFrameError::InvalidView { .. })
    ));
}

/// 证明 Snapshot Range 中重复或乱序的 key 会整帧失败。
#[test]
fn snapshot_range_keys_are_unique_and_strictly_ordered() {
    let (mut value, _) = canonical_snapshot();
    let keys = value["mutations"][0]["mutation"]["view"]["ranges"][0]["orderedKeys"]
        .as_array_mut()
        .expect("snapshot has ordered keys");
    if keys.len() == 1 {
        keys.push(keys[0].clone());
    } else {
        keys.swap(0, 1);
    }
    assert!(InteractionFrameV2::decode(
        serde_json::to_vec(&value)
            .expect("invalid range serializes")
            .as_slice()
    )
    .is_err());
}

/// 证明 closed union 拒绝 Replace/delete/move/trim 等非 V3 operation。
#[test]
fn timeline_delta_rejects_unknown_or_replace_operations() {
    let (mut value, _) = canonical_snapshot();
    let mutation = &mut value["mutations"][0]["mutation"];
    *mutation = serde_json::json!({
        "kind": "timelineDelta",
        "operations": [{ "kind": "replaceView", "view": {} }]
    });
    assert!(matches!(
        InteractionFrameV2::decode(
            serde_json::to_vec(&value)
                .expect("unknown operation serializes")
                .as_slice()
        ),
        Err(InteractionFrameError::Decode(_))
    ));
}
