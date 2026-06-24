//! P5a — user-misc 域 outbound（users/list·status/ids + teams×3 + modules/getAll +
//! notification/loadSend + search 四件套）集成测试。
//!
//! 回归锚: P5a
//!
//! 覆盖维度（02 文档 P5a gate + 03 测试矩阵 P5a 行）：
//! - 每命令 1 正常路径：endpoint url + body casing 断言（锚现网 Go json tag 逐字，不自造 fixture，C5）
//!   · users/list → camelCase `channelId`（user.go:23）
//!   · users/status/ids → `userIds:[]`（user.go:43，presence 接管 D2）
//!   · teams/member/quit → `{userId,teamId}`（quit_team.go:3-6）
//!   · notification/loadSend → `{page,size,state?}`（NotificationReq + Page）
//!   · search 四件套 body=DoSearchRequest camelCase（keyword/searchType/channelId/teamId/isAll）
//! - endpoint 前缀：users/teams/modules/notification/search 各自子路由
//! - 源 bug 移植纪律：4 处源 bug 属服务端响应/handler 态，outbound 只 build 请求体，不绕过/不预修
//!   （本测试断言 outbound endpoint+body 与现网 decode 结构一致，不在 build 层裁决响应 bug）
//! - PERSONAL_USER_FORBIDDEN：teams/* 的 403 响应码（personal_guard.go:13 verbatim），响应态
//!   → 文档断言侧记录（见 personal_user_forbidden_is_response_side 注释断言）
//! - health 裸 {"status":"OK"}：响应态特例（health.go ReturnStatusOK），非 outbound build 关切
//! - search channelDetails 第四 key：do=searchAll 响应含 channels/users/channelDetails（cses_search.go），响应态
//! - userId json:"-" 防漏：search body 禁前端下发 user_id（DoSearchRequest.UserId 服务端注入）
//! - 边界零信任：缺必填 → Err 不 panic（require_str / require_str_array）
//! - registry 性质：11 命令全 is_outbound 命中 + 全可 dispatch（注册数==命令数，无重复/漏）
//!
//! 可证伪（HX-C011）：改错任一 endpoint / casing → 对应断言 FAIL；把 users/list 写 snake `channel_id`
//! / search 写 snake key / 让 search 下发 user_id → 断言 FAIL；删某命令注册 → registry 计数 +
//! is_outbound FAIL；放松必填校验 → Err 断言 FAIL。

use helix_core::effect::{Effect, HttpRequest};
use helix_core::{AuthKind, Correlation, AUTH_KIND_HEADER};
use helix_im::commands::{handle_outbound, is_outbound};
use serde_json::{json, Value};

const BASE: &str = "http://x/api/cses";

fn corr() -> Correlation {
    Correlation::from_raw(5)
}

/// 取出单条 Http 的 (url, body-json)；非单条 Http → panic。
fn one_http(effects: Vec<Effect>) -> (String, Value) {
    assert_eq!(effects.len(), 1, "user-misc 命令当前恒产单条 Effect::Http");
    let req = one_http_req(effects);
    let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
    (req.url, body)
}

