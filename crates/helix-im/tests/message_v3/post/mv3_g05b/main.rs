// MV3-G05b｜emoji 快捷回复（quickReply）。
//
// 参考包（线型权威）：
//   specs/003-messagev3-gate-inventory/gates/post/mv3-g05b/*.json
// 关键不变量：
//   * INV-01/INV-05：Angular **不得**先改 reaction 对象再要求保存 —— 出站必须是 Rust 从
//     durable `message.quick_reply` 列读出的**绝对态**，不是本地增量 toggle 结果。
//   * contract.negativeAssertions：不产生新消息、不改 lastPost/unreadCount、不改 createAt；
//     同一用户对同一 emoji 重复点击幂等（userIds 里 self 只出现一次）。
//   * INV-12：sender / receiver 由两次独立 persona 回放产生，禁止复制 sender 投影。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::InboundBytes;
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::event::post::reaction_update_from_row;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, port_reply_ok, port_reply_with_body, Harness};

const CHANNEL_INDEX: u64 = 11;
const POST_ID: &str = "srvg5b00000000000000000011";
const ACTOR_ID: &str = "user-actor-444";
const RECEIVER_ID: &str = "user-receiver-445";
const OTHER_ID: &str = "user-other-446";
const EMOJI: &str = "👍";
const SECOND_EMOJI: &str = "🎉";
const CREATE_AT: i64 = 1_700_000_000_000;
const UPDATE_AT: i64 = CREATE_AT + 60;

type TestResult<T> = Result<T, String>;

// ─── 参考包 ────────────────────────────────────────────────────────────────

/// 证明 MV3-G05b 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G05b", "post/mv3-g05b");
}

/// 读取参考包内某个文件。
fn reference_json(file_name: &str) -> Value {
    let path = phase1_reference::reference_root()
        .join("post/mv3-g05b")
        .join(file_name);
    phase1_reference::read_json(&path)
}

// ─── durable row → 绝对投影（`event::post::reaction_update_from_row`）─────────

/// 构造本地 message 行；`quick_reply` 传入原始列值（可为非法形状用于负向）。
fn message_row(columns: Vec<(&str, SqlValue)>) -> Row {
    columns
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect()
}

/// 构造一条服务端权威的 quickReply 绝对态列值。
fn quick_reply_column(entries: Value) -> SqlValue {
    SqlValue::Text(entries.to_string())
}

/// 构造完整可投影的 durable 行。
fn durable_row(quick_reply: Value) -> Row {
    message_row(vec![
        ("id", SqlValue::Text(POST_ID.to_string())),
        ("channel_id", SqlValue::Text(common::cid(CHANNEL_INDEX))),
        ("event_seq", SqlValue::Integer(1)),
        ("update_at", SqlValue::Integer(UPDATE_AT)),
        ("quick_reply", quick_reply_column(quick_reply)),
    ])
}

/// 把 durable 行投影成业务事件 JSON。
fn project(row: &Row) -> TestResult<Option<Value>> {
    let Some(event) = reaction_update_from_row(row).map_err(|error| error.to_string())? else {
        return Ok(None);
    };
    let Effect::Emit { event } = event.into_effect() else {
        return Err("reaction 投影必须是 Emit effect".to_string());
    };
    serde_json::from_slice(event.0.as_ref())
        .map(Some)
        .map_err(|error| error.to_string())
}

/// 绝对态投影必须逐字段等于 durable 行，字段集合不多不少。
#[test]
fn reaction_projection_is_the_absolute_durable_state() -> TestResult<()> {
    let quick_reply = json!([
        { "emoji": EMOJI, "userIds": [ACTOR_ID], "users": [{"userId": ACTOR_ID, "userName": "用户444"}] },
        { "emoji": SECOND_EMOJI, "userIds": [RECEIVER_ID, OTHER_ID] }
    ]);
    let event = project(&durable_row(quick_reply.clone()))?
        .ok_or_else(|| "完整 durable 行必须能投影".to_string())?;

    assert_eq!(
        event,
        json!({
            "event": "im:post:updated",
            "data": {
                "id": POST_ID,
                "channelId": common::cid(CHANNEL_INDEX),
                "eventSeq": 1,
                "updateAt": UPDATE_AT,
                "quickReply": quick_reply,
            }
        }),
        "quickReply 出站必须是 Rust 算完的绝对态，字段集合与参考包一致（多一个 / 少一个都红）"
    );
    Ok(())
}

