//! P3a — posts 读族 outbound（getSchedule/postContext/top20/get/getPostsAfterIndex/
//! getReplies/getReplyBranch/queryTodoList/getUpdatedPosts/getLatestPost + bookmark×3 +
//! announcement 读×3 + post/read/list）集成测试。
//!
//! 回归锚: P3a
//!
//! 覆盖维度（02 文档 P3a gate + 03 测试矩阵 P3a 行）：
//! - 每命令 1 正常路径：endpoint url + body casing 断言（锚现网 Go 真源 posts.go/post_bookmark.go
//!   逐字 json tag，不自造 fixture，C5）
//! - **命名陷阱 casing 断言**（decode 静默失败防线）：
//!   · top20 → body **snake_case** `channel_id`（唯一 snake 读端，posts.go:550）
//!   · getPostsAfterIndex → body key `postIds` 但值是**单 id string**（非数组，posts.go:318）
//!   · getUpdatedPosts → body key `timeStamp` **大写 S**（posts.go CursorTs tag）
//! - endpoint 前缀陷阱：bookmark/announcement/read-list 走 `post/`（单数）+ 子路由；posts 读族走 `posts/`
//! - 边界零信任：缺必填 → Err 不 panic（require_str / require_str_array / time_stamp 缺）
//! - registry 性质：17 命令全 is_outbound 命中 + 全可 dispatch（注册数==命令数，无重复/漏）
//!
//! 架构边界（P3A_EXPECT ③）：读命令是 request/response——本 Phase 验 **build wire body 正确性**
//! （driver fire 后响应回灌 await 的语义 invoke）；response→projection emit 属 WS 推送态 + driver
//! 投影命令两条已在册/P6 路径，projection-schema 冻结后才接，不在 P3a build 层断言。
//!
//! 可证伪（HX-C011）：改错任一 endpoint / casing → 对应断言 FAIL；把 top20 写成 camelCase
//! `channelId`/把 getPostsAfterIndex 值改数组/把 getUpdatedPosts 写成小写 `timestamp` → 命名陷阱
//! 断言 FAIL；删某命令注册 → registry 计数断言 + is_outbound FAIL；放松必填校验 → Err 断言 FAIL。

use helix_core::effect::Effect;
use helix_core::Correlation;
use helix_im::commands::{handle_outbound, is_outbound};
use serde_json::{json, Value};

const BASE: &str = "http://x/api/cses";

fn corr() -> Correlation {
    Correlation::from_raw(73)
}

