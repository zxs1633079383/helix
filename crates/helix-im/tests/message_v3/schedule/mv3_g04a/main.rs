// 回归锚: specs/004-messagev3-helix-phase1-send-domain T097 / T098，Gate MV3-G04a 创建定时消息。
//
// 合同（gates/schedule/mv3-g04a/）：
// - `effects-contract.json.allowed_effect_tags = ["Http","Persist","Emit"]`，
//   `reference_sequence` = Http → Persist → Emit。
// - 出站唯一事件 `im:channel:schedule-created`，**persona-local**（非 owner 零反应）。
// - `inbound-sample.json` 的 Angular 意图形状是 `{channel_id, execute_at, post:{type,text,props,…}}`，
//   而内核 `im_create_schedule` 的冻结键集是 `{channel_id, message, schedule_post_at, type, props, req_id}`
//   —— 两者之间的收敛只允许发生在 `client_api::build_command` 的公开边界（T098）。
//
// **INV-09 是本 Gate 的核心**：Go 是定时消息到点执行权威，Helix 只负责把创建请求送达并把服务端
// 权威回声投影给 Angular。因此全链路**零 ScheduleTimer**——一旦有人给创建链加本地执行 timer，
// `create_schedule_never_arms_a_local_execution_timer` 与 `effects_stay_within_the_contract_tag_set`
// 会同时变红。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use helix_im::client_api::build_command;
use helix_im::outbound::{handle_outbound, is_outbound, is_read};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{cid, command_json, port_reply_err, port_reply_ok, port_reply_with_body, Harness};

const CREATOR_ID: &str = "user-actor-444";
const OBSERVER_ID: &str = "user-receiver-445";
const SCHEDULE_ID: &str = "schedule-001";
const MESSAGE: &str = "定时发送消息";
const PREVIEW: &str = "定时发送消息";
const EXECUTE_AT: i64 = 1_700_003_600_000;
const REVISION: u64 = 8;
const NOW_MS: u64 = 1_700_000_000_000;
const SAMPLE_CHANNEL_ID: &str = "channel-phase1-001";
const BASE_URL: &str = "http://test/api/cses";

// ── fixtures ────────────────────────────────────────────────────────────────

fn channel() -> String {
    cid(11)
}

fn gate_json(file: &str) -> Value {
    let gate_root = phase1_reference::reference_root().join("schedule/mv3-g04a");
    phase1_reference::read_json(&gate_root.join(file))
}

fn creator_harness() -> Harness {
    Harness::new_with_identity(&[11], CREATOR_ID, "company-phase1", "", "", "")
}

/// 内核冻结键集形态的创建 payload（公开边界归一化之后的样子）。
fn core_payload() -> Value {
    json!({
        "channel_id": channel(),
        "message": MESSAGE,
        "schedule_post_at": EXECUTE_AT,
        "type": "TEXT",
        "props": {},
        "req_id": "mv3-g04a-create-1"
    })
}

/// Go `post_schedule_created` creator authority 帧。
fn created_authority() -> Value {
    json!({
        "action": "post_schedule_created",
        "eventSeq": REVISION,
        "data": {
            "channelId": channel(),
            "scheduleId": SCHEDULE_ID,
            "scheduleOwnerUserId": CREATOR_ID,
            "message": MESSAGE,
            "messagePreview": PREVIEW,
            "sendAt": EXECUTE_AT,
            "status": "scheduled",
            "revision": REVISION
        }
    })
}

fn inbound(frame: &Value) -> Tick {
    let bytes = serde_json::to_vec(frame).expect("frame must serialize");
    Tick::Inbound(InboundBytes(Bytes::from(bytes)))
}

/// 创建提交后的 durable `channel_schedule` 行。
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
        ("message".to_string(), SqlValue::Text(MESSAGE.to_string())),
        (
            "message_preview".to_string(),
            SqlValue::Text(PREVIEW.to_string()),
        ),
        ("send_at".to_string(), SqlValue::Integer(EXECUTE_AT)),
        (
            "status".to_string(),
            SqlValue::Text("scheduled".to_string()),
        ),
        ("revision".to_string(), SqlValue::Integer(REVISION as i64)),
    ]
}

