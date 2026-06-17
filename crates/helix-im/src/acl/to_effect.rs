//! IM 概念 → core Effect 翻译（ACL-1 写出端）

use crate::state::{ChannelId, Seq, TemporaryId};
use crate::sync_session::EventEnvelope;
use bytes::Bytes;
use helix_core::effect::{
    DomainEventBytes, HttpRequest, MonotonicUpsertSpec, Row, StorageOp, TransportId, UpsertSpec,
};
use helix_core::{Correlation, Effect};

/// 出站 WS ping 帧（带单调 `seq`）。现网 Go `websocket_router.go:34 if r.Seq <= 0` 拒绝
/// seq≤0（"Invalid sequence" → 关连接），故每条出站 WS 帧必带 `seq = ++client_seq`。
/// 真 ping piggyback（cursors/allHash，§8 心跳）后续扩；当前最小 `{action,seq}` 保活。
pub fn ping_frame(seq: u64) -> Effect {
    let payload = serde_json::json!({ "action": "ping", "seq": seq });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("ping_frame: static JSON shape must not fail"),
    );
    Effect::Send {
        transport: TransportId::from_raw(0),
        frame: bytes,
    }
}

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
            table: "message",
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

// ─── C3-core：sync 驱动的 post:* mutation 事件 ────────────────────────────────
//
// 真源 = cses-client `apply_event_with_messages`（post.rs:1146-1448）。sync `events`
// 路径逐事件 dispatch，按 event_type + 是否命中 messages map（phantom 判定）emit
// 对应 `im:*` 业务事件。helix 无 `actorId` 解析（EventEnvelope 仅携 msg_id），故
// payload 携 `channel_id`/`event_seq`/`msg_id`——前端 im:__bus__ dispatcher 按 channel
// 分发，actor 维度暂缺（功能性需求，非 sync-v2 mutation 核心）。
//
// **信封铁律**：`{event, data}` 形态与既有 7 个 emit 一致（emit_post_received/sending/...），
// driver 侧由 event_sink 透传进 im:__bus__（host-cli 无窗口 → 以 DomainEvent 计数验，
// 真 app.emit 留 M2 接缝）。phantom 不 emit（与落库一致，HX-C008 cursor 独立推进不依赖可见性）。

