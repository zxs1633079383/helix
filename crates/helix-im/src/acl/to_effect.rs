//! IM 概念 → core Effect 翻译（ACL-1 写出端）

use helix_core::{Effect, Correlation};
use helix_core::effect::{
    StorageOp, UpsertSpec, MonotonicUpsertSpec, Row, SqlValue, DomainEventBytes,
    HttpRequest,
};
use bytes::Bytes;
use crate::state::{ChannelId, TemporaryId, Seq};
use crate::sync_session::EventEnvelope;

/// 乐观落库：INSERT INTO messages ON CONFLICT(temporary_id) DO UPDATE
///
/// `conflict_key = "temporary_id"` 是 IM 业务键名，由此函数提供给 core，
/// core 只把它作为参数值传递，不理解其含义。
///
/// MAJ-5：`table` 和 `conflict_key` 改为 `&'static str` 常量，
/// 消除热路径的 `"messages".to_string()` 堆分配。
pub fn upsert_message(
    _temporary_id: &TemporaryId,
    row: Row,
    corr: Correlation,
) -> Effect {
    Effect::Persist {
        corr,
        ops: vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![row],
            conflict_key: Some("temporary_id"),
        })],
    }
}

/// advance_cursor：推进 per-channel 同步 cursor（fire-and-forget，写成功后调用）。
///
/// cursor 是 IM 概念，停留在 helix-im；底层用 core 的通用单调原语 `MonotonicUpsert` 表达。
/// `scope_key = channel_id` 是 IM 业务键名，core 不知道这是 channel。
pub fn advance_cursor(channel_id: ChannelId, target_seq: Seq) -> Effect {
    Effect::PersistFire {
        ops: vec![StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
            scope_key: channel_id.0.to_string(),
            value: target_seq.0 as i64,
        })],
    }
}

/// 触发 /sync/notify HTTP 请求
pub fn sync_notify(base_url: &str, channel_id: ChannelId, from_seq: Seq, corr: Correlation) -> Effect {
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "GET".to_string(),
            url: format!("{}/sync/notify?channel_id={}&from_seq={}",
                         base_url, channel_id.0, from_seq.0),
            headers: vec![],
            body: None,
        },
    }
}

/// 序列化 ImDomainEvent 为 DomainEventBytes（Emit 的载荷）
///
/// 此处是 im:__bus__ event 名的唯一生成点。
/// helix-core / helix-tauri 只接收已序列化的字节，不知道 "im:" 前缀。
pub fn emit_post_received(channel_id: ChannelId, server_msg_id: u64) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:received",
        "data": {
            "channel_id": channel_id.0,
            "msg_id": server_msg_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_received: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 发送乐观上屏事件（im:post:sending）
pub fn emit_post_sending(channel_id: ChannelId, temporary_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:sending",
        "data": {
            "channel_id": channel_id.0,
            "temporary_id": temporary_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_sending: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 发送 im:sync:too_long 事件（前端收到后清内存 buffer + 重拉首屏）
pub fn emit_sync_too_long(channel_id: ChannelId) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:sync:too_long",
        "data": {
            "channel_id": channel_id.0,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_sync_too_long: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 批量 upsert EventEnvelope 列表（sync 路径）
pub fn batch_upsert_events(events: &[EventEnvelope]) -> Vec<StorageOp> {
    events.iter().map(|ev| {
        StorageOp::BatchUpsert(UpsertSpec {
            table: "messages",
            rows: vec![vec![
                ("seq".to_string(), SqlValue::Integer(ev.seq.0 as i64)),
                ("channel_id".to_string(), SqlValue::Integer(ev.id.0 as i64)),
                ("event_type".to_string(), SqlValue::Integer(ev.kind.type_num() as i64)),
                ("raw".to_string(), SqlValue::Text(
                    String::from_utf8_lossy(ev.raw.as_ref()).to_string()
                )),
            ]],
            conflict_key: Some("seq"),
        })
    }).collect()
}
