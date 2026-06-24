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
//! 不误种服务端水位）→ gate 接管：event_seq==1 立即 apply（落库 + emit + cursor→1）；
//! event_seq>1 缺口 → buffer + arm 1s gate → sync 回拉前置消息，首条不丢。
//!
//! 可证伪（HX-C011）：
//! - 陌生 channel post 不落库 → 无 message BatchUpsert FAIL
//! - 陌生 channel post 不 emit → 无 Emit FAIL
//! - 陌生 channel 未注册 → cursor_for 仍 None FAIL

mod common;
use common::*;

use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::InboundBytes;
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

/// 在 sink 里找 message 表 BatchUpsert（PersistFire / Persist）。
fn has_message_upsert(h: &Harness) -> bool {
    h.sink.as_slice().iter().any(|e| {
        let ops = match e {
            Effect::PersistFire { ops } => ops,
            Effect::Persist { ops, .. } => ops,
            _ => return false,
        };
        ops.iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "message"))
    })
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
    // ② emit im:post:received（前端可渲染）。
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Emit)),
        "陌生 channel post 必须 emit（im:post:received），got: {effects:?}"
    );
    // ③ 自动注册 + cursor 推进到 event_seq（seq==cursor+1==1 → 立即 apply）。
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
