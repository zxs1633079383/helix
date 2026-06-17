//! C1 echo→reconcile 回放测试（sync-v2 §1）
//!
//! 回归锚: C1（sync-v2 echo 对账激活）
//!
//! 验证发送对账全链路：
//!   send_message → PendingSend(Local) → posted echo 帧（带 temporary_id）→ reconcile：
//!     - PendingSend 推进到 Sent
//!     - message 表覆写：`id=server_id, send_status='sent' WHERE temporary_id=tmp`（Persist BatchUpsert）
//!     - CancelTimer 撤 15s 超时
//!     - cursor 独立推进（与可见性解耦，HX-C008）
//!
//! 行为真源：cses-client `src-tauri/src/features/im/handlers/post.rs:58`
//!   （`post.temporary_id` 非空 → sendMessage 回流对账）+ `message_service.rs:46 handle_new_post`
//!   （batch_upsert ON CONFLICT(temporary_id) 覆写 post.id 为 server id）。
//!
//! **真帧待核**（TODO sync-v2 C1）：echo 帧当前按最可能形态构造
//!   `{action:"posted", data:{temporary_id, id, channel_id, event_seq}}`；抓到首个真 echo
//!   帧后核对字段路径并补 golden fixture。

mod common;
use common::*;

use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::Tick;
use helix_im::state::{SendStatus, ServerId};

/// 测试桥接：数值 → 26 字符 server id（与 ServerId::from_test_u64 一致）。
fn sid(n: u64) -> String {
    ServerId::from_test_u64(n).as_str().to_string()
}

/// 构造 echo（posted 回声）帧：真 Go 信封 `{action:"posted", data:{...post...}}`。
fn echo_frame(channel_id: u64, seq: u64, temporary_id: &str, server_id: u64) -> Tick {
    use helix_core::tick::InboundBytes;
    use serde_json::json;
    let payload = json!({
        "v": 1,
        "action": "posted",
        "data": {
            "id": sid(server_id),
            "temporary_id": temporary_id,
            "channel_id": cid(channel_id),
            "event_seq": seq,
            "message": "hello",
        },
        "broadcast": {},
        "seq": 99
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Inbound(InboundBytes(bytes.into()))
}

/// 从 effects 中提取首个 ScheduleTimer 的 id_raw。
fn first_schedule_timer(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|e| {
            if let EffectKind::ScheduleTimer(id) = e {
                Some(*id)
            } else {
                None
            }
        })
        .expect("send should schedule a 15s timeout timer")
}

// ─── echo 到达 → reconcile 全链路 ────────────────────────────────────────────

#[test]
fn echo_reconciles_pending_send() {
    let mut h = Harness::new(&[1]);

    // 1. 发送命令 → PendingSend(Local) 注册
    let send_effects = h.step(send_message_cmd(1, "tmp_1", "hello"), 1000);
    let timeout_timer = first_schedule_timer(&send_effects);
    assert_eq!(h.module.pending_send_count(), 1, "send registers 1 pending");
    assert_eq!(
        h.module.pending_send_status("tmp_1"),
        Some(SendStatus::Local),
        "before echo status is Local"
    );

    // 2. echo 帧到达（带 temporary_id=tmp_1，server_id=42，seq=7）
    let echo_effects = h.step(echo_frame(1, 7, "tmp_1", 42), 1100);

    // 3. PendingSend 推进到 Sent
    assert_eq!(
        h.module.pending_send_status("tmp_1"),
        Some(SendStatus::Sent),
        "echo reconcile should advance PendingSend to Sent"
    );

    // 4. 应产出 Persist（message 表覆写）+ CancelTimer + advance_cursor 的 PersistFire
    assert!(
        echo_effects
            .iter()
            .any(|e| matches!(e, EffectKind::Persist(_))),
        "echo should emit Persist to overwrite message row, got: {:?}",
        echo_effects
    );
    assert!(
        echo_effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(t) if *t == timeout_timer)),
        "echo should CancelTimer the 15s timeout, got: {:?}",
        echo_effects
    );

    // 5. cursor 独立推进到 echo 帧 seq（HX-C008：与消息可见性解耦）
    let cid1 = helix_im::state::ChannelId::from_test_u64(1);
    assert_eq!(
        h.module.cursor_for(cid1),
        Some(7),
        "echo should advance cursor to event_seq=7"
    );
}

