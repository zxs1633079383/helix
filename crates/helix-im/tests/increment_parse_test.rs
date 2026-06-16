//! B-rest 分片A：increment_channel 真帧解析 + event_seq 三级 fallback 回放测试。
//!
//! 验证（设计文档 §1.3 / §1.5）：
//! - increment_channel 真帧（顶层 action + data{id,lastEventSeq,needSync}）
//!   → classify_frame 返 FrameClass::Increment，字段逐一对齐，raw 透传 data。
//! - event_seq 三级 fallback：① data.event_seq ② props.channel_event_seq ③ 顶层 channel_event_seq
//!   各命中一级 + legacy data.seq 兼容（无 action 帧）。
//! - 非法 data（缺 id / id 非 26 字符 / lastEventSeq 非数值）→ Ignored/None（零信任，不 panic）。
//!
//! 回归锚: B-rest 分片A（increment_channel 真 wire 解析 + event_seq 三级 fallback）

use helix_im::parser::{classify_frame, parse_increment_channel, FrameClass};
use helix_im::state::{ChannelId, Seq};

/// 26 字符 ASCII channelId（base32 形态占位，长度恰 26）。
const CH26: &str = "00000000000000000000000001";

// ─── ① increment_channel 真帧 → FrameClass::Increment 且字段正确 ──────────────

#[test]
fn increment_channel_real_frame_classifies_with_correct_fields() {
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

    match classify_frame(&bytes) {
        FrameClass::Increment(ic) => {
            assert_eq!(ic.channel_id, ChannelId::from_str(CH26).unwrap());
            assert_eq!(ic.last_event_seq, Seq(4242));
            assert!(ic.need_sync);
            // raw 透传整个 data（供 emit im:channel:increment）。
            let raw_v: serde_json::Value = serde_json::from_slice(&ic.raw).unwrap();
            assert_eq!(raw_v["displayName"], "general");
            assert_eq!(raw_v["id"], CH26);
        }
        other => panic!("expected Increment, got {:?}", other),
    }
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

// ─── ② event_seq 三级 fallback：三用例各命中一级 ──────────────────────────────

/// 构造 legacy 数值事件帧（无 action → parse_inbound 路径）。
fn event_envelope_from(frame: &serde_json::Value) -> helix_im::sync_session::EventEnvelope {
    let bytes = serde_json::to_vec(frame).unwrap();
    helix_im::parser::parse_inbound(&bytes).expect("event frame should parse")
}

#[test]
fn event_seq_fallback_level1_data_event_seq() {
    // ① 顶层 data.event_seq 权威（真 Go int64 数值，非 26 字符 id）。
    let frame = serde_json::json!({
        "data": { "channel_id": CH26, "event_seq": 111, "event_type": 1 }
    });
    let ev = event_envelope_from(&frame);
    assert_eq!(ev.seq, Seq(111));
    assert_eq!(ev.channel_id, ChannelId::from_str(CH26).unwrap());
}

#[test]
fn event_seq_fallback_level2_props_channel_event_seq() {
    // ② per-post props.channel_event_seq（posted 冗余），data.event_seq 缺失时命中。
    let frame = serde_json::json!({
        "data": {
            "channel_id": CH26,
            "event_type": 1,
            "props": { "channel_event_seq": 222 }
        }
    });
    let ev = event_envelope_from(&frame);
    assert_eq!(ev.seq, Seq(222));
}

#[test]
fn event_seq_fallback_level3_top_level_channel_event_seq() {
    // ③ 顶层 channel_event_seq（legacy），前两级缺失时命中。
    let frame = serde_json::json!({
        "channel_event_seq": 333,
        "data": { "channel_id": CH26, "event_type": 1 }
    });
    let ev = event_envelope_from(&frame);
    assert_eq!(ev.seq, Seq(333));
}

#[test]
fn event_seq_legacy_data_seq_still_compatible() {
    // legacy data.seq 兼容（既有抽象 wire / 测试帧，无三级字段时兜底）。
    let frame = serde_json::json!({
        "data": { "channel_id": CH26, "seq": 444, "event_type": 1 }
    });
    let ev = event_envelope_from(&frame);
    assert_eq!(ev.seq, Seq(444));
}

#[test]
fn event_seq_priority_level1_over_others() {
    // 三级同时存在时 data.event_seq 优先（权威）。
    let frame = serde_json::json!({
        "channel_event_seq": 333,
        "data": {
            "channel_id": CH26,
            "event_seq": 111,
            "event_type": 1,
            "props": { "channel_event_seq": 222 }
        }
    });
    let ev = event_envelope_from(&frame);
    assert_eq!(ev.seq, Seq(111), "data.event_seq 应优先于 props / 顶层");
}

// ─── ③ 非法 data → Ignored / None（零信任，不 panic） ────────────────────────

#[test]
fn increment_channel_missing_id_returns_ignored() {
    let frame = serde_json::json!({
        "action": "increment_channel",
        "data": { "lastEventSeq": 1, "needSync": true }
    });
    let bytes = serde_json::to_vec(&frame).unwrap();
    assert_eq!(classify_frame(&bytes), FrameClass::Ignored);
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
    assert_eq!(classify_frame(&bytes), FrameClass::Ignored);
}

#[test]
fn increment_channel_missing_last_event_seq_returns_none() {
    let data = serde_json::json!({ "id": CH26, "needSync": true });
    assert!(parse_increment_channel(&data).is_none());
}

#[test]
fn increment_channel_last_event_seq_non_numeric_returns_none() {
    // lastEventSeq 是字符串（非 int64）→ as_u64 None → None。
    let data = serde_json::json!({ "id": CH26, "lastEventSeq": "not_a_number" });
    assert!(parse_increment_channel(&data).is_none());
}
