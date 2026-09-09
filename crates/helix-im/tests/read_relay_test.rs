//! spec06 缺陷A 回归测试 — 读族 outbound HTTP 响应回灌前端（request-response 回灌）。
//!
//! 回归锚：spec06「06-读族回灌+完整投影」缺陷A。
//!
//! ## 被测不变量
//! 1. 读命令（`is_read=true`，含 Search Gate 四件套）dispatch 后注册
//!    `CorrelationContext::OutboundReadReply{req_id}` → HTTP 回报时 emit `im:read:result{req_id,body}`
//!    **透传后端响应体**（后端返回什么吐什么）。
//! 2. 写命令（`is_read=false`）**不**注册回灌上下文 → HTTP 回报无 corr 命中 → 静默 no-op（数据走 WS 回声）。
//! 3. HTTP 失败 / 信封畸形 → emit `im:read:result{req_id,error}`（不丢 req_id 关联，前端 reject）。
//!
//! ## 可证伪性（HX-C011）
//! `read_command_registers_relay_and_emits_result` 依赖 `module.rs` dispatch 的
//! `corr_map.insert(OutboundReadReply)`：**删该行** → PortReply 查不到上下文 → 不 emit → 本测试 FAIL
//! （正是缺陷A 现状：响应静默丢弃，前端永远拿不到数据）。

mod common;
use common::*;

use helix_core::tick::AppCommand;
use helix_core::{Effect, EffectSink, Tick};
use helix_im::commands;

// ─── 局部 helper ──────────────────────────────────────────────────────────────

/// 构造一条 outbound 读/写命令 Tick（任意命令名 + JSON payload）。
fn outbound_cmd(name: &'static str, payload: serde_json::Value) -> Tick {
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Command(AppCommand::new(name, bytes))
}

/// 从 sink 取本批次第一条 `Effect::Http` 的 corr.raw()（读命令 dispatch 产单条 Http）。
fn http_corr(sink: &EffectSink) -> Option<u64> {
    sink.as_slice().iter().find_map(|e| match e {
        Effect::Http { corr, .. } => Some(corr.raw()),
        _ => None,
    })
}

/// 从 sink 取本批次第一条 `Effect::Emit` 的 payload（解为 JSON）。
fn emit_json(sink: &EffectSink) -> Option<serde_json::Value> {
    sink.as_slice().iter().find_map(|e| match e {
        Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
        _ => None,
    })
}

fn emit_jsons(sink: &EffectSink) -> Vec<serde_json::Value> {
    sink.as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

// ─── 不变量 1：读命令注册回灌 + emit 透传 body ──────────────────────────────────

#[test]
fn read_command_registers_relay_and_emits_result() {
    // 前置：is_read 真源——top20 是读族。
    assert!(
        commands::is_read("im_top20"),
        "im_top20 必为读族（posts_read）"
    );

    let mut h = Harness::new(&[]);

    // ① 喂读命令（带前端 bridge 注入的 req_id）。
    let cmd = outbound_cmd(
        "im_top20",
        serde_json::json!({ "channel_id": "ch00000000000000000000000a", "req_id": "rq-1" }),
    );
    let kinds = h.step(cmd, 1000);
    // dispatch 产单条 Http；取出其 corr（不硬编码，鲁棒）。
    let corr = http_corr(&h.sink).expect("读命令 dispatch 必产 Effect::Http");
    assert!(
        kinds.iter().any(|k| matches!(k, EffectKind::Http(_))),
        "读命令产 Http"
    );

    // ② HTTP 回报：driver 回灌 ADR-007 信封 {status,headers,body:base64(raw_go_body)}。
    //    raw_go_body = 后端真实响应体（这里是 top20 的消息数组样例）。
    let raw_go_body = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status": "SUCCESS",
            "data": [ { "id": "m1", "message": "hi" }, { "id": "m2", "message": "yo" } ]
        }))
        .unwrap(),
    );
    let reply = port_reply_with_body(corr, enveloped(raw_go_body.clone()));
    h.step(reply, 1001);

    // ③ 断言 emit im:read:result{req_id, body}——body = 后端响应体**原样透传**。
    let v = emit_json(&h.sink).expect("读 HTTP Ok 必 emit im:read:result");
    assert_eq!(v["event"], "im:read:result", "channel 必为 im:read:result");
    assert_eq!(v["data"]["req_id"], "rq-1", "req_id 必原样回填");
    // 透传：body 结构 == 后端响应体（status + data 数组逐字）。
    assert_eq!(v["data"]["body"]["status"], "SUCCESS");
    assert_eq!(v["data"]["body"]["data"][0]["id"], "m1");
    assert_eq!(v["data"]["body"]["data"][1]["message"], "yo");
    // 无 error 字段（成功路径）。
    assert!(v["data"].get("error").is_none(), "成功路径不带 error");
}

