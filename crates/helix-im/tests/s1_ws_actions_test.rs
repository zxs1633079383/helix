//! S1 验收：缺失的 8 个 WS action handler 回放测试（spec §S1）。
//!
//! 回归锚: S1
//!
//! 覆盖 post_update / posts_update / post_read / channel_created / channel_close /
//! channel_member_update / update_channel_member_nickName / post_schedule_created /
//! post_schedule_canceled。每个：构造 WS 帧 → 断言 Effect 序列（落库 StorageOp + emit）。
//!
//! 行为真源：现网 cses-client `handlers/{post,channel,schedule}.rs`（根 CLAUDE.md §4.5）。

mod common;
use common::*;

use helix_core::effect::StorageOp;
use helix_core::tick::InboundBytes;
use helix_core::{Effect, Tick};

// ─── 辅助 ─────────────────────────────────────────────────────────────────────

fn inbound(value: serde_json::Value) -> Tick {
    let bytes = serde_json::to_vec(&value).unwrap();
    Tick::Inbound(InboundBytes(bytes.into()))
}

/// 取本批所有 Emit 的 event 名（保序）。
fn emit_names(sink: &helix_core::EffectSink) -> Vec<String> {
    sink.as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => {
                let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                v.get("event").and_then(|s| s.as_str()).map(str::to_string)
            }
            _ => None,
        })
        .collect()
}

fn emit_data(sink: &helix_core::EffectSink, event_name: &str) -> Vec<serde_json::Value> {
    sink.as_slice()
        .iter()
        .filter_map(|e| match e {
            Effect::Emit { event } => {
                let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
                (v.get("event").and_then(|s| s.as_str()) == Some(event_name))
                    .then(|| v.get("data").cloned().unwrap_or_default())
            }
            _ => None,
        })
        .collect()
}

/// 取本批所有 StorageOp（展开 Persist/PersistFire 的 ops）。
fn storage_ops(sink: &helix_core::EffectSink) -> Vec<StorageOp> {
    sink.as_slice()
        .iter()
        .flat_map(|e| match e {
            Effect::Persist { ops, .. } | Effect::PersistFire { ops } => ops.clone(),
            _ => Vec::new(),
        })
        .collect()
}

/// 断言：存在一个写 `table` 表、含 `col` 列且值匹配的 BatchUpdate/BatchUpsert。
fn has_write_to_table(ops: &[StorageOp], table: &str) -> bool {
    ops.iter().any(|op| match op {
        StorageOp::BatchUpdate(s) => s.table == table,
        StorageOp::BatchUpsert(s) => s.table == table,
        _ => false,
    })
}

fn patch_value<'a>(
    ops: &'a [StorageOp],
    table: &str,
    col: &str,
) -> Option<&'a helix_core::effect::SqlValue> {
    ops.iter().find_map(|op| match op {
        StorageOp::BatchUpdate(s) if s.table == table => {
            s.patch.iter().find(|(k, _)| k == col).map(|(_, v)| v)
        }
        _ => None,
    })
}

fn upsert_row_value<'a>(
    ops: &'a [StorageOp],
    table: &str,
    col: &str,
) -> Option<&'a helix_core::effect::SqlValue> {
    ops.iter().find_map(|op| match op {
        StorageOp::BatchUpsert(s) if s.table == table => s
            .rows
            .first()
            .and_then(|row| row.iter().find(|(k, _)| k == col).map(|(_, v)| v)),
        _ => None,
    })
}

// ─── update_channel（member 维度 dialog 绝对态）───────────────────────────────

