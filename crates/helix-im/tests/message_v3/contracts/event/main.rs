// 回归锚: Spec 047/T009，MessageV3 envelope 与 PersistOk 释放屏障。

use helix_core::effect::{Effect, Row, SqlValue, StorageOp, UpsertSpec};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{EffectSink, Module, Tick};
use helix_im::event::post;
use helix_im::{ImConfig, ImModule};
use serde_json::{json, Value};

/// 从 Emit 中解析 MessageV3 JSON，拒绝测试侧重建业务信封。
fn emitted_json(effect: &Effect) -> Value {
    let Effect::Emit { event } = effect else {
        panic!("expected MessageV3 Emit, got {effect:?}");
    };
    serde_json::from_slice(event.0.as_ref()).expect("MessageV3 event must be JSON")
}

const CONTRACT_USER: &str = "message-v3-contract-user";
const CONTRACT_COMPANY: &str = "message-v3-contract-company";

/// 构造带可信频道归属的最小提交，确保 terminal event 经过真实 scope 屏障。
fn persisted_rows() -> Vec<StorageOp> {
    let channel: Row = vec![
        (
            "id".to_string(),
            SqlValue::Text("channel-message-v3-contract".to_string()),
        ),
        (
            "team_id".to_string(),
            SqlValue::Text(CONTRACT_COMPANY.to_string()),
        ),
        (
            "user_id".to_string(),
            SqlValue::Text(CONTRACT_USER.to_string()),
        ),
    ];
    let message: Row = vec![
        (
            "temporary_id".to_string(),
            SqlValue::Text("tmp-message-v3-contract".to_string()),
        ),
        (
            "channel_id".to_string(),
            SqlValue::Text("channel-message-v3-contract".to_string()),
        ),
    ];
    vec![
        StorageOp::BatchUpsert(UpsertSpec::new("channel", vec![channel], Some("id"))),
        StorageOp::BatchUpsert(UpsertSpec::new(
            "message",
            vec![message],
            Some("temporary_id"),
        )),
    ]
}

/// MessageV3 业务信封只能暴露 event 与 data，不携带呈现层套壳。
#[test]
fn message_v3_event_contains_only_event_and_data() {
    let effect = post::received(json!({
        "id": "post-message-v3-contract",
        "channelId": "channel-message-v3-contract"
    }))
    .expect("typed post event")
    .into_effect();
    let value = emitted_json(&effect);
    let keys = value
        .as_object()
        .expect("event envelope must be an object")
        .keys()
        .cloned()
        .collect::<Vec<_>>();

    assert_eq!(keys, vec!["data", "event"]);
    assert_eq!(value["event"], "im:post:received");
    let encoded = serde_json::to_string(&value).expect("event must re-encode");
    for retired in [
        "InteractionFrame",
        "Surface",
        "SurfaceMutation",
        "ActionBinding",
        "UiDirective",
        "FrameApplied",
    ] {
        assert!(
            !encoded.contains(retired),
            "retired field leaked: {retired}"
        );
    }
}

/// 只有匹配的 PersistOk 才释放一次 terminal event，错误和重复回报均为 no-op。
#[test]
fn matching_persist_ok_releases_terminal_event_exactly_once() {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: CONTRACT_USER.to_string(),
        company_id: CONTRACT_COMPANY.to_string(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    let terminal = post::received(json!({
        "id": "post-message-v3-contract",
        "channelId": "channel-message-v3-contract"
    }))
    .expect("typed post event");
    let corr = module.queue_message_v3_commit(persisted_rows(), vec![terminal], &mut sink);

    assert!(matches!(
        sink.as_slice(),
        [Effect::PersistAtomic { corr: emitted_corr, .. }] if *emitted_corr == corr
    ));
    assert!(!sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Emit { .. })));

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: helix_core::Correlation::from_raw(corr.raw() + 100),
                outcome: PortOutcome::Ok(ReplyBytes::default()),
            },
            1,
            &mut sink,
        )
        .expect("unknown correlation must be ignored");
    assert!(sink.is_empty());

    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes::default()),
            },
            2,
            &mut sink,
        )
        .expect("matching PersistOk must release");
    assert_eq!(sink.as_slice().len(), 1);
    assert_eq!(
        emitted_json(&sink.as_slice()[0])["event"],
        "im:post:received"
    );

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes::default()),
            },
            3,
            &mut sink,
        )
        .expect("duplicate PersistOk must be idempotent");
    assert!(sink.is_empty());
}

/// PersistErr 消费同一事务上下文，但绝不能释放 terminal event。
#[test]
fn persist_error_discards_terminal_event() {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: CONTRACT_USER.to_string(),
        company_id: CONTRACT_COMPANY.to_string(),
        ..Default::default()
    });
    let mut sink = EffectSink::new();
    let corr = module.queue_message_v3_commit(
        persisted_rows(),
        vec![post::received(json!({"id": "post-message-v3-contract"})).expect("typed post event")],
        &mut sink,
    );

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
            },
            1,
            &mut sink,
        )
        .expect("PersistErr must fail closed");
    assert!(sink.is_empty());
}
