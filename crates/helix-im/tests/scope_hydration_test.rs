//! 未知频道首个在线 post 的本地 scope hydration 回放。
//!
//! 回归锚：PersistOk 只证明消息事实落库；频道归属未知时必须由 Helix 主动补水，
//! render scope 仍保持 fail-closed，且同频道在途补水只能有一个。

mod common;

use bytes::Bytes;
use common::*;
use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::{InboundBytes, PortError, PortOutcome, Tick};
use helix_core::{effect::TransportId, Correlation};
use serde_json::{json, Value};

fn post_frame(channel_num: u64, event_seq: u64) -> Tick {
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "channelId": cid(channel_num),
            "event_seq": event_seq,
            "id": sid(event_seq),
            "temporaryId": format!("scope-hydration-{channel_num}-{event_seq}"),
            "message": "scope hydration regression",
            "type": "TEXT",
            "createBy": "5ffd465eaaaaaaaaaaaaaaaaaa",
            "props": { "channel_event_seq": event_seq }
        },
        "broadcast": {},
        "seq": 261
    });
    Tick::Inbound(InboundBytes(
        serde_json::to_vec(&payload)
            .expect("post fixture must serialize")
            .into(),
    ))
}

fn persist_atomic_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("post must enter one atomic persist")
}

fn hydration_http_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req }
                if req.url.ends_with("/channel/load/incrementByChannelId") =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("unknown scope must start one channel hydration HTTP")
}

fn hydration_http_count(harness: &Harness) -> usize {
    harness
        .sink
        .as_slice()
        .iter()
        .filter(|effect| {
            matches!(
                effect,
                Effect::Http { req, .. }
                    if req.url.ends_with("/channel/load/incrementByChannelId")
            )
        })
        .count()
}

fn increment_success(channel_num: u64, viewer: &str, company: &str) -> Bytes {
    enveloped(Bytes::from(
        serde_json::to_vec(&json!({
            "data": {
                "id": cid(channel_num),
                "teamId": company,
                "displayName": "scope-hydration",
                "type": "P",
                "lastEventSeq": 0,
                "needSync": false,
                "memberCount": 1,
                "members": [{ "userId": viewer, "teamId": company, "role": "MEMBER" }]
            }
        }))
        .expect("increment fixture must serialize"),
    ))
}

fn increment_persist_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "channel"),
                ) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("increment reply must queue channel persist")
}

fn readback_corr(harness: &Harness, table: &str) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| {
            let Effect::Persist { corr, ops } = effect else {
                return None;
            };
            ops.iter().find_map(|op| {
                let op_table = match op {
                    StorageOp::Get(spec) => spec.table,
                    StorageOp::Scan(spec) => spec.table,
                    _ => return None,
                };
                (op_table == table).then_some(corr.raw())
            })
        })
        .expect("hydration readback persist must exist")
}

fn rows_reply(corr: u64, rows: Value) -> Tick {
    port_reply_with_body(
        corr,
        Bytes::from(serde_json::to_vec(&rows).expect("rows fixture must serialize")),
    )
}

/// 回放 no-sync hydration 的 channel/member/message/cursor 四段 durable readback。
fn complete_no_sync_hydration(
    harness: &mut Harness,
    channel_num: u64,
    viewer: &str,
    company: &str,
    now_ms: u64,
) -> Vec<Value> {
    let hydration_persist = increment_persist_corr(harness);
    harness.step(port_reply_ok(hydration_persist), now_ms);

    let channel_readback = readback_corr(harness, "channel");
    harness.step(
        rows_reply(
            channel_readback,
            json!([{
                "id": cid(channel_num),
                "display_name": "scope-hydration",
                "team_id": company,
                "user_id": viewer,
                "members": "[]",
                "unread_count": 0,
            }]),
        ),
        now_ms + 1,
    );

    let member_readback = readback_corr(harness, "channel_member");
    harness.step(
        rows_reply(
            member_readback,
            json!([{
                "channel_id": cid(channel_num),
                "user_id": viewer,
                "team_id": company,
                "role": "MEMBER",
                "unread_count": 0,
            }]),
        ),
        now_ms + 2,
    );

    let message_readback = readback_corr(harness, "message");
    harness.step(rows_reply(message_readback, json!([])), now_ms + 3);

    let cursor_readback = readback_corr(harness, "channel_event_cursor");
    harness.step(
        rows_reply(
            cursor_readback,
            json!([{
                "channel_id": cid(channel_num),
                "last_event_seq": 0,
            }]),
        ),
        now_ms + 4,
    );

    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

fn timeout_reply(corr: u64) -> Tick {
    Tick::PortReply {
        corr: Correlation::from_raw(corr),
        outcome: PortOutcome::Err(PortError::Timeout),
    }
}

#[test]
fn unknown_scope_hydration_deduplicates_same_channel_until_terminal() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    assert!(!first
        .iter()
        .any(|effect| matches!(effect, EffectKind::Emit)));
    harness.step(port_reply_ok(first_persist), 1_001);
    let _first_http = hydration_http_corr(&harness);

    let second = harness.step(post_frame(7, 2), 1_002);
    let second_persist = persist_atomic_corr(&harness);
    assert!(!second
        .iter()
        .any(|effect| matches!(effect, EffectKind::Emit)));
    harness.step(port_reply_ok(second_persist), 1_003);
    assert_eq!(
        hydration_http_count(&harness),
        0,
        "同频道在途补水不得重复发 HTTP"
    );
}

