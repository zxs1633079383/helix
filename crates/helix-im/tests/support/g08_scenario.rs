use helix_im::interaction_frame::{
    InteractionFrameV2, SurfaceId, SurfaceScope, TypedSurfaceMutation,
};
use serde_json::json;

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{
    assert_group_created_contract_for, sha256_hex, surface_names, ScheduleCreatedScenarioTrace,
    ScheduleFailureScenarioTrace, SchedulePersistFailureScenarioTrace,
    ScheduleRecoveryScenarioTrace, SendScenarioSpec,
};

pub const CHANNEL_ID: &str = "chfixx00000000000000000008";
pub const REQUEST_ID: &str = "g08-wdio-request-1";
pub const MARKER: &str = "g08-schedule-fixture-marker";
pub const ACTOR_ID: &str = "g08-wdio-actor";
pub const OBSERVER_ID: &str = "g08-wdio-observer";
pub const TEAM_ID: &str = "g08-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_800_002;
pub const GROUP_NAME: &str = "G08 Fixture Group";

/// 返回 G08 fresh-group、身份、时钟与 schedule 内容的唯一确定性参数。
pub fn g08_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: "srvg0800000000000000000001",
        request_id: REQUEST_ID,
        marker: MARKER,
        simple_message: "g08-schedule-preview",
        message_type: "TEXT",
        props: json!({}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g08-wdio.invalid",
    }
}

/// 为 G08 暴露 happy/failure/replay/recovery 的共享生产回放入口。
pub struct G08ScenarioRunner;

#[allow(dead_code)]
impl G08ScenarioRunner {
    /// 回放 command、WS authority、原子持久化、readback 与迟到 HTTP。
    pub fn run_created() -> ScheduleCreatedScenarioTrace {
        shared::G01ScenarioRunner::run_schedule_created_for(g08_scenario_spec())
    }

    /// 回放 createSchedule HTTP 失败且不产生终态 Frame。
    pub fn run_http_failure() -> ScheduleFailureScenarioTrace {
        shared::G01ScenarioRunner::run_schedule_http_failure_for(g08_scenario_spec())
    }

    /// 回放 schedule PersistAtomic 失败并以同一 authority revision 重放。
    pub fn run_persist_failure_replay() -> SchedulePersistFailureScenarioTrace {
        shared::G01ScenarioRunner::run_schedule_persist_failure_for(g08_scenario_spec())
    }

    /// 从 durable channel read model 恢复 schedule channel-summary。
    pub fn run_recovery() -> ScheduleRecoveryScenarioTrace {
        shared::G01ScenarioRunner::run_schedule_recovery_for(g08_scenario_spec())
    }
}

/// 逐字段验证 G08 未排程或已排程 channel-summary 的 typed 合同。
pub fn assert_schedule_summary_contract(
    frame: &InteractionFrameV2,
    scheduled: bool,
    expected_causation: Option<&str>,
) {
    frame.validate().expect("G08 Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.causation_id.as_deref(), expected_causation);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageChannelSummary);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Channel {
            channel_id: CHANNEL_ID.to_string(),
        }
    );
    assert_eq!(
        mutation.mutation.causation_id.as_deref(),
        expected_causation
    );
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::ChannelSummarySnapshot { view } => view,
        TypedSurfaceMutation::ChannelSummaryPatch { patch } => &patch.view,
        other => panic!("expected channel summary mutation, got {other:?}"),
    };
    assert_eq!(view.channel_id, CHANNEL_ID);
    assert_eq!(view.has_scheduled_message, scheduled);
    if scheduled {
        assert_eq!(view.schedule_id.as_deref(), Some("g08-schedule-1"));
        assert_eq!(view.schedule_owner_user_id.as_deref(), Some(ACTOR_ID));
        assert_eq!(view.schedule_message.as_deref(), Some(MARKER));
        assert_eq!(
            view.schedule_message_preview.as_deref(),
            Some("g08-schedule-preview")
        );
        assert_eq!(view.schedule_send_at, Some(CREATED_AT + 60_000));
        assert_eq!(view.schedule_status.as_deref(), Some("scheduled"));
        assert_eq!(view.schedule_revision, 8);
        let cancel = view
            .actions
            .iter()
            .find(|action| action.kind == "cancelSchedule")
            .expect("scheduled owner summary exposes cancelSchedule");
        assert!(cancel.enabled);
        assert_eq!(cancel.surface_epoch, mutation.surface_epoch);
    } else {
        assert!(view.schedule_id.is_none());
        assert!(view.schedule_message.is_none());
        assert!(
            view.actions
                .iter()
                .all(|action| action.kind != "cancelSchedule"),
            "unscheduled summary cannot expose cancelSchedule"
        );
    }
}
