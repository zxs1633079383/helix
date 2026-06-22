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
            exclude_from_update: Vec::new(),
        })],
    }
}

/// S3 path1：channel 全量 upsert（INSERT … ON CONFLICT(id) DO UPDATE，52 列）。
///
/// 入参 `cols` = `channel_write::program_full` 产出的 `(列名, 值)` 行（含 JSON rename）；
/// `exclude_from_update` = 守卫 / 本地列（server None / 本地维护 → 冲突时不覆盖，保留既有值）。
/// 写非热路径（仅 increment_channel 帧 / 冷启动补齐触发），列名 `&'static str` → `String`
/// 转换在此完成（core `Row` 契约要求 String key）。复杂度 O(1)（单行 upsert WHERE PK=id）。
///
/// 用 `PersistFire`（幂等：ON CONFLICT 全列覆盖；丢失可由后续 increment / sync 重投覆盖恢复）。
pub fn upsert_channel_full(
    cols: Vec<(&'static str, helix_core::effect::SqlValue)>,
    exclude_from_update: Vec<&'static str>,
) -> Effect {
    let row: Row = cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Effect::PersistFire {
        ops: vec![StorageOp::BatchUpsert(UpsertSpec {
            table: "channel",
            rows: vec![row],
            conflict_key: Some("id"),
            exclude_from_update,
        })],
    }
}

/// S3 path2/3：channel 部分更新（UPDATE channel SET <cols> WHERE id=?）。
///
/// 入参 `cols` = `channel_write::collect_present`（path2）的白名单列集；空 cols → None（无写意图）。
/// 用 `BatchUpdate`（`UPDATE … SET … WHERE id IN (?)`，单 key）。本地无此行 → 0 行命中 no-op
/// （与现网 update_partial 同口径）。列名 `&'static str` → `String`。复杂度 O(1)（WHERE PK=id）。
///
/// 用 `PersistFire`（幂等：绝对列覆盖；丢失由后续 increment 全量 upsert 兜底）。
pub fn update_channel_partial(
    channel_id: ChannelId,
    cols: Vec<(&'static str, helix_core::effect::SqlValue)>,
) -> Option<Effect> {
    if cols.is_empty() {
        return None;
    }
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    let patch: Row = cols.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Some(Effect::PersistFire {
        ops: vec![StorageOp::BatchUpdate(BatchUpdateSpec {
            table: "channel",
            key_col: "id",
            key_vals: vec![SqlValue::Text(channel_id.as_str().to_string())],
            patch,
        })],
    })
}

/// S3 path3：新消息触发 channel 写——未读 +1 **SQL 自增**（守 HX-C005，禁读改写）+ lastPost 组。
///
/// 入参 = `channel_write::PostChannelUpdate`（纯计算决策）。落成单条 `GuardedBump`：
/// `UPDATE channel SET unread_count = unread_count + <delta>[, unread_post_id=?, last_post=?,
///  last_root_post_at=?] WHERE id=? AND <msg_create_at> > last_root_post_at`。
/// 守卫 `msg_create_at > last_root_post_at` 同时门控 lastPost 组与未读组（旧 / 重复消息不污染）。
/// `unread_delta==0`（自己消息 / 不可见）→ 自增 0，但 lastPost 组仍在守卫命中时写（与现网
/// 「可见但自己发」仍更新 last_post 一致；unread_post_id 仅 delta>0 时写）。
///
/// 用 `PersistFire`（幂等：守卫保证同消息重复投递不重复 +1）。`set_cols` 列名 `&'static str`。
pub fn bump_channel_unread(upd: &crate::channel_write::PostChannelUpdate) -> Effect {
    Effect::PersistFire {
        ops: vec![bump_channel_unread_op(upd)],
    }
}

/// `bump_channel_unread` 的 StorageOp 内核（A3：供 sync 应用路径并入同一 `Effect::Persist` 批，
/// 与 message 落库同 corr 落定 → DB unread 列写成功后 cursor 才推进，CAP-9 在线/离线同口径）。
///
/// `GuardedBump`（`SET unread_count=unread_count+delta, set_cols… WHERE id=? AND guard>last_root`）
/// 守卫保证旧 / 重复消息不重复 +1（幂等，HX-C005 O(1) SQL 自增禁 RMW）。`delta==0`（自己 /
/// 不可见）→ 自增 0，但守卫命中时 lastPost 组仍写（与现网一致）。
pub fn bump_channel_unread_op(upd: &crate::channel_write::PostChannelUpdate) -> StorageOp {
    use helix_core::effect::{GuardedBumpSpec, SqlValue};
    let mut set_cols: Row = Vec::with_capacity(3);
    // unread_post_id 仅在确有未读 +1（他人可见消息）时写绝对值。
    if let Some(ref pid) = upd.unread_post_id {
        set_cols.push(("unread_post_id".to_string(), SqlValue::Text(pid.clone())));
    }
    // lastPost 组（可见时；不可见 last_post 为空串，本函数据此跳过——对齐现网 update_last_post
    // 仅在可见时才赋值 last_post）。
    if !upd.last_post.is_empty() {
        set_cols.push((
            "last_post".to_string(),
            SqlValue::Text(upd.last_post.clone()),
        ));
    }
    if upd.has_schedule_post {
        set_cols.push(("has_schedule_post".to_string(), SqlValue::Integer(1)));
    }
    set_cols.push((
        "last_root_post_at".to_string(),
        SqlValue::Integer(upd.msg_create_at),
    ));
    StorageOp::GuardedBump(GuardedBumpSpec {
        table: "channel",
        key_col: "id",
        key_val: SqlValue::Text(upd.channel_id.as_str().to_string()),
        bump_col: "unread_count",
        bump_delta: upd.unread_delta,
        set_cols,
        guard_col: "last_root_post_at",
        guard_val: upd.msg_create_at,
    })
}

/// advance_cursor：推进 per-channel 同步 cursor（fire-and-forget，写成功后调用）。
///
/// cursor 是 IM 概念，停留在 helix-im；底层用 core 的通用单调原语 `MonotonicUpsert` 表达。
/// `scope_key = channel_id` 是 IM 业务键名，core 不知道这是 channel。
///
/// ## S2：目标表 = `channel_event_cursor`（byte 对齐现网 678）
///
/// 现网真源 `tables/channel_event_cursor.rs:49 upsert_cursor`：
/// `INSERT INTO channel_event_cursor (channel_id, last_event_seq, updated_at) … ON CONFLICT
///  (channel_id) DO UPDATE SET last_event_seq = MAX(existing, excluded), updated_at = excluded`。
/// 表名 / 列名作**数据**经 `MonotonicUpsertSpec` 传给 core（守 HX-C001，core 不解释）；
/// MAX guard 由 core MonotonicUpsert 语义保证（严格 >，cursor 不回退）；`updated_at` 由
/// driver 自身 ambient 时钟写穿（`touch_col=Some("updated_at")`，非权威镜像列，cursor 单一
/// 真源仍是 helix gate）。
pub fn advance_cursor(channel_id: ChannelId, target_seq: Seq) -> Effect {
    Effect::PersistFire {
        ops: vec![StorageOp::MonotonicUpsert(MonotonicUpsertSpec {
            table: "channel_event_cursor",
            key_col: "channel_id",
            value_col: "last_event_seq",
            touch_col: Some("updated_at"),
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
pub fn emit_post_received(channel_id: ChannelId, event_seq: u64, msg_id: &str) -> Effect {
    use serde_json::json;
    // 信封对齐同批 post:updated/deleted/read（D2 收敛）+ 现网真源 cses post.rs:1252
    // {channelId, eventSeq, msgId(字符串)}：msg_id 是 26 字符 Id26 字符串，非 event_seq 数字。
    let payload = json!({
        "event": "im:post:received",
        "data": {
            "channel_id": channel_id.as_str(),
            "event_seq": event_seq,
            "msg_id": msg_id,
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

/// 发送 im:sync:too_long 事件（前端收到后清内存 buffer + `getLatestPost` 重拉首屏 + `cursor=resetTo-1` 记账）。
///
/// payload 对齐现网 `handlers/post.rs:918` 的 `{channelId, resetTo}`（前端 `im-sync.service.ts`
/// `onRustTooLong` 按此读：用 `channelId` 定位、`resetTo` 做本地 cursor 记账 `cursor=resetTo-1`）。
/// **铁律**：too_long 是覆盖式重拉（前端 getLatestPost 20 条 + batch_upsert 幂等覆盖），
/// helix/Rust 侧**不**物理 `DELETE FROM message`（现网语义，im-sync.service.ts 实证）。
pub fn emit_sync_too_long(channel_id: ChannelId, reset_to: Seq) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:sync:too_long",
        "data": {
            "channelId": channel_id.as_str(),
            "resetTo": reset_to.0,
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

/// 在线 `post_read`（type=6）→ `im:post:read`（真源 post.rs:157/167，WS 在线已读回声）。
///
/// 仅**在线** WS post_read handler 路径走此事件名（gate apply 经 `emit_for_kind`）。
/// **离线 sync 应用**的 type6 read 走另一事件名 `im:channel:read_echo`（见
/// `emit_channel_read_echo` / `sync_mutation_emits`，真源 post.rs:1428）——A5 修复：两路径
/// 按现网区分，不再统一命名（守 spec §0.3/§S9 前端零改动契约）。
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

/// 离线 sync 应用的 type=6 read → `im:channel:read_echo`（真源 post.rs:1428）。
///
/// **A5 修复**：现网区分两条 read 回声——在线 WS post_read 发 `im:post:read`（post.rs:157/167），
/// **sync 应用**的 type6 read 发 `im:channel:read_echo`（post.rs:1428）。此前 helix
/// `sync_mutation_emits` 统一发 `im:post:read`（违 spec §0.3/§S9 前端零改动契约——前端 read_echo
/// 订阅分支收不到）。本工厂供 sync 应用路径专用，与在线路径分流。
pub fn emit_channel_read_echo(channel_id: ChannelId, event_seq: u64, msg_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channel:read_echo",
        "data": {
            "channel_id": channel_id.as_str(),
            "event_seq": event_seq,
            "msg_id": msg_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_read_echo: static JSON shape must not fail to serialize"),
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

/// `posts_update`（批量编辑/撤回）→ `im:post:batch-updated`（真源 post.rs:254）。
///
/// payload 形态 = `{posts: [...]}`（前端 message_loop.rs:187/341 专门按此事件名 + `posts` 数组
/// 形态解析）。**A4 修复**：此前 posts_update 误用 `emit_post_updated`（事件名 `im:post:updated`
/// + payload `{event_seq,msg_id:''}`）——事件名与形态双错，前端批量编辑分支收不到。
/// `posts` 由调用方透传帧的 `data.posts` 数组（前端直接消费），非法/缺失降级空数组。
pub fn emit_post_batch_updated(channel_id: ChannelId, posts: &serde_json::Value) -> Effect {
    use serde_json::json;
    let posts = if posts.is_array() {
        posts.clone()
    } else {
        json!([])
    };
    let payload = json!({
        "event": "im:post:batch-updated",
        "data": {
            "channel_id": channel_id.as_str(),
            "posts": posts,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_batch_updated: static JSON shape must not fail to serialize"),
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
/// - **type6 read**：有 `msg_id` → `im:channel:read_echo`（真源 1428，sync 应用路径专用事件名；
///   A5：与在线 post_read 的 `im:post:read` 分流，无须 body 恒 emit）。
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
                    emits.push(emit_post_received(ev.channel_id, ev.seq.0, msg_id));
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
            // type6：无须 body，恒 emit read_echo（A5：sync 应用路径发 im:channel:read_echo，
            // 真源 1428；与在线 post_read 的 im:post:read 分流）。
            EventKind::PostRead => {
                emits.push(emit_channel_read_echo(ev.channel_id, ev.seq.0, msg_id));
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
///   保留本地 read_bits/send_status，真源 `update_content_by_message_id`）；`∉` → noop。
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
            // send_status，真源 update_content_by_message_id）；∉ map → noop。
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

/// A3/CAP-9：离线 sync 应用的可见 type1 新消息 → channel 未读 +1 **落 DB**（不只 emit）。
///
/// 真源 post.rs:1234 离线 sync apply 显式调 `update_channel_from_post`（内含 sender 豁免 +
/// 可见性门控）把 `unread_count` +1 落库——否则离线 >N 事件重连后 `store_query channel` 读出的
/// `unread_count` 列不含离线未读（UC-3 / 红点 +N 不达成）。此前 helix sync apply 仅 emit
/// `im:channel:update-by-post`（把 +N 推前端裁决），**DB 列不增** = A3 缺口。
///
/// 与在线 `post.rs` handler 的 `bump_channel_unread` 同口径（CAP-9 在线/离线对单条事件互斥，
/// 不重复 +1：在线帧已 reconcile 的本端 echo 不进 sync；他人消息在线已 +1 则 cursor 已过、
/// 不会再进 sync 区间）。phantom（msg_id ∉ messages）= 不可见内容 → 不 bump（与落库一致）。
/// 复杂度：每事件 O(1)（HashMap 查 + 谓词 + 单 GuardedBump 构造）。
pub fn sync_unread_bump_ops(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<StorageOp> {
    use crate::sync_session::EventKind;

    let mut ops = Vec::new();
    for ev in events {
        if !matches!(ev.kind, EventKind::PostUpsert) {
            continue;
        }
        // 仅可见 type1（命中 messages map）才参与未读决策——与落库 / emit 同 phantom 口径。
        let Some(fields) = ev.msg_id.as_deref().and_then(|id| messages.get(id)) else {
            continue;
        };
        let upd =
            crate::channel_write::post_updates_from_fields(ev.channel_id, fields, auth_user_id);
        // delta==0（自己 / 不可见）仍可能写 lastPost 组；GuardedBump 守卫幂等，安全并入批。
        ops.push(bump_channel_unread_op(&upd));
    }
    ops
}
