//! P5b — bot-agent 域 outbound（gateway/bot_manage/config/calls/agents/webhook 6 子族 35 命令）集成测试。
//!
//! 回归锚: P5b
//!
//! 覆盖维度（02 文档 P5b gate + 03 测试矩阵 P5b 行）：
//! - 每命令 1 正常路径：endpoint url + body casing 断言（锚现网 Go json tag 逐字，不自造 fixture，C5）
//! - **C6 鉴权铁律**：bot-token 调用命令（#22-27）产 `X-Auth-Kind: bot` 头（实际 token 留 driver）；
//!   session 命令不产该头；grep `mp_` in helix-im/src 必空（CI 闸门，本测试不含 mp_ 字面量）
//! - path-param（`{botUserId}`/`{channelId}`/`{teamId}`/`{userId}`）+ query（`?botUserId=`）插值进 path
//! - path 段零信任：空/含 `/` → Err（防路径注入/越权）
//! - 响应态特例：#2/#32 内存构造 post 不推 cursor + #22/#35-37 裸 map 响应——outbound 只 build 请求体，
//!   断言锚边界（请求体不含 cursor/event_seq/响应信封字段）
//! - 排除：timeline/cost(#33/34 D7 废弃) + cross-repo demo(#38-40 非业务) 不注册
//! - registry 性质：35 命令全 is_outbound 命中 + 全 dispatchable + 不与 P3a/P5a 命令名碰撞
//!
//! 可证伪（HX-C011）：改错任一 endpoint / casing → 对应断言 FAIL；mp_ 命令丢 `X-Auth-Kind: bot` →
//! auth 断言 FAIL；session 命令误加 bot 头 → FAIL；放松 path 段校验 → Err 断言 FAIL；删某命令注册 →
//! registry 计数 + dispatch FAIL；误注册 timeline/cost → 排除断言 FAIL。

use helix_core::effect::Effect;
use helix_core::{AuthKind, Correlation, AUTH_KIND_HEADER};
use helix_im::commands::{handle_outbound, is_outbound};
use serde_json::{json, Value};

const BASE: &str = "http://x/api/cses";

fn corr() -> Correlation {
    Correlation::from_raw(5)
}

/// 取出单条 Http 的 (url, body-json, headers)；非单条 Http → panic。
fn one_http(effects: Vec<Effect>) -> (String, Value, Vec<(String, String)>) {
    assert_eq!(effects.len(), 1, "bot-agent 命令当前恒产单条 Effect::Http");
    match effects.into_iter().next().unwrap() {
        Effect::Http { req, .. } => {
            let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
            (req.url, body, req.headers)
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

fn run(name: &str, payload: Value) -> Vec<Effect> {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        Some("conn-5b"),
        corr(),
    )
    .expect("handle_outbound should not fail")
}

fn err(name: &str, payload: Value) -> bool {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        None,
        corr(),
    )
    .is_err()
}

/// headers 中是否带 `X-Auth-Kind: bot`（bot-token 鉴权意图，真 token 由 driver 注）。
fn has_bot_auth(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(k, v)| k == AUTH_KIND_HEADER && v == AuthKind::Bot.as_str())
}

/// headers 中是否带任何 `X-Auth-Kind`（session 命令应不带，driver 缺省 session）。
fn has_any_auth_kind(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(k, _)| k == AUTH_KIND_HEADER)
}

// ── gateway（#1 createBot / #2 botCallback，session）─────────────────────────────────

