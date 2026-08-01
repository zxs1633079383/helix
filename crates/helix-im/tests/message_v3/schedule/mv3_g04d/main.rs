// 回归锚: specs/004-messagev3-helix-phase1-send-domain T107，Gate MV3-G04d 查询定时消息。
//
// 合同（gates/schedule/mv3-g04d/）：
// - `effects-contract.json.allowed_effect_tags = ["Http"]` —— 纯读，**零 Persist / 零 Timer**。
// - `outbound-contract.json.channels = []` —— **零事件**，只回结构化 `ScheduleProjection[]`。
// - 查询结果按 `executeAt` **稳定排序**；无待发对象返回空集合而不是抛错。

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::client_api::{build_command, command_alias};
use helix_im::outbound::posts::read::schedule_projections;
use helix_im::outbound::{handle_outbound, is_outbound, is_read};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, enveloped, port_reply_with_body, Harness};

const CHANNEL_ID: &str = "channel-phase1-001";
const EXECUTE_AT: i64 = 1_700_003_600_000;
const NOW_MS: u64 = 1_700_000_000_000;

// ── fixtures ────────────────────────────────────────────────────────────────

fn gate_json(file: &str) -> Value {
    let gate_root = phase1_reference::reference_root().join("schedule/mv3-g04d");
    phase1_reference::read_json(&gate_root.join(file))
}

/// 服务端 `posts/getSchedule` 的单条待发对象（Go 的 camelCase 形状）。
fn server_item(schedule_id: &str, execute_at: i64) -> Value {
    json!({
        "id": schedule_id,
        "channelId": CHANNEL_ID,
        "schedulePostAt": execute_at,
        "status": "scheduled",
        "type": "TEXT",
        "message": "定时发送消息",
        "viewers": ["all"]
    })
}

fn http_url_and_body(effects: &[Effect]) -> (String, Value) {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } => Some((
                req.url.clone(),
                req.body
                    .as_ref()
                    .and_then(|body| serde_json::from_slice(body.as_ref()).ok())
                    .unwrap_or(Value::Null),
            )),
            _ => None,
        })
        .expect("MV3-G04d 必须发出一条 Http")
}

fn build_get_schedule(payload: &Value, corr: u64) -> Result<Vec<Effect>, helix_im::ImError> {
    let bytes = serde_json::to_vec(payload).expect("payload must serialize");
    handle_outbound(
        "im_get_schedule",
        &bytes,
        "http://test/api/cses",
        "http://test",
        None,
        Correlation::from_raw(corr),
    )
}

// ── 1. 参考包完整性 ─────────────────────────────────────────────────────────

/// 证明 MV3-G04d 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G04d", "schedule/mv3-g04d");
}

// ── 2. inbound sample → 真实 wire request ───────────────────────────────────

/// 参考包 `inbound-sample.json` 的 `Tick::Command` 必须驱动出现网真实 endpoint 与 body。
#[test]
fn inbound_sample_builds_the_authoritative_wire_request() {
    let sample = gate_json("inbound-sample.json");
    let input = sample["rust_entry_inputs"][0]["value"].clone();
    assert_eq!(input["name"], "im_get_schedule");
    assert!(is_outbound("im_get_schedule"));

    let effects = build_get_schedule(&input["payload"], 1).expect("reference payload must build");
    let (url, body) = http_url_and_body(&effects);
    assert!(
        url.ends_with("posts/getSchedule"),
        "endpoint 必须是 posts/getSchedule，实际 {url}"
    );
    assert_eq!(body, json!({ "channelId": CHANNEL_ID }));
}

// ── 3. Effect 集合：Http only，零事件 ───────────────────────────────────────

