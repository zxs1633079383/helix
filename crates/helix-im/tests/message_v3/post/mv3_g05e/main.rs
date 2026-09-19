// 回归锚: visual spec 004 / T132-T134，MV3-G05e TEMPLATE 收到确认与已读回执。
//
// 参考包（线型权威）：
//   specs/003-messagev3-gate-inventory/gates/post/mv3-g05e/{inbound,outbound}-sample.json
//
// 本文件覆盖 INV-06 / INV-07：
//   - snapshotId 是权威回执键，channelId + createAt 只作修复回退；
//   - receiver snapshot 固定用消息创建时的成员顺序并剔除发送者；
//   - 发送者不占 readBits；
//   - Angular 只收有序 readUserIds / unreadUserIds 与绝对计数；
//   - 批量回执逐行独立 fail-closed；
//   - sender / receiver 双 persona 的出站投影 byte-exact。

use bytes::Bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{cid, command_json, enveloped, port_reply_ok, port_reply_with_body, Harness};

const POST_ID: &str = "post-template-001";
const AUTHOR_ID: &str = "user-actor-444";
const READER_ID: &str = "user-receiver-445";
const UNREAD_ID: &str = "user-receiver-446";
const SNAPSHOT_ID: &str = "snapshot-post-template-001";
const CHANNEL_IX: u64 = 0x5e;

/// 取本批次首个 `Effect::Http` 的 correlation。
fn http_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("command must emit HTTP")
}

/// 取本批次首个 `Effect::Http` 的 URL。
fn http_url(harness: &Harness) -> String {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some(req.url.clone()),
            _ => None,
        })
        .expect("command must emit HTTP")
}

/// 取本批次首个 `Effect::Persist` 的 correlation。
fn persist_corr(harness: &Harness) -> u64 {
    harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("authority must persist before releasing the receipt")
}

/// 收集本批次指定业务事件的 data 段。
fn emits(harness: &Harness, event: &str) -> Vec<Value> {
    harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event: bytes } => serde_json::from_slice::<Value>(bytes.0.as_ref()).ok(),
            _ => None,
        })
        .filter(|payload| payload["event"] == event)
        .map(|payload| payload["data"].clone())
        .collect()
}

/// 构造 Go `post/read/list` 的权威回执 body。
fn read_list_body(rows: Value) -> Bytes {
    enveloped(Bytes::from(
        serde_json::to_vec(&json!({ "status": "SUCCESS", "data": rows })).expect("body"),
    ))
}

/// Read/list projections carry a channel id and therefore need the exact persisted
/// channel ownership row used by the receipt fixture.
fn scoped_reader_harness(viewer_user_id: &str) -> Harness {
    let mut harness = Harness::new_with_identity(&[], viewer_user_id, "mv3-g05e", "", "", "");
    harness.seed_channel_scope(&cid(CHANNEL_IX), "mv3-g05e", viewer_user_id);
    harness.sink.clear();
    harness.effect_log.clear();
    harness
}

/// 以指定 viewer 身份跑一次 `im_post_read_list`，返回全部逐条 canonical 回执。
fn run_read_list_events(viewer_user_id: &str, req_id: &str, rows: Value) -> Vec<Value> {
    let mut harness = scoped_reader_harness(viewer_user_id);
    harness.step(
        command_json(
            "im_post_read_list",
            json!({ "req_id": req_id, "post_ids": [POST_ID] }),
        ),
        1,
    );
    let corr = http_corr(&harness);
    harness.step(port_reply_with_body(corr, read_list_body(rows)), 2);
    emits(&harness, "im:post:readers")
}

/// 以指定 viewer 身份跑一次单行查询，返回首条 canonical 回执。
fn run_read_list_data(viewer_user_id: &str, req_id: &str, rows: Value) -> Value {
    run_read_list_events(viewer_user_id, req_id, rows)
        .into_iter()
        .next()
        .expect("post/read/list reply must project a render-ready receipt")
}

/// 同上，直接取第一条回执行。
fn run_read_list(viewer_user_id: &str, req_id: &str, rows: Value) -> Value {
    run_read_list_data(viewer_user_id, req_id, rows)
}

