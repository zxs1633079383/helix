// 回归锚: specs/004-messagev3-helix-phase1-send-domain T103 / T104，Gate MV3-G04c 编辑定时消息。
//
// 合同（gates/schedule/mv3-g04c/）：
// - `inbound-contract.json.rust_commands` = `im_cancel_schedule` **然后** `im_create_schedule`
//   —— 编辑**没有**自己的命令，是两条独立 Helix Command 的编排（T104：Angular 也不合并状态）。
// - `effects-contract.json` = Http → Persist → Emit，**两轮**。
// - `outbound-sample.json` 两条事件的相对顺序固定：`schedule-canceled` 先于 `schedule-created`。
//
// 顺序与部分失败不是靠调用方自觉，而是靠 `committed_schedule_revisions` 的 revision 单调闸门：
// - 新建 authority 的 revision 必须 **>** 已提交的取消 revision，否则被吞掉
//   → 不可能出现「先看到新定时、再看到取消」这种把新定时误删的乱序终态。
// - 取消提交成功后 create 失败 → 终态是干净的「已取消」（可重试），
//   不是「既没取消也没新建」的不可恢复中间态。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use helix_im::client_api::{build_command, command_alias};
use helix_im::outbound::{handle_outbound, is_outbound};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{cid, command_json, port_reply_err, port_reply_ok, port_reply_with_body, Harness};

const CREATOR_ID: &str = "user-actor-444";
const SCHEDULE_ID: &str = "schedule-001";
const EDITED_SCHEDULE_ID: &str = "schedule-002";
const ORIGINAL_MESSAGE: &str = "定时发送消息";
const EDITED_MESSAGE: &str = "编辑后的定时消息";
const EXECUTE_AT: i64 = 1_700_003_600_000;
const SEEDED_REVISION: u64 = 8;
const CANCELED_REVISION: u64 = 9;
const RECREATED_REVISION: u64 = 10;
const NOW_MS: u64 = 1_700_000_000_000;
const SAMPLE_CHANNEL_ID: &str = "channel-phase1-001";
const BASE_URL: &str = "http://test/api/cses";

// ── fixtures ────────────────────────────────────────────────────────────────

fn channel() -> String {
    cid(11)
}

fn gate_json(file: &str) -> Value {
    let gate_root = phase1_reference::reference_root().join("schedule/mv3-g04c");
    phase1_reference::read_json(&gate_root.join(file))
}

fn creator_harness() -> Harness {
    Harness::new_with_identity(&[11], CREATOR_ID, "company-phase1", "", "", "")
}

/// 内核冻结键集形态的创建 payload。
fn create_payload(message: &str, req_id: &str) -> Value {
    json!({
        "channel_id": channel(),
        "message": message,
        "schedule_post_at": EXECUTE_AT,
        "type": "TEXT",
        "props": {},
        "req_id": req_id
    })
}

/// 构造生产边界接受的取消 payload；频道是唯一定位键。
fn cancel_payload(req_id: &str) -> Value {
    json!({ "channel_id": channel(), "req_id": req_id })
}

fn created_authority(schedule_id: &str, message: &str, revision: u64) -> Value {
    json!({
        "action": "post_schedule_created",
        "eventSeq": revision,
        "data": {
            "channelId": channel(),
            "scheduleId": schedule_id,
            "scheduleOwnerUserId": CREATOR_ID,
            "message": message,
            "messagePreview": message,
            "sendAt": EXECUTE_AT,
            "status": "scheduled",
            "revision": revision
        }
    })
}

fn canceled_authority(revision: u64) -> Value {
    json!({
        "action": "post_schedule_canceled",
        "eventSeq": revision,
        "data": {
            "channelId": channel(),
            "scheduleOwnerUserId": CREATOR_ID,
            "status": "canceled",
            "revision": revision
        }
    })
}

fn inbound(frame: &Value) -> Tick {
    let bytes = serde_json::to_vec(frame).expect("frame must serialize");
    Tick::Inbound(InboundBytes(Bytes::from(bytes)))
}

