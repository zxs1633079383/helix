//! 回归锚：hydration 中断必须返回关联失败，不能留到 renderer 的30秒兜底。
//! Command 为远程读；公开 Module::handle → HTTP/Persist/sync → read result。
//! 网络/持久化为回放端口，真实Go/UI N/A；时间注入，不使用墙钟等待。
use super::*;
use crate::{state::SyncTrigger, ImConfig};
use helix_core::effect::TransportId;
use helix_core::tick::{AppCommand, InboundBytes, ReplyBytes};
use helix_core::{Module, Tick};
use serde_json::{json, Value};
const CHANNEL: &str = "chfixx00000000000000000001";
const BODY: &str = "eyJzdGF0dXMiOiJTVUNDRVNTIiwiZGF0YSI6eyJpZCI6ImNoZml4eDAwMDAwMDAwMDAwMDAwMDAwMDAxIiwidGVhbUlkIjoiY29tcGFueS1hIiwiZGlzcGxheU5hbWUiOiJoeWRyYXRpb24iLCJ0eXBlIjoiUCIsImxhc3RFdmVudFNlcSI6MSwibmVlZFN5bmMiOnRydWUsIm1lbWJlckNvdW50IjoxLCJtZW1iZXJzIjpbeyJ1c2VySWQiOiJzY29wZS11c2VyIiwidGVhbUlkIjoiY29tcGFueS1hIiwicm9sZSI6Ik1FTUJFUiJ9XX19";

/// 公共入口逐跳回灌，每步观察独立Effect批次。
fn step(m: &mut ImModule, out: &mut EffectSink, tick: Tick) -> Result<(), helix_core::CoreError> {
    out.clear();
    m.handle(&tick, 100, out)
}
/// JSON端口回包，HTTP body采用driver的base64信封。
fn ok(v: Value) -> PortOutcome {
    PortOutcome::Ok(ReplyBytes(serde_json::to_vec(&v).unwrap().into()))
}
/// 从实际输出读取下一跳correlation，禁止手工指定生产Effect。
fn http(out: &EffectSink, suffix: &str) -> Correlation {
    out.as_slice()
        .iter()
        .find_map(|fx| match fx {
            Effect::Http { corr, req } if req.url.ends_with(suffix) => Some(*corr),
            _ => None,
        })
        .expect("HTTP effect")
}
/// 结果必须是唯一关联失败；否则Host receiver会一直等待renderer兜底。
fn assert_error(out: &EffectSink, req: &str) {
    let results: Vec<Value> = out
        .as_slice()
        .iter()
        .filter_map(|fx| match fx {
            Effect::Emit { event } => Some(serde_json::from_slice::<Value>(&event.0).unwrap()),
            _ => None,
        })
        .filter(|v| v["event"] == "im:read:result" && v["data"]["req_id"] == req)
        .collect();
    assert_eq!(
        results.len(),
        1,
        "missing unique terminal: hydration would wait until 30s timeout"
    );
    assert!(results[0]["data"]["error"].is_string());
    assert!(results[0]["data"].get("body").is_none());
}
/// 真实Command/HTTP/Persist已完成，hydration正等待sync终态。
fn pending_sync() -> (ImModule, EffectSink, Correlation) {
    pending_sync_with(None)
}

