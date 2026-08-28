//! MessageV3 Sync type=1 的 NativeStorage / NativeEventSink 本地集成证据。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use helix_core::effect::{Effect, GetSpec, ScopedGetSpec, SqlValue, StorageOp};
use helix_core::module_host::Module;
use helix_core::ports::{EventSink, Storage};
use helix_core::tick::{InboundBytes, PortError, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Tick};
use helix_driver_host::{base64_encode, execute_storage_ops, execute_storage_ops_atomic};
use helix_driver_native::{NativeEventSink, NativeStorage};
use helix_im::state::{ChannelId, Seq};
use helix_im::sync_session::{EventEnvelope, EventKind, PostFields};
use helix_im::{ImConfig, ImModule, IM_SCHEMA, IM_SCHEMA_MIGRATIONS};

const CHANNEL_ID: &str = "chfixx00000000000000000001";
const CHANNEL_ID_2: &str = "chfixx00000000000000000002";
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
fn sync_reply_for(channel_id: &str, suffix: u64) -> Bytes {
    let message_id = format!("srvfix{suffix:020x}");
    let raw = serde_json::to_vec(&serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "entries": [{
                "channelId": channel_id,
                "kind": "events",
                "events": [{
                    "channelId": channel_id,
                    "eventSeq": 1,
                    "eventType": 1,
                    "msgId": message_id
                }],
                "messages": {
                    message_id.clone(): {
                        "id": message_id,
                        "temporaryId": format!("tmp-{suffix}"),
                        "channelId": channel_id,
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

/// 构造两个显式频道缺口的无 action pong ACK。
fn pong_gap_tick() -> Tick {
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&serde_json::json!({
            "status": "OK",
            "data": {
                "gaps": [
                    {"channelId": CHANNEL_ID, "fromSeq": 1, "maxSeq": 1},
                    {"channelId": CHANNEL_ID_2, "fromSeq": 1, "maxSeq": 1}
                ],
                "hashMismatch": false
            }
        }))
        .expect("serialize pong gap"),
    )))
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
                outcome: PortOutcome::Ok(ReplyBytes(sync_reply_for(CHANNEL_ID, 1))),
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
    let reopened = NativeStorage::open(&format!("sqlite:{}?mode=rwc", path.display()))
        .await
        .expect("reopen NativeStorage after simulated restart");
    let member = reopened
        .scoped_get(ScopedGetSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL_ID.to_string()),
            key_col: "user_id",
            key_val: SqlValue::Text(VIEWER_ID.to_string()),
        })
        .await
        .expect("restart member readback")
        .expect("restart member row");
    assert!(member
        .iter()
        .any(|(name, value)| name == "unread_count" && matches!(value, SqlValue::Integer(1))));
    let cursor = reopened
        .get(GetSpec {
            table: "channel_event_cursor",
            key_col: "channel_id",
            key_val: SqlValue::Text(CHANNEL_ID.to_string()),
        })
        .await
        .expect("restart cursor readback")
        .expect("restart cursor row");
    assert!(cursor
        .iter()
        .any(|(name, value)| name == "last_event_seq" && matches!(value, SqlValue::Integer(1))));
    drop(reopened);
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

/// PongGap fan-out 必须等待整批提交后按频道去重读回最终 viewer dialog。
#[tokio::test]
async fn pong_gap_reads_each_viewer_dialog_once_after_all_native_persists() {
    let (storage, path) = open_storage().await;
    let mut config = ImConfig::default();
    config.auth_user_id = VIEWER_ID.to_string();
    config.company_id = "tenant-1".to_string();
    let mut module = ImModule::new(config);
    for channel_id in [CHANNEL_ID, CHANNEL_ID_2] {
        module.register_channel(
            helix_im::state::ChannelId::from_str(channel_id).expect("canonical channel id"),
            0,
        );
    }
    let mut sink = EffectSink::new();

    module
        .handle(&pong_gap_tick(), 1_000, &mut sink)
        .expect("pong gap handle");
    let sync_corrs: Vec<_> = sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(*corr),
            _ => None,
        })
        .collect();
    assert_eq!(sync_corrs.len(), 2);

    let mut persist_rounds = Vec::new();
    for (index, sync_corr) in sync_corrs.into_iter().enumerate() {
        sink.clear();
        let channel_id = if index == 0 { CHANNEL_ID } else { CHANNEL_ID_2 };
        module
            .handle(
                &Tick::PortReply {
                    corr: sync_corr,
                    outcome: PortOutcome::Ok(ReplyBytes(sync_reply_for(
                        channel_id,
                        index as u64 + 1,
                    ))),
                },
                2_000 + index as u64,
                &mut sink,
            )
            .expect("pong sync reply");
        persist_rounds.push(take_persist(&sink, true));
    }

    let (first_corr, first_ops) = persist_rounds.remove(0);
    let first_outcome = execute_storage_ops_atomic(&storage, &first_ops).await;
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: first_corr,
                outcome: first_outcome,
            },
            3_000,
            &mut sink,
        )
        .expect("first pong persist");
    assert!(
        !sink.as_slice().iter().any(|effect| matches!(
            effect,
            Effect::Persist { ops, .. }
                if matches!(ops.as_slice(), [StorageOp::ScopedGet(spec)]
                    if spec.table == "channel_member")
        )),
        "fan-out 尚有 Persist 未完成时不得提前读回 dialog: {:?}",
        sink.as_slice()
    );

    let (second_corr, second_ops) = persist_rounds.remove(0);
    let second_outcome = execute_storage_ops_atomic(&storage, &second_ops).await;
    sink.clear();
    module
        .handle(
            &Tick::PortReply {
                corr: second_corr,
                outcome: second_outcome,
            },
            3_001,
            &mut sink,
        )
        .expect("second pong persist");
    let readbacks: Vec<_> = sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Persist { corr, ops }
                if matches!(ops.as_slice(), [StorageOp::ScopedGet(spec)]
                    if spec.table == "channel_member") =>
            {
                Some((*corr, ops.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(readbacks.len(), 2, "每个 dirty channel 只能读回一次");

    let mut projected_channels = Vec::new();
    for (read_corr, read_ops) in readbacks {
        let read_outcome = execute_storage_ops(&storage, &read_ops).await;
        sink.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr: read_corr,
                    outcome: read_outcome,
                },
                4_000,
                &mut sink,
            )
            .expect("pong dialog readback");
        projected_channels.extend(sink.as_slice().iter().filter_map(|effect| {
            match effect {
                Effect::Emit { event } => {
                    serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                        .ok()
                        .filter(|event| event["event"] == "im:channel:update")
                        .and_then(|event| event["data"]["channelId"].as_str().map(str::to_owned))
                }
                _ => None,
            }
        }));
    }
    projected_channels.sort();
    assert_eq!(
        projected_channels,
        vec![CHANNEL_ID.to_string(), CHANNEL_ID_2.to_string()]
    );

    drop(storage);
    let _ = std::fs::remove_file(path);
}

