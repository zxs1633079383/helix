//! MV3-G07b｜发布 / 修改群公告（`im_save_announcement`）。
//!
//! 回归锚：specs/004-messagev3-helix-phase1-send-domain T116/T117。
//!
//! 断言全部指向真实实现（`helix_im::outbound::posts_announcement` +
//! `client_api::build_command` + WS `post` 入帧链）：改坏其中任一不变量，这里至少一条用例变红。
//! 参考包 `specs/003-messagev3-gate-inventory/gates/announcement/mv3-g07b/` 是 wire 形状唯一真源。
//!
//! 覆盖面：
//! - **INV-10（本 Gate 核心）**：只调用公告专用接口，全链路 **零 `posts/create`**，
//!   同一保存意图收敛到**唯一** Post（唯一 temporaryId、零 echo timer、零乐观落库）。
//! - **INV-01**：`temporaryId / createAt / simpleMessage / readBits / viewers / type` 归 Helix，
//!   客户端提交即 fail-closed。
//! - **INV-12**：sender 与 receiver 各自独立回放公告 Post，禁止复制。

use bytes::Bytes;
use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::port_codec::rows_to_reply_bytes;
use helix_core::tick::{AppCommand, InboundBytes};
use helix_core::{Correlation, EffectSink, Module, Tick};
use helix_im::client_api::build_command;
use helix_im::commands::handle_outbound;
use helix_im::module::ImConfig;
use helix_im::ImModule;
use serde_json::{json, Value};

#[path = "../../../common.rs"]
mod common;
#[path = "../../support/phase1_reference.rs"]
mod phase1_reference;

use common::{cid, port_reply_ok, port_reply_with_body, Harness};

/// Gate 权威公开命令名（`inbound-sample.json.rust_entry_inputs[0].value.name`）。
const PUBLIC_COMMAND: &str = "im_save_announcement";
/// 内核命令名（`client_api::COMMAND_ALIASES` 收敛的命名漂移）。
const CORE_COMMAND: &str = "im_announcement_save";

const REFERENCE_PATH: &str = "announcement/mv3-g07b";
const ACTOR_ID: &str = "user-actor-444";
const RECEIVER_ID: &str = "user-receiver-445";
const TEAM_ID: &str = "company-phase1";
const CONTENT: &str = "Phase 1 群公告";
const ANNOUNCEMENT_ID: &str = "announcement-001";
const API_BASE: &str = "http://h/api/cses";
const DEFAULT_BASE: &str = "http://h";

/// WS 侧公告 Post 的固定 fixture（channel 必须是真实形态 Id26，故不复用样本的展示用 id）。
const WS_CHANNEL: u64 = 0x07b;
const WS_POST_ID: &str = "srvann0000000000000000007b";
const WS_CREATE_AT: i64 = 1_700_000_000_000;

// ── fixtures ────────────────────────────────────────────────────────────────

/// Gate 权威业务意图（`inbound-sample.json`）+ 宿主注入身份。
fn save_args(announcement_id: Value, content: &str) -> Value {
    json!({
        "channel_id": "channel-phase1-001",
        "announcement_id": announcement_id,
        "content": content,
        "self_id": ACTOR_ID,
        "team_id": TEAM_ID,
        "user_name": "用户444",
    })
}

fn contract_json() -> Value {
    let gate_root = phase1_reference::reference_root().join(REFERENCE_PATH);
    phase1_reference::read_json(&gate_root.join("contract.json"))
}

fn persona(auth_user_id: &str) -> Harness {
    Harness::new_with_identity(&[WS_CHANNEL], auth_user_id, TEAM_ID, "", "", "")
}

/// 一次出站 HTTP 调用的可断言快照。
#[derive(Debug, Clone, PartialEq, Eq)]
struct HttpCall {
    method: String,
    url: String,
    body: Vec<u8>,
}

impl HttpCall {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).expect("http body must be JSON")
    }
}

