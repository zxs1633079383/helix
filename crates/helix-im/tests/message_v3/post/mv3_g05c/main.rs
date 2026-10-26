//! MV3-G05c｜对指定未读成员加急（`im_urgent_post`）。
//!
//! 覆盖面（对齐 visual 参考包 `post/mv3-g05c` 与 helix-gate-test-matrix §MV3-G05c）：
//! - `targetIds` 语义：wire 只带 `postId + targetIds`，空/重复/空串/超限一律 fail-closed。
//! - 加急写集边界：只写 message 的 `expedite_map/event_seq/update_at`，不写内容列、
//!   不写任何频道级加急列（会话级标识由 `urgentPostList` 承载，`urgentCount` 是死字段）。
//! - receiver-local 加急投影：同一份绝对 `expediteMap` 在不同 viewer 下派生不同待确认态。
//! - 独立 persona（INV-12）：sender 与 receiver 各自独立回放，绝不互相复制；
//!   **加急发起人自己不得出现待确认/待办**。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use helix_im::commands::handle_outbound;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{channel_id, cid, command_json, port_reply_ok, port_reply_with_body, Harness};

const CHANNEL_INDEX: u64 = 0x5c;
const POST_ID: &str = "srvurg0000000000000000005c";
const ACTOR_ID: &str = "user-actor-444";
/// 加急发起人显示名——会话列表摘要 `${urgentCurrentName}加急了你` 的唯一来源。
const ACTOR_NAME: &str = "用户444";
const RECEIVER_ID: &str = "user-receiver-445";
const BYSTANDER_ID: &str = "user-bystander-446";
const CREATE_AT: i64 = 1_700_000_000_000;
const BASE: &str = "http://x/api/cses";

/// 加急投影允许公开的字段全集；多一个或少一个都是契约破坏。
const PROJECTION_KEYS: [&str; 5] = ["id", "channelId", "eventSeq", "updateAt", "expediteMap"];

/// 会话级加急信号允许公开的字段全集（当前成员投影合同）。
const CHANNEL_SIGNAL_KEYS: [&str; 7] = [
    "channelId",
    "urgentPostList",
    "urgentCurrentName",
    "hasUrgentPost",
    "userId",
    "projectionAuthority",
    "memberScope",
];

/// 加急写集允许触碰的 message 列全集（BatchUpdate patch 顺序即声明顺序）。
const URGENT_PATCH_COLUMNS: [&str; 3] = ["expedite_map", "event_seq", "update_at"];

// ── fixtures ────────────────────────────────────────────────────────────────

/// 构造服务端权威 expediteMap：sender + 每个 recipient 的绝对 status。
fn expedite_map(recipients: &[(&str, i64)]) -> Value {
    expedite_map_from(json!({"id": ACTOR_ID, "name": ACTOR_NAME}), recipients)
}

/// 构造指定 sender 分支的 expediteMap（用于显示名缺失等退化形态）。
fn expedite_map_from(sender: Value, recipients: &[(&str, i64)]) -> Value {
    let mut map = serde_json::Map::new();
    for (id, status) in recipients {
        map.insert(
            (*id).to_string(),
            json!({
                "status": status,
                "confirmAt": if *status > 0 { Some(CREATE_AT + 20) } else { None::<i64> }
            }),
        );
    }
    json!({"sender": sender, "recipients": Value::Object(map)})
}

/// 构造 Go `post_update` 广播的加急 authority 帧。
fn urgent_authority(event_seq: u64, map: &Value) -> Value {
    json!({
        "action": "post_update",
        "data": {
            "id": POST_ID,
            "channelId": cid(CHANNEL_INDEX),
            "userId": ACTOR_ID,
            "createAt": CREATE_AT,
            "updateAt": CREATE_AT + event_seq as i64 * 10,
            "message": "MV3-G05c urgent",
            "type": "TEXT",
            "props": {"channel_event_seq": event_seq},
            "expediteMap": map,
            "event_seq": event_seq
        }
    })
}