/// 走完整模块 dispatch：恰好一条 Http，零 Persist / 零 Emit / 零 Timer。
#[test]
fn effect_set_is_http_only_with_zero_events() {
    let mut harness = Harness::new(&[]);
    harness.step(
        command_json(
            "im_get_schedule",
            json!({ "channel_id": CHANNEL_ID, "req_id": "mv3-g04d-1" }),
        ),
        NOW_MS,
    );
    let effects = harness.sink.as_slice();
    assert_eq!(effects.len(), 1, "MV3-G04d 只允许一条 Http");
    assert!(matches!(effects[0], Effect::Http { .. }));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::Emit { .. }
            | Effect::Persist { .. }
            | Effect::PersistAtomic { .. }
            | Effect::ScheduleTimer { .. }
    )));
    // 读族：HTTP 响应体即数据，必须注册回灌通道，否则结果被 fire-and-forget 丢掉。
    assert!(is_read("im_get_schedule"));
}

/// 真实 Command -> HTTP -> PortReply 链必须回灌稳定排序后的结构化结果。
#[test]
fn http_reply_projects_structured_result_through_real_dispatch() {
    let mut harness = Harness::new(&[]);
    harness.step(
        command_json(
            "im_get_schedule",
            json!({ "channel_id": CHANNEL_ID, "req_id": "mv3-g04d-result-1" }),
        ),
        NOW_MS,
    );
    let corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("schedule query must register one HTTP correlation");
    let raw = serde_json::to_vec(&json!({
        "data": [
            server_item("s-late", EXECUTE_AT + 1),
            server_item("s-early", EXECUTE_AT)
        ]
    }))
    .expect("server result must serialize");
    harness.step(
        port_reply_with_body(corr, enveloped(raw.into())),
        NOW_MS + 1,
    );
    let result = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<Value>(event.0.as_ref()).ok(),
            _ => None,
        })
        .expect("schedule HTTP reply must produce a structured result");
    assert_eq!(result["event"], "im:read:result");
    assert_eq!(result["data"]["req_id"], "mv3-g04d-result-1");
    assert_eq!(result["data"]["body"]["ok"], true);
    assert_eq!(
        result["data"]["body"]["schedules"][0]["scheduleId"],
        "s-early"
    );
    assert_eq!(
        result["data"]["body"]["schedules"][1]["scheduleId"],
        "s-late"
    );
}

// ── 4. 旧 user_id 不得进入 wire ────────────────────────────────────────────

/// Angular Port 允许带 `userId`（per-user 待发对象），但 owner 由服务端 session 派生。
#[test]
fn legacy_user_id_never_reaches_the_wire() {
    let effects = build_get_schedule(
        &json!({ "channel_id": CHANNEL_ID, "user_id": "user-actor-444" }),
        2,
    )
    .expect("payload with legacy user_id must still build");
    let (_, body) = http_url_and_body(&effects);
    assert_eq!(body, json!({ "channelId": CHANNEL_ID }));
}

// ── 5. 结果投影匹配参考样本 ─────────────────────────────────────────────────

/// `outbound-sample.json` 的 `ScheduleProjection` 字段必须逐一由真实投影产出。
#[test]
fn projection_matches_the_outbound_reference_sample() {
    let sample = gate_json("outbound-sample.json");
    let expected = sample["command_results"][0]["result"][0].clone();

    let body = json!({ "data": [server_item("schedule-001", EXECUTE_AT)] });
    let projected = schedule_projections(CHANNEL_ID, &body);
    let actual = projected[0].clone();

    assert_eq!(actual["scheduleId"], expected["scheduleId"]);
    assert_eq!(actual["channelId"], expected["channelId"]);
    assert_eq!(actual["executeAt"], expected["executeAt"]);
    assert_eq!(actual["status"], expected["status"]);
    assert_eq!(actual["post"]["type"], expected["post"]["type"]);
    assert_eq!(actual["post"]["text"], expected["post"]["text"]);
    assert_eq!(actual["post"]["viewers"], expected["post"]["viewers"]);
    assert_eq!(actual["post"]["channelId"], expected["channelId"]);
    assert!(actual.get("id").is_none(), "公开结果不得继续双写 legacy id");
    assert!(
        actual["post"].get("message").is_none(),
        "公开结果不得继续双写 legacy message"
    );
}

// ── 6. 稳定排序 ────────────────────────────────────────────────────────────

