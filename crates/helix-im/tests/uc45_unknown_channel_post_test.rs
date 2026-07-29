//! UC-4.5 陌生 channel 在线 post 不丢帧——Round-3 偏差修复回放。
//!
//! 回归锚: UC-4.5（陌生 channel 自动注册，对齐真源）
//!
//! ## 行为真源（现网 source of truth）
//!
//! cses-client `handlers/post.rs:42-114 handle` 对陌生 channel：
//!   ① 落库 `message_service::handle_new_post` **无条件**调用（仅 `unwrap_or("unknown")` 用于日志，
//!      不门控 channel 注册）→ batch_upsert message + emit `im:post:received`；
//!   ② cursor/gate 路径 `on_ws_event` 用 `gate_states.entry(ch).or_insert_with(default)` **自动注册**
//!      陌生 channel（含惰性 seed 持久化 cursor），不丢弃。
//!
//! ## 此前 helix 偏差
//!
//! `ws/handlers/post.rs` 落库 + cursor 推进**整体门控** `if let Some(ch)=channels.get_mut(...)`，
//! 未注册 channel → 无 DB 写 + 无 emit + 不推 cursor，**整帧静默丢弃**。首次见到的新群 / 被拉入群
//! 的首条在线 push 在 increment 种 cursor 之前到达时丢消息且不可恢复。
//!
//! ## 修复后（本测试锁住）
//!
//! `entry().or_insert_with(|| Channel::new(channel_id, 0))` 自动注册（cursor=0 = 本地未确认，
//! 不误种服务端水位）→ gate 接管：event_seq==1 进入原子落库，成功后推进 cursor 并读回 emit；
//! event_seq>1 缺口 → buffer + arm 1s gate → sync 回拉前置消息，首条不丢。
//!
//! 可证伪（HX-C011）：
//! - 陌生 channel post 不落库 → 无 message BatchUpsert FAIL
//! - 陌生 channel post 不 emit → 无 Emit FAIL
//! - 陌生 channel 未注册 → cursor_for 仍 None FAIL

mod common;
use common::*;

use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, ReplyBytes};
use helix_core::Tick;

/// 构造真 Go 在线广播 `post` 帧（与 post_action_inbound_test 同源信封）。
fn post_frame(
    channel_num: u64,
    event_seq: u64,
    temporary_id: &str,
    server_id: &str,
    message: &str,
) -> Tick {
    use serde_json::json;
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "channelId": cid(channel_num),
            "event_seq": event_seq,
            "id": server_id,
            "temporaryId": temporary_id,
            "message": message,
            "type": "TEXT",
            "readBits": "100",
            "createBy": "5ffd465eaaaaaaaaaaaaaaaaaa",
            "props": { "channel_event_seq": event_seq }
        },
        "broadcast": {},
        "seq": 261
    });
    Tick::Inbound(InboundBytes(serde_json::to_vec(&payload).unwrap().into()))
}

/// 在 sink 里找 message 表 BatchUpsert，包括 MessageV3 原子提交。
fn has_message_upsert(h: &Harness) -> bool {
    h.sink.as_slice().iter().any(|e| {
        let ops = match e {
            Effect::PersistFire { ops } => ops,
            Effect::Persist { ops, .. } => ops,
            Effect::PersistAtomic { ops, .. } => ops,
            _ => return false,
        };
        ops.iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"))
    })
}

/// 确认陌生频道首条消息并回灌当前 viewer 的绝对态读回。
fn acknowledge_unknown_channel_post(harness: &mut Harness, effects: &[EffectKind]) {
    let persist_corr = effects.iter().find_map(|effect| match effect {
        EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    });
    let Some(persist_corr) = persist_corr else {
        panic!("unknown channel post must enter MessageV3 atomic commit");
    };
    let readback = harness.step(port_reply_ok(persist_corr), 1_001);
    let readback_corr = readback.iter().find_map(|effect| match effect {
        EffectKind::Persist(corr) => Some(*corr),
        _ => None,
    });
    let Some(readback_corr) = readback_corr else {
        panic!("committed post must read back current channel_member");
    };
    let last_post = serde_json::json!({
        "id": "7xq3r6nqrjrm8nydit6awaaaaa",
        "temporaryId": "tmpverify0001",
        "channelId": cid(7),
        "message": "first-post-new-group",
        "type": "TEXT",
        "createAt": 1_000
    });
    let row: Row = vec![
        ("channel_id".to_string(), SqlValue::Text(cid(7))),
        ("unread_count".to_string(), SqlValue::Integer(1)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
    ];
    harness.step(
        port_reply_with_body(readback_corr, ReplyBytes(rows_to_reply_bytes(&[row])).0),
        1_002,
    );
}

#[test]
fn unknown_channel_first_post_at_seq1_persists_and_emits() {
    // 不预注册任何 channel——模拟「首次见到的新群」首条在线 push。
    let mut h = Harness::new(&[]);
    assert_eq!(
        h.module.cursor_for(channel_id(7)),
        None,
        "前置：channel 7 尚未注册"
    );

    let effects = h.step(
        post_frame(
            7,
            1,
            "tmpverify0001",
            "7xq3r6nqrjrm8nydit6awaaaaa",
            "first-post-new-group",
        ),
        1_000,
    );

    // ① 落库（无条件，对齐真源 handle_new_post）——此前偏差下整帧丢弃 = 无 BatchUpsert。
    assert!(
        has_message_upsert(&h),
        "陌生 channel 首条 post（seq=1）必须落库 message，got: {effects:?}"
    );
    // ② durable commit + 绝对态读回后再 emit，入站当步不允许越过持久化屏障。
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::Emit)),
        "陌生 channel post 在持久化确认前不得 emit，got: {effects:?}"
    );
    assert_eq!(
        h.module.cursor_for(channel_id(7)),
        Some(0),
        "陌生 channel 应自动注册，但持久化确认前 cursor 仍为 0"
    );
    acknowledge_unknown_channel_post(&mut h, &effects);
    assert!(
        h.sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit { .. })),
        "陌生 channel post 在持久化和读回成功后必须 emit"
    );
    // ③ 自动注册 + durable commit 后 cursor 推进到 event_seq。
    assert_eq!(
        h.module.cursor_for(channel_id(7)),
        Some(1),
        "陌生 channel 应自动注册（cursor=0 seed）并推进到 event_seq=1"
    );
}

#[test]
fn unknown_channel_post_with_gap_auto_registers_and_arms_gate() {
    // 陌生 channel 首条 post 的 event_seq=5 > cursor+1（=1）→ 缺口：自动注册 + buffer + arm gate。
    let mut h = Harness::new(&[]);

    let effects = h.step(
        post_frame(
            8,
            5,
            "tmpverify0002",
            "8xq3r6nqrjrm8nydit6awaaaaa",
            "gap-first-post",
        ),
        1_000,
    );

    // 自动注册（cursor=0，未推进——缺口未填）。
    assert_eq!(
        h.module.cursor_for(channel_id(8)),
        Some(0),
        "陌生 channel 缺口帧应自动注册（cursor=0），不丢弃"
    );
    // arm 1s gate（缺口回拉路径）——此前偏差下整帧丢弃 = 无 ScheduleTimer。
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "陌生 channel 缺口 post 应 arm gate（1s 后 sync 回拉前置消息），got: {effects:?}"
    );
}
