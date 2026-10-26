//! C1/C2 真 Go WS `post` action 在线广播帧落库回放测试（sync-v2）
//!
//! 回归锚: C1/C2（真 Go `action=="post"` 在线新消息识别 + channel gate 落库）
//!
//! ## 背景（2026-06-17 live 注入定位）
//!
//! 真连 Go：curl 往真频道发消息 → Go `post sender success` → WS 广播 → helix 收到
//! `{"v":1,"action":"post","data":{"channelId":...,"event_seq":40113,"id":...,
//!  "temporaryId":...,"message":...,"type":"TEXT","readBits":"100",...}}` → **但 0 落库**。
//! 根因：旧 `classify_frame` 只认 `action=="posted"`，真帧 action 是 **`post`** → 落
//! `Some(_) => Ignored` 静默丢弃，永不进 gate。
//!
//! ## 行为真源（现网 source of truth）
//!
//! cses-client `router.rs:40` `"post" => post::handle` + `post.rs:42-114` 统一
//! `handle_new_post` + `message_service.rs:64-69` `batch_upsert ON CONFLICT(temporary_id)`。
//! echo 与新消息**不在落库层分叉**——统一走 gate，靠 temporary_id PK upsert 自然对账。
//!
//! ## 验证点
//!
//! 1. `post` 帧 event_seq == cursor+1 → 形成 MessageV3 原子写；matching PersistOk 后
//!    推进 cursor 并读回 viewer channel 绝对态。
//! 2. message 行 upsert 字段对齐（temporary_id PK / id / message / readBits / props）。
//! 3. event_seq > cursor+1 → buffer + arm 1s gate（不立即落库）。
//! 4. echo（本端 temporary_id 命中乐观行）复用同 `post`→gate→ON CONFLICT(temporary_id) 路径。

mod common;
use common::*;

use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::tick::InboundBytes;
use helix_core::Tick;
use helix_im::module::ImConfig;
use helix_im::ImModule;

/// 构造 Harness 并以指定 cursor 种子注册单个 channel（模拟 increment 种 cursor）。
fn harness_with_seed(channel_num: u64, seed_cursor: u64) -> Harness {
    let config = ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
        auth_user_id: "post-test-viewer".to_string(),
        company_id: "post-test-company".to_string(),
        user_name: String::new(),
        org_name: String::new(),
        dept_name: String::new(),
        client_platform: Default::default(),
        offline_sync_diagnostics: Default::default(),
    };
    let mut module = ImModule::new(config);
    module.register_channel(channel_id(channel_num), seed_cursor);
    Harness {
        module,
        sink: helix_core::EffectSink::new(),
        effect_log: Vec::new(),
    }
}

/// 构造真 Go 在线广播 `post` 帧（本 CTX live 帧形态）。
///
/// 真信封 `{v:1, action:"post", data:{channelId, event_seq, props.channel_event_seq,
/// id, temporaryId, message, type, readBits, createBy}, broadcast:{}, seq:<信封序号>}`。
fn post_frame(
    channel_id: u64,
    event_seq: u64,
    temporary_id: &str,
    server_id: &str,
    message: &str,
) -> Tick {
    use serde_json::json;
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "channelId": cid(channel_id),
            "event_seq": event_seq,
            "id": server_id,
            "temporaryId": temporary_id,
            "message": message,
            "type": "TEXT",
            "readBits": "100",
            "createBy": "5ffd465eaaaaaaaaaaaaaaaaaa",
            "props": { "channel_event_seq": event_seq }
        },
        "broadcast": {},
        "seq": 261
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    Tick::Inbound(InboundBytes(bytes.into()))
}

/// 从 sink 找到 message 表的 BatchUpsert spec（包含 MessageV3 原子写）。
fn find_message_upsert(h: &Harness) -> helix_core::effect::UpsertSpec {
    h.sink
        .as_slice()
        .iter()
        .find_map(|e| {
            let ops = match e {
                Effect::PersistFire { ops } => ops,
                Effect::Persist { ops, .. } => ops,
                Effect::PersistAtomic { ops, .. } => ops,
                _ => return None,
            };
            ops.iter().find_map(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "message" => Some(spec.clone()),
                _ => None,
            })
        })
        .expect("post frame should emit a message-table BatchUpsert")
}

