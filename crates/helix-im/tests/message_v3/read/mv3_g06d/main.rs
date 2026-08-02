// MessageV3 MV3-G06d：Helix 只把 Go receiver snapshot 投影为有序绝对回执。

use bytes::Bytes;
use helix_core::{Correlation, Effect};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;

use common::{command_json, enveloped, port_reply_with_body, Harness};

const CHANNEL_ID: &str = "channel-phase2-001";
const POST_ID: &str = "post-phase2-001";
const AUTHOR_ID: &str = "user-actor-444";
const READER_ID: &str = "user-receiver-445";
const UNREAD_ID: &str = "user-receiver-446";
const SNAPSHOT_ID: &str = "snapshot-phase2-read-001";

/// 取当前 step 首个 HTTP effect 的 correlation。
fn http_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("im_post_read_list must emit HTTP")
}

/// 取当前 step 首个 HTTP effect 的 URL。
fn http_url(harness: &Harness) -> &str {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some(req.url.as_str()),
            _ => None,
        })
        .expect("im_post_read_list must emit HTTP")
}

/// 从当前 step 提取 `im:post:readers` 的 data，不重建生产事件。
fn emitted_receipts(harness: &Harness) -> Vec<Value> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
            _ => None,
        })
        .filter(|payload| payload["event"] == "im:post:readers")
        .map(|payload| payload["data"].clone())
        .collect()
}

/// 直接调用 outbound registry，验证坏命令在 module harness 之前被拒绝。
fn outbound_rejects(payload: &Value) -> bool {
    helix_im::commands::handle_outbound(
        "im_post_read_list",
        &serde_json::to_vec(payload).expect("invalid payload still serializes"),
        "http://test/api/cses",
        "http://default",
        Some("conn-g06d"),
        Correlation::from_raw(1),
    )
    .is_err()
}

/// 把裸 Go 成功 body 包成 Helix `PortReply` 使用的 HTTP envelope。
fn read_list_reply(rows: Value) -> Bytes {
    enveloped(Bytes::from(
        serde_json::to_vec(&json!({ "status": "SUCCESS", "data": rows }))
            .expect("receipt body serializes"),
    ))
}

/// 在 ImModule 回放一次 `im_post_read_list` 命令并回灌指定 Go rows。
fn run_rows(rows: Value) -> Vec<Value> {
    let mut harness = Harness::new_with_identity(&[], AUTHOR_ID, "mv3-g06d", "", "", "");
    harness.step(
        command_json(
            "im_post_read_list",
            json!({ "req_id": "receipt-g06d", "post_ids": [POST_ID] }),
        ),
        1,
    );
    let corr = http_corr(&harness);
    harness.step(port_reply_with_body(corr, read_list_reply(rows)), 2);
    emitted_receipts(&harness)
}

/// 检查外发回执不含 Go 内部 bitmap、成员快照或旧 reader 别名。
fn assert_no_internal_receipt_fields(receipt: &Value) {
    for field in [
        "readBits",
        "memberIds",
        "readerIds",
        "unreadIds",
        "readers",
        "unreaders",
        "recipientUserIds",
        "snapshotVersion",
        "receiptKey",
    ] {
        assert!(
            receipt.get(field).is_none(),
            "MV3-G06d receipt must not expose {field}: {receipt}"
        );
    }
}

/// `im_post_read_list` 只映射 postIds 到 Go 单数前缀 endpoint，禁止内部字段进入 wire body。
#[test]
fn post_read_list_targets_authoritative_endpoint_and_body() {
    let mut harness = Harness::new_with_identity(&[], AUTHOR_ID, "mv3-g06d", "", "", "");
    harness.step(
        command_json(
            "im_post_read_list",
            json!({
                "req_id": "receipt-wire",
                "post_ids": [POST_ID]
            }),
        ),
        1,
    );
    assert!(http_url(&harness).ends_with("post/read/list"));
    assert_eq!(
        harness.last_http_body(),
        Some(json!({ "postIds": [POST_ID] })),
        "Helix wire body must not forward receipt internals"
    );
}

/// 缺失、空数组、非字符串数组和内部字段均在 Helix 命令边界 fail-closed。
#[test]
fn post_read_list_rejects_corrupt_or_internal_inputs() {
    for payload in [
        json!({}),
        json!({ "post_ids": [] }),
        json!({ "post_ids": [1] }),
        json!({ "post_ids": [POST_ID], "readBits": "1" }),
        json!({ "post_ids": [POST_ID], "memberIds": [READER_ID] }),
        json!({ "post_ids": [POST_ID], "readerIds": [READER_ID] }),
        json!({ "post_ids": [POST_ID], "req_id": 7 }),
    ] {
        assert!(
            outbound_rejects(&payload),
            "corrupt/internal input must fail closed"
        );
    }
}

