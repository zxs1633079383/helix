// 回归锚: specs/004-messagev3-helix-phase1-send-domain T101，Gate MV3-G04b 取消 / 删除定时消息。
//
// 合同（gates/schedule/mv3-g04b/）：
// - `effects-contract.json.reference_sequence` = Http → Persist → Emit。
// - 出站唯一事件 `im:channel:schedule-canceled`，**绝对频道投影**（persona-local，不传播）。
// - 负向：不得产生 `im:post:*`；取消后不得仍被 MV3-G04d 读到（提示条不得复活）。
//
// 事件由 durable `channel_schedule` 读回行构造（不是把取消请求回显），因此
// 「事件说取消了但库里还留着」这类不一致在结构上不可能出现。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::Tick;
use helix_im::outbound::posts::read::schedule_projections;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{cid, command_json, port_reply_err, port_reply_ok, port_reply_with_body, Harness};

const CREATOR_ID: &str = "user-actor-444";
const OBSERVER_ID: &str = "user-receiver-445";
const SCHEDULE_ID: &str = "schedule-001";
const CANCELED_REVISION: u64 = 9;
const NOW_MS: u64 = 1_700_000_000_000;

// ── fixtures ────────────────────────────────────────────────────────────────

fn channel() -> String {
    cid(11)
}

fn gate_json(file: &str) -> Value {
    let gate_root = phase1_reference::reference_root().join("schedule/mv3-g04b");
    phase1_reference::read_json(&gate_root.join(file))
}

fn creator_harness() -> Harness {
    Harness::new_with_identity(&[11], CREATOR_ID, "company-phase1", "", "", "")
}

/// Go `post_schedule_canceled` creator authority 帧。
fn canceled_authority() -> Value {
    json!({
        "action": "post_schedule_canceled",
        "eventSeq": CANCELED_REVISION,
        "data": {
            "channelId": channel(),
            "scheduleOwnerUserId": CREATOR_ID,
            "status": "canceled",
            "revision": CANCELED_REVISION
        }
    })
}

fn inbound(frame: &Value) -> Tick {
    let bytes = serde_json::to_vec(frame).expect("frame must serialize");
    Tick::Inbound(InboundBytes(Bytes::from(bytes)))
}

/// 仍待发的 durable `channel_schedule` 行（创建链写出的形态：schedule_id 非空、status 非终态）。
fn scheduled_row() -> Row {
    vec![
        ("channel_id".to_string(), SqlValue::Text(channel())),
        (
            "schedule_id".to_string(),
            SqlValue::Text(SCHEDULE_ID.to_string()),
        ),
        (
            "owner_user_id".to_string(),
            SqlValue::Text(CREATOR_ID.to_string()),
        ),
        (
            "message".to_string(),
            SqlValue::Text("定时发送消息".to_string()),
        ),
        (
            "message_preview".to_string(),
            SqlValue::Text("定时发送消息".to_string()),
        ),
        ("send_at".to_string(), SqlValue::Integer(1_700_003_600_000)),
        (
            "status".to_string(),
            SqlValue::Text("scheduled".to_string()),
        ),
        (
            "revision".to_string(),
            SqlValue::Integer(CANCELED_REVISION as i64 + 1),
        ),
    ]
}

/// 取消后的 durable `channel_schedule` 行（正文全清、状态终态）。
fn canceled_row() -> Row {
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
        (
            "revision".to_string(),
            SqlValue::Integer(CANCELED_REVISION as i64),
        ),
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

fn corr_of(effects: &[Effect], kind: &str) -> Option<u64> {
    effects.iter().find_map(|effect| match (kind, effect) {
        ("http", Effect::Http { corr, .. }) => Some(corr.raw()),
        ("atomic", Effect::PersistAtomic { corr, .. }) => Some(corr.raw()),
        ("persist", Effect::Persist { corr, .. }) => Some(corr.raw()),
        _ => None,
    })
}

/// 发出取消命令并回报 HTTP 2xx（admission），返回 HTTP correlation。
fn issue_cancel(harness: &mut Harness) -> u64 {
    harness.step(
        command_json(
            "im_cancel_schedule",
            json!({ "channel_id": channel(), "req_id": "mv3-g04b-cancel-1" }),
        ),
        NOW_MS,
    );
    corr_of(harness.sink.as_slice(), "http").expect("取消必须发出 posts/cancelSchedule")
}

/// 走到 WS authority 已入原子屏障的状态，返回 PersistAtomic correlation。
fn drive_to_authority(harness: &mut Harness) -> u64 {
    let http_corr = issue_cancel(harness);
    harness.step(
        port_reply_with_body(
            http_corr,
            common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
        ),
        NOW_MS + 1,
    );
    harness.step(inbound(&canceled_authority()), NOW_MS + 2);
    corr_of(harness.sink.as_slice(), "atomic").expect("WS authority 必须落 PersistAtomic")
}

// ── 1. 参考包完整性 ─────────────────────────────────────────────────────────

/// 证明 MV3-G04b 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G04b", "schedule/mv3-g04b");
}