/// 回归锚: member-dialog —— Go `update_channel` 定向推送是当前用户 member 维度绝对态。
#[test]
fn update_channel_writes_member_dialog_absolute_and_emits_projection() {
    let mut h = Harness::new_with_identity(&[], "u-self", "", "", "", "");
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "update_channel",
            "data": {
                "channel": {
                    "id": cid(18),
                    "userId": "u-self",
                    "unreadCount": 7,
                    "mentionCount": 2,
                    "mentionCountRoot": 1,
                    "mentionList": ["post-a", "post-b"],
                    "urgentCount": 1,
                    "urgentPostList": ["post-u"],
                    "msgCount": 42,
                    "lastReadSeq": 9,
                    "lastPost": {
                        "id": "post-last",
                        "userId": "u-sender",
                        "message": "hello member"
                    }
                }
            }
        })),
        1_000,
    );

    let ops = storage_ops(&h.sink);
    assert!(
        matches!(
            upsert_row_value(&ops, "channel_member", "unread_count"),
            Some(helix_core::effect::SqlValue::Integer(7))
        ),
        "update_channel member 绝对 unread 须写 channel_member，ops: {ops:?}"
    );
    assert!(
        matches!(
            upsert_row_value(&ops, "channel_member", "mention_count"),
            Some(helix_core::effect::SqlValue::Integer(2))
        ),
        "mention_count 是 member 维度，不能只写 channel，ops: {ops:?}"
    );
    assert!(
        matches!(
            upsert_row_value(&ops, "channel_member", "urgent_mention_count"),
            Some(helix_core::effect::SqlValue::Integer(1))
        ),
        "urgent_count 须映射 channel_member.urgent_mention_count，ops: {ops:?}"
    );
    let composite = ops.iter().any(|op| matches!(
        op, StorageOp::BatchUpsert(s) if s.table == "channel_member" && s.conflict_key == Some("channel_id,user_id")
    ));
    assert!(composite, "member dialog patch 须用复合 PK，ops: {ops:?}");

    let updates = emit_data(&h.sink, "im:channel:update-by-post");
    assert!(
        updates.iter().any(|data| {
            data["op"] == "dialog.patchByMember"
                && data["projectionAuthority"] == "server-member-absolute"
                && data["dialogPatch"]["unreadCount"] == 7
                && data["dialogPatch"]["mentionCount"] == 2
                && data["dialogPatch"]["urgentCount"] == 1
                && data["senderUserId"] == "u-sender"
        }),
        "update_channel 须 emit 指令化 member dialog 投影，updates: {updates:?}"
    );
}

// ─── post_read（type=6 覆盖式 read_bits）─────────────────────────────────────

/// 回归锚: S1 —— post_read 覆盖式写 read_bits（UPDATE message SET read_bits WHERE id）+ emit。
///
/// BLOCKING-2：post_read cursor 经 channel gate（严格 +1）。Harness 种 cursor=0，故帧 event_seq=1
/// （== cursor+1）才走立即 apply 路径落库 + emit；非相邻 seq 会进 gate buffer（缺口 arm 1s gate，
/// 不落库），那是另一组乱序回放测试覆盖的场景。
#[test]
fn post_read_overwrites_read_bits_and_emits_s1() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "post_read",
            "channel_event_seq": 1,
            "data": {
                "channelId": cid(1),
                "postId": "00000000000000000000000099",
                "readMap": "1010",
                "event_seq": 1
            }
        })),
        1_000,
    );

    let ops = storage_ops(&h.sink);
    assert!(
        has_write_to_table(&ops, "message"),
        "post_read 须写 message 表"
    );
    assert!(
        matches!(patch_value(&ops, "message", "read_bits"), Some(helix_core::effect::SqlValue::Text(s)) if s == "1010"),
        "read_bits 覆盖式写入帧值，ops: {ops:?}"
    );
    assert!(
        emit_names(&h.sink).iter().any(|n| n == "im:post:read"),
        "post_read 须 emit im:post:read"
    );
}

/// 回归锚: S1 —— post_read **缺 event_seq**（后端真形态：read 是幂等全量覆盖回执，不产 sequenced
/// channel_event）仍须覆盖式写 read_bits + emit im:post:read（issue #14 L2 read-receipt 根因修复）。
///
/// 旧实现走 channel gate（`frame.event_seq()==None` → no-op），read echo 帧永不落库/永不 emit →
/// L2 ②④ 永红。新实现 post_read 走专用直路（绕 gate），缺 event_seq 也直接 apply_read_op + emit。
/// 守可证伪：把直路改回走 gate（要求 event_seq）→ 本测试 storage/emit 断言 FAIL。
#[test]
fn post_read_without_event_seq_still_writes_and_emits_s1() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "post_read",
            // 后端 post_read 广播帧真形态：data 仅 {channelId, postId, readMap, createAt, updateAt}，
            // **无任何 event_seq**（顶层 / data / props 都没有）。
            "data": {
                "channelId": cid(1),
                "postId": "00000000000000000000000077",
                "readMap": "1101",
                "createAt": 1_782_619_679_459_i64,
                "updateAt": 1_782_619_679_604_i64
            }
        })),
        1_000,
    );

    let ops = storage_ops(&h.sink);
    assert!(
        has_write_to_table(&ops, "message"),
        "缺 event_seq 的 post_read echo 仍须写 message 表（#14 根因），ops: {ops:?}"
    );
    assert!(
        matches!(patch_value(&ops, "message", "read_bits"), Some(helix_core::effect::SqlValue::Text(s)) if s == "1101"),
        "read_bits 覆盖式写入帧 readMap 值，ops: {ops:?}"
    );
    assert!(
        emit_names(&h.sink).iter().any(|n| n == "im:post:read"),
        "缺 event_seq 的 post_read echo 仍须 emit im:post:read（#14 根因）"
    );
}

