// MV3-G05a｜撤回自己发的消息。
//
// 参考包（线型权威）：
//   specs/003-messagev3-gate-inventory/gates/post/mv3-g05a/*.json
// 关键不变量：
//   * OD-01：撤回的实时事件是 `im:post:revoke`，**不得**降级成 `im:post:updated`。
//   * INV-12：sender / receiver 的 unreadCount 与 lastPost 分视角独立计算，禁止复制。
//   * contract.effects：撤回不改未读；不物理删消息；不改写 createAt。
//   * contract.lastPostOrder：撤回后的摘要由 Go 权威回退到「上一条未撤回消息」，
//     Helix 逐字段透传，不在本地二次推导。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::channel_write::message_v3_revoke_channel_ops;
use helix_im::state::ChannelId;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, port_reply_ok, Harness};

const CHANNEL_INDEX: u64 = 5;
const POST_ID: &str = "srvg5a00000000000000000005";
const TEMPORARY_ID: &str = "tmpg5a00000000000000000005";
const PREVIOUS_POST_ID: &str = "srvg5a00000000000000000004";
const ACTOR_ID: &str = "user-actor-444";
const RECEIVER_ID: &str = "user-receiver-445";
const MESSAGE: &str = "MV3-G05a revoke";
const CREATE_AT: i64 = 1_700_000_000_000;
const UPDATE_AT: i64 = CREATE_AT + 10;
const PREVIOUS_CREATE_AT: i64 = CREATE_AT - 1_000;

type TestResult<T> = Result<T, String>;

// ─── 参考包 ────────────────────────────────────────────────────────────────

/// 证明 MV3-G05a 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G05a", "post/mv3-g05a");
}

/// 读取参考包内某个文件。
fn reference_json(file_name: &str) -> Value {
    let path = phase1_reference::reference_root()
        .join("post/mv3-g05a")
        .join(file_name);
    phase1_reference::read_json(&path)
}

// ─── 写集（`message_v3_revoke_channel_ops`）────────────────────────────────

/// 取 ops 里 channel 表的 patch。
fn channel_patch(ops: &[StorageOp]) -> Row {
    ops.iter()
        .find_map(|op| match op {
            StorageOp::BatchUpdate(spec) if spec.table == "channel" => Some(spec.patch.clone()),
            _ => None,
        })
        .expect("撤回写集必须包含 channel 表的 BatchUpdate")
}

/// 取 ops 里 channel_member 表的唯一 upsert 行。
fn member_row(ops: &[StorageOp]) -> Row {
    let spec = ops
        .iter()
        .find_map(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "channel_member" => Some(spec),
            _ => None,
        })
        .expect("撤回写集必须包含 channel_member 的 BatchUpsert");
    assert_eq!(
        spec.conflict_key,
        Some("channel_id,user_id"),
        "channel_member 必须按 (channel_id,user_id) 复合键收敛，否则会给同频道每个 viewer 插重复行"
    );
    assert_eq!(spec.rows.len(), 1, "撤回只写当前 viewer 一行");
    spec.rows[0].clone()
}

/// 从一组列里读文本列。
fn text_of(row: &Row, column: &str) -> Option<String> {
    row.iter().find_map(|(name, value)| match value {
        SqlValue::Text(text) if name == column => Some(text.clone()),
        _ => None,
    })
}

/// 从一组列里读整数列。
fn int_of(row: &Row, column: &str) -> Option<i64> {
    row.iter().find_map(|(name, value)| match value {
        SqlValue::Integer(number) if name == column => Some(*number),
        _ => None,
    })
}

/// Go 在撤回末条后回退到的「上一条未撤回消息」摘要。
fn fallback_last_post() -> Value {
    json!({
        "id": PREVIOUS_POST_ID,
        "channelId": common::cid(CHANNEL_INDEX),
        "userId": RECEIVER_ID,
        "type": "TEXT",
        "message": "MV3-G05a previous post",
        "createAt": PREVIOUS_CREATE_AT
    })
}

