//! CP — `im_create_posts`（posts/createPosts 批量转发）补齐 outbound 88→89 集成测试。
//!
//! 回归锚: CP（D3 createPosts body 实证）
//!
//! 证据链（frontend-source-derived = 等价抓包，见 `outbound/posts_relay.rs` 头注释）：
//! - 前端 `message.service.ts::sendRelayMessages(params)` 逐字透传 `params` 到 `/posts/createPosts`。
//! - 两个真实 caller 均 camelCase：`chat-content-base.component.ts:967` `{ posts, channelIds }`；
//!   `messageContainerShared.service.ts:245` `{ posts: [post], channelIds: [channelId] }`。
//! - Go `csesapi/posts.go::createPosts` 解构体 `{ Posts *entity.Posts; ChannelIds []string }`
//!   **两字段无 json tag** → case-insensitive 解码，camelCase 与 PascalCase 都能解；**真实 wire
//!   = 前端构造的 camelCase**（PascalCase 是 Go 字段名，从未上线）。
//!
//! 覆盖维度：
//! - 正常路径：endpoint `posts/createPosts` + wire body **camelCase** `posts`/`channelIds`。
//! - casing fail-hard：body 含 `"posts"`/`"channelIds"`，**不含** PascalCase `"Posts"`/`"ChannelIds"`。
//! - 真 Go fixture 比对：body 顶层 key 集 == Go 解构体期望（case-insensitive 下 camelCase 形态）。
//! - 边界零信任：缺/空 posts、缺/空 channel_ids、非数组 → Err 不 panic。
//! - registry 性质：is_outbound 命中 + 注册总数 == 88（CP 补齐后；send 不在内）。
//!
//! 可证伪（HX-C011）：把 body 拼成 PascalCase → casing 断言 FAIL；改 endpoint → url 断言 FAIL；
//! 删注册 → is_outbound + 计数断言 FAIL；放松必填校验 → Err 断言 FAIL。

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::{handle_outbound, is_outbound, outbound_command_count};
use serde_json::{json, Value};

const BASE: &str = "http://x/api/cses";

fn corr() -> Correlation {
    Correlation::from_raw(89)
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

fn run(payload: Value) -> Vec<Effect> {
    handle_outbound(
        "im_create_posts",
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        Some("conn-cp"),
        corr(),
    )
    .expect("handle_outbound should not fail")
}

fn err(payload: Value) -> bool {
    handle_outbound(
        "im_create_posts",
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        None,
        corr(),
    )
    .is_err()
}

/// 一个最小有效入参（前端 UI 字段：posts 数组 + channel_ids 数组）。
fn minimal_args() -> Value {
    json!({
        "posts": [{ "message": "forwarded", "temporaryId": "t1" }],
        "channel_ids": ["chA", "chB"],
    })
}

/// 正常路径：endpoint = posts/createPosts，body camelCase 透传 posts + channelIds。
#[test]
fn create_posts_endpoint_and_wire_shape() {
    let (url, body) = one_http(run(minimal_args()));
    assert_eq!(url, format!("{BASE}/posts/createPosts"));

    // wire body camelCase（真实 wire = 前端构造形态）。
    assert_eq!(
        body["channelIds"],
        json!(["chA", "chB"]),
        "channelIds 须 camelCase 透传目标 channel 数组"
    );
    let posts = body["posts"].as_array().expect("posts 应为数组");
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0]["message"], json!("forwarded"));
    assert_eq!(posts[0]["temporaryId"], json!("t1"));
}

/// casing fail-hard：实证结论 = camelCase。body **含** camelCase key、**绝不含** PascalCase。
///
/// 这是 D3 真实 body 实证的核心断言——锚「前端构造 camelCase 即真实 wire」结论。
#[test]
fn create_posts_casing_is_camelcase_not_pascalcase() {
    let (_, body) = one_http(run(minimal_args()));
    let obj = body.as_object().expect("body 应为 JSON 对象");

    // 含 camelCase。
    assert!(obj.contains_key("posts"), "wire 须含 camelCase 'posts'");
    assert!(
        obj.contains_key("channelIds"),
        "wire 须含 camelCase 'channelIds'"
    );
    // 绝不含 PascalCase（Go 字段名从未上线，case-insensitive 解码故无需 PascalCase）。
    assert!(
        !obj.contains_key("Posts"),
        "wire 禁含 PascalCase 'Posts'（前端从不构造）"
    );
    assert!(
        !obj.contains_key("ChannelIds"),
        "wire 禁含 PascalCase 'ChannelIds'（前端从不构造）"
    );
    // 也不含 snake_case 入参名泄漏到 wire。
    assert!(
        !obj.contains_key("channel_ids"),
        "wire 禁泄漏 snake_case 入参名 'channel_ids'"
    );
    // 顶层恰好两个 key（无冗余字段）。
    assert_eq!(obj.len(), 2, "顶层 wire key 集恰为 {{posts, channelIds}}");
}