/// 冻结边界要求每条 `im:post:readers.data` 直接是一条回执，禁止泄漏内部批处理外壳。
#[test]
fn post_readers_event_emits_one_canonical_receipt_per_row() {
    let data = run_read_list_data(
        READER_ID,
        "receipt-canonical-event",
        legacy_snapshot_row("110"),
    );
    assert_eq!(data["postId"], json!(POST_ID));
    assert_eq!(data["channelId"], json!(cid(CHANNEL_IX)));
    assert_eq!(data["snapshotId"], json!(SNAPSHOT_ID));
    assert!(data.get("receipts").is_none());
    assert!(data.get("reqId").is_none());
}

/// 消息创建时的成员快照：作者在首位，两名接收者按服务端顺序排列。
fn legacy_snapshot_row(read_bits: &str) -> Value {
    json!([{
        "postId": POST_ID,
        "channelId": cid(CHANNEL_IX),
        "authorUserId": AUTHOR_ID,
        "snapshotId": SNAPSHOT_ID,
        "snapshotVersion": 1001,
        "memberIds": [AUTHOR_ID, READER_ID, UNREAD_ID],
        "readBits": read_bits,
    }])
}

/// 证明 MV3-G05e 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G05e", "post/mv3-g05e");
}

/// TEMPLATE 收到确认必须打到 Go 单数前缀 `post/templateReceived`，body 只带 postId。
#[test]
fn template_received_command_targets_go_template_endpoint() {
    let mut harness = Harness::new_with_identity(&[], READER_ID, "mv3-g05e", "", "", "");
    harness.step(
        command_json("im_template_received", json!({ "post_id": POST_ID })),
        1,
    );
    let url = http_url(&harness);
    assert!(
        url.ends_with("post/templateReceived"),
        "templateReceived 必须走单数 post 前缀，got {url}"
    );
    assert_eq!(
        harness.last_http_body(),
        Some(json!({ "postId": POST_ID })),
        "wire body 真源是匿名 struct {{postId}}"
    );
    assert!(
        emits(&harness, "im:post:updated").is_empty(),
        "HTTP 出站只是受理，终态必须等权威 WS/读回"
    );
}

/// 批量已读命令与模板确认是两条独立出站，postId 不得串用。
#[test]
fn post_read_list_command_carries_only_requested_post_ids() {
    let mut harness = Harness::new_with_identity(&[], READER_ID, "mv3-g05e", "", "", "");
    harness.step(
        command_json(
            "im_post_read_list",
            json!({ "req_id": "receipt-mv3-g05e", "post_ids": [POST_ID] }),
        ),
        1,
    );
    assert!(http_url(&harness).ends_with("post/read/list"));
    assert_eq!(
        harness.last_http_body(),
        Some(json!({ "postIds": [POST_ID] }))
    );
}

/// INV-06 + INV-07：作者被剔除，接收者按快照顺序生成有序 read/unread 列表与绝对计数。
#[test]
fn receipt_excludes_author_and_keeps_snapshot_order() {
    // bits = 作者位1 / READER 已读 / UNREAD 未读。
    let receipt = run_read_list(READER_ID, "receipt-order", legacy_snapshot_row("110"));

    assert_eq!(receipt["state"], json!("ready"));
    assert_eq!(receipt["readUserIds"], json!([READER_ID]));
    assert_eq!(receipt["unreadUserIds"], json!([UNREAD_ID]));
    assert_eq!(receipt["readCount"], json!(1));
    assert_eq!(receipt["unreadCount"], json!(1));
    assert_eq!(receipt["allRead"], json!(false));
    assert_eq!(receipt["authorExcluded"], json!(true));
}

/// INV-06 铁律：发送者不占 readBits——即使作者位为 1，也不得出现在任何一侧。
#[test]
fn sender_never_appears_in_its_own_receipt_lists() {
    for bits in ["110", "010"] {
        let receipt = run_read_list(AUTHOR_ID, "receipt-sender", legacy_snapshot_row(bits));
        let read_ids = receipt["readUserIds"].as_array().expect("readUserIds");
        let unread_ids = receipt["unreadUserIds"].as_array().expect("unreadUserIds");
        assert!(
            !read_ids.iter().any(|id| id == AUTHOR_ID),
            "bits={bits} 时作者出现在已读列表: {read_ids:?}"
        );
        assert!(
            !unread_ids.iter().any(|id| id == AUTHOR_ID),
            "bits={bits} 时作者出现在未读列表（历史高频回归点）: {unread_ids:?}"
        );
        assert_eq!(
            read_ids.len() + unread_ids.len(),
            2,
            "bits={bits} 时接收者总数必须恒为 memberIds 去作者后的长度"
        );
    }
}