/// 撤回本条即摘要（整频道全撤回）时 Go 回传的撤回态摘要。
fn revoked_last_post() -> Value {
    json!({
        "id": POST_ID,
        "temporaryId": TEMPORARY_ID,
        "channelId": common::cid(CHANNEL_INDEX),
        "userId": ACTOR_ID,
        "type": "TEXT",
        "message": MESSAGE,
        "createAt": CREATE_AT,
        "updateAt": UPDATE_AT,
        "revoke": true
    })
}

/// 摘要必须逐字段透传 Go 回退后的「上一条未撤回消息」，两张表时间戳同源。
#[test]
fn revoke_write_set_projects_the_fallback_last_post_verbatim() {
    let channel_id = common::channel_id(CHANNEL_INDEX);
    let fallback = fallback_last_post();
    let ops = message_v3_revoke_channel_ops(channel_id, ACTOR_ID, &fallback);

    for (label, row) in [
        ("channel", channel_patch(&ops)),
        ("channel_member", member_row(&ops)),
    ] {
        let encoded =
            text_of(&row, "last_post").unwrap_or_else(|| panic!("{label} 必须写 last_post"));
        let decoded: Value = serde_json::from_str(&encoded).expect("last_post 必须是合法 JSON");
        assert_eq!(
            decoded, fallback,
            "{label}.last_post 必须逐字段等于权威摘要"
        );
        assert_eq!(
            decoded.get("revoke"),
            None,
            "{label} 摘要退回到未撤回消息后不得再带 revoke 标记"
        );
        assert_eq!(
            int_of(&row, "last_post_at"),
            Some(PREVIOUS_CREATE_AT),
            "{label}.last_post_at 必须取回退后摘要的 createAt"
        );
        assert_eq!(
            int_of(&row, "last_root_post_at"),
            Some(PREVIOUS_CREATE_AT),
            "{label}.last_root_post_at 必须与 last_post_at 同源"
        );
    }
}

/// 整频道全撤回时保留撤回态本条，且 id / createAt 不被改写。
#[test]
fn revoke_write_set_keeps_the_revoked_post_when_no_earlier_post_survives() {
    let ops = message_v3_revoke_channel_ops(
        common::channel_id(CHANNEL_INDEX),
        ACTOR_ID,
        &revoked_last_post(),
    );
    let patch = channel_patch(&ops);
    let decoded: Value =
        serde_json::from_str(&text_of(&patch, "last_post").expect("必须写 last_post"))
            .expect("last_post 必须是合法 JSON");
    assert_eq!(decoded["id"], POST_ID);
    assert_eq!(decoded["revoke"], true);
    assert_eq!(
        decoded["createAt"], CREATE_AT,
        "撤回不得改写 createAt，否则时间线排序与底部锚点补偿被破坏"
    );
    assert_eq!(int_of(&patch, "last_post_at"), Some(CREATE_AT));
}

/// 撤回是 partial patch：权威缺 createAt 时不得把时间戳并成 0 清空本地既有值。
#[test]
fn revoke_write_set_omits_timestamps_when_authority_has_no_create_at() {
    let mut without_create_at = fallback_last_post();
    without_create_at
        .as_object_mut()
        .expect("fixture 是 object")
        .remove("createAt");
    let ops = message_v3_revoke_channel_ops(
        common::channel_id(CHANNEL_INDEX),
        ACTOR_ID,
        &without_create_at,
    );

    for (label, row) in [
        ("channel", channel_patch(&ops)),
        ("channel_member", member_row(&ops)),
    ] {
        assert!(
            text_of(&row, "last_post").is_some(),
            "{label} 仍必须写 last_post 本体"
        );
        assert!(
            !row.iter().any(|(name, _)| name == "last_post_at"),
            "{label} 缺 createAt 时不得写 last_post_at（0 会清空本地既有摘要时间）"
        );
        assert!(
            !row.iter().any(|(name, _)| name == "last_root_post_at"),
            "{label} 缺 createAt 时不得写 last_root_post_at"
        );
    }
}

