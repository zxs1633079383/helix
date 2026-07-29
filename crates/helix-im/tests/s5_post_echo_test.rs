//! sync-v2 §S5 echo 对账（真源 `post` action）对齐补全回放测试。
//!
//! 回归锚: S5（SPEC-sync-v2-full-replacement.md §S5）
//!
//! 现网 echo 走 `action=="post"`（router.rs:40 `"post" => post::handle`），靠 `temporary_id`
//! 非空区分自发回流（post.rs:58）。既有 echo_reconcile_test 用 `posted` 帧；本文件锁定
//! **真源 `post` 帧**路径：echo → reconcile(send_status='sent'+id=server_id) + CancelTimer
//! + cursor 推进；无 temporary_id（他人推送）→ 非 echo 不动 pending_send。
//!
//! 分层说明：sender 豁免（自己发的不计未读）**下沉 driver/前端**——helix-core 无身份
//! （HX-C001/C002），auth_user_id 空 → 保守计入未读；driver 注入真身份后裁决。

mod common;
use common::*;

use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, ReplyBytes};
use helix_core::Tick;
use helix_im::state::SendStatus;

fn sid(n: u64) -> String {
    server_id(n).as_str().to_string()
}

/// 真源 echo 帧：`action=="post"`（router.rs:40），data 带 temporary_id（自发回流标识）。
fn post_echo_frame(channel_id: u64, seq: u64, temporary_id: &str, server_id: u64) -> Tick {
    use serde_json::json;
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "id": sid(server_id),
            "temporary_id": temporary_id,
            "channelId": cid(channel_id),
            "event_seq": seq,
            "message": "hello",
            "type": "P",
        },
        "seq": 99
    });
    Tick::Inbound(InboundBytes(serde_json::to_vec(&payload).unwrap().into()))
}

fn acknowledge_optimistic_persist(
    harness: &mut Harness,
    effects: &[EffectKind],
) -> Vec<EffectKind> {
    let corr = effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("ordinary send must begin with P1 persist");
    harness.step(port_reply_ok(corr), 1_001)
}

/// 确认 G-01 原子提交并回灌当前 viewer 的 channel_member 绝对态。
fn acknowledge_post_commit(
    harness: &mut Harness,
    effects: &[EffectKind],
    temporary_id: &str,
    server_id: u64,
    now_ms: u64,
) -> Vec<EffectKind> {
    let persist_corr = effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::PersistAtomic(corr) => Some(*corr),
            _ => None,
        })
        .expect("post echo must begin with MessageV3 PersistAtomic");
    let readback = harness.step(port_reply_ok(persist_corr), now_ms);
    let readback_corr = readback
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("post commit must read back current channel_member");
    let last_post = serde_json::json!({
        "id": sid(server_id),
        "temporaryId": temporary_id,
        "channelId": cid(1),
        "message": "hello",
        "type": "TEXT",
        "createAt": 1_100
    });
    let row: Row = vec![
        ("channel_id".to_string(), SqlValue::Text(cid(1).to_string())),
        ("unread_count".to_string(), SqlValue::Integer(0)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
    ];
    harness.step(
        port_reply_with_body(readback_corr, ReplyBytes(rows_to_reply_bytes(&[row])).0),
        now_ms.saturating_add(1),
    )
}

/// 回归锚: S5 —— echo 走真源 `post` action → reconcile + CancelTimer + cursor 推进。
#[test]
fn s5_post_action_echo_reconciles() {
    let mut h = Harness::new(&[1]);
    // 1. 发送 → PendingSend(Local) + 15s timeout timer。
    let initial_effects = h.step(send_message_cmd(1, "tmp_e", "hello"), 1000);
    let send = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let timeout_timer = send
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .expect("send 须 arm 15s timeout timer");
    assert_eq!(
        h.module.pending_send_status("tmp_e"),
        Some(SendStatus::Sending)
    );

    // 2. echo 走 `post` action（真源）。seq=1（== cursor+1，连续）。
    let echo = h.step(post_echo_frame(1, 1, "tmp_e", 42), 1100);

    // 3. reconcile：产 message 表 Persist 覆写 send_status='sent' + id=server_id。
    //    注：`post` handler 在对账后**移除** pending_send（防 corr_map 泄漏，与 `posted`
    //    handler 保留态的实现选择不同），故此处不查内存 status，而验落定的 Persist 语义。
    let reconcile_persist = h.sink.as_slice().iter().find_map(|e| match e {
        Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
            StorageOp::BatchUpsert(s) if s.table == "message" => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    });
    let spec = reconcile_persist.expect("post-action echo 须产 message 表 Persist 覆写");
    assert_eq!(
        spec.conflict_key,
        Some("temporary_id"),
        "echo 覆写按 temporary_id PK"
    );
    let get = |c: &str| {
        spec.rows[0]
            .iter()
            .find(|(k, _)| k == c)
            .map(|(_, v)| v.clone())
    };
    assert!(
        matches!(get("send_status"), Some(SqlValue::Text(t)) if t == "sent"),
        "echo 须覆写 send_status='sent'，got {:?}",
        get("send_status")
    );
    assert!(
        matches!(get("id"), Some(SqlValue::Text(t)) if t == sid(42)),
        "echo 须覆写 id=server_id，got {:?}",
        get("id")
    );
    // 4. CancelTimer 撤 15s 超时（防误标 unsend）。
    assert!(
        echo.iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(t) if *t == timeout_timer)),
        "post-action echo 须 CancelTimer 15s 超时，got {echo:?}"
    );
    // 5. 原子提交成功后 cursor 独立推进到 echo seq=1。
    let _ = acknowledge_post_commit(&mut h, &echo, "tmp_e", 42, 1_101);
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(1),
        "post-action echo 须推进 cursor 到 event_seq=1"
    );
}