/// 置顶详情首次走权威 HTTP，必须在本地绝对投影 PersistOk 后回灌；后续 query 只读本地。
#[test]
fn pinned_authority_read_persists_before_local_replay() {
    use helix_core::effect::{Row, SqlValue, StorageOp};
    use helix_core::port_codec::rows_to_reply_bytes;

    let channel_id = "ch00000000000000000000000a";
    let mut h = Harness::new_with_identity(&[], "user-a", "company-a", "", "", "");
    h.step(
        outbound_cmd(
            "im_channel_load_post_pinned",
            serde_json::json!({ "channel_id": channel_id, "req_id": "pin-http-1" }),
        ),
        1000,
    );
    let http = http_corr(&h.sink).expect("pinned miss authority read emits HTTP");
    let authority = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status": "SUCCESS",
            "data": [{ "post": { "id": "post-1" }, "operator": "user-a" }]
        }))
        .unwrap(),
    );

    let kinds = h.step(
        port_reply_with_body(http, enveloped(authority.clone())),
        1001,
    );
    let persist = kinds
        .iter()
        .find_map(|kind| match kind {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("authority body waits for durable projection");
    assert!(emit_json(&h.sink).is_none(), "PersistOk 前不得回灌结果");
    assert!(h.sink.as_slice().iter().any(|effect| matches!(
        effect,
        Effect::Persist { ops, .. }
            if matches!(ops.first(), Some(StorageOp::BatchUpsert(spec)) if spec.table == "channel_pinned_projection")
    )));

    h.step(port_reply_ok(persist), 1002);
    let emitted = emit_json(&h.sink).expect("PersistOk 后回灌 authority body");
    assert_eq!(emitted["data"]["body"]["data"][0]["post"]["id"], "post-1");

    h.step(
        outbound_cmd(
            "im_query_pinned_projection",
            serde_json::json!({ "channel_id": channel_id, "req_id": "pin-local-1" }),
        ),
        1003,
    );
    let local_corr = h.sink.as_slice().iter().find_map(|effect| match effect {
        Effect::Persist { corr, ops }
            if matches!(ops.first(), Some(StorageOp::Get(spec)) if spec.table == "channel_pinned_projection") => Some(corr.raw()),
        _ => None,
    }).expect("warm pinned query uses local Get");
    let row: Row = vec![(
        "body".to_string(),
        SqlValue::Text(String::from_utf8(authority.to_vec()).unwrap()),
    )];
    h.step(
        port_reply_with_body(local_corr, rows_to_reply_bytes(&[row])),
        1004,
    );
    let cached = emit_json(&h.sink).expect("local projection returns typed result");
    assert_eq!(cached["data"]["body"]["cached"], true);
    assert_eq!(
        cached["data"]["body"]["body"]["data"][0]["post"]["id"],
        "post-1"
    );
    assert!(!h
        .sink
        .as_slice()
        .iter()
        .any(|effect| matches!(effect, Effect::Http { .. })));
}

/// 聚合搜索必须走读回 correlation，并把 Go 三段结果作为权威 JSON 原样交付。
#[test]
fn search_do_registers_relay_and_passes_three_sections_through() {
    assert!(commands::is_read("im_search_do"), "聚合搜索必须为读族");
    let mut h = Harness::new(&[]);
    h.step(
        outbound_cmd(
            "im_search_do",
            serde_json::json!({
                "req_id": "search-do-1",
                "keyword": "项目",
                "user_cursor": "user-opaque-1",
                "channel_cursor": "channel-opaque-1",
                "message_group_cursor": "message-opaque-1",
                "page_size": 20
            }),
        ),
        1000,
    );
    let (corr, wire_body) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } => Some((
                corr.raw(),
                serde_json::from_slice::<serde_json::Value>(
                    req.body.as_ref().expect("search/do 必须有 JSON body"),
                )
                .expect("search/do wire body 必须是合法 JSON"),
            )),
            _ => None,
        })
        .expect("search/do 必须产生 HTTP");
    assert_eq!(
        wire_body,
        serde_json::json!({
            "keyword": "项目",
            "userCursor": "user-opaque-1",
            "channelCursor": "channel-opaque-1",
            "messageGroupCursor": "message-opaque-1",
            "pageSize": 20
        })
    );

    let authority = serde_json::json!({
        "users": { "items": [{ "id": "u1" }], "nextCursor": null, "hasMore": false },
        "channels": { "items": [{ "id": "c1" }], "nextCursor": null, "hasMore": false },
        "posts": {
            "groups": [{ "channel": { "id": "c1" }, "items": [{ "id": "p1" }] }],
            "nextCursor": "post-next",
            "hasMore": true
        }
    });
    h.step(
        port_reply_with_body(
            corr,
            enveloped(bytes::Bytes::from(
                serde_json::to_vec(&authority).expect("静态三段结果必须可序列化"),
            )),
        ),
        1001,
    );
    let result = emit_json(&h.sink).expect("search/do HTTP Ok 必须回灌");
    assert_eq!(result["event"], "im:read:result");
    assert_eq!(result["data"]["req_id"], "search-do-1");
    assert_eq!(result["data"]["body"], authority);
}