/// 把 Effect 归到 `effects-contract.json` 使用的 tag 词表。
///
/// `PersistAtomic` / `PersistFire` 都是持久化家族，归到合同的 `Persist`；timer / upload / send
/// 各自独立成 tag —— 合同 `allowed_effect_tags` 不含它们，所以一旦创建链私自起本地 timer，
/// 契约校验会立刻失败（这正是 INV-09 的机器守卫）。
fn effect_tag(effect: &Effect) -> &'static str {
    match effect {
        Effect::Http { .. } | Effect::HttpFire { .. } => "Http",
        Effect::Persist { .. } | Effect::PersistAtomic { .. } | Effect::PersistFire { .. } => {
            "Persist"
        }
        Effect::Emit { .. } => "Emit",
        Effect::ScheduleTimer { .. } => "ScheduleTimer",
        Effect::CancelTimer { .. } => "CancelTimer",
        Effect::UploadFile { .. } => "UploadFile",
        Effect::Request { .. } => "Request",
        Effect::Send { .. } => "Send",
    }
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

/// 完整创建链的一次回放记录：按批次顺序累积的 tag 序列与业务事件。
struct CreateTrace {
    tags: Vec<&'static str>,
    events: Vec<Value>,
}

impl CreateTrace {
    fn absorb(&mut self, harness: &Harness) {
        for effect in harness.sink.as_slice() {
            self.tags.push(effect_tag(effect));
        }
        self.events.extend(emitted(harness.sink.as_slice()));
    }
}

