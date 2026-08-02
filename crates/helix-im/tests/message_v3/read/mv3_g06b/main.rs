// MessageV3 MV3-G06b：最小 post_read ID delta 与 G14 legacy 隔离。

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp, TransportId};
use helix_core::tick::{InboundBytes, Tick};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{cid, sid, Harness};

/// 构造真实 Go `post_read` WS authority 帧。
fn inbound_post_read(data: Value) -> Tick {
    let frame = serde_json::to_vec(&json!({"action": "post_read", "data": data}))
        .expect("post_read frame serializes");
    Tick::Inbound(InboundBytes(Bytes::from(frame)))
}

/// 解出当前 step 的全部 MessageV3 事件，不重建 producer payload。
fn emitted_events(harness: &Harness) -> Vec<Value> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            serde_json::from_slice(event.0.as_ref()).ok()
        })
        .collect()
}

/// 判断当前 step 是否包含旧 G14 Persist，供隔离断言使用。
fn persist_ops(harness: &Harness) -> Option<Vec<StorageOp>> {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } => Some(ops.clone()),
            _ => None,
        })
}

/// 构造 G06b 的四字段 canonical ID delta。
fn minimal_delta(channel_id: &str, post_id: &str, snapshot_id: &str, reader_id: &str) -> Value {
    json!({
        "channelId": channel_id,
        "postId": post_id,
        "snapshotId": snapshot_id,
        "readerUserId": reader_id
    })
}

/// 构造 G14 legacy 位图 authority，验证旧链仍保留 Persist 屏障。
fn legacy_authority(channel_id: &str, post_id: &str) -> Value {
    json!({
        "channelId": channel_id,
        "postId": post_id,
        "authorUserId": "g14-author",
        "readerUserId": "g14-reader",
        "snapshotId": "g14-snapshot",
        "memberIds": ["g14-reader", "g14-other"],
        "readMap": "10",
        "createAt": 1_700_001_400_000_i64,
        "updateAt": 1_700_001_400_020_i64
    })
}

/// public client_api 只把 `postIds` 一次归一到 `{channel_id,posts}`，并写出唯一 Go body。
#[test]
fn g06b_public_command_normalizes_once_and_writes_posts_wire() {
    let channel_id = cid(11);
    let post_ids = vec![sid(11), sid(12)];
    let command = helix_im::client_api::build_command(
        "im_post_read",
        serde_json::to_vec(&json!({"channelId": channel_id, "postIds": post_ids}))
            .expect("command serializes")
            .as_slice(),
    )
    .expect("canonical post read command");
    let effects = helix_im::outbound::handle_outbound(
        command.name.as_ref(),
        command.payload.as_ref(),
        "http://im/api/cses",
        "http://default",
        Some("conn-g06b"),
        helix_core::Correlation::from_raw(1),
    )
    .expect("post/read outbound");
    let Effect::Http { req, .. } = &effects[0] else {
        panic!("post/read must emit exactly one HTTP effect")
    };
    let body: Value = serde_json::from_slice(req.body.as_ref().expect("body")).expect("body JSON");
    assert_eq!(req.url, "http://im/api/cses/post/read");
    assert_eq!(
        body,
        json!({"channelId": cid(11), "posts": [sid(11), sid(12)]})
    );
}

/// public postRead 拒绝空、临时、非字符串、错误频道和区间字段。
#[test]
fn g06b_public_command_rejects_noncanonical_inputs() {
    let channel_id = cid(11);
    let cases = [
        json!({"channelId": channel_id.clone(), "postIds": []}),
        json!({"channelId": channel_id.clone(), "postIds": ["helix_tmp_1"]}),
        json!({"channelId": channel_id.clone(), "postIds": [1]}),
        json!({"channelId": "temporary-channel", "postIds": [sid(11)]}),
        json!({"channelId": channel_id, "startTime": 1, "endTime": 2}),
    ];
    for payload in cases {
        let encoded = serde_json::to_vec(&payload).expect("payload serializes");
        assert!(
            helix_im::client_api::build_command("im_post_read", &encoded).is_err(),
            "invalid public input must fail: {payload}"
        );
    }
}

