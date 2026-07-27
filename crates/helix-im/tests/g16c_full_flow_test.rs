//! G16c channel settings 的 Helix sans-IO 完整流程合同。

#[path = "support/g15a_scenario.rs"]
mod prerequisite;
#[path = "support/g16c_scenario.rs"]
mod scenario;

use scenario::{assert_settings_contract, G16cScenarioRunner};

#[test]
/// 证明 canonical G16c 验收仍先建立 fresh group authority。
fn prerequisite_creates_fresh_group_before_settings_change() {
    let created = prerequisite::G15aScenarioRunner::run_http_created();
    prerequisite::assert_created_contract(&created.created, Some(prerequisite::REQUEST_ID));
}

#[test]
/// 证明 info HTTP authority 仅在 channel PersistOk 后发布完整 settings patch。
fn info_change_is_durable_and_field_complete() {
    let trace = G16cScenarioRunner::run_info();
    assert_settings_contract(&trace.initial, "G16c Group", "旧简介", "旧规则", 7, false);
    assert_settings_contract(
        &trace.terminal,
        "G16c Renamed",
        "服务端简介",
        "服务端规则",
        8,
        true,
    );
    assert!(trace.persist_corr.raw() > 0);
}

#[test]
/// 证明 displayName 与 notice 专用命令复用同一 typed summary 屏障。
fn dedicated_settings_commands_share_the_authoritative_frame_contract() {
    let display = G16cScenarioRunner::run_display_name();
    assert_settings_contract(
        &display.terminal,
        "G16c Display",
        "旧简介",
        "旧规则",
        9,
        true,
    );
    let notice = G16cScenarioRunner::run_notice();
    assert_settings_contract(
        &notice.terminal,
        "G16c Group",
        "旧简介",
        "G16c 公告",
        10,
        true,
    );
}

#[test]
/// 证明 observer WS authority 经持久屏障发布，重复同版本事实不再出帧。
fn observer_update_channel_is_durable_and_duplicate_safe() {
    let (trace, duplicate_frames) = G16cScenarioRunner::run_ws_update();
    assert_settings_contract(
        &trace.terminal,
        "G16c Observer",
        "观察者简介",
        "观察者规则",
        11,
        true,
    );
    assert_eq!(duplicate_frames, 0);
}

#[test]
/// 证明业务、传输和本地持久失败都不会形成 typed 完成态。
fn failures_are_fail_closed() {
    assert_eq!(G16cScenarioRunner::run_failure(false, false), 0);
    assert_eq!(G16cScenarioRunner::run_failure(true, false), 0);
    assert_eq!(G16cScenarioRunner::run_failure(false, true), 0);
}

#[test]
/// 证明冷启动可直接从 durable channel row 重建 settings snapshot。
fn recovery_rebuilds_settings_from_durable_channel_row() {
    assert_settings_contract(
        &G16cScenarioRunner::run_recovery(),
        "G16c Recovered",
        "恢复简介",
        "恢复规则",
        10,
        false,
    );
}
