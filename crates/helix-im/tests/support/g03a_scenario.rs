use helix_im::interaction_frame::{InteractionFrameV2, MessageContentVm};
use serde_json::json;

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{
    assert_dialog_contract_for, assert_group_created_contract_for, delivery_kinds, first_message,
    surface_names, RetryHttpScenarioTrace, RetryRecoveryScenarioTrace, RetryWsScenarioTrace,
    SendScenarioSpec, Terminal,
};

pub const CHANNEL_ID: &str = "chretry0000000000000000001";
pub const SERVER_ID: &str = "srvretry000000000000000001";
pub const REQUEST_ID: &str = "g03a-wdio-request-1";
pub const MARKER: &str = "g03a-retry-fixture-marker";
pub const ACTOR_ID: &str = "g03a-wdio-actor";
pub const OBSERVER_ID: &str = "g03a-wdio-observer";
pub const TEAM_ID: &str = "g03a-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_200_002;
pub const GROUP_NAME: &str = "G03a Fixture Group";

/// 返回 G03a 失败重试的唯一确定性文本场景参数。
pub fn g03a_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: SERVER_ID,
        request_id: REQUEST_ID,
        marker: MARKER,
        simple_message: MARKER,
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g03a-wdio.invalid",
    }
}

/// 为 G03a 提供失败后重试的共享确定性回放入口。
pub struct G03aScenarioRunner;

#[allow(dead_code)]
impl G03aScenarioRunner {
    /// 回放首次失败后重试到指定 HTTP 终态。
    pub fn run_http(terminal: Terminal) -> RetryHttpScenarioTrace {
        shared::G01ScenarioRunner::run_retry_http_for(g03a_scenario_spec(), terminal)
    }

    /// 回放 retry 后 WS 先行结算与重复 authority 幂等分支。
    pub fn run_ws_echo_with_dialog() -> RetryWsScenarioTrace {
        shared::G01ScenarioRunner::run_retry_ws_echo_with_dialog_for(g03a_scenario_spec())
    }

    /// 从 durable failed row 冷启动恢复并完成 retry。
    pub fn run_recovery() -> RetryRecoveryScenarioTrace {
        shared::G01ScenarioRunner::run_retry_recovery_for(g03a_scenario_spec())
    }
}

/// 逐字段验证 G03a timeline、身份复用、retry action 与 causation。
pub fn assert_retry_timeline_contract(
    frame: &InteractionFrameV2,
    expected_delivery: &str,
    expected_id: &str,
) {
    let spec = g03a_scenario_spec();
    shared::assert_timeline_contract_for(
        frame,
        &spec,
        expected_delivery,
        expected_id,
        Some(spec.request_id),
    );
    let MessageContentVm::Text { text } = &first_message(frame).content else {
        panic!("G03a timeline contract requires text content")
    };
    assert_eq!(text, MARKER);
    let retry = first_message(frame)
        .actions
        .iter()
        .find(|action| action.kind == "retry")
        .expect("G03a timeline must expose the typed retry action");
    assert_eq!(retry.surface_epoch, frame.mutations[0].surface_epoch);
    assert_eq!(
        retry.enabled,
        expected_delivery == "failed",
        "retry is executable only while the row is failed"
    );
}