/// 回归锚: S1 —— post_read 缺 postId → 不写 message（边界零信任 noop），仍可 emit。
#[test]
fn post_read_missing_post_id_no_message_write_s1() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "post_read",
            "data": { "channelId": cid(1), "readMap": "11", "event_seq": 3 }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    assert!(
        !has_write_to_table(&ops, "message"),
        "缺 postId 不写 message"
    );
}

// ─── post_update（type=2 内容编辑）─────────────────────────────────────────────

/// 回归锚: S1 —— post_update 走 edit_content_op（仅内容列 UPDATE，不擦 read_bits）+ emit updated。
///
/// BLOCKING-2：post_update cursor 经 channel gate（严格 +1）。Harness 种 cursor=0，故帧 event_seq=1
/// （== cursor+1）才走立即 apply 路径落库 + emit（gate 已 kind-aware 用 edit_content_op patch）。
#[test]
fn post_update_edits_content_only_and_emits_s1() {
    let mut h = Harness::new(&[2]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "post_update",
            "data": {
                "id": "00000000000000000000000088",
                "channelId": cid(2),
                "message": "edited text",
                "props": "{}",
                "expediteMap": {"recipients":{"444":{"status":0}}},
                "event_seq": 1
            }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    assert!(
        has_write_to_table(&ops, "message"),
        "post_update 须写 message"
    );
    // edit_content_op 是 BatchUpdate（非全量 upsert）→ patch 不含 read_bits（保留本地已读位）。
    let is_edit_update = ops.iter().any(|op| {
        matches!(
            op, StorageOp::BatchUpdate(s) if s.table == "message"
                && s.patch.iter().any(|(k, _)| k == "message")
                && !s.patch.iter().any(|(k, _)| k == "read_bits")
        )
    });
    assert!(
        is_edit_update,
        "post_update 须用 edit_content_op（不擦 read_bits），ops: {ops:?}"
    );
    assert!(
        ops.iter().any(|op| matches!(
            op, StorageOp::BatchUpdate(s) if s.table == "message"
                && s.patch.iter().any(|(k, v)| k == "expedite_map"
                    && matches!(v, helix_core::effect::SqlValue::Text(t) if t.contains("444")))
        )),
        "post_update 明确携带 expediteMap 时必须持久化，ops: {ops:?}"
    );
    assert!(emit_names(&h.sink).iter().any(|n| n == "im:post:updated"));
}

// ─── post_pin（消息置顶/取消置顶）──────────────────────────────────────────────

#[test]
fn post_pin_updates_pinned_props_and_emits_s1() {
    let mut h = Harness::new(&[2]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "post_pin",
            "data": {
                "channelId": cid(2),
                "postId": "00000000000000000000000055",
                "operation": "add"
            }
        })),
        1_000,
    );

    let ops = storage_ops(&h.sink);
    assert!(
        has_write_to_table(&ops, "message"),
        "post_pin 须 patch message 表，ops: {ops:?}"
    );
    assert!(
        matches!(patch_value(&ops, "message", "props"), Some(helix_core::effect::SqlValue::Text(s)) if s.contains("\"pinned\":true")),
        "post_pin 须把 pinned=true 写入 props，ops: {ops:?}"
    );
    assert!(
        emit_names(&h.sink).iter().any(|n| n == "im:post:updated"),
        "post_pin 须 emit im:post:updated"
    );
    let has_pinned_projection = h.sink.as_slice().iter().any(|e| match e {
        Effect::Emit { event } => {
            let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
            v["event"] == "im:post:updated" && v["data"]["pinned"] == true
        }
        _ => false,
    });
    assert!(has_pinned_projection, "post_pin 投影须携 pinned=true");
}

