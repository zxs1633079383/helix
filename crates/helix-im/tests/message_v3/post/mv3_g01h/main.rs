//! MV3-G01h｜TASK_CARD（**已登记 Gap：`PARTIAL_DEFERRED_GAP`**）。
//!
//! 参考包（线型权威）：`specs/003-messagev3-gate-inventory/gates/post/mv3-g01h/`。
//!
//! # 本 Gate 的边界（spec 004 §1.2 / §2.2 / §13 SC-7）
//!
//! 本期**只**接入站：Java → Go → WS → Helix `Tick::Inbound` → 现有 TASK_CARD 渲染器。
//! **不做**客户端创建任务卡、**不做**任务查询——那是 Java 侧，不在 Phase 1 分母内。
//! 因此本文件只有两类断言：
//!
//! 1. **入站投影面**：真 Go `action=post` 的 TASK_CARD 帧经 `Tick::Inbound` 落 `Persist`，
//!    read-back 后 `Emit` `im:post:received` + `im:channel:update`，且 `props.taskCard`
//!    这条 Java/Go 权威业务载荷必须**逐字**穿过 Helix，双 persona 各自独立算未读（INV-12）。
//! 2. **Gap 未被冒充实现**：本 Gate 的 `rust_commands` 恒为空，公开 command 面**不得**出现
//!    任何任务卡创建命令；喂一条伪造的创建命令必须零 Effect（不是「悄悄成功」）。
//!
//! # 证伪（本批次只允许改本文件，故用**输入突变**做因果实验，见 REPORT）
//!
//! - 把驱动帧的 `props.taskCard` 改成别的值 → `#3` 参考样本 parity 红（证明断言读的是
//!   真实 emit 出来的投影，不是把样本原样回显）。
//! - 把驱动帧的 `userId` 改成当前 persona 自己 → `#5` 红（证明未读 +1 由「作者 != 观察者」
//!   这条真实业务分支决定）。
//! - 把驱动帧的 `viewers` 改成不含当前 persona 的白名单 → `#4` 的提交事务消失 → 红。

use bytes::Bytes;
use helix_core::effect::{Effect, Row, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{InboundBytes, PortOutcome, ReplyBytes};
use helix_core::{Correlation, Tick};
use serde_json::{json, Map, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{command_json, port_reply_ok, EffectKind, Harness};

/// 参考样本里这条 TASK_CARD 的作者（样本把它记为 `sender` 视角）。
const AUTHOR_ID: &str = "user-receiver-445";
/// 参考样本里的另一位可见成员（样本把它记为 `receiver` 视角）。
const OTHER_ID: &str = "user-actor-444";
const COMPANY_ID: &str = "company-phase1";

/// Helix 侧 id 必须是 26 字符 typed PK，样本占位符按 `substitute_reference_ids` 映射到这里。
const POST_ID: &str = "srvfix00000000000000000001";
const TEMPORARY_ID: &str = "tmpfix00000000000000000001";
/// 与 `outbound-sample.json` 的 `createAt` 逐字相同——parity 断言不做时间豁免。
const CREATE_AT: i64 = 1_700_000_000_000;

/// `im:post:received` 当前**未**投影、但参考样本已声明的字段。
///
/// 这是一条**单调收紧**的漂移登记：断言的是「缺失集 ⊆ 本表」。实现补齐任何一个字段后，
/// 它立刻进入 parity 比对集（parity 走交集），本表无需修改也不会误红；
/// 反之若实现**新**丢一个样本已声明的字段（例如某天 `props` 被剥离），缺失集越界 → 红。
const RECEIVED_DECLARED_DRIFT: [&str; 4] = ["simpleMessage", "viewers", "snapshotId", "receipt"];

/// `im:channel:update` 的 `lastPost` 当前未投影、但样本已声明的字段。同上，只允许收缩。
const LAST_POST_DECLARED_DRIFT: [&str; 1] = ["temporaryId"];

type TestResult<T> = Result<T, String>;

// ───────────────────────── 参考包读取 ─────────────────────────

/// 读取本 Gate 的某份参考文件（线型权威：`gates/README.md`）。
fn reference_file(name: &str) -> Value {
    phase1_reference::read_json(
        &phase1_reference::reference_root()
            .join("post/mv3-g01h")
            .join(name),
    )
}

/// 取 `outbound-sample.json` 中指定 `(event, viewer)` 的 `rust.data`。
fn reference_data(event: &str, viewer: &str) -> Value {
    reference_file("outbound-sample.json")["samples"]
        .as_array()
        .expect("outbound-sample.samples must be an array")
        .iter()
        .find(|entry| entry["rust"]["event"] == event && entry["viewer"] == viewer)
        .unwrap_or_else(|| panic!("outbound-sample must declare {event} for viewer {viewer}"))
        ["rust"]["data"]
        .clone()
}

/// 把样本的确定性占位 id 替换成本 Harness 的真实 Id26（其余字段逐字保留）。
fn substitute_reference_ids(value: &Value) -> Value {
    match value {
        Value::String(text) => match text.as_str() {
            "channel-phase1-001" => json!(common::cid(1)),
            "post-task-card-001" => json!(POST_ID),
            "server-mv3-g01h-001" => json!(TEMPORARY_ID),
            _ => value.clone(),
        },
        Value::Array(items) => Value::Array(items.iter().map(substitute_reference_ids).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, item)| (key.clone(), substitute_reference_ids(item)))
                .collect(),
        ),
        _ => value.clone(),
    }
}