/// 撤回不改未读、不碰已读位与发送态：这些列一列都不许出现在写集里。
#[test]
fn revoke_write_set_never_touches_unread_or_read_state() {
    let ops = message_v3_revoke_channel_ops(
        common::channel_id(CHANNEL_INDEX),
        ACTOR_ID,
        &revoked_last_post(),
    );
    let mut columns: Vec<String> = channel_patch(&ops)
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    columns.extend(member_row(&ops).into_iter().map(|(name, _)| name));

    for forbidden in [
        "unread_count",
        "unread_post_id",
        "mention_count",
        "mention_list",
        "read_bits",
        "last_read_seq",
        "send_status",
        "urgent_count",
    ] {
        assert!(
            !columns.iter().any(|name| name == forbidden),
            "撤回写集不得触碰 {forbidden}（撤回不做未读补偿）"
        );
    }
}

/// viewer 本地行只能绑定当前登录用户，禁止把 sender 的 channel_member 行写进 receiver 库。
#[test]
fn revoke_write_set_binds_the_member_row_to_the_current_viewer_only() {
    let channel_id = common::channel_id(CHANNEL_INDEX);
    let last_post = revoked_last_post();
    let sender_row = member_row(&message_v3_revoke_channel_ops(
        channel_id, ACTOR_ID, &last_post,
    ));
    let receiver_row = member_row(&message_v3_revoke_channel_ops(
        channel_id,
        RECEIVER_ID,
        &last_post,
    ));

    assert_eq!(text_of(&sender_row, "user_id").as_deref(), Some(ACTOR_ID));
    assert_eq!(
        text_of(&receiver_row, "user_id").as_deref(),
        Some(RECEIVER_ID)
    );
    assert_ne!(
        text_of(&sender_row, "user_id"),
        text_of(&receiver_row, "user_id"),
        "两个 persona 的成员行必须各自锚在自己的 user_id 上（INV-12）"
    );
    assert_eq!(
        text_of(&sender_row, "channel_id").as_deref(),
        Some(channel_id.as_str())
    );
}

// ─── 入站（`im_revoke` Command）───────────────────────────────────────────

/// 参考包的 Command 样本必须原样反序列化为 `Tick::Command{im_revoke}`，并只出站 posts/revoke。
#[test]
fn inbound_sample_command_only_admits_posts_revoke() {
    let sample = reference_json("inbound-sample.json");
    let entry = &sample["rust_entry_inputs"][0];
    assert_eq!(entry["tick_variant"], "Tick::Command");
    assert_eq!(entry["value"]["name"], "im_revoke");
    let payload = entry["value"]["payload"].clone();
    let post_id = payload["post_id"]
        .as_str()
        .expect("参考包必须提供 post_id")
        .to_string();

    let mut harness =
        Harness::new_with_identity(&[CHANNEL_INDEX], ACTOR_ID, "company-phase1", "", "", "");
    harness.step(command_json("im_revoke", payload), CREATE_AT as u64);

    assert_eq!(
        harness.last_http_body(),
        Some(json!({ "postId": post_id })),
        "撤回 body 只能是 {{postId}}，Angular 不下发任何本地推导字段"
    );
    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit { .. })),
        "命令阶段不得 Emit：HTTP 只是 admission，撤回权威来自 WS/read-back（INV-03）"
    );
}

/// 无 server id（仅 temporaryId 的本地消息）不得撤回：fail-closed 且零 Effect。
#[test]
fn revoke_command_fails_closed_without_a_server_post_id() {
    for payload in [json!({}), json!({ "post_id": "" })] {
        let mut harness =
            Harness::new_with_identity(&[CHANNEL_INDEX], ACTOR_ID, "company-phase1", "", "", "");
        let mut sink = EffectSink::new();
        let tick = command_json("im_revoke", payload.clone());
        let result = harness.module.handle(&tick, CREATE_AT as u64, &mut sink);
        assert!(
            result.is_err(),
            "缺 / 空 post_id 必须 fail-closed，实际被接受：{payload}"
        );
        assert!(
            sink.as_slice().is_empty(),
            "fail-closed 路径负向计数必须恒为 0（不得留下半截 Http/Persist）"
        );
    }
}

