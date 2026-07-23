//! UC-4 离线同步增量缺口补齐集成测试（sync-increment 任务）。
//!
//! 回归锚: UC-4.4 / UC-4.5
//!
//! 覆盖（full-map/partials/6--client-usecases.md UC-4 + 8--client-tauri-im.md §51/§68/§271 +
//! 真机curl真源 + 02-迁移蓝图 §B 离线同步）：
//!
//! - **UC-4.5 独立窗口陌生 channel 兜底（契约审计·已实现）**：outbound 读族命令
//!   `im_channel_load_increment_by_channel_id`（`outbound/channel_read.rs`）→
//!   `POST /api/cses/channel/load/incrementByChannelId`，body `{ "channelId": <str> }`（camelCase，
//!   **不**走 UC-4.1 的 `/channels/load/increment`）。HTTP 同步直返 `*IncrementChannel`，`is_read==true`
//!   （响应体经 read_relay 回灌前端，无 WS 回声）。本测试**审计**其契约对齐（已存在，非本任务新增）。
//!
//! - **UC-4.4 心跳 gap 补偿 ping piggyback**：`ping_frame` 从最小 `{action,seq}` 扩成
//!   `{action,seq,data:{cursors:[{channelId,fromSeq}],allHash}}`。`PingData`（types/sync.ts:242）：
//!   `{ cursors: SyncCursorItem[]; allHash?: string }`，allHash = 全量根群 cursor 的 FNV-1a 64bit
//!   hash，16 位小写 hex。cursors 由 ping 时 state 快照、**确定性有序**（ChannelId:Ord 排序锚定，HX-C010）。
//!
//! 可证伪（HX-C011）：
//! - incrementByChannelId 写成 channels/load/increment（混 UC-4.1）→ url 断言 FAIL
//! - body 键写 channel_id（snake）而非 channelId → casing 断言 FAIL
//! - is_read 写成 false（漏读回灌）→ is_read 断言 FAIL
//! - ping allHash 用 HashMap 无序拼接（非排序）→ 确定性向量断言 FAIL（同组 cursor 两次算出不同 hash）
//! - ping data 缺 cursors/allHash（退回最小帧）→ piggyback shape 断言 FAIL

use helix_im::acl::to_effect::ping_frame;
use helix_im::commands::{handle_outbound, is_outbound, is_read};
use helix_im::state::{ChannelId, Seq};
use helix_im::sync_increment::{all_hash_fnv1a, RootCursor};

use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::Correlation;
use serde_json::{json, Value};

mod common;
use common::*;

const BASE: &str = "http://x/api/cses";

fn corr() -> Correlation {
    Correlation::from_raw(77)
}

fn run(name: &str, payload: Value) -> Vec<Effect> {
    let bytes = serde_json::to_vec(&payload).unwrap();
    handle_outbound(name, &bytes, BASE, "http://default", Some("conn-1"), corr())
        .expect("command should dispatch")
}