/// 最小 G06b 帧只产生一次同字段 `im:post:read` ID delta，不落库、不推进 cursor。
#[test]
fn g06b_minimal_frame_emits_one_id_delta_without_persist() {
    let channel_id = cid(11);
    let post_id = sid(11);
    let mut harness = Harness::new_with_identity(&[11], "g06b-reader", "g06b-company", "", "", "");
    harness.step(
        inbound_post_read(minimal_delta(
            &channel_id,
            &post_id,
            "snapshot-1",
            "g06b-reader",
        )),
        1_700_001_406_000,
    );
    let events = emitted_events(&harness);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["event"], "im:post:read");
    assert_eq!(
        events[0]["data"],
        json!({
            "channelId": channel_id,
            "postId": post_id,
            "snapshotId": "snapshot-1",
            "readerUserId": "g06b-reader"
        })
    );
    assert!(persist_ops(&harness).is_none());
    assert!(harness
        .sink
        .as_slice()
        .iter()
        .all(|effect| matches!(effect, Effect::Emit { .. })));
}

/// 缺字段或 forbidden 字段的混合帧必须终止，不得降级到旧位图 Persist。
#[test]
fn g06b_missing_or_forbidden_fields_are_noop() {
    let channel_id = cid(11);
    let post_id = sid(11);
    let cases = [
        json!({"channelId": channel_id, "postId": post_id, "readerUserId": "reader"}),
        json!({"channelId": cid(11), "postId": sid(11), "snapshotId": "snapshot"}),
        json!({"channelId": cid(11), "postId": sid(11), "snapshotId": "snapshot", "readerUserId": "reader", "readBits": "1"}),
        json!({"channelId": cid(11), "postId": sid(11), "snapshotId": "snapshot", "readerUserId": "reader", "memberIds": ["reader"]}),
    ];
    for data in cases {
        let mut harness = Harness::new_with_identity(&[11], "reader", "company", "", "", "");
        harness.step(inbound_post_read(data), 1_700_001_406_001);
        assert!(
            harness.sink.as_slice().is_empty(),
            "invalid delta must be no-op"
        );
    }
}

/// 同一四元组 O(1) 去重，但不同 snapshot/reader 必须继续发布独立增量。
#[test]
fn g06b_replay_dedupes_only_same_snapshot_and_reader() {
    let channel_id = cid(11);
    let post_id = sid(11);
    let mut harness = Harness::new_with_identity(&[11], "reader-a", "company", "", "", "");
    let first = minimal_delta(&channel_id, &post_id, "snapshot-1", "reader-a");
    harness.step(inbound_post_read(first.clone()), 1_700_001_406_010);
    assert_eq!(emitted_events(&harness).len(), 1);
    harness.step(inbound_post_read(first), 1_700_001_406_011);
    assert!(harness.sink.as_slice().is_empty());

    harness.step(
        inbound_post_read(minimal_delta(
            &channel_id,
            &post_id,
            "snapshot-2",
            "reader-a",
        )),
        1_700_001_406_012,
    );
    assert_eq!(emitted_events(&harness).len(), 1);
    harness.step(
        inbound_post_read(minimal_delta(
            &channel_id,
            &post_id,
            "snapshot-1",
            "reader-b",
        )),
        1_700_001_406_013,
    );
    assert_eq!(emitted_events(&harness).len(), 1);

    harness.step(
        Tick::Disconnected(TransportId::from_raw(1)),
        1_700_001_406_014,
    );
    assert!(harness.sink.as_slice().is_empty());
    harness.step(
        inbound_post_read(minimal_delta(
            &channel_id,
            &post_id,
            "snapshot-1",
            "reader-a",
        )),
        1_700_001_406_015,
    );
    assert_eq!(
        emitted_events(&harness).len(),
        1,
        "transport reset clears G06b dedupe"
    );
}

/// G14 带 `readMap/memberIds` 的 legacy authority 仍独立进入原 Persist 链。
#[test]
fn g14_legacy_post_read_still_persists_read_bits() {
    let mut harness = Harness::new_with_identity(&[14], "g14-reader", "g14-company", "", "", "");
    harness.step(
        inbound_post_read(legacy_authority(&cid(14), &sid(14))),
        1_700_001_414_000,
    );
    let ops = persist_ops(&harness).expect("legacy G14 must persist");
    assert_eq!(ops.len(), 1);
    assert!(matches!(
        &ops[0],
        StorageOp::BatchUpdate(spec)
            if spec.table == "message"
                && spec.patch.iter().any(|(column, value)| {
                    column == "read_bits"
                        && matches!(value, helix_core::effect::SqlValue::Text(bits) if bits == "10")
                })
    ));
    assert!(emitted_events(&harness).is_empty());
}