// ── 2. 命令 → 真实 wire request ────────────────────────────────────────────

/// 取消命令只发一条 `posts/cancelSchedule`，body 只带 `channelId`（owner 由 session 派生）。
#[test]
fn cancel_command_emits_only_the_authoritative_http() {
    let mut harness = creator_harness();
    harness.step(
        command_json(
            "im_cancel_schedule",
            json!({ "channel_id": channel(), "req_id": "mv3-g04b-cancel-1" }),
        ),
        NOW_MS,
    );
    let effects = harness.sink.as_slice();
    assert_eq!(effects.len(), 1, "取消命令阶段只允许一条 Http");
    match &effects[0] {
        Effect::Http { req, .. } => {
            assert!(
                req.url.ends_with("posts/cancelSchedule"),
                "endpoint 必须是 posts/cancelSchedule，实际 {}",
                req.url
            );
            let body: Value = req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok())
                .expect("cancel body must be JSON");
            assert_eq!(body, json!({ "channelId": channel() }));
        }
        other => panic!("取消必须是 Http，实际 {other:?}"),
    }
    assert!(emitted(effects).is_empty(), "命令阶段零事件");
}

// ── 3. HTTP 2xx 不是权威 ───────────────────────────────────────────────────

/// HTTP 2xx 只是 admission：不得据此发布取消终态（INV-03 的定时域对应物）。
#[test]
fn http_2xx_alone_never_settles_the_cancel() {
    let mut harness = creator_harness();
    let http_corr = issue_cancel(&mut harness);
    harness.step(
        port_reply_with_body(
            http_corr,
            common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
        ),
        NOW_MS + 1,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "HTTP 成功不得产生任何 Effect / 事件"
    );
}

// ── 4. WS authority → 原子写集合，事件仍未释放 ─────────────────────────────

/// authority 必须把 `channel_schedule` 终态与 `channel` 读模型放进同一个原子屏障。
#[test]
fn ws_authority_persists_atomically_before_any_event() {
    let mut harness = creator_harness();
    let _ = drive_to_authority(&mut harness);
    let effects = harness.sink.as_slice();
    assert!(emitted(effects).is_empty(), "原子写提交前不得发布终态");

    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops),
            _ => None,
        })
        .expect("必须落 PersistAtomic");
    assert_eq!(ops.len(), 2, "schedule 事实 + channel 读模型必须同屏障");

    let schedule_row = ops
        .iter()
        .find_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "channel_schedule" => spec.rows.first(),
            _ => None,
        })
        .expect("必须写 channel_schedule");
    // 正文全清 + 终态 canceled：MV3-G04d 之后不可能再读出待发对象（提示条不会复活）。
    assert!(schedule_row
        .iter()
        .any(|(column, value)| column == "schedule_id"
            && matches!(value, SqlValue::Text(v) if v.is_empty())));
    assert!(schedule_row
        .iter()
        .any(|(column, value)| column == "message"
            && matches!(value, SqlValue::Text(v) if v.is_empty())));
    assert!(schedule_row.iter().any(|(column, value)| column == "status"
        && matches!(value, SqlValue::Text(v) if v == "canceled")));
    assert!(schedule_row
        .iter()
        .any(|(column, value)| column == "send_at" && matches!(value, SqlValue::Integer(0))));

    let channel_patch = ops
        .iter()
        .find_map(|op| match op {
            StorageOp::BatchUpdate(spec) if spec.table == "channel" => Some(&spec.patch),
            _ => None,
        })
        .expect("必须同步 channel 读模型");
    assert!(channel_patch
        .iter()
        .any(|(column, value)| column == "has_schedule_post"
            && matches!(value, SqlValue::Integer(0))));
    assert!(channel_patch.iter().any(|(column, value)| {
        column == "schedule_revision"
            && matches!(value, SqlValue::Integer(v) if *v == CANCELED_REVISION as i64)
    }));
}