#[test]
fn post_notice_add_post_pin_updates_target_pinned_s1() {
    let target = "00000000000000000000000056";
    let mut h = Harness::new(&[2]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "post",
            "data": {
                "id": "00000000000000000000000057",
                "channelId": cid(2),
                "event_seq": 1,
                "type": "NOTICE",
                "message": "",
                "temporaryId": "00000000000000000000000057",
                "props": {
                    "channel_event_seq": 1,
                    "type": "addPostPin",
                    "content": serde_json::json!({
                        "id": target,
                        "channelId": cid(2),
                        "temporaryId": "tmp-pin-target",
                        "type": "TEXT",
                        "message": "pinned target"
                    }).to_string()
                },
                "readBits": "00",
                "userId": "SYS",
                "viewers": ["all"]
            }
        })),
        1_000,
    );

    let ops = storage_ops(&h.sink);
    assert!(
        matches!(patch_value(&ops, "message", "props"), Some(helix_core::effect::SqlValue::Text(s)) if s.contains("\"pinned\":true")),
        "addPostPin 系统通知须把目标消息 pinned=true 写入 props，ops: {ops:?}"
    );
    let has_target_update = h.sink.as_slice().iter().any(|e| match e {
        Effect::Emit { event } => {
            let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
            v["event"] == "im:post:updated"
                && v["data"]["msg_id"] == target
                && v["data"]["pinned"] == true
        }
        _ => false,
    });
    assert!(
        has_target_update,
        "addPostPin 系统通知须额外 emit 目标消息 im:post:updated pinned=true"
    );
}

// ─── posts_update（批量编辑 / 撤回）────────────────────────────────────────────

