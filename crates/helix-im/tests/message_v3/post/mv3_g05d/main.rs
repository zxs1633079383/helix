//! MV3-G05d｜被加急者点「已读」（`im_urgent_confirm`）。
//!
//! 覆盖面（对齐 visual 参考包 `post/mv3-g05d` 与 helix-gate-test-matrix §MV3-G05d）：
//! - wire 只带 `postId`（Go 由 postId 权威解析频道与确认人），未知字段 fail-closed。
//! - **只更新当前用户的 `status=1`**，其余成员条目逐项不变——本 Gate 最易写漏的一条，
//!   由 `confirm_only_flips_the_confirming_member_and_leaves_peers_untouched` 独立守护。
//! - 重复确认幂等：同 seq 重放零 Effect；再次确认权威收敛到同一 `status=1`，peers 仍不变。
//! - 独立 persona（INV-12）：确认人与加急发起人各自独立回放；确认后的待确认动作分视角派生，
//!   绝不在两侧之间复制。
//!
//! 本 Gate 的 sender 视角 = 执行确认的被加急者，receiver 视角 = 加急发起人。

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

use common::{cid, command_json, port_reply_ok, port_reply_with_body, Harness};

const CHANNEL_INDEX: u64 = 0x5d;
const POST_ID: &str = "srvurg0000000000000000005d";
/// 加急发起人（本 Gate 的 receiver 视角）。
const REQUESTER_ID: &str = "user-actor-444";
/// 执行确认的被加急者（本 Gate 的 sender 视角）。
const CONFIRMER_ID: &str = "user-receiver-445";
/// 同批被加急但尚未确认的成员——「其他成员状态不变」的观测点。
const PEER_ID: &str = "user-peer-446";
/// 更早已确认的成员——确认不得把既有 status 回退。
const SETTLED_ID: &str = "user-settled-447";
const CREATE_AT: i64 = 1_700_000_000_000;
const BASE: &str = "http://x/api/cses";

/// 确认写集允许触碰的 message 列全集。
const URGENT_PATCH_COLUMNS: [&str; 3] = ["expedite_map", "event_seq", "update_at"];

/// 确认投影允许公开的字段全集；多一个或少一个都是契约破坏。
const PROJECTION_KEYS: [&str; 5] = ["id", "channelId", "eventSeq", "updateAt", "expediteMap"];

/// 会话级加急信号允许公开的字段全集（对齐参考包 `outbound-sample.json` sequence 2）。
const CHANNEL_SIGNAL_KEYS: [&str; 4] = [
    "channelId",
    "urgentPostList",
    "urgentCurrentName",
    "hasUrgentPost",
];

/// 加急发起人显示名。
const REQUESTER_NAME: &str = "用户444";

// ── fixtures ────────────────────────────────────────────────────────────────

/// 构造服务端权威 expediteMap 绝对结构。
fn expedite_map(recipients: &[(&str, i64)]) -> Value {
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
    json!({
        "sender": {"id": REQUESTER_ID, "name": REQUESTER_NAME},
        "recipients": Value::Object(map)
    })
}

/// 确认前的三人加急绝对态：确认人待确认、peer 待确认、settled 已确认。
fn map_before_confirm() -> Value {
    expedite_map(&[(CONFIRMER_ID, 0), (PEER_ID, 0), (SETTLED_ID, 1)])
}

/// 确认后的三人加急绝对态：只有确认人由 0 变 1。
fn map_after_confirm() -> Value {
    expedite_map(&[(CONFIRMER_ID, 1), (PEER_ID, 0), (SETTLED_ID, 1)])
}

/// 构造 Go `post_update` 广播的确认 authority 帧。
fn confirm_authority(event_seq: u64, map: &Value) -> Value {
    json!({
        "action": "post_update",
        "data": {
            "id": POST_ID,
            "channelId": cid(CHANNEL_INDEX),
            "userId": REQUESTER_ID,
            "createAt": CREATE_AT,
            "updateAt": CREATE_AT + event_seq as i64 * 10,
            "message": "MV3-G05d urgent confirm",
            "type": "TEXT",
            "props": {"channel_event_seq": event_seq},
            "expediteMap": map,
            "event_seq": event_seq
        }
    })
}

/// 构造确认提交后 message 表的真实读回行。
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
        "user_id": REQUESTER_ID,
        "type": "TEXT",
        "message": "MV3-G05d urgent confirm",
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

