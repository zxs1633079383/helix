//! MV3-G01b｜发送 @所有人（TEMPLATE 壳承载文本 + 收到确认）。
//!
//! 参考包（线型权威）：`specs/003-messagev3-gate-inventory/gates/post/mv3-g01b/`。
//!
//! 三个面（对应 T049「TEMPLATE/TEXT、mention 双视角、sender 零 mention」）：
//!
//! 1. **出站规范化**：TEMPLATE 壳与 `mentions:["all"]` 哨兵由 Helix 权威构造（#2..#5），
//!    并与 `outbound-sample.json` 逐字节一致（#6）。
//! 2. **mention 双视角**（INV-12）：`@所有人` 的 mention 判定必须**逐 viewer 独立计算**——
//!    同一条帧喂给不同 `auth_user_id` 各算各的，禁止跨视角复制（#7）。
//! 3. **sender 零 mention**（INV-12 / AS-2.7）：发布者对自己发的 @所有人消息恒 `mention_hit=false`，
//!    并且任何 persona 的 viewer-local 写集里都不出现 mention 累加列（#7 / #8）——
//!    这正是参考包 `contract.json` 的 negativeAssertion「不得递增任何成员的
//!    mentionCount / mentionList / mentionUser」。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use helix_im::channel_write::{message_class, post_updates};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{port_reply_ok, port_reply_with_body, send_message_cmd_json, EffectKind, Harness};

/// 参考包 `deterministic_ids`。
const ACTOR_ID: &str = "user-actor-444";
const RECEIVER_ID: &str = "user-receiver-445";
const THIRD_ID: &str = "user-third-446";
const COMPANY_ID: &str = "company-phase1";
const POST_ID: &str = "srvfix00000000000000000001";
const TEMPORARY_ID: &str = "tmpfix00000000000000000001";
const CREATE_AT: i64 = 1_700_000_000_001;

/// 返回首个指定类别的 correlation。
fn correlation_of(effects: &[EffectKind], kind: &str) -> u64 {
    effects
        .iter()
        .find_map(|effect| match (kind, effect) {
            ("persist", EffectKind::Persist(corr)) | ("http", EffectKind::Http(corr)) => {
                Some(*corr)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {kind} correlation in {effects:?}"))
}

/// 读取 posts/create 的结构化 JSON body。
fn posts_create_body(effects: &[Effect]) -> Value {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Http { req, .. } if req.url.ends_with("/posts/create") => req
                .body
                .as_ref()
                .and_then(|body| serde_json::from_slice(body.as_ref()).ok()),
            _ => None,
        })
        .expect("missing posts/create body")
}

/// 证明 MV3-G01b 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01b", "post/mv3-g01b");
}

const MENTION_ALL_TEXT: &str = "@所有人请确认收到";

/// 发一条「最小意图」的 @所有人消息并取回 posts/create body。
///
/// 入参刻意是**未规范化的原始前端意图**（INV-01：Angular 只提交最小业务意图），
/// wire 形态由 Helix 构造。
fn posts_create_body_for(
    message_type: &str,
    temporary_id: &str,
    props: Value,
    mentions: Value,
) -> Value {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "temporary_id": temporary_id,
            "type": message_type,
            "text": MENTION_ALL_TEXT,
            "props": props,
            "viewers": ["all"],
            "mentions": mentions
        })),
        1_700_000_000_000,
    );
    harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        1_700_000_000_001,
    );
    posts_create_body(harness.sink.as_slice())
}

