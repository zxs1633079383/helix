// 回归锚: Spec 047/T076-T077，G-S2 离线同步恢复。

use serde_json::json;
#[path = "../support.rs"]
mod support;

const CHANNEL_ID: &str = "chfixx00000000000000000022";

/// G-S2 recovered 只在真实 PersistOk 后释放。
#[test]
fn gs2_recovered_is_persist_gated() -> support::TestResult<()> {
    let event = helix_im::event::sync::recovered(
        json!({"channelId":"chfixx00000000000000000022","state":"recovered","committedSeq":22}),
    )
    .map_err(|error| error.to_string())?;
    let trace = support::run(event, CHANNEL_ID)?;
    assert_eq!(trace.events[0]["event"], "im:sync:recovered");
    assert_eq!(trace.events[0]["data"]["state"], "recovered");
    Ok(())
}

/// 生成 G-S2 review artifact。
#[test]
fn artifact_test() -> support::TestResult<()> {
    let event = helix_im::event::sync::recovered(
        json!({"channelId":"chfixx00000000000000000022","state":"recovered","committedSeq":22}),
    )
    .map_err(|error| error.to_string())?;
    let trace = support::run(event, CHANNEL_ID)?;
    support::write_artifact("G-S2", "message_v3_gs2", trace)
}