/// 对「样本声明 ∩ 实际 emit」做 byte-level parity，并检查缺失集未越过登记的漂移。
///
/// - 交集非空且覆盖 `min_shared` 个字段，防止本断言退化成空集比对。
/// - 缺失集必须是 `declared_drift` 的子集：实现补字段自动收紧，实现丢字段立刻变红。
fn assert_declared_parity(
    label: &str,
    expected: &Value,
    actual: &Value,
    declared_drift: &[&str],
    min_shared: usize,
) {
    let expected = expected
        .as_object()
        .unwrap_or_else(|| panic!("{label}: 样本 data 必须是对象"));
    let actual = actual
        .as_object()
        .unwrap_or_else(|| panic!("{label}: 实际 emit data 必须是对象"));

    let mut shared_expected = Map::new();
    let mut shared_actual = Map::new();
    let mut missing = Vec::new();
    for (key, value) in expected {
        match actual.get(key) {
            Some(found) => {
                shared_expected.insert(key.clone(), value.clone());
                shared_actual.insert(key.clone(), found.clone());
            }
            None => missing.push(key.clone()),
        }
    }

    assert!(
        shared_expected.len() >= min_shared,
        "{label}: 样本与实现的共有字段只有 {} 个（要求 ≥ {min_shared}），\
         parity 断言已退化为近似空集；样本={expected:?} 实际={actual:?}",
        shared_expected.len()
    );
    for key in &missing {
        assert!(
            declared_drift.contains(&key.as_str()),
            "{label}: 样本声明的 `{key}` 既未被投影、也不在已登记漂移 {declared_drift:?} 中；\
             参考包是权威，新缺字段必须先移交 Helix owner 补齐"
        );
    }
    assert_eq!(
        serde_json::to_string(&Value::Object(shared_actual)).unwrap(),
        serde_json::to_string(&Value::Object(shared_expected)).unwrap(),
        "{label}: 投影与 outbound-sample.json 不一致（参考包是权威）"
    );
}

// ───────────────────────── 驱动与观测 ─────────────────────────

/// 真 Go `action=post` 的 TASK_CARD 权威帧，业务字段逐字取自参考样本。
///
/// 注意：帧里**没有** `sendStatus` / `eventSeq` 派生结果 / 未读，这三样都由 Helix 计算，
/// 因此后面的断言不可能是「把输入原样比对回来」。
fn task_card_authority() -> Value {
    let sample = substitute_reference_ids(&reference_data("im:post:received", "sender"));
    json!({
        "action": "post",
        "data": {
            "channelId": common::cid(1),
            "event_seq": 1,
            "post": {
                "id": sample["id"],
                "temporaryId": sample["temporaryId"],
                "channelId": sample["channelId"],
                "userId": sample["userId"],
                "type": sample["type"],
                "message": sample["message"],
                "simpleMessage": sample["simpleMessage"],
                "props": sample["props"],
                "viewers": sample["viewers"],
                "createAt": sample["createAt"],
                "updateAt": sample["createAt"]
            }
        }
    })
}

fn correlated_persist(effects: &[Effect], atomic: bool) -> Option<(Correlation, &[StorageOp])> {
    effects.iter().find_map(|effect| match (atomic, effect) {
        (false, Effect::Persist { corr, ops }) => Some((*corr, ops.as_slice())),
        (true, Effect::PersistAtomic { corr, ops }) => Some((*corr, ops.as_slice())),
        _ => None,
    })
}

/// 该批次是否含 viewer-local 未读 +1（未读自增的唯一形态）。
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

