//! MV3-G01k｜带回复发送。
//!
//! 参考包（线型权威）：`specs/003-messagev3-gate-inventory/gates/post/mv3-g01k/`。
//!
//! 三个面（对应 T055「reply descriptor、received + updated、幂等」）：
//!
//! 1. **reply descriptor**（DRIFT-F）：Angular 只提交「我点了哪条」的四锚描述符
//!    `replied{id,reply_id,reply_root_id,reply_first_level_id,snapshot}`，Go 需要的三锚
//!    `{replyId,replyRootId,replyFirstLevelId}` 与引用快照 `repliedMessage` 全部由 Helix 派生
//!    （#2..#6）。
//! 2. **出站三态**：`im:post:sending` 与参考样本逐字节一致并带上三锚（#7）；权威 echo 落库后
//!    三锚进入 durable `message` 行（#8）。
//! 3. **源消息回填**：Go 在回复落定后发布根消息 `post_update`，Helix 只在持久化成功后发布
//!    参考包序 4 的 `im:post:updated`，sender/receiver 各自回放得到同一绝对态（#9）。
//! 4. **幂等**（INV-02）：同一 `temporaryId` 重放 Command 只覆盖同一行，不产生第二条消息（#10）。
//!
//! 序 2 `im:post:received` 的三锚缺失已修复（`event::post::authority_projection` 改为按参考包
//! 声明字段集投影），锁在 `#10 authority_echo_projects_the_reply_anchors_into_received`。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, Tick};
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{port_reply_ok, port_reply_with_body, send_message_cmd_json, EffectKind, Harness};

/// 参考包 `deterministic_ids`。
const ACTOR_ID: &str = "user-actor-444";
const COMPANY_ID: &str = "company-phase1";
const REPLY_TEXT: &str = "这是回复内容";
const POST_ID: &str = "srvfix00000000000000000001";
const NOW_MS: u64 = 1_700_000_000_000;
const CREATE_AT: i64 = 1_700_000_000_001;

/// 返回首个指定类别 effect 的 correlation。
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

/// 读取实际 posts/create effect 的结构化 JSON body。
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

/// 证明 MV3-G01k 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01k", "post/mv3-g01k");
}

/// Helix 根据被选消息的四锚事实计算 Go posts/create 最终三锚。
#[test]
fn replied_descriptor_is_mapped_to_go_three_anchor_body() {
    let mut harness =
        Harness::new_with_identity(&[1], "user-actor-444", "company-phase1", "", "", "");
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "type": "TEXT",
            "text": "这是回复内容",
            "props": {},
            "viewers": ["all"],
            "mentions": [],
            "replied": {
                "id": "post-selected-001",
                "reply_id": "post-parent-001",
                "reply_root_id": "post-root-001",
                "reply_first_level_id": "post-first-level-001",
                "snapshot": {
                    "replied_user_id": "user-receiver-445",
                    "replied_user_name": "接收者",
                    "message": "被回复的正文",
                    "simple_message": "被回复的正文",
                    "is_revoke": false,
                    "type": "TEXT",
                    "props": {},
                    "viewers": ["all"]
                }
            }
        })),
        1_700_000_000_000,
    );
    harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        1_700_000_000_001,
    );
    let body = posts_create_body(harness.sink.as_slice());

    assert_eq!(body["replyId"], "post-selected-001");
    assert_eq!(body["replyRootId"], "post-root-001");
    assert_eq!(body["replyFirstLevelId"], "post-first-level-001");
    assert_eq!(
        body["repliedMessage"],
        json!({
            "repliedUserId": "user-receiver-445",
            "repliedUserName": "接收者",
            "message": "被回复的正文",
            "isRevoke": false,
            "type": "TEXT",
            "props": {},
            "viewers": ["all"]
        })
    );
}

/// 直接回复线程根时 Helix 仍输出完整三锚键，一级锚保持协议允许的空串。
#[test]
fn direct_root_reply_keeps_the_root_anchor_without_frontend_derivation() {
    let mut harness = Harness::new(&[1]);
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "type": "TEXT",
            "text": "直接回复根消息",
            "replied": {
                "id": "post-root-001",
                "reply_id": "",
                "reply_root_id": "",
                "reply_first_level_id": ""
            }
        })),
        1_700_000_000_000,
    );
    harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        1_700_000_000_001,
    );
    let body = posts_create_body(harness.sink.as_slice());

    assert_eq!(body["replyId"], "post-root-001");
    assert_eq!(body["replyRootId"], "post-root-001");
    assert_eq!(body["replyFirstLevelId"], "");
}