// ── 5. PersistOk → durable read-back ───────────────────────────────────────

/// 提交成功只触发读回，终态仍不释放——事件必须来自 durable 行而非请求回显。
#[test]
fn persist_ok_requests_the_durable_readback_without_emitting() {
    let mut harness = creator_harness();
    let atomic_corr = drive_to_authority(&mut harness);
    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);

    let effects = harness.sink.as_slice();
    assert!(emitted(effects).is_empty(), "读回前不得发布终态");
    let ops = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { ops, .. } => Some(ops),
            _ => None,
        })
        .expect("PersistOk 必须请求 durable 读回");
    match &ops[0] {
        StorageOp::Get(spec) => {
            assert_eq!(spec.table, "channel_schedule");
            assert_eq!(spec.key_col, "channel_id");
            assert!(matches!(&spec.key_val, SqlValue::Text(value) if *value == channel()));
        }
        other => panic!("读回必须是 channel_schedule 单行 Get，实际 {other:?}"),
    }
}

// ── 6. 读回行 → 绝对频道投影 ───────────────────────────────────────────────

/// 唯一终态事件从 durable 行构造，字段与合同 `payloadShape` 一致。
#[test]
fn readback_row_publishes_the_absolute_channel_projection() {
    let sample = gate_json("outbound-sample.json");
    let expected = sample["samples"][0]["rust"].clone();

    let mut harness = creator_harness();
    let atomic_corr = drive_to_authority(&mut harness);
    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须请求 durable 读回");
    harness.step(
        port_reply_with_body(
            readback_corr,
            rows_to_reply_bytes(std::slice::from_ref(&canceled_row())),
        ),
        NOW_MS + 4,
    );

    let events = emitted(harness.sink.as_slice());
    assert_eq!(events.len(), 1, "取消只发布一条终态事件");
    assert_eq!(events[0]["event"], expected["event"]);
    assert_eq!(events[0]["event"], "im:channel:schedule-canceled");
    assert_eq!(events[0]["data"]["channelId"], channel());
    assert_eq!(events[0]["data"]["status"], expected["data"]["status"]);
    assert_eq!(events[0]["data"]["status"], "canceled");
    // 绝对频道投影：提示条靠 hasSchedulePost 消失，revision 用于乱序防回退。
    assert_eq!(events[0]["data"]["hasSchedulePost"], json!(false));
    assert_eq!(
        events[0]["data"]["revision"],
        json!(CANCELED_REVISION as i64)
    );
}

// ── 7. 负向：取消不是发送 ──────────────────────────────────────────────────

/// 全链路不得出现任何 `im:post:*`（合同负向断言：取消不产生 postReceived / channelUpdateByPost）。
#[test]
fn cancel_never_publishes_a_post_event() {
    let mut harness = creator_harness();
    let mut all = Vec::new();
    let http_corr = issue_cancel(&mut harness);
    all.extend(emitted(harness.sink.as_slice()));
    harness.step(
        port_reply_with_body(
            http_corr,
            common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
        ),
        NOW_MS + 1,
    );
    all.extend(emitted(harness.sink.as_slice()));
    harness.step(inbound(&canceled_authority()), NOW_MS + 2);
    all.extend(emitted(harness.sink.as_slice()));
    let atomic_corr = corr_of(harness.sink.as_slice(), "atomic").expect("必须落 PersistAtomic");
    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    all.extend(emitted(harness.sink.as_slice()));
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须请求读回");
    harness.step(
        port_reply_with_body(
            readback_corr,
            rows_to_reply_bytes(std::slice::from_ref(&canceled_row())),
        ),
        NOW_MS + 4,
    );
    all.extend(emitted(harness.sink.as_slice()));

    assert_eq!(all.len(), 1, "全链路恰好一条事件");
    for event in &all {
        let name = event["event"].as_str().unwrap_or_default();
        assert!(!name.starts_with("im:post:"), "取消不得产生 {name}");
    }
}

// ── 8. 取消后不可被 MV3-G04d 读回 ──────────────────────────────────────────

/// 服务端不再返回待发对象时，查询投影必须是空集合——提示条不会复活、不会到点二次发送。
#[test]
fn canceled_schedule_is_not_readable_afterwards() {
    for body in [json!({ "data": [] }), json!({ "data": null }), json!({})] {
        assert_eq!(schedule_projections(&channel(), &body), json!([]));
    }
}