/// 构造加急提交后 message 表的真实读回行。
fn durable_row(event_seq: u64, map: &Value) -> Row {
    vec![
        ("id".to_string(), SqlValue::Text(POST_ID.to_string())),
        ("channel_id".to_string(), SqlValue::Text(cid(CHANNEL_INDEX))),
        ("event_seq".to_string(), SqlValue::Integer(event_seq as i64)),
        (
            "update_at".to_string(),
            SqlValue::Integer(CREATE_AT + event_seq as i64 * 10),
        ),
        ("expedite_map".to_string(), SqlValue::Text(map.to_string())),
    ]
}

/// 构造 render-ready 读族行（DB snake 形态），用于 viewer-local 投影断言。
fn render_row(map: &Value) -> Value {
    json!({
        "id": POST_ID,
        "channel_id": cid(CHANNEL_INDEX),
        "user_id": ACTOR_ID,
        "type": "TEXT",
        "message": "MV3-G05c urgent",
        "create_at": CREATE_AT,
        "send_status": "sent",
        "expedite_map": map.to_string()
    })
}

// ── effect 读取 ─────────────────────────────────────────────────────────────

/// 从当前 Effect batch 解出全部 MessageV3 业务事件。
fn emitted_events(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// 按 Effect 类型读取当前唯一 correlation。
fn correlation(effects: &[Effect], kind: &str) -> Option<Correlation> {
    effects.iter().find_map(|effect| match (kind, effect) {
        ("http", Effect::Http { corr, .. }) => Some(*corr),
        ("atomic", Effect::PersistAtomic { corr, .. }) => Some(*corr),
        ("persist", Effect::Persist { corr, .. }) => Some(*corr),
        _ => None,
    })
}

/// 取本批次唯一 `PersistAtomic` 的 ops。
fn persist_atomic_ops(effects: &[Effect]) -> Vec<StorageOp> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops.clone()),
            _ => None,
        })
        .expect("urgent authority must persist atomically")
}

/// 取 message 表 BatchUpdate 的 (key_vals 文本, patch 列名, patch map)。
fn message_patch(ops: &[StorageOp]) -> (Vec<String>, Vec<String>, Value) {
    let spec = ops
        .iter()
        .find_map(|op| match op {
            StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(spec),
            _ => None,
        })
        .expect("urgent write must target the message table");
    assert_eq!(spec.key_col, "id", "urgent write must key on message id");
    let columns: Vec<String> = spec.patch.iter().map(|(name, _)| name.clone()).collect();
    let mut patch = serde_json::Map::new();
    for (name, value) in &spec.patch {
        patch.insert(
            name.clone(),
            match value {
                SqlValue::Text(text) => json!(text),
                SqlValue::Integer(number) => json!(number),
                other => json!(format!("{other:?}")),
            },
        );
    }
    let key_vals = spec
        .key_vals
        .iter()
        .map(|value| match value {
            SqlValue::Text(text) => text.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    (key_vals, columns, Value::Object(patch))
}

/// 加急写集允许的列（顺序敏感），用于精确集合比较。
fn expected_patch_columns() -> Vec<String> {
    URGENT_PATCH_COLUMNS
        .iter()
        .map(|column| (*column).to_string())
        .collect()
}

// ── 回放 ────────────────────────────────────────────────────────────────────

/// 以指定身份创建独立 persona harness（各 persona 状态完全隔离）。
fn persona(viewer_id: &str) -> Harness {
    Harness::new_with_identity(&[CHANNEL_INDEX], viewer_id, "mv3-g05c-company", "", "", "")
}

/// 回放一次 authority → PersistAtomic → durable readback，返回写集与终态事件。
fn apply_authority(
    harness: &mut Harness,
    event_seq: u64,
    wire_map: &Value,
    durable_map: &Value,
    now_ms: u64,
) -> (Vec<StorageOp>, Vec<Value>) {
    let frame = serde_json::to_vec(&urgent_authority(event_seq, wire_map))
        .expect("authority frame must serialize");
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(frame))), now_ms);
    let ops = persist_atomic_ops(harness.sink.as_slice());
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "authority must not emit before the durable commit"
    );
    let persist_corr =
        correlation(harness.sink.as_slice(), "atomic").expect("missing urgent PersistAtomic");

    harness.step(port_reply_ok(persist_corr.raw()), now_ms + 1);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "PersistOk must schedule a durable readback before the terminal emit"
    );
    let readback_corr =
        correlation(harness.sink.as_slice(), "persist").expect("missing urgent readback");

    let row = durable_row(event_seq, durable_map);
    harness.step(
        port_reply_with_body(
            readback_corr.raw(),
            rows_to_reply_bytes(std::slice::from_ref(&row)),
        ),
        now_ms + 2,
    );
    (ops, emitted_events(harness.sink.as_slice()))
}