/// 真 Go fixture 比对：顶层 key 集 == Go 解构体期望字段（case-insensitive 下 camelCase 形态）。
///
/// Go `createPosts` 解构体 `{ Posts *entity.Posts; ChannelIds []string }`——只认这两个字段
/// （无 tag → 大小写不敏感）。helix 产的 camelCase key 经 case-insensitive 折叠须命中且无多余。
#[test]
fn create_posts_keys_match_go_struct_case_insensitive() {
    // Go 解构体字段（PascalCase 字段名），case-insensitive 折叠到小写比对。
    let go_fields_lower = ["posts", "channelids"];
    let (_, body) = one_http(run(minimal_args()));
    let obj = body.as_object().unwrap();

    let mut wire_lower: Vec<String> = obj.keys().map(|k| k.to_lowercase()).collect();
    wire_lower.sort();
    let mut expected: Vec<String> = go_fields_lower.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(
        wire_lower, expected,
        "wire 顶层字段（case-insensitive 折叠）须恰为 Go 解构体的 Posts/ChannelIds"
    );
}

/// channelIds 多目标透传（批量转发本质：N channel）。
#[test]
fn create_posts_multi_channel_passthrough() {
    let args = json!({
        "posts": [{ "message": "m" }],
        "channel_ids": ["c1", "c2", "c3"],
    });
    let (_, body) = one_http(run(args));
    assert_eq!(body["channelIds"], json!(["c1", "c2", "c3"]));
}

/// 边界零信任：缺/空 posts、缺/空 channel_ids、非数组 → Err（不 panic，HX-C 不变量 4）。
#[test]
fn create_posts_rejects_invalid_input() {
    // 缺 posts
    assert!(err(json!({ "channel_ids": ["c1"] })));
    // 空 posts 数组
    assert!(err(json!({ "posts": [], "channel_ids": ["c1"] })));
    // posts 非数组
    assert!(err(json!({ "posts": "nope", "channel_ids": ["c1"] })));
    // 缺 channel_ids
    assert!(err(json!({ "posts": [{ "message": "m" }] })));
    // 空 channel_ids 数组
    assert!(err(
        json!({ "posts": [{ "message": "m" }], "channel_ids": [] })
    ));
    // channel_ids 非数组
    assert!(err(
        json!({ "posts": [{ "message": "m" }], "channel_ids": "c1" })
    ));
    // 完全空对象
    assert!(err(json!({})));
}

/// 坏 JSON payload → Err（不 panic）。
#[test]
fn create_posts_bad_json_payload_errs() {
    assert!(handle_outbound("im_create_posts", b"not-json", BASE, BASE, None, corr()).is_err());
}

/// registry 性质：is_outbound 命中 + 注册总数 == 88（CP 补齐后）。
///
/// 88 = 原 87 outbound（registry）+ createPosts；`im_send_message` 不在 registry（走 send arm）。
/// 加上 send 命令 = 89 出站/写命令（04 文档表 B 口径）。
#[test]
fn create_posts_registered_and_total_is_88() {
    assert!(
        is_outbound("im_create_posts"),
        "im_create_posts 应被 registry 认领"
    );
    assert_eq!(
        outbound_command_count(),
        99,
        "outbound registry 总数应为 99（94 + vote/score 第二网关族 5：createVote/readVote/deleteVote + average read/delete；send 不在内）"
    );
    // send 命令不在 outbound registry（走 module.rs send arm，带乐观落库/timer）。
    assert!(!is_outbound("im_send_message"));
}