/// 取 message 表 BatchUpdate 的 (patch 列名, patch map)。
fn message_patch(ops: &[StorageOp]) -> (Vec<String>, Value) {
    let spec = ops
        .iter()
        .find_map(|op| match op {
            StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(spec),
            _ => None,
        })
        .expect("confirm write must target the message table");
    assert_eq!(spec.key_col, "id");
    let key_vals: Vec<String> = spec
        .key_vals
        .iter()
        .map(|value| match value {
            SqlValue::Text(text) => text.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(key_vals, vec![POST_ID.to_string()]);
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
    (columns, Value::Object(patch))
}

/// 从 patch 读出落库的 expediteMap。
fn persisted_map(ops: &[StorageOp]) -> Value {
    let (_, patch) = message_patch(ops);
    serde_json::from_str(
        patch["expedite_map"]
            .as_str()
            .expect("expedite_map is text"),
    )
    .expect("persisted expedite_map must be JSON")
}

// ── 回放 ────────────────────────────────────────────────────────────────────

/// 以指定身份创建独立 persona harness（各 persona 状态完全隔离）。
fn persona(viewer_id: &str) -> Harness {
    Harness::new_with_identity(&[CHANNEL_INDEX], viewer_id, "mv3-g05d-company", "", "", "")
}

/// 回放一次 authority → PersistAtomic → durable readback，返回写集与终态事件。
fn apply_authority(
    harness: &mut Harness,
    event_seq: u64,
    map: &Value,
    now_ms: u64,
) -> (Vec<StorageOp>, Vec<Value>) {
    let frame =
        serde_json::to_vec(&confirm_authority(event_seq, map)).expect("frame must serialize");
    harness.step(Tick::Inbound(InboundBytes(Bytes::from(frame))), now_ms);
    let ops = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops.clone()),
            _ => None,
        })
        .expect("confirm authority must persist atomically");
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "authority must not emit before the durable commit"
    );
    let persist_corr =
        correlation(harness.sink.as_slice(), "atomic").expect("missing confirm PersistAtomic");

    harness.step(port_reply_ok(persist_corr.raw()), now_ms + 1);
    let readback_corr =
        correlation(harness.sink.as_slice(), "persist").expect("missing confirm readback");

    let row = durable_row(event_seq, map);
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
        "im_urgent_confirm",
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
    assert_eq!(effects.len(), 1);
    match effects.into_iter().next().expect("one effect") {
        Effect::Http { req, .. } => {
            let body = serde_json::from_slice(&req.body.expect("body")).unwrap();
            (req.url, body)
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

/// 取某 recipient 的绝对条目。
fn recipient<'a>(map: &'a Value, user_id: &str) -> &'a Value {
    map.get("recipients")
        .and_then(|recipients| recipients.get(user_id))
        .unwrap_or_else(|| panic!("recipient {user_id} must survive in the absolute map"))
}

// ── 测试 ────────────────────────────────────────────────────────────────────

/// 证明 MV3-G05d 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G05d", "post/mv3-g05d");
}

/// wire 只承载 `postId`：确认人由服务端会话身份决定，客户端不得外发成员或频道。
#[test]
fn urgent_confirm_wire_carries_only_the_post_id() {
    let effects = handle_outbound(
        "im_urgent_confirm",
        &serde_json::to_vec(&json!({
            "post_id": POST_ID,
            "channel_id": cid(CHANNEL_INDEX),
            "req_id": "mv3-g05d-request"
        }))
        .unwrap(),
        BASE,
        BASE,
        Some("conn-1"),
        Correlation::from_raw(1),
    )
    .expect("im_urgent_confirm must build");
    let (url, body) = single_http(effects);
    assert_eq!(url, format!("{BASE}/posts/urgentConfirm"));
    assert_eq!(
        body,
        json!({"postId": POST_ID}),
        "confirm wire must be exactly postId (no channelId, no userId, no status)"
    );
}