// ─── message 行覆写内容核验（id=server_id + send_status='sent'）────────────────

#[test]
fn echo_overwrites_message_id_and_status() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(send_message_cmd(1, "tmp_x", "hi"), 1000);
    let _ = h.step(echo_frame(1, 5, "tmp_x", 77), 1100);

    // 在 sink 中找 message 表 BatchUpsert（reconcile 产出），核验列覆写。
    let persist_op = h
        .sink
        .as_slice()
        .iter()
        .find_map(|e| match e {
            Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "message" => Some(spec.clone()),
                _ => None,
            }),
            _ => None,
        })
        .expect("reconcile should emit a message-table BatchUpsert Persist");

    assert_eq!(
        persist_op.conflict_key,
        Some("temporary_id"),
        "reconcile upsert must conflict on temporary_id (PK 覆写)"
    );
    let row = &persist_op.rows[0];
    let get =
        |col: &str| -> Option<&SqlValue> { row.iter().find(|(k, _)| k == col).map(|(_, v)| v) };
    // temporary_id = tmp_x（按 PK 定位）
    assert!(
        matches!(get("temporary_id"), Some(SqlValue::Text(t)) if t == "tmp_x"),
        "row should key on temporary_id=tmp_x, got: {:?}",
        get("temporary_id")
    );
    // id 覆写为 server_id（26 字符 Id26 Text）
    assert!(
        matches!(get("id"), Some(SqlValue::Text(t)) if t == &sid(77)),
        "row should overwrite id=server_id (Text Id26), got: {:?}",
        get("id")
    );
    // send_status 覆写为 'sent'
    assert!(
        matches!(get("send_status"), Some(SqlValue::Text(t)) if t == "sent"),
        "row should overwrite send_status='sent', got: {:?}",
        get("send_status")
    );
}

// ─── echo 先到则 15s 超时永不标 UnSend（C006 收敛）────────────────────────────

#[test]
fn echo_before_timeout_keeps_sent_idempotent() {
    let mut h = Harness::new(&[1]);
    let send_effects = h.step(send_message_cmd(1, "tmp_2", "hello"), 1000);
    let timeout_timer = first_schedule_timer(&send_effects);

    // echo 先到 → Sent
    let _ = h.step(echo_frame(1, 3, "tmp_2", 9), 1100);
    assert_eq!(
        h.module.pending_send_status("tmp_2"),
        Some(SendStatus::Sent)
    );

    // 迟到的 15s timer fire → 幂等忽略（已 Sent，不产出 PersistFire 标 unsend）
    let late = h.step(timer_tick(timeout_timer), 16_001);
    assert!(
        !late.iter().any(|e| matches!(e, EffectKind::PersistFire)),
        "timer after echo-Sent should be idempotent (no unsend), got: {:?}",
        late
    );
    assert_eq!(
        h.module.pending_send_status("tmp_2"),
        Some(SendStatus::Sent),
        "status stays Sent after late timer"
    );
}

// ─── 非 echo 的 posted（temporary_id 空）不误触发 reconcile ────────────────────

#[test]
fn posted_without_temporary_id_is_not_echo() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(send_message_cmd(1, "tmp_3", "hello"), 1000);

    // 他人推送的 posted 帧：无 temporary_id → 不是 echo → 不动 pending_send
    use helix_core::tick::InboundBytes;
    use serde_json::json;
    let frame = json!({
        "v": 1,
        "action": "posted",
        "data": {
            "id": sid(100),
            "channel_id": cid(1),
            "event_seq": 4,
            "message": "from someone else",
        },
        "seq": 99
    });
    let bytes = serde_json::to_vec(&frame).unwrap();
    let effects = h.step(Tick::Inbound(InboundBytes(bytes.into())), 1100);

    // pending_send 不受影响（仍 Local，未对账）
    assert_eq!(
        h.module.pending_send_status("tmp_3"),
        Some(SendStatus::Local),
        "non-echo posted must not reconcile our pending send"
    );
    // 不产出 reconcile 的 message Persist + CancelTimer
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(_))),
        "non-echo posted should not CancelTimer, got: {:?}",
        effects
    );
}
