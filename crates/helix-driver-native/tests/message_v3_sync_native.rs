//! MessageV3 Sync type=1 的 NativeStorage / NativeEventSink 本地集成证据。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::module_host::Module;
use helix_core::ports::EventSink;
use helix_core::tick::{InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Tick};
use helix_driver_host::{base64_encode, execute_storage_ops, execute_storage_ops_atomic};
use helix_driver_native::{NativeEventSink, NativeStorage};
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};

const CHANNEL_ID: &str = "chfixx00000000000000000001";
const VIEWER_ID: &str = "viewer-444";
static DB_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// 创建当前测试独占的 SQLite，并应用生产 IM schema。
async fn open_storage() -> (NativeStorage, std::path::PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let sequence = DB_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "helix-sync-native-{}-{sequence}-{nonce}.db",
        std::process::id()
    ));
    let storage = NativeStorage::open(&format!("sqlite:{}?mode=rwc", path.display()))
        .await
        .expect("open NativeStorage");
    for statement in IM_SCHEMA {
        storage
            .execute_raw(statement)
            .await
            .expect("apply IM schema");
    }
    for statement in IM_SCHEMA_MIGRATIONS {
        if let Err(error) = storage.execute_raw(statement).await {
            assert!(
                error.to_string().contains("duplicate column name"),
                "unexpected migration error: {error}"
            );
        }
    }
    (storage, path)
}

/// 构造触发大缺口的在线 post；目标群未 attach。
fn gap_tick() -> Tick {
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "action": "post",
            "data": {
                "id": "srvfix00000000000000000099",
                "temporaryId": "tmp-99",
                "channelId": CHANNEL_ID,
                "event_seq": 99,
                "userId": "sender-678",
                "type": "TEXT",
                "message": "gap"
            }
        }))
        .expect("serialize gap frame"),
    )))
}

/// 构造可见的连续在线 post，用于验证小 gap 缓冲与补齐顺序。
fn post_tick(seq: u64) -> Tick {
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "action": "post",
            "data": {
                "id": format!("srvfix{seq:020x}"),
                "temporaryId": format!("tmp-{seq}"),
                "channelId": CHANNEL_ID,
                "event_seq": seq,
                "userId": "sender-678",
                "type": "TEXT",
                "message": format!("message-{seq}"),
                "simpleMessage": format!("message-{seq}"),
                "createAt": seq,
                "viewers": [VIEWER_ID]
            }
        }))
        .expect("serialize post frame"),
    )))
}

/// 构造 host 实际交还给 Helix 的 ADR-007 Sync HTTP 信封。
fn sync_reply() -> Bytes {
    let raw = serde_json::to_vec(&serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "entries": [{
                "channelId": CHANNEL_ID,
                "kind": "events",
                "events": [{
                    "channelId": CHANNEL_ID,
                    "eventSeq": 1,
                    "eventType": 1,
                    "msgId": "srvfix00000000000000000001"
                }],
                "messages": {
                    "srvfix00000000000000000001": {
                        "id": "srvfix00000000000000000001",
                        "temporaryId": "tmp-1",
                        "channelId": CHANNEL_ID,
                        "userId": "sender-678",
                        "type": "TEXT",
                        "message": "first unread",
                        "simpleMessage": "first unread",
                        "createAt": 1,
                        "viewers": [VIEWER_ID]
                    }
                },
                "nextSeq": 1,
                "resetTo": null
            }],
            "serverTime": 1
        }
    }))
    .expect("serialize sync body");
    Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status": 200,
            "headers": {},
            "body": base64_encode(&raw)
        }))
        .expect("serialize HTTP envelope"),
    )
}

/// 从 Effect 批次取出指定类别的 correlation 与操作集。
fn take_persist(sink: &EffectSink, atomic: bool) -> (Correlation, Vec<StorageOp>) {
    sink.as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } if atomic => Some((*corr, ops.clone())),
            Effect::Persist { corr, ops } if !atomic => Some((*corr, ops.clone())),
            _ => None,
        })
        .expect("expected persist effect")
}