#[test]
fn announcement_delete_replies_and_reloads_same_version_list() {
    let mut h = Harness::new(&[]);
    h.step(
        outbound_cmd(
            "im_announcement_delete",
            serde_json::json!({
                "channel_id": "ch00000000000000000000000a",
                "announcement_id": "ann-1",
                "req_id": "ann-delete-1"
            }),
        ),
        1000,
    );
    let delete_corr = http_corr(&h.sink).expect("announcement delete must produce HTTP");
    h.step(
        port_reply_with_body(
            delete_corr,
            enveloped(bytes::Bytes::from(
                serde_json::to_vec(&serde_json::json!({
                    "status": "SUCCESS",
                    "data": {
                        "channelId": "ch00000000000000000000000a",
                        "deletedAnnouncementId": "ann-1",
                        "version": 7
                    }
                }))
                .unwrap(),
            )),
        ),
        1001,
    );
    let delete_result = emit_json(&h.sink).expect("delete HTTP must resolve req_id");
    assert_eq!(delete_result["data"]["req_id"], "ann-delete-1");
    assert_eq!(
        delete_result["data"]["body"],
        serde_json::json!({
            "channelId": "ch00000000000000000000000a",
            "deletedAnnouncementId": "ann-1",
            "version": 7
        })
    );
    let reload_corr = http_corr(&h.sink).expect("delete must trigger same-version list reload");
    let reload_body = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if corr.raw() == reload_corr => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok()),
            _ => None,
        })
        .expect("reload body JSON");
    assert_eq!(
        reload_body,
        serde_json::json!({ "channelId": "ch00000000000000000000000a" })
    );

    h.step(
        port_reply_with_body(
            reload_corr,
            enveloped(bytes::Bytes::from(
                serde_json::to_vec(&serde_json::json!({
                    "status": "SUCCESS",
                    "data": {
                        "channelId": "ch00000000000000000000000a",
                        "version": 7,
                        "announcements": []
                    }
                }))
                .unwrap(),
            )),
        ),
        1002,
    );
    let events = emit_jsons(&h.sink);
    assert!(events.iter().any(|event| {
        event["event"] == "im:announcement:list-updated" && event["data"]["version"] == 7
    }));
}

#[test]
fn announcement_delete_noop_version_zero_replies_without_list_reload() {
    let mut h = Harness::new(&[]);
    h.step(
        outbound_cmd(
            "im_announcement_delete",
            serde_json::json!({
                "channel_id": "ch00000000000000000000000a",
                "announcement_id": "ann-missing",
                "req_id": "ann-delete-noop"
            }),
        ),
        1000,
    );
    let delete_corr = http_corr(&h.sink).expect("announcement delete must produce HTTP");
    h.step(
        port_reply_with_body(
            delete_corr,
            enveloped(bytes::Bytes::from(
                serde_json::to_vec(&serde_json::json!({
                    "status": "SUCCESS",
                    "data": {
                        "channelId": "ch00000000000000000000000a",
                        "deletedAnnouncementId": "ann-missing",
                        "version": 0
                    }
                }))
                .unwrap(),
            )),
        ),
        1001,
    );

    let result = emit_json(&h.sink).expect("no-op delete must resolve req_id");
    assert_eq!(result["data"]["req_id"], "ann-delete-noop");
    assert_eq!(
        result["data"]["body"],
        serde_json::json!({
            "channelId": "ch00000000000000000000000a",
            "deletedAnnouncementId": "ann-missing",
            "version": 0,
            "noOp": true
        })
    );
    assert!(
        h.sink
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Http { .. })),
        "version=0/no-op delete must not enqueue announcement list reload"
    );
}

#[test]
fn get_schedule_keeps_req_id_local_and_omits_client_owner_from_wire() {
    let mut h = Harness::new(&[]);
    h.step(
        outbound_cmd(
            "im_get_schedule",
            serde_json::json!({
                "channel_id": "ch00000000000000000000000a",
                "user_id": "legacy-client-owner",
                "req_id": "schedule-r1"
            }),
        ),
        1000,
    );

    let (corr, body) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } => Some((
                corr.raw(),
                serde_json::from_slice::<serde_json::Value>(
                    req.body.as_ref().expect("getSchedule 必须有 JSON body"),
                )
                .expect("getSchedule body 必须是合法 JSON"),
            )),
            _ => None,
        })
        .expect("getSchedule 必须发 HTTP");
    assert_eq!(
        body,
        serde_json::json!({"channelId": "ch00000000000000000000000a"}),
        "owner 由 Go session 派生；user_id/req_id 都不得进入 wire"
    );

    let raw = bytes::Bytes::from(br#"{"status":"SUCCESS","data":[]}"#.to_vec());
    h.step(port_reply_with_body(corr, enveloped(raw)), 1001);
    let result = emit_json(&h.sink).expect("getSchedule 读回必须 emit im:read:result");
    assert_eq!(result["event"], "im:read:result");
    assert_eq!(
        result["data"]["req_id"], "schedule-r1",
        "req_id 必须只保留在本地 CorrelationContext 并在读回时恢复"
    );
}