// ─── post 帧 event_seq == cursor+1 → 立即 apply + 落库 + 推 cursor ───────────────

/// 回归锚: C1 —— 真 Go `post` 帧（event_seq == cursor+1）→ apply → message 行 + cursor 推进。
///
/// 复刻 live 帧：channel cursor 种子 = 40112（increment 种），post 帧 event_seq = 40113 ==
/// cursor+1 → gate 立即接纳。断言 PersistAtomic 后 cursor → 40113。
#[test]
fn post_frame_at_cursor_plus_one_persists_and_advances() {
    let mut h = harness_with_seed(1, 40_112);

    let effects = h.step(
        post_frame(
            1,
            40_113,
            "helixverify1781669870",
            "7xq3r6nqrjrm8nydit6awaaaaa",
            "helix-verify-live",
        ),
        1_100,
    );

    let persist_corr = effects.iter().find_map(|effect| match effect {
        EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    });
    let Some(persist_corr) = persist_corr else {
        panic!("post at cursor+1 must enter PersistAtomic, got {effects:?}");
    };
    assert!(!effects.iter().any(|e| matches!(e, EffectKind::Emit)));
    let committed = h.step(port_reply_ok(persist_corr), 1_101);
    assert!(committed
        .iter()
        .any(|effect| matches!(effect, EffectKind::Persist(_))));
    // matching PersistOk 后 cursor 单调推进到 event_seq。
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(40_113),
        "cursor must advance to event_seq=40113"
    );
}

/// 回归锚: C2 —— message 行字段对齐（temporary_id PK / id / message / read_bits / props）。
#[test]
fn post_frame_upsert_row_fields_aligned() {
    let mut h = harness_with_seed(1, 40_112);
    let _ = h.step(
        post_frame(
            1,
            40_113,
            "helixverify1781669870",
            "7xq3r6nqrjrm8nydit6awaaaaa",
            "hello-post",
        ),
        1_100,
    );

    let spec = find_message_upsert(&h);
    assert_eq!(
        spec.conflict_key,
        Some("temporary_id"),
        "post upsert must conflict on temporary_id (echo PK 自然对账)"
    );
    let row = &spec.rows[0];
    let get =
        |col: &str| -> Option<&SqlValue> { row.iter().find(|(k, _)| k == col).map(|(_, v)| v) };
    // PK = temporary_id（本端乐观行键，echo 命中则覆写）
    assert!(
        matches!(get("temporary_id"), Some(SqlValue::Text(t)) if t == "helixverify1781669870"),
        "row PK should be temporary_id, got: {:?}",
        get("temporary_id")
    );
    // id = server 权威 post.id
    assert!(
        matches!(get("id"), Some(SqlValue::Text(t)) if t == "7xq3r6nqrjrm8nydit6awaaaaa"),
        "row id should be server post.id, got: {:?}",
        get("id")
    );
    // message 内容落库
    assert!(
        matches!(get("message"), Some(SqlValue::Text(t)) if t == "hello-post"),
        "row message should be the post body, got: {:?}",
        get("message")
    );
    // read_bits（type=6 / readBits 真源）落库（单调覆盖列）
    assert!(
        matches!(get("read_bits"), Some(SqlValue::Text(t)) if t == "100"),
        "row read_bits should carry wire readBits, got: {:?}",
        get("read_bits")
    );
    // 服务端 WS post 是已送达事实：全新远端/转发行不得在 SQLite 留空发送态。
    assert!(
        matches!(get("send_status"), Some(SqlValue::Text(t)) if t == "sent"),
        "new server-confirmed post row should persist send_status=sent, got: {:?}",
        get("send_status")
    );
    assert!(
        spec.exclude_from_update.contains(&"send_status"),
        "send_status must be INSERT-only here; optimistic-row conflicts are settled by reconcile"
    );
}

// ─── post 帧 event_seq > cursor+1 → buffer + arm gate（不立即落库）──────────────

