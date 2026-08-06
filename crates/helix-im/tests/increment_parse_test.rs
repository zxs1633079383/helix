//! B-rest 分片A：increment_channel 真帧生产路径 + event_seq 三级 fallback 回放测试。
//!
//! 验证（设计文档 §1.3 / §1.5）：
//! - increment_channel 真帧（顶层 action + data{id,lastEventSeq,needSync}）
//!   → Tick::Inbound 走 WS registry，注册 channel；global end 同批 Persist Ok 后只发 typed ready。
//! - event_seq 三级 fallback：① data.event_seq ② props.channel_event_seq ③ 顶层 channel_event_seq
//!   各命中一级 + legacy data.seq 兼容（无 action 帧）。
//! - 仅 id 非法（缺 / 非 26 字符）→ 生产路径 no-op / helper None；
//!   lastEventSeq 缺失/非数值 → **仍解析**（cursor 种子回退 0，P6-realframe 真帧兼容）。
//!
//! 回归锚: B-rest 分片A（increment_channel 真 wire 解析 + event_seq 三级 fallback）

mod common;
use common::*;

use helix_core::effect::Effect;
use helix_core::tick::InboundBytes;
use helix_core::Module;
use helix_core::Tick;
use helix_im::parser::parse_increment_channel;
use helix_im::state::{ChannelId, Seq};

/// 26 字符 ASCII channelId（base32 形态占位，长度恰 26）。
const CH26: &str = "00000000000000000000000001";

fn complete_increment_batch(h: &mut Harness) {
    let end = serde_json::json!({
        "v": 1,
        "action": "increment_channel_end",
        "data": {},
        "seq": 10
    });
    h.module
        .handle(
            &Tick::Inbound(InboundBytes(serde_json::to_vec(&end).unwrap().into())),
            110,
            &mut h.sink,
        )
        .expect("increment_channel_end should dispatch");
    let persist_corr = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("global increment end must create a persistence barrier");
    h.step(port_reply_with_body(persist_corr, bytes::Bytes::new()), 120);
}

// ─── ① increment_channel 真帧 → 生产路径 registry handler ───────────────────

#[test]
fn increment_channel_real_frame_dispatches_through_registry() {
    // 按 §1.3 手造真 Go increment_channel 帧（顶层 action + data 含 ToMap 字段）。
    let frame = serde_json::json!({
        "v": 1,
        "action": "increment_channel",
        "data": {
            "id": CH26,
            "teamId": "team_x",
            "displayName": "general",
            "lastEventSeq": 4242,
            "unreadCount": 3,
            "needSync": true,
            "mentionList": [],
            "urgentPostList": []
        },
        "broadcast": {},
        "seq": 9
    });
    let bytes = serde_json::to_vec(&frame).unwrap();
    let mut h = Harness::new_with_identity(&[], "viewer-1", "team_x", "", "", "");

    h.module
        .handle(&Tick::Inbound(InboundBytes(bytes.into())), 100, &mut h.sink)
        .expect("increment_channel should dispatch through WS registry");

    let channel_id = ChannelId::from_str(CH26).unwrap();
    // 修 todo#1：冷启动真新 channel 内存 cursor 种子 = 0（本地确认水位），不是 lastEventSeq(4242,
    // 服务端水位)。lastEventSeq 只进 increment_target。误种子 → from_seq=max → 离线 backfill 永不触发。
    assert_eq!(
        h.module.cursor_for(channel_id),
        Some(0),
        "increment_channel cold-start should seed cursor 0, not lastEventSeq (server watermark)"
    );
    assert!(
        h.module.increment_fetched_contains(channel_id),
        "increment_channel should record fetched channel for increment_end"
    );
    assert!(
        h.sink
            .as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Emit { .. })),
        "increment projection must wait for the batch persistence barrier"
    );
    complete_increment_batch(&mut h);
    let emitted = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(String::from_utf8_lossy(event.0.as_ref()).to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        emitted
            .iter()
            .any(|body| body.contains("im:channel-sync-ready")),
        "Persist Ok 后必须发布 typed channel-sync-ready，got {:?}",
        emitted
    );
    assert!(
        emitted
            .iter()
            .all(|body| !body.contains("im:channel:increment")
                && !body.contains("im:channels:loaded")),
        "channel sync 不得恢复 raw increment/loaded，got {:?}",
        emitted
    );
}

#[test]
fn increment_channel_needsync_defaults_true_when_absent() {
    // §1.3：needSync 缺省 true（多拉不漏，防前端永久 loading）。
    let data = serde_json::json!({ "id": CH26, "lastEventSeq": 1 });
    let ic = parse_increment_channel(&data).expect("valid increment data");
    assert!(ic.need_sync, "needSync 缺失应缺省 true");
    assert_eq!(ic.last_event_seq, Seq(1));
}

#[test]
fn increment_channel_needsync_false_respected() {
    let data = serde_json::json!({ "id": CH26, "lastEventSeq": 7, "needSync": false });
    let ic = parse_increment_channel(&data).expect("valid increment data");
    assert!(!ic.need_sync);
}

