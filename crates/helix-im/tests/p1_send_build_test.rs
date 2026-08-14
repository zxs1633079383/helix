//! P1 SendPosts 编排下沉 —— `outbound/send_build.rs` + `handle_send_message` 回放测试。
//!
//! 回归锚: G-01 / UC-1.1（前端 UC 投影清单 §2：发送文本）
//!
//! 覆盖维度（含 fail-hard 断言）：
//! - 正常 + 空消息 trim 拦截 + viewers 空补['all'] + 回复 root 合并 + userSnapshot 注入
//!   + 15s timer 超时 + echo CancelTimer；
//! - **fail-hard ①** `action="posted"` 不命中 reconcile（B3：echo 锚 `post` 非 `posted`）；
//! - **fail-hard ②** body 含 `createAt` 不含 `createdAt`（B3）；
//! - **fail-hard ③** `SystemTime|Instant::now|Utc::now` grep 必空（C3，gate 脚本侧；此处用
//!   「createAt == 注入 now_ms」证 Clock 注入而非 ambient now）。
//!
//! `send_build` 的纯函数分支（findReplyRootId 三分支 / 2000 截断 / 空补）走直接单测打满覆盖；
//! 端到端 body/对账走 Harness 回放。

mod common;
use common::*;

use helix_core::effect::{Effect, SqlValue, StorageOp};
use helix_core::tick::InboundBytes;
use helix_core::Tick;
use helix_im::outbound::send_build::{
    build_from_command, build_message_object, BuildInput, RepliedRef, RepliedSnapshot,
    UserIdentity, MAX_MESSAGE_CHARS,
};
use serde_json::json;

// ─── 直接单测 build_message_object（纯函数全分支覆盖）─────────────────────────

fn identity<'a>() -> UserIdentity<'a> {
    UserIdentity {
        user_id: "u_self",
        team_id: "team_x",
        user_name: "张立超",
        org_name: "后端开发",
        dept_name: "技术部",
    }
}

fn base_input<'a>(text: &'a str, now_ms: u64) -> BuildInput<'a> {
    BuildInput {
        channel_id: "ch_1",
        temporary_id: "tmp_1",
        msg_type: "TEXT",
        post_id: "",
        text,
        viewers: Vec::new(),
        mentions: json!([]),
        props: json!({}),
        topic_id: "",
        replied: None,
        now_ms,
        identity: identity(),
    }
}

#[test]
fn build_normal_message_full_wire_body() {
    let body = build_message_object(&base_input("hello", 1700)).expect("非空消息须产 body");
    // camelCase 字段集逐项断言（§1 真抓包对齐）。
    assert_eq!(body["channelId"], json!("ch_1"));
    assert_eq!(body["temporaryId"], json!("tmp_1"));
    assert_eq!(body["message"], json!("hello"));
    assert_eq!(body["simpleMessage"], json!("hello"));
    assert_eq!(body["type"], json!("TEXT"));
    assert_eq!(body["viewers"], json!(["all"]));
    assert_eq!(body["mentions"], json!([]));
    assert_eq!(body["props"], json!({}));
    assert_eq!(body["topicId"], json!(""));
    assert_eq!(body["revoke"], json!(false));
    assert_eq!(body["id"], json!(""));
    assert_eq!(body["userId"], json!("u_self"));
    assert_eq!(body["teamId"], json!("team_x"));
    // userSnapshot 5 字段全注入。
    assert_eq!(body["userSnapshot"]["orgName"], json!("后端开发"));
    assert_eq!(body["userSnapshot"]["deptName"], json!("技术部"));
    assert_eq!(body["userSnapshot"]["userName"], json!("张立超"));
    assert_eq!(body["userSnapshot"]["userId"], json!("u_self"));
    assert_eq!(body["userSnapshot"]["teamId"], json!("team_x"));
    // fail-hard ②：createAt == 注入 now_ms，且不含 createdAt。
    assert_eq!(body["createAt"], json!(1700u64));
    assert!(body.get("createdAt").is_none(), "禁止 createdAt（B3）");
    // 普通消息无回复三字段。
    assert!(body.get("replyId").is_none());
}