/// TEMPLATE 壳由 Helix 权威构造：前端不传 `props.template`，wire body 仍必须是
/// `{type:'TEXT',text:<正文>}`（参考包 post/mv3-g01b `outbound-sample.json`）。
///
/// 前端只提交 `type='TEMPLATE'` + 正文——没有任何 Helix 侧规范化时 props 会原样是 `{}`，
/// 本用例即失败。
#[test]
fn template_shell_is_constructed_by_helix_from_minimal_intent() {
    let body = posts_create_body_for("TEMPLATE", "tmp-mv3-g01b-001", json!({}), json!(["all"]));

    assert_eq!(body["type"], "TEMPLATE");
    assert_eq!(
        body["props"]["template"],
        json!({ "type": "TEXT", "text": MENTION_ALL_TEXT }),
        "TEMPLATE 壳必须由 Helix 补全，前端不得被要求自造 wire props"
    );
    assert_eq!(body["message"], MENTION_ALL_TEXT);
    assert_eq!(body["simpleMessage"], MENTION_ALL_TEXT);
    assert_eq!(body["viewers"], json!(["all"]));
}

/// 补全是**加法**：前端已给的模板字段、回执名单与同级 sibling props 一律不覆盖、不删除。
#[test]
fn template_normalization_preserves_client_supplied_fields_and_siblings() {
    let body = posts_create_body_for(
        "TEMPLATE",
        "tmp-mv3-g01b-002",
        json!({
            "template": {
                "text": "发布者自定义模板文案",
                "userIds": ["user-receiver-445"]
            },
            "legalSibling": { "keep": "unchanged" }
        }),
        json!(["all"]),
    );

    // type 缺失 → 补 TEXT；text 已给 → 不被正文覆盖。
    assert_eq!(body["props"]["template"]["type"], "TEXT");
    assert_eq!(body["props"]["template"]["text"], "发布者自定义模板文案");
    // 回执名单（收到确认权威来源）与 sibling props 原样保留。
    assert_eq!(
        body["props"]["template"]["userIds"],
        json!(["user-receiver-445"])
    );
    assert_eq!(
        body["props"]["legalSibling"],
        json!({ "keep": "unchanged" })
    );
}

/// `mentions` 的 `all` 哨兵由 Helix 收敛：脏序 / 混合具体 id / 重复项一律塌缩为 `["all"]`。
///
/// 纯透传实现会把 `["user-receiver-445","all","all"]` 原样发给 Go，本用例即失败。
#[test]
fn mention_all_sentinel_collapses_regardless_of_client_ordering() {
    let body = posts_create_body_for(
        "TEMPLATE",
        "tmp-mv3-g01b-003",
        json!({}),
        json!(["user-receiver-445", " all ", "all"]),
    );

    assert_eq!(
        body["mentions"],
        json!(["all"]),
        "@所有人 的 wire 形态恒为 [\"all\"]，哨兵吞并具体 id"
    );
}

/// TEXT 分支不被模板化：props 保持空对象，mentions 去重后按首现序保留具体 id。
#[test]
fn text_post_is_not_templated_and_keeps_deduplicated_mentions() {
    let body = posts_create_body_for(
        "TEXT",
        "tmp-mv3-g01b-004",
        json!({}),
        json!([
            "user-receiver-445",
            "",
            "user-receiver-445",
            "user-third-446"
        ]),
    );

    assert_eq!(body["type"], "TEXT");
    assert_eq!(body["props"], json!({}), "TEXT 不得被补出 template 壳");
    assert_eq!(
        body["mentions"],
        json!(["user-receiver-445", "user-third-446"])
    );
}

// ─────────────── #6 出站载荷 ↔ 权威样本 byte-level parity ───────────────

/// 读取本 Gate 的权威出站样本（线型权威：`gates/README.md`）。
fn outbound_sample() -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root().join("post/mv3-g01b/outbound-sample.json"),
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
            "tmp-mv3-g01b-001" => json!(temporary_id),
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