/// outbound fail-closed 判定。
fn outbound_fails(payload: Value) -> bool {
    handle_outbound(
        "im_urgent_post",
        &serde_json::to_vec(&payload).expect("payload must serialize"),
        BASE,
        BASE,
        None,
        Correlation::from_raw(1),
    )
    .is_err()
}

/// 取唯一 `Effect::Http` 的 (url, body)。
fn single_http(effects: Vec<Effect>) -> (String, Value) {
    assert_eq!(
        effects.len(),
        1,
        "outbound urgent must emit exactly one Http"
    );
    match effects.into_iter().next().expect("one effect") {
        Effect::Http { req, .. } => {
            let body =
                serde_json::from_slice(&req.body.expect("urgent Http must carry a body")).unwrap();
            (req.url, body)
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

// ── 测试 ────────────────────────────────────────────────────────────────────

/// 证明 MV3-G05c 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G05c", "post/mv3-g05c");
}

/// wire 只承载 `postId + targetIds`：顺序保真，`channel_id` 不得外发（Go 由 postId 权威解析频道）。
#[test]
fn urgent_post_wire_carries_only_post_id_and_ordered_target_ids() {
    let effects = handle_outbound(
        "im_urgent_post",
        &serde_json::to_vec(&json!({
            "post_id": POST_ID,
            "target_ids": [RECEIVER_ID, BYSTANDER_ID],
            "channel_id": cid(CHANNEL_INDEX),
            "req_id": "mv3-g05c-request"
        }))
        .unwrap(),
        BASE,
        BASE,
        Some("conn-1"),
        Correlation::from_raw(1),
    )
    .expect("im_urgent_post must build");
    let (url, body) = single_http(effects);
    assert_eq!(url, format!("{BASE}/posts/urgentPost"));
    assert_eq!(
        body,
        json!({"postId": POST_ID, "targetIds": [RECEIVER_ID, BYSTANDER_ID]}),
        "urgent wire must be exactly postId + targetIds (no channelId, no urgentCount)"
    );
}

/// targetIds 语义 fail-closed：空数组 / 空串 / 重复 / 非字符串 / 超限 / 非 canonical key 一律拒绝。
#[test]
fn urgent_post_target_ids_are_fail_closed() {
    assert!(outbound_fails(
        json!({"post_id": POST_ID, "target_ids": []})
    ));
    assert!(outbound_fails(
        json!({"post_id": POST_ID, "target_ids": [""]})
    ));
    assert!(outbound_fails(
        json!({"post_id": POST_ID, "target_ids": [RECEIVER_ID, RECEIVER_ID]})
    ));
    assert!(outbound_fails(
        json!({"post_id": POST_ID, "target_ids": [42]})
    ));
    assert!(outbound_fails(
        json!({"post_id": POST_ID, "target_ids": RECEIVER_ID})
    ));
    assert!(outbound_fails(json!({"target_ids": [RECEIVER_ID]})));
    assert!(
        outbound_fails(json!({"post_id": POST_ID, "targetIds": [RECEIVER_ID]})),
        "camelCase targetIds is not a canonical inbound key"
    );
    assert!(outbound_fails(
        json!({"post_id": POST_ID, "target_ids": [RECEIVER_ID], "req_id": ""})
    ));

    let at_limit: Vec<String> = (0..100).map(|index| format!("user-{index:03}")).collect();
    let over_limit: Vec<String> = (0..101).map(|index| format!("user-{index:03}")).collect();
    assert!(
        !outbound_fails(json!({"post_id": POST_ID, "target_ids": at_limit})),
        "100 targets is the accepted upper bound"
    );
    assert!(outbound_fails(
        json!({"post_id": POST_ID, "target_ids": over_limit})
    ));
}

/// HTTP 2xx 只是 admission：命令与 ack 阶段都不得产生任何 MessageV3 终态（INV-03）。
#[test]
fn urgent_post_http_ack_is_admission_only() {
    let mut harness = persona(ACTOR_ID);
    harness.step(
        command_json(
            "im_urgent_post",
            json!({"post_id": POST_ID, "target_ids": [RECEIVER_ID]}),
        ),
        CREATE_AT as u64,
    );
    assert_eq!(
        harness.last_http_body(),
        Some(json!({"postId": POST_ID, "targetIds": [RECEIVER_ID]}))
    );
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "command must not settle urgent state locally"
    );
    let http_corr = correlation(harness.sink.as_slice(), "http").expect("missing urgent Http");

    harness.step(port_reply_ok(http_corr.raw()), CREATE_AT as u64 + 1);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "HTTP ack is admission only; only the WS authority may settle urgent state"
    );
}