// ─────────────── #4..#6 reply descriptor 派生（Angular 不组装） ───────────────

/// 用给定 `replied` 描述符发一条回复，返回 `posts/create` body。
fn reply_body(replied: Value) -> Value {
    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "type": "TEXT",
            "text": REPLY_TEXT,
            "props": {},
            "viewers": ["all"],
            "mentions": [],
            "replied": replied
        })),
        NOW_MS,
    );
    harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        NOW_MS + 1,
    );
    posts_create_body(harness.sink.as_slice())
}

/// 被回复消息**直接回复了根**（有 `reply_id`、无 `reply_first_level_id`）时，本条新消息的
/// 一级锚必须被 Helix 派生为「被回复消息自己」——这是 `findReplyRootId` 三分支里唯一会
/// **凭空造出**一级锚的分支，纯透传实现会留空。
#[test]
fn first_level_anchor_is_derived_when_the_selected_post_replied_to_the_root() {
    let body = reply_body(json!({
        "id": "post-selected-001",
        "reply_id": "post-root-001",
        "reply_root_id": "post-root-001",
        "reply_first_level_id": ""
    }));

    assert_eq!(body["replyId"], "post-selected-001");
    assert_eq!(body["replyRootId"], "post-root-001");
    assert_eq!(
        body["replyFirstLevelId"], "post-selected-001",
        "被回复消息直接挂在根上时，它自己就是本条新消息的一级锚（Angular 不做这个推导）"
    );
}

/// 前端只给描述符、不给快照时，Helix **不得**凭空造 `repliedMessage`（零信任：宁缺勿造假引用）。
///
/// 对照组同时证明「有快照时一定会出现」，避免本断言退化成「反正永远没有」。
#[test]
fn replied_message_snapshot_is_absent_unless_the_client_supplies_one() {
    let without = reply_body(json!({
        "id": "post-selected-001",
        "reply_id": "",
        "reply_root_id": "",
        "reply_first_level_id": ""
    }));
    assert!(
        without.get("repliedMessage").is_none(),
        "无快照时不得伪造 repliedMessage，实测 {without}"
    );
    // 三锚仍必须完整（描述符是必需的，快照是可选的）。
    assert_eq!(without["replyId"], "post-selected-001");
    assert_eq!(without["replyRootId"], "post-selected-001");

    let with = reply_body(json!({
        "id": "post-selected-001",
        "reply_id": "",
        "reply_root_id": "",
        "reply_first_level_id": "",
        "snapshot": {
            "replied_user_id": "user-receiver-445",
            "replied_user_name": "接收者",
            "message": "被回复的正文",
            "simple_message": "被回复的正文",
            "is_revoke": false,
            "type": "TEXT",
            "props": {},
            "viewers": ["all"]
        }
    }));
    assert_eq!(with["repliedMessage"]["repliedUserId"], "user-receiver-445");

    let without_summary = reply_body(json!({
        "id": "post-selected-001",
        "reply_id": "",
        "reply_root_id": "",
        "reply_first_level_id": "",
        "snapshot": {
            "replied_user_id": "user-receiver-445",
            "replied_user_name": "接收者",
            "message": "被回复的正文",
            "is_revoke": false,
            "type": "TEXT",
            "props": {},
            "viewers": ["all"]
        }
    }));
    assert_eq!(
        without_summary["repliedMessage"]["message"], "被回复的正文",
        "Angular 不提交 simple_message 时由 Helix 从原始展示事实派生摘要"
    );
}

