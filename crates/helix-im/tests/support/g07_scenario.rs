use std::collections::BTreeMap;

use helix_im::interaction_frame::{
    InteractionFrameV2, ReceiptKind, ReceiptStatus, SurfaceDataVm, SurfaceId, SurfaceLoadState,
    SurfaceScope, TypedSurfaceMutation,
};
use serde_json::json;

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{
    assert_group_created_contract_for, first_message, sha256_hex, surface_names, SendScenarioSpec,
    UrgentConfirmScenarioTrace, UrgentFailureScenarioTrace, UrgentPersistFailureScenarioTrace,
    UrgentPostScenarioTrace, UrgentRecoveryScenarioTrace,
};

pub const CHANNEL_ID: &str = "churgent000000000000000001";
pub const SERVER_ID: &str = "srvurgent00000000000000001";
pub const REQUEST_ID: &str = "g07-wdio-request-1";
pub const MARKER: &str = "g07-urgent-fixture-marker";
pub const ACTOR_ID: &str = "g07-wdio-actor";
pub const OBSERVER_ID: &str = "g07-wdio-observer";
pub const TEAM_ID: &str = "g07-wdio-company";
pub const CREATED_AT: i64 = 1_700_000_700_002;
pub const GROUP_NAME: &str = "G07 Fixture Group";

/// 返回 G07 发起者场景的唯一确定性参数。
pub fn g07_scenario_spec() -> SendScenarioSpec {
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
        api_base_url: "http://g07-wdio.invalid",
    }
}

/// 返回 G07 确认者场景参数，并保持同一 channel/post authority。
fn g07_observer_spec() -> SendScenarioSpec {
    let mut spec = g07_scenario_spec();
    spec.actor_id = OBSERVER_ID;
    spec.observer_id = ACTOR_ID;
    spec.request_id = "g07-wdio-confirm-request-1";
    spec
}

/// 为 G07 提供发起、确认、失败、竞态与恢复的共享确定性入口。
pub struct G07ScenarioRunner;

impl G07ScenarioRunner {
    /// 回放发起加急到 pending receipt 的完整成功链。
    pub fn run_urgent_post() -> UrgentPostScenarioTrace {
        shared::G01ScenarioRunner::run_urgent_post_ws_for(g07_scenario_spec(), OBSERVER_ID)
    }

    /// 回放 Observer 确认加急到 completed receipt 的完整成功链。
    pub fn run_urgent_confirm() -> UrgentConfirmScenarioTrace {
        shared::G01ScenarioRunner::run_urgent_confirm_ws_for(g07_observer_spec(), ACTOR_ID)
    }

    /// 回放发起加急 HTTP 基础设施失败。
    pub fn run_http_failure() -> UrgentFailureScenarioTrace {
        shared::G01ScenarioRunner::run_urgent_http_failure_for(g07_scenario_spec(), OBSERVER_ID)
    }

    /// 回放加急 PersistAtomic 失败后的 authority 重放。
    pub fn run_persist_failure_replay() -> UrgentPersistFailureScenarioTrace {
        shared::G01ScenarioRunner::run_urgent_persist_failure_for(g07_scenario_spec(), OBSERVER_ID)
    }

    /// 从 durable expedite_map 恢复 timeline 与 receipt snapshots。
    pub fn run_recovery() -> UrgentRecoveryScenarioTrace {
        shared::G01ScenarioRunner::run_urgent_recovery_for(g07_scenario_spec(), OBSERVER_ID)
    }
}

/// 验证未加急消息只暴露发起 action，不伪造 active 状态。
pub fn assert_urgentable_contract(frame: &InteractionFrameV2) {
    frame
        .validate()
        .expect("G07 urgentable Frame remains valid");
    assert_eq!(frame.schema_version, 3);
    assert_eq!(frame.causation_id, None);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::Timeline {
            channel_id: CHANNEL_ID.to_string(),
            window_token: "latest".to_string(),
        }
    );
    let message = first_message(frame);
    let urgency = urgency_record(message.urgency.as_ref());
    assert_eq!(urgency_string(urgency, "urgentState"), "inactive");
    assert_eq!(urgency_u64(urgency, "urgentRequiredCount"), 0);
    let action = message
        .actions
        .iter()
        .find(|action| action.kind == "urgent")
        .expect("G07 urgentable row exposes urgent");
    assert!(action.enabled);
    assert_eq!(action.surface_epoch, mutation.surface_epoch);
}