/// **参考包 parity 锁**：@所有人 的 `im:post:sending` 载荷必须与 `outbound-sample.json`
/// 逐字节一致——TEMPLATE 壳、`props.template{type,text}`、正文与 `sendStatus` 全部锁死。
///
/// 一旦实现把 @所有人 回落成 `type:"TEXT"`（丢掉整套「收到确认」能力，见 contract.json 的
/// openDecisions），本测试立刻红。
#[test]
fn sending_projection_matches_the_authoritative_outbound_sample() {
    let expected_raw = reference_data(&outbound_sample(), "im:post:sending", "sender");
    let expected_fields = expected_raw
        .as_object()
        .expect("sample im:post:sending data must be an object");
    // 防呆：样本被清空 / 被改成不含 TEMPLATE 壳时，本测试会退化为 vacuous 断言。
    assert_eq!(expected_fields.get("type"), Some(&json!("TEMPLATE")));
    assert!(
        expected_fields["props"]["template"].is_object() && expected_fields.len() >= 9,
        "参考样本必须声明含 props.template 的完整 sending 载荷，实测 {expected_fields:?}"
    );

    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "type": "TEMPLATE",
            "text": MENTION_ALL_TEXT,
            "props": {},
            "viewers": ["all"],
            "mentions": ["all"]
        })),
        1_700_000_000_000,
    );
    harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        1_700_000_000_001,
    );
    let sending = emitted_event(harness.sink.as_slice(), "im:post:sending")
        .expect("PersistOk must publish the optimistic sending projection");
    let actual = sending["data"]
        .as_object()
        .expect("emitted im:post:sending data must be an object")
        .clone();

    let temporary_id = actual["temporaryId"]
        .as_str()
        .expect("emitted projection must carry temporaryId")
        .to_string();
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

// ───────────────────────── #7 mention 双视角 ─────────────────────────

/// 构造一条已由 Go 广播的权威 post `data`（`post_updates` 的真实入参形态）。
fn broadcast_data(mentions: Value) -> Value {
    json!({
        "id": POST_ID,
        "temporaryId": TEMPORARY_ID,
        "channelId": common::cid(1),
        "userId": ACTOR_ID,
        "type": "TEMPLATE",
        "message": MENTION_ALL_TEXT,
        "simpleMessage": MENTION_ALL_TEXT,
        "props": { "template": { "type": "TEXT", "text": MENTION_ALL_TEXT } },
        "viewers": ["all"],
        "mentions": mentions,
        "createAt": CREATE_AT,
        "updateAt": CREATE_AT
    })
}

/// **mention 双视角 + sender 零 mention**（INV-12 / AS-2.7）。
///
/// `channel_write::post_updates` 是 WS `post` handler 唯一的 mention/未读判定点
/// （`ws/handlers/post.rs` 用 `ctx.auth_user_id` 调用它）。同一条帧对不同 viewer 必须**各算各的**。
///
/// 三组对照：
/// - **@所有人**（`mentions:["all"]`）：**任何** persona（含发布者自己）`mention_hit` 恒 false，
///   `message_class == "normal"`。这是参考包 negativeAssertion 的直接编码：@所有人 的触达确认走
///   `props.template.userIds` 回执，而不是 @ 角标；
/// - **定向 @**（`mentions:[RECEIVER]`）：只有被点名的 receiver `mention_hit=true`，
///   发布者与第三方均 false —— 证明上一组的「全 false」不是恒 false 的常量，
///   而是真的按 viewer 逐个算出来的；
/// - 两组里发布者的 `unread_delta` 恒为 0（自己发的消息不给自己涨未读）。
#[test]
fn mention_all_is_computed_per_viewer_and_never_credits_the_sender() {
    let channel = common::channel_id(1);

    // ① @所有人：三个 persona 全 false（含发布者自己）。
    let all = broadcast_data(json!(["all"]));
    for viewer in [ACTOR_ID, RECEIVER_ID, THIRD_ID] {
        let update = post_updates(channel, &all, viewer, CREATE_AT);
        assert!(
            !update.mention_hit,
            "{viewer}: @所有人 不得命中 mention（触达确认走 props.template.userIds 回执）"
        );
        assert_eq!(
            message_class(&update),
            "normal",
            "{viewer}: @所有人 的消息分类必须是 normal，不得是 mention"
        );
        assert_eq!(
            update.mentions,
            vec!["all".to_string()],
            "{viewer}: mentions 是消息级事实，跨视角逐字相同"
        );
    }

    // ② 定向 @：只有被点名者命中 —— 证明 mention 判定确实是 viewer-local 而非恒 false。
    let directed = broadcast_data(json!([RECEIVER_ID]));
    let sender_view = post_updates(channel, &directed, ACTOR_ID, CREATE_AT);
    let receiver_view = post_updates(channel, &directed, RECEIVER_ID, CREATE_AT);
    let third_view = post_updates(channel, &directed, THIRD_ID, CREATE_AT);

    assert!(
        receiver_view.mention_hit,
        "被点名的 receiver 必须命中 mention（否则 ① 的全 false 是恒真断言）"
    );
    assert_eq!(message_class(&receiver_view), "mention");
    assert!(!sender_view.mention_hit, "发布者不得因自己 @ 别人而被 @ 到");
    assert!(!third_view.mention_hit, "未被点名的第三方不得命中 mention");
    assert_ne!(
        receiver_view.mention_hit, third_view.mention_hit,
        "同一条帧的 mention 结论必须随 viewer 变化"
    );

    // ③ 两组里发布者的未读增量恒为 0（sender 豁免，与 mention 轴正交）。
    assert_eq!(
        post_updates(channel, &all, ACTOR_ID, CREATE_AT).unread_delta,
        0
    );
    assert_eq!(sender_view.unread_delta, 0);
    assert_eq!(receiver_view.unread_delta, 1);
}

