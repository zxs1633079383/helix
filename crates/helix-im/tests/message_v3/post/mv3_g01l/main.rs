//! MV3-G01l｜定向可见发送（密聊 / 指定接收人）。
//!
//! 参考包（线型权威）：`specs/003-messagev3-gate-inventory/gates/post/mv3-g01l/`。
//!
//! 本 Gate 的两个面：
//!
//! 1. **出站面（Command）**：`im_send_message{viewers:[...]}` → `posts/create` body 的
//!    `viewers` 由 Helix 规范化（空补 `["all"]` / `all` 哨兵塌缩 / 去重），勾选白名单**原样**
//!    上 wire 并原样投影。可见集 `viewers ∪ {senderId}` 是 Go `NormalizeViewersWithUser` 的
//!    服务端语义，体现在回推的 authority 帧上，**不由客户端预并入**——客户端预并会污染
//!    `im:post:sending` / `im:post:received` 的 `data.viewers`，与线型权威
//!    `outbound-sample.json` 直接矛盾（缺陷 10 已回改实现对齐样本）。
//! 2. **投影面（Inbound 回放）**：同一条定向 post 帧在三个 persona 的独立 Harness 上回放，
//!    白名单内成员产生投影与未读，白名单**外**成员零原子写、零未读、零事件（INV-12）。
//!
//! 证伪性：把 `send_build::normalize_viewers` 的发送者并入加回来 → `#1b/#2/#3` 红；
//! 把 `channel::write::post::visible_to_user` 恒真化 → `#6` 红。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, port_reply_ok, EffectKind, Harness};

/// 参考包 `deterministic_ids`（inbound-sample.json）。
const ACTOR_ID: &str = "user-actor-444";
const RECEIVER_ID: &str = "user-receiver-445";
/// 白名单**外**的第三 persona（参考包 `negativeAssertions` 的「非 viewers 用户」）。
const OUTSIDER_ID: &str = "user-outsider-446";
const COMPANY_ID: &str = "company-phase1";
const MESSAGE: &str = "仅指定成员可见";
const POST_ID: &str = "srvfix00000000000000000001";
const TEMPORARY_ID: &str = "tmpfix00000000000000000001";
const CREATE_AT: i64 = 1_700_000_000_001;
const NOW_MS: u64 = 1_700_000_000_000;

type TestResult<T> = Result<T, String>;

// ───────────────────────── 出站面辅助 ─────────────────────────

/// 发一条 `im_send_message` 并推进到 HTTP 出站，返回 `(posts/create body, sending 投影)`。
fn send_and_capture(viewers: Value, sender_id: &str) -> (Value, Value) {
    let mut harness = Harness::new_with_identity(&[1], sender_id, COMPANY_ID, "", "", "");
    let initial = harness.step(
        command_json(
            "im_send_message",
            json!({
                "channel_id": common::cid(1),
                "type": "TEXT",
                "text": MESSAGE,
                "props": {},
                "viewers": viewers,
                "mentions": []
            }),
        ),
        NOW_MS,
    );
    let persist_corr = match initial.as_slice() {
        [EffectKind::Persist(corr)] => *corr,
        other => panic!("expected exactly one optimistic persist, got {other:?}"),
    };
    harness.step(port_reply_ok(persist_corr), NOW_MS + 1);
    let body = harness
        .last_http_body()
        .expect("PersistOk must continue to the real posts/create HTTP");
    let sending = emitted_event(harness.sink.as_slice(), "im:post:sending")
        .expect("PersistOk must publish the optimistic sending projection");
    (body, sending)
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

// ───────────────────────── 投影面辅助 ─────────────────────────

/// 真 Go `post` action 帧：`viewers` 是已规范化的定向白名单（含发送者）。
fn directed_authority() -> Value {
    json!({
        "action": "post",
        "data": {
            "channelId": common::cid(1),
            "event_seq": 1,
            "post": {
                "id": POST_ID,
                "temporaryId": TEMPORARY_ID,
                "channelId": common::cid(1),
                "userId": ACTOR_ID,
                "type": "TEXT",
                "message": MESSAGE,
                "simpleMessage": MESSAGE,
                "props": {},
                "viewers": [RECEIVER_ID, ACTOR_ID],
                "createAt": CREATE_AT,
                "updateAt": CREATE_AT
            }
        }
    })
}

/// 取本批次唯一相关持久请求（`atomic=true` = MessageV3 提交事务）。
fn correlated_persist(effects: &[Effect], atomic: bool) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match (atomic, effect) {
        (false, Effect::Persist { corr, ops }) => Some((*corr, ops.as_slice())),
        (true, Effect::PersistAtomic { corr, ops }) => Some((*corr, ops.as_slice())),
        _ => None,
    })
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

