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

use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::tick::InboundBytes;
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

/// 回归锚: S5 —— echo 走真源 `post` action → reconcile + CancelTimer + cursor 推进。
#[test]
fn s5_post_action_echo_reconciles() {
    let mut h = Harness::new(&[1]);
    // 1. 发送 → PendingSend(Local) + 15s timeout timer。
    let send = h.step(send_message_cmd(1, "tmp_e", "hello"), 1000);
    let timeout_timer = send
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .expect("send 须 arm 15s timeout timer");
    assert_eq!(
        h.module.pending_send_status("tmp_e"),
        Some(SendStatus::Local)
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
    // 5. cursor 独立推进到 echo seq=1。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(1),
        "post-action echo 须推进 cursor 到 event_seq=1"
    );
}

/// 回归锚: S5 —— sender 豁免下沉 driver：core 无身份 → echo 保守产 unread bump=1。
///
/// 钉死分层契约：helix-core **不**在内核做 sender 豁免（避免引入身份）；auth_user_id 空 →
/// 保守计入（不漏未读），故 echo 仍产 GuardedBump unread+1；真正的「自己发的不计未读」由
/// driver 注入真 auth_user_id 后裁决（与 emit 路径把 sender 豁免下沉前端同策略）。
#[test]
fn s5_core_echo_conservatively_bumps_unread_sender_exemption_deferred() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(send_message_cmd(1, "tmp_e", "hello"), 1000);
    let _ = h.step(post_echo_frame(1, 1, "tmp_e", 42), 1100);

    let bump = h.sink.as_slice().iter().find_map(|e| match e {
        Effect::PersistFire { ops } => ops.iter().find_map(|op| match op {
            StorageOp::GuardedBump(s) if s.bump_col == "unread_count" => Some(s.bump_delta),
            _ => None,
        }),
        _ => None,
    });
    assert_eq!(
        bump,
        Some(1),
        "S5/分层：core 无身份 → echo 保守产 unread bump=1（sender 豁免下沉 driver），got {bump:?}"
    );
}

/// 回归锚: fix/im-send-echo-reconcile —— 本端 self-echo 在 cursor gap（seq≫cursor+1·被 gate
/// buffer）下，仍**兜底 emit `im:post:received`** 结算乐观行 DOM（壳纯渲染靠投影；DB 已由
/// reconcile_post_echo 落 id）。守不变量：cursor **不**被该兜底推进（仍卡在 gap 前）。
#[test]
fn s5_self_echo_under_gap_still_emits_post_received_for_dom_settle() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(send_message_cmd(1, "tmp_gap", "hello"), 1000);
    // HTTP 200 无 server id（cses Pulsar 异步建）→ 标 Sent 但保留 pending（path1 修复）。
    // 直接打 echo 即可验 path2 DOM 兜底（pending 仍在）。

    // echo seq=9 ≫ cursor(0)+1 → ch.ingest buffer（不 apply-emit）。
    let echo = h.step(post_echo_frame(1, 9, "tmp_gap", 55), 1100);

    // 兜底 emit：sink 中须有 im:post:received，data.temporaryId==tmp_gap·id==server_id·sendStatus==sent。
    let settle = h.sink.as_slice().iter().find_map(|e| match e {
        Effect::Emit { event } => {
            let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
            if v.get("event").and_then(|x| x.as_str()) == Some("im:post:received") {
                Some(v)
            } else {
                None
            }
        }
        _ => None,
    });
    let v = settle.expect("self-echo under gap 须兜底 emit im:post:received（DOM 结算），否则乐观行永停 sending");
    let data = v.get("data").expect("im:post:received 须含 data");
    assert_eq!(
        data.get("temporaryId").and_then(|x| x.as_str()),
        Some("tmp_gap"),
        "兜底投影须按 temporaryId 结算乐观行"
    );
    assert_eq!(
        data.get("msg_id").and_then(|x| x.as_str()),
        Some(sid(55).as_str()),
        "兜底投影 msg_id 须为 server_id（tmp→server 覆写）"
    );
    assert_eq!(
        data.get("sendStatus").and_then(|x| x.as_str()),
        Some("sent"),
        "兜底投影 sendStatus 须 sent"
    );

    // 不变量：cursor 不被兜底推进（gap 未填→仍卡 0；HX-C008 单调由 gate 严格 +1 守）。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(0),
        "DOM 兜底投影**不**推 cursor（cursor 仍卡在 gap 前，待 sync 回补）"
    );
    let _ = echo;
}

/// 回归锚: fix/im-send-echo-reconcile —— 连续 echo（seq==cursor+1·gate apply）下**不**双发
/// im:post:received（gate apply 自然 emit 一次 → echo_settle=None·防双发）。
#[test]
fn s5_self_echo_contiguous_does_not_double_emit_post_received() {
    let mut h = Harness::new(&[1]);
    let _ = h.step(send_message_cmd(1, "tmp_c", "hello"), 1000);
    let _ = h.step(post_echo_frame(1, 1, "tmp_c", 66), 1100); // seq=1==cursor+1 连续

    let count = h
        .sink
        .as_slice()
        .iter()
        .filter(|e| match e {
            Effect::Emit { event } => serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                .ok()
                .and_then(|v| v.get("event").and_then(|x| x.as_str()).map(str::to_string))
                .as_deref()
                == Some("im:post:received"),
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
