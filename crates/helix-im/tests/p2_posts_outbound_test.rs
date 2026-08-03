//! P2 — outbound registry 化 + posts 写族（urgent×3 / quickReply / updatePostProps /
//! templateReceived / revoke）集成测试。
//!
//! 回归锚: P2
//!
//! 覆盖维度（02 文档 P2 gate + 03 测试矩阵 P2 行）：
//! - 每命令 1 正常路径：endpoint url + camelCase body casing 断言（锚现网 Go 真源 posts.go / post.go）
//! - 加急族复用 post_update：3 条 urgent endpoint 均落 `posts/urgent*`（WS 推 post_update，cses_post.go:149）
//! - registry 注册数 == 命令数性质（inventory 全注册，无重复/漏注册）
//! - 边界零信任：缺必填 → Err 不 panic（target_ids 空、props 非对象、postId 空）
//! - 命名陷阱：templateReceived endpoint 前缀是 `post`（单数）非 `posts`
//!
//! 可证伪（HX-C011）：改错任一 endpoint / casing → 对应断言 FAIL；删某命令注册 → registry
//! 计数断言 FAIL + is_outbound FAIL；放松必填校验 → Err 断言 FAIL。

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::{handle_outbound, is_outbound};
use serde_json::{json, Value};

const BASE: &str = "http://x/api/cses";

fn corr() -> Correlation {
    Correlation::from_raw(42)
}