/// bit 顺序必须严格对齐快照下标：换 bit 即换人，不能靠字典序或计数糊过去。
#[test]
fn read_bit_index_maps_to_the_member_at_the_same_position() {
    let first_read = run_read_list(READER_ID, "receipt-bit-a", legacy_snapshot_row("010"));
    assert_eq!(first_read["readUserIds"], json!([READER_ID]));
    assert_eq!(first_read["unreadUserIds"], json!([UNREAD_ID]));

    let second_read = run_read_list(READER_ID, "receipt-bit-b", legacy_snapshot_row("001"));
    assert_eq!(
        second_read["readUserIds"],
        json!([UNREAD_ID]),
        "第二个接收者位置的 1 必须映射到第二个成员"
    );
    assert_eq!(second_read["unreadUserIds"], json!([READER_ID]));

    let all_read = run_read_list(READER_ID, "receipt-bit-c", legacy_snapshot_row("011"));
    assert_eq!(all_read["readUserIds"], json!([READER_ID, UNREAD_ID]));
    assert_eq!(all_read["unreadUserIds"], json!([]));
    assert_eq!(all_read["allRead"], json!(true));
    assert_eq!(all_read["readCount"], json!(2));
}

/// 快照顺序即服务端顺序：颠倒 memberIds 必须颠倒输出，投影不得自行排序。
#[test]
fn receipt_lists_are_not_sorted_by_the_projection() {
    let reversed = json!([{
        "postId": POST_ID,
        "channelId": cid(CHANNEL_IX),
        "authorUserId": AUTHOR_ID,
        "snapshotId": SNAPSHOT_ID,
        "snapshotVersion": 1001,
        "memberIds": [UNREAD_ID, READER_ID],
        "readBits": "11",
    }]);
    let receipt = run_read_list(READER_ID, "receipt-reversed", reversed);
    assert_eq!(receipt["readUserIds"], json!([UNREAD_ID, READER_ID]));
}

/// INV-06：snapshotId 是权威键，且必须原样回传给 Angular 作为对账锚。
#[test]
fn snapshot_id_is_the_authoritative_receipt_key() {
    let receipt = run_read_list(READER_ID, "receipt-snapshot", legacy_snapshot_row("110"));
    assert_eq!(receipt["snapshotId"], json!(SNAPSHOT_ID));
    assert_eq!(receipt["snapshotVersion"], json!(1001));
    assert_eq!(receipt["receiptKey"], json!("snapshotId"));

    // 缺 snapshotId → fail-closed，不得用 memberIds 兜出一个"看起来对"的回执。
    let missing_snapshot = json!([{
        "postId": POST_ID,
        "channelId": cid(CHANNEL_IX),
        "createAt": 1_700_000_000_000_i64,
        "authorUserId": AUTHOR_ID,
        "snapshotVersion": 1001,
        "memberIds": [AUTHOR_ID, READER_ID, UNREAD_ID],
        "readBits": "110",
    }]);
    let degraded = run_read_list(READER_ID, "receipt-no-snapshot", missing_snapshot);
    assert_eq!(degraded["state"], json!("unavailable"));
    assert_eq!(degraded["readUserIds"], json!([]));
    assert_eq!(degraded["unreadUserIds"], json!([]));
    assert_eq!(degraded["receiptKey"], json!("channelIdCreateAtFallback"));
}