#[test]
fn scope_hydration_http_failure_clears_inflight_for_retry() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let first_http = hydration_http_corr(&harness);
    harness.step(port_reply_err(first_http), 1_002);

    let second = harness.step(post_frame(7, 2), 1_003);
    let second_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(second_persist), 1_004);
    assert_eq!(
        hydration_http_count(&harness),
        1,
        "HTTP 失败后下一条 post 必须可重试补水"
    );
}

#[test]
fn scope_hydration_timeout_clears_inflight_for_retry() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let first_http = hydration_http_corr(&harness);
    harness.step(timeout_reply(first_http), 1_002);

    let second = harness.step(post_frame(7, 2), 1_003);
    let second_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(second_persist), 1_004);
    assert_eq!(
        hydration_http_count(&harness),
        1,
        "HTTP timeout 后必须清理并允许重试补水"
    );
}

#[test]
fn scope_hydration_business_failure_clears_inflight_without_ui_read_result() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let first_http = hydration_http_corr(&harness);
    let failed_body = enveloped(Bytes::from(
        serde_json::to_vec(&json!({
            "status": "FAIL",
            "message": "not a current member"
        }))
        .expect("business failure fixture must serialize"),
    ));
    harness.step(port_reply_with_body(first_http, failed_body), 1_002);
    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit { .. })),
        "内部补水业务失败不得把 req_id 或 raw body 暴露成 UI read result"
    );

    let second = harness.step(post_frame(7, 2), 1_003);
    let second_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(second_persist), 1_004);
    assert_eq!(
        hydration_http_count(&harness),
        1,
        "业务失败后必须清理并允许重试补水"
    );
}

#[test]
fn scope_hydration_persist_failure_clears_inflight_for_retry() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let first_http = hydration_http_corr(&harness);
    harness.step(
        port_reply_with_body(
            first_http,
            increment_success(7, "scope-viewer", "scope-company"),
        ),
        1_002,
    );
    let hydration_persist = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } | Effect::PersistAtomic { corr, ops }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "channel"),
                ) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("increment reply must queue channel persist");
    harness.step(port_reply_err(hydration_persist), 1_003);

    let second = harness.step(post_frame(7, 2), 1_004);
    let second_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(second_persist), 1_005);
    assert_eq!(
        hydration_http_count(&harness),
        1,
        "Persist 失败后必须清理并允许重试补水"
    );
}

#[test]
fn known_external_channel_does_not_trigger_scope_hydration() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");
    harness.seed_channel_scope(&cid(7), "other-company", "scope-viewer");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    assert_eq!(
        hydration_http_count(&harness),
        0,
        "已有外公司归属不得重复补水"
    );
}

#[test]
fn wrong_owner_channel_triggers_scope_hydration() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");
    harness.seed_channel_scope(&cid(7), "other-company", "other-viewer");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    assert_eq!(
        hydration_http_count(&harness),
        1,
        "错误 owner 的频道必须重新补水"
    );
}

#[test]
fn auto_hydration_current_scope_emits_increment_after_terminal_readback() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let first_http = hydration_http_corr(&harness);
    harness.step(
        port_reply_with_body(
            first_http,
            increment_success(7, "scope-viewer", "scope-company"),
        ),
        1_002,
    );

    let events =
        complete_no_sync_hydration(&mut harness, 7, "scope-viewer", "scope-company", 1_003);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "im:channel:increment")
            .count(),
        1,
        "当前 user/company 的自动补水终态必须发布真实频道投影"
    );
    assert_eq!(events.len(), 1, "内部补水 read result 不得广播给 UI");
    assert_eq!(events[0]["event"], "im:channel:increment");
    assert_eq!(events[0]["data"]["id"], cid(7));
    assert_eq!(events[0]["data"]["teamId"], "scope-company");
}

#[test]
fn auto_hydration_external_scope_never_emits_increment() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");
    harness.seed_channel_scope(&cid(7), "other-company", "other-viewer");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let first_http = hydration_http_corr(&harness);
    harness.step(
        port_reply_with_body(
            first_http,
            increment_success(7, "scope-viewer", "other-company"),
        ),
        1_002,
    );

    let events =
        complete_no_sync_hydration(&mut harness, 7, "scope-viewer", "other-company", 1_003);
    assert!(events.is_empty(), "外公司频道补水不得向 UI 广播任何事件");
}

