use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceId, SurfaceScope, TypedSurfaceMutation,
};
use serde_json::json;

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{
    assert_group_created_contract_for, sha256_hex, surface_names,
    ScheduleCancelFailureScenarioTrace, ScheduleCancelPersistFailureScenarioTrace,
    ScheduleCancelRecoveryScenarioTrace, ScheduleCanceledScenarioTrace, SendScenarioSpec,
};

pub const CHANNEL_ID: &str = "chfixx00000000000000000009";
pub const REQUEST_ID: &str = "g09-wdio-request-1";
pub const MARKER: &str = "g09-cancel-fixture-marker";
pub const ACTOR_ID: &str = "g09-wdio-actor";
pub const OBSERVER_ID: &str = "g09-wdio-observer";
pub const TEAM_ID: &str = "g09-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_900_002;
pub const GROUP_NAME: &str = "G09 Fixture Group";

/// 返回 G09 fresh-group、身份、时钟与被取消排程的唯一确定性参数。
pub fn g09_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: "srvg0900000000000000000001",
        request_id: REQUEST_ID,
        marker: MARKER,
        simple_message: "g09-cancel-preview",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g09-wdio.invalid",
    }
}

/// 为 G09 暴露 happy/failure/replay/recovery 的共享生产回放入口。
pub struct G09ScenarioRunner;

#[allow(dead_code)]
impl G09ScenarioRunner {
    /// 回放 G08 权威排程前置、G09 cancel、原子持久化与迟到 HTTP。
    pub fn run_canceled() -> ScheduleCanceledScenarioTrace {
        shared::G01ScenarioRunner::run_schedule_canceled_for(g09_scenario_spec())
    }

    /// 回放 cancelSchedule HTTP 失败且不产生 terminal Frame。
    pub fn run_http_failure() -> ScheduleCancelFailureScenarioTrace {
        shared::G01ScenarioRunner::run_schedule_cancel_http_failure_for(g09_scenario_spec())
    }

    /// 回放 cancel PersistAtomic 失败并以同一 authority revision 重放。
    pub fn run_persist_failure_replay() -> ScheduleCancelPersistFailureScenarioTrace {
        shared::G01ScenarioRunner::run_schedule_cancel_persist_failure_for(g09_scenario_spec())
    }

    /// 从 durable canceled channel read model 恢复 terminal summary。
    pub fn run_recovery() -> ScheduleCancelRecoveryScenarioTrace {
        shared::G01ScenarioRunner::run_schedule_cancel_recovery_for(g09_scenario_spec())
    }
}

/// 取得 G09 channel-summary mutation 的 typed view，拒绝其他 Surface 或 mutation。
fn channel_summary_view(
    frame: &InteractionFrameV2,
) -> &helix_im::interaction_frame::ChannelSummaryVm {
    frame.validate().expect("G09 Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageChannelSummary);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Channel {
            channel_id: CHANNEL_ID.to_string(),
        }
    );
    match &mutation.mutation.mutation {
        TypedSurfaceMutation::ChannelSummarySnapshot { view } => view,
        TypedSurfaceMutation::ChannelSummaryPatch { patch } => &patch.view,
        other => panic!("expected channel summary mutation, got {other:?}"),
    }
}

/// 逐字段验证 G09 前置排程仍由完整 G08 authority Frame 提供。
pub fn assert_scheduled_prerequisite_contract(frame: &InteractionFrameV2) {
    let view = channel_summary_view(frame);
    assert!(view.has_scheduled_message);
    assert_eq!(view.schedule_id.as_deref(), Some("g08-schedule-1"));
    assert_eq!(view.schedule_owner_user_id.as_deref(), Some(ACTOR_ID));
    assert_eq!(view.schedule_message.as_deref(), Some(MARKER));
    assert_eq!(
        view.schedule_message_preview.as_deref(),
        Some("g09-cancel-preview")
    );
    assert_eq!(view.schedule_send_at, Some(CREATED_AT + 60_000));
    assert_eq!(view.schedule_status.as_deref(), Some("scheduled"));
    assert_eq!(view.schedule_revision, 8);
    assert!(view
        .actions
        .iter()
        .any(|action| action.kind == "cancelSchedule" && action.enabled));
}

/// 逐字段验证 G09 canceled terminal 清空内容、撤销 action 并保留单调 revision。
pub fn assert_canceled_summary_contract(
    frame: &InteractionFrameV2,
    expected_causation: Option<&str>,
) {
    assert_eq!(frame.causation_id.as_deref(), expected_causation);
    let mutation = &frame.mutations[0];
    assert_eq!(
        mutation.mutation.causation_id.as_deref(),
        expected_causation
    );
    let view = channel_summary_view(frame);
    assert!(!view.has_scheduled_message);
    assert!(view.schedule_id.is_none());
    assert!(view.schedule_owner_user_id.is_none());
    assert!(view.schedule_message.is_none());
    assert!(view.schedule_message_preview.is_none());
    assert!(view.schedule_send_at.is_none());
    assert_eq!(view.schedule_status.as_deref(), Some("canceled"));
    assert_eq!(view.schedule_revision, 9);
    assert!(view
        .actions
        .iter()
        .all(|action| action.kind != "cancelSchedule"));
}
