//! 真 DB 落库集成测试（防"绿测试锁死 bug"陷阱）。
//!
//! ## 为什么需要这个测试
//!
//! 此前 helix-im 的事件落库测试只断言 `Effect` 里 `StorageOp.table` 字符串，
//! 恰好把错误表名（不存在的 `messages`）固化为期望值——绿测试反而锁死了 bug：
//! 两条 live 写路径（WS 推送 / B-rest 增量）都写向永不存在的表，运行时必 `no such
//! table: messages`，sync cursor 永不推进 → 无限重拉风暴。
//!
//! 本测试用真 `IM_SCHEMA` 建真 SQLite（rusqlite，host driver 同款执行器），
//! 喂 `event_to_upsert_op` / `batch_upsert_events` 产物，**断言 `message` 表真有行**
//! （而非断 Effect 字符串）。表名错配 / 缺列 / PK 撞键任一回归都会让此测试红。

use helix_core::effect::{ScanSpec, SqlValue};
use helix_core::ports::Storage;
use helix_driver_host::HostStorage;
use helix_im::channel::event_to_upsert_op;
use helix_im::state::{ChannelId, Seq};
use helix_im::sync_session::{EventEnvelope, EventKind};
use std::sync::atomic::{AtomicUsize, Ordering};

static DB_SEQ: AtomicUsize = AtomicUsize::new(0);

/// 以真 `IM_SCHEMA` 建库（含 message / channel / channel_member / channel_event_cursor）。
async fn storage_with_im_schema() -> HostStorage {
    let seq = DB_SEQ.fetch_add(1, Ordering::Relaxed);
    let db = format!(
        "file:helix_im_persist_{}_{}?mode=memory&cache=shared",
        std::process::id(),
        seq
    );
    let storage = HostStorage::open_sqlite_url(&db)
        .await
        .expect("open storage");
    // host-cli 启动同款：migrate 后 loop execute_raw 叠加 IM_SCHEMA。
    for &sql in helix_im::IM_SCHEMA {
        storage
            .execute_raw(sql)
            .await
            .unwrap_or_else(|e| panic!("execute_raw IM_SCHEMA failed for {sql:?}: {e:?}"));
    }
    storage
}

/// 造一条 type=1 PostUpsert 事件（raw = ChannelEvent wire JSON，含真 post 字段）。
fn post_event(channel: u64, seq: u64, temporary_id: &str, id: &str, text: &str) -> EventEnvelope {
    let raw = serde_json::to_vec(&serde_json::json!({
        "eventSeq": seq,
        "eventType": 1,
        "temporaryId": temporary_id,
        "id": id,
        "channelId": ChannelId::from_test_u64(channel).as_str(),
        "userId": "user-zhang",
        "type": "P",
        "message": text,
        "props": { "channel_event_seq": seq },
        "createAt": 1_700_000_000_000_i64 + seq as i64,
    }))
    .expect("serialize post event");
    EventEnvelope::new(
        ChannelId::from_test_u64(channel),
        Seq(seq),
        EventKind::PostUpsert,
        bytes::Bytes::from(raw),
    )
}

async fn scan_message_rows(storage: &HostStorage) -> Vec<helix_core::effect::Row> {
    storage
        .scan(ScanSpec {
            table: "message",
            limit: None,
        })
        .await
        .expect("scan message table")
}

fn col<'a>(row: &'a helix_core::effect::Row, name: &str) -> Option<&'a SqlValue> {
    row.iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

fn col_text(row: &helix_core::effect::Row, name: &str) -> String {
    match col(row, name) {
        Some(SqlValue::Text(s)) => s.clone(),
        other => panic!("column {name} expected Text, got {other:?}"),
    }
}

