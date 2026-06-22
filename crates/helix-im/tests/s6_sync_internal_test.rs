//! sync-v2 §S6 sync 引擎内化 对齐补全回放测试。
//!
//! 回归锚: S6（SPEC-sync-v2-full-replacement.md §S6）
//!
//! 补齐既有 gate_test / increment_flow_test 未显式锁定的契约：
//! - too_long **不在 Rust DELETE message**（只 emit + 复位 cursor，清表交前端）；
//! - increment_end fan-out 对 (fetched − skip) **确定性有序**（HX-C010，ChannelId: Ord）。
//!
//! 注：续拉（events.len()>=500 → no_change）已由 e3_continuation_test 覆盖（UC-3）。

mod common;
use common::*;

use helix_core::effect::{Effect, StorageOp, TransportId};
use helix_core::tick::{PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use helix_im::state::ChannelId;

/// 回归锚: S6 —— too_long 只 emit + 复位 cursor，**helix 侧不 DELETE message**（清表交前端）。
///
/// 结构性保证：`StorageOp` 枚举无 Delete 变体，类型层即排除「Rust 侧删消息」。本测试断言
/// too_long 批次产出的 Persist/PersistFire ops 中无任何 message 表写（不擦库），只 emit。
#[test]
fn s6_too_long_emits_but_does_not_delete_message() {
    let mut h = Harness::new(&[1]);
    let gate = h.step(inbound_event(1, 5, 1), 1000);
    let gate_timer = gate
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .expect("gate timer");
    let sync = h.step(timer_tick(gate_timer), 2001);
    let sync_corr = sync
        .iter()
        .find_map(|e| match e {
            EffectKind::Http(c) => Some(*c),
            _ => None,
        })
        .expect("sync Http corr");

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

    // 1b. emit payload 须含 resetTo（=服务端 reset_to 100）+ channelId（对齐现网 post.rs:918，
    // 前端 onRustTooLong 按此做 cursor=resetTo-1 记账 + getLatestPost 重拉）。删 resetTo → FAIL。
    let emit_has_resetto = h.sink.as_slice().iter().any(|e| {
        matches!(e, Effect::Emit { event } if {
            let s = String::from_utf8_lossy(event.0.as_ref());
            s.contains("im:sync:too_long")
                && s.contains("\"resetTo\":100")
                && s.contains("\"channelId\"")
        })
    });
    assert!(
        emit_has_resetto,
        "too_long emit 须带 resetTo=100 + channelId（前端 cursor 记账契约，对齐现网）"
    );

    // 2. 不得产生任何 message 表写（无 DELETE 语义；StorageOp 也无 Delete 变体）。
    for e in h.sink.as_slice() {
        let ops: &[StorageOp] = match e {
            Effect::Persist { ops, .. } => ops,
            Effect::PersistFire { ops } => ops,
            _ => continue,
        };
        for op in ops {
            let touches_message = match op {
                StorageOp::BatchUpsert(s) => s.table == "message",
                StorageOp::BatchUpdate(s) => s.table == "message",
                StorageOp::GuardedBump(s) => s.table == "message",
                StorageOp::MonotonicUpsert(s) => s.table == "message",
                StorageOp::Scan(s) => s.table == "message",
                StorageOp::Get(s) => s.table == "message",
                StorageOp::BatchDelete(s) => s.table == "message",
            };
            assert!(
                !touches_message,
                "S6：too_long 不得在 Rust 侧写/删 message 表（清表交前端），got op {op:?}"
            );
        }
    }

    // 3. cursor 复位 reset_to-1 = 99。
    assert_eq!(
        h.module.cursor_for(ChannelId::from_test_u64(1)),
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

    fn inc(ch: u64, seq: u64) -> IncrementChannel {
        IncrementChannel {
            channel_id: ChannelId::from_test_u64(ch),
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
    h.module.ingest_increment(&inc(30, 5), &mut h.sink);
    h.module.ingest_increment(&inc(2, 5), &mut h.sink);
    h.module.ingest_increment(&inc(17, 5), &mut h.sink);

    h.sink.clear();
    h.module.ingest_increment_end(None, &mut h.sink);

    let order = fanout_channels(&h.sink);
    // from_test_u64 零填充十进制 → 字典序==数值序；sorted = [2, 17, 30]。
    let expected = vec![cid(2), cid(17), cid(30)];
    assert_eq!(
        order, expected,
        "S6 HX-C010：increment_end fan-out 须按 ChannelId 升序确定性发 sync，got {order:?}"
    );
}