// ── 9. 重复 authority 幂等 ─────────────────────────────────────────────────

/// 同一 revision 的 authority 重复到达不得二次写库、二次发事件。
#[test]
fn duplicate_authority_is_idempotent() {
    let mut harness = creator_harness();
    let atomic_corr = drive_to_authority(&mut harness);
    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须请求读回");
    harness.step(
        port_reply_with_body(
            readback_corr,
            rows_to_reply_bytes(std::slice::from_ref(&canceled_row())),
        ),
        NOW_MS + 4,
    );
    assert_eq!(emitted(harness.sink.as_slice()).len(), 1);

    harness.step(inbound(&canceled_authority()), NOW_MS + 5);
    assert!(
        harness.sink.as_slice().is_empty(),
        "已提交 revision 的重复 authority 必须被吞掉"
    );
}

// ── 10. 提交失败抑制终态 ───────────────────────────────────────────────────

/// PersistErr → 不读回、不发布终态（本地不得单方面宣称已取消）。
#[test]
fn persist_failure_suppresses_the_terminal_projection() {
    let mut harness = creator_harness();
    let atomic_corr = drive_to_authority(&mut harness);
    harness.step(port_reply_err(atomic_corr), NOW_MS + 3);
    assert!(
        emitted(harness.sink.as_slice()).is_empty(),
        "提交失败不得发布业务终态"
    );
    assert!(
        corr_of(harness.sink.as_slice(), "persist").is_none(),
        "提交失败不得读回"
    );
    let failures: Vec<Value> = harness
        .sink
        .as_slice()
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["event"], "im:read:result");
    assert_eq!(failures[0]["data"]["error"], "SCHEDULE_PERSIST_FAILED");
}

/// 读回缺行 / 读回失败 → 抑制终态（宁可提示条不消失，也不发假终态）。
#[test]
fn readback_without_a_durable_row_suppresses_the_event() {
    let mut harness = creator_harness();
    let atomic_corr = drive_to_authority(&mut harness);
    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须请求读回");
    harness.step(
        port_reply_with_body(readback_corr, rows_to_reply_bytes(&[])),
        NOW_MS + 4,
    );
    assert!(emitted(harness.sink.as_slice()).is_empty());
}

// ── 10b. 读回方向守卫：绝对投影必须与 durable 行同向 ───────────────────────

/// 取消读回落在**仍待发**的行上时，不得发布 `hasSchedulePost=false`。
///
/// 触发场景：取消提交成功后、读回返回前，同频道又建了一条定时（`channel_schedule` 是
/// 单行 per-channel 读模型，会被新建覆写）。此时照发取消终态会把一条**仍会到点发出**的
/// 定时消息的提示条抹掉：用户看不到提示条 → 以为没设上 → 再设一条 → 到点发两条。
/// 事件必须与 durable 行同向，否则「绝对频道投影」这个词就是假的。
#[test]
fn cancel_readback_on_an_active_row_suppresses_the_terminal_projection() {
    let mut harness = creator_harness();
    let atomic_corr = drive_to_authority(&mut harness);
    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须请求读回");
    harness.step(
        port_reply_with_body(
            readback_corr,
            rows_to_reply_bytes(std::slice::from_ref(&scheduled_row())),
        ),
        NOW_MS + 4,
    );
    assert!(
        emitted(harness.sink.as_slice()).is_empty(),
        "durable 行仍待发时不得发布 hasSchedulePost=false"
    );

    // 同一条链换成真正的 canceled 行仍必须发布终态——守卫不能把正常路径一起吞掉。
    let mut ok = creator_harness();
    let atomic_corr = drive_to_authority(&mut ok);
    ok.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    let readback_corr = corr_of(ok.sink.as_slice(), "persist").expect("必须请求读回");
    ok.step(
        port_reply_with_body(
            readback_corr,
            rows_to_reply_bytes(std::slice::from_ref(&canceled_row())),
        ),
        NOW_MS + 4,
    );
    let events = emitted(ok.sink.as_slice());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["hasSchedulePost"], json!(false));
}