/// 编码当前 viewer 的 `channel_member` 绝对态读回。
fn member_reply(user_id: &str, unread_count: i64) -> ReplyBytes {
    let last_post = json!({
        "id": POST_ID,
        "userId": AUTHOR_ID,
        "message": reference_data("im:post:received", "sender")["message"],
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

struct PersonaTrace {
    events: Vec<Value>,
    unread_bumped: bool,
    /// 从 Inbound 到最终 Emit 的**全部**批次的 Effect 分类，供负向断言。
    all_effects: Vec<EffectKind>,
}

/// 以独立本地库回放一个 persona 的 TASK_CARD 入站投影。
///
/// 返回 `None` = 该 persona 根本没有提交事务（不可见 → 零原子写 → 零投影）。
fn run_persona(viewer_id: &str, initial_unread: i64) -> TestResult<Option<PersonaTrace>> {
    let mut harness = Harness::new_with_identity(&[1], viewer_id, COMPANY_ID, "", "", "");
    let inbound_bytes =
        serde_json::to_vec(&task_card_authority()).map_err(|error| error.to_string())?;
    let mut all_effects = harness.step(Tick::Inbound(InboundBytes(Bytes::from(inbound_bytes))), 1);
    assert!(
        emitted_events(harness.sink.as_slice()).is_empty(),
        "{viewer_id}: 摄取批次绝不 Emit（Emit 必须在 read-back 之后）"
    );

    let Some((persist_corr, writes)) = correlated_persist(harness.sink.as_slice(), true) else {
        return Ok(None);
    };
    let unread_bumped = has_unread_bump(writes);

    all_effects.extend(harness.step(port_reply_ok(persist_corr.raw()), 2));
    let (readback_corr, readback_ops) = correlated_persist(harness.sink.as_slice(), false)
        .ok_or_else(|| format!("{viewer_id}: missing scoped read-back"))?;
    assert!(
        readback_ops
            .iter()
            .any(|op| matches!(op, StorageOp::ScopedGet(spec) if spec.table == "channel_member")),
        "{viewer_id}: read-back must scope to channel_member"
    );

    let final_unread = if viewer_id == AUTHOR_ID {
        0
    } else {
        initial_unread + 1
    };
    all_effects.extend(harness.step(
        Tick::PortReply {
            corr: readback_corr,
            outcome: PortOutcome::Ok(member_reply(viewer_id, final_unread)),
        },
        3,
    ));
    Ok(Some(PersonaTrace {
        events: emitted_events(harness.sink.as_slice()),
        unread_bumped,
        all_effects,
    }))
}

/// 取某个 persona 的指定事件（缺失即 panic，避免断言静默跳过）。
fn event_of<'a>(trace: &'a PersonaTrace, name: &str) -> &'a Value {
    trace
        .events
        .iter()
        .find(|event| event["event"] == name)
        .unwrap_or_else(|| panic!("expected {name} in {:?}", trace.events))
}

// ───────────────────────── #1 参考包 ─────────────────────────

/// 证明 MV3-G01h 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G01h", "post/mv3-g01h");
}

// ───────────────────────── #2 Gap 形状 ─────────────────────────

/// 参考包必须把本 Gate 声明成「只接入站、零 Command」的 Gap，且六份文件 status 一致。
///
/// 这条锁的是 spec §13 SC-7：三个 Gap 必须**保持**声明的 Gap 状态，不得被悄悄改写成
/// 真实接入 Gate（例如有人给 `rust_commands` 塞一个 `im_create_task_card`）。
#[test]
fn gate_is_declared_an_inbound_only_partial_gap() {
    let inbound_contract = reference_file("inbound-contract.json");
    assert_eq!(inbound_contract["status"], "PARTIAL_DEFERRED_GAP");
    assert_eq!(inbound_contract["rust_entry"], "Tick::Inbound");
    assert_eq!(
        inbound_contract["rust_commands"],
        json!([]),
        "G01h 是入站 Gap：客户端创建链不在本期，rust_commands 必须为空"
    );

    for file in [
        "inbound-contract.json",
        "inbound-sample.json",
        "effects-contract.json",
        "outbound-contract.json",
        "outbound-sample.json",
        "provenance.json",
    ] {
        assert_eq!(
            reference_file(file)["status"],
            "PARTIAL_DEFERRED_GAP",
            "{file}: 六份文件的 Gap 状态必须一致，否则分母会被混算"
        );
    }

    // 入站样本的唯一入口就是 Tick::Inbound，且不携带任何 Angular 侧客户端意图。
    let sample = reference_file("inbound-sample.json");
    assert_eq!(sample["angular_client_intent"], Value::Null);
    let inputs = sample["rust_entry_inputs"]
        .as_array()
        .expect("rust_entry_inputs must be an array");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0]["tick_variant"], "Tick::Inbound");
    assert_eq!(inputs[0]["value"]["data"]["type"], "TASK_CARD");

    // Effect 只允许 Persist→Emit（入站投影，不走 HTTP、不走上传）。
    let effects = reference_file("effects-contract.json");
    assert_eq!(effects["allowed_effect_tags"], json!(["Persist", "Emit"]));
    let sequence = effects["reference_sequence"]
        .as_array()
        .expect("reference_sequence must be an array");
    assert_eq!(
        sequence
            .iter()
            .map(|step| step["tag"].clone())
            .collect::<Vec<_>>(),
        vec![json!("Persist"), json!("Emit")]
    );
}