#[test]
/// 证明 G11 已从 raw read/located 双事件硬切到 attached timeline Frame V2。
fn locate_post_without_attached_timeline_emits_no_raw_projection() {
    let mut h = Harness::new(&[]);
    h.step(
        outbound_cmd(
            "im_get_posts",
            serde_json::json!({ "post_ids": ["p1"], "req_id": "locate-1", "locate": true }),
        ),
        1000,
    );
    let corr = http_corr(&h.sink).expect("locate dispatch must emit Http");
    let raw = bytes::Bytes::from(
        br#"{"status":"SUCCESS","data":[{"id":"p1","temporaryId":"tmp-p1","channelId":"ch00000000000000000000000a","userId":"u1","type":"TEXT","message":"target","viewers":["all"]}]}"#
            .to_vec(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 1001);

    assert!(
        emit_jsons(&h.sink).is_empty(),
        "G11 cannot publish raw read/located events or insert an unattached row"
    );
}

// ─── 不变量 2：写命令不注册回灌（fire-and-forget）────────────────────────────────

#[test]
fn write_command_does_not_register_relay() {
    // 前置：revoke 是写族（撤回，数据更新走 WS 回声）。
    assert!(
        commands::is_outbound("im_revoke"),
        "im_revoke 必为 outbound"
    );
    assert!(!commands::is_read("im_revoke"), "im_revoke 必非读族（写）");

    let mut h = Harness::new(&[]);

    // ① 喂写命令（即使带 req_id，写族也不注册回灌）。
    let cmd = outbound_cmd(
        "im_revoke",
        serde_json::json!({ "post_id": "p1", "req_id": "rq-w" }),
    );
    h.step(cmd, 1000);
    let corr = http_corr(&h.sink).expect("写命令 dispatch 必产 Effect::Http");

    // ② HTTP 回报：corr 未注册回灌上下文 → handle_port_reply 单次 remove 查不到 → 静默 no-op。
    let raw = bytes::Bytes::from(br#"{"status":"SUCCESS"}"#.to_vec());
    let kinds = h.step(port_reply_with_body(corr, enveloped(raw)), 1001);

    // ③ 断言：无任何 emit（写族 ack 可丢，数据走 WS 回声，不回灌）。
    assert!(
        !kinds.iter().any(|k| matches!(k, EffectKind::Emit)),
        "写命令 HTTP 回报不应 emit（fire-and-forget）"
    );
    assert!(emit_json(&h.sink).is_none(), "写命令 HTTP 回报零 Emit");
}

// ─── 不变量 3：HTTP 失败 / 信封畸形 → emit error（不丢 req_id）─────────────────────

#[test]
fn read_http_error_emits_read_error() {
    let mut h = Harness::new(&[]);
    let cmd = outbound_cmd(
        "im_top20",
        serde_json::json!({ "channel_id": "ch00000000000000000000000a", "req_id": "rq-e" }),
    );
    h.step(cmd, 1000);
    let corr = http_corr(&h.sink).expect("读命令产 Http");

    // PortReply Err（网络/HTTP 层失败）。
    let reply = Tick::PortReply {
        corr: helix_core::effect::Correlation::from_raw(corr),
        outcome: helix_core::tick::PortOutcome::Err(helix_core::tick::PortError::Network),
    };
    h.step(reply, 1001);

    let v = emit_json(&h.sink).expect("读 HTTP Err 必 emit im:read:result（带 error）");
    assert_eq!(v["event"], "im:read:result");
    assert_eq!(
        v["data"]["req_id"], "rq-e",
        "失败路径也回填 req_id（前端 reject）"
    );
    assert!(v["data"].get("error").is_some(), "失败路径带 error 字段");
    assert!(v["data"].get("body").is_none(), "失败路径不带 body");
}

#[test]
fn read_malformed_envelope_emits_read_error() {
    let mut h = Harness::new(&[]);
    let cmd = outbound_cmd(
        "im_top20",
        serde_json::json!({ "channel_id": "ch00000000000000000000000a", "req_id": "rq-m" }),
    );
    h.step(cmd, 1000);
    let corr = http_corr(&h.sink).expect("读命令产 Http");

    // Ok 但 body 不是合法 ADR-007 信封（缺 body 字段）→ unwrap_sync_envelope Err → read error。
    let bad = bytes::Bytes::from(br#"{"not":"an envelope"}"#.to_vec());
    h.step(port_reply_with_body(corr, bad), 1001);

    let v = emit_json(&h.sink).expect("信封畸形必 emit im:read:result（带 error，不丢 req_id）");
    assert_eq!(v["event"], "im:read:result");
    assert_eq!(v["data"]["req_id"], "rq-m");
    assert!(v["data"].get("error").is_some(), "信封畸形走 error 路径");
}

// ─── 不变量 4：缺 req_id 降级 fire-and-forget（不注册，不 panic）────────────────────

#[test]
fn read_command_without_req_id_degrades_to_fire_and_forget() {
    let mut h = Harness::new(&[]);
    // 读命令但前端未注入 req_id（非 bridge 调用）→ 仍 fire HTTP，但不注册回灌。
    let cmd = outbound_cmd(
        "im_top20",
        serde_json::json!({ "channel_id": "ch00000000000000000000000a" }),
    );
    let kinds = h.step(cmd, 1000);
    assert!(
        kinds.iter().any(|k| matches!(k, EffectKind::Http(_))),
        "缺 req_id 仍 fire HTTP（命令本身不受 req_id 影响）"
    );
    let corr = http_corr(&h.sink).expect("读命令产 Http");

    // HTTP 回报：未注册 → 查不到上下文 → 无 emit（无 req_id 无从回灌，符合零信任降级）。
    let raw = bytes::Bytes::from(br#"{"status":"SUCCESS","data":[]}"#.to_vec());
    let after = h.step(port_reply_with_body(corr, enveloped(raw)), 1001);
    assert!(
        !after.iter().any(|k| matches!(k, EffectKind::Emit)),
        "缺 req_id 的读命令回报不回灌（无关联键）"
    );
}

#[test]
fn reply_query_without_req_id_never_emits_uncorrelated_projection() {
    let mut h = Harness::new(&[]);
    h.step(
        outbound_cmd(
            "im_get_replies",
            serde_json::json!({"reply_id":"root","channel_id":"c1"}),
        ),
        1000,
    );
    let corr = http_corr(&h.sink).expect("缺 req_id 仍可发 HTTP，但不得注册投影回灌");
    let raw = bytes::Bytes::from(
        br#"{"data":{"rootPost":{"id":"root","channelId":"c1"},"replies":[]}}"#.to_vec(),
    );
    let effects = h.step(port_reply_with_body(corr, enveloped(raw)), 1001);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, EffectKind::Emit)),
        "无 req_id 无法关联抽屉，必须 fail-closed 为零投影"
    );
}

#[test]
fn members_by_ids_emits_authoritative_snapshot_projection() {
    let mut h = Harness::new(&[]);
    let channel_id = cid(21);
    h.step(
        outbound_cmd(
            "im_channels_members_by_ids",
            serde_json::json!({"channel_ids":[channel_id],"req_id":"members-snapshot"}),
        ),
        1_000,
    );
    let corr = http_corr(&h.sink).expect("byIds 必须发 HTTP");
    let raw = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status":"SUCCESS",
            "data": {(channel_id.clone()): [{"id":"u1","role":"MEMBER","nickName":"成员"}]}
        }))
        .unwrap(),
    );
    h.step(port_reply_with_body(corr, enveloped(raw)), 1_001);

    let members = emit_jsons(&h.sink)
        .into_iter()
        .find(|value| value["event"] == "im:channel:members")
        .expect("byIds 必须输出成员快照");
    assert_eq!(members["data"]["projectionMode"], "snapshot");
    assert_eq!(members["data"]["channelId"], channel_id);
}

