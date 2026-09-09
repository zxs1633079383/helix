// 回归锚: specs/004-messagev3-helix-phase1-send-domain T109 / T110，Gate MV3-G04e 定时消息立即发送。
//
// 合同（gates/schedule/mv3-g04e/）：
// - `inbound-contract.json.rust_commands` = `im_send_message` + `im_cancel_schedule`
//   —— 「立即发送」没有自己的命令，是两条既有公开命令的**组合**（T110）。
// - `outbound-sample.json` 6 条样本 / 4 类事件：
//   `im:post:sending`(sender) → `im:post:received`(sender+receiver)
//   → `im:channel:update`(sender unread 0 / receiver unread 1) → `im:channel:schedule-canceled`(sender)。
// - 发送三态与 `schedule-canceled` 必须**都**到达，不得只发一半。
//
// ## CONTRACT_DRIFT-G04e-01（已回报，未擅自改样本）
//
// 本 Gate 的 `effects-contract.json` 写的是 `allowed_effect_tags=["Http","Persist","Emit"]` +
// `reference_sequence` Http→Persist→Emit，但它复用的是**发送内核**，而 spec §1.3 INV-04 与
// §2.1 的 G01a 行都要求发送链是 `Persist→Http→ScheduleTimer→Emit`（optimistic 持久化屏障在前，
// 15 秒 echo timer 从 `posts/create` 真正出站开始计时）。二者不可能同时成立。
// 本文件按 INV-04（用户已拍板的不变量，裁定顺序高于 Gate JSON 的 Effect 概览）断言真实发送链，
// 并把差异如实记录在此，等 Spec owner 统一；**不**改 `specs/003-*` 下任何 JSON。
// 取消半场仍严格按 Http→Persist→Emit 断言。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use helix_im::client_api::{build_command, command_alias};
use helix_im::outbound::{handle_outbound, is_outbound};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{
    cid, command_json, port_reply_err, port_reply_ok, port_reply_with_body, EffectKind, Harness,
};

const SENDER_ID: &str = "user-actor-444";
const RECEIVER_ID: &str = "user-receiver-445";
const MESSAGE: &str = "立即发送定时消息";
const POST_ID: &str = "post-mv3-g04e-001";
const CREATE_AT: i64 = 1_700_000_000_001;
const CANCELED_REVISION: u64 = 9;
const NOW_MS: u64 = 1_700_000_000_000;
const SAMPLE_CHANNEL_ID: &str = "channel-phase1-001";
const BASE_URL: &str = "http://test/api/cses";

// ── fixtures ────────────────────────────────────────────────────────────────

fn channel() -> String {
    cid(11)
}

fn gate_json(file: &str) -> Value {
    let gate_root = phase1_reference::reference_root().join("schedule/mv3-g04e");
    phase1_reference::read_json(&gate_root.join(file))
}

fn harness_for(user_id: &str) -> Harness {
    Harness::new_with_identity(&[11], user_id, "company-phase1", "", "", "")
}

/// 「立即发送」提交的就是普通发送最小意图 —— 没有任何 schedule 专属字段。
fn send_payload() -> Value {
    json!({
        "channel_id": channel(),
        "type": "TEXT",
        "text": MESSAGE,
        "props": {},
        "viewers": ["all"],
        "mentions": []
    })
}

/// 内核冻结键集形态的取消 payload。
///
/// CONTRACT_DRIFT-G04b-01（已回报）：参考样本还带 `schedule_id`，内核 `require_exact_keys`
/// 只收 `{channel_id, req_id}`。裁定前用内核合法形态驱动真实链路，样本现状在
/// `inbound_sample_composes_send_and_cancel` 里如实固定。
fn cancel_payload() -> Value {
    json!({ "channel_id": channel(), "req_id": "mv3-g04e-cancel-1" })
}

