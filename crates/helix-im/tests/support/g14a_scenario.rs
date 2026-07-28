use helix_im::interaction_frame::{
    DeliveryVm, InteractionFrameV2, MessageContentVm, SurfaceId, SurfaceScope,
};
use serde_json::json;

#[path = "../g01_shared/scenario.rs"]
#[allow(dead_code)]
mod shared;

#[allow(unused_imports)]
pub use shared::{
    assert_group_created_contract_for, first_message, sha256_hex, surface_names, SendScenarioSpec,
    TemplateReceivedFailureScenarioTrace, TemplateReceivedPersistFailureScenarioTrace,
    TemplateReceivedRecoveryScenarioTrace, TemplateReceivedWsScenarioTrace,
};

pub const CHANNEL_ID: &str = "chtemplate0000000000000001";
pub const SERVER_ID: &str = "srvtemplate000000000000001";
pub const REQUEST_ID: &str = "g14a-wdio-request-1";
pub const MARKER: &str = "g14a-template-fixture-marker";
pub const ACTOR_ID: &str = "g14a-wdio-actor";
pub const OBSERVER_ID: &str = "g14a-wdio-observer";
pub const TEAM_ID: &str = "g14a-wdio-company";
pub const CREATED_AT: i64 = 1_700_001_400_102;
pub const GROUP_NAME: &str = "G14a Fixture Group";

/// 返回 G14a 模板已收到的唯一确定性场景参数。
pub fn g14a_scenario_spec() -> SendScenarioSpec {
    SendScenarioSpec {
        channel_id: CHANNEL_ID,
        server_id: SERVER_ID,
        request_id: REQUEST_ID,
        marker: MARKER,
        simple_message: MARKER,
        message_type: "TEMPLATE",
        props: json!({"template": {"userIds": []}}),
        actor_id: ACTOR_ID,
        observer_id: OBSERVER_ID,
        team_id: TEAM_ID,
        created_at: CREATED_AT,
        group_name: GROUP_NAME,
        api_base_url: "http://g14a-wdio.invalid",
    }
}

/// 为 G14a 提供 admission、authority、失败与恢复的共享确定性回放入口。
pub struct G14AScenarioRunner;

impl G14AScenarioRunner {
    /// 回放模板已收到从 direct command 到 WS authority 的完整成功链。
    pub fn run_ws_received() -> TemplateReceivedWsScenarioTrace {
        shared::G01ScenarioRunner::run_template_received_ws_for(g14a_scenario_spec())
    }

    /// 回放模板已收到 HTTP 基础设施失败且保持既有 timeline。
    pub fn run_http_failure() -> TemplateReceivedFailureScenarioTrace {
        shared::G01ScenarioRunner::run_template_received_http_failure_for(g14a_scenario_spec())
    }

    /// 回放 template PersistAtomic 失败后由同一 authority fact 重放收敛。
    pub fn run_persist_failure_replay() -> TemplateReceivedPersistFailureScenarioTrace {
        shared::G01ScenarioRunner::run_template_received_persist_failure_for(g14a_scenario_spec())
    }

    /// 从 durable template confirmation row 冷启动恢复 snapshot。
    pub fn run_recovery() -> TemplateReceivedRecoveryScenarioTrace {
        shared::G01ScenarioRunner::run_template_received_recovery_for(g14a_scenario_spec())
    }
}

/// 逐字段验证模板 authority 前 timeline 仍为未收到且无可执行确认 binding。
pub fn assert_receivable_contract(frame: &InteractionFrameV2) {
    assert_template_contract(frame, false);
}

/// 逐字段验证模板 authority 后 timeline 的确认计数与 reader identity。
pub fn assert_received_contract(frame: &InteractionFrameV2) {
    assert_template_contract(frame, true);
}

/// 校验 G14a timeline envelope、scope、内容、确认字段与 action 边界。
fn assert_template_contract(frame: &InteractionFrameV2, received: bool) {
    let spec = g14a_scenario_spec();
    frame.validate().expect("G14a timeline Frame remains valid");
    assert_eq!(frame.schema_version, 3);
    assert_eq!(frame.causation_id, None);
    assert_eq!(frame.mutations.len(), 1);
    assert!(frame.directives.is_empty());
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
    let message = first_message(frame);
    assert_eq!(message.id, spec.server_id);
    assert_eq!(message.author.user_id, spec.observer_id);
    assert_eq!(message.delivery, DeliveryVm::Sent);
    assert!(matches!(
        &message.content,
        MessageContentVm::Text { text } if text == spec.marker
    ));
    let confirmation = serde_json::to_value(
        message
            .template_confirmation
            .as_ref()
            .expect("template message carries confirmation data"),
    )
    .expect("template confirmation serializes");
    assert_eq!(confirmation["templateReceived"], received);
    assert_eq!(confirmation["confirmedCount"], usize::from(received));
    assert_eq!(
        confirmation["readerIds"],
        if received {
            json!([spec.actor_id])
        } else {
            json!([])
        }
    );
    let confirm = message
        .actions
        .iter()
        .find(|action| action.kind == "templateConfirm")
        .expect("template row keeps an explicit fail-closed confirmation binding");
    assert!(!confirm.enabled);
    assert_eq!(
        confirm.disabled_reason.as_deref(),
        Some(if received {
            "notAvailable"
        } else {
            "unsupportedByAuthority"
        })
    );
    let read_receipt =
        serde_json::to_value(&message.read_receipt).expect("read receipt serializes");
    assert_eq!(read_receipt["readBits"], "");
    assert_eq!(
        read_receipt["readerIds"],
        json!([]),
        "template authority must not fabricate ordinary read receipt identities"
    );
    if let Some(reader_count) = read_receipt.get("readerCount") {
        assert_eq!(reader_count, 0);
    }
    if let Some(unread_count) = read_receipt.get("unreadCount") {
        assert_eq!(unread_count, 0);
    }
}