/// 有效 snapshot 按原始顺序映射为严格的 postReaders 合同字段。
#[test]
fn ready_projection_preserves_order_counts_and_snapshot_id() {
    let rows = json!([{
        "postId": POST_ID,
        "channelId": CHANNEL_ID,
        "authorUserId": AUTHOR_ID,
        "snapshotId": SNAPSHOT_ID,
        "snapshotVersion": 1001,
        "memberIds": [READER_ID, UNREAD_ID],
        "readBits": "10",
        "readerIds": ["legacy-wrong"],
        "readers": [{"userId": "legacy-wrong"}]
    }]);
    let receipts = run_rows(rows);
    assert_eq!(receipts.len(), 1);
    assert_eq!(
        receipts[0],
        json!({
            "channelId": CHANNEL_ID,
            "postId": POST_ID,
            "snapshotId": SNAPSHOT_ID,
            "readUserIds": [READER_ID],
            "unreadUserIds": [UNREAD_ID],
            "readCount": 1,
            "unreadCount": 1,
            "allRead": false
        })
    );
    assert_no_internal_receipt_fields(&receipts[0]);
}

/// 作者即使仍出现在旧行中也不占位，接收者列表继续保持 snapshot 原始顺序。
#[test]
fn projection_excludes_author_without_reordering_receivers() {
    let rows = json!([{
        "postId": POST_ID,
        "channelId": CHANNEL_ID,
        "authorUserId": AUTHOR_ID,
        "snapshotId": SNAPSHOT_ID,
        "memberIds": [UNREAD_ID, AUTHOR_ID, READER_ID],
        "readBits": "011"
    }]);
    let receipt = run_rows(rows).pop().expect("one receipt");
    assert_eq!(receipt["readUserIds"], json!([READER_ID]));
    assert_eq!(receipt["unreadUserIds"], json!([UNREAD_ID]));
    assert_eq!(receipt["readCount"], json!(1));
    assert_eq!(receipt["unreadCount"], json!(1));
    assert_no_internal_receipt_fields(&receipt);
}

/// bitmap、snapshot 或行状态损坏时逐行 unavailable，且仍不泄漏内部字段。
#[test]
fn malformed_rows_fail_closed_independently() {
    let rows = json!([
        {
            "postId": "post-bad-bits",
            "channelId": CHANNEL_ID,
            "authorUserId": AUTHOR_ID,
            "snapshotId": "snapshot-bad-bits",
            "memberIds": [READER_ID, UNREAD_ID],
            "readBits": "1"
        },
        {
            "postId": "post-missing-snapshot",
            "channelId": CHANNEL_ID,
            "authorUserId": AUTHOR_ID,
            "memberIds": [READER_ID],
            "readBits": "1"
        },
        {
            "postId": "post-unavailable",
            "channelId": CHANNEL_ID,
            "snapshotId": "snapshot-unavailable",
            "status": "unavailable",
            "unavailableReason": "missing_member_snapshot",
            "readBits": "111",
            "memberIds": [AUTHOR_ID, READER_ID, UNREAD_ID],
            "readerIds": ["must-not-leak"]
        }
    ]);
    let receipts = run_rows(rows);
    assert_eq!(receipts.len(), 3);
    for receipt in &receipts {
        assert_eq!(receipt["state"], json!("unavailable"));
        assert_eq!(receipt["readUserIds"], json!([]));
        assert_eq!(receipt["unreadUserIds"], json!([]));
        assert_eq!(receipt["readCount"], json!(0));
        assert_eq!(receipt["unreadCount"], json!(0));
        assert_eq!(receipt["allRead"], json!(false));
        assert_no_internal_receipt_fields(receipt);
    }
}

/// 非数组 data 不产生伪造 receipt 事件；相同完整快照重复投影保持 byte-stable。
#[test]
fn malformed_envelope_is_empty_and_replay_is_idempotent() {
    let mut harness = Harness::new_with_identity(&[], AUTHOR_ID, "mv3-g06d", "", "", "");
    harness.step(
        command_json(
            "im_post_read_list",
            json!({ "req_id": "receipt-malformed", "post_ids": [POST_ID] }),
        ),
        1,
    );
    let corr = http_corr(&harness);
    harness.step(
        port_reply_with_body(
            corr,
            enveloped(Bytes::from_static(br#"{"status":"SUCCESS","data":{}}"#)),
        ),
        2,
    );
    assert!(emitted_receipts(&harness).is_empty());

    let rows = json!([{
        "postId": POST_ID,
        "channelId": CHANNEL_ID,
        "authorUserId": AUTHOR_ID,
        "snapshotId": SNAPSHOT_ID,
        "memberIds": [READER_ID, UNREAD_ID],
        "readBits": "10"
    }]);
    let first = run_rows(rows.clone()).pop().expect("first receipt");
    let second = run_rows(rows).pop().expect("replayed receipt");
    assert_eq!(
        serde_json::to_vec(&first).expect("first receipt bytes"),
        serde_json::to_vec(&second).expect("second receipt bytes")
    );
}
