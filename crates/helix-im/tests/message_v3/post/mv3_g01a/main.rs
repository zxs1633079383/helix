//! MV3-G01a｜发送文本（发送内核基线 Gate）。
//!
//! 参考包（线型权威）：`specs/003-messagev3-gate-inventory/gates/post/mv3-g01a/`。
//!
//! 本 Gate 的四个面（对应 T041「三态、双 persona、Storage、PC/Mobile parity」）：
//!
//! 1. **乐观屏障**（INV-04）：`im:post:sending` 只能在 optimistic `PersistOk` 之后发布，
//!    HTTP / 15s timer 同样在屏障之后才起（#2 / #3）。
//! 2. **Storage**（INV-01 / FR-STO-3）：乐观落库行是 `message` 表、PK `temporary_id`、
//!    `send_status='sending'`，且 `temporaryId / createAt / simpleMessage` 全部由 Helix 铸造
//!    ——前端一个字节都没提交（#4）。权威 echo 的提交事务覆盖
//!    `message` + `channel` + `channel_member` + `channel_event_cursor` 四表（#7）。
//! 3. **三态**（INV-03）：`sending` → HTTP 2xx **仍是 sending**（只是 admission）→ WS echo
//!    才结算 `sent`（#5）。
//! 4. **双 persona**（INV-12）：同一条权威 post 帧在 sender / receiver 两个独立 Harness 上
//!    回放，消息级事实完全一致，viewer-local 未读各自独立计算，禁止跨视角复制（#6）。
//! 5. **PC / Mobile parity**（A2 三端 local-first）：`LocalStoreMode::Durable`（PC）与
//!    `LocalStoreMode::Session`（移动端）下发送链的 Effect 序列与 wire body 逐字节一致（#8）。
//! 6. **参考包 parity**：`im:post:sending` 载荷与 `outbound-sample.json` 逐字节一致（#1b）。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use helix_im::query::LocalStoreMode;
use helix_im::state::SendStatus;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{
    command_json, enveloped, port_reply_err, port_reply_ok, port_reply_with_body, EffectKind,
    Harness,
};

const NOW_MS: u64 = 1_700_000_000_000;
/// 参考包 `deterministic_ids`（inbound-sample.json）。
const ACTOR_ID: &str = "user-actor-444";
const RECEIVER_ID: &str = "user-receiver-445";
const COMPANY_ID: &str = "company-phase1";
const MESSAGE: &str = "Phase 1 text message";
const POST_ID: &str = "srvfix00000000000000000001";
const CREATE_AT: i64 = 1_700_000_000_001;

/// 从 optimistic message 写集中读取 Helix 分配的 temporaryId。
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

/// 从当前 effect batch 解码指定 MessageV3 事件。
fn emitted_event(effects: &[Effect], name: &str) -> Option<Value> {
    effects.iter().find_map(|effect| {
        let Effect::Emit { event } = effect else {
            return None;
        };
        let value = serde_json::from_slice::<Value>(event.0.as_ref()).ok()?;
        (value["event"] == name).then_some(value)
    })
}

/// 从当前 Effect batch 解出全部 MessageV3 envelope。
fn emitted_events(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| {
            let Effect::Emit { event } = effect else {
                return None;
            };
            serde_json::from_slice(event.0.as_ref()).ok()
        })
        .collect()
}

/// 取本批次首个 `Effect::Persist`（非原子）的写集。
fn persist_ops(effects: &[Effect]) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match effect {
        Effect::Persist { corr, ops } => Some((*corr, ops.as_slice())),
        _ => None,
    })
}

/// 取本批次首个 `Effect::PersistAtomic`（MessageV3 提交事务）的写集。
fn atomic_ops(effects: &[Effect]) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match effect {
        Effect::PersistAtomic { corr, ops } => Some((*corr, ops.as_slice())),
        _ => None,
    })
}

