//! 本地公开seam回放与可选真实OTLP验收；不访问真实业务数据。
use bytes::Bytes;
use helix_core::{
    effect::{Effect, HttpResponse, Row, SqlValue, StorageOp, UpsertSpec},
    tick::{AppCommand, InboundBytes, PortError, PortOutcome, ReplyBytes},
    EffectSink, ExecutionShell, Module, Tick,
};
use helix_driver_host::{
    lifecycle::LifecycleContext,
    otel::{HostOtelConfig, HostOtelRuntime},
    trace::{TraceCarrier, TraceHooks},
};
use helix_im::{module::ImConfig, ImModule};
use serde_json::{json, Value};

const CHANNEL: &str = "ch00000000000000000000000a";

/// 注入确定性已提交频道归属，避免测试被scope校验替代原业务失败。
fn shell() -> ExecutionShell {
    let mut module = ImModule::new(ImConfig {
        api_base_url: "https://example.test/api/cses".into(),
        auth_user_id: "viewer".into(),
        company_id: "company".into(),
        ..Default::default()
    });
    let row: Row = vec![
        ("id".into(), SqlValue::Text(CHANNEL.into())),
        ("team_id".into(), SqlValue::Text("company".into())),
        ("user_id".into(), SqlValue::Text("viewer".into())),
    ];
    let mut sink = EffectSink::new();
    let corr = module.queue_message_v3_commit(
        vec![StorageOp::BatchUpsert(UpsertSpec::new(
            "channel",
            vec![row],
            Some("id"),
        ))],
        vec![],
        &mut sink,
    );
    module
        .handle(
            &Tick::PortReply {
                corr,
                outcome: PortOutcome::Ok(ReplyBytes(Bytes::new())),
            },
            0,
            &mut sink,
        )
        .unwrap();
    let mut shell = ExecutionShell::new();
    shell.register(module);
    shell
}

fn runtime() -> HostOtelRuntime {
    HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        endpoint: "noop".into(),
        protocol: "noop".into(),
        ..Default::default()
    })
}

fn command(name: &'static str, payload: Value) -> Tick {
    Tick::Command(AppCommand::new(name, serde_json::to_vec(&payload).unwrap()))
}

fn outcome(body: &'static [u8]) -> PortOutcome {
    helix_core::port_codec::http_result_to_outcome(Ok(HttpResponse {
        status: 200,
        headers: vec![],
        body: Bytes::from_static(body),
    }))
}

fn event(effects: &[Effect]) -> Value {
    effects
        .iter()
        .find_map(|e| {
            if let Effect::Emit { event } = e {
                Some(serde_json::from_slice(&event.0).unwrap())
            } else {
                None
            }
        })
        .unwrap()
}

/// command→HTTP回包继承同一Trace；失败信息同时进入客户端和当前root。
fn hydration(runtime: &HostOtelRuntime, outcome: PortOutcome) -> (String, Value) {
    let hooks = TraceHooks::noop().with_otel(runtime.clone());
    let mut shell = shell();
    let tick = command(
        "im_channel_load_increment_by_channel_id",
        json!({"channel_id":CHANNEL,"req_id":"otel-read"}),
    );
    let (mut scope, context) =
        hooks.start_tick_with_context(&tick, &LifecycleContext::new(1, None, None));
    let corr = scope
        .step(&mut shell, tick, 1000)
        .unwrap()
        .iter()
        .find_map(|e| {
            if let Effect::Http { corr, .. } = e {
                Some(*corr)
            } else {
                None
            }
        })
        .unwrap();
    drop(scope);
    let reply = Tick::PortReply { corr, outcome };
    let (mut scope, _) = hooks.start_tick_with_context(&reply, &context);
    let trace_id = context.otel_parent().unwrap().traceparent.as_ref().unwrap()[3..35].to_owned();
    let result = event(scope.step(&mut shell, reply, 1001).unwrap());
    assert!(result.to_string().contains(&trace_id));
    assert!(!result.to_string().contains("RENDER_SCOPE_MISMATCH"));
    drop(scope);
    (trace_id, result)
}

#[test]
fn im_failure_carries_current_trace_id() {
    let runtime = runtime();
    let (id, result) = hydration(&runtime, PortOutcome::Err(PortError::Timeout));
    assert!(result["data"]["error"]
        .as_str()
        .unwrap()
        .contains("Timeout"));
    let snapshot = runtime.last_completed_span_for_test().unwrap();
    assert_eq!(snapshot.trace_id.as_deref(), Some(id.as_str()));
    assert!(snapshot.attributes.iter().any(|(k, _)| k == "error.type"));
    assert_eq!(snapshot.failures.len(), 1);
    assert_eq!(
        snapshot.failures[0].message(),
        result["data"]["error"].as_str().unwrap()
    );
}