/// 回归锚: C1 —— gap 的 post 帧进 buffer + arm 1s gate，不立即落库（gate 状态机）。
#[test]
fn post_frame_with_gap_buffers_and_arms_gate() {
    let mut h = harness_with_seed(1, 40_112);

    // event_seq=40115 > cursor(40112)+1 → 缺口 → buffer + arm gate
    let effects = h.step(
        post_frame(
            1,
            40_115,
            "tmp_gap",
            "7xq3r6nqrjrm8nydit6awaaaaa",
            "future-post",
        ),
        1_100,
    );

    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::ScheduleTimer(_))),
        "gapped post must arm a 1s gate timer, got: {:?}",
        effects
    );
    assert!(
        !effects.iter().any(|e| matches!(e, EffectKind::PersistFire)),
        "gapped post must NOT persist immediately (buffered), got: {:?}",
        effects
    );
    // cursor 不动（仍在种子位）
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(40_112),
        "gapped post must not advance cursor"
    );
}

// ─── echo（本端 temporary_id）复用同 post→gate→ON CONFLICT(temporary_id) 路径 ─────

/// 从 send 的 effect 流里取首个 ScheduleTimer 的 id_raw（= 15s 发送超时 timer）。
fn first_schedule_timer(effects: &[EffectKind]) -> u64 {
    effects
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .expect("send should schedule a 15s timeout timer")
}

/// Normal text does not arm its send timeout until P1 has been acknowledged.
/// This Harness has no attached timeline, so the P1 callback may start H1
/// directly; attached-surface Frame ordering is verified in local-first tests.
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
        .expect("ordinary send must begin with a P1 persist");
    harness.step(port_reply_ok(corr), 1_001)
}

/// 回归锚: C1 —— echo 经 `post` 路径完整对账（BLOCKING 修复）。
///
/// 本端先 send（注册 tmp_echo 乐观行 + 15s timer），随后真 Go `post` echo 帧（同 temporary_id）
/// 到达 → 经 gate apply（cursor + 落库），**并额外 reconcile PendingSend**：
///   - 落库 upsert：PK=temporary_id 覆写乐观行，server id 落库（gate 路径）；
///   - gate INSERT：服务端确认的新行写 send_status='sent'，冲突时不覆盖本地发送态；
///   - reconcile Persist：命中乐观行时显式 send_status='sent' 覆写；
///   - CancelTimer 撤 15s 发送超时（否则 15s 后误标 UnSend）；
///   - PendingSend 落定移除（无泄漏，status 查询返回 None）。
#[test]
fn echo_post_frame_reconciles_via_temporary_id_upsert() {
    let mut h = harness_with_seed(1, 40_112);

    // 1. 本端 send（注册乐观行 tmp_echo + 15s timer）
    let initial_effects = h.step(send_message_cmd(1, "tmp_echo", "from-me"), 1_000);
    let send_effects = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let timeout_timer = first_schedule_timer(&send_effects);
    assert_eq!(
        h.module.pending_send_count(),
        1,
        "send 注册 1 个在途 PendingSend"
    );

    // 2. 真 Go echo 帧（action=post，同 temporary_id=tmp_echo，event_seq=cursor+1）
    let effects = h.step(
        post_frame(
            1,
            40_113,
            "tmp_echo",
            "7xq3r6nqrjrm8nydit6awaaaaa",
            "from-me",
        ),
        1_100,
    );

    // (a) gate 原子写以 temporary_id 命中乐观行，reconcile 另写 send_status。
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::PersistAtomic(_))),
        "echo post must enter MessageV3 PersistAtomic, got: {:?}",
        effects
    );
    // (b) CancelTimer 撤 15s 发送超时（核心修复点：否则 15s 后误标 UnSend）
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(t) if *t == timeout_timer)),
        "echo post must CancelTimer the 15s send timeout, got: {:?}",
        effects
    );
    // (c) PendingSend 落定移除（无泄漏）
    assert_eq!(
        h.module.pending_send_count(),
        0,
        "reconciled PendingSend must be removed (no leak), got count={}",
        h.module.pending_send_count()
    );
    assert_eq!(
        h.module.pending_send_status("tmp_echo"),
        None,
        "reconciled-and-removed PendingSend status must be None"
    );

    // (d) gate upsert 行字段对齐（PK + server id 覆写）
    let spec = find_message_upsert(&h);
    assert_eq!(spec.conflict_key, Some("temporary_id"));
    let row = &spec.rows[0];
    let get =
        |col: &str| -> Option<&SqlValue> { row.iter().find(|(k, _)| k == col).map(|(_, v)| v) };
    assert!(
        matches!(get("temporary_id"), Some(SqlValue::Text(t)) if t == "tmp_echo"),
        "echo upsert PK must be the local optimistic temporary_id"
    );
    assert!(
        matches!(get("id"), Some(SqlValue::Text(t)) if t == "7xq3r6nqrjrm8nydit6awaaaaa"),
        "echo upsert must overwrite id with server post.id"
    );

    // (e) reconcile 的 correlated Persist 必须仍显式写 send_status='sent'。gate upsert 对
    // 冲突更新排除此列，只负责让全新的服务端 WS 行初始即为 sent。
    let sent_status_written = h.sink.as_slice().iter().any(|e| {
        let Effect::Persist { ops, .. } = e else {
            return false;
        };
        ops.iter().any(|op| match op {
            StorageOp::BatchUpsert(s) if s.table == "message" => s.rows.iter().any(|r| {
                r.iter().any(|(k, v)| {
                    k == "send_status" && matches!(v, SqlValue::Text(t) if t == "sent")
                })
            }),
            _ => false,
        })
    });
    assert!(
        sent_status_written,
        "echo reconcile must explicitly write send_status='sent' for the optimistic-row conflict"
    );

    // (f) matching PersistOk 后 cursor 推进；reconcile 不重复推进。
    let atomic_corr = effects.iter().find_map(|effect| match effect {
        EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    });
    let Some(atomic_corr) = atomic_corr else {
        panic!("echo post missing MessageV3 atomic correlation");
    };
    h.step(port_reply_ok(atomic_corr), 1_101);
    assert_eq!(
        h.module.cursor_for(channel_id(1)),
        Some(40_113),
        "echo post advances cursor to event_seq (once, by gate)"
    );
}