/// 每一列权威字段缺失都必须 fail-closed（返回 None），不得用默认值兜底发半截投影。
#[test]
fn reaction_projection_fails_closed_on_any_missing_authority_column() -> TestResult<()> {
    let full = durable_row(json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }]));
    for column in ["id", "channel_id", "event_seq", "update_at", "quick_reply"] {
        let partial: Row = full
            .iter()
            .filter(|(name, _)| name != column)
            .cloned()
            .collect();
        assert!(
            project(&partial)?.is_none(),
            "缺少 {column} 时不得投影出 quickReply 事件（会让 Angular 拿到半截绝对态）"
        );
    }
    Ok(())
}

/// 非数组 / 非法 JSON 的 quick_reply 一律拒绝：绝对态必须是列表形状。
#[test]
fn reaction_projection_rejects_non_array_quick_reply() -> TestResult<()> {
    for broken in [
        SqlValue::Text("{\"emoji\":\"👍\"}".to_string()),
        SqlValue::Text("\"👍\"".to_string()),
        SqlValue::Text("not-json".to_string()),
        SqlValue::Text(String::new()),
    ] {
        let row = message_row(vec![
            ("id", SqlValue::Text(POST_ID.to_string())),
            ("channel_id", SqlValue::Text(common::cid(CHANNEL_INDEX))),
            ("event_seq", SqlValue::Integer(1)),
            ("update_at", SqlValue::Integer(UPDATE_AT)),
            ("quick_reply", broken.clone()),
        ]);
        assert!(
            project(&row)?.is_none(),
            "非数组 quick_reply 必须拒绝：{broken:?}"
        );
    }
    Ok(())
}

/// 列类型不做隐式转换：字符串数字不得当 event_seq / update_at 用。
#[test]
fn reaction_projection_rejects_type_coerced_columns() -> TestResult<()> {
    for (column, coerced) in [
        ("event_seq", SqlValue::Text("1".to_string())),
        ("update_at", SqlValue::Text(UPDATE_AT.to_string())),
        ("id", SqlValue::Integer(7)),
        ("channel_id", SqlValue::Integer(11)),
    ] {
        let row: Row = durable_row(json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }]))
            .into_iter()
            .map(|(name, value)| {
                if name == column {
                    (name, coerced.clone())
                } else {
                    (name, value)
                }
            })
            .collect();
        assert!(
            project(&row)?.is_none(),
            "{column} 被写成 {coerced:?} 时必须 fail-closed，不得隐式转换"
        );
    }
    Ok(())
}

/// 投影是无状态纯函数：先后两份 durable 行各自独立，不得与上一次结果做并集。
///
/// 这是「Angular 不得先改 reaction 再保存」的 Rust 侧对偶：toggle 去重由服务端算完写进
/// durable 列，Helix 只做绝对态搬运；一旦实现引入本地合并，下面第二次断言必红。
#[test]
fn successive_reaction_states_replace_rather_than_merge() -> TestResult<()> {
    let added = project(&durable_row(
        json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }]),
    ))?
    .ok_or_else(|| "第一次贴表情必须投影".to_string())?;
    assert_eq!(
        added["data"]["quickReply"],
        json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }])
    );

    // 同一用户再点同一 emoji（toggle off）→ 服务端绝对态变空数组。
    let toggled_off = project(&durable_row(json!([])))?
        .ok_or_else(|| "空 quickReply 仍是合法绝对态".to_string())?;
    assert_eq!(
        toggled_off["data"]["quickReply"],
        json!([]),
        "toggle off 后不得把上一份 quickReply 并回来"
    );

    // 别人贴同一 emoji → 绝对态覆盖为服务端顺序，不做本地 push。
    let joined = project(&durable_row(
        json!([{ "emoji": EMOJI, "userIds": [OTHER_ID, ACTOR_ID] }]),
    ))?
    .ok_or_else(|| "多人 quickReply 必须投影".to_string())?;
    assert_eq!(
        joined["data"]["quickReply"],
        json!([{ "emoji": EMOJI, "userIds": [OTHER_ID, ACTOR_ID] }]),
        "userIds 顺序由服务端权威决定，Helix 不得重排或去重后重建"
    );
    Ok(())
}