/// 从写集中取出针对某表的 upsert 行（列名 → 值）。
fn upsert_row<'a>(ops: &'a [StorageOp], table: &str) -> Option<&'a Row> {
    ops.iter().find_map(|op| match op {
        StorageOp::BatchUpsert(spec) if spec.table == table => spec.rows.first(),
        _ => None,
    })
}

/// 读取行内文本列。
fn text_col(row: &Row, column: &str) -> Option<String> {
    row.iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            (name, SqlValue::Text(text)) if name == column => Some(text.clone()),
            _ => None,
        })
}

/// 读取行内整数列。
fn int_col(row: &Row, column: &str) -> Option<i64> {
    row.iter()
        .find_map(|(name, value)| match (name.as_str(), value) {
            (name, SqlValue::Integer(number)) if name == column => Some(*number),
            _ => None,
        })
}

/// 把一行写集压成可比较指纹（`SqlValue` 不实现 `PartialEq`，用 Debug 表示逐列比对）。
fn row_fingerprint(row: &Row) -> Vec<(String, String)> {
    row.iter()
        .map(|(name, value)| (name.clone(), format!("{value:?}")))
        .collect()
}

/// 该批次是否含 channel_member 未读 +1（viewer-local 未读自增的唯一形态）。
fn has_unread_bump(ops: &[StorageOp]) -> bool {
    ops.iter().any(|op| {
        matches!(
            op,
            StorageOp::ScopedGuardedBump(spec)
                if spec.table == "channel_member"
                    && spec.bump_col == "unread_count"
                    && spec.bump_delta == 1
        )
    })
}

/// 权威 `post` 帧（真 Go wire：`action=post`，顶层 `event_seq`）。
fn authority_frame(temporary_id: &str) -> Tick {
    let frame = json!({
        "action": "post",
        "data": {
            "id": POST_ID,
            "temporaryId": temporary_id,
            "channelId": common::cid(1),
            "event_seq": 1,
            "userId": ACTOR_ID,
            "type": "TEXT",
            "message": MESSAGE,
            "simpleMessage": MESSAGE,
            "props": {},
            "viewers": ["all"],
            "mentions": [],
            "createAt": CREATE_AT,
            "updateAt": CREATE_AT
        }
    });
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&frame).expect("authority frame must serialize"),
    )))
}

/// 编码当前 viewer 的 channel_member 绝对态读回。
fn member_reply(user_id: &str, unread_count: i64, temporary_id: &str) -> Bytes {
    let last_post = json!({
        "id": POST_ID,
        "temporaryId": temporary_id,
        "userId": ACTOR_ID,
        "message": MESSAGE,
        "createAt": CREATE_AT
    });
    let row: Row = vec![
        ("channel_id".to_string(), SqlValue::Text(common::cid(1))),
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
    rows_to_reply_bytes(&[row])
}

/// 最小业务意图（INV-01：Angular 只提交这些字段）。
fn text_intent() -> Value {
    json!({
        "channel_id": common::cid(1),
        "type": "TEXT",
        "text": MESSAGE,
        "props": {},
        "viewers": ["all"],
        "mentions": []
    })
}

/// 发一条文本并推进到 `posts/create` 出站，返回 `(harness, temporaryId, http_corr, timer_id)`。
fn armed_send(sender_id: &str) -> (Harness, String, u64, u64) {
    let mut harness = Harness::new_with_identity(&[1], sender_id, COMPANY_ID, "", "", "");
    let initial = harness.step(command_json("im_send_message", text_intent()), NOW_MS);
    let temporary_id = persisted_temporary_id(harness.sink.as_slice());
    let persist_corr = match initial.as_slice() {
        [EffectKind::Persist(corr)] => *corr,
        other => panic!("expected exactly one optimistic persist, got {other:?}"),
    };
    let armed = harness.step(port_reply_ok(persist_corr), NOW_MS + 1);
    let http_corr = armed
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Http(corr) => Some(*corr),
            _ => None,
        })
        .expect("PersistOk must continue to posts/create");
    let timer_id = armed
        .iter()
        .find_map(|effect| match effect {
            EffectKind::ScheduleTimer(timer) => Some(*timer),
            _ => None,
        })
        .expect("posts/create must arm the 15s echo timeout");
    (harness, temporary_id, http_corr, timer_id)
}

