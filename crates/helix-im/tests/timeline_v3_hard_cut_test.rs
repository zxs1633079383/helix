//! Timeline V3 正向 producer hard-cut 与非 Timeline V2 保留合同。

use helix_im::interaction_frame::{InteractionFrameError, InteractionFrameV2};
use serde_json::Value;

/// 证明 schema 2 Timeline Snapshot/Replace 无条件 fail closed。
#[test]
fn positive_timeline_v2_is_rejected() {
    let mut legacy: Value = serde_json::from_slice(include_bytes!(
        "fixtures/g10b-wdio/latest-loaded.delta.frame-v3.json"
    ))
    .expect("canonical Timeline V3 fixture parses");
    legacy["schemaVersion"] = Value::from(2);
    legacy["mutations"][0]
        .as_object_mut()
        .expect("mutation is an object")
        .remove("surfaceSchemaVersion");
    assert!(matches!(
        InteractionFrameV2::decode(
            serde_json::to_vec(&legacy)
                .expect("legacy frame serializes")
                .as_slice()
        ),
        Err(InteractionFrameError::TimelineRequiresV3)
    ));
}

/// 证明不含 Timeline 的 schema 2 Surface 仍是合法资产。
#[test]
fn non_timeline_v2_remains_accepted() {
    InteractionFrameV2::decode(include_bytes!(
        "fixtures/g15a-wdio/channel-created.frame-v2.json"
    ))
    .expect("checked-in non-timeline V2 remains valid");
}