/// 加急写集只覆盖 message 权威加急列 + cursor，绝不触碰内容列或任何频道级加急列。
#[test]
fn urgent_write_set_is_message_scoped_and_never_writes_dead_urgent_count() {
    let map = expedite_map(&[(RECEIVER_ID, 0)]);
    let mut harness = persona(ACTOR_ID);
    let (ops, _) = apply_authority(&mut harness, 1, &map, &map, CREATE_AT as u64);

    let (key_vals, columns, patch) = message_patch(&ops);
    assert_eq!(key_vals, vec![POST_ID.to_string()]);
    assert_eq!(
        columns,
        expected_patch_columns(),
        "urgent write set must be exactly expedite_map + event_seq + update_at"
    );
    assert_eq!(patch["event_seq"], json!(1));
    assert_eq!(patch["update_at"], json!(CREATE_AT + 10));
    assert_eq!(
        serde_json::from_str::<Value>(patch["expedite_map"].as_str().unwrap()).unwrap(),
        map,
        "durable expedite_map must be the untouched server absolute map"
    );

    // 会话级加急标识的权威字段是 urgentPostList；urgentCount 是死字段，任何写集都不得出现。
    for forbidden in [
        "urgent_count",
        "urgent_post_list",
        "has_urgent_post",
        "urgent_current_name",
        "message",
        "props",
        "read_bits",
        "send_status",
        "quick_reply",
        "unread_count",
    ] {
        assert!(
            !columns.iter().any(|column| column == forbidden),
            "urgent message write must not touch {forbidden}"
        );
    }
    assert!(ops.iter().any(|op| matches!(
        op,
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor"
    )));
}

/// `message_v3_urgent_op` 是纯 message 级写：key/patch/溢出钳制都必须稳定。
#[test]
fn urgent_op_keys_the_message_row_and_clamps_event_seq() {
    let op = helix_im::channel_write::message_v3_urgent_op(
        POST_ID,
        expedite_map(&[(RECEIVER_ID, 0)]).to_string(),
        u64::MAX,
        CREATE_AT,
    );
    let (key_vals, columns, patch) = message_patch(std::slice::from_ref(&op));
    assert_eq!(key_vals, vec![POST_ID.to_string()]);
    assert_eq!(columns, expected_patch_columns());
    assert_eq!(
        patch["event_seq"],
        json!(i64::MAX),
        "u64 event_seq must clamp instead of wrapping negative"
    );
    assert_eq!(patch["update_at"], json!(CREATE_AT));
}

