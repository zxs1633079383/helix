//! Event-to-storage mappings for sync and online WS paths.

use crate::sync_session::EventEnvelope;
use helix_core::effect::StorageOp;

// ─── 辅助函数 ────────────────────────────────────────────────────────────────

/// 将 EventEnvelope 转为落库 StorageOp（批量 upsert，ON CONFLICT(temporary_id) DO UPDATE）。
///
/// ## sync 全量落库映射
///
/// 写向 678 真库 `message` 表（PK=temporary_id），与发送态完全同表同主键语义——
/// 不再有 "messages" 影子表。sync/B-rest 增量经此函数收口，在线 WS 稀疏 echo 通过
/// [`event_to_online_upsert_op`] 复用本函数的列映射，但使用“缺省富字段保留”策略。
///
/// ## 字段映射（真源 = cses-client `From<types::Post> for Message`）
///
/// 字段提取已上移 parser（`extract_post_fields`，`Value` 解析时一次完成），
/// 本函数只读 `ev.fields`（owned typed 视图）拼装 Row——**零再解析、零额外字符串分配**
/// （列名 `&'static str`，字段值 owned 所有权转移）。
///
/// - `temporary_id`（PK）：`fields.temporary_id` ?? `fields.id` ?? `evt:{channel}:{seq}` 合成键。
///   三级回退保证 PK 恒非空、且对同一事件**确定性幂等**（re-sync 同批 = 同 PK 覆盖 = O(1)，
///   不产生重复行、不引发 sync 风暴）。
/// - `channel_id`：`fields.channel_id` 缺省时回退 envelope 权威 channel_id（parser 已校验）。
/// - `props`：保留服务端原始 props 对象（业务扩展）。
/// - `event_seq`：协议级稳定字段独立列。实时 WS/sync 的非零序号覆盖旧值；不携序号的
///   历史 HTTP 回填排除该列，不能把已持久化的权威序号擦成 0。
///
/// ## 复杂度（诚实标注，HX-C005）
///
/// - **SQL 写**：单条 INSERT ... ON CONFLICT(temporary_id) DO UPDATE = O(1)，无 read-modify-write。
/// - **解析**：0（字段已在 parser `extract_post_fields` 一次提取，本函数不再 `from_slice` 重解析）。
/// - **分配**：字段值由 `ev.fields` clone 出（每条 ~7 个 String clone + 8 个列名 `.to_string()`，
///   均为落库必需的 owned Row 构造，非冗余重解析）；PK 合成兜底分支仅在无 id 时分配一次。
///   注：列名仍走 `String`（`Row = Vec<(String, SqlValue)>` 是 core 类型契约，`convert_row`
///   从 rusqlite 读运行时列名同样是 String；改 `&'static str` 需 core 级动核且破坏 driver
///   codec——超本 drift 修复白名单，见 MAJ-5 仅 table/conflict_key 静态化的同因取舍）。
/// 将 sync EventEnvelope 编译为完整 upsert；冲突时按服务端全量字段覆盖。
pub fn event_to_upsert_op(ev: &EventEnvelope) -> StorageOp {
    event_to_upsert_op_with_policy(ev, false, false)
}

/// 将在线 WS post 编译为“缺省富字段保留”的幂等 upsert。
///
/// 在线 `post` 广播可能是发送确认的稀疏 payload：同一 `temporary_id` 的旧行已经由
/// `/sync/notify` 写入 `expedite_map`、reply*、`read_bits` 等富字段，但回声只携带正文。
/// 冲突更新时排除这些缺省列，插入新行仍使用完整字段默认值；sync type=1 继续走上面的全量函数。
pub fn event_to_online_upsert_op(ev: &EventEnvelope) -> StorageOp {
    event_to_upsert_op_with_policy(ev, true, false)
}

/// 将 HTTP readback 编译为 presence-aware upsert，避免缺失字段擦除本地富字段。
pub fn event_to_readback_upsert_op(ev: &EventEnvelope) -> StorageOp {
    event_to_upsert_op_with_policy(ev, false, true)
}