/// Go `action=post` 权威回声帧（立即发送的服务端结算）。
fn post_authority(temporary_id: &str) -> Value {
    json!({
        "action": "post",
        "data": {
            "channelId": channel(),
            "event_seq": 1,
            "post": {
                "id": POST_ID,
                "temporaryId": temporary_id,
                "channelId": channel(),
                "userId": SENDER_ID,
                "type": "TEXT",
                "message": MESSAGE,
                "simpleMessage": MESSAGE,
                "props": {},
                "viewers": ["all"],
                "createAt": CREATE_AT,
                "updateAt": CREATE_AT
            }
        }
    })
}

fn canceled_authority() -> Value {
    json!({
        "action": "post_schedule_canceled",
        "eventSeq": CANCELED_REVISION,
        "data": {
            "channelId": channel(),
            "scheduleOwnerUserId": SENDER_ID,
            "status": "canceled",
            "revision": CANCELED_REVISION
        }
    })
}

fn inbound(frame: &Value) -> Tick {
    let bytes = serde_json::to_vec(frame).expect("frame must serialize");
    Tick::Inbound(InboundBytes(Bytes::from(bytes)))
}

/// 取消提交后的 durable `channel_schedule` 行（正文全清、终态）。
fn canceled_row() -> Row {
    vec![
        ("channel_id".to_string(), SqlValue::Text(channel())),
        ("schedule_id".to_string(), SqlValue::Text(String::new())),
        (
            "owner_user_id".to_string(),
            SqlValue::Text(SENDER_ID.to_string()),
        ),
        ("message".to_string(), SqlValue::Text(String::new())),
        ("message_preview".to_string(), SqlValue::Text(String::new())),
        ("send_at".to_string(), SqlValue::Integer(0)),
        ("status".to_string(), SqlValue::Text("canceled".to_string())),
        (
            "revision".to_string(),
            SqlValue::Integer(CANCELED_REVISION as i64),
        ),
    ]
}

/// 当前 viewer 的 `channel_member` 绝对态读回（viewer-local 未读，INV-12）。
fn member_reply(user_id: &str, unread_count: i64) -> ReplyBytes {
    let last_post = json!({
        "id": POST_ID,
        "userId": SENDER_ID,
        "message": MESSAGE,
        "createAt": CREATE_AT
    });
    let row: Row = vec![
        ("channel_id".to_string(), SqlValue::Text(channel())),
        ("user_id".to_string(), SqlValue::Text(user_id.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(unread_count)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
        ("last_post_at".to_string(), SqlValue::Integer(CREATE_AT)),
        (
            "last_root_post_at".to_string(),
            SqlValue::Integer(CREATE_AT),
        ),
    ];
    ReplyBytes(rows_to_reply_bytes(&[row]))
}

/// 领域事件独立计数；关联请求结果另由 read_relay 回归核对。
fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
            _ => None,
        })
        .filter(|event| event["event"] != "im:read:result")
        .collect()
}

fn corr_of(effects: &[Effect], kind: &str) -> Option<u64> {
    effects.iter().find_map(|effect| match (kind, effect) {
        ("http", Effect::Http { corr, .. }) => Some(corr.raw()),
        ("atomic", Effect::PersistAtomic { corr, .. }) => Some(corr.raw()),
        ("persist", Effect::Persist { corr, .. }) => Some(corr.raw()),
        _ => None,
    })
}

/// 从 optimistic 写集中读出 Helix 分配的 temporaryId（INV-01：Angular 不生成）。
fn persisted_temporary_id(effects: &[Effect]) -> String {
    effects
        .iter()
        .find_map(|effect| {
            let Effect::Persist { ops, .. } = effect else {
                return None;
            };
            ops.iter().find_map(|op| {
                let StorageOp::BatchUpsert(spec) = op else {
                    return None;
                };
                spec.rows.iter().find_map(|row| {
                    row.iter()
                        .find_map(|(key, value)| match (key.as_str(), value) {
                            ("temporary_id", SqlValue::Text(value)) => Some(value.clone()),
                            _ => None,
                        })
                })
            })
        })
        .expect("optimistic persistence must contain temporary_id")
}

fn event_names(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["event"].as_str().unwrap_or_default())
        .collect()
}