#[test]
fn scope_hydration_disconnect_clears_inflight_for_retry() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let old_http = hydration_http_corr(&harness);
    harness.step(Tick::Disconnected(TransportId::from_raw(0)), 1_002);
    // Late reply is ignored after disconnect; the next post must create a fresh request.
    harness.step(port_reply_err(old_http), 1_003);
    harness.step(Tick::Connected(TransportId::from_raw(0)), 1_004);
    harness.step(hello_inbound("scope-reconnect"), 1_005);

    let second = harness.step(post_frame(7, 2), 1_006);
    let second_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(second_persist), 1_007);
    assert_eq!(
        hydration_http_count(&harness),
        1,
        "断连后必须释放旧补水并允许重试"
    );
}

#[test]
fn identity_switch_drops_old_scope_hydration_and_allows_new_session_retry() {
    let mut harness =
        Harness::new_with_identity(&[], "scope-viewer-a", "scope-company-a", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let old_http = hydration_http_corr(&harness);

    harness.step(
        command_json(
            helix_im::module::RUNTIME_IDENTITY_COMMAND,
            json!({ "auth_user_id": "scope-viewer-b", "company_id": "scope-company-b" }),
        ),
        1_002,
    );
    harness.step(port_reply_err(old_http), 1_003);

    let second = harness.step(post_frame(7, 1), 1_004);
    let second_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(second_persist), 1_005);
    assert_eq!(
        hydration_http_count(&harness),
        1,
        "新身份必须重新建立自己的补水生命周期"
    );
}

/// 个人身份保留跨公司频道补水、持久屏障及唯一终态。
#[test]
fn personal_account_hydrates_company_channel_after_terminal_readback() {
    let mut harness = Harness::new_with_identity(&[], "scope-viewer", "", "", "", "");

    let first = harness.step(post_frame(7, 1), 1_000);
    let first_persist = persist_atomic_corr(&harness);
    harness.step(port_reply_ok(first_persist), 1_001);
    let first_http = hydration_http_corr(&harness);
    harness.step(
        port_reply_with_body(
            first_http,
            increment_success(7, "scope-viewer", "scope-company"),
        ),
        1_002,
    );

    let events =
        complete_no_sync_hydration(&mut harness, 7, "scope-viewer", "scope-company", 1_003);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["event"] == "im:channel:increment")
            .count(),
        1,
        "当前 user/company 的自动补水终态必须发布真实频道投影"
    );
    assert_eq!(events.len(), 1, "内部补水 read result 不得广播给 UI");
    assert_eq!(events[0]["event"], "im:channel:increment");
    assert_eq!(events[0]["data"]["id"], cid(7));
    assert_eq!(events[0]["data"]["teamId"], "scope-company");
}

/// TooLong恢复失败也清理自动补水owner，不能泄漏内部read error或永久阻断后续补水。
#[test]
fn auto_hydration_too_long_reload_failure_releases_owner_for_retry() {
    let mut h = Harness::new_with_identity(&[], "scope-viewer", "scope-company", "", "", "");
    h.step(post_frame(7, 1), 1000);
    let first = persist_atomic_corr(&h);
    h.step(port_reply_ok(first), 1001);
    let increment = hydration_http_corr(&h);
    let body = enveloped(Bytes::from(
        serde_json::to_vec(&json!({"data":{
            "id":cid(7),"teamId":"scope-company","displayName":"recovery","type":"P",
            "lastEventSeq":6206,"needSync":true,"memberCount":1,
            "members":[{"userId":"scope-viewer","role":"MEMBER"}]
        }}))
        .unwrap(),
    ));
    h.step(port_reply_with_body(increment, body), 1002);
    let persist = persist_atomic_corr(&h);
    h.step(port_reply_ok(persist), 1003);
    let sync = h
        .sink
        .as_slice()
        .iter()
        .find_map(|fx| match fx {
            Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .unwrap();
    let body = enveloped(Bytes::from(
        serde_json::to_vec(&json!({"status":"SUCCESS","data":{"entries":[{
            "channelId":cid(7),"kind":"too_long","resetTo":6206,"events":[]
        }]}}))
        .unwrap(),
    ));
    h.step(port_reply_with_body(sync, body), 1004);
    let reload = h
        .sink
        .as_slice()
        .iter()
        .find_map(|fx| match fx {
            Effect::Http { corr, req } if req.url.ends_with("/posts/getLatestPost") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("automatic hydration reload");
    h.step(port_reply_err(reload), 1005);
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .any(|fx| matches!(fx, Effect::Emit { .. })),
        "内部恢复失败不发布UI read result"
    );
    h.seed_channel_scope(&cid(7), "other-company", "other-viewer");
    h.step(post_frame(7, 2), 1006);
    let persist = persist_atomic_corr(&h);
    h.step(port_reply_ok(persist), 1007);
    assert_eq!(
        hydration_http_count(&h),
        1,
        "恢复失败必须释放旧auto owner并允许新补水"
    );
}