/// 按来源策略拼装 message upsert，保证 sync 全量与在线稀疏回声共享同一列映射。
fn event_to_upsert_op_with_policy(
    ev: &EventEnvelope,
    preserve_empty_online_fields: bool,
    preserve_missing_readback_fields: bool,
) -> StorageOp {
    use helix_core::effect::{SqlValue, UpsertSpec};

    let f = &ev.fields;

    // channel_id 缺省回退到 envelope 权威 channel_id（parser 已校验，含正确性）。
    let channel_id = if f.channel_id.is_empty() {
        ev.channel_id.as_str().to_string()
    } else {
        f.channel_id.clone()
    };

    // PK 三级回退（真源 cses-client：temporary_id ?? id；再加 evt:{ch}:{seq} 合成兜底，
    // 保证收到的事件即便无 temporary_id 也有确定性非空 PK，re-sync 幂等覆盖同行）。
    let pk = if !f.temporary_id.is_empty() {
        f.temporary_id.clone()
    } else if !f.id.is_empty() {
        f.id.clone()
    } else {
        format!("evt:{}:{}", ev.channel_id.as_str(), ev.seq.0)
    };

    let mut exclude_from_update = vec!["send_status"];
    if ev.seq.0 == 0 {
        exclude_from_update.push("event_seq");
    }
    if preserve_empty_online_fields {
        // parser 对缺失 JSON 字段使用空值/0；在线冲突更新必须把它们当作“未提供”，
        // 否则稀疏 WS echo 会覆盖 sync 已落地的加急、回复、已读和身份快照。
        let reply_messages_missing = online_json_field_missing(&f.reply_messages);
        let preserve = [
            ("id", f.id.is_empty()),
            ("user_id", f.user_id.is_empty()),
            ("type", f.msg_type.is_empty()),
            ("message", f.message.is_empty()),
            ("viewers", f.viewers.is_empty()),
            ("user_snapshot", online_json_field_missing(&f.user_snapshot)),
            ("mentions", f.mentions.is_empty()),
            ("expedite_map", online_json_field_missing(&f.expedite_map)),
            ("topic", online_json_field_missing(&f.topic)),
            ("simple_message", f.simple_message.is_empty()),
            ("reply_id", f.reply_id.is_empty()),
            ("reply_root_id", f.reply_root_id.is_empty()),
            ("reply_first_level_id", f.reply_first_level_id.is_empty()),
            (
                "replied_message",
                online_json_field_missing(&f.replied_message),
            ),
            ("reply_messages", reply_messages_missing),
            ("reply_count", f.reply_count == 0 && reply_messages_missing),
            ("read_bits", f.read_bits.is_empty()),
            ("snapshot_id", f.snapshot_id.is_empty()),
            ("create_at", f.create_at == 0),
            ("update_at", f.update_at == 0),
        ];
        exclude_from_update.extend(
            preserve
                .into_iter()
                .filter_map(|(column, should_preserve)| should_preserve.then_some(column)),
        );
    }
    if preserve_missing_readback_fields {
        // HTTP readback 与在线回声不同：只有 JSON 明确提供的字段才能覆盖已有行。
        // parser 的默认空值不代表服务端明确清空，presence mask 保留该语义差异。
        let preserve = [
            ("id", !f.has_field(crate::sync_session::POST_FIELD_ID)),
            (
                "channel_id",
                !f.has_field(crate::sync_session::POST_FIELD_CHANNEL_ID),
            ),
            (
                "user_id",
                !f.has_field(crate::sync_session::POST_FIELD_USER_ID),
            ),
            ("type", !f.has_field(crate::sync_session::POST_FIELD_TYPE)),
            (
                "message",
                !f.has_field(crate::sync_session::POST_FIELD_MESSAGE),
            ),
            (
                "simple_message",
                !f.has_field(crate::sync_session::POST_FIELD_SIMPLE_MESSAGE),
            ),
            ("props", !f.has_field(crate::sync_session::POST_FIELD_PROPS)),
            (
                "user_snapshot",
                !f.has_field(crate::sync_session::POST_FIELD_USER_SNAPSHOT),
            ),
            (
                "viewers",
                !f.has_field(crate::sync_session::POST_FIELD_VIEWERS),
            ),
            (
                "mentions",
                !f.has_field(crate::sync_session::POST_FIELD_MENTIONS),
            ),
            (
                "expedite_map",
                !f.has_field(crate::sync_session::POST_FIELD_EXPEDITE_MAP),
            ),
            ("topic", !f.has_field(crate::sync_session::POST_FIELD_TOPIC)),
            (
                "quick_reply",
                !f.has_field(crate::sync_session::POST_FIELD_QUICK_REPLY),
            ),
            (
                "reply_id",
                !f.has_field(crate::sync_session::POST_FIELD_REPLY_ID),
            ),
            (
                "reply_root_id",
                !f.has_field(crate::sync_session::POST_FIELD_REPLY_ROOT_ID),
            ),
            (
                "reply_first_level_id",
                !f.has_field(crate::sync_session::POST_FIELD_REPLY_FIRST_LEVEL_ID),
            ),
            (
                "replied_message",
                !f.has_field(crate::sync_session::POST_FIELD_REPLIED_MESSAGE),
            ),
            (
                "reply_messages",
                !f.has_field(crate::sync_session::POST_FIELD_REPLY_MESSAGES),
            ),
            (
                "reply_count",
                !f.has_field(crate::sync_session::POST_FIELD_REPLY_COUNT),
            ),
            (
                "read_bits",
                !f.has_field(crate::sync_session::POST_FIELD_READ_BITS),
            ),
            (
                "snapshot_id",
                !f.has_field(crate::sync_session::POST_FIELD_SNAPSHOT_ID),
            ),
            (
                "create_at",
                !f.has_field(crate::sync_session::POST_FIELD_CREATE_AT),
            ),
            (
                "update_at",
                !f.has_field(crate::sync_session::POST_FIELD_UPDATE_AT),
            ),
        ];
        exclude_from_update.extend(
            preserve
                .into_iter()
                .filter_map(|(column, should_preserve)| should_preserve.then_some(column)),
        );
    }

    StorageOp::BatchUpsert(UpsertSpec {
        table: "message",
        rows: vec![vec![
            ("temporary_id".to_string(), SqlValue::Text(pk)),
            ("id".to_string(), SqlValue::Text(f.id.clone())),
            ("channel_id".to_string(), SqlValue::Text(channel_id)),
            ("user_id".to_string(), SqlValue::Text(f.user_id.clone())),
            ("type".to_string(), SqlValue::Text(f.msg_type.clone())),
            ("message".to_string(), SqlValue::Text(f.message.clone())),
            ("props".to_string(), SqlValue::Text(f.props.clone())),
            (
                "viewers".to_string(),
                SqlValue::Text(serde_json::json!(f.viewers.clone()).to_string()),
            ),
            (
                "user_snapshot".to_string(),
                SqlValue::Text(f.user_snapshot.clone()),
            ),
            (
                "mentions".to_string(),
                SqlValue::Text(serde_json::json!(f.mentions.clone()).to_string()),
            ),
            (
                "expedite_map".to_string(),
                SqlValue::Text(f.expedite_map.clone()),
            ),
            ("topic".to_string(), SqlValue::Text(f.topic.clone())),
            (
                "simple_message".to_string(),
                SqlValue::Text(f.simple_message.clone()),
            ),
            ("reply_id".to_string(), SqlValue::Text(f.reply_id.clone())),
            (
                "reply_root_id".to_string(),
                SqlValue::Text(f.reply_root_id.clone()),
            ),
            (
                "reply_first_level_id".to_string(),
                SqlValue::Text(f.reply_first_level_id.clone()),
            ),
            (
                "replied_message".to_string(),
                SqlValue::Text(f.replied_message.clone()),
            ),
            (
                "reply_messages".to_string(),
                SqlValue::Text(f.reply_messages.clone()),
            ),
            ("reply_count".to_string(), SqlValue::Integer(f.reply_count)),
            ("read_bits".to_string(), SqlValue::Text(f.read_bits.clone())),
            (
                "snapshot_id".to_string(),
                SqlValue::Text(f.snapshot_id.clone()),
            ),
            ("event_seq".to_string(), SqlValue::Integer(ev.seq.0 as i64)),
            (
                "send_status".to_string(),
                SqlValue::Text("sent".to_string()),
            ),
            ("create_at".to_string(), SqlValue::Integer(f.create_at)),
            ("update_at".to_string(), SqlValue::Integer(f.update_at)),
        ]],
        conflict_key: Some("temporary_id"),
        // 服务端 WS post 已是权威送达事实：新行直接写 sent。若 temporary_id 命中本地
        // 乐观行，则保留本地发送态，由 PendingSend::reconcile 在同一处理链显式落定 sent
        // 并撤销超时 timer，避免普通回放/重放越权覆盖 failed/unsend。
        exclude_from_update,
    })
}