// ───────────────────────── 参考样本 parity 辅助 ─────────────────────────

/// 读取本 Gate 的权威出站样本（线型权威：`gates/README.md`）。
fn outbound_sample() -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root().join("post/mv3-g01a/outbound-sample.json"),
    )
}

/// 取样本中指定 `(event, viewer)` 的 `rust.data`。
fn reference_data(sample: &Value, event: &str, viewer: &str) -> Value {
    sample["samples"]
        .as_array()
        .expect("outbound-sample.samples must be an array")
        .iter()
        .find(|entry| entry["rust"]["event"] == event && entry["viewer"] == viewer)
        .unwrap_or_else(|| panic!("outbound-sample must declare {event} for viewer {viewer}"))
        ["rust"]["data"]
        .clone()
}

/// 把样本里的确定性占位 id 替换成本 Harness 的真实 id（其余字段逐字保留）。
fn substitute_reference_ids(value: &Value, channel_id: &str, temporary_id: &str) -> Value {
    match value {
        Value::String(text) => match text.as_str() {
            "channel-phase1-001" => json!(channel_id),
            "tmp-mv3-g01a-001" => json!(temporary_id),
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| substitute_reference_ids(item, channel_id, temporary_id))
                .collect(),
        ),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| {
                    (
                        key.clone(),
                        substitute_reference_ids(item, channel_id, temporary_id),
                    )
                })
                .collect(),
        ),
        _ => value.clone(),
    }
}

// ───────────────────────── #1 参考包 ─────────────────────────

/// 证明 MV3-G01a 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01a", "post/mv3-g01a");
}

// ─────────────── #1b 出站载荷 ↔ 权威样本 byte-level parity ───────────────

/// **参考包 parity 锁**：`im:post:sending` 载荷必须与 `outbound-sample.json` 逐字节一致。
///
/// 比对方式与 G01l 同：取样本声明的每一个 key，按样本 key 顺序重建实际载荷子集后做字符串
/// （byte）相等。样本是最小必备字段集，未声明的字段（`createAt` / `mentions`）不参与。
#[test]
fn sending_projection_matches_the_authoritative_outbound_sample() {
    let expected_raw = reference_data(&outbound_sample(), "im:post:sending", "sender");
    let expected_fields = expected_raw
        .as_object()
        .expect("sample im:post:sending data must be an object");
    // 防呆：样本一旦被清空，本测试会退化为 vacuous 断言。
    assert!(
        expected_fields.contains_key("sendStatus")
            && expected_fields.contains_key("simpleMessage")
            && expected_fields.len() >= 9,
        "参考样本必须声明完整 sending 载荷，实测 {expected_fields:?}"
    );

    let (harness, temporary_id, _http, _timer) = armed_send(ACTOR_ID);
    let sending = emitted_event(harness.sink.as_slice(), "im:post:sending")
        .expect("PersistOk must publish the optimistic sending projection");
    let actual = sending["data"]
        .as_object()
        .expect("emitted im:post:sending data must be an object")
        .clone();

    let expected = substitute_reference_ids(&expected_raw, &common::cid(1), &temporary_id);
    let expected = expected
        .as_object()
        .expect("substitution preserves the object shape")
        .clone();

    let mut actual_subset = serde_json::Map::new();
    for key in expected.keys() {
        let value = actual.get(key).unwrap_or_else(|| {
            panic!("emitted im:post:sending 缺少样本声明的字段 `{key}`；实测字段 {actual:?}")
        });
        actual_subset.insert(key.clone(), value.clone());
    }
    assert_eq!(
        serde_json::to_string(&Value::Object(actual_subset)).unwrap(),
        serde_json::to_string(&Value::Object(expected)).unwrap(),
        "im:post:sending 载荷与 outbound-sample.json 不一致（参考包是权威，只能改实现）"
    );
}