/// 引用预览文案由 Helix 按业务规则挑选（`buildRepliedMessageInfo`）：
/// 非 TEXT 或正文超 30 字 → 用 `simpleMessage` 摘要；短 TEXT → 用原文。
///
/// 纯透传实现会把长正文整段塞进引用条，气泡被撑爆——三个分支必须各自成立。
#[test]
fn replied_preview_picks_summary_for_long_or_non_text_sources() {
    let snapshot = |msg_type: &str, message: &str| {
        json!({
            "id": "post-selected-001",
            "reply_id": "",
            "reply_root_id": "",
            "reply_first_level_id": "",
            "snapshot": {
                "replied_user_id": "user-receiver-445",
                "replied_user_name": "接收者",
                "message": message,
                "simple_message": "[摘要]",
                "is_revoke": false,
                "type": msg_type,
                "props": {},
                "viewers": ["all"]
            }
        })
    };

    // 短 TEXT → 原文。
    let short_text = reply_body(snapshot("TEXT", "短正文"));
    assert_eq!(short_text["repliedMessage"]["message"], "短正文");

    // 长 TEXT（> 30 字）→ 摘要。
    let long = "正".repeat(31);
    let long_text = reply_body(snapshot("TEXT", &long));
    assert_eq!(
        long_text["repliedMessage"]["message"], "[摘要]",
        "超长正文必须降级为摘要，不得整段塞进引用条"
    );

    // 非 TEXT（短正文也一样）→ 摘要。
    let image = reply_body(snapshot("IMAGE", "短正文"));
    assert_eq!(image["repliedMessage"]["message"], "[摘要]");
    assert_eq!(image["repliedMessage"]["type"], "IMAGE");
    assert_eq!(image["repliedMessage"]["isRevoke"], false);
}

// ─────────────── #7 出站载荷 ↔ 权威样本 byte-level parity ───────────────

/// 读取本 Gate 的权威出站样本（线型权威：`gates/README.md`）。
fn outbound_sample() -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root().join("post/mv3-g01k/outbound-sample.json"),
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
            "tmp-mv3-g01k-001" => json!(temporary_id),
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

/// 发一条带回复的消息并推进到出站，返回 `(harness, sending 投影)`。
fn armed_reply_send() -> (Harness, Value) {
    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");
    let initial = harness.step(
        send_message_cmd_json(json!({
            "channel_id": common::cid(1),
            "type": "TEXT",
            "text": REPLY_TEXT,
            "props": {},
            "viewers": ["all"],
            "mentions": [],
            "replied": {
                "id": "post-parent-001",
                "reply_id": "post-root-001",
                "reply_root_id": "post-root-001",
                "reply_first_level_id": "post-parent-001",
                "snapshot": {
                    "replied_user_id": "user-receiver-445",
                    "replied_user_name": "接收者",
                    "message": "被回复的正文",
                    "simple_message": "被回复的正文",
                    "is_revoke": false,
                    "type": "TEXT",
                    "props": {},
                    "viewers": ["all"]
                }
            }
        })),
        NOW_MS,
    );
    harness.step(
        port_reply_ok(correlation_of(&initial, "persist")),
        NOW_MS + 1,
    );
    let sending = emitted_event(harness.sink.as_slice(), "im:post:sending")
        .expect("PersistOk must publish the optimistic sending projection");
    (harness, sending)
}

/// **参考包 parity 锁**：回复消息的 `im:post:sending` 载荷必须与 `outbound-sample.json`
/// 逐字节一致；此外乐观投影必须**额外**携带三锚，否则前端在 `sending` 阶段渲染不出引用条。
#[test]
fn sending_projection_matches_the_sample_and_carries_the_reply_anchors() {
    let expected_raw = reference_data(&outbound_sample(), "im:post:sending", "sender");
    let expected_fields = expected_raw
        .as_object()
        .expect("sample im:post:sending data must be an object");
    // 防呆：样本被清空时本测试会退化为 vacuous 断言。
    assert_eq!(expected_fields.get("message"), Some(&json!(REPLY_TEXT)));
    assert!(expected_fields.len() >= 9);

    let (_harness, sending) = armed_reply_send();
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

    // 样本的最小字段集里没有三锚，但乐观阶段必须已经有——否则引用条要等 echo 才出现。
    assert_eq!(actual["replyId"], json!("post-parent-001"));
    assert_eq!(actual["replyRootId"], json!("post-root-001"));
    assert_eq!(actual["replyFirstLevelId"], json!("post-parent-001"));
    assert_eq!(
        actual["repliedMessage"]["repliedUserId"],
        json!("user-receiver-445"),
        "sending 投影必须携带输入快照生成的引用块，不能等 authority echo 才出现"
    );
}

// ─────────────── #8 权威 echo 落库保留三锚 ───────────────

/// 取本批次首个 `Effect::PersistAtomic`（MessageV3 提交事务）的写集。
fn atomic_ops(effects: &[Effect]) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match effect {
        Effect::PersistAtomic { corr, ops } => Some((*corr, ops.as_slice())),
        _ => None,
    })
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