/// 将在线 JSON 富字段的空对象/数组视为缺省，避免稀疏 echo 覆盖已同步的绝对态。
fn online_json_field_missing(value: &str) -> bool {
    value.is_empty() || value == "{}" || value == "[]"
}

/// type=2 编辑：**仅覆盖内容列**，保留本地权威列（C2 真源 message_repo.rs:185-221
/// `update_content_by_message_id`）。
///
/// 真源 cses edit 走 `UPDATE message SET <内容列> WHERE id = msg_id`，**显式不含**
/// `read_bits` / `send_status`（本地权威列）+ 身份列（id/channel_id/user_id/
/// create_at/temporary_id）——对齐 Telegram `applyEdit`「只改内容、不碰已读态」。
///
/// 为何**不能**复用 `event_to_upsert_op`：后者全量 upsert（含 `read_bits=excluded.read_bits`），
/// edit body 的 readBits 常为空（map body 无已读位 → unwrap_or_default → ""），upsert 的
/// ON CONFLICT DO UPDATE 会把已落地行的 read_bits **擦成空串** → 丢已读位（与真源 + 本仓 spec
/// 双重不一致）。改用 `BatchUpdate`（WHERE id IN）做**字段级 patch**，read_bits 列不在 patch
/// 中 → 已读位保留。本地无此行 → no-op（真源 0 行命中=未覆盖，留段查询/远端补拉）。
///
/// patch 范围 = helix `message` 表中 PostFields 携带的内容列：`type` / `message` / `props`，
/// 以及仅在 wire 明确携带时更新的 `expedite_map`。后者承载 G-07 加急/确认终态；缺字段时不写，
/// 避免普通编辑把既有加急状态擦空。G-06 使用独立单列写，不经过此旧内容 patch。
/// 复杂度 O(1)（UPDATE WHERE id，idx_message_id 命中）。
pub fn edit_content_op(msg_id: &str, fields: &crate::sync_session::PostFields) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    let mut patch = vec![
        ("type".to_string(), SqlValue::Text(fields.msg_type.clone())),
        (
            "message".to_string(),
            SqlValue::Text(fields.message.clone()),
        ),
        ("props".to_string(), SqlValue::Text(fields.props.clone())),
    ];
    if !fields.expedite_map.is_empty() {
        patch.push((
            "expedite_map".to_string(),
            SqlValue::Text(fields.expedite_map.clone()),
        ));
    }
    if !fields.topic.is_empty() {
        patch.push(("topic".to_string(), SqlValue::Text(fields.topic.clone())));
    }
    if !fields.reply_id.is_empty() {
        patch.push((
            "reply_id".to_string(),
            SqlValue::Text(fields.reply_id.clone()),
        ));
    }
    if !fields.reply_root_id.is_empty() {
        patch.push((
            "reply_root_id".to_string(),
            SqlValue::Text(fields.reply_root_id.clone()),
        ));
    }
    if !fields.reply_first_level_id.is_empty() {
        patch.push((
            "reply_first_level_id".to_string(),
            SqlValue::Text(fields.reply_first_level_id.clone()),
        ));
    }
    if !fields.replied_message.is_empty() {
        patch.push((
            "replied_message".to_string(),
            SqlValue::Text(fields.replied_message.clone()),
        ));
    }
    if !fields.reply_messages.is_empty() {
        patch.push((
            "reply_messages".to_string(),
            SqlValue::Text(fields.reply_messages.clone()),
        ));
        patch.push((
            "reply_count".to_string(),
            SqlValue::Integer(fields.reply_count),
        ));
    }
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        // 仅内容列；显式排除 read_bits / send_status（本地权威）+ 身份列。
        patch,
    })
}