// ───────────────────────── #2..#3 乐观屏障 ─────────────────────────

/// 首批只允许 durable optimistic 写，成功回执后才发布 sending 并启动 HTTP/timer。
#[test]
fn sending_projection_is_behind_the_optimistic_persist_barrier() {
    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");
    let initial = harness.step(command_json("im_send_message", text_intent()), NOW_MS);

    assert!(matches!(initial.as_slice(), [EffectKind::Persist(_)]));
    let temporary_id = persisted_temporary_id(harness.sink.as_slice());
    assert!(temporary_id.starts_with("helix_tmp_"));
    assert!(emitted_event(harness.sink.as_slice(), "im:post:sending").is_none());

    let persist_corr = match initial.as_slice() {
        [EffectKind::Persist(corr)] => *corr,
        other => panic!("expected one optimistic persist, got {other:?}"),
    };
    let after_persist = harness.step(port_reply_ok(persist_corr), NOW_MS + 1);
    assert_eq!(
        after_persist
            .iter()
            .filter(|effect| matches!(effect, EffectKind::Http(_)))
            .count(),
        1
    );
    assert_eq!(
        after_persist
            .iter()
            .filter(|effect| matches!(effect, EffectKind::ScheduleTimer(_)))
            .count(),
        1
    );
    let sending = emitted_event(harness.sink.as_slice(), "im:post:sending")
        .expect("PersistOk must publish the optimistic absolute projection");
    assert_eq!(sending["data"]["temporaryId"], temporary_id);
    assert_eq!(sending["data"]["channelId"], common::cid(1));
    assert_eq!(sending["data"]["userId"], ACTOR_ID);
    assert_eq!(sending["data"]["type"], "TEXT");
    assert_eq!(sending["data"]["message"], MESSAGE);
    assert_eq!(sending["data"]["simpleMessage"], MESSAGE);
    assert_eq!(sending["data"]["props"], json!({}));
    assert_eq!(sending["data"]["viewers"], json!(["all"]));
    assert_eq!(sending["data"]["sendStatus"], "sending");
    assert!(sending["data"].get("readBits").is_none());
}

/// Optimistic 持久化失败时禁止发送 HTTP/timer，也不得先发布 sending。
#[test]
fn optimistic_persist_failure_never_admits_the_message_to_transport() {
    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");
    let initial = harness.step(
        command_json(
            "im_send_message",
            json!({
                "channel_id": common::cid(1),
                "type": "TEXT",
                "text": "must stay local"
            }),
        ),
        NOW_MS,
    );
    let persist_corr = match initial.as_slice() {
        [EffectKind::Persist(corr)] => *corr,
        other => panic!("expected one optimistic persist, got {other:?}"),
    };

    let failure = harness.step(port_reply_err(persist_corr), NOW_MS + 1);
    assert!(failure
        .iter()
        .all(|effect| !matches!(effect, EffectKind::Http(_) | EffectKind::ScheduleTimer(_))));
    assert!(emitted_event(harness.sink.as_slice(), "im:post:sending").is_none());
}

// ───────────────────────── #4 Storage（乐观行） ─────────────────────────