// ─── 双 persona 回放 ──────────────────────────────────────────────────────

struct PersonaTrace {
    events: Vec<Value>,
    revoke_ops: Vec<StorageOp>,
}

/// 构造 Go 提交后广播的 posts_update 撤回 authority。
fn posts_update_authority(event_seq: u64) -> Value {
    json!({
        "action": "posts_update",
        "data": {
            "channelId": common::cid(CHANNEL_INDEX),
            "events": [{
                "event_seq": event_seq,
                "post": {
                    "id": POST_ID,
                    "temporaryId": TEMPORARY_ID,
                    "channelId": common::cid(CHANNEL_INDEX),
                    "userId": ACTOR_ID,
                    "type": "TEXT",
                    "message": MESSAGE,
                    "simpleMessage": MESSAGE,
                    "props": { "channel_event_seq": event_seq },
                    "viewers": ["all"],
                    "createAt": CREATE_AT,
                    "updateAt": UPDATE_AT,
                    "revoke": true
                }
            }]
        }
    })
}

/// 构造 Go 在被撤回消息命中 lastPost 时广播的 update_channel authority。
fn update_channel_authority() -> Value {
    json!({
        "action": "update_channel",
        "data": {
            "id": common::cid(CHANNEL_INDEX),
            "lastPost": revoked_last_post().to_string()
        }
    })
}

/// 构造某个 viewer 自己本地库的 channel_member 读回（未读数与摘要分视角独立）。
fn member_reply(viewer_id: &str, unread_count: i64, last_post: &Value) -> ReplyBytes {
    let row: Row = vec![
        (
            "channel_id".to_string(),
            SqlValue::Text(common::cid(CHANNEL_INDEX)),
        ),
        ("user_id".to_string(), SqlValue::Text(viewer_id.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(unread_count)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
    ];
    ReplyBytes(rows_to_reply_bytes(&[row]))
}

/// 解出当前批次的业务事件。
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

/// 取当前批次原子提交的 ops。
fn atomic_ops(effects: &[Effect]) -> Option<Vec<StorageOp>> {
    effects.iter().find_map(|effect| match effect {
        Effect::PersistAtomic { ops, .. } => Some(ops.clone()),
        _ => None,
    })
}

/// 在一个独立本地库上回放一个 persona 的撤回全链。
///
/// `readback_last_post` 模拟该 viewer **自己本地库**在撤回提交后的摘要行：Go 若已把摘要
/// 退回上一条未撤回消息，读回的就是那条；整频道全撤回时读回的才是撤回态本条。
fn run_persona(
    viewer_id: &str,
    unread_count: i64,
    issue_command: bool,
    readback_last_post: &Value,
) -> TestResult<PersonaTrace> {
    let mut harness =
        Harness::new_with_identity(&[CHANNEL_INDEX], viewer_id, "company-phase1", "", "", "");
    if issue_command {
        harness.step(
            command_json("im_revoke", json!({ "post_id": POST_ID })),
            CREATE_AT as u64,
        );
        let http_corr = correlation(harness.sink.as_slice(), "http")
            .ok_or_else(|| "im_revoke 必须出站 posts/revoke".to_string())?;
        harness.step(port_reply_ok(http_corr.raw()), CREATE_AT as u64 + 1);
        assert!(
            emitted_events(harness.sink.as_slice()).is_empty(),
            "HTTP 2xx 只是 admission，不得结算撤回终态"
        );
    }

    let authority = posts_update_authority(1);
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&authority).map_err(|error| error.to_string())?,
        ))),
        CREATE_AT as u64 + 2,
    );
    let revoke_ops = atomic_ops(harness.sink.as_slice())
        .ok_or_else(|| "撤回 authority 必须原子提交".to_string())?;
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "撤回必须先落库再发布，不得在 PersistOk 之前 Emit"
    );
    let posts_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "缺撤回原子提交 correlation".to_string())?;
    harness.step(port_reply_ok(posts_corr.raw()), CREATE_AT as u64 + 3);
    let mut events = emitted_events(harness.sink.as_slice());

    let channel_authority = update_channel_authority();
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&channel_authority).map_err(|error| error.to_string())?,
        ))),
        CREATE_AT as u64 + 4,
    );
    let channel_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "update_channel 撤回摘要必须原子提交".to_string())?;
    harness.step(port_reply_ok(channel_corr.raw()), CREATE_AT as u64 + 5);
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "撤回摘要提交后必须读回当前 viewer 的 channel_member".to_string())?;
    harness.step(
        Tick::PortReply {
            corr: readback_corr,
            outcome: PortOutcome::Ok(member_reply(viewer_id, unread_count, readback_last_post)),
        },
        CREATE_AT as u64 + 6,
    );
    events.extend(emitted_events(harness.sink.as_slice()));

    Ok(PersonaTrace { events, revoke_ops })
}