/// C3：type=6 已读 echo → `im:post:read`（真源 post.rs:1413-1437，无须 body 恒 emit）。
///
/// 真源 emit `im:channel:read_echo`；helix spec C3 统一命名 `im:post:read`（mutation 语义）。
/// 服务端权威 read_bits 已经 C2 落库单调覆盖；本事件让前端刷新该 msg 的已读态 UI。
pub fn emit_post_read(channel_id: ChannelId, event_seq: u64, msg_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:read",
        "data": {
            "channel_id": channel_id.as_str(),
            "event_seq": event_seq,
            "msg_id": msg_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_read: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// C3：type=2 编辑 → `im:post:updated`（真源 post.rs:1337，**仅可见时** emit；phantom noop）。
pub fn emit_post_updated(channel_id: ChannelId, event_seq: u64, msg_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:updated",
        "data": {
            "channel_id": channel_id.as_str(),
            "event_seq": event_seq,
            "msg_id": msg_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_updated: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// C3：type=3 撤回 → `im:post:deleted`（真源 post.rs:1402，无须 body，有 msgId 即 emit）。
pub fn emit_post_deleted(channel_id: ChannelId, event_seq: u64, msg_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:deleted",
        "data": {
            "channel_id": channel_id.as_str(),
            "event_seq": event_seq,
            "msg_id": msg_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_deleted: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// C3：type=1 可见新消息 → `im:channel:update-by-post`（离线未读 +N，真源 post.rs:1204-1248）。
///
/// 真源走 `update_channel_from_post`（内含 sender 豁免：msg_user_id != auth_id 才 +1）后
/// emit `im:channel:update-by-post` 让 +N 落到前端 badge。helix-core 无 userId（不认识身份），
/// sender 豁免归 driver/前端裁决——本事件仅透传 channel_id + msg_id，前端按 auth 决定是否 +N。
pub fn emit_channel_update_by_post(channel_id: ChannelId, event_seq: u64, msg_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channel:update-by-post",
        "data": {
            "channel_id": channel_id.as_str(),
            "event_seq": event_seq,
            "msg_id": msg_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_update_by_post: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// C3：逐 sync 事件产出 mutation 事件 Effect（真源 dispatch table post.rs:1157-1448）。
///
/// phantom-aware（与 `batch_upsert_events_with_messages` 落库分桶同口径）：
/// - **type1 new**：`msg_id ∈ messages`（可见）→ `im:post:received` + `im:channel:update-by-post`
///   （真源 1252/1232，离线补未读 +N）；`∉`（phantom）→ **不 emit**（cursor 照推，UI 不渲染）。
/// - **type2 edit**：`msg_id ∈ messages`（可见）→ `im:post:updated`（真源 1337）；phantom → noop。
/// - **type3 revoke**：有 `msg_id` → `im:post:deleted`（真源 1402，无须 body）。
/// - **type6 read**：有 `msg_id` → `im:post:read`（真源 1428，无须 body 恒 emit）。
/// - **Other(n)**：前向兼容，不 emit。
///
/// 复杂度：每事件 O(1)（HashMap 查 + 1-2 个 Emit 构造），与落库批同遍历语义但分离关注点
/// （落库 = 状态变更；emit = UI 通知）。两者 phantom 判定一致：可见才动 UI（HX-C008）。
pub fn sync_mutation_emits(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
) -> Vec<Effect> {
    use crate::sync_session::EventKind;

    let mut emits = Vec::new();
    for ev in events {
        let Some(msg_id) = ev.msg_id.as_deref() else {
            // 无 msgId（read 事件可空 / legacy）→ 无可通知的 post，跳过。
            continue;
        };
        match ev.kind {
            // type1：可见（命中 map）才 emit received + channel update-by-post（离线未读 +N）。
            EventKind::PostUpsert => {
                if messages.contains_key(msg_id) {
                    emits.push(emit_post_received(ev.channel_id, ev.seq.0));
                    emits.push(emit_channel_update_by_post(ev.channel_id, ev.seq.0, msg_id));
                }
                // else phantom → 不 emit（UI 不渲染，真源 1270-1280）。
            }
            // type2：可见才 emit updated（phantom noop，真源 1346）。
            EventKind::PostEdit => {
                if messages.contains_key(msg_id) {
                    emits.push(emit_post_updated(ev.channel_id, ev.seq.0, msg_id));
                }
            }
            // type3：无须 body，有 msgId 即 emit deleted（真源 1367/1402）。
            EventKind::PostRevoke => {
                emits.push(emit_post_deleted(ev.channel_id, ev.seq.0, msg_id));
            }
            // type6：无须 body，恒 emit read（真源 1413-1437）。
            EventKind::PostRead => {
                emits.push(emit_post_read(ev.channel_id, ev.seq.0, msg_id));
            }
            // 前向兼容：未知类型不 emit（cursor 仍照推）。
            EventKind::Other(_) => {}
        }
    }
    emits
}

/// 批量 upsert EventEnvelope 列表（**inline 内容**路径：Snapshot / 直接事件态 / 测试帧）。
///
/// 收口到 `channel::event_to_upsert_op` —— 与 WS 推送路径**共用同一落库映射**，
/// 写向 678 真库 `message` 表（PK=temporary_id，与发送态同表同主键语义）。
/// 不再有 "messages" 影子表：sync 增量主链路落库后 cursor 正常推进。
///
/// **注意**：此函数把每个事件**无条件**落库（内容取自事件自身 `fields`）——用于内容内联在
/// 事件里的场景（Snapshot 重建、legacy 测试帧）。sync `events` 路径（内容在分离的 messages
/// map + phantom 语义）须用 `batch_upsert_events_with_messages`。
pub fn batch_upsert_events(events: &[EventEnvelope]) -> Vec<StorageOp> {
    events
        .iter()
        .map(crate::channel::event_to_upsert_op)
        .collect()
}

/// C2：批量落库 sync `events`——按 `messages` map 做 phantom / readBits / edit / revoke 分桶。
///
/// 真源 = cses-client `batch_write_events`（post.rs:1043-1134）。逐事件按 `event.msg_id` 在
/// `messages` map 中的命中情况分发（铁律：cursor 推进由调用方独立负责，phantom 不卡 cursor）：
///
/// - **type1 new**：`msg_id ∈ map` → 用 **map 内容**（含权威 read_bits）upsert message 行
///   （真源 1071-1078）；`∉ map` → **phantom**：不产 StorageOp（cursor 由 commit_target 照推，
///   HX-C008 §1.6）。
/// - **type2 edit**：`msg_id ∈ map` → `edit_content_op`（**仅覆盖内容列** UPDATE WHERE id，
///   保留本地 read_bits/send_status/segment_id，真源 `update_content_by_message_id`）；`∉` → noop。
///   **不**走 upsert——避免 edit 空 readBits 经 ON CONFLICT 擦回已读位（C2 BLOCKING-1）。
/// - **type3 revoke**：**无须** map body → 恒发 `revoke_op`（UPDATE revoke=1 WHERE id，真源 1086/1110）。
/// - **type6 read**：读 `map[msg_id].read_bits` → `apply_read_op`（**单调覆盖** read_bits 列，
///   真源 1087-1090/1116；map 无该 msg 或 read_bits 空 → 跳过，无可写位）。
/// - **Other(n)**：前向兼容，不落库（cursor 照推）。
///
/// 复杂度：每事件 O(1)（HashMap 查 + 单条 upsert/update 构造）；phantom/未命中分支零分配。
/// 内容取自 map 的 owned `PostFields`（HX-C005，parser 阶段已一次解析），落库零再解析。
pub fn batch_upsert_events_with_messages(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
) -> Vec<StorageOp> {
    use crate::sync_session::EventKind;

    let mut ops = Vec::with_capacity(events.len());
    for ev in events {
        // wire msgId（phantom 判定键）；缺省回退 fields 派生值（new 已填）。
        let msg_id = ev.msg_id.as_deref();
        match ev.kind {
            // type1 new：内容来自 messages map（命中才落，未命中=phantom noop）。
            // 全量 upsert（含 read_bits=excluded）——new 行不预存，无擦回风险。
            EventKind::PostUpsert => {
                if let Some(fields) = msg_id.and_then(|id| messages.get(id)) {
                    // 用 map 权威内容重建一个 EventEnvelope 视图（seq/channel 取事件，内容取 map），
                    // 复用 event_to_upsert_op 的同一落库映射（PK 三级回退 / read_bits 列对齐 678）。
                    let ev_with_body = crate::sync_session::EventEnvelope::new(
                        ev.channel_id,
                        ev.seq,
                        ev.kind.clone(),
                        fields.clone(),
                    );
                    ops.push(crate::channel::event_to_upsert_op(&ev_with_body));
                }
                // else: phantom（type1）→ 不落行，cursor 由调用方照推。
            }
            // type2 edit：命中 map → **仅覆盖内容列** UPDATE WHERE id（保留本地 read_bits/
            // send_status/segment_id，真源 update_content_by_message_id）；∉ map → noop。
            // 关键：edit **不**走 event_to_upsert_op 全量 upsert——否则 edit body 空 readBits
            // 会经 ON CONFLICT DO UPDATE 把已落地行的 read_bits 擦成空串（丢已读位，C2 BLOCKING-1）。
            EventKind::PostEdit => {
                if let Some(id) = msg_id {
                    if let Some(fields) = messages.get(id) {
                        ops.push(crate::channel::edit_content_op(id, fields));
                    }
                    // else: 不可见（type2 ∉ map）→ 不落行，cursor 照推。
                }
            }
            // type3 revoke：无须 body，按 msgId 命中行置 revoke=1（无 msgId 则跳过）。
            EventKind::PostRevoke => {
                if let Some(id) = msg_id {
                    ops.push(crate::channel::revoke_op(id));
                }
            }
            // type6 read：读 map[msgId].read_bits 单调覆盖；无 body / 空位 → 跳过。
            EventKind::PostRead => {
                if let Some(id) = msg_id {
                    if let Some(fields) = messages.get(id) {
                        if !fields.read_bits.is_empty() {
                            ops.push(crate::channel::apply_read_op(id, &fields.read_bits));
                        }
                    }
                }
            }
            // 前向兼容：未知类型不落库（cursor 照推）。
            EventKind::Other(_) => {}
        }
    }
    ops
}
