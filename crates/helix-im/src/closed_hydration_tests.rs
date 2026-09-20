//! 关闭群补水：公开 Command/PortReply 回放，断言终态、公司闸门与内部等待清理。
use super::*;
use crate::ImConfig;
use helix_core::tick::{AppCommand, PortError, ReplyBytes};
use helix_core::{Module, Tick};
use serde_json::{json, Value};

const CHANNEL: &str = "chfixx00000000000000000001";
const CLOSED_AT: i64 = 1789897616720;
// 现场结构：SUCCESS、deleteAt>0、lastEventSeq=7、完整 roster，故意缺 needSync。
const CLOSED_BODY: &str = "eyJzdGF0dXMiOiJTVUNDRVNTIiwiZGF0YSI6eyJpZCI6ImNoZml4eDAwMDAwMDAwMDAwMDAwMDAwMDAxIiwidGVhbUlkIjoiY29tcGFueS1hIiwiZGlzcGxheU5hbWUiOiJjbG9zZWQiLCJkZWxldGVBdCI6MTc4OTg5NzYxNjcyMCwibGFzdEV2ZW50U2VxIjo3LCJ1bnJlYWRDb3VudCI6MCwibWVtYmVyQ291bnQiOjEsIm1lbWJlcnMiOlt7InVzZXJJZCI6InNjb3BlLXVzZXIiLCJyb2xlIjoiTUVNQkVSIn1dfX0=";

/// 每步只经 Module 入口，并禁止关闭响应后的再次网络调用或计时器等待。
fn reply(module: &mut ImModule, sink: &mut EffectSink, corr: Correlation, outcome: PortOutcome) {
    sink.clear();
    module
        .handle(&Tick::PortReply { corr, outcome }, 100, sink)
        .unwrap();
    assert!(sink.as_slice().iter().all(|effect| !matches!(
        effect,
        Effect::Http { .. }
            | Effect::HttpFire { .. }
            | Effect::Send { .. }
            | Effect::ScheduleTimer { .. }
    )));
}

fn ok(value: Value) -> PortOutcome {
    PortOutcome::Ok(ReplyBytes(serde_json::to_vec(&value).unwrap().into()))
}

/// 发起真实补水命令，再回灌脱敏现场 HTTP 信封；保持 Persist 未兑现。
fn start(company: &str, cursor: u64) -> (ImModule, EffectSink, Correlation) {
    let mut module = ImModule::new(ImConfig {
        auth_user_id: "scope-user".into(),
        company_id: company.into(),
        api_base_url: "http://closed.invalid/api/cses".into(),
        ..Default::default()
    });
    module.register_channel(ChannelId::from_str(CHANNEL).unwrap(), cursor);
    let mut sink = EffectSink::new();
    module
        .handle(
            &Tick::Command(AppCommand::new(
                "im_channel_load_increment_by_channel_id",
                serde_json::to_vec(&json!({"channel_id":CHANNEL,"req_id":"closed-read"})).unwrap(),
            )),
            100,
            &mut sink,
        )
        .unwrap();
    let http = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } => {
                assert!(req.url.ends_with("/channel/load/incrementByChannelId"));
                Some(*corr)
            }
            _ => None,
        })
        .expect("hydration HTTP");
    reply(
        &mut module,
        &mut sink,
        http,
        ok(json!({"status":200,"headers":[],"body":CLOSED_BODY})),
    );
    assert!(
        events(&sink).is_empty(),
        "must not emit before durable barrier"
    );
    let persist = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("channel/member atomic barrier");
    (module, sink, persist)
}

fn events(sink: &EffectSink) -> Vec<Value> {
    sink.as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(serde_json::from_slice(event.0.as_ref()).unwrap()),
            _ => None,
        })
        .collect()
}

/// 读回 fixture 来自持久表行，而不是直接用 HTTP 投影冒充结果。
fn readback(sink: &EffectSink, table: &str, cursor: u64) -> (Correlation, PortOutcome) {
    let corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(|op| match op {
                    StorageOp::Get(spec) => spec.table == table,
                    StorageOp::Scan(spec) => spec.table == table,
                    _ => false,
                }) =>
            {
                Some(*corr)
            }
            _ => None,
        })
        .unwrap_or_else(|| {
            panic!(
                "missing {table} readback after closed hydration persist: {:?}",
                sink.as_slice()
            )
        });
    let rows = match table {
        "channel" => json!([{"id":CHANNEL,"team_id":"company-a","user_id":"scope-user",
            "display_name":"durable closed","is_active":0,"delete_at":CLOSED_AT,"members":"[]"}]),
        "channel_member" => json!([{"channel_id":CHANNEL,"user_id":"scope-user",
            "team_id":"company-a","role":"MEMBER","unread_count":0}]),
        "message" => json!([]),
        "channel_event_cursor" => json!([{"channel_id":CHANNEL,"last_event_seq":cursor}]),
        _ => panic!("unexpected read table"),
    };
    (corr, ok(rows))
}