/// 乐观落库行落在 `message` 表、PK `temporary_id`、`send_status='sending'`，且
/// `temporaryId / createAt / simpleMessage` 三个字段**全部由 Helix 铸造**（INV-01）。
///
/// 前端的最小意图里只有 `text`，没有 `simple_message`、没有 `create_at`、没有 id——所以这三列
/// 的值只可能来自 Helix：`simple_message` 由正文派生、`create_at` 由注入时钟派生、
/// `temporary_id` 由 Helix 分配器铸造（`helix_tmp_` 前缀）。
#[test]
fn optimistic_row_is_authored_by_helix_and_keyed_by_temporary_id() {
    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");
    harness.step(command_json("im_send_message", text_intent()), NOW_MS);

    let (_corr, ops) =
        persist_ops(harness.sink.as_slice()).expect("send must open with one durable persist");
    assert_eq!(ops.len(), 1, "纯文本发送不得夹带上传日志等额外写");
    let StorageOp::BatchUpsert(spec) = &ops[0] else {
        panic!("optimistic write must be a BatchUpsert, got {:?}", ops[0]);
    };
    assert_eq!(spec.table, "message");
    assert_eq!(
        spec.conflict_key,
        Some("temporary_id"),
        "乐观行必须以 temporaryId 为 PK（INV-02：同一 temporaryId 重放覆盖同一行）"
    );

    let row = spec.rows.first().expect("upsert must carry one row");
    let temporary_id = text_col(row, "temporary_id").expect("row must carry temporary_id");
    assert!(
        temporary_id.starts_with("helix_tmp_"),
        "temporaryId 必须由 Helix 铸造，实测 {temporary_id}"
    );
    assert_eq!(
        text_col(row, "channel_id").as_deref(),
        Some(common::cid(1).as_str())
    );
    assert_eq!(text_col(row, "user_id").as_deref(), Some(ACTOR_ID));
    assert_eq!(text_col(row, "type").as_deref(), Some("TEXT"));
    assert_eq!(text_col(row, "message").as_deref(), Some(MESSAGE));
    assert_eq!(
        text_col(row, "simple_message").as_deref(),
        Some(MESSAGE),
        "simpleMessage 由 Helix 派生，前端从未提交该字段"
    );
    assert_eq!(
        text_col(row, "send_status").as_deref(),
        Some("sending"),
        "落库即 sending（三态起点），不得先写 sent"
    );
    assert_eq!(
        int_col(row, "create_at"),
        Some(NOW_MS as i64),
        "createAt 取注入时钟，前端从未提交该字段"
    );
    assert_eq!(
        text_col(row, "id").as_deref(),
        Some(""),
        "服务端主键此刻还不存在，不得用 temporaryId 冒充（INV-02）"
    );
    assert_eq!(text_col(row, "viewers").as_deref(), Some("[\"all\"]"));
}

// ───────────────────────── #5 三态 ─────────────────────────

/// `sending` → HTTP 2xx **仍是 sending**（只是 admission）→ WS echo 才结算 `sent`（INV-03）。
///
/// 三个观察点各自独立：
/// - HTTP 回执零额外查询、零 Persist/Emit，`pending_send_status` 仍是 `Sending`；
/// - echo 到达才取消 15s timer 并进入原子提交，`pending_send` 出队；
/// - 读回落定后才发布 `im:post:received{sendStatus:"sent"}`，且 `id` 是服务端 postId、
///   `temporaryId` 仍是本地对账键（INV-02 二者不互换）。
#[test]
fn http_admission_stays_sending_until_the_ws_echo_settles_sent() {
    let (mut harness, temporary_id, http_corr, timer_id) = armed_send(ACTOR_ID);
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::Sending),
        "posts/create 出站后本条处于 sending"
    );

    // ① HTTP 2xx 只是 admission。
    let admitted = harness.step(
        port_reply_with_body(
            http_corr,
            enveloped(Bytes::from_static(
                br#"{"status":"SUCCESS","data":{"id":"srvfix00000000000000000001"}}"#,
            )),
        ),
        NOW_MS + 2,
    );
    assert!(
        admitted.is_empty(),
        "HTTP admission 不得额外查询、写终态或取消 echo timer，实测 {admitted:?}"
    );
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        Some(SendStatus::Sending),
        "2xx 之后仍必须是 sending（INV-03）"
    );

    // ② WS echo 才是权威：取消 timer + 进入原子提交 + pending 出队。
    let echoed = harness.step(authority_frame(&temporary_id), NOW_MS + 3);
    assert!(
        echoed
            .iter()
            .any(|effect| matches!(effect, EffectKind::CancelTimer(id) if *id == timer_id)),
        "echo 必须取消 15s 超时 timer，实测 {echoed:?}"
    );
    assert_eq!(
        harness.module.pending_send_status(&temporary_id),
        None,
        "echo 结算后本条离开 pending"
    );
    let (atomic_corr, _ops) =
        atomic_ops(harness.sink.as_slice()).expect("echo authority must enter atomic commit");
    assert!(
        emitted_events(harness.sink.as_slice())
            .iter()
            .all(|event| event["event"] != "im:post:received"),
        "read-back 之前不得发布 received"
    );

    // ③ 提交成功 → 复合键读回 → 才发布终态。
    let after_commit = harness.step(port_reply_ok(atomic_corr.raw()), NOW_MS + 4);
    let readback_corr = after_commit
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("commit 成功后必须发起当前 viewer 的复合键读回");
    let settled = harness.step(
        port_reply_with_body(readback_corr, member_reply(ACTOR_ID, 0, &temporary_id)),
        NOW_MS + 5,
    );
    assert!(settled
        .iter()
        .any(|effect| matches!(effect, EffectKind::Emit)));

    let received = emitted_event(harness.sink.as_slice(), "im:post:received")
        .expect("read-back 落定后必须发布 im:post:received");
    assert_eq!(received["data"]["sendStatus"], "sent");
    assert_eq!(
        received["data"]["id"], POST_ID,
        "id 必须被服务端 postId 覆写"
    );
    assert_eq!(
        received["data"]["temporaryId"], temporary_id,
        "temporaryId 仍是本地对账键，不得被 postId 顶替（INV-02）"
    );
    assert!(
        received["data"].get("readBits").is_none(),
        "INV-07：Angular 永不接触 bitmap"
    );
}