fn http_calls(effects: &[Effect]) -> Vec<HttpCall> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Http { req, .. } => Some(HttpCall {
                method: req.method.clone(),
                url: req.url.clone(),
                body: req.body.as_ref().map(|b| b.to_vec()).unwrap_or_default(),
            }),
            Effect::HttpFire { req } => Some(HttpCall {
                method: req.method.clone(),
                url: req.url.clone(),
                body: req.body.as_ref().map(|b| b.to_vec()).unwrap_or_default(),
            }),
            _ => None,
        })
        .collect()
}

/// 经**公开命令边界**把一次保存意图送进 `ImModule`，返回本 step 的全部出站 HTTP。
///
/// 走 `build_command` 而不是直接拼 `AppCommand`，是为了让 alias（public → core）、
/// camelCase 归一化与 outbound 注册这三段真实链路都进入断言范围。
fn dispatch_via_public_boundary(
    harness: &mut Harness,
    args: &Value,
) -> Result<Vec<HttpCall>, String> {
    let payload = serde_json::to_vec(args).expect("args must serialize");
    let command = build_command(PUBLIC_COMMAND, &payload).map_err(|e| format!("{e:?}"))?;
    harness.sink.clear();
    harness
        .module
        .handle(&Tick::Command(command), 1_000, &mut harness.sink)
        .map_err(|error| format!("{error:?}"))?;
    Ok(http_calls(harness.sink.as_slice()))
}

/// 直接走 outbound 注册表拿单条 Http（不经 module，用于 body 形状断言）。
fn build_http(args: &Value) -> Result<HttpCall, String> {
    let payload = serde_json::to_vec(args).expect("args must serialize");
    let effects = handle_outbound(
        CORE_COMMAND,
        &payload,
        API_BASE,
        DEFAULT_BASE,
        Some("conn-1"),
        Correlation::from_raw(1),
    )
    .map_err(|e| format!("{e:?}"))?;
    let calls = http_calls(&effects);
    assert_eq!(calls.len(), 1, "公告保存恒为单条出站 HTTP");
    Ok(calls.into_iter().next().expect("single call"))
}

fn emitted(effects: &[Effect]) -> Vec<Value> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).ok(),
            _ => None,
        })
        .collect()
}

/// Go 在线广播的 ANNOUNCEMENT Post 帧（公告专用接口写入后由 WS 形成唯一 Post）。
fn announcement_ws_frame(event_seq: u64, temporary_id: &str) -> Tick {
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "channelId": cid(WS_CHANNEL),
            "event_seq": event_seq,
            "id": WS_POST_ID,
            "temporaryId": temporary_id,
            "userId": ACTOR_ID,
            "type": "ANNOUNCEMENT",
            "message": CONTENT,
            "simpleMessage": CONTENT,
            "createAt": WS_CREATE_AT,
            "viewers": ["all"],
            "readBits": "",
            "props": {
                "channel_event_seq": event_seq,
                "announcement": { "id": ANNOUNCEMENT_ID, "content": CONTENT }
            }
        },
        "broadcast": {},
        "seq": 7
    });
    Tick::Inbound(InboundBytes(Bytes::from(
        serde_json::to_vec(&payload).expect("frame must serialize"),
    )))
}

/// 抓本批次唯一 `PersistAtomic` 的写集。
fn persist_atomic_ops(effects: &[Effect]) -> Vec<StorageOp> {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { ops, .. } => Some(ops.clone()),
            _ => None,
        })
        .expect("公告 post 帧必须进入 MessageV3 原子写")
}

/// 该 persona 为**自己**派生的未读增量（`ScopedGuardedBump.bump_delta`；无 bump = 0）。
///
/// 这是 viewer-local 面的真源：Helix 按 `auth_user_id` 决定「这条公告算不算我的未读」，
/// 两个 persona 各算各的，禁止互抄（INV-12）。
fn derived_unread_bump(ops: &[StorageOp], viewer: &str) -> i64 {
    ops.iter()
        .find_map(|op| match op {
            StorageOp::ScopedGuardedBump(spec) if spec.table == "channel_member" => {
                assert!(
                    matches!(&spec.key_val, SqlValue::Text(value) if value == viewer),
                    "未读只能记在当前 persona 自己名下，实际 {:?}",
                    spec.key_val
                );
                Some(spec.bump_delta)
            }
            _ => None,
        })
        .unwrap_or(0)
}