/// 预置已排队普通sync的调度态；后续Command/Persist必须真实合并到它。
fn pending_sync_with(queued: Option<SyncTrigger>) -> (ImModule, EffectSink, Correlation) {
    let mut m = ImModule::new(ImConfig {
        auth_user_id: "scope-user".into(),
        company_id: "company-a".into(),
        api_base_url: "http://fixture/api/cses".into(),
        ..Default::default()
    });
    let mut out = EffectSink::new();
    step(
        &mut m,
        &mut out,
        Tick::Command(AppCommand::new(
            "im_channel_load_increment_by_channel_id",
            serde_json::to_vec(&json!({"channel_id":CHANNEL,"req_id":"hydration-primary"}))
                .unwrap(),
        )),
    )
    .unwrap();
    let corr = http(&out, "/channel/load/incrementByChannelId");
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr,
            outcome: ok(json!({"status":200,"headers":[],"body":BODY})),
        },
    )
    .unwrap();
    let corr = out
        .as_slice()
        .iter()
        .find_map(|fx| match fx {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("durable barrier");
    if let Some(trigger) = queued {
        m.state
            .sync_scheduler
            .enqueue_with_trigger(ChannelId::from_str(CHANNEL).unwrap(), trigger);
    }
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr,
            outcome: ok(json!(null)),
        },
    )
    .unwrap();
    let sync = http(&out, "/channel/sync/notify");
    (m, out, sync)
}
/// 断线应立即失败，重复断线和迟到sync不得重复结算。
#[test]
fn hydration_terminal_disconnect_cancels_pending_sync() {
    let (mut m, mut out, sync) = pending_sync();
    step(
        &mut m,
        &mut out,
        Tick::Disconnected(TransportId::from_raw(1)),
    )
    .unwrap();
    assert_error(&out, "hydration-primary");
    assert!(m.state.hydration_req_ids.is_empty());
    assert_eq!(
        m.sync_inflight(),
        0,
        "disconnect must release abandoned sync windows"
    );
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr: sync,
            outcome: PortOutcome::Err(helix_core::tick::PortError::Network),
        },
    )
    .unwrap();
    assert!(out.as_slice().is_empty());
}
/// 新hello重置前必须结算旧hydration，即使Disconnected未先送达。
#[test]
fn hydration_terminal_new_hello_cancels_pending_sync() {
    let (mut m, mut out, sync) = pending_sync();
    step(
        &mut m,
        &mut out,
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({"action":"hello","data":{"connectionId":"replacement"}}))
                .unwrap()
                .into(),
        )),
    )
    .unwrap();
    assert_error(&out, "hydration-primary");
    assert!(m.state.hydration_req_ids.is_empty());
    step(&mut m,&mut out,Tick::PortReply{corr:sync,outcome:ok(json!({"status":200,"headers":[],"body":"eyJzdGF0dXMiOiJTVUNDRVNTIiwiZGF0YSI6eyJlbnRyaWVzIjpbeyJjaGFubmVsSWQiOiJjaGZpeHgwMDAwMDAwMDAwMDAwMDAwMDAwMSIsImtpbmQiOiJldmVudHMiLCJldmVudHMiOlt7ImNoYW5uZWxJZCI6ImNoZml4eDAwMDAwMDAwMDAwMDAwMDAwMDAxIiwiZXZlbnRTZXEiOjEsImV2ZW50VHlwZSI6MX1dLCJuZXh0U2VxIjoxLCJyZXNldFRvIjpudWxsfV0sInNlcnZlclRpbWUiOjEwMH19"}))}).unwrap();
    assert!(
        out.as_slice().is_empty(),
        "old sync receipt must not re-enter new session"
    );
}

/// 合并普通sync后，网络、超时和解析错误仍必须结算hydration，不受trigger限制。
#[test]
fn hydration_terminal_reused_sync_failure_releases_waiter() {
    for trigger in [SyncTrigger::Routine, SyncTrigger::PongGap] {
        for outcome in [
            PortOutcome::Err(helix_core::tick::PortError::Network),
            PortOutcome::Err(helix_core::tick::PortError::Timeout),
            ok(json!({"invalid":"sync"})),
        ] {
            let (mut m, mut out, sync) = pending_sync_with(Some(trigger));
            let result = step(
                &mut m,
                &mut out,
                Tick::PortReply {
                    corr: sync,
                    outcome,
                },
            );
            assert_error(&out, "hydration-primary");
            assert!(
                result.is_ok(),
                "handled hydration failure must reach EventSink"
            );
            assert!(m.state.hydration_req_ids.is_empty());
            assert_eq!(m.sync_inflight(), 0);
        }
    }
}