/// Sync type=2 只更新 authority 字段，不得改变 viewer 未读。
#[tokio::test]
async fn sync_type2_patches_native_message_without_unread_delta() {
    let (storage, path) = open_storage().await;
    storage
        .execute_raw("INSERT INTO message (temporary_id,id,channel_id,user_id,message,quick_reply) VALUES ('tmp-edit','msg-edit','chfixx00000000000000000001','sender-678','before','[]')")
        .await
        .expect("seed message");
    storage
        .execute_raw("INSERT INTO channel_member (channel_id,user_id,unread_count) VALUES ('chfixx00000000000000000001','viewer-444',5)")
        .await
        .expect("seed member");
    let channel_id = ChannelId::from_str(CHANNEL_ID).expect("channel id");
    let event = EventEnvelope::new(
        channel_id,
        Seq(2),
        EventKind::PostEdit,
        PostFields::default(),
    )
    .with_msg_id(Some("msg-edit".to_string()));
    let mut messages = HashMap::new();
    let mut edited = PostFields::default();
    edited.id = "msg-edit".to_string();
    edited.message = "after".to_string();
    edited.quick_reply = "[{\"emoji\":\"ok\",\"userIds\":[\"viewer-444\"]}]".to_string();
    messages.insert("msg-edit".to_string(), edited);
    let ops = helix_im::acl::to_effect::batch_upsert_events_with_messages_and_auth(
        &[event],
        &messages,
        VIEWER_ID,
    );
    assert!(
        ops.iter().all(|op| !matches!(
            op,
            StorageOp::GuardedBump(_) | StorageOp::ScopedGuardedBump(_)
        )),
        "type=2 写集不得包含 unread delta"
    );
    assert!(matches!(
        execute_storage_ops_atomic(&storage, &ops).await,
        PortOutcome::Ok(_)
    ));
    let member = storage
        .scoped_get(ScopedGetSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL_ID.to_string()),
            key_col: "user_id",
            key_val: SqlValue::Text(VIEWER_ID.to_string()),
        })
        .await
        .expect("member read")
        .expect("member row");
    assert!(member
        .iter()
        .any(|(name, value)| name == "unread_count" && matches!(value, SqlValue::Integer(5))));

    drop(storage);
    let _ = std::fs::remove_file(path);
}