/// 该 persona 写进 `channel_member` 的成员行 user_id（viewer-local 作用域）。
fn member_row_user_id(ops: &[StorageOp]) -> Option<String> {
    ops.iter().find_map(|op| match op {
        StorageOp::BatchUpsert(spec) if spec.table == "channel_member" => {
            spec.rows.first().and_then(|row| {
                row.iter().find_map(|(name, value)| match value {
                    SqlValue::Text(value) if name == "user_id" => Some(value.clone()),
                    _ => None,
                })
            })
        }
        _ => None,
    })
}

/// 把一帧公告 Post 推到终态，返回（该 persona 自己的原子写集，该 persona 独立派生的事件）。
///
/// 三步真实链路：apply → `PersistOk` → 当前 viewer 复合键 read-back → 终态 Emit。
/// read-back 行的 `unread_count` 直接取该 persona **自己算出来的** bump，
/// 不塞人工常数，避免把断言变成同义反复。
fn apply_announcement(
    harness: &mut Harness,
    event_seq: u64,
    temporary_id: &str,
    viewer: &str,
) -> (Vec<StorageOp>, Vec<Value>) {
    harness.step(announcement_ws_frame(event_seq, temporary_id), 1_100);
    let ops = persist_atomic_ops(harness.sink.as_slice());
    assert!(
        emitted(harness.sink.as_slice()).is_empty(),
        "durable commit 之前不得发布任何终态"
    );
    let commit_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistAtomic { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("missing PersistAtomic");

    harness.step(port_reply_ok(commit_corr), 1_101);
    assert!(
        emitted(harness.sink.as_slice()).is_empty(),
        "PersistOk 必须先发起 viewer 复合键读回，再释放终态"
    );
    let readback_corr = harness
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Persist { corr, .. } => Some(corr.raw()),
            _ => None,
        })
        .expect("PersistOk 后必须发起 channel_member 读回");

    let bump = derived_unread_bump(&ops, viewer);
    let last_post = json!({
        "id": WS_POST_ID,
        "channelId": cid(WS_CHANNEL),
        "userId": ACTOR_ID,
        "type": "ANNOUNCEMENT",
        "message": CONTENT,
        "createAt": WS_CREATE_AT,
    });
    let rows = vec![vec![
        ("channel_id".to_string(), SqlValue::Text(cid(WS_CHANNEL))),
        ("user_id".to_string(), SqlValue::Text(viewer.to_string())),
        ("unread_count".to_string(), SqlValue::Integer(bump)),
        (
            "last_post".to_string(),
            SqlValue::Text(last_post.to_string()),
        ),
    ]];
    harness.step(
        port_reply_with_body(readback_corr, rows_to_reply_bytes(&rows)),
        1_102,
    );
    (ops, emitted(harness.sink.as_slice()))
}

// ── 1. 参考包完整性（REFERENCE 等级）────────────────────────────────────────

/// 证明 MV3-G07b 的 visual 目标参考包完整且未伪装为运行时证据。
#[test]
fn reference_package_is_complete() {
    phase1_reference::assert_reference_gate("MV3-G07b", REFERENCE_PATH);
}

// ── 2. inbound sample 可反序列化为预期 Tick ─────────────────────────────────