/// 驱动一条带三锚的权威 echo 帧进 `Tick::Inbound`，返回 `(harness, 提交事务写集快照)`。
///
/// 只到「提交事务已发出」为止；`drive_readback` 负责把它推到 `im:post:received`。
fn authority_echo_frame(temporary_id: &str) -> Value {
    json!({
        "action": "post",
        "data": {
            "id": POST_ID,
            "temporaryId": temporary_id,
            "channelId": common::cid(1),
            "event_seq": 1,
            "userId": ACTOR_ID,
            "type": "TEXT",
            "message": REPLY_TEXT,
            "simpleMessage": REPLY_TEXT,
            "props": {},
            "viewers": ["all"],
            "mentions": [],
            "replyId": "post-parent-001",
            "replyRootId": "post-root-001",
            "replyFirstLevelId": "post-parent-001",
            "repliedMessage": {
                "repliedUserId": "user-receiver-445",
                "repliedUserName": "接收者",
                "message": "被回复的正文",
                "isRevoke": false,
                "type": "TEXT",
                "props": {},
                "viewers": ["all"]
            },
            "createAt": CREATE_AT,
            "updateAt": CREATE_AT
        }
    })
}

/// 把已提交的 MessageV3 事务推过 channel_member 读回，返回 `im:post:received`。
fn drive_readback_to_received(harness: &mut Harness, atomic_corr: Correlation) -> Value {
    let after_commit = harness.step(port_reply_ok(atomic_corr.raw()), CREATE_AT as u64 + 1);
    let readback_corr = after_commit
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("commit 成功后必须读回当前 viewer 的 channel_member");
    let row: Row = vec![
        ("channel_id".to_string(), SqlValue::Text(common::cid(1))),
        ("user_id".to_string(), SqlValue::Text(ACTOR_ID.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(0)),
        (
            "last_post".to_string(),
            SqlValue::Text(json!({ "id": POST_ID }).to_string()),
        ),
    ];
    harness.step(
        port_reply_with_body(readback_corr, rows_to_reply_bytes(&[row])),
        CREATE_AT as u64 + 2,
    );
    emitted_event(harness.sink.as_slice(), "im:post:received")
        .expect("read-back 落定后必须发布 im:post:received")
}

/// **序 2 参考包 parity**：权威 echo 覆盖乐观态时，`im:post:received` 必须**继续**携带三锚。
///
/// 这是真实业务回归而不是形式断言：`sending` 阶段的乐观投影已带三锚（见 `#7`），前端据此渲染
/// 引用条；echo 回来的 `received` 是**绝对态覆盖**，它少一个键前端就把引用关系抹掉——用户看到
/// 引用条在消息「发送中 → 已发送」的瞬间凭空消失，且冷启动前再也回不来。
///
/// 参考包 `gates/post/mv3-g01k/outbound-sample.json` 序 2 的 sender 与 receiver 两侧都声明了
/// `replyId / replyRootId / replyFirstLevelId`，故此处按样本值逐字比对。
#[test]
fn authority_echo_projects_the_reply_anchors_into_received() {
    let expected = reference_data(&outbound_sample(), "im:post:received", "sender");
    // 防呆：样本被清空 / 三锚被删时本测试必须退化为红，而不是空转通过。
    for key in ["replyId", "replyRootId", "replyFirstLevelId"] {
        assert!(
            expected.get(key).and_then(Value::as_str).is_some(),
            "参考样本序 2 必须声明 `{key}`，否则本 parity 断言无权威可比：{expected}"
        );
    }

    let temporary_id = "tmpfix00000000000000000001";
    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&authority_echo_frame(temporary_id))
                .expect("authority frame must serialize"),
        ))),
        CREATE_AT as u64,
    );
    let (atomic_corr, _writes) =
        atomic_ops(harness.sink.as_slice()).expect("可见 post 必须进入 MessageV3 提交事务");
    let received = drive_readback_to_received(&mut harness, atomic_corr);

    for key in ["replyId", "replyRootId", "replyFirstLevelId"] {
        assert_eq!(
            received["data"][key], expected[key],
            "im:post:received 必须投影样本声明的 `{key}`；三锚只落库不投影会让 echo 覆盖掉引用条。\
             实测 data={:?}",
            received["data"]
        );
    }
    // 同一批参考包声明的其余可选字段一并锁住（同一处写死键集导致的漂移）。
    assert_eq!(
        received["data"]["simpleMessage"], expected["simpleMessage"],
        "simpleMessage 是样本声明字段，缺失会让左侧会话列表拿不到摘要"
    );
    assert_eq!(received["data"]["viewers"], expected["viewers"]);
    assert_eq!(
        received["data"]["repliedMessage"]["repliedUserId"],
        json!("user-receiver-445"),
        "authority echo 必须保留 parser 已解析的引用快照，避免 sent 绝对态抹掉引用块"
    );
}

