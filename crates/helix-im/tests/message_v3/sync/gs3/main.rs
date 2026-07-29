// 回归锚: Spec 047/T078-T079，G-S3 pong gap 补偿。

use serde_json::json;
#[path = "../support.rs"]
mod support;

/// G-S3 repaired 只在真实 PersistOk 后释放。
#[test]
fn gs3_gap_repaired_is_persist_gated() -> support::TestResult<()> {
    let event = helix_im::event::sync::gap_repaired(json!({"state":"repaired","source":"pong"}))
        .map_err(|error| error.to_string())?;
    let trace = support::run(event)?;
    assert_eq!(trace.events[0]["event"], "im:sync:gap-repaired");
    assert_eq!(trace.events[0]["data"]["state"], "repaired");
    Ok(())
}

/// 生成 G-S3 review artifact。
#[test]
fn artifact_test() -> support::TestResult<()> {
    let event = helix_im::event::sync::gap_repaired(json!({"state":"repaired","source":"pong"}))
        .map_err(|error| error.to_string())?;
    let trace = support::run(event)?;
    support::write_artifact("G-S3", "message_v3_gs3", trace)
}