fn scheduled_row(schedule_id: &str, message: &str, revision: u64) -> Row {
    vec![
        ("channel_id".to_string(), SqlValue::Text(channel())),
        (
            "schedule_id".to_string(),
            SqlValue::Text(schedule_id.to_string()),
        ),
        (
            "owner_user_id".to_string(),
            SqlValue::Text(CREATOR_ID.to_string()),
        ),
        ("message".to_string(), SqlValue::Text(message.to_string())),
        (
            "message_preview".to_string(),
            SqlValue::Text(message.to_string()),
        ),
        ("send_at".to_string(), SqlValue::Integer(EXECUTE_AT)),
        (
            "status".to_string(),
            SqlValue::Text("scheduled".to_string()),
        ),
        ("revision".to_string(), SqlValue::Integer(revision as i64)),
    ]
}

fn canceled_row(revision: u64) -> Row {
    vec![
        ("channel_id".to_string(), SqlValue::Text(channel())),
        ("schedule_id".to_string(), SqlValue::Text(String::new())),
        (
            "owner_user_id".to_string(),
            SqlValue::Text(CREATOR_ID.to_string()),
        ),
        ("message".to_string(), SqlValue::Text(String::new())),
        ("message_preview".to_string(), SqlValue::Text(String::new())),
        ("send_at".to_string(), SqlValue::Integer(0)),
        ("status".to_string(), SqlValue::Text("canceled".to_string())),
        ("revision".to_string(), SqlValue::Integer(revision as i64)),
    ]
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

/// 失败只能结束关联请求，不能产生业务终态或后续 I/O。
fn assert_request_failed(effects: &[Effect], code: &str) {
    assert_eq!(effects.len(), 1);
    let Effect::Emit {event} = &effects[0] else { panic!("expected correlated failure"); };
    let payload: Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(payload["event"], "im:read:result");
    assert_eq!(payload["data"]["error"], code);
}

fn corr_of(effects: &[Effect], kind: &str) -> Option<u64> {
    effects.iter().find_map(|effect| match (kind, effect) {
        ("http", Effect::Http { corr, .. }) => Some(corr.raw()),
        ("atomic", Effect::PersistAtomic { corr, .. }) => Some(corr.raw()),
        ("persist", Effect::Persist { corr, .. }) => Some(corr.raw()),
        _ => None,
    })
}

/// 一步（cancel 或 create）从 Command 驱动到 durable read-back，返回该步产生的全部事件。
fn drive_step(
    harness: &mut Harness,
    command: &'static str,
    payload: Value,
    authority: &Value,
    readback: &[Row],
    base_ms: u64,
) -> Vec<Value> {
    let mut events = Vec::new();

    harness.step(command_json(command, payload), base_ms);
    events.extend(emitted(harness.sink.as_slice()));
    let http_corr = corr_of(harness.sink.as_slice(), "http")
        .unwrap_or_else(|| panic!("{command} 必须发出权威 Http"));

    harness.step(port_reply_with_body(http_corr, common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#))), base_ms + 1);
    events.extend(emitted(harness.sink.as_slice()));

    harness.step(inbound(authority), base_ms + 2);
    events.extend(emitted(harness.sink.as_slice()));
    let Some(atomic_corr) = corr_of(harness.sink.as_slice(), "atomic") else {
        // authority 被 revision 闸门吞掉：本步零事件，交由调用方断言。
        return events;
    };

    harness.step(port_reply_ok(atomic_corr), base_ms + 3);
    events.extend(emitted(harness.sink.as_slice()));
    let readback_corr = corr_of(harness.sink.as_slice(), "persist")
        .unwrap_or_else(|| panic!("{command} 提交成功后必须请求 durable 读回"));

    harness.step(
        port_reply_with_body(readback_corr, rows_to_reply_bytes(readback)),
        base_ms + 4,
    );
    events.extend(emitted(harness.sink.as_slice()));
    events
}

/// 先把一条既有定时消息推进到已提交终态（编辑的前置状态）。
fn seed_existing_schedule(harness: &mut Harness) -> Vec<Value> {
    drive_step(
        harness,
        "im_create_schedule",
        create_payload(ORIGINAL_MESSAGE, "mv3-g04c-seed"),
        &created_authority(SCHEDULE_ID, ORIGINAL_MESSAGE, SEEDED_REVISION),
        std::slice::from_ref(&scheduled_row(
            SCHEDULE_ID,
            ORIGINAL_MESSAGE,
            SEEDED_REVISION,
        )),
        NOW_MS,
    )
}

fn event_names(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["event"].as_str().unwrap_or_default())
        .collect()
}

// ── 1. 参考包完整性 ─────────────────────────────────────────────────────────

/// 证明 MV3-G04c 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G04c", "schedule/mv3-g04c");
}

// ── 2. 编辑 = 两条独立命令，不存在合并命令 ─────────────────────────────────

/// 参考包把编辑声明为 cancel → create 两条 Command；Helix 侧不得存在任何「编辑」合并命令。
#[test]
fn inbound_sample_declares_two_independent_commands() {
    let sample = gate_json("inbound-sample.json");
    let inputs = sample["rust_entry_inputs"]
        .as_array()
        .expect("rust_entry_inputs must be an array");
    assert_eq!(inputs.len(), 2, "编辑必须是两步，不是一步");
    assert_eq!(inputs[0]["value"]["name"], "im_cancel_schedule");
    assert_eq!(inputs[1]["value"]["name"], "im_create_schedule");
    assert_eq!(
        inputs[0]["value"]["payload"]["channel_id"],
        SAMPLE_CHANNEL_ID
    );
    assert_eq!(inputs[1]["value"]["payload"]["message"], EDITED_MESSAGE);

    let contract = gate_json("inbound-contract.json");
    assert_eq!(
        contract["angular_port_method"],
        "cancelSchedule + createSchedule"
    );

    // 两条命令都必须在公开边界登记为已知命令（T104：Angular 分别 invoke，不合并状态）。
    assert_eq!(
        command_alias("im_cancel_schedule"),
        Some("im_cancel_schedule")
    );
    assert_eq!(
        command_alias("im_create_schedule"),
        Some("im_create_schedule")
    );
    assert!(is_outbound("im_cancel_schedule"));
    assert!(is_outbound("im_create_schedule"));

    // 负向：不存在把两步合成一步的「编辑」命令 —— 一旦有人加，本用例立刻变红。
    for merged in [
        "im_edit_schedule",
        "im_update_schedule",
        "im_replace_schedule",
    ] {
        assert!(!is_outbound(merged), "编辑不得有合并命令 {merged}");
        assert!(
            command_alias(merged).is_none(),
            "编辑不得有合并别名 {merged}"
        );
    }

    assert!(inputs[0]["value"]["payload"].get("schedule_id").is_none());
    let frozen = serde_json::to_vec(&inputs[0]["value"]["payload"]).expect("payload serializes");
    let built = build_command("im_cancel_schedule", &frozen).and_then(|command| {
        handle_outbound(
            "im_cancel_schedule",
            command.payload.as_ref(),
            BASE_URL,
            "http://test",
            None,
            Correlation::from_raw(1),
        )
    });
    assert!(built.is_ok(), "冻结后的取消 sample 必须通过真实生产边界");
}

/// 编辑第二步的参考意图必须能真的驱动出 `posts/createSchedule` wire request。
#[test]
fn edited_create_intent_builds_the_authoritative_wire_request() {
    let sample = gate_json("inbound-sample.json");
    let payload =
        serde_json::to_vec(&sample["rust_entry_inputs"][1]["value"]["payload"]).expect("serialize");
    let command =
        build_command("im_create_schedule", &payload).expect("编辑后的创建意图必须能通过公开边界");
    let effects = handle_outbound(
        "im_create_schedule",
        command.payload.as_ref(),
        BASE_URL,
        "http://test",
        None,
        Correlation::from_raw(2),
    )
    .expect("归一化后的 payload 必须能构造 wire request");

    let (url, body) = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some((
                req.url.clone(),
                req.body
                    .as_ref()
                    .and_then(|body| serde_json::from_slice::<Value>(body.as_ref()).ok())
                    .unwrap_or(Value::Null),
            )),
            _ => None,
        })
        .expect("必须发出一条 Http");
    assert!(url.ends_with("posts/createSchedule"), "实际 {url}");
    // 编辑后的正文必须真的进 wire —— 否则「改了内容却发出旧文案」。
    assert_eq!(body["post"]["message"], EDITED_MESSAGE);
    assert_eq!(body["post"]["channelId"], SAMPLE_CHANNEL_ID);
    assert_eq!(body["schedulePostAt"], json!(EXECUTE_AT));
}