/// OD-01：撤回只发 `im:post:revoke` + `im:channel:update`，双 persona 各自独立算未读与摘要。
#[test]
fn revoke_publishes_revoke_and_channel_update_for_both_personas() -> TestResult<()> {
    // 两个 persona 用**各自独立的本地库**回放；receiver 侧的摘要已被 Go 退回上一条未撤回
    // 消息，sender 侧仍停在撤回态本条 —— 若实现从 sender 复制投影，下面的差异断言必红。
    let sender = run_persona(ACTOR_ID, 0, true, &revoked_last_post())?;
    let receiver = run_persona(RECEIVER_ID, 3, false, &fallback_last_post())?;

    for trace in [&sender, &receiver] {
        let kinds: Vec<&str> = trace
            .events
            .iter()
            .map(|event| event["event"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(
            kinds,
            ["im:post:revoke", "im:channel:update"],
            "撤回的正式实时事件必须是 im:post:revoke，不得降级为 im:post:updated（OD-01）"
        );
        assert_eq!(trace.events[0]["data"]["id"], POST_ID);
        assert_eq!(
            trace.events[0]["data"]["channelId"],
            common::cid(CHANNEL_INDEX)
        );
        assert_eq!(trace.events[0]["data"]["revoke"], true);
    }

    assert_eq!(
        sender.events[0]["data"], receiver.events[0]["data"],
        "同一条撤回事实对双端是同一份绝对态"
    );
    assert_eq!(sender.events[1]["data"]["unreadCount"], 0);
    assert_eq!(
        receiver.events[1]["data"]["unreadCount"], 3,
        "receiver 未读数必须来自自己的 channel_member 读回，不得从 sender 复制或取反推导（INV-12）"
    );
    assert_eq!(
        sender.events[1]["data"]["lastPost"]["id"], POST_ID,
        "整频道全撤回时摘要保留撤回态本条"
    );
    assert_eq!(
        receiver.events[1]["data"]["lastPost"]["id"], PREVIOUS_POST_ID,
        "Go 已回退摘要时，Helix 必须逐字段透传上一条未撤回消息，不得把被撤回原文钉死为 lastPost"
    );
    assert_eq!(
        receiver.events[1]["data"]["lastPost"].get("revoke"),
        None,
        "回退后的摘要不得再带 revoke 标记"
    );
    Ok(())
}

/// 撤回只置标记：消息行仍在，createAt / 已读位 / 发送态一律不改写。
#[test]
fn revoke_marks_the_message_without_deleting_or_rewriting_it() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID, 0, true, &revoked_last_post())?;
    let patch = sender
        .revoke_ops
        .iter()
        .find_map(|op| match op {
            StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(spec.patch.clone()),
            _ => None,
        })
        .ok_or_else(|| "撤回必须以 UPDATE 方式标记 message 行".to_string())?;

    assert_eq!(
        int_of(&patch, "revoke"),
        Some(1),
        "撤回必须把 message.revoke 置 1"
    );
    for forbidden in ["create_at", "read_bits", "send_status"] {
        assert!(
            !patch.iter().any(|(name, _)| name == forbidden),
            "撤回不得改写 {forbidden}"
        );
    }
    assert!(
        !sender
            .revoke_ops
            .iter()
            .any(|op| format!("{op:?}").contains("Delete")),
        "撤回不得物理删除消息，消息必须留在时间线上渲染「已撤回」占位"
    );
    Ok(())
}