/// 确认入参 fail-closed：缺 postId / 空 postId / 未知字段 / 空 req_id 一律拒绝。
#[test]
fn urgent_confirm_inbound_is_fail_closed() {
    assert!(outbound_fails(json!({})));
    assert!(outbound_fails(json!({"post_id": ""})));
    assert!(outbound_fails(json!({"postId": POST_ID})));
    assert!(outbound_fails(
        json!({"post_id": POST_ID, "target_ids": [CONFIRMER_ID]})
    ));
    assert!(outbound_fails(json!({"post_id": POST_ID, "status": 1})));
    assert!(outbound_fails(json!({"post_id": POST_ID, "req_id": ""})));
    assert!(!outbound_fails(json!({"post_id": POST_ID})));
}

/// HTTP 2xx 只是 admission：确认命令与 ack 都不得本地结算加急态（INV-03）。
#[test]
fn urgent_confirm_http_ack_is_admission_only() {
    let mut harness = persona(CONFIRMER_ID);
    harness.step(
        command_json("im_urgent_confirm", json!({"post_id": POST_ID})),
        CREATE_AT as u64,
    );
    assert_eq!(harness.last_http_body(), Some(json!({"postId": POST_ID})));
    assert!(emitted_events(harness.sink.as_slice()).is_empty());
    let http_corr = correlation(harness.sink.as_slice(), "http").expect("missing confirm Http");

    harness.step(port_reply_ok(http_corr.raw()), CREATE_AT as u64 + 1);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "HTTP ack is admission only; WS authority settles the confirm"
    );
}

/// 本 Gate 的核心：确认只翻当前用户的 status，其他成员条目逐项完全不变。
#[test]
fn confirm_only_flips_the_confirming_member_and_leaves_peers_untouched() {
    let before = map_before_confirm();
    let after = map_after_confirm();

    let mut harness = persona(CONFIRMER_ID);
    let (_, first) = apply_authority(&mut harness, 1, &before, CREATE_AT as u64);
    let (ops, second) = apply_authority(&mut harness, 2, &after, CREATE_AT as u64 + 10);

    let pending = &first[0]["data"]["expediteMap"];
    let confirmed = &second[0]["data"]["expediteMap"];

    // 当前用户：0 → 1。
    assert_eq!(recipient(pending, CONFIRMER_ID)["status"], json!(0));
    assert_eq!(recipient(confirmed, CONFIRMER_ID)["status"], json!(1));

    // 其他成员：整条目深度相等，不是「只看 status」而是逐字段比对。
    assert_eq!(
        recipient(confirmed, PEER_ID),
        recipient(pending, PEER_ID),
        "未确认的其他成员条目必须逐字段不变"
    );
    assert_eq!(
        recipient(confirmed, SETTLED_ID),
        recipient(pending, SETTLED_ID),
        "更早已确认的成员不得被回退或重写"
    );

    // 落库的也必须是服务端绝对态本身，Helix 不做本地重算。
    assert_eq!(persisted_map(&ops), after);
    assert_eq!(confirmed, &after);
}

/// 确认不得删除任何 recipient 条目（加急历史靠完整数组回溯）。
#[test]
fn confirm_projection_preserves_every_recipient_entry() {
    let after = map_after_confirm();
    let mut harness = persona(CONFIRMER_ID);
    let (_, events) = apply_authority(&mut harness, 1, &after, CREATE_AT as u64);

    let recipients = events[0]["data"]["expediteMap"]["recipients"]
        .as_object()
        .expect("recipients must stay an object");
    let mut ids: Vec<&str> = recipients.keys().map(String::as_str).collect();
    ids.sort_unstable();
    let mut expected = vec![CONFIRMER_ID, PEER_ID, SETTLED_ID];
    expected.sort_unstable();
    assert_eq!(
        ids, expected,
        "确认投影必须保留全部 recipient 条目，禁止裁剪已确认项"
    );
    assert_eq!(
        events[0]["data"]["expediteMap"]["sender"]["id"], REQUESTER_ID,
        "发起人身份必须随绝对态一起保留"
    );
}