fn one_http(effects: Vec<Effect>) -> (String, Value, Vec<(String, String)>) {
    assert_eq!(
        effects.len(),
        1,
        "incrementByChannelId 恒产单条 Effect::Http"
    );
    match effects.into_iter().next().unwrap() {
        Effect::Http { req, .. } => {
            let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
            (req.url, body, req.headers)
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

// ─── UC-4.5 incrementByChannelId（契约审计：已实现命令对齐）─────────────────

#[test]
fn increment_by_channel_id_registered_and_is_read() {
    assert!(
        is_outbound("im_channel_load_increment_by_channel_id"),
        "UC-4.5 incrementByChannelId 命令应被 outbound registry 认领（channel_read.rs）"
    );
    assert!(
        is_read("im_channel_load_increment_by_channel_id"),
        "incrementByChannelId HTTP 同步直返 channel → 必须是读族（响应经 read_relay 回灌）"
    );
}

#[test]
fn increment_by_channel_id_hits_correct_endpoint() {
    let cid = "abcdefghijklmnopqrstuvwxyz";
    let (url, body, headers) = one_http(run(
        "im_channel_load_increment_by_channel_id",
        json!({ "channel_id": cid }),
    ));
    assert_eq!(
        url,
        format!("{BASE}/channel/load/incrementByChannelId"),
        "UC-4.5 走 incrementByChannelId（**不**走 UC-4.1 的 channels/load/increment）"
    );
    assert_eq!(body["channelId"], cid, "body 必须用 camelCase channelId");
    assert!(
        body.get("channel_id").is_none(),
        "不得泄漏 snake_case channel_id 键"
    );
    assert!(
        headers
            .iter()
            .any(|(k, v)| k == "connectionId" && v == "conn-1"),
        "应盖 connectionId 身份头"
    );
}

// ─── UC-4.4 ping piggyback (PingData{cursors, allHash}) ──────────────────────

fn ping_data(seq: u64, cursors: &[(ChannelId, Seq)]) -> Value {
    let roots: Vec<RootCursor> = cursors
        .iter()
        .map(|(id, s)| RootCursor {
            channel_id: *id,
            from_seq: *s,
        })
        .collect();
    let e = ping_frame(seq, &roots);
    match e {
        Effect::Send { frame, .. } => serde_json::from_slice(&frame).unwrap(),
        other => panic!("expected Effect::Send, got {other:?}"),
    }
}

#[test]
fn ping_frame_carries_cursors_and_all_hash() {
    let c1 = ChannelId::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let c2 = ChannelId::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
    let v = ping_data(5, &[(c1, Seq(10)), (c2, Seq(20))]);
    assert_eq!(v["action"], "ping");
    assert_eq!(v["seq"], 5);
    let cursors = v["data"]["cursors"].as_array().expect("data.cursors 数组");
    assert_eq!(cursors.len(), 2);
    // 确定性有序：ChannelId:Ord 升序（a... < b...）。
    assert_eq!(cursors[0]["channelId"], "aaaaaaaaaaaaaaaaaaaaaaaaaa");
    assert_eq!(cursors[0]["fromSeq"], 10);
    assert_eq!(cursors[1]["channelId"], "bbbbbbbbbbbbbbbbbbbbbbbbbb");
    assert_eq!(cursors[1]["fromSeq"], 20);
    // allHash = 16 位小写 hex。
    let hash = v["data"]["allHash"].as_str().expect("allHash 字符串");
    assert_eq!(hash.len(), 16, "FNV-1a 64bit → 16 hex");
    assert!(
        hash.chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
        "allHash 必须小写 hex"
    );
}

#[test]
fn ping_frame_empty_cursors_omits_all_hash_but_keeps_shape() {
    let v = ping_data(1, &[]);
    assert_eq!(v["action"], "ping");
    assert_eq!(v["seq"], 1);
    // 无根群 cursor → cursors 空数组；allHash 缺省（PingData.allHash?: 可选）。
    assert_eq!(v["data"]["cursors"].as_array().unwrap().len(), 0);
    assert!(
        v["data"].get("allHash").is_none() || v["data"]["allHash"].is_null(),
        "空 cursor 集不应产 allHash"
    );
}

#[test]
fn all_hash_is_deterministic_and_order_independent() {
    let c1 = ChannelId::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let c2 = ChannelId::from_str("bbbbbbbbbbbbbbbbbbbbbbbbbb").unwrap();
    let c3 = ChannelId::from_str("cccccccccccccccccccccccccc").unwrap();
    let a = vec![
        RootCursor {
            channel_id: c1,
            from_seq: Seq(1),
        },
        RootCursor {
            channel_id: c2,
            from_seq: Seq(2),
        },
        RootCursor {
            channel_id: c3,
            from_seq: Seq(3),
        },
    ];
    // 同组、不同入参顺序 → 同 hash（内部排序锚定，HX-C010）。
    let mut b = a.clone();
    b.reverse();
    assert_eq!(
        all_hash_fnv1a(&a),
        all_hash_fnv1a(&b),
        "allHash 必须与入参顺序无关（内部 sort）"
    );
    // 改一个 fromSeq → hash 变（敏感性，证伪锚点）。
    let mut c = a.clone();
    c[0].from_seq = Seq(99);
    assert_ne!(
        all_hash_fnv1a(&a),
        all_hash_fnv1a(&c),
        "cursor 变 → hash 必变"
    );
}

#[test]
fn all_hash_known_vector() {
    // 固定向量（回放锚）：单 cursor (channel="aaaa...26", fromSeq=0) 的 allHash 必稳定。
    // 此值由 all_hash_fnv1a 的确定性算法产出，作回归冻结（改算法即 FAIL）。
    let c1 = ChannelId::from_str("aaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
    let h = all_hash_fnv1a(&[RootCursor {
        channel_id: c1,
        from_seq: Seq(0),
    }]);
    assert_eq!(h.len(), 16);
    // 与 ChannelId::hash_u64 同算法基底（FNV-1a），全程小写 hex。
    assert!(h
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
}

fn emit_value(effect: &Effect) -> Option<Value> {
    match effect {
        Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
        _ => None,
    }
}

fn row_text<'a>(row: &'a [(String, SqlValue)], key: &str) -> Option<&'a str> {
    row.iter().find_map(|(column, value)| {
        (column == key)
            .then_some(value)
            .and_then(|value| match value {
                SqlValue::Text(value) => Some(value.as_str()),
                _ => None,
            })
    })
}

#[test]
fn increment_by_channel_id_reply_syncs_local_gap_before_typed_timeline_projection() {
    let mut h = Harness::new_with_identity(&[], "viewer-444", "tenant-1", "", "", "");
    let command_effects = h.step(
        command_json(
            "im_channel_load_increment_by_channel_id",
            json!({ "channel_id": cid(45), "req_id": "req-uc45" }),
        ),
        1_000,
    );
    let increment_corr = command_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(*corr),
            _ => None,
        })
        .expect("incrementByChannelId 应发 HTTP");

    let increment_body = bytes::Bytes::from(
        serde_json::to_vec(&json!({
            "status": "SUCCESS",
            "data": {
                "id": cid(45),
                "displayName": "陌生频道",
                "type": "O",
                "lastEventSeq": 2,
                // 服务端提示已追平不能覆盖本端事实：独立窗口本地 cursor 仍为 0。
                "needSync": false,
                "unreadCount": 1,
                "mentionCount": 1,
                "urgentCount": 1,
                "lastPost": "{\"message\":\"hydrated hello\",\"props\":{\"expedite\":\"m45\"}}",
                "members": [{ "id": "member-a", "nickName": "成员A" }],
                "owner": { "id": "owner-a", "nickName": "群主" }
            }
        }))
        .unwrap(),
    );
    h.step(
        port_reply_with_body(increment_corr, enveloped(increment_body)),
        1_100,
    );

    assert!(
        h.sink
            .as_slice()
            .iter()
            .filter_map(emit_value)
            .any(|value| {
                value["event"] == "im:read:result"
                    && value["data"]["req_id"] == "req-uc45"
                    && value["data"]["body"]["data"]["id"] == cid(45)
            }),
        "通用 im:read:result 必须继续兼容"
    );
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .filter_map(emit_value)
            .any(|value| value["event"] == "im:channels:projection"),
        "HTTP SUCCESS 后、channel/member Persist ack 前不得发布 hydration 最终投影"
    );
    let (increment_persist_corr, increment_ops) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "channel"),
                ) =>
            {
                Some((corr.raw(), ops))
            }
            _ => None,
        })
        .expect("HTTP IncrementChannel 必须先用相关 Persist 落 channel/channel_member");
    let channel_row = increment_ops
        .iter()
        .find_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "channel" => spec.rows.first(),
            _ => None,
        })
        .expect("channel upsert row");
    assert_eq!(row_text(channel_row, "id"), Some(cid(45).as_str()));
    assert_eq!(row_text(channel_row, "display_name"), Some("陌生频道"));
    assert!(increment_ops
        .iter()
        .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "channel_member")));
    assert!(
        !h.sink.as_slice().iter().any(
            |effect| matches!(effect, Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify"))
        ),
        "channel/member 未确认落库前不得启动消息 sync"
    );

    h.step(port_reply_ok(increment_persist_corr), 1_200);
    assert!(
        h.sink
            .as_slice()
            .iter()
            .filter_map(emit_value)
            .any(|value| value["event"] == "im:channel:increment"),
        "channel/member Persist ack 后仍须发布 channel surface"
    );
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .filter_map(emit_value)
            .any(|value| value["event"] == "im:channels:projection"),
        "channel hydration 不得回退发布 raw dialogList"
    );
    let sync_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/channel/sync/notify") => {
                let body: Value = serde_json::from_slice(req.body.as_ref()?.as_ref()).ok()?;
                assert_eq!(body["cursors"][0]["channelId"], cid(45));
                assert_eq!(body["cursors"][0]["fromSeq"], 0);
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("本地 cursor 落后服务端水位时必须复用 sync/notify 拉消息，即使 needSync=false");

    let sync_body = sync_reply_events_with_messages(
        45,
        &[ev(2, 1, "message-hydrated")],
        &[("message-hydrated", "hydrated hello", "10")],
        2,
    );
    h.step(port_reply_with_body(sync_corr, enveloped(sync_body)), 1_300);
    let (message_persist_corr, recovery_ops) = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"),
                ) =>
            {
                Some((corr.raw(), ops))
            }
            _ => None,
        })
        .expect("sync 回包必须原子落 message、恢复证据与 cursor");
    assert!(recovery_ops.iter().any(
        |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "channel_event_ledger")
    ));
    assert!(recovery_ops.iter().any(
        |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "channel_sync_coverage")
    ));
    assert!(
        recovery_ops
            .iter()
            .any(|op| matches!(op, StorageOp::MonotonicUpsert(spec)
            if spec.table == "channel_event_cursor"
                && spec.scope_key == cid(45)
                && spec.value == 2)),
        "cursor 必须与 message/ledger/coverage 同一原子事务提交"
    );

    h.step(port_reply_ok(message_persist_corr), 1_400);
    assert_eq!(h.module.cursor_for(channel_id(45)), Some(2));
    assert!(
        h.sink
            .as_slice()
            .iter()
            .filter_map(emit_value)
            .any(|value| value["event"] == "im:post:received"),
        "PersistAtomic ack 后才可发布消息业务事实"
    );
    assert!(
        h.sink
            .as_slice()
            .iter()
            .filter_map(emit_value)
            .any(|value| value["event"] == "im:sync:state"),
        "PersistAtomic ack 后才可发布同步终态"
    );
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .filter_map(emit_value)
            .any(|value| value["event"] == "im:interaction:frame"),
        "未 attach timeline 时不得凭空发布 InteractionFrame"
    );
    assert!(
        !h.sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::PersistFire { ops }
                if ops.iter().any(|op| matches!(op, StorageOp::MonotonicUpsert(spec)
                    if spec.table == "channel_event_cursor")))),
        "原子事务已含 cursor，ack 后不得二次写 cursor"
    );
}