/// type=3 撤回：标记 message 行 `revoke=1`（C2 真源 post.rs:1110-1114 `batch_revoke`）。
///
/// 撤回**无须** messages body——直接按 wire msgId 命中行置 revoke。用 `BatchUpdate`
/// （`UPDATE message SET revoke=1 WHERE id IN (msg_id)`）：本地无此行 → no-op（真源
/// `mark_revoked` 返回 Ok(false)，幂等不报错）。**不**用 ON CONFLICT(id)——message 表
/// `id` 仅有非唯一索引 idx_message_id（无 UNIQUE 约束），upsert 冲突子句会运行时报错。
/// 复杂度 O(1)（UPDATE WHERE id，idx_message_id 命中）。
pub fn revoke_op(msg_id: &str) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        patch: vec![("revoke".to_string(), SqlValue::Integer(1))],
    })
}

/// 将在线 posts_update 的完整撤回 authority 覆盖到可变业务列并置 revoke。
pub fn revoke_authority_op(
    msg_id: &str,
    fields: crate::sync_session::PostFields,
    event_seq: u64,
) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    let crate::sync_session::PostFields {
        msg_type,
        message,
        props,
        viewers,
        simple_message,
        update_at,
        ..
    } = fields;
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        patch: vec![
            ("type".to_string(), SqlValue::Text(msg_type)),
            ("message".to_string(), SqlValue::Text(message)),
            ("props".to_string(), SqlValue::Text(props)),
            (
                "viewers".to_string(),
                SqlValue::Text(serde_json::json!(viewers).to_string()),
            ),
            ("simple_message".to_string(), SqlValue::Text(simple_message)),
            ("event_seq".to_string(), SqlValue::Integer(event_seq as i64)),
            ("update_at".to_string(), SqlValue::Integer(update_at)),
            ("revoke".to_string(), SqlValue::Integer(1)),
        ],
    })
}