// ── 3. 两步顺序：canceled 必须先于 created ─────────────────────────────────

/// 编辑成功路径的事件序列必须是 `schedule-canceled` → `schedule-created`，与样本一致。
#[test]
fn edit_emits_canceled_then_created_in_order() {
    let sample = gate_json("outbound-sample.json");
    let expected: Vec<&str> = sample["samples"]
        .as_array()
        .expect("samples must be array")
        .iter()
        .map(|item| item["rust"]["event"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        expected,
        vec![
            "im:channel:schedule-canceled",
            "im:channel:schedule-created"
        ]
    );

    let mut harness = creator_harness();
    let seeded = seed_existing_schedule(&mut harness);
    assert_eq!(event_names(&seeded), vec!["im:channel:schedule-created"]);

    let mut edit_events = Vec::new();
    edit_events.extend(drive_step(
        &mut harness,
        "im_cancel_schedule",
        cancel_payload("mv3-g04c-cancel"),
        &canceled_authority(CANCELED_REVISION),
        std::slice::from_ref(&canceled_row(CANCELED_REVISION)),
        NOW_MS + 100,
    ));
    edit_events.extend(drive_step(
        &mut harness,
        "im_create_schedule",
        create_payload(EDITED_MESSAGE, "mv3-g04c-create"),
        &created_authority(EDITED_SCHEDULE_ID, EDITED_MESSAGE, RECREATED_REVISION),
        std::slice::from_ref(&scheduled_row(
            EDITED_SCHEDULE_ID,
            EDITED_MESSAGE,
            RECREATED_REVISION,
        )),
        NOW_MS + 200,
    ));

    assert_eq!(event_names(&edit_events), expected, "两步事件顺序固定");
    assert_eq!(edit_events[0]["data"]["hasSchedulePost"], json!(false));
    assert_eq!(
        edit_events[0]["data"]["revision"],
        json!(CANCELED_REVISION as i64)
    );
    assert_eq!(edit_events[1]["data"]["hasSchedulePost"], json!(true));
    assert_eq!(
        edit_events[1]["data"]["revision"],
        json!(RECREATED_REVISION as i64)
    );
    // 编辑后的绝对投影必须承载**新**正文与**新** scheduleId，而不是旧定时的残留。
    assert_eq!(edit_events[1]["data"]["message"], EDITED_MESSAGE);
    assert_eq!(edit_events[1]["data"]["scheduleId"], EDITED_SCHEDULE_ID);
    // 编辑全程不得产生任何 Post —— 编辑定时不是发送。
    for name in event_names(&edit_events) {
        assert!(!name.starts_with("im:post:"), "编辑不得产生 {name}");
    }
}

/// 顺序不是靠调用方自觉：新建 revision 必须严格大于已提交取消 revision，否则被吞。
///
/// 这条守卫掉「先到 created、后到 canceled」的乱序终态——否则新建的定时会被迟到的取消
/// 事件抹掉，用户看到「刚编辑完就没了」。
#[test]
fn out_of_order_authority_is_gated_by_revision_monotonicity() {
    // ① canceled 已提交在 revision 9 之后，revision <= 9 的 created 不得复活旧定时。
    let mut harness = creator_harness();
    seed_existing_schedule(&mut harness);
    drive_step(
        &mut harness,
        "im_cancel_schedule",
        cancel_payload("mv3-g04c-cancel"),
        &canceled_authority(CANCELED_REVISION),
        std::slice::from_ref(&canceled_row(CANCELED_REVISION)),
        NOW_MS + 100,
    );
    harness.step(
        inbound(&created_authority(
            SCHEDULE_ID,
            ORIGINAL_MESSAGE,
            SEEDED_REVISION,
        )),
        NOW_MS + 300,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "旧 revision 的 created 不得复活已取消的定时"
    );

    // ② created 已提交在 revision 10 之后，revision <= 10 的 canceled 不得抹掉新定时。
    let mut reordered = creator_harness();
    seed_existing_schedule(&mut reordered);
    drive_step(
        &mut reordered,
        "im_create_schedule",
        create_payload(EDITED_MESSAGE, "mv3-g04c-create"),
        &created_authority(EDITED_SCHEDULE_ID, EDITED_MESSAGE, RECREATED_REVISION),
        std::slice::from_ref(&scheduled_row(
            EDITED_SCHEDULE_ID,
            EDITED_MESSAGE,
            RECREATED_REVISION,
        )),
        NOW_MS + 100,
    );
    reordered.step(
        inbound(&canceled_authority(CANCELED_REVISION)),
        NOW_MS + 300,
    );
    assert!(
        reordered.sink.as_slice().is_empty(),
        "迟到的取消 authority 不得抹掉 revision 更高的新定时"
    );
}

// ── 4. 部分失败：取消成功 + 新建失败 ───────────────────────────────────────

/// 取消已结算后新建 HTTP 失败：终态必须是干净的「已取消」，且可直接重试新建。
///
/// 反面（本用例守卫掉的坏状态）：
/// - 发出假的 `schedule-created`（服务端根本没建）→ 提示条显示不存在的定时；
/// - 把 channel 卡在 pending 里 → 用户再点保存不再发 HTTP，编辑永久失败。
#[test]
fn create_failure_after_cancel_leaves_a_clean_recoverable_canceled_state() {
    let mut harness = creator_harness();
    seed_existing_schedule(&mut harness);
    let canceled = drive_step(
        &mut harness,
        "im_cancel_schedule",
        cancel_payload("mv3-g04c-cancel"),
        &canceled_authority(CANCELED_REVISION),
        std::slice::from_ref(&canceled_row(CANCELED_REVISION)),
        NOW_MS + 100,
    );
    assert_eq!(event_names(&canceled), vec!["im:channel:schedule-canceled"]);

    // 第二步：新建 HTTP 失败。
    harness.step(
        command_json(
            "im_create_schedule",
            create_payload(EDITED_MESSAGE, "mv3-g04c-create-fail"),
        ),
        NOW_MS + 200,
    );
    let http_corr = corr_of(harness.sink.as_slice(), "http").expect("必须发出创建 Http");
    harness.step(port_reply_err(http_corr), NOW_MS + 201);
    assert_request_failed(harness.sink.as_slice(), "SCHEDULE_REQUEST_FAILED");

    // 失败后立即重试：必须重新发出真实 HTTP（pending 已释放，不是不可恢复中间态）。
    let retried = drive_step(
        &mut harness,
        "im_create_schedule",
        create_payload(EDITED_MESSAGE, "mv3-g04c-create-retry"),
        &created_authority(EDITED_SCHEDULE_ID, EDITED_MESSAGE, RECREATED_REVISION),
        std::slice::from_ref(&scheduled_row(
            EDITED_SCHEDULE_ID,
            EDITED_MESSAGE,
            RECREATED_REVISION,
        )),
        NOW_MS + 300,
    );
    assert_eq!(
        event_names(&retried),
        vec!["im:channel:schedule-created"],
        "取消后新建失败必须可重试并最终收敛到新定时"
    );
    assert_eq!(retried[0]["data"]["message"], EDITED_MESSAGE);
}

/// 取消 HTTP 就失败：不得发布任何取消终态，旧定时保持原样（编辑整体未生效）。
#[test]
fn cancel_failure_never_publishes_a_canceled_projection() {
    let mut harness = creator_harness();
    seed_existing_schedule(&mut harness);

    harness.step(
        command_json("im_cancel_schedule", cancel_payload("mv3-g04c-cancel-fail")),
        NOW_MS + 100,
    );
    let http_corr = corr_of(harness.sink.as_slice(), "http").expect("必须发出取消 Http");
    harness.step(port_reply_err(http_corr), NOW_MS + 101);
    assert_request_failed(harness.sink.as_slice(), "SCHEDULE_REQUEST_FAILED");

    // 取消失败后重试必须仍能发出真实 HTTP。
    let retried = drive_step(
        &mut harness,
        "im_cancel_schedule",
        cancel_payload("mv3-g04c-cancel-retry"),
        &canceled_authority(CANCELED_REVISION),
        std::slice::from_ref(&canceled_row(CANCELED_REVISION)),
        NOW_MS + 200,
    );
    assert_eq!(event_names(&retried), vec!["im:channel:schedule-canceled"]);
}

/// 取消提交（PersistAtomic）失败时同样不得发布终态，也不得吞掉后续真实 authority。
#[test]
fn cancel_persist_failure_suppresses_the_terminal_and_stays_retryable() {
    let mut harness = creator_harness();
    seed_existing_schedule(&mut harness);

    harness.step(
        command_json("im_cancel_schedule", cancel_payload("mv3-g04c-cancel-1")),
        NOW_MS + 100,
    );
    let http_corr = corr_of(harness.sink.as_slice(), "http").expect("必须发出取消 Http");
    harness.step(port_reply_with_body(http_corr, common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#))), NOW_MS + 101);
    harness.step(
        inbound(&canceled_authority(CANCELED_REVISION)),
        NOW_MS + 102,
    );
    let atomic_corr = corr_of(harness.sink.as_slice(), "atomic").expect("必须落 PersistAtomic");

    harness.step(port_reply_err(atomic_corr), NOW_MS + 103);
    assert_request_failed(harness.sink.as_slice(), "SCHEDULE_PERSIST_FAILED");

    // inflight revision 已释放：更高 revision 的 authority 重放仍能重新入屏障。
    harness.step(
        inbound(&canceled_authority(CANCELED_REVISION + 1)),
        NOW_MS + 104,
    );
    assert!(
        corr_of(harness.sink.as_slice(), "atomic").is_some(),
        "提交失败后必须仍接受后续 authority 重放"
    );
}

// ── 5. 幂等 ────────────────────────────────────────────────────────────────

/// 编辑完成后，两步的 authority 各自重复到达都不得二次发事件。
#[test]
fn duplicate_authorities_after_edit_are_idempotent() {
    let mut harness = creator_harness();
    seed_existing_schedule(&mut harness);
    drive_step(
        &mut harness,
        "im_cancel_schedule",
        cancel_payload("mv3-g04c-cancel"),
        &canceled_authority(CANCELED_REVISION),
        std::slice::from_ref(&canceled_row(CANCELED_REVISION)),
        NOW_MS + 100,
    );
    drive_step(
        &mut harness,
        "im_create_schedule",
        create_payload(EDITED_MESSAGE, "mv3-g04c-create"),
        &created_authority(EDITED_SCHEDULE_ID, EDITED_MESSAGE, RECREATED_REVISION),
        std::slice::from_ref(&scheduled_row(
            EDITED_SCHEDULE_ID,
            EDITED_MESSAGE,
            RECREATED_REVISION,
        )),
        NOW_MS + 200,
    );

    harness.step(
        inbound(&created_authority(
            EDITED_SCHEDULE_ID,
            EDITED_MESSAGE,
            RECREATED_REVISION,
        )),
        NOW_MS + 300,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "重复 created 必须被吞掉"
    );

    harness.step(
        inbound(&canceled_authority(CANCELED_REVISION)),
        NOW_MS + 301,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "重复 canceled 必须被吞掉"
    );
}

// ── 6. 越权 / 脏输入 fail-closed ──────────────────────────────────────────

/// 两条命令各自的必填校验都必须在产生任何 Effect 之前生效。
#[test]
fn both_steps_fail_closed_on_malformed_input() {
    let reject = |command: &str, payload: Value| {
        let bytes = serde_json::to_vec(&payload).expect("payload must serialize");
        handle_outbound(
            command,
            &bytes,
            BASE_URL,
            "http://test",
            None,
            Correlation::from_raw(9),
        )
        .is_err()
    };

    assert!(reject("im_cancel_schedule", json!({})));
    assert!(reject("im_cancel_schedule", json!({ "channel_id": "" })));
    assert!(reject("im_create_schedule", json!({})));
    assert!(reject(
        "im_create_schedule",
        json!({ "channel_id": channel(), "message": EDITED_MESSAGE })
    ));
    assert!(reject(
        "im_create_schedule",
        json!({
            "channel_id": channel(),
            "message": EDITED_MESSAGE,
            "schedule_post_at": EXECUTE_AT,
            "unexpected": "x"
        })
    ));
}