/// Sync type=1 经真实 SQLite 提交与复合键读回后必须首条即投影 unread=1。
#[tokio::test]
async fn sync_type1_projects_first_unread_through_native_ports() {
    let (storage, path) = open_storage().await;
    let (event_sink, mut event_rx) = NativeEventSink::new();
    let mut config = ImConfig::default();
    config.auth_user_id = VIEWER_ID.to_string();
    config.company_id = "tenant-1".to_string();
    config.api_base_url = "http://127.0.0.1:18066".to_string();
    let mut module = ImModule::new(config);
    module.register_channel(
        helix_im::state::ChannelId::from_str(CHANNEL_ID).expect("canonical channel id"),
        0,
    );
    let mut sink = EffectSink::new();

    module
        .handle(&gap_tick(), 1_000, &mut sink)
        .expect("gap handle");
    let sync_corr = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .expect("gap must request sync");
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: sync_corr,
                outcome: PortOutcome::Ok(ReplyBytes(sync_reply())),
            },
            2_000,
            &mut sink,
        )
        .expect("sync reply handle");
    let (persist_corr, persist_ops) = take_persist(&sink, true);
    let persist_outcome = execute_storage_ops_atomic(&storage, &persist_ops).await;
    assert!(matches!(persist_outcome, PortOutcome::Ok(_)));

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: persist_corr,
                outcome: persist_outcome,
            },
            2_001,
            &mut sink,
        )
        .expect("persist reply handle");
    let (read_corr, read_ops) = take_persist(&sink, false);
    assert!(matches!(
        read_ops.as_slice(),
        [StorageOp::ScopedGet(spec)]
            if spec.table == "channel_member"
                && spec.scope_col == "channel_id"
                && spec.key_col == "user_id"
    ));
    let read_outcome = execute_storage_ops(&storage, &read_ops).await;

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: read_corr,
                outcome: read_outcome,
            },
            2_002,
            &mut sink,
        )
        .expect("member readback handle");
    for effect in sink.as_slice() {
        if let Effect::Emit { event } = effect {
            event_sink.emit(event.clone());
        }
    }
    let event = tokio::time::timeout(std::time::Duration::from_secs(1), event_rx.recv())
        .await
        .expect("NativeEventSink projection timeout")
        .expect("NativeEventSink open");
    let event: serde_json::Value =
        serde_json::from_slice(event.0.as_ref()).expect("decode projected event");
    assert_eq!(event["event"], "im:channel:update");
    assert_eq!(event["data"]["channelId"], CHANNEL_ID);
    assert_eq!(event["data"]["unreadCount"], 1);

    drop(storage);
    let _ = std::fs::remove_file(path);
}