#[test]
fn build_viewers_nonempty_preserved() {
    let mut input = base_input("hi", 1);
    input.viewers = vec!["u_a".to_string(), "u_b".to_string()];
    let body = build_message_object(&input).unwrap();
    assert_eq!(
        body["viewers"],
        json!(["u_a", "u_b"]),
        "非空 viewers 原样保留"
    );
}

#[test]
fn build_empty_message_intercepted() {
    // trim 空 + props 空 → None（拦截）。
    assert!(build_message_object(&base_input("   ", 1)).is_none());
    assert!(build_message_object(&base_input("", 1)).is_none());
}

#[test]
fn build_empty_text_but_props_not_empty_passes() {
    // message 空但 props 非空对象 → 不拦截（现网 hasValidProps 放行，如纯文件消息）。
    let mut input = base_input("", 1);
    input.props = json!({ "file": { "id": "f1" } });
    let body = build_message_object(&input).expect("props 非空须放行");
    assert_eq!(body["message"], json!(""));
    assert_eq!(body["props"], json!({ "file": { "id": "f1" } }));
}

#[test]
fn build_truncates_to_2000_chars() {
    let long: String = "啊".repeat(MAX_MESSAGE_CHARS + 500);
    let body = build_message_object(&base_input(&long, 1)).unwrap();
    let msg = body["message"].as_str().unwrap();
    assert_eq!(
        msg.chars().count(),
        MAX_MESSAGE_CHARS,
        "超 2000 char 截断到 2000"
    );
    // simpleMessage = 前 50 char。
    assert_eq!(body["simpleMessage"].as_str().unwrap().chars().count(), 50);
}

#[test]
fn build_text_exactly_2000_not_truncated() {
    let exact: String = "x".repeat(MAX_MESSAGE_CHARS);
    let body = build_message_object(&base_input(&exact, 1)).unwrap();
    assert_eq!(
        body["message"].as_str().unwrap().chars().count(),
        MAX_MESSAGE_CHARS
    );
}

#[test]
fn build_simple_message_short_text_full() {
    // text < 50 char → simpleMessage == message。
    let body = build_message_object(&base_input("短", 1)).unwrap();
    assert_eq!(body["simpleMessage"], json!("短"));
}

#[test]
fn build_image_simple_message_keeps_frontend_prefix_and_caption() {
    let mut input = base_input("配图说明", 1);
    input.msg_type = "IMAGE";
    input.props = json!({
        "files": [{"name": "a.png"}],
        "imagePosition": "bottom"
    });
    let body = build_message_object(&input).unwrap();
    assert_eq!(body["simpleMessage"], json!("[图片]配图说明"));
}

#[test]
fn build_file_simple_message_uses_stable_file_name() {
    let mut input = base_input("", 1);
    input.msg_type = "FILE";
    input.props = json!({
        "file": {
            "mediaInput": {
                "fileName": "季度报告.pdf"
            }
        }
    });
    let body = build_message_object(&input).unwrap();
    assert_eq!(body["simpleMessage"], json!("[文件]季度报告.pdf"));
}

// findReplyRootId 三分支 ──────────────────────────────────────────────────────

fn input_with_reply<'a>(replied: RepliedRef<'a>) -> BuildInput<'a> {
    let mut input = base_input("re", 1);
    input.replied = Some(replied);
    input
}

#[test]
fn reply_branch1_replied_is_root() {
    // 被回复消息没回复过别人（reply_id 空）→ 它是根：replyId=replyRootId=id，firstLevel 空。
    let body = build_message_object(&input_with_reply(RepliedRef {
        id: "m_root",
        reply_id: "",
        reply_root_id: "",
        reply_first_level_id: "",
        snapshot: None,
    }))
    .unwrap();
    assert_eq!(body["replyId"], json!("m_root"));
    assert_eq!(body["replyRootId"], json!("m_root"));
    assert_eq!(body["replyFirstLevelId"], json!(""));
}