/// 编码当前 viewer 的 channel_member 绝对态读回。
fn member_reply(user_id: &str, unread_count: i64) -> ReplyBytes {
    let last_post = json!({
        "id": POST_ID,
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
    ReplyBytes(rows_to_reply_bytes(&[row]))
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

struct PersonaTrace {
    events: Vec<Value>,
    unread_bumped: bool,
}

/// 以独立本地库回放一个 persona 的定向 post 摄取。
///
/// 返回 `None` = 该 persona 根本没有产生 MessageV3 提交事务（不可见 → 零原子写），
/// 此时 `unread_bumped` 也必然为 false。
fn run_persona(viewer_id: &str, initial_unread: i64) -> TestResult<(Option<PersonaTrace>, bool)> {
    let mut harness = Harness::new_with_identity(&[1], viewer_id, COMPANY_ID, "", "", "");
    let inbound_bytes =
        serde_json::to_vec(&directed_authority()).map_err(|error| error.to_string())?;
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(inbound_bytes))),
        CREATE_AT as u64,
    );
    // 摄取批次绝不 Emit（Emit 必须在 read-back 之后）。
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "{viewer_id}: ingest batch must not emit before read-back"
    );

    let Some((persist_corr, writes)) = correlated_persist(harness.sink.as_slice(), true) else {
        // 不可见 persona：零原子写。顺带证明连非原子 Persist 也不带未读自增。
        let stray_bump = correlated_persist(harness.sink.as_slice(), false)
            .map(|(_, ops)| has_unread_bump(ops))
            .unwrap_or(false);
        return Ok((None, stray_bump));
    };
    let unread_bumped = has_unread_bump(writes);

    harness.step(port_reply_ok(persist_corr.raw()), CREATE_AT as u64 + 1);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "{viewer_id}: PersistOk batch must not emit before read-back"
    );
    let (readback_corr, readback_ops) = correlated_persist(harness.sink.as_slice(), false)
        .ok_or_else(|| format!("{viewer_id}: missing scoped read-back"))?;
    assert!(
        readback_ops
            .iter()
            .any(|op| matches!(op, StorageOp::ScopedGet(spec) if spec.table == "channel_member")),
        "{viewer_id}: read-back must scope to channel_member"
    );

    let final_unread = if viewer_id == ACTOR_ID {
        0
    } else {
        initial_unread + 1
    };
    harness.step(
        Tick::PortReply {
            corr: readback_corr,
            outcome: PortOutcome::Ok(member_reply(viewer_id, final_unread)),
        },
        CREATE_AT as u64 + 2,
    );
    Ok((
        Some(PersonaTrace {
            events: emitted_events(harness.sink.as_slice()),
            unread_bumped,
        }),
        unread_bumped,
    ))
}

// ───────────────────────── 参考样本 parity 辅助 ─────────────────────────