/// 除唯一终态外，还要证明全部补水等待和 correlation 已释放。
fn assert_clean(module: &ImModule) {
    let state = &module.state;
    assert!(state.hydration_pending.is_empty());
    assert!(state.hydration_req_ids.is_empty());
    assert!(state.hydration_emit_channel_increment.is_empty());
    assert!(state.hydration_ordered_rosters.is_empty());
    assert!(state.hydration_authority_unreads.is_empty());
    assert!(state.corr_map.is_empty());
    assert_eq!(module.sync_inflight(), 0);
    assert_eq!(module.sync_pending_len(), 0);
}

#[test]
fn closed_hydration_reads_archive_without_sync_or_cursor_forgery() {
    for cursor in [0, 7] {
        let (mut module, mut sink, persist) = start("company-a", cursor);
        reply(&mut module, &mut sink, persist, ok(json!(null)));
        for table in [
            "channel",
            "channel_member",
            "message",
            "channel_event_cursor",
        ] {
            assert!(events(&sink).is_empty());
            let (corr, outcome) = readback(&sink, table, cursor);
            reply(&mut module, &mut sink, corr, outcome);
        }
        let emitted = events(&sink);
        let results: Vec<_> = emitted
            .iter()
            .filter(|e| e["event"] == "im:read:result")
            .collect();
        assert_eq!(results.len(), 1);
        let data = &results[0]["data"];
        assert_eq!(data["req_id"], "closed-read");
        assert_eq!(data["body"]["completion"], "hydrated");
        assert_eq!(data["body"]["channel"]["deleteAt"], CLOSED_AT);
        assert_eq!(data["body"]["channel"]["displayName"], "durable closed");
        assert_eq!(data["body"]["cursor"], cursor);
        let id = ChannelId::from_str(CHANNEL).unwrap();
        assert_eq!(module.cursor_for(id), Some(cursor));
        assert_eq!(module.terminal_event_seq_for(id), Some(7));
        assert_clean(&module);
        reply(&mut module, &mut sink, persist, ok(json!(null)));
        assert!(
            sink.is_empty(),
            "late persist must not emit twice or restart work"
        );
        assert_clean(&module);
    }
}

#[test]
fn closed_hydration_foreign_company_finishes_without_render_body() {
    let (mut module, mut sink, persist) = start("company-b", 7);
    reply(&mut module, &mut sink, persist, ok(json!(null)));
    for table in [
        "channel",
        "channel_member",
        "message",
        "channel_event_cursor",
    ] {
        let (corr, outcome) = readback(&sink, table, 7);
        reply(&mut module, &mut sink, corr, outcome);
    }
    let emitted = events(&sink);
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0]["event"], "im:read:result");
    assert_eq!(emitted[0]["data"]["req_id"], "closed-read");
    assert_eq!(emitted[0]["data"]["error"], "RENDER_SCOPE_MISMATCH");
    assert!(emitted[0]["data"].get("body").is_none());
    assert_clean(&module);
}

#[test]
fn closed_hydration_each_storage_failure_cleans_waiters_once() {
    for failed_stage in 0..5 {
        let (mut module, mut sink, mut corr) = start("company-a", 7);
        for stage in 0..failed_stage {
            let outcome = if stage == 0 {
                ok(json!(null))
            } else {
                readback(
                    &sink,
                    [
                        "channel",
                        "channel_member",
                        "message",
                        "channel_event_cursor",
                    ][stage - 1],
                    7,
                )
                .1
            };
            reply(&mut module, &mut sink, corr, outcome);
            corr = readback(
                &sink,
                [
                    "channel",
                    "channel_member",
                    "message",
                    "channel_event_cursor",
                ][stage],
                7,
            )
            .0;
        }
        reply(
            &mut module,
            &mut sink,
            corr,
            PortOutcome::Err(PortError::Timeout),
        );
        let emitted = events(&sink);
        assert_eq!(emitted.len(), 1, "stage {failed_stage}");
        assert_eq!(emitted[0]["event"], "im:read:result");
        assert!(emitted[0]["data"]["error"].is_string());
        assert_clean(&module);
        reply(&mut module, &mut sink, corr, ok(json!([])));
        assert!(sink.is_empty());
    }
}
