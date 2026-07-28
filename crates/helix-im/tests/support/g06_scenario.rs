use helix_im::interaction_frame::{
    DeliveryVm, InteractionFrameV2, MessageContentVm, SurfaceId, SurfaceScope,
};
use serde_json::json;

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{
    assert_group_created_contract_for, first_message, sha256_hex, surface_names,
    ReactionFailureScenarioTrace, ReactionPersistFailureScenarioTrace,
    ReactionRecoveryScenarioTrace, ReactionWsScenarioTrace, SendScenarioSpec,
};

pub const CHANNEL_ID: &str = "chreaction0000000000000001";
pub const SERVER_ID: &str = "srvreaction000000000000001";
pub const REQUEST_ID: &str = "g06-wdio-request-1";
pub const MARKER: &str = "g06-reaction-fixture-marker";
pub const ACTOR_ID: &str = "g06-wdio-actor";
pub const OBSERVER_ID: &str = "g06-wdio-observer";
pub const TEAM_ID: &str = "g06-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_600_002;
pub const GROUP_NAME: &str = "G06 Fixture Group";
pub const EMOJI: &str = ":thumbsup:";

/// 返回 G06 快捷回复 emoji 的唯一确定性场景参数。
pub fn g06_scenario_spec() -> SendScenarioSpec {
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
        api_base_url: "http://g06-wdio.invalid",
    }
}

/// 为 G06 提供 action、authority、失败与恢复的共享确定性回放入口。
pub struct G06ScenarioRunner;

impl G06ScenarioRunner {
    /// 回放快捷回复从 opaque action 到 WS authority 的完整成功链。
    pub fn run_ws_reaction() -> ReactionWsScenarioTrace {
        shared::G01ScenarioRunner::run_reaction_ws_for(g06_scenario_spec())
    }

    /// 回放快捷回复 HTTP 基础设施失败且保持既有 timeline。
    pub fn run_http_failure() -> ReactionFailureScenarioTrace {
        shared::G01ScenarioRunner::run_reaction_http_failure_for(g06_scenario_spec())
    }

    /// 回放 reaction PersistAtomic 失败后由同一 authority fact 重放收敛。
    pub fn run_persist_failure_replay() -> ReactionPersistFailureScenarioTrace {
        shared::G01ScenarioRunner::run_reaction_persist_failure_for(g06_scenario_spec())
    }

    /// 从 durable reacted row 冷启动恢复 reaction snapshot。
    pub fn run_recovery() -> ReactionRecoveryScenarioTrace {
        shared::G01ScenarioRunner::run_reaction_recovery_for(g06_scenario_spec())
    }
}

/// 逐字段验证 reaction 前 timeline 的权限、内容和 opaque action。
pub fn assert_reactable_contract(frame: &InteractionFrameV2) {
    let spec = g06_scenario_spec();
    frame.validate().expect("G06 reactable Frame remains valid");
    assert_eq!(frame.schema_version, 3);
    assert_eq!(frame.causation_id, None);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Timeline {
            channel_id: spec.channel_id.to_string(),
            window_token: "latest".to_string(),
        }
    );
    assert!(mutation.mutation.revision > 0);
    assert!(frame.directives.is_empty());
    let message = first_message(frame);
    assert_eq!(message.id, spec.server_id);
    assert_eq!(message.delivery, DeliveryVm::Sent);
    assert!(matches!(
        &message.content,
        MessageContentVm::Text { text } if text == spec.marker
    ));
    assert!(message.reactions.is_empty());
    let react = message
        .actions
        .iter()
        .find(|action| action.kind == "react")
        .expect("G06 reactable row exposes react");
    assert!(react.enabled);
    assert!(!react.action_ref.is_empty());
    assert_eq!(react.surface_epoch, mutation.surface_epoch);
}

/// 逐字段验证权威 reaction 后的 content/delivery/reaction/action 与 causation。
pub fn assert_reacted_contract(frame: &InteractionFrameV2, expected_causation: Option<&str>) {
    let spec = g06_scenario_spec();
    frame.validate().expect("G06 reacted Frame remains valid");
    assert_eq!(frame.schema_version, 3);
    assert_eq!(frame.causation_id.as_deref(), expected_causation);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(
        mutation.mutation.causation_id.as_deref(),
        expected_causation
    );
    assert!(mutation.mutation.revision > 0);
    assert!(frame.directives.is_empty());
    let message = first_message(frame);
    assert_eq!(message.id, spec.server_id);
    assert_eq!(message.delivery, DeliveryVm::Sent);
    assert!(matches!(
        &message.content,
        MessageContentVm::Text { text } if text == spec.marker
    ));
    assert_eq!(message.reactions.len(), 1);
    assert_eq!(message.reactions[0].emoji, EMOJI);
    assert_eq!(message.reactions[0].count, 1);
    let react = message
        .actions
        .iter()
        .find(|action| action.kind == "react")
        .expect("G06 reacted row keeps its react binding");
    assert!(react.enabled);
    assert_eq!(react.surface_epoch, mutation.surface_epoch);
}