/// 服务端顺序不稳定时，投影仍按 `executeAt` 升序、同刻按 id 升序输出。
#[test]
fn projection_is_stable_ordered_by_execute_at() {
    let body = json!({
        "data": [
            server_item("s-late", EXECUTE_AT + 60_000),
            server_item("s-b", EXECUTE_AT),
            server_item("s-early", EXECUTE_AT - 60_000),
            server_item("s-a", EXECUTE_AT)
        ]
    });
    let projected = schedule_projections(CHANNEL_ID, &body);
    let ids: Vec<&str> = projected
        .as_array()
        .expect("projection must be an array")
        .iter()
        .map(|item| item["scheduleId"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(ids, vec!["s-early", "s-a", "s-b", "s-late"]);

    // 打乱输入顺序不改变输出（稳定排序 = 同一份数据永远同一个数组）。
    let shuffled = json!({
        "data": [
            server_item("s-a", EXECUTE_AT),
            server_item("s-early", EXECUTE_AT - 60_000),
            server_item("s-late", EXECUTE_AT + 60_000),
            server_item("s-b", EXECUTE_AT)
        ]
    });
    assert_eq!(projected, schedule_projections(CHANNEL_ID, &shuffled));
}

// ── 7. 幂等 ────────────────────────────────────────────────────────────────

/// 同一响应体重复到达产出完全相同的绝对结果（Angular 内存与 DOM 不变）。
#[test]
fn repeated_projection_is_idempotent() {
    let body = json!({ "data": [server_item("schedule-001", EXECUTE_AT)] });
    assert_eq!(
        schedule_projections(CHANNEL_ID, &body),
        schedule_projections(CHANNEL_ID, &body)
    );
}

// ── 8. 无待发对象 graceful ──────────────────────────────────────────────────

/// 无待发定时消息必须返回空集合而不是错误——否则切会话流程被读取失败拖垮。
#[test]
fn missing_schedule_returns_empty_array_not_error() {
    for body in [
        json!(null),
        json!({}),
        json!([]),
        json!({ "data": null }),
        json!({ "data": [] }),
        json!({ "status": "SUCCESS", "data": {} }),
    ] {
        assert_eq!(
            schedule_projections(CHANNEL_ID, &body),
            json!([]),
            "{body} 必须 graceful 返回空集合"
        );
    }
}

// ── 9. 脏数据 fail-soft ────────────────────────────────────────────────────

/// 缺 `executeAt` 锚点的条目不进结果（否则排序不稳定、提示条显示错误时间）。
#[test]
fn items_without_an_execute_at_anchor_are_dropped() {
    let body = json!({
        "data": [
            { "id": "broken", "channelId": CHANNEL_ID, "status": "scheduled" },
            "not-an-object",
            server_item("schedule-001", EXECUTE_AT)
        ]
    });
    let projected = schedule_projections(CHANNEL_ID, &body);
    assert_eq!(projected.as_array().map(Vec::len), Some(1));
    assert_eq!(projected[0]["scheduleId"], "schedule-001");
}

// ── 10. 频道作用域兜底 ──────────────────────────────────────────────────────

/// 服务端条目省略 `channelId` 时回落到请求作用域，绝不产出空 channelId 的投影。
#[test]
fn channel_id_falls_back_to_the_requested_scope() {
    let body = json!({
        "data": [{
            "id": "schedule-002",
            "sendAt": EXECUTE_AT,
            "message": "无 channelId 的服务端条目"
        }]
    });
    let projected = schedule_projections(CHANNEL_ID, &body);
    assert_eq!(projected[0]["channelId"], CHANNEL_ID);
    assert_eq!(projected[0]["post"]["channelId"], CHANNEL_ID);
    // status 缺省必须是 scheduled（否则提示条判定不了是否还待发）。
    assert_eq!(projected[0]["status"], "scheduled");
    assert_eq!(projected[0]["post"]["viewers"], json!(["all"]));
}

// ── 11. 待发对象正文原样读回 ────────────────────────────────────────────────

/// MV3-G04a 存下的整条待发对象（message / type / props / schedulePostAt）必须原样读回。
///
/// `props` 是 MessageV3 与 Go 共享的 opaque 业务 JSON（@人、引用、模板 ID 都在里面）：
/// 投影若把它丢掉或压平，提示条重建出来的待发消息就和当初设定的不是同一条。
#[test]
fn stored_schedule_payload_round_trips_verbatim() {
    let props = json!({
        "mentions": ["user-receiver-445"],
        "template": { "id": "tpl-7", "fields": { "title": "周会" } }
    });
    let body = json!({
        "data": [{
            "id": "schedule-props",
            "channelId": CHANNEL_ID,
            "schedulePostAt": EXECUTE_AT,
            "status": "scheduled",
            "type": "TEMPLATE",
            "message": "周会提醒",
            "viewers": ["user-receiver-445"],
            "props": props
        }]
    });
    let projected = schedule_projections(CHANNEL_ID, &body);
    assert_eq!(projected[0]["post"]["props"], props);
    assert_eq!(projected[0]["post"]["type"], "TEMPLATE");
    assert_eq!(projected[0]["post"]["text"], "周会提醒");
    assert_eq!(
        projected[0]["post"]["viewers"],
        json!(["user-receiver-445"])
    );
    assert_eq!(projected[0]["executeAt"], EXECUTE_AT);
}

// ── 12. 响应外壳形态 ────────────────────────────────────────────────────────

/// 双层信封 `{status,data:{schedules:[…]}}` 必须被拆开，而不是当成一条脏对象丢掉。
///
/// 丢掉的表现是「定时设上了、切回会话提示条却永远不出现」——用户以为没设上而重复设定，
/// 到点多发（contract.json `openDecisions.impact`）。
#[test]
fn nested_envelope_payload_is_unwrapped() {
    let body = json!({
        "status": "SUCCESS",
        "data": { "schedules": [server_item("schedule-001", EXECUTE_AT)] }
    });
    let projected = schedule_projections(CHANNEL_ID, &body);
    assert_eq!(projected.as_array().map(Vec::len), Some(1));
    assert_eq!(projected[0]["scheduleId"], "schedule-001");
    assert_eq!(projected[0]["executeAt"], EXECUTE_AT);

    // 内层列表键只认数组：`{data:{schedules:{…}}}` 不得被误当列表拆开。
    let not_a_list = json!({ "data": { "schedules": { "id": "x" } } });
    assert_eq!(schedule_projections(CHANNEL_ID, &not_a_list), json!([]));
}

/// 单对象外壳（服务端只返回一条、不套数组）必须产出一条投影而不是空集合。
#[test]
fn single_object_payload_yields_one_projection() {
    let body = json!({ "data": server_item("schedule-001", EXECUTE_AT) });
    let projected = schedule_projections(CHANNEL_ID, &body);
    assert_eq!(projected.as_array().map(Vec::len), Some(1));
    assert_eq!(projected[0]["scheduleId"], "schedule-001");
}

// ── 13. 公开边界注册 ────────────────────────────────────────────────────────

/// 命令必须在 `client_api` 公开边界登记，且 camelCase 入参归一为内核 snake_case。
#[test]
fn command_is_registered_on_the_public_boundary() {
    assert_eq!(command_alias("im_get_schedule"), Some("im_get_schedule"));
    let payload = serde_json::to_vec(&json!({ "channelId": CHANNEL_ID, "userId": "u-1" }))
        .expect("payload must serialize");
    let command = build_command("im_get_schedule", &payload).expect("public command must build");
    assert_eq!(command.name.as_ref(), "im_get_schedule");
    let normalized: Value =
        serde_json::from_slice(command.payload.as_ref()).expect("normalized payload must be JSON");
    assert_eq!(normalized["channel_id"], CHANNEL_ID);
    assert_eq!(normalized["user_id"], "u-1");
}

// ── 14. 缺必填入参 fail-closed ──────────────────────────────────────────────

/// 缺 / 空 `channel_id` 必须在产生任何 Effect 之前拒绝。
#[test]
fn missing_channel_id_fails_closed() {
    for payload in [json!({}), json!({ "channel_id": "" })] {
        assert!(build_get_schedule(&payload, 3).is_err());
    }
}