/// 同一 event_seq 的撤回 authority 重放必须零 Effect（幂等，不重复发布）。
#[test]
fn repeated_revoke_authority_is_idempotent() -> TestResult<()> {
    let mut harness =
        Harness::new_with_identity(&[CHANNEL_INDEX], ACTOR_ID, "company-phase1", "", "", "");
    let authority = serde_json::to_vec(&posts_update_authority(1)).map_err(|e| e.to_string())?;

    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(authority.clone()))),
        CREATE_AT as u64,
    );
    let corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "首次撤回 authority 必须原子提交".to_string())?;
    harness.step(port_reply_ok(corr.raw()), CREATE_AT as u64 + 1);
    assert_eq!(
        emitted_events(harness.sink.as_slice()).len(),
        1,
        "首次撤回发布唯一 im:post:revoke"
    );

    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(authority))),
        CREATE_AT as u64 + 2,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "重复撤回必须收敛到同一绝对态：零写、零出站、零事件"
    );
    Ok(())
}

/// 参考包声明的两条出站事件名与 Helix 实际发布名必须一致（防止事件词表漂移）。
#[test]
fn outbound_reference_event_names_match_the_runtime_projection() -> TestResult<()> {
    let outbound = reference_json("outbound-contract.json");
    let declared: Vec<String> = outbound["channels"]
        .as_array()
        .ok_or_else(|| "outbound-contract.channels 必须是数组".to_string())?
        .iter()
        .filter_map(|channel| channel["rust_event"].as_str().map(str::to_string))
        .collect();
    assert_eq!(declared, ["im:post:revoke", "im:channel:update"]);

    let sender = run_persona(ACTOR_ID, 0, true, &revoked_last_post())?;
    let actual: Vec<String> = sender
        .events
        .iter()
        .filter_map(|event| event["event"].as_str().map(str::to_string))
        .collect();
    assert_eq!(actual, declared);

    // 参考包 im:channel:update 的 data 字段集合是渲染契约，不允许多字段/少字段。
    let mut keys: Vec<&String> = sender.events[1]["data"]
        .as_object()
        .ok_or_else(|| "im:channel:update data 必须是 object".to_string())?
        .keys()
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        ["channelId", "lastPost", "unreadCount"],
        "im:channel:update 只发绝对态三字段（参考包 outbound-sample）"
    );
    Ok(())
}

/// 撤回写集必须锚在真实 26 字符 channel id 上，防止跨频道误写。
#[test]
fn revoke_write_set_is_scoped_to_the_authority_channel() {
    let channel_id =
        ChannelId::from_str(&common::cid(CHANNEL_INDEX)).expect("fixture channel id 必须合法");
    let ops = message_v3_revoke_channel_ops(channel_id, ACTOR_ID, &revoked_last_post());
    let scoped = ops.iter().any(|op| match op {
        StorageOp::BatchUpdate(spec) => {
            spec.table == "channel"
                && spec.key_col == "id"
                && spec.key_vals.len() == 1
                && matches!(&spec.key_vals[0], SqlValue::Text(id) if id == &common::cid(CHANNEL_INDEX))
        }
        _ => false,
    });
    assert!(
        scoped,
        "channel 写必须以 WHERE id = <authority channel> 收敛"
    );
    assert_eq!(
        text_of(&member_row(&ops), "channel_id").as_deref(),
        Some(common::cid(CHANNEL_INDEX).as_str())
    );
}
