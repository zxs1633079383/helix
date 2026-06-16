//! IM 概念 → core Effect 翻译（ACL-1 写出端）

use crate::state::{ChannelId, Seq, TemporaryId};
use crate::sync_session::EventEnvelope;
use bytes::Bytes;
use helix_core::effect::{
    DomainEventBytes, HttpRequest, MonotonicUpsertSpec, Row, SqlValue, StorageOp, UpsertSpec,
};
use helix_core::{Correlation, Effect};

/// 乐观落库：INSERT INTO messages ON CONFLICT(temporary_id) DO UPDATE
///
/// `conflict_key = "temporary_id"` 是 IM 业务键名，由此函数提供给 core，
/// core 只把它作为参数值传递，不理解其含义。
///
/// MAJ-5：`table` 和 `conflict_key` 改为 `&'static str` 常量，
/// 消除热路径的 `"messages".to_string()` 堆分配。
pub fn upsert_message(_temporary_id: &TemporaryId, row: Row, corr: Correlation) -> Effect {
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
            scope_key: channel_id.as_str().to_string(),
            value: target_seq.0 as i64,
        })],
    }
}

/// 触发 /sync/notify HTTP 请求（pull 模型，设计文档 §1.4）。
///
/// 真 Go wire：`POST /channel/sync/notify`，body `{cursors:[{channelId, fromSeq}]}`。
/// helix 的 sync 链路（gate / 续拉 / proactive resync）逐 channel 发起，故 cursors
/// 数组恒含单元素（≤500 约束天然满足）。
///
/// A3b：`connection_id`（hello 回注，§A4）若 Some 则盖章进 headers——现网 IM HTTP
/// 身份头（`connectionId`）。core 不认识它，只透传字符串。
pub fn sync_notify(
    base_url: &str,
    channel_id: ChannelId,
    from_seq: Seq,
    corr: Correlation,
    connection_id: Option<&str>,
) -> Effect {
    let body = serde_json::json!({
        "cursors": [
            { "channelId": channel_id.as_str(), "fromSeq": from_seq.0 }
        ]
    });
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(connection_id_headers(connection_id));
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{}/channel/sync/notify", base_url),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).expect(
                "sync_notify: static JSON shape must not fail to serialize",
            ))),
        },
    }
}

/// B-rest：触发 increment HTTP（push 模型，设计文档 §1.3 / §3 Phase 2）。
///
/// `POST /channels/load/increment`，body `{timestamp:0, cursors:[{channelId, fromSeq}]}`
/// （根群带已知 channel 游标）。HTTP **仅 ack 无数据**，真增量走 WS `increment_channel`
/// 单播（→ handle_increment）。connectionId 身份头经 connection_id_headers 盖章。
pub fn increment_http_trigger(
    base_url: &str,
    cursors: &[(ChannelId, Seq)],
    corr: Correlation,
    connection_id: Option<&str>,
) -> Effect {
    let cursor_json: Vec<serde_json::Value> = cursors
        .iter()
        .map(
            |(ch, from_seq)| serde_json::json!({ "channelId": ch.as_str(), "fromSeq": from_seq.0 }),
        )
        .collect();
    let body = serde_json::json!({
        "timestamp": 0,
        "cursors": cursor_json,
    });
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(connection_id_headers(connection_id));
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{}/channels/load/increment", base_url),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).expect(
                "increment_http_trigger: static JSON shape must not fail to serialize",
            ))),
        },
    }
}

/// A3b：把 connectionId（若 Some）组装成 IM HTTP 身份头列表。
///
/// 单点收口——sync/notify、posts/create 共用，避免散落漏盖。
/// cookieId(=userId) 是 **driver 全局头**（host 注入），不在此（core 不认识 userId）。
pub fn connection_id_headers(connection_id: Option<&str>) -> Vec<(String, String)> {
    match connection_id {
        Some(id) => vec![("connectionId".to_string(), id.to_string())],
        None => vec![],
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
            "channel_id": channel_id.as_str(),
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
            "channel_id": channel_id.as_str(),
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
            "channel_id": channel_id.as_str(),
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

/// A4：发送 im:connection:established 事件（hello 握手完成信号）。
///
/// 现网：前端收到此事件 + connectionId 才放行 pendingSends（businessReady gate）。
/// 是 sync-v2「业务模块启动」类的首个事件，UC-1 验收依赖它可见。
pub fn emit_connection_established(connection_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:connection:established",
        "data": {
            "connectionId": connection_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_connection_established: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// B-rest：发送 im:channels:loaded 事件（hello 冷启动补齐信号，设计文档 §3 Phase 2）。
///
/// increment HTTP 触发后立即 emit（不在此等数据——真增量走 WS `increment_channel`）。
/// im:__bus__ 信封 array payload 形态：`data` 是数组时包 `{items}`（前端总线约定）。
pub fn emit_channels_loaded() -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channels:loaded",
        "data": { "items": [] }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channels_loaded: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// B-rest：发送 im:channel:increment 事件（设计文档 §3 Phase 3）。
///
/// 透传 `increment_channel` 帧 `data` 原始字节（含 mentionList/urgentPostList，前端直接消费）。
/// `raw` 是边界字节（零信任已在 parser 完成），此处仅透传不解释；若非合法 JSON 则降级为空对象。
pub fn emit_channel_increment(channel_id: ChannelId, raw: &[u8]) -> Effect {
    use serde_json::json;
    // raw 透传：parser 产出的 increment data 字节。非法 JSON 时降级 null（不 panic）。
    let inc_data: serde_json::Value =
        serde_json::from_slice(raw).unwrap_or(serde_json::Value::Null);
    let payload = json!({
        "event": "im:channel:increment",
        "data": {
            "channel_id": channel_id.as_str(),
            "increment": inc_data,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_increment: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// B-rest：发送 im:channel:update 事件（设计文档 §3 Phase 4，子 topic 批次结束）。
pub fn emit_channel_update(channel_id: ChannelId) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channel:update",
        "data": { "channel_id": channel_id.as_str() }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_update: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 批量 upsert EventEnvelope 列表（sync 路径）
pub fn batch_upsert_events(events: &[EventEnvelope]) -> Vec<StorageOp> {
    events
        .iter()
        .map(|ev| {
            StorageOp::BatchUpsert(UpsertSpec {
                table: "messages",
                rows: vec![vec![
                    ("seq".to_string(), SqlValue::Integer(ev.seq.0 as i64)),
                    (
                        "channel_id".to_string(),
                        SqlValue::Text(ev.id.as_str().to_string()),
                    ),
                    (
                        "event_type".to_string(),
                        SqlValue::Integer(ev.kind.type_num() as i64),
                    ),
                    (
                        "raw".to_string(),
                        SqlValue::Text(String::from_utf8_lossy(ev.raw.as_ref()).to_string()),
                    ),
                ]],
                conflict_key: Some("seq"),
            })
        })
        .collect()
}