// ───────────────────────── #6 双 persona ─────────────────────────

/// 一个 persona 独立回放同一条权威帧后的观察量。
struct PersonaTrace {
    events: Vec<Value>,
    unread_bumped: bool,
    /// 提交事务写进 `message` 表的行指纹（消息级事实，应跨视角完全一致）。
    message_row: Vec<(String, String)>,
    /// 读回落定后 `im:channel:update` 报出的 viewer-local 未读绝对值。
    unread_count: i64,
}

/// 以独立本地库回放一个 persona 的 post 摄取（sender 与 receiver 各一份 Harness）。
fn run_persona(viewer_id: &str, initial_unread: i64, temporary_id: &str) -> PersonaTrace {
    let mut harness = Harness::new_with_identity(&[1], viewer_id, COMPANY_ID, "", "", "");
    harness.step(authority_frame(temporary_id), CREATE_AT as u64);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "{viewer_id}: 摄取批次不得在 read-back 之前 Emit"
    );

    let (atomic_corr, writes) = atomic_ops(harness.sink.as_slice())
        .unwrap_or_else(|| panic!("{viewer_id}: 可见 post 必须进入 MessageV3 提交事务"));
    let unread_bumped = has_unread_bump(writes);
    let message_row = row_fingerprint(
        upsert_row(writes, "message")
            .unwrap_or_else(|| panic!("{viewer_id}: 提交事务必须写 message 表")),
    );

    let after_commit = harness.step(port_reply_ok(atomic_corr.raw()), CREATE_AT as u64 + 1);
    let readback_corr = after_commit
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{viewer_id}: commit 后必须读回本 viewer 的 channel_member"));

    // viewer-local 绝对值由各自本地库给出：sender 归零，receiver 在自己的初值上 +1。
    let unread_count = if viewer_id == ACTOR_ID {
        0
    } else {
        initial_unread + 1
    };
    harness.step(
        port_reply_with_body(
            readback_corr,
            member_reply(viewer_id, unread_count, temporary_id),
        ),
        CREATE_AT as u64 + 2,
    );
    PersonaTrace {
        events: emitted_events(harness.sink.as_slice()),
        unread_bumped,
        message_row,
        unread_count,
    }
}

