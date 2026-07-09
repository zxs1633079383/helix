//! sync-v2 §S6 sync 引擎内化 对齐补全回放测试。
//!
//! 回归锚: S6（SPEC-sync-v2-full-replacement.md §S6）
//!
//! 补齐既有 gate_test / increment_flow_test 未显式锁定的契约：
//! - too_long 在 Rust 侧 drop channel 本地 message，并 emit drop_and_reload；
//! - increment_end fan-out 对 (fetched − skip) **确定性有序**（HX-C010，ChannelId: Ord）。
//!
//! 注：续拉（events.len()>=500 → no_change）已由 e3_continuation_test 覆盖（UC-3）。

mod common;
use common::*;

use helix_core::effect::{Effect, StorageOp, TransportId};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use helix_im::state::ChannelId;

/// 回归锚: S6 —— too_long drop-and-reload，Helix 侧按 channel 丢弃本地 message 派生视图。
///
/// 本测试断言 too_long 批次产出 `DeleteWhere(message WHERE channel_id=...)`，而不是让前端自清。
#[test]
fn s6_too_long_drops_channel_messages_and_emits_reload() {
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
                && s.contains("\"dropLocalMessages\":true")
        })
    });
    assert!(
        emit_has_resetto,
        "too_long emit 须带 resetTo=100 + channelId + drop_and_reload"
    );

    // 2. 必须按 channel 删除 message 派生行；禁止变成全表删或前端自清。
    let mut saw_message_drop = false;
    for e in h.sink.as_slice() {
        let ops: &[StorageOp] = match e {
            Effect::Persist { ops, .. } => ops,
            Effect::PersistFire { ops } => ops,
            _ => continue,
        };
        for op in ops {
            if let StorageOp::DeleteWhere(s) = op {
                if s.table == "message" && s.key_col == "channel_id" {
                    saw_message_drop = true;
                }
            }
        }
    }
    assert!(
        saw_message_drop,
        "S6：too_long 须由 Helix drop channel 本地 message，got {:?}",
        h.sink.as_slice()
    );

    // 3. cursor 复位 reset_to-1 = 99。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(99),
        "too_long 须复位 cursor 到 reset_to-1=99"
    );
}

/// 回归锚: S6 —— increment_end(None global) fan-out 对 (fetched − skip) **确定性有序**
/// （HX-C010：increment_fetched 是 HashSet，迭代序随机；必须 sort 后逐 channel 发 sync）。
#[test]
fn s6_increment_end_fanout_deterministic_order() {
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

    // 乱序 ingest（30, 2, 17）——HashSet 内部序不可预测；end 必须 sort 后发。
    h.sink.clear();
    h.module.ingest_increment(&inc(ch(CH_HIGH), 5), &mut h.sink);
    h.module.ingest_increment(&inc(ch(CH_LOW), 5), &mut h.sink);
    h.module.ingest_increment(&inc(ch(CH_MID), 5), &mut h.sink);

    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);

    let order = fanout_channels(&h.sink);
    let expected = vec![CH_LOW.to_string(), CH_MID.to_string(), CH_HIGH.to_string()];
    assert_eq!(
        order, expected,
        "S6 HX-C010：increment_end fan-out 须按 ChannelId 升序确定性发 sync，got {order:?}"
    );
}