/// 幂等：同一用户重复点同一 emoji 时服务端绝对态里 self 只出现一次，投影不得复制放大。
#[test]
fn repeated_same_user_emoji_stays_deduplicated_in_the_projection() -> TestResult<()> {
    let event = project(&durable_row(json!([
        { "emoji": EMOJI, "userIds": [ACTOR_ID], "users": [{"userId": ACTOR_ID, "userName": "用户444"}] }
    ])))?
    .ok_or_else(|| "必须投影".to_string())?;
    let entry = &event["data"]["quickReply"][0];
    let user_ids = entry["userIds"]
        .as_array()
        .ok_or_else(|| "userIds 必须是数组".to_string())?;
    assert_eq!(
        user_ids.iter().filter(|id| *id == ACTOR_ID).count(),
        1,
        "同一用户对同一 emoji 只能出现一次"
    );
    assert_eq!(
        entry["users"].as_array().map(Vec::len),
        Some(user_ids.len()),
        "users 长度必须与 userIds 恒等"
    );
    Ok(())
}

/// 快捷回复不是新消息：投影里不得出现消息体 / 会话摘要 / 未读 / 本地传输字段。
#[test]
fn reaction_projection_carries_no_message_or_channel_side_effects() -> TestResult<()> {
    let event = project(&durable_row(
        json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }]),
    ))?
    .ok_or_else(|| "必须投影".to_string())?;
    let data = event["data"]
        .as_object()
        .ok_or_else(|| "data 必须是 object".to_string())?;
    for forbidden in [
        "lastPost",
        "lastPostAt",
        "unreadCount",
        "createAt",
        "temporaryId",
        "sendStatus",
        "readBits",
        "props",
        "reactions",
        "reactionCount",
    ] {
        assert!(
            !data.contains_key(forbidden),
            "quickReply 绝对态不得携带 {forbidden}（贴表情不产生新消息、不改会话摘要与未读）"
        );
    }
    assert_eq!(
        event["event"], "im:post:updated",
        "快捷回复走 im:post:updated，不得降级/升级成其它事件名"
    );
    Ok(())
}

// ─── 入站（`im_quick_reply` Command）───────────────────────────────────────

/// 参考包的 Command 样本必须原样反序列化并只出站 `posts/quickReply {postId,emoji}`。
#[test]
fn inbound_sample_command_only_admits_posts_quick_reply() {
    let sample = reference_json("inbound-sample.json");
    let entry = &sample["rust_entry_inputs"][0];
    assert_eq!(entry["tick_variant"], "Tick::Command");
    assert_eq!(entry["value"]["name"], "im_quick_reply");
    let payload = entry["value"]["payload"].clone();
    let post_id = payload["post_id"]
        .as_str()
        .expect("参考包必须给 post_id")
        .to_string();
    let emoji = payload["emoji"]
        .as_str()
        .expect("参考包必须给 emoji")
        .to_string();

    let mut harness =
        Harness::new_with_identity(&[CHANNEL_INDEX], ACTOR_ID, "company-phase1", "", "", "");
    harness.step(command_json("im_quick_reply", payload), CREATE_AT as u64);

    assert_eq!(
        harness.last_http_body(),
        Some(json!({ "postId": post_id, "emoji": emoji })),
        "quickReply body 只能是 {{postId,emoji}}；actor 由 Go 会话注入，客户端不得下发"
    );
    assert!(
        !harness
            .sink
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit { .. })),
        "命令阶段不得 Emit：Angular 不得先改 reaction 再等保存（INV-01）"
    );
}