/// 参考包声明的公开命令名必须真的能穿过 `client_api` alias 落到已注册的 outbound 内核命令。
#[test]
fn reference_inbound_sample_reaches_the_registered_announcement_command() {
    let gate_root = phase1_reference::reference_root().join(REFERENCE_PATH);
    let sample = phase1_reference::read_json(&gate_root.join("inbound-sample.json"));
    let input = &sample["rust_entry_inputs"][0]["value"];
    assert_eq!(
        sample["rust_entry_inputs"][0]["tick_variant"], "Tick::Command",
        "本 Gate 入口是 Tick::Command"
    );
    assert_eq!(input["name"], PUBLIC_COMMAND);

    let payload = serde_json::to_vec(&input["payload"]).expect("payload must serialize");
    let command = build_command(PUBLIC_COMMAND, &payload).expect("公开命令必须可构造");
    assert_eq!(
        command.name.as_ref(),
        CORE_COMMAND,
        "公开名必须经 alias 收敛到内核名"
    );
    assert!(
        helix_im::commands::is_outbound(CORE_COMMAND),
        "内核名必须落在 outbound 注册表内，否则命令会被静默丢弃"
    );

    // 参考包的三键 payload（announcement_id = null）必须被真实实现接受。
    let call = build_http(&input["payload"]).expect("参考包 payload 必须被实现接受");
    assert_eq!(call.json()["message"], CONTENT);
    assert_eq!(call.json()["props"]["announcement"]["announcementId"], "");
    assert_eq!(call.json()["props"]["announcement"]["content"], CONTENT);
}

// ── 3. 首次 Effect 顺序：恰一条 Http，指向公告专用接口 ───────────────────────

/// `effects-contract.json` 首个 Effect 是 `Http`；endpoint 必须逐字等于合同登记的 Go 端点。
#[test]
fn first_effect_is_exactly_one_http_to_the_contracted_endpoint() {
    let mut harness = persona(ACTOR_ID);
    let calls = dispatch_via_public_boundary(&mut harness, &save_args(Value::Null, CONTENT))
        .expect("保存公告必须被接受");

    let effects = harness.sink.as_slice();
    assert_eq!(effects.len(), 1, "公告保存首个 Effect 集合恰为一条 Http");
    assert!(matches!(effects[0], Effect::Http { .. }));
    assert_eq!(calls[0].method, "POST");

    // 端点真源：contract.json.coverage.goEndpoint（`POST /post/announcement/save`）。
    let go_endpoint = contract_json()["coverage"]["goEndpoint"]
        .as_str()
        .expect("contract 必须登记 goEndpoint")
        .to_string();
    let path = go_endpoint
        .trim_start_matches("POST ")
        .trim_start_matches('/')
        .to_string();
    // Harness 注入的 IM 网关 base 是 `http://test`（`Harness::new_with_identity`）。
    assert_eq!(
        calls[0].url,
        format!("http://test/{path}"),
        "公告只能打合同登记的专用接口"
    );
}

// ── 4. INV-10：零 posts/create，唯一 Post ───────────────────────────────────

/// **INV-10 负向断言**：公告全链路不得出现 `posts/create`，也不得启动发送内核那套 echo timer。
///
/// 这是本 Gate 的核心：一旦有人「顺手」在公告链上补一次 `posts/create`（想让公告也进时间线），
/// 会话里就会出现两条 Post —— 一条来自公告接口的 WS 回声，一条来自 posts/create。
#[test]
fn announcement_save_never_calls_posts_create() {
    let mut harness = persona(ACTOR_ID);
    // 新建 + 编辑 + 重复保存三种形态各跑一遍，任一形态漏出 posts/create 都要红。
    let intents = [
        save_args(Value::Null, CONTENT),
        save_args(json!(ANNOUNCEMENT_ID), CONTENT),
        save_args(json!(ANNOUNCEMENT_ID), CONTENT),
    ];
    for intent in &intents {
        let calls = dispatch_via_public_boundary(&mut harness, intent).expect("保存必须被接受");
        assert_eq!(calls.len(), 1, "一次保存意图只允许一次出站调用");
        assert!(
            calls[0].url.ends_with("/post/announcement/save"),
            "唯一出站调用必须是公告专用接口，实际 {}",
            calls[0].url
        );
        assert!(
            !calls[0].url.contains("posts/create"),
            "INV-10 违反：公告链调用了 {}",
            calls[0].url
        );

        for effect in harness.sink.as_slice() {
            // 发送内核的 15s echo timer 只属于 posts/create；公告链上出现即说明走了发送内核。
            assert!(
                !matches!(effect, Effect::ScheduleTimer { .. }),
                "公告链不得启动 posts/create 的 echo timer"
            );
            // Http-first：公告没有乐观落库，Persist 只能出现在 HttpReply continuation 上。
            assert!(
                !matches!(
                    effect,
                    Effect::Persist { .. }
                        | Effect::PersistAtomic { .. }
                        | Effect::PersistFire { .. }
                ),
                "公告链首个 Effect 集合不得含乐观落库（effects-contract：Http 在 Persist 之前）"
            );
        }
    }
}