/// 读取本 Gate 的权威出站样本（线型权威：`gates/README.md`）。
fn outbound_sample() -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root().join("post/mv3-g01l/outbound-sample.json"),
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
///
/// **只**替换 id：样本的 `channel-phase1-001` / `tmp-mv3-g01l-001` 是参考包占位符，而 Helix 的
/// ChannelId / TemporaryId 是 26 字符 base32 typed PK，二者不可能字面相等。除 id 外的任何字段
/// 差异都必须暴露为红——那正是本测试要锁住的东西。
fn substitute_reference_ids(value: &Value, channel_id: &str, temporary_id: &str) -> Value {
    match value {
        Value::String(text) => match text.as_str() {
            "channel-phase1-001" => json!(channel_id),
            "tmp-mv3-g01l-001" => json!(temporary_id),
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

/// 证明 MV3-G01l 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01l", "post/mv3-g01l");
}

// ─────────────── #1b 出站载荷 ↔ 权威样本 byte-level parity ───────────────

/// **参考包 parity 锁**：`im:post:sending` 载荷必须与 `outbound-sample.json` 逐字节一致。
///
/// 参考包是线型权威（`specs/003-messagev3-gate-inventory/gates/README.md`）。任何一侧漂移
/// （实现改了字段值 / 样本改了声明）都必须在这里立刻变红，而不是靠人工比对发现。
///
/// 比对方式：取样本声明的**每一个 key**，按样本 key 顺序重建实际载荷子集，序列化后做字符串
/// （byte）相等。样本未声明的字段（`createAt` / `mentions` / 回复三锚）不参与比对——样本是
/// **最小必备字段集**：G01k 的回复样本同样不声明 `replyId`，按闭集解释会自相矛盾。
///
/// 证伪：把 `send_build::normalize_viewers` 的发送者并入加回来 → `viewers` 由
/// `["user-receiver-445"]` 变成 `["user-receiver-445","user-actor-444"]` → 本测试红。
#[test]
fn sending_projection_matches_the_authoritative_outbound_sample() {
    let expected_raw = reference_data(&outbound_sample(), "im:post:sending", "sender");
    let expected_fields = expected_raw
        .as_object()
        .expect("sample im:post:sending data must be an object");
    // 防呆：样本一旦被清空或被改成不含 viewers，本测试会退化为 vacuous 断言。
    assert!(
        expected_fields.contains_key("viewers") && expected_fields.len() >= 9,
        "参考样本必须声明含 viewers 的完整 sending 载荷，实测 {expected_fields:?}"
    );

    // 用样本 inbound 的客户端意图（只勾了 receiver，未勾自己）驱动真实发送链。
    let (_body, sending) = send_and_capture(json!([RECEIVER_ID]), ACTOR_ID);
    assert_eq!(sending["event"], "im:post:sending");
    let actual = sending["data"]
        .as_object()
        .expect("emitted im:post:sending data must be an object")
        .clone();

    let channel_id = common::cid(1);
    let temporary_id = actual["temporaryId"]
        .as_str()
        .expect("emitted projection must carry temporaryId")
        .to_string();
    let expected = substitute_reference_ids(&expected_raw, &channel_id, &temporary_id);
    let expected = expected
        .as_object()
        .expect("substitution preserves the object shape")
        .clone();

    // 按样本 key 顺序重建实际子集 → 序列化后 byte 相等。
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

// ───────────────────────── #2..#4 出站面 ─────────────────────────

/// 定向白名单原序、原样送上 wire，并与乐观投影同源。
///
/// **契约更正（缺陷 10）**：批次 1 此处断言 `body.viewers == [RECEIVER, ACTOR]`（客户端预并入
/// senderId）。该断言与本 Gate 的线型权威 `outbound-sample.json` 直接矛盾——样本的
/// `im:post:sending` 与 `im:post:received` 两处 `data.viewers` 均为 `["user-receiver-445"]`，
/// `contract.json` outbound 序 1/2 亦写死「勾选的白名单数组」「白名单原样回显」。因为
/// `im:post:sending` 的 `data.viewers` 就是本 body 的 `viewers` 逐字拷贝，两者不可能同时成立。
/// 按「参考包是权威，冲突时改实现」回改：wire 与投影统一为白名单原样。
/// 可见集 `viewers ∪ {senderId}` 仍然成立，只是它是 Go `NormalizeViewersWithUser` 的服务端
/// 语义（体现在回推的 authority 帧上，见 `directed_authority()` 与 `#5`），不由客户端预并。
#[test]
fn directed_viewers_keep_the_whitelist_verbatim() {
    let (body, sending) = send_and_capture(json!([RECEIVER_ID]), ACTOR_ID);

    assert_eq!(
        body["viewers"],
        json!([RECEIVER_ID]),
        "posts/create body 的 viewers 必须是勾选白名单原样（并入 senderId 是 Go 的职责）"
    );
    // 白名单原序即前端勾选顺序（UI 按此序回显）。
    assert_eq!(body["viewers"][0], RECEIVER_ID);
    // 乐观投影与 wire body 同源：Angular 立刻就能按同一白名单渲染。
    assert_eq!(sending["data"]["viewers"], body["viewers"]);
    assert_eq!(sending["data"]["sendStatus"], "sending");
    assert_eq!(sending["data"]["userId"], ACTOR_ID);
    // 定向不改变消息本体语义。
    assert_eq!(body["message"], MESSAGE);
    assert_eq!(body["type"], "TEXT");
}

/// 多人白名单：按首现序去重、剔空白；发送者不被客户端追加，前端勾了自己则原样保留。
///
/// **契约更正（缺陷 10）**：批次 1 期望 `[RECEIVER, "user-third-447", ACTOR]`（尾部追加
/// senderId）。同上，与 `outbound-sample.json` 的「白名单原样」矛盾，故去掉尾部 ACTOR。
/// 去重 / 剔空白 / 前端已勾自己时的幂等这三条既有覆盖**原样保留**。
#[test]
fn directed_viewers_are_deduplicated_without_appending_the_sender() {
    let (body, _) = send_and_capture(
        json!([RECEIVER_ID, "  ", RECEIVER_ID, "user-third-447"]),
        ACTOR_ID,
    );
    assert_eq!(body["viewers"], json!([RECEIVER_ID, "user-third-447"]));

    // 前端自己把自己勾进去 → 原样保留且不出现两次（幂等）。
    let (already, _) = send_and_capture(json!([ACTOR_ID, RECEIVER_ID]), ACTOR_ID);
    assert_eq!(already["viewers"], json!([ACTOR_ID, RECEIVER_ID]));
}

/// `all` 哨兵塌缩为 `["all"]`；空 viewers 仍走既有空补 `["all"]`（不得被并入逻辑污染）。
#[test]
fn all_sentinel_collapses_and_empty_viewers_still_fall_back_to_all() {
    let (sentinel, _) = send_and_capture(json!([RECEIVER_ID, "all"]), ACTOR_ID);
    assert_eq!(
        sentinel["viewers"],
        json!(["all"]),
        "命中 all 哨兵必须整体塌缩，不得残留具体 id 造成 Go 侧双语义"
    );

    // 既有归一化回归锚：空数组 / 全空白都补 ["all"]（G01a 样本同为 ["all"]）。
    let (empty, _) = send_and_capture(json!([]), ACTOR_ID);
    assert_eq!(empty["viewers"], json!(["all"]));
    let (blank, _) = send_and_capture(json!(["   "]), ACTOR_ID);
    assert_eq!(blank["viewers"], json!(["all"]));
}

// ───────────────────────── #5..#6 投影面 ─────────────────────────

/// sender 与白名单内 receiver 各自独立回放，投影一致但 viewer-local 未读分别为 0 与 N+1。
#[test]
fn whitelisted_personas_project_independently() -> TestResult<()> {
    let (sender, sender_bumped) = run_persona(ACTOR_ID, 7)?;
    let (receiver, receiver_bumped) = run_persona(RECEIVER_ID, 4)?;

    let sender = sender.ok_or_else(|| "sender must commit the directed post".to_string())?;
    let receiver =
        receiver.ok_or_else(|| "whitelisted receiver must commit the directed post".to_string())?;

    for (who, trace) in [("sender", &sender), ("receiver", &receiver)] {
        assert_eq!(trace.events.len(), 2, "{who}: expected received + update");
        assert_eq!(trace.events[0]["event"], "im:post:received");
        assert_eq!(trace.events[1]["event"], "im:channel:update");
        assert_eq!(trace.events[0]["data"]["id"], POST_ID);
        assert_eq!(trace.events[0]["data"]["temporaryId"], TEMPORARY_ID);
        assert_eq!(trace.events[0]["data"]["sendStatus"], "sent");
        // INV-07：Angular 永不接触 bitmap。
        assert!(trace.events[0]["data"].get("readBits").is_none());
    }

    // viewer-local 未读必须分视角独立计算（INV-12）：自己发的 +0，白名单内接收者 +1。
    assert!(!sender_bumped, "sender 自己发的消息不得给自己 +1");
    assert!(receiver_bumped, "白名单内接收者未读必须 +1");
    assert!(!sender.unread_bumped);
    assert!(receiver.unread_bumped);
    assert_eq!(sender.events[1]["data"]["unreadCount"], 0);
    assert_eq!(receiver.events[1]["data"]["unreadCount"], 5);
    Ok(())
}

/// **负向（本 Gate 的关键）**：白名单外 persona 零提交、零未读、零事件。
///
/// 「看不到」的证据不是「未读没涨」，而是这条消息**根本没有进入该 persona 的提交事务**：
/// 没有 MessageV3 PersistAtomic → 没有 read-back → 没有 `im:post:received` /
/// `im:channel:update`，其 `lastPost` 也就不可能被这条消息改写。
#[test]
fn non_viewer_persona_receives_nothing_at_all() -> TestResult<()> {
    let (outsider, bumped) = run_persona(OUTSIDER_ID, 3)?;

    assert!(
        outsider.is_none(),
        "白名单外成员不得产生 MessageV3 提交事务（否则会拿到投影与 lastPost）"
    );
    assert!(
        !bumped,
        "白名单外成员未读恒不变（不是 +1 后再减，而是从未 +1）"
    );

    // 与白名单内 persona 做对照，防止上面两条因为「谁都跑不出提交」而变得 vacuous。
    let (receiver, receiver_bumped) = run_persona(RECEIVER_ID, 3)?;
    assert!(
        receiver.is_some() && receiver_bumped,
        "对照组必须成立：同一帧对白名单内成员产生提交与未读，负向断言才有证明力"
    );
    Ok(())
}