/// 终态投影保持绝对消息态，频道信号按当前成员投影（INV-12）。
#[test]
fn urgent_projection_keeps_absolute_post_and_member_scoped_channel_signal() {
    let map = expedite_map(&[(RECEIVER_ID, 0), (BYSTANDER_ID, 0)]);

    let mut sender = persona(ACTOR_ID);
    let (_, sender_events) = apply_authority(&mut sender, 1, &map, &map, CREATE_AT as u64);
    let mut receiver = persona(RECEIVER_ID);
    let (_, receiver_events) = apply_authority(&mut receiver, 1, &map, &map, CREATE_AT as u64);

    // sequence 1 是两个 persona 共有的绝对消息投影；sequence 2 只反映当前 viewer 的会话徽标。
    assert_eq!(sender_events.len(), 2);
    assert_eq!(sender_events[0], receiver_events[0]);
    let event = &sender_events[0];
    assert_eq!(event["event"], "im:post:updated");
    let mut keys: Vec<&str> = event["data"]
        .as_object()
        .expect("data must be an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = PROJECTION_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected, "投影字段集合必须精确匹配契约");
    assert_eq!(event["data"]["id"], POST_ID);
    assert_eq!(event["data"]["channelId"], cid(CHANNEL_INDEX));
    assert_eq!(event["data"]["expediteMap"], map);
    assert_eq!(event["data"]["eventSeq"], json!(1));
    assert_eq!(event["data"]["updateAt"], json!(CREATE_AT + 10));

    assert_eq!(sender_events[1]["data"]["userId"], ACTOR_ID);
    assert_eq!(sender_events[1]["data"]["hasUrgentPost"], json!(false));
    assert_eq!(sender_events[1]["data"]["urgentPostList"], json!([]));
    assert_eq!(receiver_events[1]["data"]["userId"], RECEIVER_ID);
    assert_eq!(receiver_events[1]["data"]["hasUrgentPost"], json!(true));
    assert_eq!(
        receiver_events[1]["data"]["urgentPostList"],
        json!([POST_ID])
    );

    let encoded = serde_json::to_string(event).unwrap();
    for forbidden in [
        "urgentCount",
        "canConfirmUrgent",
        "urgentState",
        "urgentTargetIds",
        "todo",
        "props",
        "readBits",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "绝对投影不得携带 viewer-local 字段 {forbidden}"
        );
    }
}

/// 终态只认 durable read-back：wire 与本地行不一致时，投影必须跟随本地行而非复制 wire。
#[test]
fn urgent_projection_follows_the_durable_row_not_the_wire_frame() {
    let wire = expedite_map(&[(RECEIVER_ID, 0)]);
    let durable = expedite_map(&[(RECEIVER_ID, 0), (BYSTANDER_ID, 0)]);
    let mut harness = persona(RECEIVER_ID);
    let (_, events) = apply_authority(&mut harness, 1, &wire, &durable, CREATE_AT as u64);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["data"]["expediteMap"], durable);
    assert_ne!(events[0]["data"]["expediteMap"], wire);
}

/// durable 行残缺或类型不符时 fail-closed：不得半构造出加急终态。
#[test]
fn urgent_projection_rejects_malformed_durable_rows() {
    let map = expedite_map(&[(RECEIVER_ID, 0)]);
    let complete = durable_row(3, &map);
    assert!(
        helix_im::event::post::urgent_update_from_row(&complete)
            .unwrap()
            .is_some(),
        "完整行必须能重建终态"
    );

    for missing in ["id", "channel_id", "event_seq", "update_at", "expedite_map"] {
        let partial: Row = complete
            .iter()
            .filter(|(name, _)| name != missing)
            .cloned()
            .collect();
        assert!(
            helix_im::event::post::urgent_update_from_row(&partial)
                .unwrap()
                .is_none(),
            "缺列 {missing} 必须 fail-closed"
        );
    }

    let mut string_seq = complete.clone();
    for entry in string_seq.iter_mut() {
        if entry.0 == "event_seq" {
            entry.1 = SqlValue::Text("3".to_string());
        }
    }
    assert!(
        helix_im::event::post::urgent_update_from_row(&string_seq)
            .unwrap()
            .is_none(),
        "event_seq 不接受字符串数字别名"
    );

    for bogus in ["[]", "null", "\"urgent\"", "{"] {
        let mut broken = complete.clone();
        for entry in broken.iter_mut() {
            if entry.0 == "expedite_map" {
                entry.1 = SqlValue::Text(bogus.to_string());
            }
        }
        assert!(
            helix_im::event::post::urgent_update_from_row(&broken)
                .unwrap()
                .is_none(),
            "非 object expedite_map({bogus}) 必须 fail-closed"
        );
    }
}