/// 唯一 Post 的另一半：同一保存意图重放，对账键 `temporaryId` 必须收敛到同一值。
///
/// 三次重放若铸出三个 temporaryId，服务端 `ON CONFLICT(temporary_id)` 就会落三行，
/// 「唯一 Post」当场失效。
#[test]
fn the_same_save_intent_converges_to_one_temporary_id() {
    let edit = save_args(json!(ANNOUNCEMENT_ID), CONTENT);
    let first = build_http(&edit).expect("保存必须被接受");
    let second = build_http(&edit).expect("保存必须被接受");
    assert_eq!(
        first.body, second.body,
        "同一保存意图必须产出 byte-exact 的 wire body（含同一 temporaryId）"
    );

    let temporary_id = |args: &Value| -> String {
        build_http(args).expect("保存必须被接受").json()["temporaryId"]
            .as_str()
            .expect("temporaryId 必须是字符串")
            .to_string()
    };
    let base = temporary_id(&edit);
    assert!(
        !base.is_empty(),
        "temporaryId 不得为空——否则无法对账 WS 回声"
    );
    // 三个维度各自独立参与铸造：任一维度变化必须换 temporaryId（否则两条不同公告会撞成一条）。
    assert_ne!(
        base,
        temporary_id(&save_args(json!(ANNOUNCEMENT_ID), "改过的公告")),
        "内容变化必须换 temporaryId"
    );
    assert_ne!(
        base,
        temporary_id(&save_args(Value::Null, CONTENT)),
        "新建与编辑必须是两个不同的对账键"
    );
    let mut other_channel = save_args(json!(ANNOUNCEMENT_ID), CONTENT);
    other_channel["channel_id"] = json!("channel-phase1-002");
    assert_ne!(
        base,
        temporary_id(&other_channel),
        "跨频道必须换 temporaryId"
    );
}

// ── 5. wire body 形状 ───────────────────────────────────────────────────────

/// wire body 逐字段对齐 `contract.json.inbound.params`：常量项、身份项与 props 三段都不许漂移。
#[test]
fn wire_body_is_the_full_announcement_post_shape() {
    let body = build_http(&save_args(json!(ANNOUNCEMENT_ID), CONTENT))
        .expect("保存必须被接受")
        .json();

    // 常量项（contract.json 明确写死的值）。
    assert_eq!(body["type"], "ANNOUNCEMENT");
    assert_eq!(
        body["viewers"],
        json!(["all"]),
        "公告对全群可见，不支持定向"
    );
    assert_eq!(body["readBits"], "", "公告不走消息位图已读");
    assert_eq!(body["topicId"], "");
    // Helix 铸造项。
    assert_eq!(body["message"], CONTENT);
    assert_eq!(
        body["simpleMessage"], CONTENT,
        "摘要由 Helix 派生，不由客户端提交"
    );
    assert!(body["temporaryId"]
        .as_str()
        .is_some_and(|id| !id.is_empty()));
    // 身份项。
    assert_eq!(body["channelId"], "channel-phase1-001");
    assert_eq!(body["userId"], ACTOR_ID);
    assert_eq!(body["teamId"], TEAM_ID);
    assert!(body.get("userSnapshot").is_none());
    assert!(body.get("userSnapshot").is_none());
    // 公告体。
    assert_eq!(
        body["props"]["announcement"]["announcementId"],
        ANNOUNCEMENT_ID
    );
    assert_eq!(body["props"]["announcement"]["content"], CONTENT);

    // wire 必须全 camelCase：任何 snake_case 顶层键都是入参泄漏。
    for key in body.as_object().expect("body 必须是对象").keys() {
        assert!(
            !key.contains('_'),
            "wire body 不得出现 snake_case 键 {key}（入参泄漏）"
        );
    }
}

