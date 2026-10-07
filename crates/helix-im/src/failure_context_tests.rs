use super::*;
use crate::{module::ImConfig, ImModule};
use helix_core::tick::{AppCommand, InboundBytes, PortError};
use helix_core::Module;

/// 通过公开 Module seam 发出真实 HTTP；测试只观察私有诊断资源数量。
fn start(module: &mut ImModule, sink: &mut EffectSink, id: &str) -> Correlation {
    sink.clear();
    module
        .handle(
            &Tick::Command(AppCommand::new(
                "im_top20",
                serde_json::to_vec(&serde_json::json!({
                    "channel_id":"ch00000000000000000000000a","req_id":id
                }))
                .unwrap(),
            )),
            1000,
            sink,
        )
        .unwrap();
    sink.as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap()
}

/// 成功/失败/重复回复、取消和 stop 后不残留弱引用索引或路由。
#[test]
fn public_reply_and_stop_release_diagnostic_resources() {
    let mut module = ImModule::new(ImConfig {
        api_base_url: "https://example.test/api/cses".into(),
        ..ImConfig::default()
    });
    let mut sink = EffectSink::new();
    for i in 0..300 {
        let corr = start(&mut module, &mut sink, &format!("read-{i}"));
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Err(PortError::Timeout),
                },
                1001,
                &mut sink,
            )
            .unwrap();
        assert!(module.failure_context.pending.is_empty());
        assert!(module.failure_context.requests.is_empty());
        assert!(module.failure_context.request_routes.is_empty());
        assert!(module.failure_context.recent.len() <= 256);
        assert!(module.failure_context.recent_order.len() <= 256);
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Err(PortError::Timeout),
                },
                1002,
                &mut sink,
            )
            .unwrap();
        assert!(sink.is_empty());
    }
    assert_eq!(module.failure_context.recent.len(), 256);
    start(&mut module, &mut sink, "stop-me");
    sink.clear();
    module.on_stop(&mut sink).unwrap();
    assert!(module.failure_context.pending.is_empty());
    assert!(module.failure_context.timers.is_empty());
    assert!(module.failure_context.recent.is_empty());
    assert!(module.failure_context.requests.is_empty());
    assert!(module.failure_context.request_routes.is_empty());
}

/// 可信 envelope correlation 能标明来源；业务 data 中伪造 req_id 不得关联 Command。
#[test]
fn websocket_origin_uses_only_trusted_envelope_correlation() {
    let mut module = ImModule::new(ImConfig {
        api_base_url: "https://example.test/api/cses".into(),
        ..ImConfig::default()
    });
    let mut sink = EffectSink::new();
    start(&mut module, &mut sink, "trusted-request");
    for (frame,command) in [
        (br#"{"action":"unknown_test_action","tracing":{"csesTrackId":"trusted-request"},"data":{}}"#.as_slice(),"im_top20"),
        (br#"{"action":"unknown_test_action","data":{"req_id":"trusted-request"}}"#.as_slice(),"server_push")
    ] {
        sink.clear();
        let message=module.handle(&Tick::Inbound(InboundBytes(bytes::Bytes::copy_from_slice(frame))),1001,&mut sink).unwrap_err().to_string();
        assert!(message.contains(command)&&message.contains("WS unknown_test_action"),"{message}");
    }
    sink.clear();
    module.on_stop(&mut sink).unwrap();
}

/// 路径不带 authority/query/fragment；成功 WS 来源复用静态 action。
#[test]
fn paths_and_error_text_do_not_export_credentials() {
    assert_eq!(
        request_path("https://user:pass@example.test/api/cses/posts?token=hidden#x"),
        "/api/cses/posts"
    );
    assert_eq!(
        clean_text("denied\nBearer private token=private password=private", 128),
        "denied [redacted] [redacted] [redacted] [redacted]"
    );
    assert_eq!(clean_text(&"甲".repeat(2000), 32).chars().count(), 32);
    let mut context = FailureContext::default();
    context.websocket(&crate::ws::WsFrame::parse(br#"{"action":"post","data":{}}"#).unwrap());
    assert!(matches!(
        context.current.ws_action,
        Some(Cow::Borrowed("post"))
    ));
}

/// 与别的请求交错的 timer 取消也必须释放它自己的诊断索引。
#[test]
fn timer_cancel_releases_origin_without_scanning_other_requests() {
    let mut context = FailureContext::default();
    let command = Tick::Command(AppCommand::new(
        "im_top20",
        br#"{"req_id":"timer-only"}"#.to_vec(),
    ));
    let mut sink = EffectSink::new();
    let timer = TimerId::from_raw(42);
    context.enter(&command);
    sink.push(Effect::ScheduleTimer {
        id: timer,
        after_ms: 10,
    });
    context
        .finish(Some(&command), 0, &mut sink, Ok(()))
        .unwrap();
    assert_eq!(context.requests.len(), 1);
    sink.clear();
    let other = Tick::Connected(helix_core::effect::TransportId::from_raw(1));
    context.enter(&other);
    sink.push(Effect::CancelTimer { id: timer });
    context.finish(Some(&other), 0, &mut sink, Ok(())).unwrap();
    assert!(context.timers.is_empty());
    assert!(context.requests.is_empty());
}