#[test]
fn reply_branch2_replied_has_first_level() {
    // 被回复消息回复过、且有 firstLevel → 继承 root/firstLevel，replyId=id。
    let body = build_message_object(&input_with_reply(RepliedRef {
        id: "m_x",
        reply_id: "m_prev",
        reply_root_id: "m_root",
        reply_first_level_id: "m_first",
        snapshot: None,
    }))
    .unwrap();
    assert_eq!(body["replyId"], json!("m_x"));
    assert_eq!(body["replyRootId"], json!("m_root"));
    assert_eq!(body["replyFirstLevelId"], json!("m_first"));
}

#[test]
fn reply_branch3_replied_directly_to_root() {
    // 被回复消息回复过、但无 firstLevel（直接回根）→ replyId=id，firstLevel=id，继承 root。
    let body = build_message_object(&input_with_reply(RepliedRef {
        id: "m_y",
        reply_id: "m_prev",
        reply_root_id: "m_root",
        reply_first_level_id: "",
        snapshot: None,
    }))
    .unwrap();
    assert_eq!(body["replyId"], json!("m_y"));
    assert_eq!(body["replyRootId"], json!("m_root"));
    assert_eq!(body["replyFirstLevelId"], json!("m_y"));
}

// build_from_command 边界分支 ─────────────────────────────────────────────────

#[test]
fn build_from_command_replied_without_id_skipped() {
    // `replied` 存在但缺 `id`（closure `?` 早返）→ 当作非回复（不写回复三字段）。
    let cmd = json!({ "text": "hi", "replied": { "reply_id": "x" } });
    let body = build_from_command(&cmd, "ch", "tmp", 1, &identity()).unwrap();
    assert!(body.get("replyId").is_none(), "replied 缺 id → 不当回复");
}

#[test]
/// 验证完整 snake_case snapshot 被转换成 Go RepliedMessage 的 camelCase wire。
fn build_from_command_adds_replied_message_snapshot() {
    let cmd = json!({
        "text": "新回复",
        "replied": {
            "id": "m_root",
            "reply_id": "",
            "reply_root_id": "",
            "reply_first_level_id": "",
            "snapshot": {
                "replied_user_id": "u_target",
                "replied_user_name": "被回复者",
                "message": "原始短消息",
                "simple_message": "原始摘要",
                "is_revoke": false,
                "type": "TEXT",
                "props": {"source": "timeline"},
                "viewers": ["all"]
            }
        }
    });
    let body = build_from_command(&cmd, "ch", "tmp", 1, &identity()).unwrap();
    assert_eq!(
        body["repliedMessage"],
        json!({
            "repliedUserId": "u_target",
            "repliedUserName": "被回复者",
            "message": "原始短消息",
            "isRevoke": false,
            "type": "TEXT",
            "props": {"source": "timeline"},
            "viewers": ["all"]
        })
    );
}

#[test]
/// 验证长文本和非文本快照沿用 message-v3 的 simpleMessage 预览规则。
fn replied_message_preview_uses_simple_message_for_long_or_non_text() {
    for (msg_type, message) in [
        (
            "TEXT",
            "这是一条超过三十个字符的原始消息，用于验证回复快照会采用摘要而非完整正文。",
        ),
        ("FILE", "季度报告.pdf"),
    ] {
        let body = build_message_object(&input_with_reply(RepliedRef {
            id: "m_target",
            reply_id: "",
            reply_root_id: "",
            reply_first_level_id: "",
            snapshot: Some(RepliedSnapshot {
                replied_user_id: "u_target".to_string(),
                replied_user_name: "被回复者".to_string(),
                message: message.to_string(),
                simple_message: "[摘要]".to_string(),
                is_revoke: false,
                msg_type: msg_type.to_string(),
                props: json!({}),
                viewers: vec!["all".to_string()],
            }),
        }))
        .unwrap();
        assert_eq!(body["repliedMessage"]["message"], json!("[摘要]"));
    }
}

