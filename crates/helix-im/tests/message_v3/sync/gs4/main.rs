// 回归锚: Spec 047/T080-T081，G-S4 陌生频道补全。

use serde_json::json;
#[path = "../support.rs"]
mod support;

/// G-S4 hydrated 只在真实 PersistOk 后释放。
#[test]
fn gs4_channel_hydrated_is_persist_gated() -> support::TestResult<()> {
    let event = helix_im::event::sync::channel_hydrated(
        json!({"channelId":"chfixx00000000000000000024","state":"hydrated","needSync":true}),
    )
    .map_err(|error| error.to_string())?;
    let trace = support::run(event)?;
    assert_eq!(trace.events[0]["event"], "im:sync:channel-hydrated");
    assert_eq!(trace.events[0]["data"]["state"], "hydrated");
    Ok(())
}

/// 生成 G-S4 review artifact。
#[test]
fn artifact_test() -> support::TestResult<()> {
    let event = helix_im::event::sync::channel_hydrated(
        json!({"channelId":"chfixx00000000000000000024","state":"hydrated","needSync":true}),
    )
    .map_err(|error| error.to_string())?;
    let trace = support::run(event)?;
    support::write_artifact("G-S4", "message_v3_gs4", trace)
}
