//! MessageV3 Sync type=1 的 NativeStorage / NativeEventSink 本地集成证据。

use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use helix_core::effect::{Effect, StorageOp};
use helix_core::module_host::Module;
use helix_core::ports::EventSink;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Tick};
use helix_driver_host::{base64_encode, execute_storage_ops, execute_storage_ops_atomic};
use helix_driver_native::{NativeEventSink, NativeStorage};
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};

const CHANNEL_ID: &str = "chfixx00000000000000000001";
const VIEWER_ID: &str = "viewer-444";

/// 创建当前测试独占的 SQLite，并应用生产 IM schema。
async fn open_storage() -> (NativeStorage, std::path::PathBuf) {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("helix-sync-native-{nonce}.db"));
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