/// 新建 vs 编辑的唯一依据是 `announcement_id`；缺省 / null / 空串三种写法都必须归一为新建。
#[test]
fn new_and_edit_are_distinguished_only_by_announcement_id() {
    let announcement_id_of = |args: &Value| -> Value {
        build_http(args).expect("保存必须被接受").json()["props"]["announcement"]["announcementId"]
            .clone()
    };

    let mut absent = save_args(Value::Null, CONTENT);
    absent
        .as_object_mut()
        .expect("args 必须是对象")
        .remove("announcement_id");
    assert_eq!(announcement_id_of(&absent), json!(""), "缺省 = 新建");
    assert_eq!(
        announcement_id_of(&save_args(Value::Null, CONTENT)),
        json!(""),
        "null = 新建（参考包新建样本即 null）"
    );
    assert_eq!(
        announcement_id_of(&save_args(json!(""), CONTENT)),
        json!(""),
        "空串 = 新建"
    );
    assert_eq!(
        announcement_id_of(&save_args(json!(ANNOUNCEMENT_ID), CONTENT)),
        json!(ANNOUNCEMENT_ID),
        "非空 = 编辑同一条公告"
    );

    // 新建与编辑走同一 endpoint（`openDecisions` 已按单端点承担新建 + 编辑落地）。
    assert_eq!(
        build_http(&absent).expect("保存必须被接受").url,
        build_http(&save_args(json!(ANNOUNCEMENT_ID), CONTENT))
            .expect("保存必须被接受")
            .url
    );
}

// ── 6. 错误与越权输入 fail-closed ───────────────────────────────────────────

/// INV-01 越权字段、未知字段与缺必填一律 fail-closed，且**零 Effect**（不得半途发出 HTTP）。
#[test]
fn client_minted_and_unknown_inputs_fail_closed() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "客户端自造 temporaryId",
            json!({"channel_id": "c1", "content": CONTENT, "temporary_id": "tmp-1"}),
        ),
        (
            "客户端自造 readBits",
            json!({"channel_id": "c1", "content": CONTENT, "read_bits": "11"}),
        ),
        (
            "客户端自造 simpleMessage",
            json!({"channel_id": "c1", "content": CONTENT, "simple_message": "摘要"}),
        ),
        (
            "客户端自造 createAt",
            json!({"channel_id": "c1", "content": CONTENT, "create_at": 1}),
        ),
        (
            "客户端自造 viewers（定向公告）",
            json!({"channel_id": "c1", "content": CONTENT, "viewers": [ACTOR_ID]}),
        ),
        (
            "客户端自造 type",
            json!({"channel_id": "c1", "content": CONTENT, "type": "TEXT"}),
        ),
        (
            "整包透传旧 post 对象",
            json!({"channel_id": "c1", "content": CONTENT, "post": {"message": CONTENT}}),
        ),
        (
            "未知字段",
            json!({"channel_id": "c1", "content": CONTENT, "notice_permission": 1}),
        ),
        ("缺 channel_id", json!({"content": CONTENT})),
        (
            "空 channel_id",
            json!({"channel_id": "", "content": CONTENT}),
        ),
        ("缺 content", json!({"channel_id": "c1"})),
        ("空 content", json!({"channel_id": "c1", "content": ""})),
        (
            "announcement_id 类型错",
            json!({"channel_id": "c1", "content": CONTENT, "announcement_id": 7}),
        ),
        (
            "self_id 类型错",
            json!({"channel_id": "c1", "content": CONTENT, "self_id": 7}),
        ),
    ];

    for (name, args) in cases {
        assert!(build_http(&args).is_err(), "{name} 必须 fail-closed");

        // 模块入口同样必须拒绝，且不得留下任何 Effect（半途发出的 HTTP 无法回滚）。
        let mut harness = persona(ACTOR_ID);
        let result = dispatch_via_public_boundary(&mut harness, &args);
        assert!(result.is_err(), "{name} 必须在模块入口被拒");
        assert!(
            harness.sink.as_slice().is_empty(),
            "{name} 被拒后不得留下任何 Effect"
        );
    }

    // 非对象 payload。
    let payload = serde_json::to_vec(&json!(["not-an-object"])).expect("serialize");
    assert!(handle_outbound(
        CORE_COMMAND,
        &payload,
        API_BASE,
        DEFAULT_BASE,
        None,
        Correlation::from_raw(1)
    )
    .is_err());
}