/// 回归锚: S5 —— temporaryId 命中 pending send 即可识别 own echo，core 无身份也不能加 unread。
///
/// 钉死分层契约：身份归属仍不进 helix-core；但 `temporaryId` 命中在途 send 是本端 echo 的强信号，
/// Helix 应直接把 unread_delta 置 0，同时保留 GuardedBump 写 lastPost，前端不再做 +1/-1 补偿。
#[test]
fn s5_core_echo_temporary_id_hit_does_not_increment_unread() {
    let mut h = Harness::new(&[1]);
    let initial = h.step(send_message_cmd(1, "tmp_e", "hello"), 1000);
    let _ = acknowledge_optimistic_persist(&mut h, &initial);
    let _ = h.step(post_echo_frame(1, 1, "tmp_e", 42), 1100);

    let bump = h.sink.as_slice().iter().find_map(|e| match e {
        Effect::PersistFire { ops }
        | Effect::Persist { ops, .. }
        | Effect::PersistAtomic { ops, .. } => ops.iter().find_map(|op| match op {
            StorageOp::GuardedBump(s) if s.bump_col == "unread_count" => Some(s.bump_delta),
            StorageOp::ScopedGuardedBump(s) if s.bump_col == "unread_count" => Some(s.bump_delta),
            _ => None,
        }),
        _ => None,
    });
    assert!(
        bump.is_none_or(|delta| delta == 0),
        "temporaryId 命中 pending send 的 own echo 不得增加 unread_count，got {bump:?}"
    );
}

/// 回归锚: MessageV3 硬切 —— cursor gap 下禁止绕过 gate 提前发布终态事件。
#[test]
fn s5_self_echo_under_gap_waits_for_durable_gate_commit() {
    let mut h = Harness::new(&[1]);
    let initial = h.step(send_message_cmd(1, "tmp_gap", "hello"), 1000);
    let _ = acknowledge_optimistic_persist(&mut h, &initial);

    // echo seq=9 ≫ cursor(0)+1 → ch.ingest buffer（不 apply-emit）。
    let echo = h.step(post_echo_frame(1, 9, "tmp_gap", 55), 1100);

    // MessageV3 禁止为 DOM 结算绕过持久化屏障；补齐缺口后才释放 post/channel 终态。
    let settle = h.sink.as_slice().iter().any(|e| match e {
        Effect::Emit { event } => {
            serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                .ok()
                .and_then(|value| {
                    value
                        .get("event")
                        .and_then(|item| item.as_str())
                        .map(str::to_string)
                })
                .as_deref()
                == Some("im:post:received")
        }
        _ => false,
    });
    assert!(
        !settle,
        "gap self-echo must not emit im:post:received before durable gate commit"
    );

    // 不变量：gap 未补齐时 cursor 不推进，且不会产生 MessageV3 原子提交。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(0),
        "cursor remains at the gap boundary until sync backfill"
    );
    assert!(
        !echo
            .iter()
            .any(|effect| matches!(effect, EffectKind::PersistAtomic(_))),
        "buffered gap echo must not enter MessageV3 commit early"
    );
}

/// 回归锚: fix/im-send-echo-reconcile —— 连续 echo（seq==cursor+1·gate apply）下**不**双发
/// im:post:received（gate apply 自然 emit 一次 → echo_settle=None·防双发）。
#[test]
fn s5_self_echo_contiguous_does_not_double_emit_post_received() {
    let mut h = Harness::new(&[1]);
    let initial = h.step(send_message_cmd(1, "tmp_c", "hello"), 1000);
    let _ = acknowledge_optimistic_persist(&mut h, &initial);
    let echo = h.step(post_echo_frame(1, 1, "tmp_c", 66), 1100); // seq=1==cursor+1 连续
    let _ = acknowledge_post_commit(&mut h, &echo, "tmp_c", 66, 1_101);

    let count = h
        .sink
        .as_slice()
        .iter()
        .filter(|e| match e {
            Effect::Emit { event } => {
                serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                    .ok()
                    .and_then(|v| v.get("event").and_then(|x| x.as_str()).map(str::to_string))
                    .as_deref()
                    == Some("im:post:received")
            }
            _ => false,
        })
        .count();
    assert_eq!(
        count, 1,
        "连续 echo 须恰发 1 次 im:post:received（gate apply 一次·无兜底双发），got {count}"
    );
}

/// 回归锚: S5 —— `post` 帧无 temporary_id（他人推送新消息）→ 非 echo，不动 pending_send。
#[test]
fn s5_post_action_without_temp_id_is_not_echo() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(send_message_cmd(1, "tmp_other", "hi"), 1000);
    use serde_json::json;
    let frame = json!({
        "v": 1, "action": "post",
        "data": { "id": sid(100), "channelId": cid(1), "event_seq": 1, "message": "他人" },
        "seq": 99
    });
    let effects = h.step(
        Tick::Inbound(InboundBytes(serde_json::to_vec(&frame).unwrap().into())),
        1100,
    );
    assert_eq!(
        h.module.pending_send_status("tmp_other"),
        Some(SendStatus::Local),
        "无 temporary_id 的 post（他人推送）不得对账我方 pending send"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(_))),
        "他人推送 post 不得 CancelTimer，got {effects:?}"
    );
}