/// 回归锚: C1 —— BLOCKING 根因回归：`post` echo 后触发 15s timer **不得**误标 UnSend。
///
/// 这是修复前的真实故障：旧 `InboundEvent::Post` 只走 ch.ingest（不取消 15s timer / 不写
/// send_status），本端已送达的消息在 15s 后被 on_timeout 翻成 UnSend + 持久化 unsend。
/// 修复后 reconcile 撤了 timer 且移除 PendingSend，timer fire 必须是无害 no-op。
#[test]
fn post_echo_then_15s_timer_does_not_mark_unsend() {
    let mut h = harness_with_seed(1, 40_112);

    // 1. 本端 send → 拿到 15s timer
    let initial_effects = h.step(send_message_cmd(1, "tmp_live", "from-me"), 1_000);
    let send_effects = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let timeout_timer = first_schedule_timer(&send_effects);

    // 2. `post` echo（cursor+1）到达 → reconcile（CancelTimer + 移除 PendingSend）
    let _ = h.step(
        post_frame(
            1,
            40_113,
            "tmp_live",
            "7xq3r6nqrjrm8nydit6awaaaaa",
            "from-me",
        ),
        1_100,
    );
    assert_eq!(
        h.module.pending_send_count(),
        0,
        "post echo must reconcile-and-remove the PendingSend"
    );

    // 3. 迟到的 15s timer fire → 必须无害（PendingSend 已移除）：
    //    不得产出任何写 send_status='unsend' 的 Persist/PersistFire。
    let late = h.step(timer_tick(timeout_timer), 16_001);
    let wrote_unsend = h.sink.as_slice().iter().any(|e| {
        let ops = match e {
            Effect::Persist { ops, .. } => ops,
            Effect::PersistFire { ops } => ops,
            _ => return false,
        };
        ops.iter().any(|op| match op {
            StorageOp::BatchUpsert(s) if s.table == "message" => s.rows.iter().any(|r| {
                r.iter().any(|(k, v)| {
                    k == "send_status" && matches!(v, SqlValue::Text(t) if t == "unsend")
                })
            }),
            _ => false,
        })
    });
    assert!(
        !wrote_unsend,
        "15s timer after post-echo must NOT mark a delivered message UnSend, got: {:?}",
        late
    );
}