/// 迟到的**创建**读回落在已取消的行上时，不得发布 `hasSchedulePost=true`。
///
/// 这是合同负向断言「取消后提示条不得复活」的读回侧对应物：创建读回把 `hasSchedulePost`
/// 写死成 true，若不校验 durable 行方向，一个迟到的创建读回就能把已取消的提示条点亮，
/// 而库里根本没有待发对象（MV3-G04d 读回来是空的）——UI 与事实直接对不上。
#[test]
fn stale_created_readback_on_a_canceled_row_never_revives_the_banner() {
    let mut harness = creator_harness();
    harness.step(
        command_json(
            "im_create_schedule",
            json!({
                "channel_id": channel(),
                "message": "定时发送消息",
                "schedule_post_at": 1_700_003_600_000i64,
                "type": "TEXT",
                "props": {},
                "req_id": "mv3-g04b-create-1"
            }),
        ),
        NOW_MS,
    );
    let http_corr = corr_of(harness.sink.as_slice(), "http").expect("创建必须发出 Http");
    harness.step(
        port_reply_with_body(
            http_corr,
            common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#)),
        ),
        NOW_MS + 1,
    );
    harness.step(
        inbound(&json!({
            "action": "post_schedule_created",
            "eventSeq": CANCELED_REVISION - 1,
            "data": {
                "channelId": channel(),
                "scheduleId": SCHEDULE_ID,
                "scheduleOwnerUserId": CREATOR_ID,
                "message": "定时发送消息",
                "messagePreview": "定时发送消息",
                "sendAt": 1_700_003_600_000i64,
                "status": "scheduled",
                "revision": CANCELED_REVISION - 1
            }
        })),
        NOW_MS + 2,
    );
    let atomic_corr = corr_of(harness.sink.as_slice(), "atomic").expect("必须落 PersistAtomic");
    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    let readback_corr = corr_of(harness.sink.as_slice(), "persist").expect("必须请求读回");
    harness.step(
        port_reply_with_body(
            readback_corr,
            rows_to_reply_bytes(std::slice::from_ref(&canceled_row())),
        ),
        NOW_MS + 4,
    );
    assert!(
        emitted(harness.sink.as_slice()).is_empty(),
        "durable 行已取消时不得发布 hasSchedulePost=true（提示条复活）"
    );
}

// ── 11. persona-local：接收者零状态变化 ────────────────────────────────────

/// creator-private 取消广播落到非 owner 时必须整体 no-op（合同：receiver 侧 outbound 必须为 []）。
#[test]
fn observer_persona_is_a_full_no_op() {
    let mut observer = Harness::new_with_identity(&[11], OBSERVER_ID, "company-phase1", "", "", "");
    observer.step(inbound(&canceled_authority()), NOW_MS + 2);
    assert!(
        observer.sink.as_slice().is_empty(),
        "非 owner 不得因 creator 的取消产生任何 Effect"
    );
}

// ── 12. 越权 / 脏输入 fail-closed ──────────────────────────────────────────

/// 未知字段与缺 `channel_id` 在产生任何 Effect 之前拒绝（合同 §2 forbid_unknown）。
#[test]
fn malformed_cancel_input_fails_closed() {
    use helix_core::Correlation;
    use helix_im::outbound::handle_outbound;

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
                "http://test/api/cses",
                "http://test",
                None,
                Correlation::from_raw(7),
            )
            .is_err(),
            "{payload} 必须 fail-closed"
        );
    }

    // 无 revision 的 authority 帧不得推进本地终态（防止 0 值覆盖已提交 revision）。
    let mut harness = creator_harness();
    let _ = issue_cancel(&mut harness);
    harness.step(
        inbound(&json!({
            "action": "post_schedule_canceled",
            "data": { "channelId": channel(), "scheduleOwnerUserId": CREATOR_ID }
        })),
        NOW_MS + 2,
    );
    assert!(harness.sink.as_slice().is_empty());
}

// ── 13. Gate identity ──────────────────────────────────────────────────────

/// 参考包的 inbound sample 必须仍然指向本 Gate 的权威命令名。
#[test]
fn inbound_sample_targets_the_cancel_command() {
    let sample = gate_json("inbound-sample.json");
    assert_eq!(
        sample["rust_entry_inputs"][0]["value"]["name"],
        "im_cancel_schedule"
    );
    assert_eq!(
        sample["rust_entry_inputs"][0]["value"]["payload"]["channel_id"],
        "channel-phase1-001"
    );
    assert!(
        sample["rust_entry_inputs"][0]["value"]["payload"]
            .get("schedule_id")
            .is_none(),
        "取消按频道唯一待发项执行，sample 不得注入未被生产边界接受的 schedule_id"
    );
}