#[test]
/// 验证旧壳不提供 snapshot 时只写回复关系锚，不伪造 repliedMessage。
fn build_from_command_without_snapshot_keeps_legacy_reply_contract() {
    let cmd = json!({
        "text": "兼容回复",
        "replied": {
            "id": "m_root",
            "reply_id": "",
            "reply_root_id": "",
            "reply_first_level_id": ""
        }
    });
    let body = build_from_command(&cmd, "ch", "tmp", 1, &identity()).unwrap();
    assert_eq!(body["replyId"], json!("m_root"));
    assert!(body.get("repliedMessage").is_none());
}

#[test]
fn build_from_command_defaults_when_fields_absent() {
    // 仅 text，其余字段缺省（viewers 空补 / mentions [] / props {} / topicId ""）。
    let cmd = json!({ "text": "hi" });
    let body = build_from_command(&cmd, "ch", "tmp", 9, &identity()).unwrap();
    assert_eq!(body["viewers"], json!(["all"]));
    assert_eq!(body["mentions"], json!([]));
    assert_eq!(body["props"], json!({}));
    assert_eq!(body["topicId"], json!(""));
    assert_eq!(body["createAt"], json!(9u64));
}

#[test]
fn build_from_command_empty_intercepted() {
    let cmd = json!({ "text": "" });
    assert!(build_from_command(&cmd, "ch", "tmp", 1, &identity()).is_none());
}

// ─── Round-4 P1：type/id 透传修复（UC-1.2 DOCUMENT 降级 + UC-1.6 编辑丢 id）─────────
//
// 回归锚: Round-4 P1（Round-3 审计实锤两 wire-bug）
// - UC-1.2：硬编 type="TEXT" → 文档/图片消息降级纯文本（现网 entity.Post.Type 真值丢失）。
// - UC-1.6：硬编 id="" → 编辑丢命令 post id，posts/create 永走新建而非 update。
// 真源：full-map/partials/1（create 直 decode *entity.Post，Type/ID 顶层字段）
//       + 真机curl真源.md §1（普通发消息 type="TEXT"、id=""）。

#[test]
fn build_document_type_passthrough() {
    // UC-1.2：msg_type="DOCUMENT" 透传，不被硬编降级成 "TEXT"。
    let mut input = base_input("文件名.pdf", 1);
    input.msg_type = "DOCUMENT";
    let body = build_message_object(&input).unwrap();
    assert_eq!(
        body["type"],
        json!("DOCUMENT"),
        "DOCUMENT 须透传，不得硬编降级 TEXT（UC-1.2）"
    );
}

#[test]
fn build_default_msg_type_is_text() {
    // 默认值不回归：未指定 msg_type（空串容错）→ "TEXT"。
    let mut input = base_input("hi", 1);
    input.msg_type = "";
    let body = build_message_object(&input).unwrap();
    assert_eq!(body["type"], json!("TEXT"), "空 msg_type 容错为 TEXT");
}

#[test]
fn build_edit_carries_real_post_id() {
    // UC-1.6：post_id 非空（编辑）→ body.id 携真 post id（Go 当 update）。
    let mut input = base_input("edited", 1);
    input.post_id = "post_real_123";
    let body = build_message_object(&input).unwrap();
    assert_eq!(
        body["id"],
        json!("post_real_123"),
        "编辑时 id 须携真 post id（非空 → Go update，UC-1.6）"
    );
}

#[test]
fn build_default_post_id_empty_for_new() {
    // 默认值不回归：未指定 post_id → "" = 新建（现网 self message id:''）。
    let body = build_message_object(&base_input("new", 1)).unwrap();
    assert_eq!(body["id"], json!(""), "新建消息 id 为空串");
}

#[test]
fn build_from_command_parses_type_and_id() {
    // 命令载荷零信任解析 type/id（前端 DOCUMENT/编辑显式传）。
    let cmd = json!({ "text": "doc", "type": "DOCUMENT", "id": "post_edit_1" });
    let body = build_from_command(&cmd, "ch", "tmp", 1, &identity()).unwrap();
    assert_eq!(body["type"], json!("DOCUMENT"));
    assert_eq!(body["id"], json!("post_edit_1"));
}