/// 证明 G12 新快照只产 Frame，迟到旧 authority 不再泄漏 raw read/reply event。
#[test]
fn stale_reply_snapshot_is_suppressed_after_latest_frame() {
    let mut h = Harness::new(&[]);
    let channel_id = cid(31);
    let root_id = cid(32);
    let latest_reply_id = cid(33);
    let stale_reply_id = cid(34);
    h.step(
        outbound_cmd(
            "im_get_replies",
            serde_json::json!({
                "reply_id":root_id,
                "channel_id":channel_id,
                "req_id":"old"
            }),
        ),
        1000,
    );
    let old_corr = http_corr(&h.sink).unwrap();
    h.step(
        outbound_cmd(
            "im_get_replies",
            serde_json::json!({
                "reply_id":root_id,
                "channel_id":channel_id,
                "req_id":"new"
            }),
        ),
        1001,
    );
    let new_corr = http_corr(&h.sink).unwrap();

    let latest = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "data": {
                "rootPost": {
                    "id": root_id,
                    "channelId": channel_id,
                    "userId": "viewer",
                    "type": "TEXT",
                    "message": "root"
                },
                "replies": [{
                    "id": latest_reply_id,
                    "channelId": channel_id,
                    "userId": "viewer",
                    "type": "TEXT",
                    "message": "latest",
                    "replyRootId": root_id
                }]
            }
        }))
        .expect("latest G12 authority serializes"),
    );
    h.step(port_reply_with_body(new_corr, enveloped(latest)), 1002);
    let latest_emits = emit_jsons(&h.sink);
    assert_eq!(latest_emits.len(), 1);
    assert_eq!(latest_emits[0]["event"], "im:timeline:thread");
    assert_eq!(latest_emits[0]["data"]["requestId"], "new");
    assert_eq!(latest_emits[0]["data"]["projectionMode"], "snapshot");

    let stale = bytes::Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "data": {
                "rootPost": {
                    "id": root_id,
                    "channelId": channel_id,
                    "userId": "viewer",
                    "type": "TEXT",
                    "message": "root"
                },
                "replies": [{
                    "id": stale_reply_id,
                    "channelId": channel_id,
                    "userId": "viewer",
                    "type": "TEXT",
                    "message": "stale",
                    "replyRootId": root_id
                }]
            }
        }))
        .expect("stale G12 authority serializes"),
    );
    h.step(port_reply_with_body(old_corr, enveloped(stale)), 1003);
    let stale_emits = emit_jsons(&h.sink);
    assert!(stale_emits.is_empty(), "旧 revision 不得输出任何渲染事件");
}

// ─── is_read 注册表覆盖（读族标注正确性，防漂移）──────────────────────────────────

/// 注册表必须把所有 request-response 命令标为读族，并排除 WS authority 写族。
#[test]
fn is_read_marks_read_family_only() {
    // posts_read 17 全读。
    for name in [
        "im_get_schedule",
        "im_post_context",
        "im_top20",
        "im_get_posts",
        "im_get_posts_after_index",
        "im_get_replies",
        "im_get_reply_branch",
        "im_query_todo_list",
        "im_get_updated_posts",
        "im_get_latest_post",
        "im_bookmark_create",
        "im_bookmark_delete",
        "im_bookmark_load",
        "im_announcement_accept_list",
        "im_announcement_list",
        "im_announcement_delete",
        "im_announcement_detail",
        "im_post_read_list",
    ] {
        assert!(commands::is_read(name), "{name} 必为读族（posts_read）");
    }
    // user-misc 读族包含 Search Gate 四件套。
    for name in [
        "im_list_users",
        "im_get_user_statuses",
        "im_get_all_modules",
        "im_load_send_notifications",
        "im_search_post",
        "im_search_user",
        "im_search_channel",
        "im_search_do",
    ] {
        assert!(commands::is_read(name), "{name} 必为读族（user-misc 读）");
    }
    // 写族 / 排除项 → 非读。
    for name in [
        "im_revoke",
        "im_urgent_post",
        "im_team_upsert",
        "im_team_member_add",
        "im_team_quit",
    ] {
        assert!(!commands::is_read(name), "{name} 必非读族");
    }
    // 未认领命令 → 非读（不 panic）。
    assert!(!commands::is_read("not_a_command"));
}