/// 取出单条 Http 的 (url, body-json)；非单条 Http → panic。
fn one_http(effects: Vec<Effect>) -> (String, Value) {
    assert_eq!(effects.len(), 1, "读族命令当前恒产单条 Effect::Http");
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
        Some("conn-3a"),
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

// ── posts/* 读族 ────────────────────────────────────────────────────────────────

#[test]
fn get_schedule_builds_channel_user_body() {
    // GetSchedulePostReq{channelId, userId}（schedule_post.go:22，Validate 要求 userId）
    let (url, body) = one_http(run(
        "im_get_schedule",
        json!({ "channel_id": "cS", "user_id": "uS" }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/getSchedule");
    assert_eq!(body["channelId"], "cS");
    assert_eq!(body["userId"], "uS");
    assert!(body.get("channel_id").is_none());
}

#[test]
fn get_schedule_requires_channel_and_user() {
    assert!(err("im_get_schedule", json!({ "channel_id": "c" }))); // 缺 user_id
    assert!(err("im_get_schedule", json!({ "user_id": "u" }))); // 缺 channel_id
}

#[test]
fn post_context_builds_post_before_body() {
    // {postId, before:int}（posts.go:564）
    let (url, body) = one_http(run(
        "im_post_context",
        json!({ "post_id": "pC", "before": 30 }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/postContext");
    assert_eq!(body["postId"], "pC");
    assert_eq!(body["before"], 30);
}

#[test]
fn post_context_before_defaults_zero_and_requires_post() {
    let (_url, body) = one_http(run("im_post_context", json!({ "post_id": "p" })));
    assert_eq!(body["before"], 0, "before 缺省 0（Go int 零值）");
    assert!(err("im_post_context", json!({}))); // 缺 post_id
}

#[test]
fn top20_uses_snake_case_channel_id_trap() {
    // **命名陷阱**：唯一 snake_case 读端（posts.go:550 `ChannelId string json:"channel_id"`）
    let (url, body) = one_http(run("im_top20", json!({ "channel_id": "cT" })));
    assert_eq!(url, "http://x/api/cses/posts/top20");
    assert_eq!(
        body["channel_id"], "cT",
        "top20 body 必须 snake_case channel_id"
    );
    assert!(
        body.get("channelId").is_none(),
        "top20 禁 camelCase channelId（decode 静默失败陷阱）"
    );
    assert!(err("im_top20", json!({}))); // 缺 channel_id
}

#[test]
fn get_posts_builds_postids_array() {
    // {postIds:*[]string}（posts.go:535，复数数组）
    let (url, body) = one_http(run("im_get_posts", json!({ "post_ids": ["a", "b"] })));
    assert_eq!(url, "http://x/api/cses/posts/get");
    assert_eq!(body["postIds"][0], "a");
    assert_eq!(body["postIds"][1], "b");
    assert!(body["postIds"].is_array());
}

#[test]
fn get_posts_requires_nonempty_string_array() {
    assert!(err("im_get_posts", json!({ "post_ids": [] }))); // 空
    assert!(err("im_get_posts", json!({ "post_ids": [1, 2] }))); // 非 string
    assert!(err("im_get_posts", json!({}))); // 缺
}

#[test]
fn get_posts_after_index_uses_singular_string_postids_trap() {
    // **命名陷阱**：body key `postIds` 但值是**单 id string**（posts.go:318 `PostId string json:"postIds"`）
    let (url, body) = one_http(run(
        "im_get_posts_after_index",
        json!({ "post_id": "pAfter" }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/getPostsAfterIndex");
    assert_eq!(
        body["postIds"], "pAfter",
        "getPostsAfterIndex postIds 必须是单 string"
    );
    assert!(
        body["postIds"].is_string() && !body["postIds"].is_array(),
        "getPostsAfterIndex postIds 是 string 非数组（命名陷阱）"
    );
    assert!(err("im_get_posts_after_index", json!({}))); // 缺 post_id
}

#[test]
fn get_replies_builds_page_body() {
    // GetPostOpts 嵌 PageOpts{pageNumber,pageSize} + replyId（posts.go:280）
    let (url, body) = one_http(run(
        "im_get_replies",
        json!({ "reply_id": "rId", "page_number": 2, "page_size": 20, "revoke": true }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/getReplies");
    assert_eq!(body["replyId"], "rId");
    assert_eq!(body["pageNumber"], 2);
    assert_eq!(body["pageSize"], 20);
    assert_eq!(body["revoke"], true);
}

#[test]
fn get_replies_page_defaults_and_revoke_omitted_and_requires_id() {
    let (_url, body) = one_http(run("im_get_replies", json!({ "reply_id": "r" })));
    assert_eq!(body["pageNumber"], 1);
    assert_eq!(body["pageSize"], 20);
    assert!(body.get("revoke").is_none(), "revoke 仅显式 bool 时下发");
    assert!(err("im_get_replies", json!({}))); // 缺 reply_id
}

#[test]
fn get_reply_branch_builds_first_level_page_body() {
    // {replyFirstLevelId, pageNumber, pageSize}（posts.go:261）
    let (url, body) = one_http(run(
        "im_get_reply_branch",
        json!({ "reply_first_level_id": "fId", "page_number": 1, "page_size": 10 }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/getReplyBranch");
    assert_eq!(body["replyFirstLevelId"], "fId");
    assert_eq!(body["pageNumber"], 1);
    assert_eq!(body["pageSize"], 10);
    assert!(err("im_get_reply_branch", json!({}))); // 缺 reply_first_level_id
}

#[test]
fn query_todo_list_builds_postids_array() {
    // {postIds:[]string} 非空（posts.go:694）
    let (url, body) = one_http(run(
        "im_query_todo_list",
        json!({ "post_ids": ["t1", "t2"] }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/queryTodoList");
    assert_eq!(body["postIds"][0], "t1");
    assert!(err("im_query_todo_list", json!({ "post_ids": [] }))); // 空 → Err
}

#[test]
fn get_updated_posts_uses_capital_s_timestamp_trap() {
    // **命名陷阱**：body key `timeStamp` 大写 S（posts.go CursorTs `json:"timeStamp"`）
    let (url, body) = one_http(run(
        "im_get_updated_posts",
        json!({ "time_stamp": 1_700_000_000_i64, "limit": 50 }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/getUpdatedPosts");
    assert_eq!(body["timeStamp"], 1_700_000_000_i64, "必须大写 S timeStamp");
    assert!(
        body.get("timestamp").is_none() && body.get("time_stamp").is_none(),
        "禁小写 timestamp / snake time_stamp（命名陷阱）"
    );
    assert_eq!(body["limit"], 50);
}

#[test]
fn get_updated_posts_requires_timestamp_int_and_limit_defaults() {
    let (_url, body) = one_http(run("im_get_updated_posts", json!({ "time_stamp": 5 })));
    assert_eq!(body["limit"], 0, "limit 缺省 0");
    assert!(err("im_get_updated_posts", json!({}))); // 缺 time_stamp
    assert!(err("im_get_updated_posts", json!({ "time_stamp": "x" }))); // 非 int
}

#[test]
fn get_latest_post_builds_channel_timestamp_body() {
    // {channelId, timestamp}（posts.go:201）—— 注意此处 timestamp **小写**（与 getUpdatedPosts 相反）
    let (url, body) = one_http(run(
        "im_get_latest_post",
        json!({ "channel_id": "cL", "timestamp": 999 }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/getLatestPost");
    assert_eq!(body["channelId"], "cL");
    assert_eq!(
        body["timestamp"], 999,
        "getLatestPost 用小写 timestamp（posts.go:201）"
    );
    assert!(err("im_get_latest_post", json!({}))); // 缺 channel_id
}

// ── post/bookmark/* 读族（endpoint 走 `post/`（单数）+ `/bookmark/`）─────────────────

#[test]
fn bookmark_create_builds_body_and_post_singular_prefix() {
    // {channelId, userId, postIds:[]string}（post_bookmark.go:54）
    let (url, body) = one_http(run(
        "im_bookmark_create",
        json!({ "channel_id": "cB", "user_id": "uB", "post_ids": ["b1"] }),
    ));
    assert_eq!(url, "http://x/api/cses/post/bookmark/create");
    // endpoint 前缀陷阱：必须 /post/bookmark（单数 post）而非 /posts/
    assert!(
        url.contains("/post/bookmark/") && !url.contains("/posts/bookmark/"),
        "bookmark 必须落 /post（单数）前缀: {url}"
    );
    assert_eq!(body["channelId"], "cB");
    assert_eq!(body["userId"], "uB");
    assert_eq!(body["postIds"][0], "b1");
}

#[test]
fn bookmark_create_requires_all_three() {
    assert!(err(
        "im_bookmark_create",
        json!({ "user_id": "u", "post_ids": ["x"] })
    )); // 缺 channel
    assert!(err(
        "im_bookmark_create",
        json!({ "channel_id": "c", "post_ids": ["x"] })
    )); // 缺 user
    assert!(err(
        "im_bookmark_create",
        json!({ "channel_id": "c", "user_id": "u" })
    )); // 缺 post_ids
    assert!(err(
        "im_bookmark_create",
        json!({ "channel_id": "c", "user_id": "u", "post_ids": [] })
    )); // 空数组
}

#[test]
fn bookmark_delete_builds_user_post_body() {
    // {userId, postId}（post_bookmark.go:36）
    let (url, body) = one_http(run(
        "im_bookmark_delete",
        json!({ "user_id": "uD", "post_id": "pD" }),
    ));
    assert_eq!(url, "http://x/api/cses/post/bookmark/delete");
    assert_eq!(body["userId"], "uD");
    assert_eq!(body["postId"], "pD");
    assert!(err("im_bookmark_delete", json!({ "user_id": "u" }))); // 缺 post_id
    assert!(err("im_bookmark_delete", json!({ "post_id": "p" }))); // 缺 user_id
}

#[test]
fn bookmark_load_builds_channel_user_body() {
    // {channelId, userId}（post_bookmark.go:17）+ 内嵌 entity.PageOpts 扁平分页字段。
    let (url, body) = one_http(run(
        "im_bookmark_load",
        json!({ "channel_id": "cLd", "user_id": "uLd" }),
    ));
    assert_eq!(url, "http://x/api/cses/post/bookmark/load");
    assert_eq!(body["channelId"], "cLd");
    assert_eq!(body["userId"], "uLd");
    // 分页缺省时不注入键（边界零信任：仅透传存在的入参，缺省由后端 PageOpts 零值兜底）
    assert!(body.get("pageSize").is_none());
    assert!(body.get("pageNumber").is_none());
    assert!(body.get("offset").is_none());
    assert!(err("im_bookmark_load", json!({ "channel_id": "c" }))); // 缺 user_id
    assert!(err("im_bookmark_load", json!({ "user_id": "u" }))); // 缺 channel_id（覆盖 channel_id `?` 早退）
}

#[test]
fn bookmark_load_carries_pagination_flat_camel() {
    // full-map partials/1:543-553：loadPostBookmark 内嵌 entity.PageOpts（embedded struct
    // → 扁平序列化），前端 queryBookMarks(:763) 透传 {channelId,userId,pageSize,pageNumber?,offset?}。
    // 01-接口可达性清单.md:333 证 post_bookmark JOIN post raw SQL Limit N+1 主动消费分页 →
    // pageSize 缺失 = SQL LIMIT≈0/1 = 书签面板最多回 1 条 + hasMore 失真 + 无限滚动断。
    // 入参 snake_case（Tauri bridge 约定），出 wire 扁平 camelCase（对齐 PageOpts json tag）。
    let (url, body) = one_http(run(
        "im_bookmark_load",
        json!({
            "channel_id": "cP",
            "user_id": "uP",
            "page_size": 20,
            "page_number": 2,
            "offset": 40
        }),
    ));
    assert_eq!(url, "http://x/api/cses/post/bookmark/load");
    assert_eq!(body["channelId"], "cP");
    assert_eq!(body["userId"], "uP");
    // 扁平（非嵌套）+ camelCase + 数值原值（不强转字符串）
    assert_eq!(body["pageSize"], 20);
    assert_eq!(body["pageNumber"], 2);
    assert_eq!(body["offset"], 40);
    // 部分分页：只给 pageSize（pageNumber/offset 缺省）→ 只透传存在的
    let (_, body2) = one_http(run(
        "im_bookmark_load",
        json!({ "channel_id": "c", "user_id": "u", "page_size": 50 }),
    ));
    assert_eq!(body2["pageSize"], 50);
    assert!(body2.get("pageNumber").is_none());
    assert!(body2.get("offset").is_none());
}

// ── post/announcement/* 读族（endpoint 走 `post/`（单数））────────────────────────────

#[test]
fn announcement_accept_list_builds_post_body() {
    // {postId}（posts.go AnnouncementAcceptList）
    let (url, body) = one_http(run(
        "im_announcement_accept_list",
        json!({ "post_id": "pA" }),
    ));
    assert_eq!(url, "http://x/api/cses/post/announcement/acceptList");
    assert_eq!(body["postId"], "pA");
    assert!(err("im_announcement_accept_list", json!({}))); // 缺 post_id
}

#[test]
fn announcement_list_builds_channel_body_with_optional_post() {
    // {channelId, postId?}（posts.go AnnouncementList：channelId 空回退 postId；前端实测 list 的
    // postId 实际传 channelId，见 公告功能-前端联调.md:214）
    let (url, body) = one_http(run(
        "im_announcement_list",
        json!({ "channel_id": "cAl", "post_id": "pAl" }),
    ));
    assert_eq!(url, "http://x/api/cses/post/announcement/list");
    assert_eq!(body["channelId"], "cAl");
    assert_eq!(body["postId"], "pAl");
    // post_id 可选
    let (_url, body2) = one_http(run(
        "im_announcement_list",
        json!({ "channel_id": "cOnly" }),
    ));
    assert_eq!(body2["channelId"], "cOnly");
    assert!(body2.get("postId").is_none());
    assert!(err("im_announcement_list", json!({}))); // 缺 channel_id
}

#[test]
fn announcement_detail_builds_postids_array() {
    // {postIds:[]string}（posts.go AnnouncementDetail）
    let (url, body) = one_http(run(
        "im_announcement_detail",
        json!({ "post_ids": ["d1", "d2"] }),
    ));
    assert_eq!(url, "http://x/api/cses/post/announcement/detail");
    assert_eq!(body["postIds"][1], "d2");
    assert!(err("im_announcement_detail", json!({ "post_ids": [] }))); // 空
}

// ── post/read/list 已读位历史读（B1：非 bitmap，走 post/ 单数）───────────────────────

#[test]
fn post_read_list_builds_postids_array_post_singular_prefix() {
    // PostReadListParam{postIds:[]string}（post.go:554 → post/read/list 路由 posts.go:31）
    let (url, body) = one_http(run("im_post_read_list", json!({ "post_ids": ["rl1"] })));
    assert_eq!(url, "http://x/api/cses/post/read/list");
    assert!(
        url.contains("/post/read/list") && !url.contains("/posts/read/list"),
        "post/read/list 必须落 /post（单数）前缀: {url}"
    );
    assert_eq!(body["postIds"][0], "rl1");
    assert!(err("im_post_read_list", json!({ "post_ids": [] }))); // 空
}

// ── registry 性质：17 命令全 is_outbound 命中 + 全可 dispatch ─────────────────────

/// P3a 读族全集（17）。改这个列表必须同步 posts_read.rs 注册文件。
const ALL_P3A_READ: &[(&str, fn() -> Value)] = &[
    (
        "im_get_schedule",
        || json!({ "channel_id": "c", "user_id": "u" }),
    ),
    ("im_post_context", || json!({ "post_id": "p" })),
    ("im_top20", || json!({ "channel_id": "c" })),
    ("im_get_posts", || json!({ "post_ids": ["p"] })),
    ("im_get_posts_after_index", || json!({ "post_id": "p" })),
    ("im_get_replies", || json!({ "reply_id": "r" })),
    (
        "im_get_reply_branch",
        || json!({ "reply_first_level_id": "f" }),
    ),
    ("im_query_todo_list", || json!({ "post_ids": ["p"] })),
    ("im_get_updated_posts", || json!({ "time_stamp": 1 })),
    ("im_get_latest_post", || json!({ "channel_id": "c" })),
    (
        "im_bookmark_create",
        || json!({ "channel_id": "c", "user_id": "u", "post_ids": ["p"] }),
    ),
    (
        "im_bookmark_delete",
        || json!({ "user_id": "u", "post_id": "p" }),
    ),
    (
        "im_bookmark_load",
        || json!({ "channel_id": "c", "user_id": "u" }),
    ),
    ("im_announcement_accept_list", || json!({ "post_id": "p" })),
    ("im_announcement_list", || json!({ "channel_id": "c" })),
    ("im_announcement_detail", || json!({ "post_ids": ["p"] })),
    ("im_post_read_list", || json!({ "post_ids": ["p"] })),
];

#[test]
fn registry_covers_all_p3a_read_commands() {
    for (name, _) in ALL_P3A_READ {
        assert!(is_outbound(name), "{name} 应被 registry 认领");
    }
    // 非读族命令不被误纳
    assert!(!is_outbound("im_send_message"));
    assert!(!is_outbound("bogus_read"));
}

/// 注册数 == 命令数性质：每个注册命令都能成功 dispatch（无重复 name 导致 registry 构建失败）。
/// 若任一命令 distributed_slice 注册了重复 name → registry 构建 Err → handle_outbound 全 Err →
/// 本测试 FAIL（即「注册数==命令数」的可证伪体现）。
#[test]
fn every_p3a_read_command_is_dispatchable() {
    assert_eq!(ALL_P3A_READ.len(), 17, "P3a 读族应为 17 命令");
    for (name, mk) in ALL_P3A_READ {
        let effects = run(name, mk());
        assert_eq!(effects.len(), 1, "{name} 应产单条 Effect::Http");
        assert!(
            matches!(effects[0], Effect::Http { .. }),
            "{name} 应产 Effect::Http"
        );
    }
}

/// handle_outbound 错误闭包路径（边界零信任，不 panic）：坏 JSON payload。
#[test]
fn handle_outbound_bad_payload_is_error_not_panic() {
    assert!(handle_outbound("im_top20", b"{not json", BASE, BASE, None, corr()).is_err());
}