// ───────────────────────── #3 入站投影 ↔ 权威样本 ─────────────────────────

/// TASK_CARD 入站投影必须与 `outbound-sample.json` 逐字节一致（交集口径 + 漂移登记）。
///
/// 非同义反复的关键：`sendStatus` / `eventSeq` / `unreadCount` **不在驱动帧里**，
/// 是 Helix 算出来的；而 `props.taskCard` 是 Java/Go 权威载荷，必须原样穿过。
#[test]
fn task_card_projection_matches_the_authoritative_sample() -> TestResult<()> {
    let author = run_persona(AUTHOR_ID, 0)?
        .ok_or_else(|| "作者 persona 必须提交这条 TASK_CARD".to_string())?;

    let received = event_of(&author, "im:post:received");
    assert_declared_parity(
        "im:post:received",
        &substitute_reference_ids(&reference_data("im:post:received", "sender")),
        &received["data"],
        &RECEIVED_DECLARED_DRIFT,
        9,
    );

    // 样本未声明、但由 Helix 计算的字段：证明本用例不是把样本原样回显。
    assert_eq!(
        received["data"]["eventSeq"], 1,
        "eventSeq 由 Helix 从帧的 event_seq 派生，驱动帧的 post 体里并没有这个字段"
    );
    assert_eq!(
        received["data"]["sendStatus"], "sent",
        "sendStatus 是 Helix 的对账结论，驱动帧不含该字段"
    );
    // INV-07：Angular 永不接触 bitmap。
    assert!(received["data"].get("readBits").is_none());

    // props.taskCard 是 Java/Go 权威业务载荷，必须逐字穿过 Helix。
    assert_eq!(
        received["data"]["props"]["taskCard"],
        reference_data("im:post:received", "sender")["props"]["taskCard"],
        "TASK_CARD 的任务卡载荷必须原样投影给现有渲染器"
    );

    let update = event_of(&author, "im:channel:update");
    let expected_update = substitute_reference_ids(&reference_data("im:channel:update", "sender"));
    assert_eq!(update["data"]["channelId"], expected_update["channelId"]);
    assert_eq!(
        update["data"]["unreadCount"], expected_update["unreadCount"],
        "作者视角未读必须绝对归零（W0）"
    );
    assert_declared_parity(
        "im:channel:update.lastPost",
        &expected_update["lastPost"],
        &update["data"]["lastPost"],
        &LAST_POST_DECLARED_DRIFT,
        2,
    );
    Ok(())
}

// ───────────────────────── #4 Effect 序列（负向） ─────────────────────────

/// 入站投影链上只允许 Persist 与 Emit：不得出现 HTTP、上传或 15 秒 echo timer。
///
/// 对应参考包 `effects-contract.allowed_effect_tags = [Persist, Emit]` 与
/// `negativeAssertions`「不得走上传链路」。TASK_CARD 是**收到**的消息，不是本端发送。
#[test]
fn task_card_ingest_never_touches_send_or_upload_transport() -> TestResult<()> {
    let trace =
        run_persona(OTHER_ID, 0)?.ok_or_else(|| "可见成员必须提交这条 TASK_CARD".to_string())?;

    assert!(
        !trace.all_effects.is_empty(),
        "对照防呆：链路必须真的产生过 Effect，否则本负向断言是空转"
    );
    for effect in &trace.all_effects {
        assert!(
            !matches!(
                effect,
                EffectKind::Http(_)
                    | EffectKind::HttpFire
                    | EffectKind::UploadFile(_)
                    | EffectKind::ScheduleTimer(_)
                    | EffectKind::Send
            ),
            "入站 TASK_CARD 投影出现了越界 Effect {effect:?}；allowed_effect_tags 只有 Persist/Emit"
        );
    }
    // 正向对照：Persist 与 Emit 两类都真实出现过。
    assert!(trace.all_effects.iter().any(|effect| matches!(
        effect,
        EffectKind::PersistAtomic(_) | EffectKind::Persist(_)
    )));
    assert!(trace
        .all_effects
        .iter()
        .any(|effect| matches!(effect, EffectKind::Emit)));
    Ok(())
}

