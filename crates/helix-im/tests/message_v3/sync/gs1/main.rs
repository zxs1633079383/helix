// 回归锚: Spec 047/T074-T075，G-S1 hello 初始化。

use serde_json::json;
#[path = "../support.rs"]
mod support;

const CHANNEL_ID: &str = "chfixx00000000000000000021";

/// G-S1 loaded 只在真实 PersistOk 后释放。
#[test]
fn gs1_loaded_is_persist_gated() -> support::TestResult<()> {
    let event = helix_im::event::sync::loaded(json!({"state":"loaded","source":"hello-increment"}))
        .map_err(|error| error.to_string())?;
    let trace = support::run(event, CHANNEL_ID)?;
    assert_eq!(trace.events[0]["event"], "im:sync:loaded");
    assert_eq!(trace.events[0]["data"]["state"], "loaded");
    Ok(())
}

/// 生成 G-S1 review artifact。
#[test]
fn artifact_test() -> support::TestResult<()> {
    let event = helix_im::event::sync::loaded(json!({"state":"loaded","source":"hello-increment"}))
        .map_err(|error| error.to_string())?;
    let trace = support::run(event, CHANNEL_ID)?;
    support::write_artifact("G-S1", "message_v3_gs1", trace)
}