/// INV-12：同一份绝对 map 在三个 persona 下各自独立派生，发起人自己没有待确认/待办。
#[test]
fn urgent_view_is_derived_per_persona_and_sender_is_never_pending() {
    let map = expedite_map(&[(RECEIVER_ID, 0), (BYSTANDER_ID, 0)]);
    let rows = json!([render_row(&map)]);

    let view = |viewer: &str| -> Value {
        helix_im::render_ready::shape_message_rows_for_viewer(&rows, viewer)[0].clone()
    };
    let sender_view = view(ACTOR_ID);
    let receiver_view = view(RECEIVER_ID);
    let outsider_view = view("user-outsider-999");

    // 绝对面：三个视角读到同一份 target/confirmed 集合。
    for candidate in [&sender_view, &receiver_view, &outsider_view] {
        assert_eq!(candidate["urgentRequesterId"], ACTOR_ID);
        assert_eq!(candidate["urgentRequiredCount"], json!(2));
        assert_eq!(candidate["urgentConfirmedIds"], json!([]));
        assert_eq!(candidate["urgentState"], "active");
        let mut targets: Vec<&str> = candidate["urgentTargetIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect();
        targets.sort_unstable();
        assert_eq!(targets, vec![BYSTANDER_ID, RECEIVER_ID]);
    }

    // viewer-local 面：只有被点名且未确认的人有待确认动作。
    assert_eq!(
        sender_view["canConfirmUrgent"],
        json!(false),
        "加急发起人自己不得出现待确认/待办（INV-12）"
    );
    assert_eq!(receiver_view["canConfirmUrgent"], json!(true));
    assert_eq!(
        outsider_view["canConfirmUrgent"],
        json!(false),
        "未被点名的成员不得被加急待办命中"
    );

    // 未确认的被点名者不得出现在 confirmed 集合里（防止 sender/receiver 状态互抄）。
    assert!(!receiver_view["urgentConfirmedIds"]
        .as_array()
        .unwrap()
        .iter()
        .any(|id| id == RECEIVER_ID));
}

/// INV-12：加急发起人即使同时被写进 `recipients`，也不得出现待确认动作。
///
/// 这条独立守护 `render_ready::urgent` 的 `requester_id != viewer` 过滤——
/// 去掉该过滤后本用例必须变红（否则该分支处于无测试裸奔状态）。
#[test]
fn urgent_requester_listed_as_recipient_still_has_no_pending_action() {
    let map = expedite_map(&[(ACTOR_ID, 0), (RECEIVER_ID, 0)]);
    let rows = json!([render_row(&map)]);
    let view = |viewer: &str| -> Value {
        helix_im::render_ready::shape_message_rows_for_viewer(&rows, viewer)[0].clone()
    };

    let requester_view = view(ACTOR_ID);
    // 绝对面如实保留：发起人确实在服务端 target 集合里。
    assert!(
        requester_view["urgentTargetIds"]
            .as_array()
            .unwrap()
            .iter()
            .any(|id| id == ACTOR_ID),
        "绝对 target 集合必须原样反映服务端 recipients"
    );
    assert_eq!(
        requester_view["urgentState"], "active",
        "仍有未确认成员时加急态保持 active"
    );
    // viewer-local 面：发起人永不被自己的加急催办（INV-12 禁止 sender/receiver 状态互抄）。
    assert_eq!(
        requester_view["canConfirmUrgent"],
        json!(false),
        "加急发起人即使在 recipients 里也不得出现待确认动作（INV-12）"
    );
    assert_eq!(
        view(RECEIVER_ID)["canConfirmUrgent"],
        json!(true),
        "普通被点名成员的待确认动作不受发起人过滤影响"
    );
}

/// INV-12：`expediteMap.sender` 分支不是加急目标——发起人不得在自己的加急上被判为「被加急」。
///
/// 会话级 `urgent_post_list` 与 dialog `message_class` 都直接读 `urgent_hit`，
/// 一旦发起人自命中，发起人自己的会话就会亮加急闪电、并进入 about-me 待办族。
#[test]
fn urgent_targets_exclude_the_sender_and_expedite_map_structure_keys() {
    let data = json!({
        "id": POST_ID,
        "channelId": cid(CHANNEL_INDEX),
        "userId": ACTOR_ID,
        "type": "TEXT",
        "message": "MV3-G05c urgent",
        "createAt": CREATE_AT,
        "viewers": ["all"],
        "expediteMap": expedite_map(&[(RECEIVER_ID, 0)]),
    });
    let update = |viewer: &str| {
        helix_im::channel_write::post_updates(channel_id(CHANNEL_INDEX), &data, viewer, CREATE_AT)
    };

    let sender = update(ACTOR_ID);
    assert_eq!(
        sender.urgent_user_ids,
        vec![RECEIVER_ID.to_string()],
        "加急目标集合只能是 recipients 的键；sender / id / name / status / confirmAt 都是结构键"
    );
    assert!(
        !sender.urgent_hit,
        "加急发起人不得被自己发起的加急命中（INV-12：禁止把 sender 复制进 receiver 侧状态）"
    );

    assert!(update(RECEIVER_ID).urgent_hit, "被点名的成员必须命中加急");
    assert!(
        !update(BYSTANDER_ID).urgent_hit,
        "未被点名的成员不得命中加急"
    );
}

/// 缺陷 13：加急终态必须再出一条会话级 `im:channel:update`（参考包 outbound sequence 2）。
#[test]
fn urgent_authority_emits_channel_urgent_signal_as_sequence_two() {
    let map = expedite_map(&[(RECEIVER_ID, 0)]);

    let mut sender = persona(ACTOR_ID);
    let (_, sender_events) = apply_authority(&mut sender, 1, &map, &map, CREATE_AT as u64);
    let mut receiver = persona(RECEIVER_ID);
    let (_, receiver_events) = apply_authority(&mut receiver, 1, &map, &map, CREATE_AT as u64);

    assert_eq!(
        sender_events
            .iter()
            .map(|event| event["event"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["im:post:updated", "im:channel:update"],
        "加急终态出站序列必须是 post 绝对投影 → 会话级加急信号"
    );
    // 消息本体是绝对态；频道 signal 按当前成员隔离，不能要求两个 persona byte-exact。
    assert_eq!(sender_events[0], receiver_events[0]);

    let channel = &sender_events[1]["data"];
    let mut keys: Vec<&str> = channel
        .as_object()
        .expect("channel data must be an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = CHANNEL_SIGNAL_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected, "会话级加急信号字段集合必须精确匹配参考包");
    assert_eq!(channel["channelId"], cid(CHANNEL_INDEX));
    assert_eq!(channel["urgentPostList"], json!([]));
    assert_eq!(channel["urgentCurrentName"], "");
    assert_eq!(channel["hasUrgentPost"], json!(false));
    assert_eq!(channel["userId"], ACTOR_ID);
    assert_eq!(receiver_events[1]["data"]["userId"], RECEIVER_ID);
    assert_eq!(receiver_events[1]["data"]["hasUrgentPost"], json!(true));
    assert_eq!(
        receiver_events[1]["data"]["urgentPostList"],
        json!([POST_ID])
    );
    assert_eq!(receiver_events[1]["data"]["urgentCurrentName"], ACTOR_NAME);
    assert!(
        !serde_json::to_string(&sender_events[1])
            .unwrap()
            .contains("urgentCount"),
        "urgentCount 是死字段，会话列表只读 urgentPostList，不得再造第二个源"
    );
}

/// 发起人显示名缺失时会话级信号仍成立：只降级 `urgentCurrentName`，不 fail 掉闪电。
#[test]
fn urgent_channel_signal_degrades_name_but_keeps_the_lightning() {
    let map = expedite_map_from(json!({"id": ACTOR_ID}), &[(RECEIVER_ID, 0)]);
    let mut harness = persona(RECEIVER_ID);
    let (_, events) = apply_authority(&mut harness, 1, &map, &map, CREATE_AT as u64);

    assert_eq!(events[1]["event"], "im:channel:update");
    assert_eq!(events[1]["data"]["hasUrgentPost"], json!(true));
    assert_eq!(events[1]["data"]["urgentPostList"], json!([POST_ID]));
    assert_eq!(
        events[1]["data"]["urgentCurrentName"], "",
        "显示名不可得时降级为空串，绝不回落成 userId"
    );
}

/// 同一 eventSeq 重复到达必须完全幂等（不重复写、不重复 emit）。
#[test]
fn duplicate_urgent_authority_is_idempotent() {
    let map = expedite_map(&[(RECEIVER_ID, 0)]);
    let mut harness = persona(RECEIVER_ID);
    let (_, events) = apply_authority(&mut harness, 1, &map, &map, CREATE_AT as u64);
    assert_eq!(events.len(), 2);

    let replay = serde_json::to_vec(&urgent_authority(1, &map)).unwrap();
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(replay))),
        CREATE_AT as u64 + 10,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "same eventSeq replay must produce no effect at all"
    );
}