// ───────────────────────── #5 双 persona ─────────────────────────

/// 作者与其他可见成员各自独立回放：投影同源，viewer-local 未读分别为 0 与 N+1（INV-12）。
#[test]
fn task_card_personas_compute_unread_independently() -> TestResult<()> {
    let author = run_persona(AUTHOR_ID, 7)?.ok_or_else(|| "作者必须提交".to_string())?;
    let other = run_persona(OTHER_ID, 4)?.ok_or_else(|| "可见成员必须提交".to_string())?;

    for (who, trace) in [("author", &author), ("other", &other)] {
        assert_eq!(trace.events.len(), 2, "{who}: expected received + update");
        assert_eq!(trace.events[0]["event"], "im:post:received");
        assert_eq!(trace.events[1]["event"], "im:channel:update");
        assert_eq!(trace.events[0]["data"]["id"], POST_ID);
        assert_eq!(trace.events[0]["data"]["type"], "TASK_CARD");
        assert_eq!(
            trace.events[0]["data"]["props"]["taskCard"]["id"], "task-001",
            "{who}: 两个视角看到的任务卡载荷必须同源"
        );
    }

    assert!(!author.unread_bumped, "作者自己发的任务卡不得给自己 +1");
    assert!(other.unread_bumped, "其他可见成员未读必须 +1");
    assert_eq!(
        event_of(&author, "im:channel:update")["data"]["unreadCount"],
        0
    );
    assert_eq!(
        event_of(&other, "im:channel:update")["data"]["unreadCount"],
        5,
        "接收者未读必须由自己的绝对态读回得出（4 + 1），不得从作者视角复制"
    );
    Ok(())
}

// ───────────────────────── #6 Gap 未被冒充实现 ─────────────────────────

/// 客户端创建任务卡与任务查询必须保持 Gap：公开 command 面不得出现任何任务卡创建命令。
///
/// spec §13 SC-7：三个已登记 Gap 必须保持声明状态；一旦有人接了创建链却没把 Gate 从
/// `PARTIAL_DEFERRED_GAP` 提升出去，这里会立刻变红，逼迫先改 registry 与分母。
#[test]
fn client_side_task_card_creation_stays_a_registered_gap() {
    let capabilities: Value = serde_json::from_slice(&helix_im::client_api::capabilities_json())
        .expect("capabilities JSON must parse");
    let commands = capabilities["commands"]
        .as_array()
        .expect("capabilities.commands must be an array")
        .iter()
        .map(|name| name.as_str().expect("command name must be a string"))
        .collect::<Vec<_>>();

    // 防呆：command 面本身必须非空且含已接入的发送命令，否则下面的负向断言是空转。
    assert!(
        commands.contains(&"im_send_message"),
        "对照组失败：公开 command 面应当含已接入的 im_send_message，实测 {commands:?}"
    );
    for forbidden in ["im_create_task_card", "im_send_task_card", "im_query_tasks"] {
        assert!(
            !commands.contains(&forbidden),
            "`{forbidden}` 出现在公开 command 面，但 MV3-G01h 仍登记为 PARTIAL_DEFERRED_GAP"
        );
    }
}

/// 伪造的任务卡创建命令必须零 Effect：Gap 是真的没接，而不是「悄悄成功」。
#[test]
fn forged_task_card_creation_command_produces_no_effect() {
    let mut harness = Harness::new_with_identity(&[1], AUTHOR_ID, COMPANY_ID, "", "", "");
    let effects = harness.step(
        command_json(
            "im_create_task_card",
            json!({
                "channel_id": common::cid(1),
                "task": {"id": "task-001", "title": "处理 Phase 1 任务"}
            }),
        ),
        1,
    );
    assert!(
        effects.is_empty(),
        "未接入的任务卡创建命令必须零 Effect，实测 {effects:?}"
    );

    // 对照组：同一 Harness 上已接入的命令确实会产生 Effect，证明上面的空不是 Harness 坏了。
    let baseline = harness.step(
        command_json(
            "im_send_message",
            json!({"channel_id": common::cid(1), "type": "TEXT", "text": "control"}),
        ),
        2,
    );
    assert!(
        !baseline.is_empty(),
        "对照组失败：im_send_message 必须产生 Effect"
    );
}