#[test]
fn http_200_business_failure_marks_span_error() {
    let runtime = runtime();
    let (_, result) = hydration(
        &runtime,
        outcome(br#"{"status":"failed","message":"ent: channel_member not found"}"#),
    );
    assert!(result["data"]["error"]
        .as_str()
        .unwrap()
        .contains("ent: channel_member not found"));
    assert_eq!(
        runtime
            .last_completed_span_for_test()
            .unwrap()
            .failures
            .len(),
        1
    );
}

/// 不同WS carrier不能串TraceId；CoreError保留来源，成功轮清空诊断。
#[test]
fn ws_core_error_and_next_success_do_not_leak_context() {
    let runtime = runtime();
    let hooks = TraceHooks::noop().with_otel(runtime.clone());
    let mut shell = shell();
    for id in [
        "11111111111111111111111111111111",
        "22222222222222222222222222222222",
    ] {
        let carrier = TraceCarrier::from_json_str(&format!(
            r#"{{"traceparent":"00-{id}-123456789abcdef0-01"}}"#
        ))
        .unwrap();
        let tick = Tick::Inbound(InboundBytes(Bytes::from_static(
            br#"{"action":"future_unknown_action","data":{}}"#,
        )));
        let (mut scope, _) =
            hooks.start_tick_with_context(&tick, &LifecycleContext::new(1, None, Some(carrier)));
        let error = scope.step(&mut shell, tick, 1).unwrap_err().to_string();
        assert!(error.contains(id));
        assert!(error.contains("WS future_unknown_action"));
        drop(scope);
        assert_eq!(
            runtime
                .last_completed_span_for_test()
                .unwrap()
                .failures
                .len(),
            1
        );
    }
    let tick = Tick::Inbound(InboundBytes(Bytes::from_static(b"not-json")));
    let (mut scope, _) =
        hooks.start_tick_with_context(&tick, &LifecycleContext::new(3, None, None));
    scope.step(&mut shell, tick, 2).unwrap();
    assert!(shell.diagnostics().failures().is_empty());
    drop(scope);
    assert!(runtime
        .last_completed_span_for_test()
        .unwrap()
        .failures
        .is_empty());
}

/// 无OTel时不伪造TraceId，不收集无消费者的诊断。
#[test]
fn disabled_otel_preserves_existing_error() {
    let mut shell = shell();
    let tick = Tick::Inbound(InboundBytes(Bytes::from_static(
        br#"{"action":"future_unknown_action","data":{}}"#,
    )));
    let (mut scope, _) =
        TraceHooks::noop().start_tick_with_context(&tick, &LifecycleContext::new(1, None, None));
    let error = scope.step(&mut shell, tick, 1).unwrap_err().to_string();
    assert!(!error.contains("trace_id="));
    assert!(shell.diagnostics().failures().is_empty());
}

/// 显式运行才向指定Collector发送合成故障；打印ID供Jaeger同ID对账。
#[test]
#[ignore = "requires explicitly selected OTLP collector; synthetic traffic only"]
fn live_otlp_business_failure() {
    let runtime = HostOtelRuntime::new(HostOtelConfig {
        enabled: true,
        endpoint: std::env::var("HELIX_ERROR_TEST_OTLP_ENDPOINT").expect("explicit collector"),
        protocol: "grpc".into(),
        service_name: "helix-error-verification".into(),
        deployment_environment: "test".into(),
    });
    assert!(runtime.is_exporter_ready());
    let (id, result) = hydration(
        &runtime,
        outcome(br#"{"status":"failed","message":"ent: channel_member not found"}"#),
    );
    println!("TRACE_ID={id}\nCLIENT_ERROR={}", result["data"]["error"]);
    // 给现有后台batch exporter完成一次发送；网络是否成功由Jaeger回查判定。
    std::thread::sleep(std::time::Duration::from_secs(2));
}

/// 连续业务失败通过生产step桥接保留Error，并把省略数量交给当前span。
#[test]
fn error_burst_preserves_status_and_propagates_dropped_count() {
    struct Burst;
    impl Module for Burst {
        fn name(&self) -> &'static str {
            "burst"
        }
        fn accepts(&self, _: &Tick) -> bool {
            true
        }
        fn handle(
            &mut self,
            _: &Tick,
            _: u64,
            out: &mut EffectSink,
        ) -> Result<(), helix_core::CoreError> {
            let text = "x".repeat(1024);
            for _ in 0..100 {
                out.diagnostics_mut().report("burst.failed", &text);
            }
            Ok(())
        }
    }
    let runtime = runtime();
    let hooks = TraceHooks::noop().with_otel(runtime.clone());
    let mut shell = ExecutionShell::new();
    shell.register(Burst);
    let tick = command("burst", json!({}));
    let (mut scope, _) =
        hooks.start_tick_with_context(&tick, &LifecycleContext::new(1, None, None));
    scope.step(&mut shell, tick, 1).unwrap();
    drop(scope);
    let snapshot = runtime.last_completed_span_for_test().unwrap();
    assert!(snapshot
        .attributes
        .iter()
        .any(|(key, value)| key == "error.type" && value == "burst.failed"));
    assert_eq!(snapshot.failures.len(), 4);
    assert_eq!(snapshot.failures.dropped(), 96);
    assert_eq!(
        snapshot
            .failures
            .iter()
            .map(|failure| failure.message().len())
            .sum::<usize>(),
        4096
    );
}