/// 权威 echo 帧带回三锚时，提交事务必须把三锚原样写进 durable `message` 行——
/// 冷启动重开会话后引用条靠这三列重建，丢一列就永久丢引用关系。
#[test]
fn authority_echo_persists_the_reply_anchors_into_the_durable_row() {
    let temporary_id = "tmpfix00000000000000000001";
    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&authority_echo_frame(temporary_id))
                .expect("authority frame must serialize"),
        ))),
        CREATE_AT as u64,
    );
    let (atomic_corr, writes) =
        atomic_ops(harness.sink.as_slice()).expect("可见 post 必须进入 MessageV3 提交事务");

    assert_eq!(
        upsert_text(writes, "message", "reply_id").as_deref(),
        Some("post-parent-001")
    );
    assert_eq!(
        upsert_text(writes, "message", "reply_root_id").as_deref(),
        Some("post-root-001")
    );
    assert_eq!(
        upsert_text(writes, "message", "reply_first_level_id").as_deref(),
        Some("post-parent-001")
    );

    // 新回复的 post echo 只负责 received + channel:update；根消息 replyCount 的绝对更新
    // 由 Go 随后的 post_update 权威帧承载，见下一组 sequence-4 测试。
    let received = drive_readback_to_received(&mut harness, atomic_corr);
    assert_eq!(received["data"]["sendStatus"], "sent");
    assert_eq!(received["data"]["id"], POST_ID);
    assert_eq!(received["data"]["temporaryId"], temporary_id);
}

// ─────────────── #9 序 4 根消息 replyCount 绝对回填 ───────────────

/// 构造 Go 在回复聚合落定后广播的根消息 `post_update` 权威帧。
fn reply_root_update_frame(event_seq: u64) -> Value {
    json!({
        "action": "post_update",
        "data": {
            "id": "post-root-001",
            "channelId": common::cid(1),
            "userId": "user-receiver-445",
            "type": "TEXT",
            "message": "Phase 1 message operation",
            "simpleMessage": "Phase 1 message operation",
            "createAt": NOW_MS as i64,
            "updateAt": CREATE_AT,
            "sendStatus": "sent",
            "props": {},
            "replyCount": 1,
            "event_seq": event_seq
        }
    })
}

/// 独立回放一个 persona 的根消息更新，并验证重复 seq 不产生第二次写或事件。
fn replay_reply_root_update(viewer_id: &str, viewer: &str) -> Value {
    let mut harness = Harness::new_with_identity(&[1], viewer_id, COMPANY_ID, "", "", "");
    let frame = reply_root_update_frame(1);
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&frame).expect("root update frame must serialize"),
        ))),
        CREATE_AT as u64,
    );
    assert!(
        emitted_event(harness.sink.as_slice(), "im:post:updated").is_none(),
        "根消息更新落库前不得发布序 4 终态"
    );
    let (corr, _ops) =
        atomic_ops(harness.sink.as_slice()).expect("root post_update 必须原子提交内容与 cursor");
    harness.step(port_reply_ok(corr.raw()), CREATE_AT as u64 + 1);
    let updated = emitted_event(harness.sink.as_slice(), "im:post:updated")
        .expect("PersistOk 后必须发布根消息绝对态");

    let expected = substitute_reference_ids(
        &reference_data(&outbound_sample(), "im:post:updated", viewer),
        &common::cid(1),
        "",
    );
    let expected_fields = expected
        .as_object()
        .expect("sequence-4 sample data must be an object");
    let actual_fields = updated["data"]
        .as_object()
        .expect("sequence-4 projection data must be an object");
    for (key, expected_value) in expected_fields {
        assert_eq!(
            actual_fields.get(key),
            Some(expected_value),
            "{viewer} 的 sequence-4 投影必须逐字段匹配权威样本键 `{key}`"
        );
    }

    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&frame).expect("duplicate root update frame must serialize"),
        ))),
        CREATE_AT as u64 + 2,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "同 event_seq 重放必须零写、零事件"
    );
    updated
}

