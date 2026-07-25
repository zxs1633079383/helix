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
    OfflineRevokeNoopTrace, OfflineRevokePersistFailureTrace, OfflineRevokeScenarioTrace,
    RevokeRecoveryScenarioTrace, SendScenarioSpec,
};

pub const CHANNEL_ID: &str = "chrevoff000000000000000001";
pub const SERVER_ID: &str = "srvoffrev00000000000000000";
pub const REQUEST_ID: &str = "g04a-wdio-sync-1";
pub const MARKER: &str = "g04a-offline-revoke-fixture-marker";
pub const ACTOR_ID: &str = "g04a-wdio-actor";
pub const OBSERVER_ID: &str = "g04a-wdio-observer";
pub const TEAM_ID: &str = "g04a-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_400_002;
pub const GROUP_NAME: &str = "G04a Fixture Group";

/// 返回 G04a 离线撤回重放唯一确定性场景参数。
pub fn g04a_scenario_spec() -> SendScenarioSpec {
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
        api_base_url: "http://g04a-wdio.invalid",
    }
}

/// 为 G04a 提供 explicit sync、失败、幂等与恢复的共享确定性回放入口。
pub struct G04AScenarioRunner;

impl G04AScenarioRunner {
    /// 回放 explicit sync 从 Helix cursor 到 recalled Frame 的完整成功链。
    pub fn run_offline_revoke() -> OfflineRevokeScenarioTrace {
        shared::G01ScenarioRunner::run_offline_revoke_sync_for(g04a_scenario_spec())
    }

    /// 回放 authority 明确无变化且禁止发布业务副作用。
    pub fn run_no_change() -> OfflineRevokeNoopTrace {
        shared::G01ScenarioRunner::run_offline_revoke_no_change_for(g04a_scenario_spec())
    }

    /// 回放 sync/notify HTTP 基础设施失败且保持既有 timeline。
    pub fn run_http_failure() -> OfflineRevokeNoopTrace {
        shared::G01ScenarioRunner::run_offline_revoke_http_failure_for(g04a_scenario_spec())
    }

    /// 回放 PersistAtomic 失败后以新 sync correlation 重拉并收敛。
    pub fn run_persist_failure_replay() -> OfflineRevokePersistFailureTrace {
        shared::G01ScenarioRunner::run_offline_revoke_persist_failure_for(g04a_scenario_spec())
    }

    /// 从 durable revoked row 冷启动恢复 recalled snapshot。
    pub fn run_recovery() -> RevokeRecoveryScenarioTrace {
        shared::G01ScenarioRunner::run_revoke_recovery_for(g04a_scenario_spec())
    }
}

/// 逐字段验证离线撤回前 timeline 的内容、权限和 opaque action。
pub fn assert_revocable_contract(frame: &InteractionFrameV2) {
    let spec = g04a_scenario_spec();
    frame
        .validate()
        .expect("G04a revocable Frame remains valid");
    assert_eq!(frame.schema_version, 2);
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
    let revoke = message
        .actions
        .iter()
        .find(|action| action.kind == "revoke")
        .expect("G04a revocable row exposes revoke");
    assert!(revoke.enabled);
    assert!(!revoke.action_ref.is_empty());
    assert_eq!(revoke.surface_epoch, mutation.surface_epoch);
}

/// 逐字段验证离线 authority 提交后的 recalled content 与无平台 causation。
pub fn assert_offline_recalled_contract(frame: &InteractionFrameV2) {
    let spec = g04a_scenario_spec();
    frame
        .validate()
        .expect("G04a recalled Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(
        frame.causation_id, None,
        "offline authority has no platform action causation"
    );
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(mutation.mutation.causation_id, None);
    assert!(mutation.mutation.revision > 0);
    assert!(frame.directives.is_empty());
    let message = first_message(frame);
    assert_eq!(message.id, spec.server_id);
    assert_eq!(message.event_seq, 1);
    assert_eq!(message.delivery, DeliveryVm::Recalled);
    assert!(matches!(
        &message.content,
        MessageContentVm::Recalled { text } if text == spec.marker
    ));
    assert!(message.actions.is_empty());
    assert!(message.quick_actions.is_empty());
    assert!(message.reply_thread.is_none());
    assert!(message.reactions.is_empty());
    assert!(message.topic_link.is_none());
}