/// 小 gap 必须先缓冲高序号，缺失事件持久化后才依次投影 unread=1、2。
#[tokio::test]
async fn small_gap_advances_only_after_each_native_persist() {
    let (storage, path) = open_storage().await;
    let (event_sink, mut event_rx) = NativeEventSink::new();
    let mut config = ImConfig::default();
    config.auth_user_id = VIEWER_ID.to_string();
    config.company_id = "tenant-1".to_string();
    let mut module = ImModule::new(config);
    module.register_channel(
        helix_im::state::ChannelId::from_str(CHANNEL_ID).expect("canonical channel id"),
        0,
    );
    let mut sink = EffectSink::new();

    module
        .handle(&post_tick(2), 1_000, &mut sink)
        .expect("buffer seq=2");
    assert!(
        !sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::PersistAtomic { .. })),
        "高序号事件在缺口闭合前不得写库"
    );
    assert_eq!(
        module.cursor_for(
            helix_im::state::ChannelId::from_str(CHANNEL_ID).expect("canonical channel id")
        ),
        Some(0),
        "高序号事件不得 MAX-jump cursor"
    );

    sink.clear();
    module
        .handle(&post_tick(1), 1_001, &mut sink)
        .expect("accept seq=1");
    let (first_corr, first_ops) = take_persist(&sink, true);
    let first_outcome = execute_storage_ops_atomic(&storage, &first_ops).await;
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: first_corr,
                outcome: first_outcome,
            },
            1_002,
            &mut sink,
        )
        .expect("commit seq=1");

    let (first_read_corr, first_read_ops) = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .expect("seq=1 member readback");
    let (second_corr, second_ops) = sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, ops } => Some((*corr, ops.clone())),
            _ => None,
        })
        .expect("buffered seq=2 starts only after seq=1 commit");
    let first_read_outcome = execute_storage_ops(&storage, &first_read_ops).await;
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: first_read_corr,
                outcome: first_read_outcome,
            },
            1_003,
            &mut sink,
        )
        .expect("project seq=1");
    for effect in sink.as_slice() {
        if let Effect::Emit { event } = effect {
            event_sink.emit(event.clone());
        }
    }

    let second_outcome = execute_storage_ops_atomic(&storage, &second_ops).await;
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: second_corr,
                outcome: second_outcome,
            },
            1_004,
            &mut sink,
        )
        .expect("commit seq=2");
    let (second_read_corr, second_read_ops) = take_persist(&sink, false);
    let second_read_outcome = execute_storage_ops(&storage, &second_read_ops).await;
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: second_read_corr,
                outcome: second_read_outcome,
            },
            1_005,
            &mut sink,
        )
        .expect("project seq=2");
    for effect in sink.as_slice() {
        if let Effect::Emit { event } = effect {
            event_sink.emit(event.clone());
        }
    }

    let mut unreads = Vec::new();
    while let Ok(event) = event_rx.try_recv() {
        let event: serde_json::Value =
            serde_json::from_slice(event.0.as_ref()).expect("decode event");
        if event["event"] == "im:channel:update" {
            unreads.push(event["data"]["unreadCount"].as_i64().expect("unread"));
        }
    }
    assert_eq!(unreads, vec![1, 2]);
    assert_eq!(
        module.cursor_for(
            helix_im::state::ChannelId::from_str(CHANNEL_ID).expect("canonical channel id")
        ),
        Some(2)
    );

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: first_corr,
                outcome: PortOutcome::Ok(ReplyBytes::default()),
            },
            1_006,
            &mut sink,
        )
        .expect("duplicate reply is ignored");
    assert!(sink.as_slice().is_empty(), "重复回报不得产生第二次终态");

    drop(storage);
    let _ = std::fs::remove_file(path);
}

/// 小 gap 的首笔原子写失败时不得推进 cursor、读回或发布成功终态。
#[tokio::test]
async fn small_gap_persist_error_and_duplicate_reply_fail_closed() {
    let mut config = ImConfig::default();
    config.auth_user_id = VIEWER_ID.to_string();
    config.company_id = "tenant-1".to_string();
    let mut module = ImModule::new(config);
    let channel_id =
        helix_im::state::ChannelId::from_str(CHANNEL_ID).expect("canonical channel id");
    module.register_channel(channel_id, 0);
    let mut sink = EffectSink::new();

    module
        .handle(&post_tick(2), 1_000, &mut sink)
        .expect("buffer seq=2");
    sink.clear();
    module
        .handle(&post_tick(1), 1_001, &mut sink)
        .expect("accept seq=1");
    let (persist_corr, _) = take_persist(&sink, true);

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: persist_corr,
                outcome: PortOutcome::Err(PortError::Storage(1)),
            },
            1_002,
            &mut sink,
        )
        .expect("persist error handle");
    assert_eq!(module.cursor_for(channel_id), Some(0));
    assert!(
        sink.as_slice()
            .iter()
            .all(|effect| !matches!(effect, Effect::Persist { .. } | Effect::Emit { .. })),
        "PersistErr 后不得读回或发布成功终态"
    );

    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: persist_corr,
                outcome: PortOutcome::Ok(ReplyBytes::default()),
            },
            1_003,
            &mut sink,
        )
        .expect("late fake success ignored");
    assert_eq!(module.cursor_for(channel_id), Some(0));
    assert!(sink.as_slice().is_empty());
}