/// 从真实sync响应取得原子写correlation，写回执前不得完成hydration。
fn pending_history(queued: Option<SyncTrigger>) -> (ImModule, EffectSink, Correlation) {
    let (mut m, mut out, sync) = pending_sync_with(queued);
    step(&mut m,&mut out,Tick::PortReply {corr:sync,outcome:ok(json!({"status":200,"headers":[],"body":"eyJzdGF0dXMiOiJTVUNDRVNTIiwiZGF0YSI6eyJlbnRyaWVzIjpbeyJjaGFubmVsSWQiOiJjaGZpeHgwMDAwMDAwMDAwMDAwMDAwMDAwMSIsImtpbmQiOiJldmVudHMiLCJldmVudHMiOlt7ImNoYW5uZWxJZCI6ImNoZml4eDAwMDAwMDAwMDAwMDAwMDAwMDAxIiwiZXZlbnRTZXEiOjEsImV2ZW50VHlwZSI6MX1dLCJuZXh0U2VxIjoxLCJyZXNldFRvIjpudWxsfV0sInNlcnZlclRpbWUiOjEwMH19"}))}).unwrap();
    let corr = out
        .as_slice()
        .iter()
        .find_map(|fx| match fx {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("sync atomic commit");
    assert!(!out.as_slice().iter().any(|fx| matches!(fx,Effect::Emit{event} if serde_json::from_slice::<Value>(&event.0).unwrap()["event"]=="im:read:result")));
    (m, out, corr)
}
/// 普通sync的PersistErr也必须终止已加入的hydration。
#[test]
fn hydration_terminal_reused_sync_persist_failure_releases_waiter() {
    for trigger in [SyncTrigger::Routine, SyncTrigger::PongGap] {
        let (mut m, mut out, corr) = pending_history(Some(trigger));
        step(
            &mut m,
            &mut out,
            Tick::PortReply {
                corr,
                outcome: PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
            },
        )
        .unwrap();
        assert_error(&out, "hydration-primary");
        assert!(m.state.hydration_req_ids.is_empty());
    }
}
/// 对实际生成的persist上下文注入损坏category事件，公开回灌必须转换为失败终态。
/// 仅此故障注入访问内部状态；不伪造Command/HTTP/Persist输出，也不直接调用private handler。
#[test]
fn hydration_terminal_persist_processing_error_releases_waiter() {
    let (mut m, mut out, corr) = pending_history(None);
    let Some(CorrelationContext::ChannelPersist {
        pending_domain_events,
        has_category_posts,
        ..
    }) = m.state.corr_map.get_mut(&corr)
    else {
        panic!("actual sync persist context")
    };
    *has_category_posts = true;
    pending_domain_events.push(b"invalid-category-json".to_vec());
    let result = step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr,
            outcome: ok(json!(null)),
        },
    );
    assert_error(&out, "hydration-primary");
    assert!(
        result.is_ok(),
        "handled error must be delivered rather than discarded by engine"
    );
    assert!(m.state.hydration_req_ids.is_empty());
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr,
            outcome: ok(json!(null)),
        },
    )
    .unwrap();
    assert!(
        out.as_slice().is_empty(),
        "duplicate receipt cannot repeat terminal"
    );
}