/// **双 persona 独立回放**（INV-12）：消息级事实跨视角一致，viewer-local 未读各自独立计算。
///
/// 两个 persona 用**各自的 Harness / 各自的身份 / 各自的初始未读**回放同一帧：
/// - sender 的提交事务是「未读绝对归零」的 upsert，**没有** `ScopedGuardedBump`；
/// - receiver 的提交事务带 `unread_count +1` 的 `ScopedGuardedBump`；
/// - 两侧写进 `message` 表的行逐列相等（消息本体是共享事实，不是视角状态）；
/// - 两侧 `im:channel:update.unreadCount` 分别是 0 与 `初值+1`，且**不同**——
///   任何「把 sender 的数值复制给 receiver」的实现都会让最后一条断言变红。
#[test]
fn dual_persona_replay_keeps_message_facts_shared_and_unread_viewer_local() {
    let temporary_id = "tmpfix00000000000000000001";
    let sender = run_persona(ACTOR_ID, 0, temporary_id);
    let receiver = run_persona(RECEIVER_ID, 4, temporary_id);

    for (who, trace) in [("sender", &sender), ("receiver", &receiver)] {
        assert_eq!(
            trace.events.len(),
            2,
            "{who}: 读回落定后恰好两条终态（received + channel:update），实测 {:?}",
            trace.events
        );
        assert_eq!(trace.events[0]["event"], "im:post:received");
        assert_eq!(trace.events[1]["event"], "im:channel:update");
        assert_eq!(trace.events[0]["data"]["id"], POST_ID);
        assert_eq!(trace.events[0]["data"]["temporaryId"], temporary_id);
        assert_eq!(trace.events[0]["data"]["sendStatus"], "sent");
        assert_eq!(trace.events[0]["data"]["message"], MESSAGE);
        assert!(trace.events[0]["data"].get("readBits").is_none());
    }

    // 消息级事实：两侧落库行逐列相等（含 id / message / viewers / mentions / event_seq）。
    assert_eq!(
        sender.message_row, receiver.message_row,
        "message 行是共享消息事实，不得随 viewer 变化"
    );

    // viewer-local：写形态不同 + 绝对值不同。
    assert!(
        !sender.unread_bumped,
        "sender 自己发的消息不得给自己 +1（INV-12）"
    );
    assert!(receiver.unread_bumped, "接收者未读必须 +1");
    assert_eq!(sender.unread_count, 0);
    assert_eq!(receiver.unread_count, 5);
    assert_eq!(sender.events[1]["data"]["unreadCount"], 0);
    assert_eq!(receiver.events[1]["data"]["unreadCount"], 5);
    assert_ne!(
        sender.events[1]["data"]["unreadCount"], receiver.events[1]["data"]["unreadCount"],
        "两个视角的未读绝对值必须各算各的，禁止跨视角复制"
    );
}

// ───────────────────────── #7 Storage（提交事务四表） ─────────────────────────

/// 权威 echo 的提交事务覆盖 `message` + `channel` + `channel_member` +
/// `channel_event_cursor` 四表，并且是**单个原子事务**（缺一即出现「消息落了但 cursor 没推」
/// 这类不可恢复的半提交）。
#[test]
fn authority_commit_touches_the_four_message_v3_tables_atomically() {
    let mut harness = Harness::new_with_identity(&[1], RECEIVER_ID, COMPANY_ID, "", "", "");
    harness.step(
        authority_frame("tmpfix00000000000000000002"),
        CREATE_AT as u64,
    );
    let (_corr, ops) =
        atomic_ops(harness.sink.as_slice()).expect("可见 post 必须进入 MessageV3 原子提交");

    let tables: Vec<&str> = ops
        .iter()
        .map(|op| match op {
            StorageOp::BatchUpsert(spec) => spec.table,
            StorageOp::BatchUpdate(spec) => spec.table,
            StorageOp::ScopedGuardedBump(spec) => spec.table,
            StorageOp::ScopedGet(spec) => spec.table,
            StorageOp::MonotonicUpsert(spec) => spec.table,
            other => panic!("unexpected storage op in commit: {other:?}"),
        })
        .collect();
    for table in [
        "message",
        "channel",
        "channel_member",
        "channel_event_cursor",
    ] {
        assert!(
            tables.contains(&table),
            "提交事务必须写 {table}，实测写了 {tables:?}"
        );
    }
    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Persist { .. })),
        "四表写必须在同一个 PersistAtomic 内，不得拆成非原子 Persist"
    );
}

