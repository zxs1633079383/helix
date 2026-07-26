use std::collections::BTreeMap;

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::handle_outbound;
use helix_im::interaction_frame::{
    ChannelRole, ParticipantReceiptRowVm, ReceiptKind, ReceiptStatus, SurfaceDataVm,
    SurfaceLoadState, UserRefVm,
};
use helix_im::projector::participant_receipts::{
    ParticipantReceiptsData, ParticipantReceiptsProjector, ParticipantReceiptsRequest,
};
use serde_json::{json, Value};

#[path = "support/g14_scenario.rs"]
#[allow(dead_code)]
mod scenario;

use scenario::{assert_receipt_contract, G14ScenarioRunner};

/// 执行一个 outbound 命令并返回唯一 HTTP URL 与 JSON body。
fn one_http(name: &str, payload: Value) -> (String, Value) {
    let effects = handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        "http://x/api/cses",
        "http://x/api/cses",
        Some("conn-g14"),
        Correlation::from_raw(14),
    )
    .expect("canonical receipt command");
    match effects.as_slice() {
        [Effect::Http { req, .. }] => (
            req.url.clone(),
            serde_json::from_slice(req.body.as_ref().unwrap()).unwrap(),
        ),
        other => panic!("expected one HTTP effect, got {other:?}"),
    }
}

/// 构造模板确认的显式 completed 数据，用于验证 authority fail-closed。
fn completed_data() -> ParticipantReceiptsData {
    ParticipantReceiptsData {
        summary: SurfaceDataVm::Record(BTreeMap::new()),
        completed: vec![ParticipantReceiptRowVm {
            user: UserRefVm {
                user_id: "u-1".into(),
            },
            member_role: ChannelRole::Member,
            status: ReceiptStatus::Completed,
            completed_at: Some(1),
        }],
        pending: Vec::new(),
        state: SurfaceLoadState::Ready,
    }
}

/// 证明模板收到 wire 与 G14 单条已读命令保持独立。
#[test]
fn template_received_wire_is_not_a_confirmation_command() {
    let (url, body) = one_http("im_template_received", json!({"post_id":"message-14"}));
    assert_eq!(url, "http://x/api/cses/post/templateReceived");
    assert_eq!(body, json!({"postId":"message-14"}));
}

/// 证明缺少 Go authority 时不能伪造模板确认 ready surface。
#[test]
fn template_confirmation_ready_data_is_fail_closed_until_go_has_authority() {
    let mut projector = ParticipantReceiptsProjector::default();
    let request = ParticipantReceiptsRequest::new(
        "channel-14",
        "message-14",
        ReceiptKind::TemplateConfirmation,
        "cursor-14",
    );
    let error = projector.snapshot(request, completed_data()).unwrap_err();
    assert!(error.to_string().contains("authority unavailable"));
}

/// 证明模板确认当前只允许发布显式 forbidden snapshot。
#[test]
fn template_confirmation_can_only_render_explicit_forbidden_state() {
    let mut projector = ParticipantReceiptsProjector::default();
    let request = ParticipantReceiptsRequest::new(
        "channel-14",
        "message-14",
        ReceiptKind::TemplateConfirmation,
        "cursor-14",
    );
    let mutation = projector.snapshot_forbidden(request).unwrap();
    assert_eq!(mutation.causation_id, None);
}

/// 证明 G14 terminal receipt Frame 只在 production PersistOk 与 authority query 后产生。
#[test]
fn post_read_terminal_frame_waits_for_durable_authority() {
    let trace = G14ScenarioRunner::run_success();
    assert_receipt_contract(&trace.initial, 1, false);
    assert_receipt_contract(&trace.terminal, 2, true);
}

/// 证明重复 WS authority 不重复生成 receipt Frame 或持久写。
#[test]
fn post_read_duplicate_authority_is_suppressed() {
    let trace = G14ScenarioRunner::run_duplicate_authority();
    assert_eq!(trace.frame_count, 0);
    assert_eq!(trace.persist_count, 0);
    assert_eq!(trace.receipt_http_count, 0);
}

/// 证明失败后的 cold attach 仍从 authority snapshot 恢复，不依赖平台状态。
#[test]
fn post_read_recovery_rebuilds_complete_receipt_fields() {
    assert_receipt_contract(&G14ScenarioRunner::run_recovery(), 1, true);
}