// ───────────────────── #8 双 persona 提交事务（零 mention 累加） ─────────────────────

/// 权威 `post` 帧（真 Go wire：`action=post`，顶层 `event_seq`）。
fn authority_frame(mentions: Value) -> Tick {
    let frame = json!({
        "action": "post",
        "data": broadcast_data(mentions),
    });
    let mut frame = frame;
    frame["data"]["event_seq"] = json!(1);
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&frame).expect("authority frame must serialize"),
    )))
}

/// 编码当前 viewer 的 channel_member 绝对态读回。
fn member_reply(user_id: &str, unread_count: i64) -> Bytes {
    let last_post = json!({
        "id": POST_ID,
        "temporaryId": TEMPORARY_ID,
        "userId": ACTOR_ID,
        "message": MENTION_ALL_TEXT,
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

/// 取本批次首个 `Effect::PersistAtomic`（MessageV3 提交事务）的写集。
fn atomic_ops(effects: &[Effect]) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match effect {
        Effect::PersistAtomic { corr, ops } => Some((*corr, ops.as_slice())),
        _ => None,
    })
}

/// 收集写向指定表的所有列名（含 upsert 行列、bump 列与 set 列）。
fn columns_written_to(ops: &[StorageOp], table: &str) -> Vec<String> {
    let mut columns = Vec::new();
    for op in ops {
        match op {
            StorageOp::BatchUpsert(spec) if spec.table == table => {
                for row in &spec.rows {
                    columns.extend(row.iter().map(|(name, _)| name.clone()));
                }
            }
            StorageOp::BatchUpdate(spec) if spec.table == table => {
                columns.extend(spec.patch.iter().map(|(name, _)| name.clone()));
            }
            StorageOp::ScopedGuardedBump(spec) if spec.table == table => {
                columns.push(spec.bump_col.to_string());
                columns.extend(spec.set_cols.iter().map(|(name, _)| name.clone()));
            }
            _ => {}
        }
    }
    columns
}

/// 取指定表 upsert 行内某列的文本值。
fn upsert_text(ops: &[StorageOp], table: &str, column: &str) -> Option<String> {
    ops.iter().find_map(|op| match op {
        StorageOp::BatchUpsert(spec) if spec.table == table => spec.rows.first().and_then(|row| {
            row.iter().find_map(|(name, value)| match value {
                SqlValue::Text(text) if name == column => Some(text.clone()),
                _ => None,
            })
        }),
        _ => None,
    })
}

struct PersonaTrace {
    events: Vec<Value>,
    member_columns: Vec<String>,
    message_mentions: String,
    unread_bumped: bool,
}

/// 以独立本地库回放一个 persona 的 @所有人 post 摄取。
fn run_persona(viewer_id: &str, final_unread: i64) -> PersonaTrace {
    let mut harness = Harness::new_with_identity(&[1], viewer_id, COMPANY_ID, "", "", "");
    harness.step(authority_frame(json!(["all"])), CREATE_AT as u64);
    let (atomic_corr, writes) = atomic_ops(harness.sink.as_slice())
        .unwrap_or_else(|| panic!("{viewer_id}: 可见 post 必须进入 MessageV3 提交事务"));
    let member_columns = columns_written_to(writes, "channel_member");
    let message_mentions = upsert_text(writes, "message", "mentions")
        .unwrap_or_else(|| panic!("{viewer_id}: message 行必须落 mentions 列"));
    let unread_bumped = writes.iter().any(|op| {
        matches!(op, StorageOp::ScopedGuardedBump(spec)
            if spec.table == "channel_member"
                && spec.bump_col == "unread_count"
                && spec.bump_delta > 0)
    });

    let after_commit = harness.step(port_reply_ok(atomic_corr.raw()), CREATE_AT as u64 + 1);
    let readback_corr = after_commit
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .unwrap_or_else(|| panic!("{viewer_id}: commit 后必须读回本 viewer 的 channel_member"));
    harness.step(
        port_reply_with_body(readback_corr, member_reply(viewer_id, final_unread)),
        CREATE_AT as u64 + 2,
    );
    PersonaTrace {
        events: emitted_events(harness.sink.as_slice()),
        member_columns,
        message_mentions,
        unread_bumped,
    }
}

/// **双 persona 提交事务**：@所有人 落库时，两个视角的 `channel_member` 写集里都不出现
/// 任何 mention 累加列，而 `mentions` 作为**消息级事实**逐字相同地落在 `message` 行上。
///
/// 这条断言不是空转：同一个写集里 `unread_count` 是真的被写的（receiver 侧还是 bump），
/// 所以「有 unread 无 mention」精确表达了参考包的 negativeAssertion
/// 「不得递增任何成员的 mentionCount / mentionList / mentionUser」。
#[test]
fn mention_all_never_enters_any_persona_viewer_local_write_set() {
    let sender = run_persona(ACTOR_ID, 0);
    let receiver = run_persona(RECEIVER_ID, 1);

    for (who, trace) in [("sender", &sender), ("receiver", &receiver)] {
        assert!(
            !trace.member_columns.is_empty(),
            "{who}: channel_member 必须有写（否则下面的负向断言是空转）"
        );
        assert!(
            trace.member_columns.iter().any(|col| col == "unread_count"),
            "{who}: 对照锚——未读列必须在写集内，实测 {:?}",
            trace.member_columns
        );
        for column in &trace.member_columns {
            assert!(
                !column.contains("mention"),
                "{who}: viewer-local 写集不得出现 mention 累加列 `{column}`"
            );
        }
        // 消息级事实：mentions 原样落库，跨视角逐字相同。
        assert_eq!(trace.message_mentions, "[\"all\"]");
        assert_eq!(trace.events.len(), 2);
        assert_eq!(trace.events[0]["event"], "im:post:received");
        assert_eq!(trace.events[1]["event"], "im:channel:update");
        assert_eq!(trace.events[0]["data"]["type"], "TEMPLATE");
    }
    assert_eq!(sender.message_mentions, receiver.message_mentions);

    // viewer-local 未读仍然分视角独立：发布者不 bump，接收者 bump。
    assert!(!sender.unread_bumped, "发布者不得给自己涨未读");
    assert!(receiver.unread_bumped, "接收者未读必须 +1");
    assert_eq!(sender.events[1]["data"]["unreadCount"], 0);
    assert_eq!(receiver.events[1]["data"]["unreadCount"], 1);
}