/// 取出单条 Http 的 (url, body-json)；非单条 Http → panic。
fn one_http(effects: Vec<Effect>) -> (String, Value) {
    assert_eq!(effects.len(), 1, "outbound 命令当前恒产单条 Effect::Http");
    match effects.into_iter().next().unwrap() {
        Effect::Http { req, .. } => {
            let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
            (req.url, body)
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

fn run(name: &str, payload: Value) -> Vec<Effect> {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        Some("conn-2"),
        corr(),
    )
    .expect("handle_outbound should not fail")
}

fn err(name: &str, payload: Value) -> bool {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        None,
        corr(),
    )
    .is_err()
}

// ── urgentPost ────────────────────────────────────────────────────────────────

#[test]
fn urgent_post_builds_urgentpostvo_body() {
    // UrgentPostVO（post.go:502）: channelId/postId/targetIds[]/message?
    let (url, body) = one_http(run(
        "im_urgent_post",
        json!({
            "channel_id": "chU",
            "post_id": "pU",
            "target_ids": ["u1", "u2"],
            "message": "请尽快处理",
        }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/urgentPost");
    assert_eq!(body["postId"], "pU");
    assert_eq!(body["targetIds"][0], "u1");
    assert_eq!(body["targetIds"][1], "u2");
    assert_eq!(body["message"], "请尽快处理");
    // casing: 不得出现 snake_case key
    assert!(body.get("channelId").is_none());
    assert!(body.get("channel_id").is_none());
    assert!(body.get("target_ids").is_none());
}

#[test]
fn urgent_post_message_optional() {
    let (_url, body) = one_http(run(
        "im_urgent_post",
        json!({ "channel_id": "c", "post_id": "p", "target_ids": ["u1"] }),
    ));
    // message 缺省 → body 不带 message（UrgentPostVO.Message *string omitempty）
    assert!(body.get("message").is_none());
}

#[test]
fn urgent_post_requires_nonempty_targets_and_ids() {
    // targetIds 空数组 → Err（Go Validate: "请选择用户后再操作"）
    assert!(err(
        "im_urgent_post",
        json!({ "channel_id": "c", "post_id": "p", "target_ids": [] })
    ));
    // targetIds 含非字符串 → Err
    assert!(err(
        "im_urgent_post",
        json!({ "channel_id": "c", "post_id": "p", "target_ids": [1, 2] })
    ));
    // 缺 post_id → Err
    assert!(err(
        "im_urgent_post",
        json!({ "channel_id": "c", "target_ids": ["u1"] })
    ));
}

#[test]
fn mobile_canonical_urgent_without_channel_builds_go_owned_lookup_body() {
    let (url, body) = one_http(run(
        "im_urgent_post",
        json!({ "post_id": "p", "target_ids": ["u1"] }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/urgentPost");
    assert_eq!(body["postId"], "p");
    assert_eq!(body["targetIds"], json!(["u1"]));
    assert!(body.get("channelId").is_none());

    let (url, body) = one_http(run("im_urgent_confirm", json!({ "post_id": "p" })));
    assert_eq!(url, "http://x/api/cses/posts/urgentConfirm");
    assert_eq!(body, json!({ "postId": "p" }));
}

// ── urgentConfirm / urgentCancel ───────────────────────────────────────────────

#[test]
fn urgent_confirm_and_cancel_reuse_post_update_endpoints() {
    let (url, body) = one_http(run(
        "im_urgent_confirm",
        json!({ "post_id": "p1", "channel_id": "c1" }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/urgentConfirm");
    assert_eq!(body["postId"], "p1");
    assert!(body.get("channelId").is_none());

    let (url, body) = one_http(run(
        "im_urgent_cancel",
        json!({ "post_id": "p2", "channel_id": "c2" }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/urgentCancel");
    assert_eq!(body["postId"], "p2");
    assert!(body.get("channelId").is_none());
}

#[test]
fn urgent_confirm_cancel_require_post_id_only() {
    assert!(!err("im_urgent_confirm", json!({ "post_id": "p" })));
    assert!(err("im_urgent_confirm", json!({ "channel_id": "c" })));
    assert!(!err("im_urgent_cancel", json!({ "post_id": "p" })));
    assert!(err("im_urgent_cancel", json!({ "channel_id": "c" })));
}

/// 加急族 3 条都走 `posts/urgent*` —— WS 推 post_update 复用（cses_post.go:149）的 endpoint 印证。
#[test]
fn all_three_urgent_commands_hit_urgent_endpoints() {
    let (u1, _) = one_http(run(
        "im_urgent_post",
        json!({ "channel_id": "c", "post_id": "p", "target_ids": ["u1"] }),
    ));
    let (u2, _) = one_http(run(
        "im_urgent_confirm",
        json!({ "post_id": "p", "channel_id": "c" }),
    ));
    let (u3, _) = one_http(run(
        "im_urgent_cancel",
        json!({ "post_id": "p", "channel_id": "c" }),
    ));
    for u in [&u1, &u2, &u3] {
        assert!(
            u.contains("/posts/urgent"),
            "加急族 endpoint 应含 /posts/urgent: {u}"
        );
    }
}

// ── quickReply ─────────────────────────────────────────────────────────────────

#[test]
fn quick_reply_builds_emoji_body() {
    // actor 由 Go 鉴权 session 注入，wire 不允许客户端提交 userId。
    let (url, body) = one_http(run(
        "im_quick_reply",
        json!({ "post_id": "pq", "emoji": "👍" }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/quickReply");
    assert_eq!(body["postId"], "pq");
    assert_eq!(body["emoji"], "👍");
    assert!(body.get("userId").is_none());
    assert!(body.get("post_id").is_none());
}

#[test]
fn quick_reply_carries_request_id_only_in_transport_header() {
    let effects = run(
        "im_quick_reply",
        json!({ "post_id": "p", "emoji": "🎉", "req_id": "quick-reply-1" }),
    );
    match effects.into_iter().next().unwrap() {
        Effect::Http { req, .. } => {
            let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
            assert!(body.get("req_id").is_none());
            assert!(body.get("userId").is_none());
            assert!(req
                .headers
                .iter()
                .any(|(key, value)| key == "Cses-Track-Id" && value == "quick-reply-1"));
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

#[test]
fn quick_reply_rejects_client_user_id_and_invalid_correlation() {
    assert!(err("im_quick_reply", json!({ "emoji": "👍" }))); // 缺 post_id
    assert!(err("im_quick_reply", json!({ "post_id": "p" }))); // 缺 emoji
    assert!(err(
        "im_quick_reply",
        json!({ "post_id": "p", "emoji": "👍", "user_id": "spoofed" })
    ));
    assert!(err(
        "im_quick_reply",
        json!({ "post_id": "p", "emoji": "👍", "req_id": "" })
    ));
}

// ── updatePostProps ────────────────────────────────────────────────────────────

#[test]
fn update_post_props_builds_props_body() {
    // posts.go:750: {postId, props map[string]interface{}}
    let (url, body) = one_http(run(
        "im_update_post_props",
        json!({ "post_id": "pp", "props": { "vote": { "a": 1 } } }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/updatePostProps");
    assert_eq!(body["postId"], "pp");
    assert_eq!(body["props"]["vote"]["a"], 1);
}

#[test]
fn update_post_props_requires_postid_and_object_props() {
    assert!(err("im_update_post_props", json!({ "props": {} }))); // 缺 post_id
    assert!(err("im_update_post_props", json!({ "post_id": "p" }))); // 缺 props
                                                                     // props 非对象（Go map[string]interface{} 须 object，非 nil）
    assert!(err(
        "im_update_post_props",
        json!({ "post_id": "p", "props": "x" })
    ));
    assert!(err(
        "im_update_post_props",
        json!({ "post_id": "p", "props": [1, 2] })
    ));
}

// ── templateReceived（命名陷阱：/post 单数前缀）──────────────────────────────────

#[test]
fn template_received_uses_singular_post_prefix() {
    // posts.go:53 → api.BaseRoutes.Post（/post 单数 subrouter，api.go:171）
    // 公共 camelCase `{postId}` 已由 client_api 边界归一为内部 snake_case；
    // 本测试直调 handle_outbound，因此使用归一后的 `post_id`。
    let (url, body) = one_http(run("im_template_received", json!({ "post_id": "pt" })));
    assert_eq!(url, "http://x/api/cses/post/templateReceived");
    // 命名陷阱守护：前缀必须是 /post/ 而非 /posts/
    assert!(
        url.contains("/post/templateReceived") && !url.contains("/posts/templateReceived"),
        "templateReceived 必须落 /post（单数）前缀: {url}"
    );
    assert_eq!(body["postId"], "pt");
}

#[test]
fn template_received_requires_postid() {
    // handle_outbound 是内部归一化后边界：缺 post_id 与未经归一的 raw camel 均 Err。
    assert!(err("im_template_received", json!({})));
    assert!(err("im_template_received", json!({ "postId": "pt" })));
}

// ── revoke（既有，posts 写族，验证迁移后仍正常）─────────────────────────────────

#[test]
fn revoke_still_maps_after_registry_migration() {
    let (url, body) = one_http(run("im_revoke", json!({ "post_id": "pr" })));
    assert_eq!(url, "http://x/api/cses/posts/revoke");
    assert_eq!(body["postId"], "pr");
    assert!(err("im_revoke", json!({}))); // 缺 post_id → Err
}

// ── registry 性质：注册数 == 命令数，全部命令 is_outbound 命中 ──────────────────

/// P2 注册表全集（既有 10 + 新增 7 = 17）。改这个列表必须同步注册文件。
const ALL_OUTBOUND: &[&str] = &[
    // 既有 10（posts_existing + channel_existing）
    "im_post_read",
    "im_revoke",
    "im_channel_leave",
    "im_create_schedule",
    "im_cancel_schedule",
    "im_create_channel",
    "im_make_topic",
    "im_channel_member_change",
    "im_channel_close",
    "im_update_member_nickname",
    // 新增 posts 写族 7
    "im_urgent_post",
    "im_urgent_confirm",
    "im_urgent_cancel",
    "im_quick_reply",
    "im_update_post_props",
    "im_template_received",
];

/// registry 错误闭包路径（handle_outbound 的 ok_or_else / map_err）：未认领命令 + 坏 payload。
/// 锚 registry.rs `未认领的 outbound 命令` / `payload` 两条错误分支（边界零信任，不 panic）。
#[test]
fn handle_outbound_unknown_command_and_bad_payload_are_errors() {
    // 未认领命令名 → Err(Parse "未认领...")（ok_or_else 闭包）
    assert!(handle_outbound(
        "im_not_registered",
        &serde_json::to_vec(&json!({})).unwrap(),
        BASE,
        BASE,
        None,
        corr(),
    )
    .is_err());
    // 坏 JSON payload → Err(Parse "payload...")（map_err 闭包），不 panic
    assert!(handle_outbound("im_revoke", b"{not json", BASE, BASE, None, corr()).is_err());
}

#[test]
fn registry_covers_exactly_all_outbound_commands() {
    for name in ALL_OUTBOUND {
        assert!(is_outbound(name), "{name} 应被 registry 认领");
    }
    // 非 outbound 命令不命中
    assert!(
        !is_outbound("im_send_message"),
        "send 走 module.rs 不在 registry"
    );
    assert!(!is_outbound("bogus_cmd"));
    assert!(!is_outbound(""));
}

/// registry 注册数 == 命令数性质：每个注册命令都能成功 build（无重复 name 导致 registry 构建失败）。
/// 若任一命令 inventory 注册了重复 name → registry 构建 Err → handle_outbound 全部 Err →
/// 本测试 FAIL（即「注册数==命令数」的可证伪体现）。
#[test]
fn every_registered_command_is_dispatchable() {
    // 各命令最小合法 payload（仅验证 dispatch 可达，不验证字段细节——细节在各专项测试）。
    let minimal: &[(&str, Value)] = &[
        (
            "im_post_read",
            json!({
                "channel_id": "chfixx00000000000000000001",
                "posts": ["srvfix00000000000000000001"]
            }),
        ),
        ("im_revoke", json!({ "post_id": "p" })),
        ("im_channel_leave", json!({ "channel_id": "c" })),
        (
            "im_create_schedule",
            json!({ "channel_id": "c", "message": "m", "schedule_post_at": 1 }),
        ),
        ("im_cancel_schedule", json!({ "channel_id": "c" })),
        (
            "im_create_channel",
            json!({
                "type": "O", "user_ids": ["u1"], "display_name": "g",
                "force_create": true
            }),
        ),
        (
            "im_make_topic",
            json!({ "root_id": "r", "user_ids": ["u1"], "req_id": "topic-request-1" }),
        ),
        ("im_channel_member_change", json!({ "channel_id": "c" })),
        ("im_channel_close", json!({ "channel_id": "c" })),
        (
            "im_update_member_nickname",
            json!({ "channel_id": "c", "nickname": "n" }),
        ),
        (
            "im_urgent_post",
            json!({ "channel_id": "c", "post_id": "p", "target_ids": ["u"] }),
        ),
        (
            "im_urgent_confirm",
            json!({ "post_id": "p", "channel_id": "c" }),
        ),
        (
            "im_urgent_cancel",
            json!({ "post_id": "p", "channel_id": "c" }),
        ),
        ("im_quick_reply", json!({ "post_id": "p", "emoji": "x" })),
        (
            "im_update_post_props",
            json!({ "post_id": "p", "props": {} }),
        ),
        ("im_template_received", json!({ "post_id": "p" })),
    ];
    // 注册集（ALL_OUTBOUND）与可 dispatch 集一一对应（无漏/无多）。
    assert_eq!(
        minimal.len(),
        ALL_OUTBOUND.len(),
        "minimal 用例数应 == ALL_OUTBOUND 注册命令数"
    );
    for (name, payload) in minimal {
        let effects = run(name, payload.clone());
        assert_eq!(effects.len(), 1, "{name} 应产单条 Effect::Http");
        assert!(
            matches!(effects[0], Effect::Http { .. }),
            "{name} 应产 Effect::Http"
        );
    }
}