/// 从 Command 一路驱动到 durable read-back，记录每一批 Effect 的 tag 与事件。
///
/// `readback_rows` 为空表示读回缺行（用于抑制路径）。
fn replay_create(harness: &mut Harness, readback_rows: &[Row]) -> CreateTrace {
    let mut trace = CreateTrace {
        tags: Vec::new(),
        events: Vec::new(),
    };

    harness.step(command_json("im_create_schedule", core_payload()), NOW_MS);
    trace.absorb(harness);
    let http_corr =
        corr_of(harness.sink.as_slice(), "http").expect("创建必须发出 posts/createSchedule");

    harness.step(port_reply_with_body(http_corr, common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#))), NOW_MS + 1);
    trace.absorb(harness);

    harness.step(inbound(&created_authority()), NOW_MS + 2);
    trace.absorb(harness);
    let atomic_corr =
        corr_of(harness.sink.as_slice(), "atomic").expect("WS authority 必须落 PersistAtomic");

    harness.step(port_reply_ok(atomic_corr), NOW_MS + 3);
    trace.absorb(harness);
    let readback_corr =
        corr_of(harness.sink.as_slice(), "persist").expect("提交成功必须请求 durable 读回");

    harness.step(
        port_reply_with_body(readback_corr, rows_to_reply_bytes(readback_rows)),
        NOW_MS + 4,
    );
    trace.absorb(harness);
    trace
}

// ── 1. 参考包完整性 ─────────────────────────────────────────────────────────

/// 证明 MV3-G04a 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G04a", "schedule/mv3-g04a");
}

// ── 2. 参考意图 → 真实 wire request（T098 公开边界收敛）────────────────────

/// 参考包 `inbound-sample.json` 的 Angular 意图必须真的能驱动出现网 wire request。
///
/// 样本用 `execute_at` + 嵌套 `post{type,text,props}`；内核 builder 的冻结键集是
/// `{channel_id, message, schedule_post_at, type, props, req_id}` 且 `forbid_unknown`。
/// 这条用例把「样本 → build_command → handle_outbound → Http」整条链钉死：
/// 公开边界少做任何一步映射，样本就到不了 `posts/createSchedule`。
#[test]
fn reference_intent_builds_the_authoritative_wire_request() {
    let sample = gate_json("inbound-sample.json");
    let input = sample["rust_entry_inputs"][0]["value"].clone();
    assert_eq!(input["name"], "im_create_schedule");
    assert!(is_outbound("im_create_schedule"));
    // 写族：HTTP 2xx 只是 admission，结算靠 WS authority，不能被注册成读族回灌。
    assert!(!is_read("im_create_schedule"));

    let payload = serde_json::to_vec(&input["payload"]).expect("payload must serialize");
    let command =
        build_command("im_create_schedule", &payload).expect("参考样本意图必须能通过公开边界");
    assert_eq!(command.name.as_ref(), "im_create_schedule");

    let effects = handle_outbound(
        "im_create_schedule",
        command.payload.as_ref(),
        BASE_URL,
        "http://test",
        None,
        Correlation::from_raw(1),
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
        .expect("创建必须发出一条 Http");

    assert!(
        url.ends_with("posts/createSchedule"),
        "endpoint 必须是 posts/createSchedule，实际 {url}"
    );
    assert_eq!(
        body,
        json!({
            "post": {
                "channelId": SAMPLE_CHANNEL_ID,
                "message": MESSAGE,
                "type": "TEXT",
                "props": {}
            },
            "schedulePostAt": EXECUTE_AT
        })
    );
    // Angular 侧的纯渲染字段不得渗进服务端 Post（INV-01）。
    assert!(body["post"].get("viewers").is_none());
    assert!(body["post"].get("mentions").is_none());
    assert!(body["post"].get("text").is_none());
    // temporaryId 归 Helix 发送状态机；定时入口不得携带（G08 冻结约束）。
    assert!(body["post"].get("temporaryId").is_none());
}

// ── 3. 命令阶段：唯一 Http，零事件 ─────────────────────────────────────────

/// 创建命令阶段只允许一条 `posts/createSchedule`，绝不先行发布 created 终态。
#[test]
fn create_command_emits_only_the_authoritative_http() {
    let mut harness = creator_harness();
    harness.step(command_json("im_create_schedule", core_payload()), NOW_MS);

    let effects = harness.sink.as_slice();
    assert_eq!(effects.len(), 1, "创建命令阶段只允许一条 Http");
    match &effects[0] {
        Effect::Http { req, .. } => {
            assert!(req.url.ends_with("posts/createSchedule"));
            let body: Value = req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok())
                .expect("create body must be JSON");
            assert_eq!(body["schedulePostAt"], json!(EXECUTE_AT));
            assert_eq!(body["post"]["channelId"], channel());
            assert_eq!(body["post"]["message"], MESSAGE);
        }
        other => panic!("创建必须是 Http，实际 {other:?}"),
    }
    assert!(emitted(effects).is_empty(), "命令阶段零事件");
}

// ── 4. INV-09：零本地执行 timer ────────────────────────────────────────────

/// 全链路不得出现任何 timer Effect —— Go 才是到点执行权威（INV-09）。
///
/// 这条用例覆盖 Command / HttpOk / WS authority / PersistOk / read-back 五个批次。
/// 只要有人给创建链补一个「到 executeAt 自己发消息」的本地 timer，这里立刻变红。
#[test]
fn create_schedule_never_arms_a_local_execution_timer() {
    let mut harness = creator_harness();
    let trace = replay_create(&mut harness, std::slice::from_ref(&scheduled_row()));

    assert!(
        !trace
            .tags
            .iter()
            .any(|tag| *tag == "ScheduleTimer" || *tag == "CancelTimer"),
        "定时消息创建链不得起本地 timer，实际 tag 序列 {:?}",
        trace.tags
    );
    // 终态必须来自服务端权威回声 + durable 读回，而不是本地到点触发。
    assert_eq!(trace.events.len(), 1);
    assert_eq!(trace.events[0]["event"], "im:channel:schedule-created");
    // 创建定时消息本身不产生任何 Post —— 到点后由 Go 推 `action=post`，不在本 Gate。
    for event in &trace.events {
        let name = event["event"].as_str().unwrap_or_default();
        assert!(!name.starts_with("im:post:"), "创建定时不得产生 {name}");
    }
}

/// 每一个 Effect 的 tag 必须落在 `effects-contract.json.allowed_effect_tags` 内。
///
/// 合同是权威、断言从 JSON 读取，因此 tag 词表以合同为准而不是以实现为准。
#[test]
fn effects_stay_within_the_contract_tag_set() {
    let contract = gate_json("effects-contract.json");
    let allowed: Vec<String> = contract["allowed_effect_tags"]
        .as_array()
        .expect("allowed_effect_tags must be an array")
        .iter()
        .map(|tag| tag.as_str().unwrap_or_default().to_string())
        .collect();
    assert!(allowed.contains(&"Http".to_string()));
    assert!(
        !allowed.contains(&"ScheduleTimer".to_string()),
        "合同不允许本地 timer（INV-09）"
    );

    let mut harness = creator_harness();
    let trace = replay_create(&mut harness, std::slice::from_ref(&scheduled_row()));
    for tag in &trace.tags {
        assert!(
            allowed.contains(&tag.to_string()),
            "越界 Effect tag {tag}，合同只允许 {allowed:?}"
        );
    }
}

/// Effect 顺序必须匹配合同 `reference_sequence`：Http 先于 Persist，Persist 先于 Emit。
#[test]
fn effect_order_matches_the_reference_sequence() {
    let contract = gate_json("effects-contract.json");
    let sequence: Vec<String> = contract["reference_sequence"]
        .as_array()
        .expect("reference_sequence must be an array")
        .iter()
        .map(|step| step["tag"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(sequence, vec!["Http", "Persist", "Emit"]);

    let mut harness = creator_harness();
    let trace = replay_create(&mut harness, std::slice::from_ref(&scheduled_row()));
    let first = |tag: &str| {
        trace
            .tags
            .iter()
            .position(|actual| *actual == tag)
            .unwrap_or_else(|| panic!("链路缺少 {tag}，实际 {:?}", trace.tags))
    };
    assert!(first("Http") < first("Persist"), "Http 必须先于 Persist");
    assert!(first("Persist") < first("Emit"), "Persist 必须先于 Emit");
}

// ── 5. HTTP 2xx 不是权威 ───────────────────────────────────────────────────

/// HTTP 2xx 只是 admission：不得据此发布 created 终态（INV-03 的定时域对应物）。
#[test]
fn http_2xx_alone_never_settles_the_create() {
    let mut harness = creator_harness();
    harness.step(command_json("im_create_schedule", core_payload()), NOW_MS);
    let http_corr = corr_of(harness.sink.as_slice(), "http").expect("必须发出创建 Http");

    harness.step(port_reply_with_body(http_corr, common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#))), NOW_MS + 1);
    assert!(
        harness.sink.as_slice().is_empty(),
        "HTTP 成功不得产生任何 Effect / 事件"
    );
}

// ── 6. 读回行 → 绝对频道投影 ───────────────────────────────────────────────

/// 唯一终态事件从 durable 行构造，字段与 `outbound-sample.json` 一致。
#[test]
fn readback_row_publishes_the_created_projection() {
    let sample = gate_json("outbound-sample.json");
    let expected = sample["samples"][0]["rust"].clone();

    let mut harness = creator_harness();
    let trace = replay_create(&mut harness, std::slice::from_ref(&scheduled_row()));

    assert_eq!(trace.events.len(), 1, "创建只发布一条终态事件");
    let event = &trace.events[0];
    assert_eq!(event["event"], expected["event"]);
    assert_eq!(event["event"], "im:channel:schedule-created");
    assert_eq!(event["data"]["channelId"], channel());
    assert_eq!(event["data"]["scheduleId"], expected["data"]["scheduleId"]);
    assert_eq!(event["data"]["status"], expected["data"]["status"]);
    assert_eq!(event["data"]["status"], "scheduled");
    // 绝对频道投影：提示条靠 hasSchedulePost 亮起，revision 用于乱序防回退。
    assert_eq!(event["data"]["hasSchedulePost"], json!(true));
    assert_eq!(event["data"]["revision"], json!(REVISION as i64));
    // 正文与到点时间必须来自 durable 行（提示条要显示「将于 X 发送」）。
    assert_eq!(event["data"]["message"], MESSAGE);
    assert_eq!(event["data"]["sendAt"], json!(EXECUTE_AT));
    assert_eq!(event["data"]["ownerUserId"], CREATOR_ID);
}

// ── 7. 抑制路径 ────────────────────────────────────────────────────────────

/// PersistErr → 不读回、不发布终态（本地不得单方面宣称定时已建）。
#[test]
fn persist_failure_suppresses_the_created_projection() {
    let mut harness = creator_harness();
    harness.step(command_json("im_create_schedule", core_payload()), NOW_MS);
    let http_corr = corr_of(harness.sink.as_slice(), "http").expect("必须发出创建 Http");
    harness.step(port_reply_with_body(http_corr, common::enveloped(Bytes::from_static(br#"{"status":"SUCCESS"}"#))), NOW_MS + 1);
    harness.step(inbound(&created_authority()), NOW_MS + 2);
    let atomic_corr = corr_of(harness.sink.as_slice(), "atomic").expect("必须落 PersistAtomic");

    harness.step(port_reply_err(atomic_corr), NOW_MS + 3);
    assert!(emitted(harness.sink.as_slice()).is_empty(), "提交失败不得发布业务终态");
    assert!(corr_of(harness.sink.as_slice(), "persist").is_none(), "提交失败不得读回");
    let failures: Vec<Value> = harness.sink.as_slice().iter().filter_map(|effect| match effect {
        Effect::Emit {event} => serde_json::from_slice(event.0.as_ref()).ok(), _ => None
    }).collect();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["event"], "im:read:result");
    assert_eq!(failures[0]["data"]["error"], "SCHEDULE_PERSIST_FAILED");
}

/// 读回缺行 → 抑制终态（宁可提示条不亮，也不发假 created）。
#[test]
fn readback_without_a_durable_row_suppresses_the_event() {
    let mut harness = creator_harness();
    let trace = replay_create(&mut harness, &[]);
    assert!(trace.events.is_empty(), "缺 durable 行不得发布 created");
}

// ── 8. 重复 authority 幂等 ─────────────────────────────────────────────────

/// 同一 revision 的 authority 重复到达不得二次写库、二次发事件。
#[test]
fn duplicate_authority_is_idempotent() {
    let mut harness = creator_harness();
    let trace = replay_create(&mut harness, std::slice::from_ref(&scheduled_row()));
    assert_eq!(trace.events.len(), 1);

    harness.step(inbound(&created_authority()), NOW_MS + 5);
    assert!(
        harness.sink.as_slice().is_empty(),
        "已提交 revision 的重复 authority 必须被吞掉"
    );
}

// ── 9. persona-local：非 owner 零反应 ──────────────────────────────────────

/// creator-private 创建广播落到非 owner 时必须整体 no-op（合同 receiver 视角为空）。
#[test]
fn observer_persona_is_a_full_no_op() {
    let mut observer = Harness::new_with_identity(&[11], OBSERVER_ID, "company-phase1", "", "", "");
    observer.step(inbound(&created_authority()), NOW_MS + 2);
    assert!(
        observer.sink.as_slice().is_empty(),
        "非 owner 不得因 creator 的定时创建产生任何 Effect"
    );
}

// ── 10. 越权 / 脏输入 fail-closed ──────────────────────────────────────────

/// 未知字段、缺必填与非法到点时间必须在产生任何 Effect 之前拒绝（合同 forbid_unknown）。
#[test]
fn malformed_create_input_fails_closed() {
    let reject = |payload: Value| {
        let bytes = serde_json::to_vec(&payload).expect("payload must serialize");
        handle_outbound(
            "im_create_schedule",
            &bytes,
            BASE_URL,
            "http://test",
            None,
            Correlation::from_raw(7),
        )
        .is_err()
    };

    assert!(reject(json!({})), "空 payload 必须 fail-closed");
    assert!(
        reject(json!({ "channel_id": channel(), "message": MESSAGE })),
        "缺 schedule_post_at 必须 fail-closed"
    );
    assert!(
        reject(json!({
            "channel_id": channel(),
            "message": MESSAGE,
            "schedule_post_at": 0
        })),
        "非正到点时间必须 fail-closed"
    );
    assert!(
        reject(json!({
            "channel_id": channel(),
            "message": MESSAGE,
            "schedule_post_at": EXECUTE_AT,
            "unexpected": "x"
        })),
        "未知字段必须 fail-closed"
    );
    // temporaryId 归 Helix：调用方伪造的临时 ID 必须在边界被拒。
    assert!(
        reject(json!({
            "channel_id": channel(),
            "message": MESSAGE,
            "schedule_post_at": EXECUTE_AT,
            "temporary_id": "caller-owned"
        })),
        "调用方注入 temporary_id 必须 fail-closed"
    );

    // 无 revision 的 authority 帧不得推进本地终态（防止 0 值覆盖已提交 revision）。
    let mut harness = creator_harness();
    harness.step(command_json("im_create_schedule", core_payload()), NOW_MS);
    harness.step(
        inbound(&json!({
            "action": "post_schedule_created",
            "data": {
                "channelId": channel(),
                "scheduleId": SCHEDULE_ID,
                "scheduleOwnerUserId": CREATOR_ID
            }
        })),
        NOW_MS + 2,
    );
    assert!(harness.sink.as_slice().is_empty());
}

// ── 11. Gate identity ──────────────────────────────────────────────────────

/// 参考包的 inbound sample 必须仍然指向本 Gate 的权威命令名与到点时间锚。
#[test]
fn inbound_sample_targets_the_create_command() {
    let sample = gate_json("inbound-sample.json");
    let value = &sample["rust_entry_inputs"][0]["value"];
    assert_eq!(value["name"], "im_create_schedule");
    assert_eq!(value["payload"]["channel_id"], SAMPLE_CHANNEL_ID);
    assert_eq!(value["payload"]["schedule_post_at"], json!(EXECUTE_AT));
    assert_eq!(value["payload"]["message"], MESSAGE);
    assert_eq!(value["payload"]["type"], "TEXT");
    assert_eq!(value["payload"]["props"], json!({}));
    assert!(value["payload"].get("execute_at").is_none());
    assert!(value["payload"].get("post").is_none());
    // 定时创建是单视角 Gate：目标包只有 sender 样本，不得偷偷补 receiver。
    let outbound = gate_json("outbound-sample.json");
    let samples = outbound["samples"]
        .as_array()
        .expect("samples must be array");
    assert_eq!(samples.len(), 1);
    assert_eq!(samples[0]["viewer"], "sender");
}