// ─── unregistered channel 的 post 帧静默忽略（不 panic，不落库）──────────────────

/// 回归锚: C1 + Round-3 UC-4.5 偏差修复 —— 未注册 channel 的 post 帧**自动注册**（不再静默丢弃）。
///
/// 对齐真源 `on_ws_event` `gate_states.entry().or_insert_with(default)` + `handle_new_post` 无条件
/// 落库：陌生 channel（首次见到的新群 / 被拉入群）的首条在线 push 在 increment 种 cursor 之前到达时
/// 不丢——以 cursor=0 惰性注册，gate 接管：event_seq=1（==cursor+1）→ 进入原子写屏障。
/// 此用例同时锁住 echo 不变量：陌生 channel 的 post（无对应 in-flight send）仍**不创建** PendingSend。
#[test]
fn post_frame_unregistered_channel_auto_registers_and_persists() {
    // channel 5 未注册
    let config = ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
        auth_user_id: "post-test-viewer".to_string(),
        company_id: "post-test-company".to_string(),
        user_name: String::new(),
        org_name: String::new(),
        dept_name: String::new(),
        client_platform: Default::default(),
        offline_sync_diagnostics: Default::default(),
    };
    let mut h = Harness {
        module: ImModule::new(config),
        sink: helix_core::EffectSink::new(),
        effect_log: Vec::new(),
    };

    let tick = post_frame(5, 1, "tmp_u", "7xq3r6nqrjrm8nydit6awaaaaa", "orphan");
    let effects = h.step(tick, 1_100);
    // 自动注册 + 进入原子写（event_seq=1 == cursor+1）：陌生 channel 不再丢帧。
    let atomic_corr = effects.iter().find_map(|effect| match effect {
        EffectKind::PersistAtomic(corr) => Some(*corr),
        _ => None,
    });
    let Some(atomic_corr) = atomic_corr else {
        panic!("陌生 channel post（seq=1）应进入 MessageV3 PersistAtomic，got {effects:?}");
    };
    h.step(port_reply_ok(atomic_corr), 1_101);
    assert_eq!(
        h.module.cursor_for(channel_id(5)),
        Some(1),
        "陌生 channel 应自动注册（cursor=0 seed）并推进到 event_seq=1"
    );
    // 陌生 channel 的 post（无对应 in-flight send）仍不创建 PendingSend（echo 不变量保持）。
    assert_eq!(
        h.module.pending_send_count(),
        0,
        "unregistered-channel post must not create or touch any PendingSend"
    );
}

// ─── R4-echo-gate：echo 对账与 channel gate 解耦（冷启动 send-before-cursor-seed 修复）────