/// sender/receiver 各自消费 Go 根消息更新，并得到符合各自 persona 样本的 sequence-4 绝对态。
#[test]
fn reply_root_update_projects_sequence_four_for_both_personas() {
    let sender = replay_reply_root_update(ACTOR_ID, "sender");
    let receiver = replay_reply_root_update("user-receiver-445", "receiver");

    assert_eq!(sender["event"], "im:post:updated");
    assert_eq!(receiver["event"], "im:post:updated");
    assert_eq!(sender["data"]["id"], "post-root-001");
    assert_eq!(receiver["data"]["id"], "post-root-001");
    assert_eq!(sender["data"]["replyCount"], 1);
    assert_eq!(receiver["data"]["replyCount"], 1);
    assert_eq!(sender["data"]["isSelf"], false);
    assert_eq!(receiver["data"]["isSelf"], true);
}

// ─────────────── #10 幂等（同 temporaryId 重放） ───────────────

/// 把一行写集压成可比较指纹（`SqlValue` 不实现 `PartialEq`）。
fn row_fingerprint(row: &Row) -> Vec<(String, String)> {
    row.iter()
        .map(|(name, value)| (name.clone(), format!("{value:?}")))
        .collect()
}

/// 取本批次首个 `Effect::Persist` 中 `message` 表的 upsert spec。
fn optimistic_message_upsert(
    effects: &[Effect],
) -> (&'static str, Option<&'static str>, Vec<(String, String)>) {
    effects
        .iter()
        .find_map(|effect| {
            let Effect::Persist { ops, .. } = effect else {
                return None;
            };
            ops.iter().find_map(|op| match op {
                StorageOp::BatchUpsert(spec) if spec.table == "message" => Some((
                    spec.table,
                    spec.conflict_key,
                    row_fingerprint(spec.rows.first().expect("upsert must carry a row")),
                )),
                _ => None,
            })
        })
        .expect("send must open with a durable message upsert")
}

/// **幂等**（INV-02）：同一 `temporaryId` 重放 `im_send_message`，两次乐观写落在**同一主键**
/// 且逐列一致——ON CONFLICT(temporary_id) 覆盖同一行，不会长出第二条消息；
/// 两次出站 body 的 `temporaryId` 与三锚也完全一致，Go 侧据此按同一对账键去重。
#[test]
fn replaying_the_same_temporary_id_upserts_one_row_instead_of_a_second_message() {
    let intent = json!({
        "channel_id": common::cid(1),
        "temporary_id": "tmpfix00000000000000000001",
        "type": "TEXT",
        "text": REPLY_TEXT,
        "props": {},
        "viewers": ["all"],
        "mentions": [],
        "replied": {
            "id": "post-parent-001",
            "reply_id": "post-root-001",
            "reply_root_id": "post-root-001",
            "reply_first_level_id": "post-parent-001"
        }
    });

    let mut harness = Harness::new_with_identity(&[1], ACTOR_ID, COMPANY_ID, "", "", "");

    let first = harness.step(send_message_cmd_json(intent.clone()), NOW_MS);
    let (table, conflict_key, first_row) = optimistic_message_upsert(harness.sink.as_slice());
    harness.step(port_reply_ok(correlation_of(&first, "persist")), NOW_MS + 1);
    let first_body = posts_create_body(harness.sink.as_slice());

    let second = harness.step(send_message_cmd_json(intent), NOW_MS);
    let (_table, second_conflict_key, second_row) =
        optimistic_message_upsert(harness.sink.as_slice());
    harness.step(
        port_reply_ok(correlation_of(&second, "persist")),
        NOW_MS + 1,
    );
    let second_body = posts_create_body(harness.sink.as_slice());

    assert_eq!(table, "message");
    assert_eq!(
        conflict_key,
        Some("temporary_id"),
        "没有 ON CONFLICT(temporary_id) 就没有幂等，重放会长出第二行"
    );
    assert_eq!(conflict_key, second_conflict_key);
    assert_eq!(
        first_row, second_row,
        "同一 temporaryId 的两次乐观写必须逐列一致（覆盖同一行，而不是新增一行）"
    );
    // 防呆：行里确实带着那个主键值，否则上面的「相等」可能是两个空行相等。
    assert!(first_row.iter().any(
        |(name, value)| name == "temporary_id" && value.contains("tmpfix00000000000000000001")
    ));

    assert_eq!(first_body["temporaryId"], "tmpfix00000000000000000001");
    assert_eq!(
        serde_json::to_string(&first_body).unwrap(),
        serde_json::to_string(&second_body).unwrap(),
        "重放的出站 body 必须逐字节一致（同一对账键 + 同一三锚）"
    );
}