// ─── ③ 非法 data → Ignored / None（零信任，不 panic） ────────────────────────

#[test]
fn increment_channel_missing_id_returns_ignored() {
    let frame = serde_json::json!({
        "action": "increment_channel",
        "data": { "lastEventSeq": 1, "needSync": true }
    });
    let bytes = serde_json::to_vec(&frame).unwrap();
    let mut h = Harness::new(&[]);

    h.module
        .handle(&Tick::Inbound(InboundBytes(bytes.into())), 100, &mut h.sink)
        .expect("invalid increment_channel should stay no-op");

    assert!(
        h.sink.as_slice().is_empty(),
        "invalid increment_channel should not produce effects"
    );
}

#[test]
fn increment_channel_non26_id_returns_none() {
    // id 非 26 字符 → ChannelId::from_str None → parse 返 None。
    let data = serde_json::json!({ "id": "tooshort", "lastEventSeq": 1 });
    assert!(parse_increment_channel(&data).is_none());

    let frame = serde_json::json!({
        "action": "increment_channel",
        "data": { "id": "tooshort", "lastEventSeq": 1 }
    });
    let bytes = serde_json::to_vec(&frame).unwrap();
    let mut h = Harness::new(&[]);

    h.module
        .handle(&Tick::Inbound(InboundBytes(bytes.into())), 100, &mut h.sink)
        .expect("invalid increment_channel should stay no-op");

    assert!(
        h.sink.as_slice().is_empty(),
        "invalid increment_channel should not produce effects"
    );
}

#[test]
fn increment_channel_missing_last_event_seq_seeds_zero() {
    // 回归锚: P6-realframe —— 真 Go（legacy mattermost 9.7.3 GetMaxSeq 降级）increment 帧
    // **不带** lastEventSeq。修复前 parser 用 `?` 早退丢整帧（实测 650 帧落库 0 行）；修复后
    // lastEventSeq 缺失 → cursor 种子回退 Seq(0)，帧仍解析、channel 仍 upsert（对齐现网
    // types.rs:257 Option<i64> + channel_state.rs:345 unwrap_or(0)）。
    let data = serde_json::json!({ "id": CH26, "needSync": true });
    let ic = parse_increment_channel(&data).expect("missing lastEventSeq must still parse");
    assert_eq!(
        ic.last_event_seq,
        Seq(0),
        "缺 lastEventSeq → cursor 种子回退 0"
    );
    assert!(ic.need_sync);
}

#[test]
fn increment_channel_last_event_seq_non_numeric_seeds_zero() {
    // lastEventSeq 是字符串（非 int64）→ as_u64 None → 缺省回退 Seq(0)，**不**丢帧。
    let data = serde_json::json!({ "id": CH26, "lastEventSeq": "not_a_number" });
    let ic = parse_increment_channel(&data).expect("non-numeric lastEventSeq must still parse");
    assert_eq!(ic.last_event_seq, Seq(0));
}

#[test]
fn increment_channel_no_last_event_seq_still_dispatches() {
    // 回归锚: P6-realframe —— increment_channel 帧无 lastEventSeq → 仍走 registry：
    // 注册 channel（cursor 种子 0）并进入待提交批次；Persist Ok 后才 emit。
    // 注：path1 全量 upsert 落库由 Phase C 的 apply_increment(path1+成员) 接入，此处只验
    // 「帧不被丢弃 + channel 注册 + cursor 种子 0」（parser lastEventSeq Optional 的端到端证据）。
    let frame = serde_json::json!({
        "v": 1,
        "action": "increment_channel",
        "data": {
            "id": CH26,
            "displayName": "no-seq-channel",
            "needSync": true
            // 故意不带 lastEventSeq（模拟 GetMaxSeq 降级真帧）
        },
        "seq": 1
    });
    let bytes = serde_json::to_vec(&frame).unwrap();
    let mut h = Harness::new_with_identity(&[], "viewer-1", "team_x", "", "", "");

    h.module
        .handle(&Tick::Inbound(InboundBytes(bytes.into())), 100, &mut h.sink)
        .expect("increment_channel without lastEventSeq must dispatch, not drop");

    let channel_id = ChannelId::from_str(CH26).unwrap();
    assert_eq!(
        h.module.cursor_for(channel_id),
        Some(0),
        "缺 lastEventSeq → channel 仍注册，cursor 种子 0"
    );
    assert!(
        h.module.increment_fetched_contains(channel_id),
        "缺 lastEventSeq 的帧仍应进入 increment batch"
    );
    complete_increment_batch(&mut h);
    let emitted = h
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => Some(String::from_utf8_lossy(event.0.as_ref()).to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        emitted
            .iter()
            .any(|body| body.contains("im:channel-sync-ready")),
        "缺 lastEventSeq → Persist Ok 后仍应发布 typed ready，got {:?}",
        emitted
    );
    assert!(
        emitted
            .iter()
            .all(|body| !body.contains("im:channel:increment")
                && !body.contains("im:channels:loaded")),
        "缺 lastEventSeq → 不得恢复 raw increment/loaded，got {:?}",
        emitted
    );
}