/// 核心回归：WS 推送路径用的 `event_to_upsert_op` 落到真 `message` 表（PK=temporary_id）。
///
/// 若回退成写 `messages` 影子表 → execute IM_SCHEMA 不建该表 → batch_upsert 抛
/// `no such table: messages` → 此测试在 batch_upsert 即红。
#[tokio::test]
async fn ws_event_persists_into_678_message_table() {
    let storage = storage_with_im_schema().await;
    let ev = post_event(42, 7, "tmp-abc", "post-real-1", "hello world");

    let op = event_to_upsert_op(&ev);
    storage
        .batch_upsert(match op {
            helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
            other => panic!("expected BatchUpsert, got {other:?}"),
        })
        .await
        .expect("batch_upsert into message table must succeed (table message exists in IM_SCHEMA)");

    let rows = scan_message_rows(&storage).await;
    assert_eq!(
        rows.len(),
        1,
        "事件必须真落 message 表一行（而非写不存在的 messages）"
    );
    let row = &rows[0];
    assert_eq!(
        col_text(row, "temporary_id"),
        "tmp-abc",
        "PK 取 temporary_id"
    );
    assert_eq!(col_text(row, "id"), "post-real-1");
    assert_eq!(
        col_text(row, "channel_id"),
        ChannelId::from_test_u64(42).as_str()
    );
    assert_eq!(col_text(row, "message"), "hello world");
    assert_eq!(col_text(row, "user_id"), "user-zhang");
}

/// sync 路径 `batch_upsert_events` 同样真落 message 表（与 WS 路径同表同 PK）。
#[tokio::test]
async fn sync_events_persist_into_678_message_table() {
    let storage = storage_with_im_schema().await;
    let events = vec![
        post_event(42, 10, "tmp-1", "post-1", "first"),
        post_event(42, 11, "tmp-2", "post-2", "second"),
        post_event(42, 12, "tmp-3", "post-3", "third"),
    ];

    for op in helix_im::acl::to_effect::batch_upsert_events(&events) {
        storage
            .batch_upsert(match op {
                helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
                other => panic!("expected BatchUpsert, got {other:?}"),
            })
            .await
            .expect("sync batch_upsert_events must land in message table");
    }

    let rows = scan_message_rows(&storage).await;
    assert_eq!(rows.len(), 3, "sync 三条事件全部真落 message 表");
}

/// 同一事件 re-sync（续拉重叠批）幂等：conflict_key=temporary_id → 覆盖同行，不产生重复。
/// 这正是修复"cursor 卡住 → 重拉同批"风暴的正确性前提：重拉必须 O(1) 覆盖不膨胀。
#[tokio::test]
async fn resync_same_event_is_idempotent_upsert() {
    let storage = storage_with_im_schema().await;
    let ev = post_event(99, 5, "tmp-dup", "post-dup", "v1");

    for _ in 0..3 {
        let op = event_to_upsert_op(&ev);
        storage
            .batch_upsert(match op {
                helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
                other => panic!("expected BatchUpsert, got {other:?}"),
            })
            .await
            .expect("idempotent re-upsert");
    }

    let rows = scan_message_rows(&storage).await;
    assert_eq!(
        rows.len(),
        1,
        "重拉同事件必须幂等覆盖（同 PK），不产生重复行"
    );
}

/// 服务端推送无 temporary_id（他人消息）时，PK 回退到 id（真源 cses-client From<Post>），
/// 仍能非空落库，不丢消息。
#[tokio::test]
async fn event_without_temporary_id_falls_back_to_id_as_pk() {
    let storage = storage_with_im_schema().await;
    let raw = serde_json::to_vec(&serde_json::json!({
        "eventSeq": 3,
        "eventType": 1,
        "id": "server-post-id-only",
        "channelId": ChannelId::from_test_u64(7).as_str(),
        "message": "from someone else",
    }))
    .unwrap();
    let ev = EventEnvelope::new(
        ChannelId::from_test_u64(7),
        Seq(3),
        EventKind::PostUpsert,
        bytes::Bytes::from(raw),
    );

    let op = event_to_upsert_op(&ev);
    storage
        .batch_upsert(match op {
            helix_core::effect::StorageOp::BatchUpsert(spec) => spec,
            other => panic!("expected BatchUpsert, got {other:?}"),
        })
        .await
        .expect("upsert with id-as-pk");

    let rows = scan_message_rows(&storage).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        col_text(&rows[0], "temporary_id"),
        "server-post-id-only",
        "无 temporary_id 时回退 id 作 PK"
    );
}