/// 回归锚: S1 —— posts_update 逐 post：revoke=true→revoke_op；否则 edit_content_op。
#[test]
fn posts_update_batch_revoke_and_edit_s1() {
    let mut h = Harness::new(&[3]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "posts_update",
            "data": {
                "channelId": cid(3),
                "event_seq": 11,
                "posts": [
                    { "id": "00000000000000000000000010", "channelId": cid(3), "revoke": true },
                    { "id": "00000000000000000000000011", "channelId": cid(3), "message": "edit2" }
                ]
            }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    // 撤回行 → patch revoke=1。
    assert!(
        matches!(
            patch_value(&ops, "message", "revoke"),
            Some(helix_core::effect::SqlValue::Integer(1))
        ),
        "posts_update revoke 行须置 revoke=1，ops: {ops:?}"
    );
    // 编辑行 → patch message。
    let has_edit = ops.iter().any(|op| matches!(
        op, StorageOp::BatchUpdate(s) if s.table == "message"
            && s.patch.iter().any(|(k, v)| k == "message" && matches!(v, helix_core::effect::SqlValue::Text(t) if t == "edit2"))
    ));
    assert!(has_edit, "posts_update 编辑行须写内容，ops: {ops:?}");
    // 回归锚: A4 —— posts_update emit im:post:batch-updated（非 im:post:updated），
    // payload 形态 {posts:[...]}（前端 message_loop.rs:187/341 专门按此事件名+形态解析）。
    assert!(
        emit_names(&h.sink)
            .iter()
            .any(|n| n == "im:post:batch-updated"),
        "posts_update 须 emit im:post:batch-updated（A4），got {:?}",
        emit_names(&h.sink)
    );
    assert!(
        !emit_names(&h.sink).iter().any(|n| n == "im:post:updated"),
        "posts_update 不应再发 im:post:updated（A4 修复前的错误事件名）"
    );
    // payload 含 posts 数组（透传帧）。
    let bus_has_posts = h.sink.as_slice().iter().any(|e| {
        matches!(
            e,
            helix_core::Effect::Emit { event }
                if {
                    let body = String::from_utf8_lossy(event.0.as_ref());
                    body.contains("im:post:batch-updated") && body.contains("\"posts\"")
                }
        )
    });
    assert!(bus_has_posts, "batch-updated payload 须含 posts 数组");
}

/// 回归锚: S1 —— posts_update 缺 posts 数组 → 整体 noop（不可识别 payload 静默忽略）。
#[test]
fn posts_update_missing_posts_array_noop_s1() {
    let mut h = Harness::new(&[3]);
    let effects = h.step(
        inbound(serde_json::json!({
            "action": "posts_update",
            "data": { "channelId": cid(3) }
        })),
        1_000,
    );
    assert!(effects.is_empty(), "缺 posts 数组应 noop，got: {effects:?}");
}

// ─── channel_created（path1 全量 upsert + 成员）──────────────────────────────

/// 回归锚: S1 —— channel_created 全量 upsert channel + memberChange.join 写 channel_member + emit。
#[test]
fn channel_created_full_upsert_and_members_s1() {
    let mut h = Harness::new(&[]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "channel_created",
            "data": {
                "id": cid(5),
                "type": "P",
                "displayName": "新群",
                "memberChange": { "join": [
                    { "id": "u-aaa", "teamId": "t1", "role": "MEMBER", "nickName": "甲" }
                ]}
            }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    assert!(
        has_write_to_table(&ops, "channel"),
        "channel_created 须全量 upsert channel"
    );
    assert!(
        matches!(upsert_row_value(&ops, "channel_member", "user_id"), Some(helix_core::effect::SqlValue::Text(s)) if s == "u-aaa"),
        "memberChange.join 须写 channel_member 行，ops: {ops:?}"
    );
    assert!(emit_names(&h.sink)
        .iter()
        .any(|n| n == "im:channel:created"));
}

/// 回归锚: member-leave —— memberChange.leave → BatchDelete channel_member 删离场成员
/// （复合 PK 作用域删，scope=channel_id 不跨群）。删 handler 的 collect_member_leaves→
/// delete_channel_members 接线 → 本测 FAIL（HX-C011 可证伪）。core 原无 Delete，本能力新加。
#[test]
fn channel_member_update_leave_deletes_member_row() {
    use helix_core::effect::{SqlValue, StorageOp};
    let mut h = Harness::new(&[]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "channel_member_update",
            "data": {
                "id": cid(7),
                "type": "P",
                "memberChange": { "leave": [ { "id": "u-gone" } ] }
            }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    let found = ops.iter().any(|op| match op {
        StorageOp::BatchDelete(s) => {
            s.table == "channel_member"
                && s.scope_col == "channel_id"
                && matches!(&s.scope_val, SqlValue::Text(c) if *c == cid(7))
                && s.key_col == "user_id"
                && s.key_vals
                    .iter()
                    .any(|v| matches!(v, SqlValue::Text(u) if u == "u-gone"))
        }
        _ => false,
    });
    assert!(
        found,
        "memberChange.leave 须产 BatchDelete(channel_member, scope=channel_id, user_id∋u-gone)，ops: {ops:?}"
    );
}

/// 回归锚: A1 —— increment_channel 帧 members/owner/adminUsers/boss 数组 → 写 channel_member 表。
///
/// 此前 apply_increment 只 path1 主表 upsert（成员列空字面量）+ emit，从不写 channel_member 表
/// → 增量帧成员/owner/管理员/boss 全丢，读路径 assemble_channel 读出空成员。
#[test]
fn increment_channel_writes_channel_member_table_a1() {
    let mut h = Harness::new(&[]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "increment_channel",
            "data": {
                "id": cid(9),
                "type": "P",
                "displayName": "增量群",
                "lastEventSeq": 100,
                "members": [{ "id": "u-mem", "teamId": "t1" }],
                "adminUsers": [{ "id": "u-adm" }],
                "boss": [{ "id": "u-boss" }],
                "owner": { "id": "u-own", "nickName": "群主" }
            }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    // path1 主表仍 upsert channel。
    assert!(
        has_write_to_table(&ops, "channel"),
        "increment 须 path1 upsert channel"
    );
    // 成员真值进 channel_member 表（A1 修复核心）。
    assert!(
        has_write_to_table(&ops, "channel_member"),
        "increment 帧四源成员须写 channel_member 表（A1），ops: {ops:?}"
    );
    // 四角色各一行：从 channel_member 的 upsert rows 提取 user_id+role 校验。
    let member_rows: Vec<(String, String)> = ops
        .iter()
        .filter_map(|op| match op {
            StorageOp::BatchUpsert(s) if s.table == "channel_member" => Some(s.rows.clone()),
            _ => None,
        })
        .flatten()
        .filter_map(|row| {
            let uid = row
                .iter()
                .find(|(k, _)| k == "user_id")
                .and_then(|(_, v)| match v {
                    helix_core::effect::SqlValue::Text(t) => Some(t.clone()),
                    _ => None,
                })?;
            let role = row
                .iter()
                .find(|(k, _)| k == "role")
                .and_then(|(_, v)| match v {
                    helix_core::effect::SqlValue::Text(t) => Some(t.clone()),
                    _ => None,
                })?;
            Some((uid, role))
        })
        .collect();
    let role_of = |uid: &str| {
        member_rows
            .iter()
            .find(|(u, _)| u == uid)
            .map(|(_, r)| r.as_str())
    };
    assert_eq!(role_of("u-mem"), Some("MEMBER"));
    assert_eq!(role_of("u-adm"), Some("ADMIN"));
    assert_eq!(role_of("u-boss"), Some("BOSS"));
    assert_eq!(role_of("u-own"), Some("OWNER"), "owner 写 OWNER 角色行");
    assert!(emit_names(&h.sink)
        .iter()
        .any(|n| n == "im:channel:increment"));
}

/// 回归锚: S1 —— channel_created 缺 id → 整体 noop。
#[test]
fn channel_created_missing_id_noop_s1() {
    let mut h = Harness::new(&[]);
    let effects = h.step(
        inbound(serde_json::json!({
            "action": "channel_created",
            "data": { "displayName": "no id" }
        })),
        1_000,
    );
    assert!(effects.is_empty(), "缺 id 应 noop，got: {effects:?}");
}

// ─── channel_close（delete_at + is_active 置位）────────────────────────────────

/// 回归锚: S1 —— channel_close 置 delete_at + is_active=0 + emit closed。
#[test]
fn channel_close_sets_delete_at_and_inactive_s1() {
    let mut h = Harness::new(&[6]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "channel_close",
            "data": { "channelId": cid(6), "deleteAt": 1700000000 }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    assert!(
        matches!(
            patch_value(&ops, "channel", "delete_at"),
            Some(helix_core::effect::SqlValue::Integer(1700000000))
        ),
        "channel_close 须写 delete_at，ops: {ops:?}"
    );
    assert!(
        matches!(
            patch_value(&ops, "channel", "is_active"),
            Some(helix_core::effect::SqlValue::Integer(0))
        ),
        "channel_close 须置 is_active=0"
    );
    assert!(emit_names(&h.sink).iter().any(|n| n == "im:channel:closed"));
}

// ─── channel_member_update（path1 + 成员）─────────────────────────────────────

/// 回归锚: S1 + A1 —— channel_member_update 全量 upsert + 成员行 + emit member-updated。
///
/// A1（2026-06-18）：成员收集统一走 `channel_write::collect_members`（四源 + memberChange.join），
/// 与 increment_channel / channel_created 三路径同口径——并随之**归一角色**（现网
/// `ChannelMemberInput::new` 经 `normalize_role`：MANAGER→ADMIN）。此前 `parse_member_join`
/// **不**归一会落原始 "MANAGER" = 与现网 channel_member.role 列偏差，本次一并纠正。
#[test]
fn channel_member_update_full_and_members_s1() {
    let mut h = Harness::new(&[]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "channel_member_update",
            "data": {
                "id": cid(7),
                "type": "P",
                "memberChange": { "join": [ { "id": "u-bbb", "role": "MANAGER" } ] }
            }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    assert!(has_write_to_table(&ops, "channel"));
    assert!(
        matches!(upsert_row_value(&ops, "channel_member", "role"), Some(helix_core::effect::SqlValue::Text(s)) if s == "ADMIN"),
        "成员 role 须落 channel_member 且经 normalize_role（MANAGER→ADMIN，对齐现网），ops: {ops:?}"
    );
    assert!(emit_names(&h.sink)
        .iter()
        .any(|n| n == "im:channel:member-updated"));
}

/// 回归锚: HX-C008 —— channel_member_update 冷启动真新 channel 内存 cursor 种子必须 = 0
/// （本地确认水位），**不是** 帧的 lastEventSeq（服务端 max）。lastEventSeq 只喂 increment_target。
///
/// 防回归点：此前 apply_channel_full_and_members 以 seed=lastEventSeq 种子 cursor（与已修复的
/// increment_channel.rs 同形态 HX-C008 违反）。若某真新 channel 首帧走本 handler，cursor 被钉死到 max
/// → 后续 increment_channel_end 的 from_seq=cursor=max → Go no_change → 离线 backfill 丢失；
/// 且 B2 heal 的 cursor<target 永假（cursor==target==lastEventSeq）→ heal 连带废。
#[test]
fn channel_member_update_cold_start_cursor_seed_zero_hxc008() {
    let ch = channel_id(77);
    // 真新 channel（未预注册），帧带服务端水位 lastEventSeq=999。
    let mut h = Harness::new(&[]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "channel_member_update",
            "data": {
                "id": cid(77),
                "type": "P",
                "lastEventSeq": 999,
                "memberChange": { "join": [ { "id": "u-bbb", "role": "MANAGER" } ] }
            }
        })),
        1_000,
    );
    // 内存 cursor 必须种子 0（本地确认水位），不得是 lastEventSeq(999)。
    assert_eq!(
        h.module.cursor_for(ch),
        Some(0),
        "冷启动真新 channel 内存 cursor 须种子 0（本地确认水位），不得钉到 lastEventSeq(999)"
    );
    // lastEventSeq 须独立喂 increment_target（服务端水位，B2 heal 锚点）。
    assert_eq!(
        h.module.increment_target_for(ch),
        Some(999),
        "lastEventSeq 须独立进 increment_target(999)，与内存 cursor 解耦"
    );
}

// ─── update_channel_member_nickName（驼峰混 snake 拼写）─────────────────────────

/// 回归锚: S1 —— update_channel_member_nickName 复合 PK 写 channel_member.nick_name + emit。
#[test]
fn nickname_update_writes_member_nick_and_emits_s1() {
    let mut h = Harness::new(&[]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "update_channel_member_nickName",
            "data": { "channelId": cid(8), "userId": "u-ccc", "nickName": "新昵称" }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    assert!(
        matches!(upsert_row_value(&ops, "channel_member", "nick_name"), Some(helix_core::effect::SqlValue::Text(s)) if s == "新昵称"),
        "nickName 须写 channel_member.nick_name（复合 PK），ops: {ops:?}"
    );
    // 复合 PK upsert：conflict_key 必须是 "channel_id,user_id"（防误改全 channel 成员）。
    let composite = ops.iter().any(|op| matches!(
        op, StorageOp::BatchUpsert(s) if s.table == "channel_member" && s.conflict_key == Some("channel_id,user_id")
    ));
    assert!(composite, "nickname 须用复合 PK conflict_key，ops: {ops:?}");
    assert!(emit_names(&h.sink)
        .iter()
        .any(|n| n == "im:channel:memberNickname"));
}

/// 回归锚: S1 —— nickName 缺 userId → noop（边界零信任）。
#[test]
fn nickname_update_missing_user_id_noop_s1() {
    let mut h = Harness::new(&[]);
    let effects = h.step(
        inbound(serde_json::json!({
            "action": "update_channel_member_nickName",
            "data": { "channelId": cid(8), "nickName": "x" }
        })),
        1_000,
    );
    assert!(effects.is_empty(), "缺 userId 应 noop，got: {effects:?}");
}

// ─── post_schedule_created / canceled ─────────────────────────────────────────

/// 回归锚: S1 —— post_schedule_created 置 has_schedule_post=1 + emit。
#[test]
fn schedule_created_sets_flag_s1() {
    let mut h = Harness::new(&[9]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "post_schedule_created",
            "data": { "channelId": cid(9) }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    assert!(
        matches!(
            patch_value(&ops, "channel", "has_schedule_post"),
            Some(helix_core::effect::SqlValue::Integer(1))
        ),
        "schedule_created 须置 has_schedule_post=1，ops: {ops:?}"
    );
    assert!(emit_names(&h.sink)
        .iter()
        .any(|n| n == "im:channel:schedule-created"));
}

/// 回归锚: S1 —— post_schedule_canceled 置 has_schedule_post=0 + emit。
#[test]
fn schedule_canceled_clears_flag_s1() {
    let mut h = Harness::new(&[9]);
    let _ = h.step(
        inbound(serde_json::json!({
            "action": "post_schedule_canceled",
            "data": { "channelId": cid(9) }
        })),
        1_000,
    );
    let ops = storage_ops(&h.sink);
    assert!(
        matches!(
            patch_value(&ops, "channel", "has_schedule_post"),
            Some(helix_core::effect::SqlValue::Integer(0))
        ),
        "schedule_canceled 须置 has_schedule_post=0，ops: {ops:?}"
    );
    assert!(emit_names(&h.sink)
        .iter()
        .any(|n| n == "im:channel:schedule-canceled"));
}