/// type=6 已读：**单调覆盖** message 行 `read_bits` 列（C2 真源 post.rs:1116-1120 `batch_apply_read`）。
///
/// read_bits 是服务端权威已读位（从 `messages[msg_id].read_bits` 取），直接赋值**非累加**
/// （HX-C008：read_bits 单调覆盖非累加）。用 `BatchUpdate`（`UPDATE message SET read_bits=?
/// WHERE id IN (msg_id)`）：本地无此行 → no-op（消息体尚未落地，待后续 type1 sync 带其
/// 自身 read_bits 落库）。同样**不**用 ON CONFLICT(id)（message.id 无 UNIQUE 约束）。复杂度 O(1)。
pub fn apply_read_op(msg_id: &str, read_bits: &str) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        patch: vec![(
            "read_bits".to_string(),
            SqlValue::Text(read_bits.to_string()),
        )],
    })
}

/// post_pin 置顶态：按 message id 幂等 patch props.pinned。
///
/// 本地 message 表无独立 pinned 列，且 post_pin 帧只携 `{postId, channelId, operation}`，没有完整
/// post body；因此只写最小 render-ready 元数据，不复用 `edit_content_op`，避免把正文/type 覆盖为空。
pub fn pin_state_op(msg_id: &str, pinned: bool) -> StorageOp {
    use helix_core::effect::{BatchUpdateSpec, SqlValue};
    let props = serde_json::json!({ "pinned": pinned }).to_string();
    StorageOp::BatchUpdate(BatchUpdateSpec {
        table: "message",
        key_col: "id",
        key_vals: vec![SqlValue::Text(msg_id.to_string())],
        patch: vec![("props".to_string(), SqlValue::Text(props))],
    })
}

/// kind-aware 落库映射：按 `ev.kind` 分派到对应 StorageOp（gate apply/flush 用，BLOCKING-2）。
///
/// 现网真源 `apply_event_with_messages`（post.rs:1146）按 event_type 分发：type1=全量 upsert /
/// type2=内容 patch / type3=撤回 / type6=已读位覆盖。gate buffer 可缓冲任意 kind 的乱序事件
/// （post + edit/read/revoke 都经 `ch.ingest`），flush 必须按各自语义落库——**不能**对 edit/read
/// 也走全量 upsert（会擦本地权威 read_bits/send_status，正是 `edit_content_op` 注释所述擦位 bug）。
///
/// msg_id 取 `ev.msg_id`（wire 权威）?? `ev.fields.id`；edit/read/revoke 缺 msg_id → patch
/// WHERE id="" 命中 0 行 = 安全 no-op（边界零信任）。PostUpsert 用 PK 三级回退恒非空。
pub fn event_to_storage_op(ev: &EventEnvelope) -> StorageOp {
    use crate::sync_session::EventKind;
    let msg_id = ev
        .msg_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(ev.fields.id.as_str());
    match ev.kind {
        EventKind::PostUpsert => event_to_upsert_op(ev),
        EventKind::PostEdit => edit_content_op(msg_id, &ev.fields),
        EventKind::PostRevoke => revoke_op(msg_id),
        EventKind::PostRead => apply_read_op(msg_id, ev.fields.read_bits.as_str()),
        // type7 is intercepted by sync::flow and committed via PersistAtomic. Keep this defensive
        // no-op for callers that accidentally bypass that boundary; it must never materialize a post.
        EventKind::ChannelTerminalClosed => {
            StorageOp::BatchUpdate(helix_core::effect::BatchUpdateSpec {
                table: "message",
                key_col: "id",
                key_vals: Vec::new(),
                patch: Vec::new(),
            })
        }
        // 未知类型：cursor 推进但不落 message（phantom 同口径，HX-C008）。空 patch no-op。
        EventKind::Other(_) => StorageOp::BatchUpdate(helix_core::effect::BatchUpdateSpec {
            table: "message",
            key_col: "id",
            key_vals: vec![helix_core::effect::SqlValue::Text(msg_id.to_string())],
            patch: Vec::new(),
        }),
    }
}