// ── 7. INV-12：sender / receiver 各自独立投影 ──────────────────────────────

/// 公告 Post 到达后，两个 persona 各自独立回放，viewer-local 面**不得**互抄。
///
/// 发布者自己不该因为自己的公告涨未读；接收者必须涨。两侧都要拿到同一份绝对公告体。
#[test]
fn sender_and_receiver_project_the_announcement_independently() {
    let temporary_id = build_http(&save_args(json!(ANNOUNCEMENT_ID), CONTENT))
        .expect("保存必须被接受")
        .json()["temporaryId"]
        .as_str()
        .expect("temporaryId 必须是字符串")
        .to_string();

    let mut sender = persona(ACTOR_ID);
    let (sender_ops, sender_events) = apply_announcement(&mut sender, 1, &temporary_id, ACTOR_ID);
    let mut receiver = persona(RECEIVER_ID);
    let (receiver_ops, receiver_events) =
        apply_announcement(&mut receiver, 1, &temporary_id, RECEIVER_ID);

    // ① 写集：成员行只写当前 persona 自己（作用域不串号）。
    assert_eq!(member_row_user_id(&sender_ops).as_deref(), Some(ACTOR_ID));
    assert_eq!(
        member_row_user_id(&receiver_ops).as_deref(),
        Some(RECEIVER_ID)
    );

    // ② viewer-local 未读：发布者自己不涨，接收者涨（INV-12 禁止两侧复制）。
    assert_eq!(
        derived_unread_bump(&sender_ops, ACTOR_ID),
        0,
        "公告发布者不得为自己发布的公告涨未读"
    );
    assert_eq!(
        derived_unread_bump(&receiver_ops, RECEIVER_ID),
        1,
        "接收者必须独立派生出自己的未读（不得复制 sender 的 0）"
    );

    // ③ 终态事件：两侧各自释放 im:post:received（outbound-contract 唯一出站通道）。
    let received = |events: &[Value]| -> Value {
        events
            .iter()
            .find(|event| event["event"] == "im:post:received")
            .cloned()
            .expect("公告 Post 必须产出 im:post:received")
    };
    let sender_post = received(&sender_events);
    let receiver_post = received(&receiver_events);

    // 绝对面：两侧看到同一条公告（同 id、同类型、同公告体）。
    for post in [&sender_post, &receiver_post] {
        assert_eq!(post["data"]["id"], WS_POST_ID);
        assert_eq!(post["data"]["type"], "ANNOUNCEMENT");
        assert!(
            serde_json::to_string(&post["data"])
                .expect("data must serialize")
                .contains(ANNOUNCEMENT_ID),
            "公告体必须原样带到双侧投影"
        );
    }

    // ④ 会话级绝对态同样分视角：未读数一路带到 im:channel:update。
    let unread_event = |events: &[Value]| -> i64 {
        events
            .iter()
            .find(|event| event["event"] == "im:channel:update")
            .and_then(|event| event["data"]["unreadCount"].as_i64())
            .expect("终态必须含会话绝对态")
    };
    assert_eq!(unread_event(&sender_events), 0);
    assert_eq!(unread_event(&receiver_events), 1);
}

/// 同一条公告 Post 重复到达必须完全幂等——重复 WS 帧不得把一条公告变成两条。
#[test]
fn duplicate_announcement_frame_is_idempotent() {
    let mut receiver = persona(RECEIVER_ID);
    let (_, events) = apply_announcement(&mut receiver, 1, "ann-fixture", RECEIVER_ID);
    assert!(
        events
            .iter()
            .any(|event| event["event"] == "im:post:received"),
        "首帧必须产出投影"
    );

    receiver.step(announcement_ws_frame(1, "ann-fixture"), 1_200);
    assert!(
        receiver.sink.as_slice().is_empty(),
        "同一 event_seq 重放必须零 Effect（唯一 Post）"
    );
}