#[test]
fn build_from_command_type_id_default_when_absent() {
    // 缺 type/id → 默认 "TEXT" / ""（不回归普通发消息行为）。
    let cmd = json!({ "text": "hi" });
    let body = build_from_command(&cmd, "ch", "tmp", 1, &identity()).unwrap();
    assert_eq!(body["type"], json!("TEXT"));
    assert_eq!(body["id"], json!(""));
}

#[test]
fn media_message_types_are_canonicalized_for_posts_create() {
    for (input, expected) in [
        ("rich", "RICH"),
        ("RICH", "RICH"),
        ("IMAGE", "IMAGE"),
        ("file", "FILE"),
        ("FILE", "FILE"),
    ] {
        let cmd = json!({"text":"caption", "type":input, "props":{"media":true}});
        let body = build_from_command(&cmd, "ch", "tmp", 1, &identity()).unwrap();
        assert_eq!(body["type"], expected);
    }
}

// ─── 端到端：handle_send_message 经 build_message_object ─────────────────────

/// 普通文本只有在 P1 乐观行被持久化确认后才启动 posts/create。这个 Harness 没有
/// attached timeline，因此回执会直接产出 H1；带 Surface 的 Frame-first 顺序由
/// `query::local_first` 的回放测试验证。
fn acknowledge_optimistic_persist(
    harness: &mut Harness,
    initial_effects: &[EffectKind],
) -> Vec<EffectKind> {
    let p1_corr = initial_effects
        .iter()
        .find_map(|effect| match effect {
            EffectKind::Persist(corr) => Some(*corr),
            _ => None,
        })
        .expect("ordinary send starts with one P1 persist");
    harness.step(port_reply_ok(p1_corr), 1_001)
}

#[test]
fn send_emits_http_body_with_create_at_not_created_at() {
    let mut h =
        Harness::new_with_identity(&[1], "u_self", "team_x", "张立超", "后端开发", "技术部");
    let now = 1_700_000_000_123u64;
    let initial_effects = h.step(send_message_cmd(1, "tmp_send", "hello world"), now);
    let _ = acknowledge_optimistic_persist(&mut h, &initial_effects);

    let body = h.last_http_body().expect("send 须产 Http body");
    // fail-hard ②：含 createAt 不含 createdAt（B3）。
    assert_eq!(
        body["createAt"],
        json!(now),
        "createAt == 注入 now_ms（Clock 注入，C3）"
    );
    let raw = String::from_utf8(h.last_http_body_raw().unwrap()).unwrap();
    assert!(
        raw.contains("\"createAt\"") && !raw.contains("createdAt"),
        "body 须含 createAt 不含 createdAt（B3），raw={raw}"
    );
    // 字段集对齐真抓包。
    assert_eq!(body["channelId"], json!(cid(1)));
    assert_eq!(body["temporaryId"], json!("tmp_send"));
    assert_eq!(body["type"], json!("TEXT"));
    assert_eq!(body["viewers"], json!(["all"]));
    assert_eq!(body["userSnapshot"]["userName"], json!("张立超"));
    assert_eq!(body["userSnapshot"]["orgName"], json!("后端开发"));
    assert_eq!(body["userId"], json!("u_self"));
    assert_eq!(body["teamId"], json!("team_x"));
}

#[test]
fn send_empty_message_intercepted_no_effects() {
    let mut h = Harness::new(&[1]);
    // text 空白 + 无 props → 拦截，零 Effect（现网 validMessages.length===0 → return）。
    let effects = h.step(
        send_message_cmd_json(json!({
            "channel_id": cid(1),
            "temporary_id": "tmp_empty",
            "text": "   "
        })),
        1000,
    );
    assert!(
        effects.is_empty(),
        "空消息须拦截（零 Effect），got {effects:?}"
    );
    assert_eq!(h.module.pending_send_count(), 0, "空消息不登记 PendingSend");
}