/// 验证 active/acknowledged timeline 的 Helix-owned urgency aggregate。
pub fn assert_urgent_timeline_contract(
    frame: &InteractionFrameV2,
    confirmed: bool,
    expected_causation: Option<&str>,
) {
    frame.validate().expect("G07 urgent Frame remains valid");
    assert_eq!(frame.schema_version, 3);
    assert_eq!(frame.causation_id.as_deref(), expected_causation);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(mutation.mutation.surface, SurfaceId::MessageTimeline);
    assert_eq!(
        mutation.mutation.causation_id.as_deref(),
        expected_causation
    );
    let message = first_message(frame);
    let urgency = urgency_record(message.urgency.as_ref());
    assert_eq!(urgency_bool(urgency, "urgent"), true);
    assert_eq!(urgency_u64(urgency, "urgentRequiredCount"), 1);
    assert_eq!(
        urgency_u64(urgency, "urgentConfirmedCount"),
        u64::from(confirmed)
    );
    assert_eq!(
        urgency_string(urgency, "urgentState"),
        if confirmed { "acknowledged" } else { "active" }
    );
}

/// 验证 participant-receipts 的 scope、summary 与用户行均来自 Helix。
pub fn assert_urgent_receipts_contract(frame: &InteractionFrameV2, confirmed: bool) {
    frame.validate().expect("G07 receipt Frame remains valid");
    assert_eq!(frame.schema_version, 2);
    assert_eq!(frame.mutations.len(), 1);
    let mutation = &frame.mutations[0];
    assert_eq!(
        mutation.mutation.surface,
        SurfaceId::MessageParticipantReceipts
    );
    assert_eq!(
        mutation.mutation.scope,
        SurfaceScope::ParticipantReceipts {
            channel_id: CHANNEL_ID.to_string(),
            message_id: SERVER_ID.to_string(),
            receipt_kind: ReceiptKind::UrgentConfirmation,
        }
    );
    let view = match &mutation.mutation.mutation {
        TypedSurfaceMutation::ParticipantReceiptsSnapshot { view } => view,
        other => panic!("expected urgent receipt snapshot, got {other:?}"),
    };
    assert_eq!(view.kind, ReceiptKind::UrgentConfirmation);
    assert_eq!(view.state, SurfaceLoadState::Ready);
    let summary = surface_record(&view.summary);
    assert_eq!(urgency_u64(summary, "requiredCount"), 1);
    assert_eq!(urgency_u64(summary, "completedCount"), u64::from(confirmed));
    let rows = if confirmed {
        &view.completed
    } else {
        &view.pending
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user.user_id, OBSERVER_ID);
    assert_eq!(
        rows[0].status,
        if confirmed {
            ReceiptStatus::Completed
        } else {
            ReceiptStatus::Pending
        }
    );
}

/// 将 optional urgency 解包成强类型 record。
fn urgency_record(value: Option<&SurfaceDataVm>) -> &BTreeMap<String, SurfaceDataVm> {
    value.map(surface_record).expect("urgency record exists")
}

/// 将 SurfaceDataVm 解包成 record，拒绝平台式弱类型兜底。
fn surface_record(value: &SurfaceDataVm) -> &BTreeMap<String, SurfaceDataVm> {
    match value {
        SurfaceDataVm::Record(record) => record,
        other => panic!("expected surface record, got {other:?}"),
    }
}

/// 读取 Helix aggregate 中的字符串字段。
fn urgency_string<'a>(record: &'a BTreeMap<String, SurfaceDataVm>, key: &str) -> &'a str {
    match record.get(key) {
        Some(SurfaceDataVm::String(value)) => value,
        other => panic!("expected string {key}, got {other:?}"),
    }
}

/// 读取 Helix aggregate 中的无符号计数字段。
fn urgency_u64(record: &BTreeMap<String, SurfaceDataVm>, key: &str) -> u64 {
    match record.get(key) {
        Some(SurfaceDataVm::Number(value)) => value.as_u64().expect("count is unsigned"),
        other => panic!("expected number {key}, got {other:?}"),
    }
}

/// 读取 Helix aggregate 中的布尔字段。
fn urgency_bool(record: &BTreeMap<String, SurfaceDataVm>, key: &str) -> bool {
    match record.get(key) {
        Some(SurfaceDataVm::Bool(value)) => *value,
        other => panic!("expected bool {key}, got {other:?}"),
    }
}