#[test]
fn create_bot_builds_camel_body() {
    // BotMessageRequest{sessionId,userId?,channelId?,conversationType?,text}（全可选字段一并覆盖）
    let (url, body, h) = one_http(run(
        "im_create_bot",
        json!({ "session_id": "s1", "text": "hi", "user_id": "u1", "channel_id": "c1", "conversation_type": "direct" }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/createBot");
    assert_eq!(body["sessionId"], "s1");
    assert_eq!(body["text"], "hi");
    assert_eq!(body["userId"], "u1");
    assert_eq!(body["channelId"], "c1");
    assert_eq!(body["conversationType"], "direct");
    assert!(body.get("session_id").is_none(), "禁 snake");
    assert!(
        !has_any_auth_kind(&h),
        "createBot session 命令不带 X-Auth-Kind"
    );
    assert!(err("im_create_bot", json!({ "text": "x" }))); // 缺 session_id
    assert!(err("im_create_bot", json!({ "session_id": "s" }))); // 缺 text
}

#[test]
fn create_bot_omits_optional_fields() {
    let (_u, body, _h) = one_http(run(
        "im_create_bot",
        json!({ "session_id": "s", "text": "t" }),
    ));
    assert!(
        body.get("userId").is_none(),
        "userId 仅显式给出时下发（空时服务端回填 session）"
    );
    assert!(body.get("channelId").is_none());
    assert!(body.get("conversationType").is_none());
}

#[test]
fn bot_callback_builds_camel_body_with_routing() {
    // BotCallbackRequest{messageId,timestamp,to,text,mediaUrl?,replyToId?,threadId?,routing{...}}
    let (url, body, _h) = one_http(run(
        "im_bot_callback",
        json!({
            "message_id": "m1", "timestamp": 1700000000_i64, "to": "user:u1", "text": "reply",
            "media_url": "http://f", "reply_to_id": "r1", "thread_id": "t1",
            "routing": { "channelId": "c1", "userId": "u1" }
        }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/botCallback");
    assert_eq!(body["messageId"], "m1");
    assert_eq!(body["timestamp"], 1700000000_i64);
    assert_eq!(body["to"], "user:u1");
    assert_eq!(body["mediaUrl"], "http://f");
    assert_eq!(body["replyToId"], "r1");
    assert_eq!(body["threadId"], "t1");
    assert_eq!(body["routing"]["channelId"], "c1");
    assert!(err(
        "im_bot_callback",
        json!({ "message_id": "m", "to": "x", "text": "t" })
    )); // 缺 timestamp
}

/// #2 内存构造 post 不推 cursor：outbound 请求体不含 cursor/event_seq（推 cursor 是 WS 推送/落库态，非 build）。
#[test]
fn bot_callback_body_has_no_cursor_field() {
    let (_u, body, _h) = one_http(run(
        "im_bot_callback",
        json!({ "message_id": "m", "timestamp": 1_i64, "to": "user:u", "text": "t" }),
    ));
    let s = body.to_string();
    assert!(
        !s.contains("event_seq") && !s.contains("eventSeq") && !s.contains("cursor"),
        "内存构造 post 不推 cursor：botCallback 请求体不含 cursor/event_seq（gap-bot-agent.md:14）"
    );
}

// ── bot-manage CRUD（#3-11，session）─────────────────────────────────────────────────

#[test]
fn bot_manage_list_create_token_scope() {
    // #3 list 空 body
    let (url, body, _h) = one_http(run("im_bot_list_visible", json!({})));
    assert_eq!(url, "http://x/api/cses/bot-manage/list");
    assert_eq!(body, json!({}));

    // #4 create — CreateBotRequest{name,description,teamId?}
    let (url, body, _h) = one_http(run(
        "im_bot_create",
        json!({ "name": "Bot", "description": "d", "team_id": "t1" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-manage/create");
    assert_eq!(body["name"], "Bot");
    assert_eq!(body["description"], "d");
    assert_eq!(body["teamId"], "t1");
    assert!(err("im_bot_create", json!({}))); // 缺 name

    // #5 token/generate — {botUserId,description,expireAt?}
    let (url, body, _h) = one_http(run(
        "im_bot_token_generate",
        json!({ "bot_user_id": "b1", "description": "d", "expire_at": 999_i64 }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-manage/token/generate");
    assert_eq!(body["botUserId"], "b1");
    assert_eq!(body["expireAt"], 999_i64);

    // #7 token/revoke — {tokenId}
    let (url, body, _h) = one_http(run("im_bot_token_revoke", json!({ "token_id": "tk" })));
    assert_eq!(url, "http://x/api/cses/bot-manage/token/revoke");
    assert_eq!(body["tokenId"], "tk");

    // #8 scope/set — {botUserId,scopeType,teamId?}
    let (url, body, _h) = one_http(run(
        "im_bot_scope_set",
        json!({ "bot_user_id": "b1", "scope_type": "team", "team_id": "t" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-manage/scope/set");
    assert_eq!(body["scopeType"], "team");
    assert_eq!(body["teamId"], "t");
}

#[test]
fn bot_manage_visible_user_add_remove() {
    // #9/#10 — VisibleUserRequest{botUserId,userId}
    for (name, ep) in [
        (
            "im_bot_visible_user_add",
            "bot-manage/scope/visible-user/add",
        ),
        (
            "im_bot_visible_user_remove",
            "bot-manage/scope/visible-user/remove",
        ),
    ] {
        let (url, body, _h) = one_http(run(name, json!({ "bot_user_id": "b", "user_id": "u" })));
        assert_eq!(url, format!("http://x/api/cses/{ep}"));
        assert_eq!(body["botUserId"], "b");
        assert_eq!(body["userId"], "u");
        assert!(err(name, json!({ "bot_user_id": "b" }))); // 缺 user_id
    }
}

#[test]
fn bot_manage_query_param_endpoints_interpolate() {
    // #6 token/list?botUserId= + #11 visible-user/list?botUserId=（query param 插值进 path）
    let (url, _body, _h) = one_http(run("im_bot_token_list", json!({ "bot_user_id": "b9" })));
    assert_eq!(url, "http://x/api/cses/bot-manage/token/list?botUserId=b9");

    let (url, _body, _h) = one_http(run(
        "im_bot_visible_user_list",
        json!({ "bot_user_id": "b9" }),
    ));
    assert_eq!(
        url,
        "http://x/api/cses/bot-manage/scope/visible-user/list?botUserId=b9"
    );
    // path 段零信任：空/含 '/' → Err
    assert!(err("im_bot_token_list", json!({ "bot_user_id": "" })));
    assert!(err("im_bot_token_list", json!({ "bot_user_id": "a/b" })));
    assert!(err("im_bot_token_list", json!({})));
}

// ── bot-agent config + channel（#12-21，session，path param）──────────────────────────

#[test]
fn bot_agent_config_create_passes_through_ent_object() {
    // #12 config — body = ent.BotAgentConfig 整对象（校验 botUserId 非空）
    let cfg = json!({ "botUserId": "b1", "agentName": "A", "enabled": true });
    let (url, body, _h) = one_http(run("im_bot_agent_config_create", json!({ "config": cfg })));
    assert_eq!(url, "http://x/api/cses/bot-agent/config");
    assert_eq!(body["botUserId"], "b1");
    assert_eq!(body["agentName"], "A");
    assert!(err("im_bot_agent_config_create", json!({}))); // 缺 config
    assert!(err(
        "im_bot_agent_config_create",
        json!({ "config": { "agentName": "A" } })
    )); // config.botUserId 缺
}

#[test]
fn bot_agent_config_path_param_endpoints() {
    // #13 get / #15 delete / #16 regenerate / #17 test-webhook —— {botUserId} 插值
    let (url, _b, _h) = one_http(run(
        "im_bot_agent_config_get",
        json!({ "bot_user_id": "b1" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-agent/config/b1");

    let (url, _b, _h) = one_http(run(
        "im_bot_agent_config_delete",
        json!({ "bot_user_id": "b1" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-agent/config/b1");

    let (url, _b, _h) = one_http(run(
        "im_bot_agent_regenerate_token",
        json!({ "bot_user_id": "b1" }),
    ));
    assert_eq!(
        url,
        "http://x/api/cses/bot-agent/config/b1/regenerate-token"
    );

    let (url, body, _h) = one_http(run(
        "im_bot_agent_test_webhook",
        json!({ "bot_user_id": "b1", "message": "ping" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-agent/config/b1/test-webhook");
    assert_eq!(body["message"], "ping");
    assert!(err(
        "im_bot_agent_test_webhook",
        json!({ "bot_user_id": "b1" })
    )); // 缺 message
}

#[test]
fn bot_agent_configs_enabled_no_param() {
    // #14 configs/enabled
    let (url, body, _h) = one_http(run("im_bot_agent_configs_enabled", json!({})));
    assert_eq!(url, "http://x/api/cses/bot-agent/configs/enabled");
    assert_eq!(body, json!({}));
}

#[test]
fn bot_agent_channel_path_param_endpoints() {
    // #18 bots / #19 available-bots —— {channelId}
    let (url, _b, _h) = one_http(run(
        "im_bot_agent_channel_bots",
        json!({ "channel_id": "c1" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-agent/channel/c1/bots");
    let (url, _b, _h) = one_http(run(
        "im_bot_agent_channel_available_bots",
        json!({ "channel_id": "c1" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-agent/channel/c1/available-bots");

    // #20 add / #21 remove —— {channelId}/bot/{botUserId} 双段
    for name in ["im_bot_agent_channel_add", "im_bot_agent_channel_remove"] {
        let (url, _b, _h) = one_http(run(
            name,
            json!({ "channel_id": "c1", "bot_user_id": "b1" }),
        ));
        assert_eq!(url, "http://x/api/cses/bot-agent/channel/c1/bot/b1");
        assert!(err(name, json!({ "channel_id": "c1" }))); // 缺 bot_user_id
    }
}

// ── bot-agent 调用（#22-27，bot-token，X-Auth-Kind: bot）───────────────────────────────

#[test]
fn bot_agent_info_carries_bot_auth_kind() {
    // #22 info —— bot-token 轴：必须带 X-Auth-Kind: bot（真 token 由 driver 注）
    let (url, body, h) = one_http(run("im_bot_agent_info", json!({})));
    assert_eq!(url, "http://x/api/cses/bot-agent/info");
    assert_eq!(body, json!({}));
    assert!(
        has_bot_auth(&h),
        "info 是 bot-token 命令，必带 X-Auth-Kind: bot（C6）"
    );
}

#[test]
fn bot_agent_send_direct_channel_camel_body_and_bot_auth() {
    // #23 direct — BotAgentSendMessageRequest{teamId,userId,message,createChannel?,props?,userSnapshot?}
    let (url, body, h) = one_http(run(
        "im_bot_agent_send_direct",
        json!({
            "team_id": "t1", "user_id": "u1", "message": "hello",
            "create_channel": true, "props": { "k": "v" }, "user_snapshot": { "name": "n" }
        }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-agent/messages/direct");
    assert_eq!(body["teamId"], "t1");
    assert_eq!(body["userId"], "u1");
    assert_eq!(body["message"], "hello");
    assert_eq!(body["createChannel"], true);
    assert_eq!(body["props"]["k"], "v");
    assert_eq!(body["userSnapshot"]["name"], "n");
    assert!(
        has_bot_auth(&h),
        "messages/direct 是 bot-token 命令，必带 X-Auth-Kind: bot"
    );
    assert!(err(
        "im_bot_agent_send_direct",
        json!({ "team_id": "t", "message": "m" })
    )); // 缺 user_id
    assert!(err(
        "im_bot_agent_send_direct",
        json!({ "user_id": "u", "message": "m" })
    )); // 缺 team_id

    // #24 channel — channelId 必填
    let (url, body, h) = one_http(run(
        "im_bot_agent_send_channel",
        json!({ "team_id": "t1", "channel_id": "c1", "message": "hi" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-agent/messages/channel");
    assert_eq!(body["channelId"], "c1");
    assert!(has_bot_auth(&h));
    assert!(err(
        "im_bot_agent_send_channel",
        json!({ "team_id": "t", "message": "m" })
    )); // 缺 channel_id
}

#[test]
fn bot_agent_team_path_endpoints_carry_bot_auth() {
    // #25 members?page=&per_page= — path + query
    let (url, _b, h) = one_http(run(
        "im_bot_agent_team_members",
        json!({ "team_id": "t1", "page": 2, "per_page": 50 }),
    ));
    assert_eq!(
        url,
        "http://x/api/cses/bot-agent/teams/t1/members?page=2&per_page=50"
    );
    assert!(has_bot_auth(&h));

    // #26 users/{user_id}
    let (url, _b, h) = one_http(run(
        "im_bot_agent_get_user",
        json!({ "team_id": "t1", "user_id": "u1" }),
    ));
    assert_eq!(url, "http://x/api/cses/bot-agent/teams/t1/users/u1");
    assert!(has_bot_auth(&h));

    // #27 channel
    let (url, _b, h) = one_http(run("im_bot_agent_team_channel", json!({ "team_id": "t1" })));
    assert_eq!(url, "http://x/api/cses/bot-agent/teams/t1/channel");
    assert!(has_bot_auth(&h));
    assert!(err("im_bot_agent_team_channel", json!({}))); // 缺 team_id
}

/// bot-token 命令绝不在请求体出现 token 字面量（C6：token 留 driver，build 只标意图头）。
#[test]
fn bot_call_commands_body_has_no_token_literal() {
    let (_u, body, _h) = one_http(run(
        "im_bot_agent_send_channel",
        json!({ "team_id": "t", "channel_id": "c", "message": "m" }),
    ));
    let s = body.to_string();
    // 请求体不含鉴权 token / Authorization（意图头由 driver 据 X-Auth-Kind 注真凭据）
    assert!(!s.contains("Authorization") && !s.contains("Bearer"));
}

// ── agents 会话（#28-32，session）─────────────────────────────────────────────────────

#[test]
fn agent_create_join_start_send() {
    // #28 create — {name,type,description?,teamId,ownerUserId?}（全可选字段一并覆盖）
    let (url, body, _h) = one_http(run(
        "im_agent_create",
        json!({ "name": "A", "type": "llm", "team_id": "t", "description": "d", "owner_user_id": "o" }),
    ));
    assert_eq!(url, "http://x/api/cses/agents/create");
    assert_eq!(body["name"], "A");
    assert_eq!(body["type"], "llm");
    assert_eq!(body["teamId"], "t");
    assert_eq!(body["description"], "d");
    assert_eq!(body["ownerUserId"], "o");

    // #29 join-channel — {type,channelId,teamId}
    let (url, body, _h) = one_http(run(
        "im_agent_join_channel",
        json!({ "type": "llm", "channel_id": "c", "team_id": "t" }),
    ));
    assert_eq!(url, "http://x/api/cses/agents/join-channel");
    assert_eq!(body["channelId"], "c");

    // #30 start-session — {teamId,resultChannelId,agentTypes:[],...}
    let (url, body, _h) = one_http(run(
        "im_agent_start_session",
        json!({ "team_id": "t", "result_channel_id": "rc", "agent_types": ["a", "b"], "task_name": "tk" }),
    ));
    assert_eq!(url, "http://x/api/cses/agents/start-session");
    assert_eq!(body["resultChannelId"], "rc");
    assert_eq!(body["agentTypes"][0], "a");
    assert_eq!(body["taskName"], "tk");
    assert!(err(
        "im_agent_start_session",
        json!({ "team_id": "t", "result_channel_id": "rc" })
    )); // 缺 agent_types

    // #31 send — {sessionId,agentType,text,...}
    let (url, body, _h) = one_http(run(
        "im_agent_send",
        json!({ "session_id": "s", "agent_type": "llm", "text": "go", "channel_id": "c" }),
    ));
    assert_eq!(url, "http://x/api/cses/agents/send");
    assert_eq!(body["sessionId"], "s");
    assert_eq!(body["agentType"], "llm");
    assert_eq!(body["channelId"], "c");
}

#[test]
fn agent_callback_camel_body_with_usage_routing() {
    // #32 callback — {messageId,text,usage{...},routing{...},...}
    let (url, body, _h) = one_http(run(
        "im_agent_callback",
        json!({
            "message_id": "m", "text": "done", "session_id": "s", "agent_type": "llm",
            "commands": ["c1"], "usage": { "totalTokens": 10 }, "routing": { "channelId": "c" }
        }),
    ));
    assert_eq!(url, "http://x/api/cses/agents/callback");
    assert_eq!(body["messageId"], "m");
    assert_eq!(body["sessionId"], "s");
    assert_eq!(body["agentType"], "llm");
    assert_eq!(body["commands"][0], "c1");
    assert_eq!(body["usage"]["totalTokens"], 10);
    assert_eq!(body["routing"]["channelId"], "c");
}

/// #32 内存构造 post 不推 cursor：callback 请求体不含 cursor/event_seq（推 cursor 是 WS/落库态）。
#[test]
fn agent_callback_body_has_no_cursor_field() {
    let (_u, body, _h) = one_http(run(
        "im_agent_callback",
        json!({ "message_id": "m", "text": "t" }),
    ));
    let s = body.to_string();
    assert!(
        !s.contains("event_seq") && !s.contains("eventSeq") && !s.contains("cursor"),
        "agentCallback 内存构造 post 不推 cursor（gap-bot-agent.md:44）"
    );
}

// ── webhook config（#35-37，session，响应裸 map 非 CommonRes）────────────────────────

#[test]
fn webhook_config_set_get_test() {
    // #35 config POST — WebhookConfigRequest{targetUrl,secret,botUserIds:[],triggerMode,channelFilter?}
    let (url, body, _h) = one_http(run(
        "im_webhook_config_set",
        json!({
            "target_url": "http://w", "secret": "sec", "trigger_mode": "all",
            "bot_user_ids": ["b1"], "channel_filter": ["c1"]
        }),
    ));
    assert_eq!(url, "http://x/api/cses/webhook/config");
    assert_eq!(body["targetUrl"], "http://w");
    assert_eq!(body["secret"], "sec");
    assert_eq!(body["triggerMode"], "all");
    assert_eq!(body["botUserIds"][0], "b1");
    assert_eq!(body["channelFilter"][0], "c1");
    assert!(err(
        "im_webhook_config_set",
        json!({ "target_url": "w", "secret": "s", "trigger_mode": "m" })
    )); // 缺 bot_user_ids

    // #36 config GET — 空 body
    let (url, body, _h) = one_http(run("im_webhook_config_get", json!({})));
    assert_eq!(url, "http://x/api/cses/webhook/config");
    assert_eq!(body, json!({}));

    // #37 test — message 可选
    let (url, body, _h) = one_http(run("im_webhook_test", json!({ "message": "ping" })));
    assert_eq!(url, "http://x/api/cses/webhook/test");
    assert_eq!(body["message"], "ping");
    let (_u, body2, _h) = one_http(run("im_webhook_test", json!({})));
    assert_eq!(body2, json!({}), "message 可选，缺则空 body");
}

/// #35-37 响应裸 map（非 dto.CommonRes）属响应态——outbound 只 build 请求体，不含响应信封字段。
#[test]
fn webhook_responses_are_response_side_not_in_request_body() {
    let (_u, body, _h) = one_http(run("im_webhook_config_get", json!({})));
    let s = body.to_string();
    assert!(
        !s.contains("\"success\"") && !s.contains("\"configured\""),
        "裸 map 响应字段（success/configured）是响应态，不在 outbound 请求体"
    );
}

// ── registry 性质：35 命令全 is_outbound 命中 + 全 dispatchable + 排除项不注册 ──────────

/// P5b bot-agent 全集（35）。改这个列表必须同步 bot_agent/ 各注册文件。
/// 第三元 = 是否 bot-token 命令（X-Auth-Kind: bot）。
const ALL_P5B: &[(&str, fn() -> Value, bool)] = &[
    // gateway
    (
        "im_create_bot",
        || json!({ "session_id": "s", "text": "t" }),
        false,
    ),
    (
        "im_bot_callback",
        || json!({ "message_id": "m", "timestamp": 1_i64, "to": "user:u", "text": "t" }),
        false,
    ),
    // bot-manage
    ("im_bot_list_visible", || json!({}), false),
    ("im_bot_create", || json!({ "name": "n" }), false),
    (
        "im_bot_token_generate",
        || json!({ "bot_user_id": "b" }),
        false,
    ),
    ("im_bot_token_list", || json!({ "bot_user_id": "b" }), false),
    ("im_bot_token_revoke", || json!({ "token_id": "t" }), false),
    (
        "im_bot_scope_set",
        || json!({ "bot_user_id": "b", "scope_type": "s" }),
        false,
    ),
    (
        "im_bot_visible_user_add",
        || json!({ "bot_user_id": "b", "user_id": "u" }),
        false,
    ),
    (
        "im_bot_visible_user_remove",
        || json!({ "bot_user_id": "b", "user_id": "u" }),
        false,
    ),
    (
        "im_bot_visible_user_list",
        || json!({ "bot_user_id": "b" }),
        false,
    ),
    // config + channel
    (
        "im_bot_agent_config_create",
        || json!({ "config": { "botUserId": "b" } }),
        false,
    ),
    (
        "im_bot_agent_config_get",
        || json!({ "bot_user_id": "b" }),
        false,
    ),
    ("im_bot_agent_configs_enabled", || json!({}), false),
    (
        "im_bot_agent_config_delete",
        || json!({ "bot_user_id": "b" }),
        false,
    ),
    (
        "im_bot_agent_regenerate_token",
        || json!({ "bot_user_id": "b" }),
        false,
    ),
    (
        "im_bot_agent_test_webhook",
        || json!({ "bot_user_id": "b", "message": "m" }),
        false,
    ),
    (
        "im_bot_agent_channel_bots",
        || json!({ "channel_id": "c" }),
        false,
    ),
    (
        "im_bot_agent_channel_available_bots",
        || json!({ "channel_id": "c" }),
        false,
    ),
    (
        "im_bot_agent_channel_add",
        || json!({ "channel_id": "c", "bot_user_id": "b" }),
        false,
    ),
    (
        "im_bot_agent_channel_remove",
        || json!({ "channel_id": "c", "bot_user_id": "b" }),
        false,
    ),
    // calls (bot-token)
    ("im_bot_agent_info", || json!({}), true),
    (
        "im_bot_agent_send_direct",
        || json!({ "team_id": "t", "user_id": "u", "message": "m" }),
        true,
    ),
    (
        "im_bot_agent_send_channel",
        || json!({ "team_id": "t", "channel_id": "c", "message": "m" }),
        true,
    ),
    (
        "im_bot_agent_team_members",
        || json!({ "team_id": "t" }),
        true,
    ),
    (
        "im_bot_agent_get_user",
        || json!({ "team_id": "t", "user_id": "u" }),
        true,
    ),
    (
        "im_bot_agent_team_channel",
        || json!({ "team_id": "t" }),
        true,
    ),
    // agents
    (
        "im_agent_create",
        || json!({ "name": "n", "type": "t", "team_id": "t" }),
        false,
    ),
    (
        "im_agent_join_channel",
        || json!({ "type": "t", "channel_id": "c", "team_id": "t" }),
        false,
    ),
    (
        "im_agent_start_session",
        || json!({ "team_id": "t", "result_channel_id": "rc", "agent_types": ["a"] }),
        false,
    ),
    (
        "im_agent_send",
        || json!({ "session_id": "s", "agent_type": "a", "text": "t" }),
        false,
    ),
    (
        "im_agent_callback",
        || json!({ "message_id": "m", "text": "t" }),
        false,
    ),
    // webhook
    (
        "im_webhook_config_set",
        || json!({ "target_url": "w", "secret": "s", "trigger_mode": "m", "bot_user_ids": ["b"] }),
        false,
    ),
    ("im_webhook_config_get", || json!({}), false),
    ("im_webhook_test", || json!({}), false),
];

#[test]
fn registry_covers_all_p5b_commands() {
    assert_eq!(
        ALL_P5B.len(),
        35,
        "P5b bot-agent 应为 35 命令（排除 timeline/cost + demo）"
    );
    for (name, _, _) in ALL_P5B {
        assert!(is_outbound(name), "{name} 应被 registry 认领");
    }
    // 不与 P3a / P5a 命令名碰撞（registry 构建无重复才能全 dispatch）
    assert!(is_outbound("im_top20"), "P3a 命令仍在册");
    assert!(is_outbound("im_list_users"), "P5a 命令仍在册");
    assert!(!is_outbound("im_send_message"));
    assert!(!is_outbound("bogus_bot"));
}

/// 排除项（D7 废弃 / 非业务）绝不注册：timeline/cost + cross-repo demo。
#[test]
fn excluded_commands_are_not_registered() {
    for name in [
        "im_agent_timeline",
        "im_agent_cost",
        "im_cross_repo_demo",
        "im_cross_repo_counter_poll",
        "im_trigger_mm_to_cses_counter_ack",
    ] {
        assert!(!is_outbound(name), "{name} 是排除项（D7/非业务），不应注册");
    }
}

/// 注册数 == 命令数性质：每个命令成功 dispatch（无重复 name 致 registry 构建失败）+ auth 头一致。
#[test]
fn every_p5b_command_dispatchable_with_correct_auth() {
    for (name, mk, is_bot) in ALL_P5B {
        let effects = run(name, mk());
        let (_url, _body, headers) = one_http(effects);
        if *is_bot {
            assert!(
                has_bot_auth(&headers),
                "{name} 是 bot-token 命令，必带 X-Auth-Kind: bot"
            );
        } else {
            assert!(
                !has_any_auth_kind(&headers),
                "{name} 是 session 命令，不应带 X-Auth-Kind（driver 缺省 session）"
            );
        }
    }
}

/// handle_outbound 错误闭包路径（边界零信任，不 panic）：坏 JSON payload。
#[test]
fn handle_outbound_bad_payload_is_error_not_panic() {
    assert!(handle_outbound("im_create_bot", b"{not json", BASE, None, corr()).is_err());
}

/// 每条必填字段缺失 → Err（边界零信任，零 panic）。穷举每个 `require_str?`/`path_seg?` 失败分支
/// （驱动 region 覆盖到 100%：每个 `?` 的短路 Err region 都被命中）。每行 = (命令, 缺该字段的 payload)。
#[test]
fn every_required_field_missing_is_error() {
    let cases: &[(&str, Value)] = &[
        // gateway
        ("im_create_bot", json!({ "text": "t" })), // 缺 session_id
        ("im_create_bot", json!({ "session_id": "s" })), // 缺 text
        (
            "im_bot_callback",
            json!({ "timestamp": 1_i64, "to": "u", "text": "t" }),
        ), // 缺 message_id
        (
            "im_bot_callback",
            json!({ "message_id": "m", "timestamp": 1_i64, "to": "u" }),
        ), // 缺 text
        (
            "im_bot_callback",
            json!({ "message_id": "m", "timestamp": 1_i64, "text": "t" }),
        ), // 缺 to
        // bot-manage
        ("im_bot_create", json!({})),                        // 缺 name
        ("im_bot_token_generate", json!({})),                // 缺 bot_user_id
        ("im_bot_token_list", json!({})),                    // 缺 bot_user_id（path_seg）
        ("im_bot_token_revoke", json!({})),                  // 缺 token_id
        ("im_bot_scope_set", json!({ "scope_type": "s" })),  // 缺 bot_user_id
        ("im_bot_scope_set", json!({ "bot_user_id": "b" })), // 缺 scope_type
        ("im_bot_visible_user_list", json!({})),             // 缺 bot_user_id（path_seg）
        ("im_bot_visible_user_add", json!({ "user_id": "u" })), // 缺 bot_user_id（visible_user_body）
        ("im_bot_visible_user_remove", json!({ "user_id": "u" })), // 缺 bot_user_id
        // config + channel（path_seg + body）
        ("im_bot_agent_config_get", json!({})), // 缺 bot_user_id
        ("im_bot_agent_config_delete", json!({})), // 缺 bot_user_id
        ("im_bot_agent_regenerate_token", json!({})), // 缺 bot_user_id
        ("im_bot_agent_test_webhook", json!({ "bot_user_id": "b" })), // 缺 message
        ("im_bot_agent_test_webhook", json!({ "message": "m" })), // 缺 bot_user_id（path_seg）
        ("im_bot_agent_channel_bots", json!({})), // 缺 channel_id
        ("im_bot_agent_channel_available_bots", json!({})), // 缺 channel_id
        ("im_bot_agent_channel_add", json!({ "bot_user_id": "b" })), // 缺 channel_id
        ("im_bot_agent_channel_remove", json!({ "bot_user_id": "b" })), // 缺 channel_id
        // calls（mp_/bot token 轴）
        (
            "im_bot_agent_send_direct",
            json!({ "user_id": "u", "message": "m" }),
        ), // 缺 team_id
        (
            "im_bot_agent_send_direct",
            json!({ "team_id": "t", "user_id": "u" }),
        ), // 缺 message
        (
            "im_bot_agent_send_channel",
            json!({ "channel_id": "c", "message": "m" }),
        ), // 缺 team_id
        ("im_bot_agent_team_members", json!({})), // 缺 team_id（path_seg）
        ("im_bot_agent_get_user", json!({ "user_id": "u" })), // 缺 team_id（path_seg）
        ("im_bot_agent_get_user", json!({ "team_id": "t" })), // 缺 user_id（path_seg）
        ("im_bot_agent_team_channel", json!({})), // 缺 team_id（path_seg）
        // agents
        ("im_agent_create", json!({ "type": "t", "team_id": "t" })), // 缺 name
        ("im_agent_create", json!({ "name": "n", "team_id": "t" })), // 缺 type
        ("im_agent_create", json!({ "name": "n", "type": "t" })),    // 缺 team_id
        (
            "im_agent_join_channel",
            json!({ "channel_id": "c", "team_id": "t" }),
        ), // 缺 type
        (
            "im_agent_join_channel",
            json!({ "type": "t", "team_id": "t" }),
        ), // 缺 channel_id
        (
            "im_agent_join_channel",
            json!({ "type": "t", "channel_id": "c" }),
        ), // 缺 team_id
        (
            "im_agent_start_session",
            json!({ "result_channel_id": "rc", "agent_types": ["a"] }),
        ), // 缺 team_id
        (
            "im_agent_start_session",
            json!({ "team_id": "t", "agent_types": ["a"] }),
        ), // 缺 result_channel_id
        (
            "im_agent_start_session",
            json!({ "team_id": "t", "result_channel_id": "rc" }),
        ), // 缺 agent_types
        ("im_agent_send", json!({ "agent_type": "a", "text": "t" })), // 缺 session_id
        ("im_agent_send", json!({ "session_id": "s", "text": "t" })), // 缺 agent_type
        (
            "im_agent_send",
            json!({ "session_id": "s", "agent_type": "a" }),
        ), // 缺 text
        ("im_agent_callback", json!({ "text": "t" })),               // 缺 message_id
        ("im_agent_callback", json!({ "message_id": "m" })),         // 缺 text
        // webhook
        (
            "im_webhook_config_set",
            json!({ "secret": "s", "trigger_mode": "m", "bot_user_ids": ["b"] }),
        ), // 缺 target_url
        (
            "im_webhook_config_set",
            json!({ "target_url": "u", "trigger_mode": "m", "bot_user_ids": ["b"] }),
        ), // 缺 secret
        (
            "im_webhook_config_set",
            json!({ "target_url": "u", "secret": "s", "bot_user_ids": ["b"] }),
        ), // 缺 trigger_mode
        (
            "im_webhook_config_set",
            json!({ "target_url": "u", "secret": "s", "trigger_mode": "m" }),
        ), // 缺 bot_user_ids
    ];
    for (name, payload) in cases {
        assert!(
            err(name, payload.clone()),
            "{name} 缺必填字段应 Err（payload={payload}）"
        );
    }
}

/// 动态 path 段零信任：含 `/` 的 path 段 → Err（防越权，path_seg 校验）。逐一覆盖每个 path_seg 调用点。
#[test]
fn path_seg_rejects_slash_in_segment() {
    let cases: &[(&str, Value)] = &[
        ("im_bot_token_list", json!({ "bot_user_id": "a/b" })),
        ("im_bot_visible_user_list", json!({ "bot_user_id": "a/b" })),
        ("im_bot_agent_config_get", json!({ "bot_user_id": "a/b" })),
        ("im_bot_agent_channel_bots", json!({ "channel_id": "a/b" })),
        (
            "im_bot_agent_channel_add",
            json!({ "channel_id": "a/b", "bot_user_id": "b" }),
        ),
        ("im_bot_agent_team_members", json!({ "team_id": "a/b" })),
        (
            "im_bot_agent_get_user",
            json!({ "team_id": "a/b", "user_id": "u" }),
        ),
    ];
    for (name, payload) in cases {
        assert!(err(name, payload.clone()), "{name} path 段含 '/' 应 Err");
    }
}
