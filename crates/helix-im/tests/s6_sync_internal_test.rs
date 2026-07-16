//! sync-v2 §S6 sync 引擎内化 对齐补全回放测试。
//!
//! 回归锚: S6（SPEC-sync-v2-full-replacement.md §S6）
//!
//! 补齐既有 gate_test / increment_flow_test 未显式锁定的契约：
//! - too_long 保留本地 message，复位 cursor、emit 并主动覆盖式重拉；
//! - increment_end fan-out 对 (fetched − skip) **确定性有序**（HX-C010，ChannelId: Ord）。
//!
//! 注：续拉（events.len()>=500 → no_change）已由 e3_continuation_test 覆盖（UC-3）。

mod common;
use common::*;

use helix_core::effect::{Effect, StorageOp, TransportId};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use helix_im::state::ChannelId;

fn latest_post_body(channel_id: u64, msg_id: &str, event_seq: u64) -> bytes::Bytes {
    use serde_json::json;
    let body = json!({
        "status": "SUCCESS",
        "data": {
            "posts": [{
                "id": msg_id,
                "channelId": cid(channel_id),
                "eventSeq": event_seq,
                "userId": "other-user",
                "type": "TEXT",
                "message": "reload body",
                "simpleMessage": "reload body",
                "viewers": ["all"],
                "mentions": [],
                "expediteMap": {},
                "createAt": 1_700_000_100_i64
            }]
        }
    });
    bytes::Bytes::from(serde_json::to_vec(&body).unwrap())
}

fn channel_row_reply(channel_id: u64, unread_count: i64, msg_id: &str) -> bytes::Bytes {
    use serde_json::json;
    let row = json!([{
        "id": cid(channel_id),
        "unread_count": unread_count,
        "mention_count": 0,
        "urgent_count": 0,
        "mention_list": [],
        "urgent_post_list": [],
        "unread_post_id": msg_id,
        "last_root_post_at": 1_700_000_100_i64,
    }]);
    bytes::Bytes::from(serde_json::to_vec(&row).unwrap())
}

fn is_message_delete_effect(effect: &Effect) -> bool {
    let ops: &[StorageOp] = match effect {
        Effect::Persist { ops, .. } => ops,
        Effect::PersistFire { ops } => ops,
        _ => return false,
    };
    ops.iter().any(|op| {
        let op = format!("{op:?}");
        op.contains("DeleteWhere") || (op.contains("BatchDelete") && op.contains("message"))
    })
}

/// 回归锚: S6 —— too_long 覆盖式重拉，Helix 保留本地 message 历史。
///
/// 本测试锁定：无 message delete + cursor reset + emit + getLatestPost reload/upsert。
#[test]
fn s6_too_long_preserves_messages_and_emits_reload() {
    let mut h = Harness::new(&[1]);
    // 大缺口（seq=5 / cursor=0 / gap=5 > 阈值）→ **立即** backfill resync（根治
    // fix/im-sync-cursor-catchup）：sync Http 在 ingest 当步即发出，不再等 1s gate timer。
    let gate = h.step(inbound_event(1, 5, 1), 1000);
    let sync_corr = gate
        .iter()
        .find_map(|e| match e {
            EffectKind::Http(c) => Some(*c),
            _ => None,
        })
        .expect("大缺口 ingest 当步即发 sync Http（立即 backfill）");

    // 服务端回报 too_long（resetTo=100）。
    h.step(
        Tick::PortReply {
            corr: Correlation::from_raw(sync_corr),
            // dev-zlc ADR-007：handle_sync_reply 解 HTTP 信封；enveloped() 复刻 driver 回灌字节。
            outcome: PortOutcome::Ok(ReplyBytes(enveloped(sync_reply_too_long(1, 100)))),
        },
        2100,
    );

    // 1. 须 emit im:sync:too_long。
    let saw_emit = h.sink.as_slice().iter().any(|e| {
        matches!(e, Effect::Emit { event }
            if String::from_utf8_lossy(event.0.as_ref()).contains("im:sync:too_long"))
    });
    assert!(saw_emit, "too_long 须 emit im:sync:too_long");

    // 1b. emit payload 须含 resetTo + channelId + drop_and_reload。
    let emit_has_resetto = h.sink.as_slice().iter().any(|e| {
        matches!(e, Effect::Emit { event } if {
            let s = String::from_utf8_lossy(event.0.as_ref());
            s.contains("im:sync:too_long")
                && s.contains("\"resetTo\":100")
                && s.contains("\"channelId\"")
                && s.contains("\"mode\":\"drop_and_reload\"")
        })
    });
    assert!(
        emit_has_resetto,
        "too_long emit 须带 resetTo=100 + channelId + drop_and_reload"
    );

    // 1c. Critical：不能只 drop/reset + emit，Helix 必须主动发起首屏覆盖重拉。
    let reload_http = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Http { corr, req } if req.url.ends_with("/posts/getLatestPost") => {
                let body: serde_json::Value =
                    serde_json::from_slice(req.body.as_ref().expect("reload body").as_ref())
                        .expect("reload body json");
                Some((corr.raw(), req.url.clone(), body))
            }
            _ => None,
        })
        .expect("too_long 后必须由 Helix 主动 Effect::Http getLatestPost 重拉首屏");
    assert!(reload_http.1.ends_with("/posts/getLatestPost"));
    assert_eq!(reload_http.2["channelId"], cid(1));
    assert_eq!(reload_http.2["timestamp"], 0);

    // 2. 禁止先清 message。reload HTTP 失败时旧历史必须仍可读。
    let saw_message_delete = h.sink.as_slice().iter().any(is_message_delete_effect);
    assert!(
        !saw_message_delete,
        "S6：too_long 不得物理删除本地 message，got {:?}",
        h.sink.as_slice()
    );

    // 3. cursor 复位 reset_to-1 = 99。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(99),
        "too_long 须复位 cursor 到 reset_to-1=99"
    );

    // 4. 首屏回包后必须可落库，并在 channel 写后读回后 emit 覆盖 Dialog 投影。
    h.step(
        port_reply_with_body(
            reload_http.0,
            enveloped(latest_post_body(1, "reload-msg", 100)),
        ),
        2200,
    );
    let reload_persist = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { corr, ops }
                if ops.iter().any(
                    |op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"),
                ) && ops.iter().any(|op| matches!(op, StorageOp::GuardedBump(_)))
                    && ops.iter().any(
                        |op| matches!(op, StorageOp::Get(spec) if spec.table == "channel"),
                    ) =>
            {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("getLatestPost 回包后必须落 message + bump channel + Get(channel)");

    h.step(
        port_reply_with_body(reload_persist, channel_row_reply(1, 1, "reload-msg")),
        2300,
    );
    let reload_patch = h.sink.as_slice().iter().any(|e| {
        matches!(e, Effect::Emit { event } if {
            let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
            value["event"] == "im:channel:update-by-post"
                && value["data"]["source"] == "too_long_reload"
                && value["data"]["dialogPatch"]["unreadCount"] == 1
        })
    });
    assert!(reload_patch, "重拉落库写后读回后必须 emit 覆盖 DialogPatch");
}