/// 同频道第二个真实Command等待primary，失败要分别结算且不重复sync。
#[test]
fn hydration_terminal_failure_settles_primary_and_joined_waiter() {
    let (mut m, mut out, sync) = pending_sync_with(Some(SyncTrigger::Routine));
    step(
        &mut m,
        &mut out,
        Tick::Command(AppCommand::new(
            "im_channel_load_increment_by_channel_id",
            serde_json::to_vec(&json!({"channel_id":CHANNEL,"req_id":"hydration-waiter"})).unwrap(),
        )),
    )
    .unwrap();
    let corr = http(&out, "/channel/load/incrementByChannelId");
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr,
            outcome: ok(json!({"status":200,"headers":[],"body":BODY})),
        },
    )
    .unwrap();
    let corr = out
        .as_slice()
        .iter()
        .find_map(|fx| match fx {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .unwrap();
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr,
            outcome: ok(json!(null)),
        },
    )
    .unwrap();
    assert!(!out
        .as_slice()
        .iter()
        .any(|fx| matches!(fx, Effect::Http { .. })));
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr: sync,
            outcome: PortOutcome::Err(helix_core::tick::PortError::Network),
        },
    )
    .unwrap();
    assert_error(&out, "hydration-primary");
    assert_error(&out, "hydration-waiter");
    assert!(m.state.hydration_pending.is_empty());
    assert!(m.state.hydration_req_ids.is_empty());
    assert!(m.state.hydration_waiters.is_empty());
    assert!(m.state.hydration_ordered_rosters.is_empty());
    assert!(m.state.corr_map.is_empty());
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr: sync,
            outcome: PortOutcome::Err(helix_core::tick::PortError::Network),
        },
    )
    .unwrap();
    assert!(out.as_slice().is_empty());
}
/// 活跃连接的重复hello保持请求，断线后的同connectionId新会话仍必须取消。
#[test]
fn hydration_terminal_duplicate_hello_keeps_active_request() {
    let (mut m, mut out, _) = pending_sync();
    m.state.conn = crate::state::ConnState::Connected;
    m.state.connection_id = Some("same".into());
    step(
        &mut m,
        &mut out,
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({"action":"hello","data":{"connectionId":"same"}}))
                .unwrap()
                .into(),
        )),
    )
    .unwrap();
    assert!(out.as_slice().is_empty());
    assert_eq!(m.state.hydration_req_ids.len(), 1);
    step(
        &mut m,
        &mut out,
        Tick::Disconnected(TransportId::from_raw(1)),
    )
    .unwrap();
    assert_error(&out, "hydration-primary");
    step(
        &mut m,
        &mut out,
        Tick::Inbound(InboundBytes(
            serde_json::to_vec(&json!({"action":"hello","data":{"connectionId":"same"}}))
                .unwrap()
                .into(),
        )),
    )
    .unwrap();
    assert!(!out.as_slice().iter().any(|fx|matches!(fx,Effect::Emit{event} if serde_json::from_slice::<Value>(&event.0).unwrap()["event"]=="im:read:result")));
}
/// 断线只取消请求；已经发出的本地原子写仍可提交cursor，不再结算被取消的调用。
#[test]
fn hydration_terminal_disconnect_preserves_local_commit_receipt() {
    let (mut m, mut out, corr) = pending_history(None);
    step(
        &mut m,
        &mut out,
        Tick::Disconnected(TransportId::from_raw(1)),
    )
    .unwrap();
    assert_error(&out, "hydration-primary");
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr,
            outcome: ok(json!(null)),
        },
    )
    .unwrap();
    assert_eq!(
        m.state.channels[&ChannelId::from_str(CHANNEL).unwrap()]
            .cursor
            .value()
            .0,
        1
    );
    assert!(!out.as_slice().iter().any(|fx|matches!(fx,Effect::Emit{event} if serde_json::from_slice::<Value>(&event.0).unwrap()["event"]=="im:read:result")));
}

/// 复用普通sync收到关闭事件后，其原子写失败同样要结束hydration。
#[test]
fn hydration_terminal_reused_terminal_persist_failure_releases_waiter() {
    let (mut m, mut out, sync) = pending_sync_with(Some(SyncTrigger::Routine));
    step(&mut m,&mut out,Tick::PortReply{corr:sync,outcome:ok(json!({"status":200,"headers":[],"body":"eyJzdGF0dXMiOiJTVUNDRVNTIiwiZGF0YSI6eyJlbnRyaWVzIjpbeyJjaGFubmVsSWQiOiJjaGZpeHgwMDAwMDAwMDAwMDAwMDAwMDAwMSIsImtpbmQiOiJldmVudHMiLCJldmVudHMiOlt7ImNoYW5uZWxJZCI6ImNoZml4eDAwMDAwMDAwMDAwMDAwMDAwMDAxIiwiZXZlbnRTZXEiOjEsImV2ZW50VHlwZSI6NywiaWQiOiJjbG9zZWQtZXZlbnQtMSIsInBheWxvYWQiOnsic3RhdGUiOiJjbG9zZWQifX1dLCJuZXh0U2VxIjoxLCJyZXNldFRvIjpudWxsfV0sInNlcnZlclRpbWUiOjEwMH19"}))}).unwrap();
    let corr = out
        .as_slice()
        .iter()
        .find_map(|fx| match fx {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("terminal atomic commit");
    step(
        &mut m,
        &mut out,
        Tick::PortReply {
            corr,
            outcome: PortOutcome::Err(helix_core::tick::PortError::Storage(1)),
        },
    )
    .unwrap();
    assert_error(&out, "hydration-primary");
    assert!(m.state.hydration_req_ids.is_empty());
}