/// 聚合搜索必须走读回 correlation，并把 Go 三段结果作为权威 JSON 原样交付。
#[test]
fn markdown_search_preserves_literal_query_and_authority() {
    assert!(commands::is_read("im_search_do"), "聚合搜索必须为读族");
    let mut h = Harness::new(&[]);
    h.step(
        outbound_cmd(
            "im_search_do",
            serde_json::json!({
                "req_id": "search-do-1",
                "keyword": "Vec<T> + [x]",
                "user_cursor": "user-opaque-1",
                "channel_cursor": "channel-opaque-1",
                "message_group_cursor": "message-opaque-1",
                "page_size": 20
            }),
        ),
        1000,
    );
    let (corr, wire_body) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } => Some((
                corr.raw(),
                serde_json::from_slice::<serde_json::Value>(
                    req.body.as_ref().expect("search/do 必须有 JSON body"),
                )
                .expect("search/do wire body 必须是合法 JSON"),
            )),
            _ => None,
        })
        .expect("search/do 必须产生 HTTP");
    assert_eq!(
        wire_body,
        serde_json::json!({
            "keyword": "Vec<T> + [x]",
            "userCursor": "user-opaque-1",
            "channelCursor": "channel-opaque-1",
            "messageGroupCursor": "message-opaque-1",
            "pageSize": 20
        })
    );

    let authority = serde_json::json!({
        "users": { "items": [{ "id": "u1" }], "nextCursor": null, "hasMore": false },
        "channels": { "items": [{ "id": "c1" }], "nextCursor": null, "hasMore": false },
        "posts": {
            "groups": [{ "channel": { "id": "c1" }, "items": [{ "id": "p1", "message": "**Rust** 与 `Vec<T>`", "highlight": "**Rust** 与 `<em>Vec&lt;T&gt;</em>`" }] }],
            "nextCursor": "post-next",
            "hasMore": true
        }
    });
    h.step(
        port_reply_with_body(
            corr,
            enveloped(bytes::Bytes::from(
                serde_json::to_vec(&authority).expect("静态三段结果必须可序列化"),
            )),
        ),
        1001,
    );
    let result = emit_json(&h.sink).expect("search/do HTTP Ok 必须回灌");
    assert_eq!(result["event"], "im:read:result");
    assert_eq!(result["data"]["req_id"], "search-do-1");
    assert_eq!(result["data"]["body"], authority);
}

/// 纯媒体和结构化卡片有 props 时允许空正文，不再按 TEXT 校验吞掉命令。
#[test]
fn schedule_accepts_empty_text_with_structured_content() {
    for kind in [
        "RICH",
        "IMAGE",
        "VIDEO",
        "FILE",
        "AUDIO",
        "TEMPLATE",
        "CUSTOM_CARD",
    ] {
        let result = commands::handle_outbound(
            "im_create_schedule",
            serde_json::to_vec(&serde_json::json!({
                "channel_id": "ch00000000000000000000000a", "message": "", "type": kind,
                "props": {"card": {"id": "c1"}}, "schedule_post_at": 1900000000000i64
            }))
            .unwrap()
            .as_slice(),
            "http://test/api/cses",
            "http://java",
            None,
            helix_core::Correlation::from_raw(1),
        );
        assert!(result.is_ok(), "{kind} has structured content: {result:?}");
    }
}