/// 取出单条 Http request；用于 method / header 契约断言。
fn one_http_req(effects: Vec<Effect>) -> HttpRequest {
    assert_eq!(effects.len(), 1, "user-misc 命令当前恒产单条 Effect::Http");
    match effects.into_iter().next().unwrap() {
        Effect::Http { req, .. } => req,
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

fn run(name: &str, payload: Value) -> Vec<Effect> {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        Some("conn-5a"),
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

// ── users/* ──────────────────────────────────────────────────────────────────────

#[test]
fn list_users_builds_camel_channel_id_body() {
    // 真源 `{channelId}`（user.go:23 `ChannelId string json:"channelId"`）
    let (url, body) = one_http(run("im_list_users", json!({ "channel_id": "cU" })));
    assert_eq!(url, "http://x/api/cses/users/list");
    assert_eq!(body["channelId"], "cU");
    assert!(
        body.get("channel_id").is_none(),
        "users/list body 必须 camelCase channelId（user.go:23），禁 snake"
    );
    assert!(err("im_list_users", json!({}))); // 缺 channel_id
}

#[test]
fn get_user_statuses_builds_userids_array_presence() {
    // presence 接管（D2）：真源 `{userIds:[]string}`（user.go:43）
    let (url, body) = one_http(run(
        "im_get_user_statuses",
        json!({ "user_ids": ["u1", "u2"] }),
    ));
    assert_eq!(url, "http://x/api/cses/users/status/ids");
    assert_eq!(body["userIds"][0], "u1");
    assert_eq!(body["userIds"][1], "u2");
    assert!(body["userIds"].is_array());
    assert!(err("im_get_user_statuses", json!({ "user_ids": [] }))); // 空 → Err
    assert!(err("im_get_user_statuses", json!({ "user_ids": [1] }))); // 非 string
    assert!(err("im_get_user_statuses", json!({}))); // 缺
}

// ── teams/* ───────────────────────────────────────────────────────────────────────

#[test]
fn team_upsert_passes_through_team_object() {
    // 真源 r.Body 原样透传（team.go:63 UpsertTeam(session, r.Body)）
    let team = json!({ "id": "t1", "name": "公司大群", "members": ["a"] });
    let (url, body) = one_http(run("im_team_upsert", json!({ "team": team })));
    assert_eq!(url, "http://x/api/cses/teams/upsert");
    assert_eq!(body["id"], "t1");
    assert_eq!(body["name"], "公司大群");
    assert_eq!(body["members"][0], "a");
    assert!(err("im_team_upsert", json!({}))); // 缺 team
    assert!(err("im_team_upsert", json!({ "team": "not-object" }))); // 非对象
}

#[test]
fn team_member_add_passes_through_event_object() {
    // 真源 ChannelMemberEvent（team.go:42 decode）整对象透传
    let event = json!({ "channelId": "c1", "userIds": ["u"], "type": "add" });
    let (url, body) = one_http(run("im_team_member_add", json!({ "event": event })));
    assert_eq!(url, "http://x/api/cses/teams/member/add");
    assert_eq!(body["channelId"], "c1");
    assert_eq!(body["type"], "add");
    assert!(err("im_team_member_add", json!({}))); // 缺 event
    assert!(err("im_team_member_add", json!({ "event": [] }))); // 非对象
}

#[test]
fn team_quit_builds_user_team_body() {
    // 真源 QuitTeamReq{userId,teamId}（quit_team.go:3-6）。无响应体是服务端 handler 态（透传不影响 build）。
    // Task1 只验 method/url/body；session 鉴权意图单列到 Task2，避免把两个契约耦在同一个回归点里。
    let req = one_http_req(run(
        "im_team_quit",
        json!({ "user_id": "uQ", "team_id": "tQ" }),
    ));
    assert_eq!(
        req.method, "DELETE",
        "teams/member/quit 现网是 HTTP DELETE，不能继续由 registry 写死 POST"
    );
    let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
    assert_eq!(req.url, "http://x/api/cses/teams/member/quit");
    assert_eq!(body["userId"], "uQ");
    assert_eq!(body["teamId"], "tQ");
    assert!(err("im_team_quit", json!({ "user_id": "u" }))); // 缺 team_id
    assert!(err("im_team_quit", json!({ "team_id": "t" }))); // 缺 user_id
}

#[test]
fn team_quit_marks_session_auth_kind() {
    let req = one_http_req(run(
        "im_team_quit",
        json!({ "user_id": "uQ", "team_id": "tQ" }),
    ));
    assert!(
        req.headers
            .iter()
            .any(|(k, v)| k == AUTH_KIND_HEADER && v == AuthKind::Session.as_str()),
        "默认 session outbound 必须显式标 X-Auth-Kind: session，供 host crosscut 注入真实 cookie"
    );
}

// ── modules/* ─────────────────────────────────────────────────────────────────────

#[test]
fn get_all_modules_builds_empty_body() {
    // 真源**空 body** POST（modules.go:14 不 decode；前端 message.service.ts:828 无 body）。
    // 现网 SetMessage("channelMemberChange success") 笔误是响应态，不影响 outbound build。
    let (url, body) = one_http(run("im_get_all_modules", json!({})));
    assert_eq!(url, "http://x/api/cses/modules/getAll");
    assert_eq!(body, json!({}), "modules/getAll 请求体为空对象");
    // 即便前端塞了无关字段，build 仍产空 body（不泄漏 UI 字段到 wire）。
    let (_url, body2) = one_http(run("im_get_all_modules", json!({ "junk": 1 })));
    assert_eq!(body2, json!({}));
}

// ── notification/* ────────────────────────────────────────────────────────────────

#[test]
fn load_send_notifications_builds_page_state_body() {
    // 真源 NotificationReq{Page{page,size}, state}（NotificationReq.go + page.go）。
    // loadSend↔loadTarget handler 互换是服务端响应态，outbound endpoint 锚 notification/loadSend。
    let (url, body) = one_http(run(
        "im_load_send_notifications",
        json!({ "page": 1, "size": 10, "state": "unread" }),
    ));
    assert_eq!(url, "http://x/api/cses/notification/loadSend");
    assert_eq!(body["page"], 1);
    assert_eq!(body["size"], 10);
    assert_eq!(body["state"], "unread");
}

#[test]
fn load_send_notifications_state_optional_and_page_size_required() {
    let (_url, body) = one_http(run(
        "im_load_send_notifications",
        json!({ "page": 2, "size": 20 }),
    ));
    assert_eq!(body["page"], 2);
    assert_eq!(body["size"], 20);
    assert!(body.get("state").is_none(), "state 仅显式给出时下发");
    assert!(err("im_load_send_notifications", json!({ "size": 10 }))); // 缺 page
    assert!(err("im_load_send_notifications", json!({ "page": 1 }))); // 缺 size
    assert!(err(
        "im_load_send_notifications",
        json!({ "page": "x", "size": 10 })
    )); // page 非 int
}

// ── search 四件套（body = DoSearchRequest camelCase）───────────────────────────────

#[test]
fn search_post_builds_do_search_request_camel_body() {
    // DoSearchRequest（post_search_param.go:25-34）camelCase tag
    let (url, body) = one_http(run(
        "im_search_post",
        json!({ "keyword": "hi", "search_type": 1, "channel_id": "cS", "team_id": "tS", "is_all": true }),
    ));
    assert_eq!(url, "http://x/api/cses/search/post");
    assert_eq!(body["keyword"], "hi");
    assert_eq!(
        body["searchType"], 1,
        "searchType camelCase（DoSearchRequest）"
    );
    assert_eq!(body["channelId"], "cS");
    assert_eq!(body["teamId"], "tS");
    assert_eq!(body["isAll"], true);
    // casing 陷阱：禁 snake
    assert!(body.get("search_type").is_none());
    assert!(body.get("channel_id").is_none());
    assert!(body.get("team_id").is_none());
    assert!(body.get("is_all").is_none());
}

#[test]
fn search_user_channel_do_share_do_search_body() {
    for (name, ep) in [
        ("im_search_user", "search/user"),
        ("im_search_channel", "search/channel"),
        ("im_search_do", "search/do"),
    ] {
        let (url, body) = one_http(run(name, json!({ "keyword": "q", "channel_id": "c" })));
        assert_eq!(url, format!("http://x/api/cses/{ep}"));
        assert_eq!(body["keyword"], "q");
        assert_eq!(body["channelId"], "c");
    }
}

#[test]
fn search_channel_id_defaults_empty_and_optionals_omitted() {
    // channelId 非指针（DoSearchRequest.ChannelId string）→ 缺则空串下发，不报错。
    let (_url, body) = one_http(run("im_search_do", json!({ "keyword": "k" })));
    assert_eq!(
        body["channelId"], "",
        "channelId 缺省空串（Go string 零值）"
    );
    assert!(body.get("teamId").is_none(), "teamId 仅显式给出时下发");
    assert!(body.get("searchType").is_none());
    assert!(body.get("isAll").is_none());
}

#[test]
fn search_rejects_frontend_user_id_json_dash() {
    // DoSearchRequest.UserId `json:"-"`（服务端 session 注入）→ 前端下发 user_id = 契约漂移，拒绝。
    // 四件套各自走 do_search_body 的 `?` 错误传播分支（覆盖每个 misc_cmd! 闭包的 Err 早退 region）。
    assert!(err(
        "im_search_post",
        json!({ "channel_id": "c", "user_id": "leak" })
    ));
    assert!(err(
        "im_search_user",
        json!({ "channel_id": "c", "user_id": "leak" })
    ));
    assert!(err(
        "im_search_channel",
        json!({ "channel_id": "c", "user_id": "leak" })
    ));
    assert!(err("im_search_do", json!({ "user_id": "leak" })));
}

// ── 源 bug / 响应态特例：文档化断言（outbound build 不裁决响应态，此处锚边界）──────────────

/// PERSONAL_USER_FORBIDDEN 是 teams/* 的 403 **响应**码（personal_guard.go:13 verbatim）。
/// outbound build 只产请求；403 + 该码由服务端写、driver/前端消费。本测试锚「不在 outbound build 层」。
/// 验证方式：teams/* 命令正常 build 不依赖/不产出该字符串（响应态与请求态解耦）。
#[test]
fn personal_user_forbidden_is_response_side_not_in_outbound_body() {
    let (_url, body) = one_http(run("im_team_upsert", json!({ "team": { "id": "t" } })));
    assert!(
        !body.to_string().contains("PERSONAL_USER_FORBIDDEN"),
        "PERSONAL_USER_FORBIDDEN 是 403 响应态，不应出现在 outbound 请求体"
    );
}

/// health 裸 {"status":"OK"} 与 search channelDetails 第四 key 均为**响应态**（GET health /
/// search/do searchAll 响应）——P5a outbound 不含 health 请求 build（无 body/无下沉需求），
/// 此处锚「响应态特例不混入 outbound registry」：health 不是 outbound 命令。
#[test]
fn health_and_channel_details_are_response_side_not_outbound_commands() {
    assert!(
        !is_outbound("im_health"),
        "health 裸信封是 GET 响应态，非 outbound build 命令"
    );
    // search/do 是 outbound 命令；channelDetails 第四 key 在其**响应**里（searchAll），非请求体。
    let (_url, body) = one_http(run("im_search_do", json!({ "channel_id": "c" })));
    assert!(
        !body.to_string().contains("channelDetails"),
        "channelDetails 是 searchAll 响应第四 key，不在 outbound 请求体"
    );
}

// ── registry 性质：11 命令全 is_outbound 命中 + 全可 dispatch ─────────────────────

/// P5a user-misc 全集（11）。改这个列表必须同步 user_misc.rs 注册文件。
const ALL_P5A_MISC: &[(&str, fn() -> Value)] = &[
    ("im_list_users", || json!({ "channel_id": "c" })),
    ("im_get_user_statuses", || json!({ "user_ids": ["u"] })),
    ("im_team_upsert", || json!({ "team": { "id": "t" } })),
    (
        "im_team_member_add",
        || json!({ "event": { "channelId": "c" } }),
    ),
    ("im_team_quit", || json!({ "user_id": "u", "team_id": "t" })),
    ("im_get_all_modules", || json!({})),
    (
        "im_load_send_notifications",
        || json!({ "page": 1, "size": 10 }),
    ),
    ("im_search_post", || json!({ "channel_id": "c" })),
    ("im_search_user", || json!({ "channel_id": "c" })),
    ("im_search_channel", || json!({ "channel_id": "c" })),
    ("im_search_do", || json!({ "channel_id": "c" })),
];

#[test]
fn registry_covers_all_p5a_misc_commands() {
    for (name, _) in ALL_P5A_MISC {
        assert!(is_outbound(name), "{name} 应被 registry 认领");
    }
    // 非 user-misc 命令不被误纳
    assert!(!is_outbound("im_send_message"));
    assert!(!is_outbound("bogus_misc"));
    // 不与 P3a / P5b 命令名碰撞（registry 构建无重复才能全 dispatch）
    assert!(is_outbound("im_top20"), "P3a 命令仍在册（无被覆盖）");
}

/// 注册数 == 命令数性质：每个注册命令都能成功 dispatch（无重复 name 致 registry 构建失败）。
/// 任一命令 distributed_slice 注册重复 name → registry 构建 Err → handle_outbound 全 Err → 本测试 FAIL。
#[test]
fn every_p5a_misc_command_is_dispatchable() {
    assert_eq!(ALL_P5A_MISC.len(), 11, "P5a user-misc 应为 11 命令");
    for (name, mk) in ALL_P5A_MISC {
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
    assert!(handle_outbound("im_list_users", b"{not json", BASE, BASE, None, corr()).is_err());
}