/// 未知 / 缺失字段一律 fail-closed，零 Effect。
#[test]
fn quick_reply_command_fails_closed_on_missing_or_unknown_fields() {
    for payload in [
        json!({ "emoji": EMOJI }),
        json!({ "post_id": POST_ID }),
        json!({ "post_id": POST_ID, "emoji": "" }),
        // Angular 侧本地推导的 reaction 结果绝不允许提交给 Helix。
        json!({ "post_id": POST_ID, "emoji": EMOJI, "quick_reply": [{"emoji": EMOJI}] }),
        json!({ "post_id": POST_ID, "emoji": EMOJI, "user_ids": [ACTOR_ID] }),
    ] {
        let mut harness =
            Harness::new_with_identity(&[CHANNEL_INDEX], ACTOR_ID, "company-phase1", "", "", "");
        let mut sink = EffectSink::new();
        let tick = command_json("im_quick_reply", payload.clone());
        let result = harness.module.handle(&tick, CREATE_AT as u64, &mut sink);
        assert!(result.is_err(), "必须 fail-closed，实际被接受：{payload}");
        assert!(
            sink.as_slice().is_empty(),
            "fail-closed 路径负向计数必须恒为 0：{payload}"
        );
    }
}

// ─── 双 persona 回放 ──────────────────────────────────────────────────────

/// 构造 Go `post_update` 广播的 quickReply 绝对态。
fn reaction_authority(event_seq: u64, quick_reply: Value) -> Value {
    json!({
        "action": "post_update",
        "data": {
            "id": POST_ID,
            "channelId": common::cid(CHANNEL_INDEX),
            "userId": RECEIVER_ID,
            "createAt": CREATE_AT,
            "updateAt": UPDATE_AT,
            "message": "Phase 1 message operation",
            "type": "TEXT",
            "props": { "channel_event_seq": event_seq },
            "quickReply": quick_reply,
            "event_seq": event_seq
        }
    })
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

struct PersonaTrace {
    events: Vec<Value>,
    reaction_patch: Row,
}

/// 在独立本地库上回放一个 persona 的快捷回复全链。
fn run_persona(viewer_id: &str, issue_command: bool) -> TestResult<PersonaTrace> {
    let quick_reply = json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }]);
    let mut harness =
        Harness::new_with_identity(&[CHANNEL_INDEX], viewer_id, "company-phase1", "", "", "");
    if issue_command {
        harness.step(
            command_json(
                "im_quick_reply",
                json!({ "post_id": POST_ID, "emoji": EMOJI }),
            ),
            CREATE_AT as u64,
        );
        let http_corr = correlation(harness.sink.as_slice(), "http")
            .ok_or_else(|| "im_quick_reply 必须出站 posts/quickReply".to_string())?;
        harness.step(port_reply_ok(http_corr.raw()), CREATE_AT as u64 + 1);
        assert!(
            emitted_events(harness.sink.as_slice()).is_empty(),
            "HTTP 2xx 只是 admission，不是 reaction 权威"
        );
    }

    let authority = reaction_authority(1, quick_reply.clone());
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&authority).map_err(|error| error.to_string())?,
        ))),
        CREATE_AT as u64 + 2,
    );
    let ops = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops.clone()),
            _ => None,
        })
        .ok_or_else(|| "reaction authority 必须原子提交".to_string())?;
    let reaction_patch = ops
        .iter()
        .find_map(|op| match op {
            StorageOp::BatchUpdate(spec) if spec.table == "message" => Some(spec.patch.clone()),
            _ => None,
        })
        .ok_or_else(|| "reaction 必须更新 message 行".to_string())?;
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "落库前不得发布 reaction 终态"
    );
    let atomic_corr = correlation(harness.sink.as_slice(), "atomic")
        .ok_or_else(|| "缺 reaction 原子提交 correlation".to_string())?;

    harness.step(port_reply_ok(atomic_corr.raw()), CREATE_AT as u64 + 3);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "PersistOk 后必须先读回 durable 行再发布绝对态"
    );
    let readback_corr = correlation(harness.sink.as_slice(), "persist")
        .ok_or_else(|| "reaction 提交后必须读回 message".to_string())?;

    let row = durable_row(quick_reply);
    harness.step(
        port_reply_with_body(
            readback_corr.raw(),
            rows_to_reply_bytes(std::slice::from_ref(&row)),
        ),
        CREATE_AT as u64 + 4,
    );
    let events = emitted_events(harness.sink.as_slice());

    // 同 event_seq 重放：幂等，零 Effect。
    harness.step(
        Tick::Inbound(InboundBytes(Bytes::from(
            serde_json::to_vec(&reaction_authority(
                1,
                json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }]),
            ))
            .map_err(|error| error.to_string())?,
        ))),
        CREATE_AT as u64 + 5,
    );
    assert!(
        harness.sink.as_slice().is_empty(),
        "重复 reaction authority 必须收敛到同一绝对态：零写、零事件"
    );

    Ok(PersonaTrace {
        events,
        reaction_patch,
    })
}