/// 完全空消息仍失败关闭，不能以放宽正文限制发送空 post。
#[test]
fn schedule_rejects_empty_content() {
    let result = commands::handle_outbound("im_create_schedule", br#"{"channel_id":"ch00000000000000000000000a","message":"","type":"TEXT","props":{},"schedule_post_at":1900000000000}"#, "http://test", "http://java", None, helix_core::Correlation::from_raw(1));
    assert!(result.is_err());
}

/// 定时附件必须先取得上传票据，上传完成后创建一次，重复回包不得重建。
#[test]
fn schedule_media_upload_precedes_create_and_ignores_duplicate() {
    let mut h = Harness::new(&[1]);
    h.step(outbound_cmd("im_create_schedule", serde_json::json!({
        "channel_id": cid(1), "message": "", "type": "FILE", "req_id": "schedule-media",
        "schedule_post_at": 1900000000000i64,
        "props": {"file": {"mediaInput": {"localPath": "/tmp/test.pdf", "fileName": "test.pdf", "contentType": "application/pdf", "size": 9, "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}
    })), 1);
    let prepare = http_corr(&h.sink).unwrap();
    assert!(h.sink.as_slice().iter().any(|effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/oss/getPresignedUrl"))));
    h.step(port_reply_with_body(prepare, enveloped(bytes::Bytes::from(serde_json::to_vec(&serde_json::json!({
        "isSuccess": true, "fileId": "file-1", "bucket": "attachment", "method": "PUT",
        "url": "https://oss.test/test.pdf", "uri": "media/file-1", "objectKey": "media/file-1",
        "headers": {"Content-Type": "application/pdf", "x-oss-forbid-overwrite": "true"}
    })).unwrap()))), 2);
    let upload = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::UploadFile { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .unwrap();
    assert!(http_corr(&h.sink).is_none());
    h.step(port_reply_ok(upload), 3);
    let body: serde_json::Value = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with("/posts/createSchedule") => {
                Some(serde_json::from_slice(req.body.as_ref().unwrap()).unwrap())
            }
            _ => None,
        })
        .expect("upload must complete before schedule create");
    assert_eq!(body["post"]["props"]["file"]["uri"], "media/file-1");
    assert!(body["post"]["props"]["file"].get("mediaInput").is_none());
    h.step(port_reply_ok(upload), 4);
    assert!(http_corr(&h.sink).is_none());
}

/// 票据失败与超时都返回关联错误，迟到响应不得产生创建请求。
#[test]
fn schedule_media_failure_and_timeout_release_intent() {
    for timeout in [false, true] {
        let mut h = Harness::new(&[1]);
        h.step(outbound_cmd("im_create_schedule", serde_json::json!({
            "channel_id": cid(1), "message": "", "type": "FILE", "req_id": "schedule-media-failure",
            "schedule_post_at": 1900000000000i64,
            "props": {"file": {"mediaInput": {"localPath": "/tmp/test.pdf", "fileName": "test.pdf", "contentType": "application/pdf", "size": 9, "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}
        })), 1);
        let corr = http_corr(&h.sink).unwrap();
        if timeout {
            let timer = h
                .sink
                .as_slice()
                .iter()
                .find_map(|effect| match effect {
                    Effect::ScheduleTimer { id, .. } => Some(*id),
                    _ => None,
                })
                .unwrap();
            h.step(Tick::Timer(timer), 120001);
        } else {
            h.step(port_reply_err(corr), 2);
        }
        let event = emit_json(&h.sink).unwrap();
        assert_eq!(event["data"]["req_id"], "schedule-media-failure");
        assert!(event["data"].get("error").is_some());
        h.step(port_reply_ok(corr), 120002);
        assert!(http_corr(&h.sink).is_none());
    }
}

/// 无正文且无结构内容时立即回传错误，不能让宿主等到请求超时。
#[test]
fn schedule_invalid_content_completes_request_without_io() {
    let mut h = Harness::new(&[1]);
    h.step(
        outbound_cmd(
            "im_create_schedule",
            serde_json::json!({
                "channel_id": cid(1), "message": "", "type": "TEXT", "props": {},
                "req_id": "schedule-invalid", "schedule_post_at": 1900000000000i64
            }),
        ),
        1,
    );
    assert!(http_corr(&h.sink).is_none());
    let event = emit_json(&h.sink).unwrap();
    assert_eq!(event["data"]["req_id"], "schedule-invalid");
    assert_eq!(event["data"]["error"], "SCHEDULE_CONTENT_INVALID");
}

/// Runtime 停止后释放上传等待者，旧 corr 的迟到回包不能恢复未提交意图。
#[test]
fn schedule_stop_releases_upload_and_rejects_late_reply() {
    use helix_core::Module;
    let mut h = Harness::new(&[1]);
    h.step(outbound_cmd("im_create_schedule", serde_json::json!({
        "channel_id": cid(1), "message": "", "type": "FILE", "req_id": "schedule-stop",
        "schedule_post_at": 1900000000000i64,
        "props": {"file": {"mediaInput": {"localPath": "/tmp/test.pdf", "fileName": "test.pdf", "contentType": "application/pdf", "size": 9, "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}}}
    })), 1);
    let corr = http_corr(&h.sink).unwrap();
    h.sink.clear();
    h.module.on_stop(&mut h.sink).unwrap();
    assert!(emit_jsons(&h.sink)
        .iter()
        .any(|event| event["data"]["error"] == "SCHEDULE_MEDIA_INTERRUPTED"));
    h.step(port_reply_ok(corr), 2);
    assert!(http_corr(&h.sink).is_none());
}

/// 定时读回回执覆盖创建和取消；HTTP 仅 admission，WS 提交决定请求结果。
fn schedule_request(h: &mut Harness, cancel: bool) -> u64 {
    let payload = if cancel {
        serde_json::json!({"channel_id": cid(1), "req_id": "schedule-once"})
    } else {
        serde_json::json!({"channel_id": cid(1), "message": "scheduled", "type": "TEXT",
            "schedule_post_at": 1900000000000i64, "req_id": "schedule-once"})
    };
    h.step(
        outbound_cmd(
            if cancel {
                "im_cancel_schedule"
            } else {
                "im_create_schedule"
            },
            payload,
        ),
        1,
    );
    http_corr(&h.sink).expect("schedule must dispatch HTTP")
}

/// 从公开 WS 入口送入当前 viewer 的权威版本，禁止直接调用持久化 handler。
fn schedule_authority(h: &mut Harness, cancel: bool, revision: u64) -> u64 {
    h.step(
        Tick::Inbound(helix_core::tick::InboundBytes(bytes::Bytes::from(
            serde_json::to_vec(&serde_json::json!({
                "action": if cancel { "post_schedule_canceled" } else { "post_schedule_created" },
                "data": { "channelId": cid(1), "scheduleId": "s1", "scheduleOwnerUserId": "actor",
                    "message": "scheduled", "sendAt": 1900000000000i64, "revision": revision }
            }))
            .unwrap(),
        ))),
        2 + revision,
    );
    h.sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("authority must persist atomically")
}

/// PersistErr 只结束请求一次；后续 WS 重放仍能恢复持久事实但不能再次 resolve。
#[test]
fn schedule_relay_persist_failure_then_replay_never_completes_twice() {
    for cancel in [false, true] {
        let mut h = Harness::new_with_identity(&[1], "actor", "company", "", "", "");
        let http = schedule_request(&mut h, cancel);
        h.step(
            port_reply_with_body(
                http,
                enveloped(bytes::Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
            ),
            2,
        );
        assert!(h.sink.as_slice().is_empty(), "admission cannot resolve");
        let persist = schedule_authority(&mut h, cancel, 8);
        h.step(port_reply_err(persist), 11);
        let events = emit_jsons(&h.sink);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["data"]["error"], "SCHEDULE_PERSIST_FAILED");
        assert_eq!(events[0]["data"]["req_id"], "schedule-once");
        let replay = schedule_authority(&mut h, cancel, 8);
        h.step(port_reply_ok(replay), 12);
        assert!(
            emit_jsons(&h.sink).is_empty(),
            "failed request cannot later succeed"
        );
        assert!(
            h.sink
                .as_slice()
                .iter()
                .any(|e| matches!(e, Effect::Persist { .. })),
            "authority recovery still reads durable facts"
        );
    }
}

/// WS 可能先于 HTTP 回包；已持久成功的请求不得被迟到的网络失败再次 reject。
#[test]
fn schedule_relay_late_http_failure_after_persist_is_ignored() {
    for cancel in [false, true] {
        let mut h = Harness::new_with_identity(&[1], "actor", "company", "", "", "");
        let http = schedule_request(&mut h, cancel);
        let persist = schedule_authority(&mut h, cancel, 8);
        h.step(port_reply_ok(persist), 11);
        assert_eq!(emit_jsons(&h.sink)[0]["data"]["body"]["ok"], true);
        h.step(port_reply_err(http), 12);
        assert!(
            emit_jsons(&h.sink).is_empty(),
            "durable success cannot later fail"
        );
    }
}

/// HTTP 失败发生在已入队 WS 的持久化回包前，也只能向调用方返回一次终态。
#[test]
fn schedule_relay_http_failure_during_persist_is_terminal() {
    for cancel in [false, true] {
        let mut h = Harness::new_with_identity(&[1], "actor", "company", "", "", "");
        let http = schedule_request(&mut h, cancel);
        let persist = schedule_authority(&mut h, cancel, 8);
        h.step(port_reply_err(http), 11);
        assert_eq!(
            emit_jsons(&h.sink)[0]["data"]["error"],
            "SCHEDULE_REQUEST_FAILED"
        );
        h.step(port_reply_ok(persist), 12);
        assert!(
            emit_jsons(&h.sink).is_empty(),
            "pending persist must not resolve rejected request"
        );
    }
}

/// 连续权威版本可各自提交，但共享的前端请求只能被第一个完成的持久屏障消费。
#[test]
fn schedule_relay_multiple_authorities_complete_request_once() {
    for cancel in [false, true] {
        let mut h = Harness::new_with_identity(&[1], "actor", "company", "", "", "");
        schedule_request(&mut h, cancel);
        let first = schedule_authority(&mut h, cancel, 8);
        let second = schedule_authority(&mut h, cancel, 9);
        h.step(port_reply_ok(first), 12);
        assert_eq!(emit_jsons(&h.sink).len(), 1);
        h.step(port_reply_ok(second), 13);
        assert!(
            emit_jsons(&h.sink).is_empty(),
            "same req_id must resolve once"
        );
        h.step(port_reply_ok(first), 14);
        assert!(
            h.sink.as_slice().is_empty(),
            "duplicate corr must be consumed"
        );
    }
}

/// HTTP 非 2xx、业务失败与畸形信封均拒绝请求，不能把 SUCCESS 字样当成持久成功。
#[test]
fn schedule_relay_http_failure_matrix_is_correlated_and_once() {
    for cancel in [false, true] {
        for response in [
            enveloped(bytes::Bytes::from_static(br#"{"status":"FAILED"}"#)),
            bytes::Bytes::from_static(br#"{"status":503,"body":"eyJzdGF0dXMiOiJTVUNDRVNTIn0="}"#),
            bytes::Bytes::from_static(br#"{"status":200,"body":"!"}"#),
        ] {
            let mut h = Harness::new_with_identity(&[1], "actor", "company", "", "", "");
            let http = schedule_request(&mut h, cancel);
            h.step(port_reply_with_body(http, response.clone()), 2);
            let events = emit_jsons(&h.sink);
            assert_eq!(events.len(), 1);
            assert_eq!(events[0]["data"]["req_id"], "schedule-once");
            assert_eq!(events[0]["data"]["error"], "SCHEDULE_REQUEST_FAILED");
            h.step(port_reply_with_body(http, response), 3);
            assert!(h.sink.as_slice().is_empty());
        }
    }
}

/// G04d 业务失败和非法 JSON 不得被包装成成功的空定时列表。
#[test]
fn schedule_relay_get_failure_never_becomes_empty_success() {
    for body in [
        br#"{"status":"FAILED","message":"denied"}"#.as_slice(),
        b"invalid-json".as_slice(),
    ] {
        let mut h = Harness::new(&[1]);
        h.step(
            outbound_cmd(
                "im_get_schedule",
                serde_json::json!({"channel_id":cid(1), "req_id":"get-failed"}),
            ),
            1,
        );
        let corr = http_corr(&h.sink).unwrap();
        h.step(
            port_reply_with_body(corr, enveloped(bytes::Bytes::copy_from_slice(body))),
            2,
        );
        let events = emit_jsons(&h.sink);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["data"]["req_id"], "get-failed");
        assert!(
            events[0]["data"].get("error").is_some(),
            "failed getSchedule cannot clear UI as empty success"
        );
        assert!(events[0]["data"].get("body").is_none());
    }
}