#[test]
fn s6_too_long_reload_failure_keeps_message_history() {
    let mut h = Harness::new(&[1]);
    let sync_corr = h
        .step(inbound_event(1, 5, 1), 1_000)
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(*corr),
            _ => None,
        })
        .expect("大缺口必须触发 sync Http");

    h.step(
        Tick::PortReply {
            corr: Correlation::from_raw(sync_corr),
            outcome: PortOutcome::Ok(ReplyBytes(enveloped(sync_reply_too_long(1, 100)))),
        },
        2_100,
    );
    let reload_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, req } if req.url.ends_with("/posts/getLatestPost") => {
                Some(corr.raw())
            }
            _ => None,
        })
        .expect("too_long 必须发起覆盖式 reload");

    h.step(port_reply_err(reload_corr), 2_200);
    assert!(
        !h.sink.as_slice().iter().any(is_message_delete_effect),
        "reload 失败不得补发 message delete，旧本地历史必须保留"
    );
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(99),
        "reload 失败不撤销 too_long 的 cursor reset"
    );
}

/// 回归锚: S6 —— increment_end(None global) fan-out 保留服务端交付顺序。
/// 输入帧顺序本身可确定性重放；不得再按 channelId 排序，把最近活跃频道推到历史洪峰末尾。
#[test]
fn s6_increment_end_fanout_preserves_delivery_order() {
    use helix_im::state::Seq;
    use helix_im::sync_session::IncrementChannel;

    const CH_HIGH: &str = "zzzzzz0000000000000000001e";
    const CH_LOW: &str = "aaaaaa00000000000000000002";
    const CH_MID: &str = "mmmmmm00000000000000000011";

    fn ch(raw: &str) -> ChannelId {
        assert_eq!(raw.len(), 26);
        ChannelId::from_str(raw).expect("test channel id fixture must be valid Id26")
    }

    fn inc(channel_id: ChannelId, seq: u64) -> IncrementChannel {
        IncrementChannel {
            channel_id,
            last_event_seq: Seq(seq),
            need_sync: true,
            raw: bytes::Bytes::from_static(b"{}"),
        }
    }
    /// 从 sink 取每个 sync/notify 的 body.cursors[0].channelId（fan-out 目标顺序）。
    fn fanout_channels(sink: &helix_core::EffectSink) -> Vec<String> {
        sink.as_slice()
            .iter()
            .filter_map(|e| match e {
                Effect::Http { req, .. } if req.url.ends_with("/channel/sync/notify") => {
                    let body = req.body.as_ref()?;
                    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
                    v["cursors"][0]["channelId"].as_str().map(str::to_string)
                }
                _ => None,
            })
            .collect()
    }

    let mut h = Harness::new(&[]);
    h.start(0);
    h.step(Tick::Connected(TransportId::from_raw(0)), 100);
    h.step(hello_inbound("CONN"), 200);

    // 服务端按新鲜度交付 high, low, mid；Helix 必须保持该顺序进入有界窗口。
    h.sink.clear();
    h.module.ingest_increment(&inc(ch(CH_HIGH), 5), &mut h.sink);
    h.module.ingest_increment(&inc(ch(CH_LOW), 5), &mut h.sink);
    h.module.ingest_increment(&inc(ch(CH_MID), 5), &mut h.sink);

    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);

    let order = fanout_channels(&h.sink);
    let expected = vec![CH_HIGH.to_string(), CH_LOW.to_string(), CH_MID.to_string()];
    assert_eq!(
        order, expected,
        "S6：increment_end fan-out 须保留服务端确定性交付顺序，got {order:?}"
    );
}