/// 回归锚: R4-echo-gate（drift-review 收口）—— 本端 echo 在 channel **未注册**时仍须对账。
///
/// ## 修复前真实故障
///
/// 旧 `InboundEvent::Post` 把 `reconcile_post_echo` 整个包在 `if let Some(ch)=channels.get_mut(..)`
/// 内。但 `reconcile_post_echo` 只读 `pending_sends`/`corr_map`、**从不读 channels** → 该门控多余
/// 且有害：冷启动 send-before-cursor-seed 乱序下（本端先 send、channel 尚未经 increment 种 cursor），
/// 本端 echo 帧因 channel 未注册被整段跳过 → PendingSend 残留 → 15s 后 on_timeout 把已送达的本端
/// 消息误标 UnSend。
///
/// 修复：`reconcile_post_echo` 提到 channel 查找之外**无条件**先做（ingest/cursor 仍门控注册），
/// 对照 `handle_echo` 把 reconcile 置于 channel 查找之外的形态。
#[test]
fn echo_reconciles_even_when_channel_unregistered() {
    // channel 5 故意 **不** 注册（模拟冷启动 cursor 尚未种下）。
    let config = ImConfig {
        ws_url: "ws://test".to_string(),
        api_base_url: "http://test".to_string(),
        default_api_base_url: "http://localhost:3399".to_string(),
        ping_interval_ms: 8_000,
        send_timeout_ms: 15_000,
        auth_user_id: String::new(),
        company_id: String::new(),
        user_name: String::new(),
        org_name: String::new(),
        dept_name: String::new(),
        client_platform: Default::default(),
        offline_sync_diagnostics: Default::default(),
    };
    let mut h = Harness {
        module: ImModule::new(config),
        sink: helix_core::EffectSink::new(),
        effect_log: Vec::new(),
    };

    // 1. 本端 send（注册乐观行 tmp_cold + 15s timer）—— send 不依赖 channel 注册。
    let initial_effects = h.step(send_message_cmd(5, "tmp_cold", "from-me-cold"), 1_000);
    let send_effects = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let timeout_timer = first_schedule_timer(&send_effects);
    assert_eq!(
        h.module.pending_send_count(),
        1,
        "send 注册 1 个在途 PendingSend（不依赖 channel 注册）"
    );

    // 2. 本端 echo 帧（action=post，同 temporary_id），但 channel 5 **未注册**。
    let _ = h.step(
        post_frame(
            5,
            1,
            "tmp_cold",
            "7xq3r6nqrjrm8nydit6awaaaaa",
            "from-me-cold",
        ),
        1_100,
    );

    // 3. 即便 channel 未注册（ingest 被跳过），echo 仍须对账 → PendingSend 落定移除。
    assert_eq!(
        h.module.pending_send_count(),
        0,
        "echo 必须对账掉 PendingSend，即便 channel 未注册（与 channel gate 解耦）"
    );

    // 4. 迟到的 15s timer fire → 必须无害（PendingSend 已移除）：
    //    不得产出任何写 send_status='unsend' 的 Persist/PersistFire（这是修复前的故障表现）。
    let late = h.step(timer_tick(timeout_timer), 16_001);
    let wrote_unsend = h.sink.as_slice().iter().any(|e| {
        let ops = match e {
            Effect::Persist { ops, .. } => ops,
            Effect::PersistFire { ops } => ops,
            _ => return false,
        };
        ops.iter().any(|op| match op {
            StorageOp::BatchUpsert(s) if s.table == "message" => s.rows.iter().any(|r| {
                r.iter().any(|(k, v)| {
                    k == "send_status" && matches!(v, SqlValue::Text(t) if t == "unsend")
                })
            }),
            _ => false,
        })
    });
    assert!(
        !wrote_unsend,
        "15s timer after cold-start echo (unregistered channel) must NOT mark UnSend, got: {:?}",
        late
    );
}

/// 诊断开启不改变公开入站与提交接缝，失败仍保留原游标。
#[test]
fn diagnostic_session_preserves_persist_success_and_failure_contract() {
    for success in [true, false] {
        let mut plain = harness_with_seed(1, 40_112);
        let mut observed = harness_with_seed(1, 40_112);
        observed.module = observed
            .module
            .with_diagnostic_session("login-1".into(), "device-1".into());
        let input = || post_frame(1, 40_113, "diag-post", "7xq3r6nqrjrm8nydit6awaaaaa", "body");
        let expected = plain.step(input(), 1_100);
        assert_eq!(observed.step(input(), 1_100), expected);
        let corr = expected
            .iter()
            .find_map(|kind| {
                if let EffectKind::PersistAtomic(corr) = kind {
                    Some(*corr)
                } else {
                    None
                }
            })
            .unwrap();
        let reply = || {
            if success {
                port_reply_ok(corr)
            } else {
                port_reply_err(corr)
            }
        };
        assert_eq!(observed.step(reply(), 1_101), plain.step(reply(), 1_101));
        assert_eq!(
            observed.module.cursor_for(channel_id(1)),
            Some(if success { 40_113 } else { 40_112 })
        );
    }
}