#[test]
fn send_viewers_from_command_preserved() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(
        send_message_cmd_json(json!({
            "channel_id": cid(1),
            "temporary_id": "tmp_v",
            "text": "hi",
            "viewers": ["u_a", "u_b"]
        })),
        1000,
    );
    let _ = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let body = h.last_http_body().unwrap();
    assert_eq!(body["viewers"], json!(["u_a", "u_b"]));
}

#[test]
fn send_document_and_edit_id_through_handler() {
    // 端到端：handle_send_message 解析命令 type/id 透传进 body（UC-1.2 + UC-1.6）。
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(
        send_message_cmd_json(json!({
            "channel_id": cid(1),
            "temporary_id": "tmp_doc",
            "text": "文件.pdf",
            "type": "DOCUMENT",
            "id": "post_edit_42"
        })),
        1000,
    );
    let _ = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let body = h.last_http_body().unwrap();
    assert_eq!(body["type"], json!("DOCUMENT"), "DOCUMENT 透传到 wire body");
    assert_eq!(
        body["id"],
        json!("post_edit_42"),
        "编辑 post id 透传到 wire body（Go update）"
    );
}

#[test]
fn send_default_type_text_id_empty_no_regression() {
    // 默认值不回归：普通发消息（无 type/id）→ type=TEXT、id=""。
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_plain", "hi"), 1000);
    let _ = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let body = h.last_http_body().unwrap();
    assert_eq!(body["type"], json!("TEXT"));
    assert_eq!(body["id"], json!(""));
}

#[test]
/// 验证完整发送编排同时保留 reply 锚并把引用快照送入 posts/create body。
fn send_reply_merges_root_fields() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(
        send_message_cmd_json(json!({
            "channel_id": cid(1),
            "temporary_id": "tmp_r",
            "text": "re",
            "replied": {
                "id": "m_root",
                "reply_id": "",
                "reply_root_id": "",
                "reply_first_level_id": "",
                "snapshot": {
                    "replied_user_id": "u_target",
                    "replied_user_name": "被回复者",
                    "message": "原消息",
                    "simple_message": "原摘要",
                    "is_revoke": false,
                    "type": "TEXT",
                    "props": {},
                    "viewers": ["all"]
                }
            }
        })),
        1000,
    );
    let _ = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let sending = h
        .sink
        .as_slice()
        .iter()
        .find_map(|effect| match effect {
            Effect::Emit { event } => serde_json::from_slice::<serde_json::Value>(event.0.as_ref())
                .ok()
                .filter(|value| value["event"] == "im:post:sending"),
            _ => None,
        })
        .expect("回复发送持久成功后必须给 UI 一条带关系锚的乐观投影");
    let body = h.last_http_body().unwrap();
    assert_eq!(body["replyId"], json!("m_root"));
    assert_eq!(body["replyRootId"], json!("m_root"));
    assert_eq!(body["repliedMessage"]["repliedUserId"], json!("u_target"));
    assert_eq!(body["repliedMessage"]["message"], json!("原消息"));
    assert_eq!(sending["data"]["replyId"], "m_root");
    assert_eq!(sending["data"]["replyRootId"], "m_root");
    assert!(sending["data"].get("replyFirstLevelId").is_none());
}

// ─── fail-hard ①：普通发消息 echo 锚 `post` 非 `posted`（B3，真 Go WebsocketEventPost）───
//
// 真源实证（非臆造）：mattermost csesapi 发消息广播 `WebsocketEventPost`="post"
// （server/channels/app/gateway_bot.go:175 `NewWebSocketEvent(model.WebsocketEventPost, ...)`）。
// `posted`(WebsocketEventPosted) 是 core 遗留事件，**非** csesapi send 路径。
//
// ⚠️ 已知矛盾（blocker，留 main 裁决，见 SESSION/返回）：helix-im 现存 legacy `posted` handler
// 仍按 temporary_id reconcile（`ws/handlers/posted.rs` + `echo_reconcile_test`），与本契约「send
// echo 只走 post」冲突。删 `posted` reconcile 属 **P4 dead-action 清理**（02-doc P4「不要实现
// dead action posted」）+ 会破 `echo_reconcile_test`，**不在 P1 scope** → P1 不动 `posted`
// handler，仅锚定 `post` 是 send-echo 真路径。