// ───────────────────────── #8 PC / Mobile parity ─────────────────────────

/// **PC / Mobile parity**：`Durable`（PC 持久库）与 `Session`（移动端会话库）两种本地存储模式
/// 下，发送链的 Effect 序列与 `posts/create` wire body 完全一致。
///
/// 平台差异只允许体现在**本地存储介质**上，不允许泄漏到业务链路：一旦某个平台少发一次
/// Persist、少 arm 一次 timer，或把 body 字段裁剪掉，本测试即红。
#[test]
fn send_chain_is_identical_across_pc_and_mobile_local_store_modes() {
    fn trace(mode: LocalStoreMode) -> (Vec<EffectKind>, Vec<EffectKind>, Value) {
        let mut harness = Harness::new_with_store_mode(&[1], mode);
        let initial = harness.step(command_json("im_send_message", text_intent()), NOW_MS);
        let temporary_id = persisted_temporary_id(harness.sink.as_slice());
        let persist_corr = match initial.as_slice() {
            [EffectKind::Persist(corr)] => *corr,
            other => panic!("{mode:?}: expected one optimistic persist, got {other:?}"),
        };
        let armed = harness.step(port_reply_ok(persist_corr), NOW_MS + 1);
        let mut body = harness
            .last_http_body()
            .expect("PersistOk must continue to posts/create");
        // temporaryId 是每次运行新铸的，比对前归一（其余字段必须逐字相等）。
        assert_eq!(body["temporaryId"], json!(temporary_id));
        body["temporaryId"] = json!("<helix-minted>");
        (
            initial.iter().map(discriminant).collect(),
            armed.iter().map(discriminant).collect(),
            body,
        )
    }
    /// 只保留 Effect 类别，抹掉每次运行都会变的 correlation / timer id。
    fn discriminant(effect: &EffectKind) -> EffectKind {
        match effect {
            EffectKind::Persist(_) => EffectKind::Persist(0),
            EffectKind::PersistAtomic(_) => EffectKind::PersistAtomic(0),
            EffectKind::Http(_) => EffectKind::Http(0),
            EffectKind::Request(_) => EffectKind::Request(0),
            EffectKind::ScheduleTimer(_) => EffectKind::ScheduleTimer(0),
            EffectKind::CancelTimer(_) => EffectKind::CancelTimer(0),
            EffectKind::UploadFile(_) => EffectKind::UploadFile(0),
            other => other.clone(),
        }
    }

    let (pc_initial, pc_armed, pc_body) = trace(LocalStoreMode::Durable);
    let (mobile_initial, mobile_armed, mobile_body) = trace(LocalStoreMode::Session);

    // 防呆：链路本身必须非空且含真实出站，否则「相等」毫无意义。
    assert_eq!(pc_initial, vec![EffectKind::Persist(0)]);
    assert!(pc_armed.contains(&EffectKind::Http(0)));
    assert!(pc_armed.contains(&EffectKind::ScheduleTimer(0)));
    assert_eq!(pc_body["message"], MESSAGE);

    assert_eq!(pc_initial, mobile_initial, "首批 Effect 序列跨端必须一致");
    assert_eq!(pc_armed, mobile_armed, "出站批 Effect 序列跨端必须一致");
    assert_eq!(
        serde_json::to_string(&pc_body).unwrap(),
        serde_json::to_string(&mobile_body).unwrap(),
        "posts/create wire body 跨端必须逐字节一致"
    );
}