/// 重复确认幂等：同 seq 重放零 Effect；下一 seq 的相同绝对态收敛到同一 status=1 且 peers 不变。
#[test]
fn repeated_confirm_is_idempotent_and_converges_to_status_one() {
    let after = map_after_confirm();
    let mut harness = persona(CONFIRMER_ID);
    let (_, first) = apply_authority(&mut harness, 1, &after, CREATE_AT as u64);

    // ① 同 eventSeq 重放：完全无副作用。
    let replay = serde_json::to_vec(&confirm_authority(1, &after)).unwrap();
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(replay))),
        CREATE_AT as u64 + 5,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "same eventSeq confirm replay must produce no effect at all"
    );

    // ② 再来一次确认权威（seq+1）：状态收敛不翻转，peers 依旧不变。
    let (_, second) = apply_authority(&mut harness, 2, &after, CREATE_AT as u64 + 10);
    let once = &first[0]["data"]["expediteMap"];
    let twice = &second[0]["data"]["expediteMap"];
    assert_eq!(recipient(twice, CONFIRMER_ID)["status"], json!(1));
    assert_eq!(recipient(twice, PEER_ID), recipient(once, PEER_ID));
    assert_eq!(recipient(twice, SETTLED_ID), recipient(once, SETTLED_ID));
    assert_eq!(twice, once, "重复确认必须收敛到同一份绝对态");
    assert_eq!(
        second[0]["data"]["eventSeq"],
        json!(2),
        "eventSeq 是 per-channel 单调 cursor，与业务态收敛无关（INV-13）"
    );
}

/// 确认写集只覆盖加急列 + cursor，绝不触碰内容 / 已读 / 未读 / 频道级加急列。
#[test]
fn confirm_write_set_never_touches_content_read_or_channel_columns() {
    let after = map_after_confirm();
    let mut harness = persona(CONFIRMER_ID);
    let (ops, _) = apply_authority(&mut harness, 1, &after, CREATE_AT as u64);

    let (columns, patch) = message_patch(&ops);
    assert_eq!(
        columns,
        URGENT_PATCH_COLUMNS
            .iter()
            .map(|column| (*column).to_string())
            .collect::<Vec<_>>(),
        "confirm write set must be exactly expedite_map + event_seq + update_at"
    );
    assert_eq!(patch["event_seq"], json!(1));
    assert_eq!(patch["update_at"], json!(CREATE_AT + 10));
    for forbidden in [
        "urgent_count",
        "urgent_post_list",
        "has_urgent_post",
        "message",
        "props",
        "read_bits",
        "send_status",
        "quick_reply",
        "unread_count",
        "last_post",
        "create_at",
    ] {
        assert!(
            !columns.iter().any(|column| column == forbidden),
            "confirm write must not touch {forbidden}"
        );
    }
    assert!(ops.iter().any(|op| matches!(
        op,
        StorageOp::MonotonicUpsert(spec) if spec.table == "channel_event_cursor"
    )));
}