/// 双 persona 各自独立回放，收到同一份绝对 `im:post:updated`（禁止复制 sender 投影）。
#[test]
fn quick_reply_publishes_the_same_absolute_state_to_both_personas() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID, true)?;
    let receiver = run_persona(RECEIVER_ID, false)?;

    for trace in [&sender, &receiver] {
        assert_eq!(trace.events.len(), 1, "快捷回复只发一条绝对态事件");
        assert_eq!(trace.events[0]["event"], "im:post:updated");
        assert_eq!(trace.events[0]["data"]["id"], POST_ID);
        assert_eq!(
            trace.events[0]["data"]["quickReply"],
            json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }])
        );
    }
    assert_eq!(
        sender.events[0], receiver.events[0],
        "两个 persona 的 reaction 绝对态同源于各自 durable 行，投影必须一致"
    );
    Ok(())
}

/// 写集只覆盖 quick_reply 单列：不得寄生 props，也不得连带改消息体 / 时间 / 已读态。
#[test]
fn quick_reply_write_set_only_touches_the_quick_reply_column() -> TestResult<()> {
    let sender = run_persona(ACTOR_ID, true)?;
    let columns: Vec<&str> = sender
        .reaction_patch
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        columns,
        ["quick_reply"],
        "reaction 写集必须只有 quick_reply 一列（多写一列即污染消息体 / 时间线）"
    );

    let stored = sender
        .reaction_patch
        .iter()
        .find_map(|(name, value)| match value {
            SqlValue::Text(text) if name == "quick_reply" => Some(text.clone()),
            _ => None,
        })
        .ok_or_else(|| "quick_reply 必须是文本列".to_string())?;
    assert_eq!(
        serde_json::from_str::<Value>(&stored).map_err(|error| error.to_string())?,
        json!([{ "emoji": EMOJI, "userIds": [ACTOR_ID] }]),
        "落库的必须是服务端绝对态本身"
    );
    Ok(())
}

/// 参考包声明的出站事件名与 Helix 实际发布名一致（防事件词表漂移）。
#[test]
fn outbound_reference_event_name_matches_the_runtime_projection() -> TestResult<()> {
    let outbound = reference_json("outbound-contract.json");
    let declared: Vec<String> = outbound["channels"]
        .as_array()
        .ok_or_else(|| "outbound-contract.channels 必须是数组".to_string())?
        .iter()
        .filter_map(|channel| channel["rust_event"].as_str().map(str::to_string))
        .collect();
    assert_eq!(declared, ["im:post:updated"]);

    let sender = run_persona(ACTOR_ID, true)?;
    let actual: Vec<String> = sender
        .events
        .iter()
        .filter_map(|event| event["event"].as_str().map(str::to_string))
        .collect();
    assert_eq!(actual, declared);

    // 参考样本的 sender / receiver quickReply 必须是同一份绝对态（禁止分视角伪造）。
    let sample = reference_json("outbound-sample.json");
    let samples = sample["samples"]
        .as_array()
        .ok_or_else(|| "outbound-sample.samples 必须是数组".to_string())?;
    let sender_reply = samples
        .iter()
        .find(|item| item["viewer"] == "sender")
        .map(|item| item["rust"]["data"]["quickReply"].clone())
        .ok_or_else(|| "参考包缺 sender 样本".to_string())?;
    let receiver_reply = samples
        .iter()
        .find(|item| item["viewer"] == "receiver")
        .map(|item| item["rust"]["data"]["quickReply"].clone())
        .ok_or_else(|| "参考包缺 receiver 样本（G05b 是双 persona Gate）".to_string())?;
    assert_eq!(sender_reply, receiver_reply);
    Ok(())
}