/// Sync type=3/type=6 混批只更新撤回与 readBits，viewer 未读保持不变。
#[tokio::test]
async fn sync_type3_and_type6_apply_in_order_without_unread_delta() {
    let (storage, path) = open_storage().await;
    storage
        .execute_raw("INSERT INTO message (temporary_id,id,channel_id,user_id,message,read_bits,revoke) VALUES ('tmp-revoke','msg-revoke','chfixx00000000000000000001','sender-678','body','0',0)")
        .await
        .expect("seed message");
    storage
        .execute_raw("INSERT INTO channel_member (channel_id,user_id,unread_count) VALUES ('chfixx00000000000000000001','viewer-444',5)")
        .await
        .expect("seed member");
    let channel_id = ChannelId::from_str(CHANNEL_ID).expect("channel id");
    let revoke = EventEnvelope::new(
        channel_id,
        Seq(3),
        EventKind::PostRevoke,
        PostFields::default(),
    )
    .with_msg_id(Some("msg-revoke".to_string()));
    let read = EventEnvelope::new(
        channel_id,
        Seq(4),
        EventKind::PostRead,
        PostFields::default(),
    )
    .with_msg_id(Some("msg-revoke".to_string()));
    let mut read_fields = PostFields::default();
    read_fields.id = "msg-revoke".to_string();
    read_fields.read_bits = "1".to_string();
    let mut messages = HashMap::new();
    messages.insert("msg-revoke".to_string(), read_fields);
    let ops = helix_im::acl::to_effect::batch_upsert_events_with_messages_and_auth(
        &[revoke, read],
        &messages,
        VIEWER_ID,
    );
    assert!(matches!(
        execute_storage_ops_atomic(&storage, &ops).await,
        PortOutcome::Ok(_)
    ));
    let message = storage
        .get(GetSpec {
            table: "message",
            key_col: "temporary_id",
            key_val: SqlValue::Text("tmp-revoke".to_string()),
        })
        .await
        .expect("message read")
        .expect("message row");
    assert!(message
        .iter()
        .any(|(name, value)| name == "revoke" && matches!(value, SqlValue::Integer(1))));
    assert!(message
        .iter()
        .any(|(name, value)| name == "read_bits"
            && matches!(value, SqlValue::Text(bits) if bits == "1")));
    let member = storage
        .scoped_get(ScopedGetSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_val: SqlValue::Text(CHANNEL_ID.to_string()),
            key_col: "user_id",
            key_val: SqlValue::Text(VIEWER_ID.to_string()),
        })
        .await
        .expect("member read")
        .expect("member row");
    assert!(member
        .iter()
        .any(|(name, value)| name == "unread_count" && matches!(value, SqlValue::Integer(5))));

    drop(storage);
    let _ = std::fs::remove_file(path);
}