/// sender 半场：从 `im_send_message` 一路驱动到 read-back，返回 (temporaryId, 事件序列, 每批 tag)。
fn run_sender_send(harness: &mut Harness) -> (String, Vec<Value>, Vec<EffectKind>) {
    let mut events = Vec::new();
    let mut kinds = Vec::new();

    kinds.extend(harness.step(command_json("im_send_message", send_payload()), NOW_MS));
    events.extend(emitted(harness.sink.as_slice()));
    let temporary_id = persisted_temporary_id(harness.sink.as_slice());
    let persist_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须先落 optimistic 写");

    kinds.extend(harness.step(port_reply_ok(persist_corr), NOW_MS + 1));
    events.extend(emitted(harness.sink.as_slice()));

    kinds.extend(harness.step(inbound(&post_authority(&temporary_id)), NOW_MS + 2));
    events.extend(emitted(harness.sink.as_slice()));
    let atomic_corr =
        corr_of(harness.sink.as_slice(), "atomic").expect("WS 回声必须落 PersistAtomic");

    kinds.extend(harness.step(port_reply_ok(atomic_corr), NOW_MS + 3));
    events.extend(emitted(harness.sink.as_slice()));
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("提交成功必须请求读回");

    kinds.extend(harness.step(
        Tick::PortReply {
            corr: Correlation::from_raw(readback_corr),
            outcome: PortOutcome::Ok(member_reply(SENDER_ID, 0)),
        },
        NOW_MS + 4,
    ));
    events.extend(emitted(harness.sink.as_slice()));
    (temporary_id, events, kinds)
}