/// INV-12：确认人与加急发起人各自独立回放，收敛到同一绝对态且不携带任何 viewer-local 字段。
#[test]
fn confirm_dual_persona_replays_independently_to_the_same_absolute_state() {
    let after = map_after_confirm();

    let mut confirmer = persona(CONFIRMER_ID);
    let (_, confirmer_events) = apply_authority(&mut confirmer, 1, &after, CREATE_AT as u64);
    let mut requester = persona(REQUESTER_ID);
    let (_, requester_events) = apply_authority(&mut requester, 1, &after, CREATE_AT as u64);

    // sequence 1 = im:post:updated（绝对 expediteMap），sequence 2 = im:channel:update（会话级加急信号）。
    assert_eq!(confirmer_events.len(), 2);
    assert_eq!(confirmer_events[0]["event"], "im:post:updated");
    assert_eq!(
        confirmer_events, requester_events,
        "两个 persona 独立回放，终态必须一致且互不复制"
    );

    // 投影字段集合精确匹配契约（多一个 viewer 字段或少一个绝对字段都算破坏）。
    let mut keys: Vec<&str> = confirmer_events[0]["data"]
        .as_object()
        .expect("data must be an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    let mut expected = PROJECTION_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(confirmer_events[0]["data"]["id"], POST_ID);
    assert_eq!(confirmer_events[0]["data"]["channelId"], cid(CHANNEL_INDEX));
    assert_eq!(confirmer_events[0]["data"]["eventSeq"], json!(1));
    assert_eq!(
        confirmer_events[0]["data"]["updateAt"],
        json!(CREATE_AT + 10)
    );

    let encoded = serde_json::to_string(&confirmer_events[0]).unwrap();
    for forbidden in [
        "urgentCount",
        "urgentConfirmed",
        "canConfirmUrgent",
        "urgentState",
        "todo",
        "props",
        "readBits",
    ] {
        assert!(
            !encoded.contains(forbidden),
            "绝对确认投影不得携带 viewer-local 字段 {forbidden}"
        );
    }
}

/// 确认后的待确认动作分视角独立派生：确认人熄灭、未确认 peer 仍亮、发起人恒无待办。
#[test]
fn confirm_view_is_derived_per_persona() {
    let rows = json!([render_row(&map_after_confirm())]);
    let view = |viewer: &str| -> Value {
        helix_im::render_ready::shape_message_rows_for_viewer(&rows, viewer)[0].clone()
    };

    let confirmer_view = view(CONFIRMER_ID);
    let peer_view = view(PEER_ID);
    let requester_view = view(REQUESTER_ID);

    assert_eq!(
        confirmer_view["canConfirmUrgent"],
        json!(false),
        "已确认的成员不得再出现「已读」动作（幂等的 UI 面）"
    );
    assert_eq!(
        peer_view["canConfirmUrgent"],
        json!(true),
        "其他成员的待确认状态不受本人确认影响"
    );
    assert_eq!(
        requester_view["canConfirmUrgent"],
        json!(false),
        "加急发起人自己永不出现待确认（INV-12）"
    );

    // 绝对面三视角一致：确认集合恰为已确认的两人。
    for candidate in [&confirmer_view, &peer_view, &requester_view] {
        let mut confirmed: Vec<&str> = candidate["urgentConfirmedIds"]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap())
            .collect();
        confirmed.sort_unstable();
        assert_eq!(confirmed, vec![CONFIRMER_ID, SETTLED_ID]);
        assert_eq!(candidate["urgentRequiredCount"], json!(3));
        assert_eq!(candidate["urgentState"], "active");
    }

    // 全员确认后收敛到 acknowledged，三视角一致且都不再可确认。
    let all_done = json!([render_row(&expedite_map(&[
        (CONFIRMER_ID, 1),
        (PEER_ID, 1),
        (SETTLED_ID, 1)
    ]))]);
    for viewer in [CONFIRMER_ID, PEER_ID, REQUESTER_ID] {
        let shaped = helix_im::render_ready::shape_message_rows_for_viewer(&all_done, viewer);
        assert_eq!(shaped[0]["urgentState"], "acknowledged");
        assert_eq!(shaped[0]["canConfirmUrgent"], json!(false));
    }
}

/// 缺陷 13（确认侧）：确认终态必须再出一条会话级 `im:channel:update`，
/// 且全员确认后闪电熄灭——`urgentPostList` 清空、`hasUrgentPost=false`、`urgentCurrentName` 清串。
#[test]
fn confirm_authority_emits_channel_urgent_signal_and_clears_it_when_settled() {
    let mut harness = persona(CONFIRMER_ID);

    // ① 仍有未确认成员（PEER 未确认）→ 会话级加急信号保持点亮。
    let (_, partial) = apply_authority(&mut harness, 1, &map_after_confirm(), CREATE_AT as u64);
    assert_eq!(
        partial
            .iter()
            .map(|event| event["event"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["im:post:updated", "im:channel:update"],
        "确认终态出站序列必须是 post 绝对投影 → 会话级加急信号"
    );
    assert_eq!(partial[1]["data"]["hasUrgentPost"], json!(true));
    assert_eq!(partial[1]["data"]["urgentPostList"], json!([POST_ID]));
    assert_eq!(partial[1]["data"]["urgentCurrentName"], REQUESTER_NAME);

    // ② 全员确认 → 闪电熄灭（参考包 mv3-g05d outbound-sample sequence 2 的绝对形态）。
    let settled = expedite_map(&[(CONFIRMER_ID, 1), (PEER_ID, 1), (SETTLED_ID, 1)]);
    let (_, done) = apply_authority(&mut harness, 2, &settled, CREATE_AT as u64 + 10);
    assert_eq!(done[1]["event"], "im:channel:update");

    let channel = &done[1]["data"];
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
    assert_eq!(
        channel["urgentPostList"],
        json!([]),
        "全员确认后该 postId 必须移出 urgentPostList"
    );
    assert_eq!(channel["hasUrgentPost"], json!(false));
    assert_eq!(
        channel["urgentCurrentName"], "",
        "列表清空时 urgentCurrentName 必须清成空串"
    );
    assert!(
        !serde_json::to_string(&done[1])
            .unwrap()
            .contains("urgentCount"),
        "urgentCount 是死字段，确认链全程不得写"
    );
}