/// fail-hard ①（P1 可断言部分）：`action="post"`（真 Go 真路径）命中 send echo reconcile。
#[test]
fn post_action_is_the_send_echo_reconcile_path() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_p", "hello"), 1000);
    let send = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let timer = send
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .expect("send arm timer");

    // 真路径：action="post" 带 temporary_id → 命中 reconcile + CancelTimer。
    let frame = json!({
        "v": 1, "action": "post",
        "data": {
            "id": sid(7), "temporary_id": "tmp_p",
            "channelId": cid(1), "event_seq": 1, "message": "hello"
        },
        "seq": 99
    });
    let effects = h.step(
        Tick::Inbound(InboundBytes(serde_json::to_vec(&frame).unwrap().into())),
        1100,
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(t) if *t == timer)),
        "action=post（真 Go WebsocketEventPost）是 send echo 真路径，须 CancelTimer，got {effects:?}"
    );
}

// ─── 15s timer 超时 + post echo CancelTimer（正路径对照）──────────────────────

#[test]
fn send_post_echo_reconciles_and_cancels_timer() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_ok", "hello"), 1000);
    let send = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let timer = send
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .unwrap();

    // 真源 echo：action="post" 带 temporary_id。
    let frame = json!({
        "v": 1, "action": "post",
        "data": {
            "id": sid(9), "temporary_id": "tmp_ok",
            "channelId": cid(1), "event_seq": 1, "message": "hello"
        },
        "seq": 99
    });
    let effects = h.step(
        Tick::Inbound(InboundBytes(serde_json::to_vec(&frame).unwrap().into())),
        1100,
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, EffectKind::CancelTimer(t) if *t == timer)),
        "post echo 须 CancelTimer 15s 超时，got {effects:?}"
    );
    // temporaryId 主键全程不变：reconcile Persist 按 temporary_id PK 覆写。
    let spec = h.sink.as_slice().iter().find_map(|e| match e {
        Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
            StorageOp::BatchUpsert(s) if s.table == "message" => Some(s.clone()),
            _ => None,
        }),
        _ => None,
    });
    let spec = spec.expect("echo 须产 message Persist");
    assert_eq!(spec.conflict_key, Some("temporary_id"));
    let tmp = spec.rows[0]
        .iter()
        .find(|(k, _)| k == "temporary_id")
        .map(|(_, v)| v.clone());
    assert!(
        matches!(&tmp, Some(SqlValue::Text(t)) if t == "tmp_ok"),
        "temporaryId 主键全程不变，got {tmp:?}"
    );
}

#[test]
/// 证明发送超时先持久化 unsend，再由回执链发布失败终态。
fn send_timeout_marks_unsend_when_no_echo() {
    let mut h = Harness::new(&[1]);
    let initial_effects = h.step(send_message_cmd(1, "tmp_t", "hello"), 1000);
    let send = acknowledge_optimistic_persist(&mut h, &initial_effects);
    let timer = send
        .iter()
        .find_map(|e| match e {
            EffectKind::ScheduleTimer(id) => Some(*id),
            _ => None,
        })
        .unwrap();
    let effects = h.step(timer_tick(timer), 16_001);
    assert!(
        effects.iter().any(|e| matches!(e, EffectKind::Persist(_))),
        "15s 超时无 echo → Persist 标 unsend，got {effects:?}"
    );
    assert!(h.sink.as_slice().iter().any(|effect| match effect {
        Effect::Persist { ops, .. } => ops.iter().any(|op| match op {
            StorageOp::BatchUpsert(spec) if spec.table == "message" =>
                spec.rows.iter().flatten().any(|(column, value)| {
                    column == "send_status"
                        && matches!(value, SqlValue::Text(value) if value == "unsend")
                }),
            _ => false,
        }),
        _ => false,
    }));
}

fn sid(n: u64) -> String {
    server_id(n).as_str().to_string()
}