/// sender 半场的第二条命令：取消原定时（Http → Persist → Emit）。
fn run_sender_cancel(harness: &mut Harness, base_ms: u64) -> Vec<Value> {
    let mut events = Vec::new();

    harness.step(
        command_json("im_cancel_schedule", cancel_payload()),
        base_ms,
    );
    events.extend(emitted(harness.sink.as_slice()));
    let http_corr = corr_of(harness.sink.as_slice(), "http").expect("取消必须发出 Http");

    harness.step(port_reply_with_body(http_corr, common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#))), base_ms + 1);
    events.extend(emitted(harness.sink.as_slice()));

    harness.step(inbound(&canceled_authority()), base_ms + 2);
    events.extend(emitted(harness.sink.as_slice()));
    let atomic_corr =
        corr_of(harness.sink.as_slice(), "atomic").expect("取消 authority 必须落原子写");

    harness.step(port_reply_ok(atomic_corr), base_ms + 3);
    events.extend(emitted(harness.sink.as_slice()));
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须请求 durable 读回");

    harness.step(
        port_reply_with_body(
            readback_corr,
            rows_to_reply_bytes(std::slice::from_ref(&canceled_row())),
        ),
        base_ms + 4,
    );
    events.extend(emitted(harness.sink.as_slice()));
    events
}

/// receiver 半场：只摄取 WS 权威帧（收件人没有发送命令，也不该看到 creator-private 取消）。
fn run_receiver(temporary_id: &str) -> Vec<Value> {
    let mut harness = harness_for(RECEIVER_ID);
    let mut events = Vec::new();

    harness.step(inbound(&post_authority(temporary_id)), NOW_MS + 2);
    events.extend(emitted(harness.sink.as_slice()));
    let atomic_corr = corr_of(harness.sink.as_slice(), "atomic")
        .expect("receiver 摄取权威帧必须落 PersistAtomic");

    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    events.extend(emitted(harness.sink.as_slice()));
    let readback_corr =
        corr_of(harness.sink.as_slice(), "persist").expect("receiver 必须请求 viewer-local 读回");

    harness.step(
        Tick::PortReply {
            corr: Correlation::from_raw(readback_corr),
            outcome: PortOutcome::Ok(member_reply(RECEIVER_ID, 1)),
        },
        NOW_MS + 4,
    );
    events.extend(emitted(harness.sink.as_slice()));

    // creator-private 的取消广播落到 receiver 必须整体 no-op。
    harness.step(inbound(&canceled_authority()), NOW_MS + 5);
    assert!(
        harness.sink.as_slice().is_empty(),
        "receiver 不得因 sender 的定时取消产生任何 Effect"
    );
    events.extend(emitted(harness.sink.as_slice()));
    events
}

// ── 1. 参考包完整性 ─────────────────────────────────────────────────────────

/// 证明 MV3-G04e 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G04e", "schedule/mv3-g04e");
}

// ── 2. 组合：两条既有公开命令，不是新命令 ──────────────────────────────────

/// 立即发送在协议上就是 `im_send_message` + `im_cancel_schedule` 的组合（T110）。
#[test]
fn inbound_sample_composes_send_and_cancel() {
    let sample = gate_json("inbound-sample.json");
    let inputs = sample["rust_entry_inputs"]
        .as_array()
        .expect("rust_entry_inputs must be an array");
    assert_eq!(inputs.len(), 2, "立即发送必须是两条命令的组合");
    assert_eq!(inputs[0]["value"]["name"], "im_send_message");
    assert_eq!(inputs[1]["value"]["name"], "im_cancel_schedule");
    assert_eq!(inputs[0]["value"]["payload"]["text"], MESSAGE);
    assert_eq!(
        inputs[1]["value"]["payload"]["channel_id"],
        SAMPLE_CHANNEL_ID
    );

    // 两条公开命令都必须在公开边界登记（T110：组合而非新增合并命令）。
    assert_eq!(command_alias("im_send_message"), Some("im_send_message"));
    assert_eq!(
        command_alias("im_cancel_schedule"),
        Some("im_cancel_schedule")
    );
    // 发送走模块内核，取消走 outbound 注册表 —— 两条通道各自独立。
    assert!(!is_outbound("im_send_message"));
    assert!(is_outbound("im_cancel_schedule"));

    // 负向：不存在把两步合成一步的「立即发送」命令。
    for merged in [
        "im_send_schedule_now",
        "im_schedule_send_now",
        "im_execute_schedule",
    ] {
        assert!(!is_outbound(merged), "立即发送不得有合并命令 {merged}");
        assert!(
            command_alias(merged).is_none(),
            "立即发送不得有合并别名 {merged}"
        );
    }

    // 发送意图必须能通过公开边界，并且 Helix-owned 字段被剥离（INV-01/INV-02）。
    let payload = serde_json::to_vec(&json!({
        "channelId": SAMPLE_CHANNEL_ID,
        "type": "TEXT",
        "text": MESSAGE,
        "temporaryId": "caller-owned"
    }))
    .expect("payload serializes");
    let command = build_command("im_send_message", &payload).expect("发送意图必须能通过公开边界");
    let normalized: Value =
        serde_json::from_slice(command.payload.as_ref()).expect("normalized payload must be JSON");
    assert_eq!(normalized["channel_id"], SAMPLE_CHANNEL_ID);
    assert!(
        normalized.get("temporary_id").is_none(),
        "temporaryId 归 Helix，公开边界必须剥离调用方伪造值"
    );
}

// ── 3. 四类事件必须都到达 ──────────────────────────────────────────────────

/// sender 视角：发送三态 + `schedule-canceled` 四类事件必须全部到达，且顺序与样本一致。
#[test]
fn sender_receives_all_four_projections_in_sample_order() {
    let sample = gate_json("outbound-sample.json");
    let expected: Vec<&str> = sample["samples"]
        .as_array()
        .expect("samples must be array")
        .iter()
        .filter(|item| item["viewer"] == "sender")
        .map(|item| item["rust"]["event"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        expected,
        vec![
            "im:post:sending",
            "im:post:received",
            "im:channel:update",
            "im:channel:schedule-canceled"
        ]
    );

    let mut harness = harness_for(SENDER_ID);
    let (temporary_id, mut events, _) = run_sender_send(&mut harness);
    events.extend(run_sender_cancel(&mut harness, NOW_MS + 100));

    assert_eq!(
        event_names(&events),
        expected,
        "立即发送不得只发一半：发送三态与 schedule-canceled 都必须到达"
    );

    // 发送三态以同一个 temporaryId 对账（INV-02）。
    assert_eq!(events[0]["data"]["temporaryId"], temporary_id);
    assert_eq!(events[0]["data"]["sendStatus"], "sending");
    assert_eq!(events[1]["data"]["temporaryId"], temporary_id);
    assert_eq!(events[1]["data"]["id"], POST_ID);
    // 会话摘要是 viewer-local：发送者自己的未读恒为 0（INV-12）。
    assert_eq!(events[2]["data"]["unreadCount"], json!(0));
    // 取消终态由 durable 行构造，绝对频道投影关掉提示条。
    assert_eq!(events[3]["data"]["hasSchedulePost"], json!(false));
    assert_eq!(events[3]["data"]["status"], "canceled");
}

// ── 4. 双 persona ──────────────────────────────────────────────────────────

/// receiver 独立回放：拿到同一条 Post 的绝对投影与**自己**的未读，但拿不到取消事件。
#[test]
fn receiver_persona_gets_the_post_but_never_the_schedule_cancel() {
    let mut sender = harness_for(SENDER_ID);
    let (temporary_id, _, _) = run_sender_send(&mut sender);

    let receiver_events = run_receiver(&temporary_id);
    assert_eq!(
        event_names(&receiver_events),
        vec!["im:post:received", "im:channel:update"],
        "receiver 只看到消息，不看到 creator-private 的定时取消"
    );
    // 同一条 Post 的服务端主键与对账键两侧一致（INV-02）。
    assert_eq!(receiver_events[0]["data"]["id"], POST_ID);
    assert_eq!(receiver_events[0]["data"]["temporaryId"], temporary_id);
    // 未读分视角独立计算：receiver 是 1，sender 是 0，禁止互相复制（INV-12）。
    assert_eq!(receiver_events[1]["data"]["unreadCount"], json!(1));
    for name in event_names(&receiver_events) {
        assert!(
            !name.starts_with("im:channel:schedule"),
            "receiver 不得收到 {name}"
        );
    }
}

/// 两个 persona 的未读必须各自独立 —— 同一份权威帧不得让双方读到同一个数。
#[test]
fn unread_projections_are_persona_local() {
    let mut sender = harness_for(SENDER_ID);
    let (temporary_id, sender_events, _) = run_sender_send(&mut sender);
    let receiver_events = run_receiver(&temporary_id);

    let sender_update = sender_events
        .iter()
        .find(|event| event["event"] == "im:channel:update")
        .expect("sender 必须收到会话摘要更新");
    let receiver_update = receiver_events
        .iter()
        .find(|event| event["event"] == "im:channel:update")
        .expect("receiver 必须收到会话摘要更新");

    assert_ne!(
        sender_update["data"]["unreadCount"], receiver_update["data"]["unreadCount"],
        "sender 与 receiver 的未读不得同源"
    );
    assert_eq!(sender_update["data"]["unreadCount"], json!(0));
    assert_eq!(receiver_update["data"]["unreadCount"], json!(1));
}

// ── 5. 两条命令互不串扰 ────────────────────────────────────────────────────

/// 发送半场不得产生任何 schedule 事件；取消半场不得产生任何 Post 事件。
#[test]
fn the_two_commands_never_leak_into_each_other() {
    let mut harness = harness_for(SENDER_ID);
    let (_, send_events, _) = run_sender_send(&mut harness);
    for name in event_names(&send_events) {
        assert!(
            !name.starts_with("im:channel:schedule"),
            "发送半场不得产生 {name}"
        );
    }

    let cancel_events = run_sender_cancel(&mut harness, NOW_MS + 100);
    assert_eq!(cancel_events.len(), 1, "取消半场恰好一条终态事件");
    for name in event_names(&cancel_events) {
        assert!(!name.starts_with("im:post:"), "取消半场不得产生 {name}");
    }
}

// ── 6. INV-04：发送链的 Effect 形态 ────────────────────────────────────────

/// 发送半场必须是 optimistic `Persist` 先行、`PersistOk` 之后才 Http + 15 秒 echo timer + Emit。
///
/// 见文件头 CONTRACT_DRIFT-G04e-01：本 Gate 的 `effects-contract.json` 只列了
/// `Http/Persist/Emit` 且序列写成 Http→Persist→Emit，与 INV-04 冲突；此处按 INV-04 断言。
#[test]
fn send_half_follows_inv04_optimistic_barrier_then_transport() {
    let mut harness = harness_for(SENDER_ID);

    let first = harness.step(command_json("im_send_message", send_payload()), NOW_MS);
    assert!(
        matches!(first.as_slice(), [EffectKind::Persist(_)]),
        "首批只允许 optimistic Persist，实际 {first:?}"
    );
    assert!(
        emitted(harness.sink.as_slice()).is_empty(),
        "optimistic 屏障提交前不得发布 sending"
    );

    let persist_corr = match first.as_slice() {
        [EffectKind::Persist(corr)] => *corr,
        other => panic!("expected one optimistic persist, got {other:?}"),
    };
    let second = harness.step(port_reply_ok(persist_corr), NOW_MS + 1);
    assert_eq!(
        second
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        1,
        "PersistOk 之后才允许出站 posts/create"
    );
    assert_eq!(
        second
            .iter()
            .filter(|effect| matches!(effect, EffectKind::ScheduleTimer(_)))
            .count(),
        1,
        "15 秒 echo timer 只从 posts/create 真正出站开始计时（INV-04）"
    );
    let sending = emitted(harness.sink.as_slice());
    assert_eq!(event_names(&sending), vec!["im:post:sending"]);
}

/// optimistic 持久化失败：不得出站、不得起 timer、不得发布 sending，只能发失败态。
///
/// 「立即发送」在 UI 上通常紧跟一次乐观的定时删除，所以这里必须确认失败路径**不会**假装
/// 消息已经上路：一旦 optimistic 屏障没提交成功，既没有 `posts/create`，也没有 15 秒 timer，
/// 更不能出现 `im:post:sending`（否则用户会看到一条永远停在「发送中」的幽灵消息）。
#[test]
fn optimistic_persist_failure_blocks_the_immediate_send() {
    let mut harness = harness_for(SENDER_ID);
    let first = harness.step(command_json("im_send_message", send_payload()), NOW_MS);
    let temporary_id = persisted_temporary_id(harness.sink.as_slice());
    let persist_corr = match first.as_slice() {
        [EffectKind::Persist(corr)] => *corr,
        other => panic!("expected one optimistic persist, got {other:?}"),
    };

    let failure = harness.step(port_reply_err(persist_corr), NOW_MS + 1);
    assert!(
        failure
            .iter()
            .all(|effect| !matches!(effect, EffectKind::Http(_) | EffectKind::ScheduleTimer(_))),
        "optimistic 失败必须挡住出站与 timer，实际 {failure:?}"
    );
    let events = emitted(harness.sink.as_slice());
    assert_eq!(
        event_names(&events),
        vec!["im:post:send-failed"],
        "optimistic 失败只能发失败态，不得发 sending / received"
    );
    // 失败态必须锚在同一个对账键上，否则 Angular 找不到那条乐观消息去标红（INV-02）。
    assert_eq!(events[0]["data"]["temporaryId"], temporary_id);
    assert_eq!(events[0]["data"]["channelId"], channel());
    let status = events[0]["data"]["sendStatus"]
        .as_str()
        .expect("失败态必须带 sendStatus");
    assert!(
        status != "sending" && status != "sent",
        "失败态不得回落成在途 / 已送达，实际 {status}"
    );
}

/// 取消半场严格遵守合同的 Http → Persist → Emit，且不起任何本地 timer（INV-09）。
#[test]
fn cancel_half_follows_the_contract_sequence_without_timers() {
    let mut harness = harness_for(SENDER_ID);
    let mut kinds = Vec::new();

    kinds.extend(harness.step(command_json("im_cancel_schedule", cancel_payload()), NOW_MS));
    assert!(
        matches!(kinds.as_slice(), [EffectKind::Http(_)]),
        "取消首批只允许一条 Http，实际 {kinds:?}"
    );
    let http_corr = corr_of(harness.sink.as_slice(), "http").expect("必须发出取消 Http");
    kinds.extend(harness.step(port_reply_with_body(http_corr, common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#))), NOW_MS + 1));
    kinds.extend(harness.step(inbound(&canceled_authority()), NOW_MS + 2));
    let atomic_corr = corr_of(harness.sink.as_slice(), "atomic").expect("必须落 PersistAtomic");
    kinds.extend(harness.step(port_reply_ok(atomic_corr), NOW_MS + 3));
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须请求读回");
    kinds.extend(harness.step(
        port_reply_with_body(
            readback_corr,
            rows_to_reply_bytes(std::slice::from_ref(&canceled_row())),
        ),
        NOW_MS + 4,
    ));

    assert!(
        !kinds.iter().any(|effect| matches!(
            effect,
            EffectKind::ScheduleTimer(_) | EffectKind::CancelTimer(_)
        )),
        "取消定时不得起本地 timer（INV-09），实际 {kinds:?}"
    );
    let http_at = kinds
        .iter()
        .position(|effect| matches!(effect, EffectKind::Http(_)))
        .expect("缺 Http");
    let persist_at = kinds
        .iter()
        .position(|effect| {
            matches!(
                effect,
                EffectKind::Persist(_) | EffectKind::PersistAtomic(_)
            )
        })
        .expect("缺 Persist");
    let emit_at = kinds
        .iter()
        .position(|effect| matches!(effect, EffectKind::Emit))
        .expect("缺 Emit");
    assert!(
        http_at < persist_at && persist_at < emit_at,
        "实际 {kinds:?}"
    );
}

// ── 7. 幂等 ────────────────────────────────────────────────────────────────

/// 立即发送重复触发不产生两条消息：同一 temporaryId 的重复回声必须被吞掉。
#[test]
fn duplicate_echo_never_produces_a_second_message() {
    let mut harness = harness_for(SENDER_ID);
    let (temporary_id, events, _) = run_sender_send(&mut harness);
    assert_eq!(
        event_names(&events),
        vec!["im:post:sending", "im:post:received", "im:channel:update"]
    );

    harness.step(inbound(&post_authority(&temporary_id)), NOW_MS + 10);
    let replayed = emitted(harness.sink.as_slice());
    assert!(
        !event_names(&replayed).contains(&"im:post:sending"),
        "迟到 / 重复回声不得把消息打回 sending"
    );
    // 重复回声即便再走一遍提交，也只能收敛到同一条 Post（同 id + 同 temporaryId）。
    for event in &replayed {
        if event["event"] == "im:post:received" {
            assert_eq!(event["data"]["id"], POST_ID);
            assert_eq!(event["data"]["temporaryId"], temporary_id);
        }
    }

    // 取消同样幂等：已提交 revision 的重复 authority 零 Effect。
    run_sender_cancel(&mut harness, NOW_MS + 100);
    harness.step(inbound(&canceled_authority()), NOW_MS + 200);
    assert!(
        harness.sink.as_slice().is_empty(),
        "已提交 revision 的重复取消 authority 必须被吞掉"
    );
}

// ── 8. fail-closed ─────────────────────────────────────────────────────────

/// 取消半场的必填校验必须在产生任何 Effect 之前生效。
#[test]
fn cancel_half_fails_closed_on_malformed_input() {
    for payload in [
        json!({}),
        json!({ "channel_id": "" }),
        json!({ "channel_id": channel(), "unexpected": "x" }),
    ] {
        let bytes = serde_json::to_vec(&payload).expect("payload must serialize");
        assert!(
            handle_outbound(
                "im_cancel_schedule",
                &bytes,
                BASE_URL,
                "http://test",
                None,
                Correlation::from_raw(7),
            )
            .is_err(),
            "{payload} 必须 fail-closed"
        );
    }
}