// ── 8. PC / Mobile byte-exact ───────────────────────────────────────────────

/// 同一意图在两个独立 `ImModule` 实例上产出 byte-exact 的出站请求（三端一致）。
#[test]
fn pc_and_mobile_requests_are_byte_exact() {
    let request_bytes = |auth_user_id: &str| -> (String, Vec<u8>) {
        let config = ImConfig {
            ws_url: "ws://test".to_string(),
            api_base_url: API_BASE.to_string(),
            default_api_base_url: DEFAULT_BASE.to_string(),
            auth_user_id: auth_user_id.to_string(),
            ..Default::default()
        };
        let mut module = ImModule::new(config);
        let mut sink = EffectSink::new();
        let payload =
            serde_json::to_vec(&save_args(json!(ANNOUNCEMENT_ID), CONTENT)).expect("serialize");
        let command = build_command(PUBLIC_COMMAND, &payload).expect("命令必须可构造");
        module
            .handle(&Tick::Command(command), 1_000, &mut sink)
            .expect("保存必须被接受");
        let call = http_calls(sink.as_slice())
            .into_iter()
            .next()
            .expect("必须产出一条 HTTP");
        (call.url, call.body)
    };

    assert_eq!(request_bytes("pc-user"), request_bytes("mobile-user"));
}

// ── 9. Strict Matrix：命令名与出站通道登记一致 ─────────────────────────────

/// 参考包登记的出站通道必须与 capabilities 词表一致，且本 Gate 不得新增事件 kind。
#[test]
fn outbound_channel_is_registered_and_no_new_event_kind_is_introduced() {
    let gate_root = phase1_reference::reference_root().join(REFERENCE_PATH);
    let outbound = phase1_reference::read_json(&gate_root.join("outbound-contract.json"));
    let channels = outbound["channels"]
        .as_array()
        .expect("outbound-contract 必须登记 channels");
    assert_eq!(channels.len(), 1, "本 Gate 只有一个出站通道");
    let rust_event = channels[0]["rust_event"]
        .as_str()
        .expect("channels[0].rust_event 必须是字符串");
    assert_eq!(rust_event, "im:post:received");
    assert!(
        helix_im::client_api::PUBLIC_EVENTS.contains(&rust_event),
        "出站事件必须已在 capabilities 词表内（本 Phase 禁止新增 kind）"
    );

    // 公开命令名是 Gate 权威；alias 表必须真的登记了它。
    assert_eq!(
        helix_im::client_api::command_alias(PUBLIC_COMMAND),
        Some(CORE_COMMAND)
    );
}

/// 保底：`AppCommand` 的构造路径不会把 camelCase 公开入参漏给内核。
#[test]
fn camel_case_public_payload_is_normalized_before_reaching_the_command() {
    let payload = serde_json::to_vec(&json!({
        "channelId": "channel-phase1-001",
        "announcementId": ANNOUNCEMENT_ID,
        "content": CONTENT,
        "selfId": ACTOR_ID,
    }))
    .expect("serialize");
    let command: AppCommand = build_command(PUBLIC_COMMAND, &payload).expect("命令必须可构造");
    let normalized: Value =
        serde_json::from_slice(command.payload.as_ref()).expect("payload must be JSON");
    assert_eq!(normalized["channel_id"], "channel-phase1-001");
    assert_eq!(normalized["announcement_id"], ANNOUNCEMENT_ID);
    assert!(normalized.get("channelId").is_none());

    let body = build_http(&normalized)
        .expect("归一化后必须被实现接受")
        .json();
    assert_eq!(
        body["props"]["announcement"]["announcementId"],
        ANNOUNCEMENT_ID
    );
    assert_eq!(body["userId"], ACTOR_ID);
}