/// 批量回执逐行独立判定：坏行 unavailable，好行照常 ready。
#[test]
fn batch_rows_fail_closed_independently() {
    let mixed = json!([
        {
            "postId": "post-template-bad",
            "channelId": cid(CHANNEL_IX),
            "authorUserId": AUTHOR_ID,
            "snapshotId": "snapshot-bad",
            "snapshotVersion": 1002,
            "memberIds": [AUTHOR_ID, READER_ID],
            "readBits": "1",
        },
        {
            "postId": POST_ID,
            "channelId": cid(CHANNEL_IX),
            "authorUserId": AUTHOR_ID,
            "snapshotId": SNAPSHOT_ID,
            "snapshotVersion": 1001,
            "memberIds": [AUTHOR_ID, READER_ID, UNREAD_ID],
            "readBits": "110",
        },
    ]);
    let rows = run_read_list_events(READER_ID, "receipt-batch", mixed);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["state"], json!("unavailable"));
    assert_eq!(rows[0]["readUserIds"], json!([]));
    assert_eq!(rows[0]["allRead"], json!(false));
    assert_eq!(rows[1]["state"], json!("ready"));
    assert_eq!(rows[1]["readUserIds"], json!([READER_ID]));
}

/// 双 persona：sender 与 receiver 拿到 byte-exact 相同的回执投影（回执不是 viewer-relative）。
#[test]
fn sender_and_receiver_personas_receive_byte_exact_receipts() {
    let sender = run_read_list_data(AUTHOR_ID, "receipt-dual", legacy_snapshot_row("110"));
    let receiver = run_read_list_data(READER_ID, "receipt-dual", legacy_snapshot_row("110"));
    assert_eq!(
        serde_json::to_vec(&sender).expect("sender bytes"),
        serde_json::to_vec(&receiver).expect("receiver bytes"),
        "回执列表是全局绝对态，两端不得出现视角差"
    );
}

/// HTTP 失败只回灌内部读错误，禁止向 Angular 发布伪造的空回执快照。
#[test]
fn transport_failure_emits_read_result_error_without_receipt() {
    let mut harness = scoped_reader_harness(READER_ID);
    harness.step(
        command_json(
            "im_post_read_list",
            json!({ "req_id": "receipt-err", "post_ids": [POST_ID] }),
        ),
        1,
    );
    let corr = http_corr(&harness);
    harness.step(common::port_reply_err(corr), 2);
    assert!(
        emits(&harness, "im:post:readers").is_empty(),
        "HTTP 失败没有权威回执，不得发布伪造空快照"
    );
    assert_eq!(emits(&harness, "im:read:result").len(), 1);
}

/// 权威 `post_read` 帧只在 PersistOk 后释放终态，重复帧幂等（G-14 续接不被本 Gate 破坏）。
#[test]
fn post_read_authority_releases_terminal_only_after_persist_and_is_idempotent() {
    let authority = json!({
        "action": "post_read",
        "data": {
            "channelId": cid(CHANNEL_IX),
            "postId": POST_ID,
            "authorUserId": AUTHOR_ID,
            "readerUserId": READER_ID,
            "snapshotId": SNAPSHOT_ID,
            "memberIds": [READER_ID, UNREAD_ID],
            "readMap": "10",
            "createAt": 1_700_000_000_000_i64,
            "updateAt": 1_700_000_000_020_i64,
        }
    });
    let mut harness = Harness::new_with_identity(&[CHANNEL_IX], READER_ID, "mv3-g05e", "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&authority).expect("authority"),
        ))),
        1,
    );
    assert!(
        emits(&harness, "im:post:read").is_empty(),
        "持久屏障之前不得对 renderer 可见"
    );
    let corr = persist_corr(&harness);
    harness.step(port_reply_ok(corr), 2);
    let terminal = emits(&harness, "im:post:read")
        .into_iter()
        .next()
        .expect("PersistOk 之后必须释放 reader 终态");
    assert_eq!(terminal["postId"], json!(POST_ID));
    assert_eq!(terminal["readerUserId"], json!(READER_ID));
    assert_eq!(terminal["snapshotId"], json!(SNAPSHOT_ID));
    assert_eq!(terminal["readUserIds"], json!([READER_ID]));
    assert_eq!(terminal["unreadUserIds"], json!([UNREAD_ID]));
    assert_eq!(terminal["readCount"], json!(1));
    assert_eq!(terminal["unreadCount"], json!(1));
    assert_eq!(terminal["allRead"], json!(false));
    assert!(terminal.get("readBits").is_none());
    assert!(terminal.get("memberIds").is_none());

    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&authority).expect("authority"),
        ))),
        3,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "同一 receipt revision 重复到达必须完全幂等"
    );
}
