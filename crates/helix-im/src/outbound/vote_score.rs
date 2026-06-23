//! vote/score 第二网关 outbound 命令族（spec06 方案 A → vote-score-5 扩到 10 命令）。
//!
//! ## 为什么走第二网关（`Gateway::Default`）
//!
//! 现网 vote/score 走**默认业务网关** `this.api.post$`（base = `env.restHost`，**无** `/api/cses`
//! 前缀），endpoint `/vote/*` `/average/*`；而既有 88 命令走 **IM 网关** `imHttp`（base 含
//! `/api/cses`）。两者 base host 不同 → 本族 override `gateway() -> Gateway::Default`，由 host 经
//! `ImConfig.default_api_base_url` 注入第二 base（HX-C001：core/im 零 host 硬编码，base 仍 host 注入）。
//!
//! ## endpoint / body 真源（cses-client message.service.ts 951-1043，param type 34-100）
//!
//! - createVote：字段多（fromUserId/fromUserName/title/content/votes/isReal/finishTime/options[]/
//!   orgIds[]/joinCount/source?）→ **整 args 透传**（前端 bridge 已组好 camelCase wire body，
//!   逐字段重列易漏/错；透传更稳）。校验「非空 object」即放行（边界零信任，不 panic）。
//! - readVote/deleteVote → `{ id }`；average/read·average/delete → `{ id }`（id 必填）。
//!
//! ## vote-score-5 扩充（caller = message-vote / average-content.component）
//!
//! 第二批 5 个写命令（spec：VoteSubmitParams/VoteCloseParams/AverageAttendParams/
//! AverageCloseParams/AveragePublishParams，真源 message.service.ts 29-110）：
//! - vote/vote `{id, postId?, indexes:string[]}`（提交投票）—— id+indexes 必填，postId 可选透传。
//! - vote/closeVote `{id}`（截止投票）—— 复用 `id_body`。
//! - average/attend `{id, score:number, postId?}`（提交评分）—— id 必填、score 必为 number、postId 可选。
//! - average/close `{id, postId?}`（截止评分）—— id 必填、postId 可选。
//! - average/publish `{title, content, maxScore, minScore, isDelMaxMin, isAnonymous, cutoff,
//!   members, hasDecimal?, decimalPlaces?, source?}`（发布评分）—— 字段多同 createVote → **整 args 透传**。
//!
//! 全 5 命令均**写族**（`is_read=false`：写后无 HTTP 数据体回灌，靠 WS 回声 / 前端自刷）。
//!
//! ## 读写族（spec06 缺陷A）
//!
//! readVote / average/read = **读族**（`is_read=true`，无 WS 回声、HTTP 响应体即数据 → dispatch
//! 注册 `OutboundReadReply` 回灌前端）；create/delete = 写族（默认 `is_read=false`）。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use super::registry::{
    require_str, Gateway, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS,
};

/// 注册第二网关命令样板：`name` + `build` 闭包 → struct/static/slice 三件套。
/// `$read`（bool 字面量）= is_read（读族 true / 写族 false）；`gateway()` 全族恒为 `Default`。
macro_rules! vote_cmd {
    ($cmd_struct:ident, $reg:ident, $name:literal, $read:literal, $build:expr) => {
        struct $cmd_struct;
        impl OutboundCommand for $cmd_struct {
            fn name(&self) -> &'static str {
                $name
            }
            fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
                let f: fn(&Value, &'static str) -> Result<(&'static str, Value), ImError> = $build;
                f(args, $name)
            }
            fn is_read(&self) -> bool {
                $read
            }
            // 全族走第二网关（spec06 方案 A）：base = ImConfig.default_api_base_url（env.restHost）。
            fn gateway(&self) -> Gateway {
                Gateway::Default
            }
        }
        #[distributed_slice(OUTBOUND_COMMANDS)]
        static $reg: OutboundRegistration = OutboundRegistration {
            name: $name,
            command: &$cmd_struct,
        };
    };
}

/// 取必填 `id` 字段并产 `{ "id": <id> }` body（readVote/deleteVote/average 三命令复用）。
fn id_body(args: &Value, cmd: &'static str) -> Result<Value, ImError> {
    let id = require_str(args, "id", cmd)?;
    Ok(json!({ "id": id }))
}

/// 把可选字段 `key` 原样透传进 `body`（仅当入参存在且非 null 时；保留前端 wire 原值，
/// 不强转类型——边界零信任只校验「存在」，类型由前端 bridge 保证）。
fn carry_optional(args: &Value, body: &mut serde_json::Map<String, Value>, key: &str) {
    if let Some(v) = args.get(key) {
        if !v.is_null() {
            body.insert(key.to_string(), v.clone());
        }
    }
}

// #1 createVote：发起投票。字段多 → 整 args 透传（前端 bridge 已组 camelCase wire body）。
// 校验非空 object（缺 args / 非 object → Err，边界零信任不 panic）。
vote_cmd!(
    VoteCreateCommand,
    VOTE_CREATE_REG,
    "im_vote_create",
    false,
    |args, cmd| {
        let obj = args
            .as_object()
            .filter(|o| !o.is_empty())
            .ok_or_else(|| ImError::Parse(format!("{cmd}: args 须为非空 object")))?;
        Ok(("vote/createVote", Value::Object(obj.clone())))
    }
);

// #2 readVote：读投票详情（读族）。真源 { id }。
vote_cmd!(
    VoteReadCommand,
    VOTE_READ_REG,
    "im_vote_read",
    true,
    |args, cmd| { Ok(("vote/readVote", id_body(args, cmd)?)) }
);

// #3 deleteVote：删投票（写族）。真源 { id }。
vote_cmd!(
    VoteDeleteCommand,
    VOTE_DELETE_REG,
    "im_vote_delete",
    false,
    |args, cmd| { Ok(("vote/deleteVote", id_body(args, cmd)?)) }
);

// #4 average/read：读评分（读族）。真源 { id }。
vote_cmd!(
    AverageReadCommand,
    AVERAGE_READ_REG,
    "im_average_read",
    true,
    |args, cmd| { Ok(("average/read", id_body(args, cmd)?)) }
);

// #5 average/delete：删评分（写族）。真源 { id }。
vote_cmd!(
    AverageDeleteCommand,
    AVERAGE_DELETE_REG,
    "im_average_delete",
    false,
    |args, cmd| { Ok(("average/delete", id_body(args, cmd)?)) }
);

// ── vote-score-5 扩充（5 写命令）─────────────────────────────────────────────

// #6 vote/vote：提交投票（写族）。真源 VoteSubmitParams { id, postId?, indexes:string[] }。
// id 必填；indexes 必填且须为 array（边界零信任，缺/类型错 → Err）；postId 可选透传。
vote_cmd!(
    VoteDoCommand,
    VOTE_DO_REG,
    "im_vote_do",
    false,
    |args, cmd| {
        let id = require_str(args, "id", cmd)?;
        let indexes = args
            .get("indexes")
            .filter(|v| v.is_array())
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/非 array 必填字段 'indexes'")))?;
        let mut body = serde_json::Map::new();
        body.insert("id".to_string(), json!(id));
        body.insert("indexes".to_string(), indexes.clone());
        carry_optional(args, &mut body, "postId");
        Ok(("vote/vote", Value::Object(body)))
    }
);

// #7 vote/closeVote：截止投票（写族）。真源 VoteCloseParams { id }。复用 id_body。
vote_cmd!(
    VoteCloseCommand,
    VOTE_CLOSE_REG,
    "im_vote_close",
    false,
    |args, cmd| { Ok(("vote/closeVote", id_body(args, cmd)?)) }
);

// #8 average/attend：提交评分（写族）。真源 AverageAttendParams { id, score:number, postId? }。
// id 必填；score 必填且须为 number；postId 可选透传。
vote_cmd!(
    AverageAttendCommand,
    AVERAGE_ATTEND_REG,
    "im_average_attend",
    false,
    |args, cmd| {
        let id = require_str(args, "id", cmd)?;
        let score = args
            .get("score")
            .filter(|v| v.is_number())
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/非 number 必填字段 'score'")))?;
        let mut body = serde_json::Map::new();
        body.insert("id".to_string(), json!(id));
        body.insert("score".to_string(), score.clone());
        carry_optional(args, &mut body, "postId");
        Ok(("average/attend", Value::Object(body)))
    }
);

// #9 average/close：截止评分（写族）。真源 AverageCloseParams { id, postId? }。
// id 必填；postId 可选透传。
vote_cmd!(
    AverageCloseCommand,
    AVERAGE_CLOSE_REG,
    "im_average_close",
    false,
    |args, cmd| {
        let id = require_str(args, "id", cmd)?;
        let mut body = serde_json::Map::new();
        body.insert("id".to_string(), json!(id));
        carry_optional(args, &mut body, "postId");
        Ok(("average/close", Value::Object(body)))
    }
);

// #10 average/publish：发布评分（写族）。真源 AveragePublishParams 字段多
// （title/content/maxScore/minScore/isDelMaxMin/isAnonymous/cutoff/members/hasDecimal?/
// decimalPlaces?/source?）→ 整 args 透传（同 createVote，逐字段重列易漏；透传更稳）。
// 校验非空 object（缺 args / 非 object → Err，边界零信任不 panic）。
vote_cmd!(
    AveragePublishCommand,
    AVERAGE_PUBLISH_REG,
    "im_average_publish",
    false,
    |args, cmd| {
        let obj = args
            .as_object()
            .filter(|o| !o.is_empty())
            .ok_or_else(|| ImError::Parse(format!("{cmd}: args 须为非空 object")))?;
        Ok(("average/publish", Value::Object(obj.clone())))
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbound::registry::{handle_outbound, is_outbound, is_read};
    use helix_core::{Correlation, Effect};

    /// 10 命令全注册 + 读写族正确（read=readVote/average-read；其余 8 写族）。
    #[test]
    fn ten_commands_registered_with_read_flags() {
        for n in [
            "im_vote_create",
            "im_vote_read",
            "im_vote_delete",
            "im_average_read",
            "im_average_delete",
            "im_vote_do",
            "im_vote_close",
            "im_average_attend",
            "im_average_close",
            "im_average_publish",
        ] {
            assert!(is_outbound(n), "{n} 应注册");
        }
        // 仅两个读族（HTTP 响应体即数据 → 回灌前端）。
        assert!(is_read("im_vote_read"));
        assert!(is_read("im_average_read"));
        // 其余 8 全写族（写后无数据体回灌，靠 WS 回声 / 前端自刷）。
        for n in [
            "im_vote_create",
            "im_vote_delete",
            "im_average_delete",
            "im_vote_do",
            "im_vote_close",
            "im_average_attend",
            "im_average_close",
            "im_average_publish",
        ] {
            assert!(!is_read(n), "{n} 应为写族");
        }
    }

    /// 各写命令走第二网关（base = default_api_base_url，非 /api/cses IM 网关）。
    fn build_http(cmd: &str, body: Value) -> (String, Value) {
        let corr = Correlation::from_raw(9);
        let payload = serde_json::to_vec(&body).unwrap();
        let effects = handle_outbound(
            cmd,
            &payload,
            "http://h/api/cses",
            "http://localhost:3399",
            None,
            corr,
        )
        .unwrap_or_else(|e| panic!("{cmd} dispatch: {e:?}"));
        match &effects[0] {
            Effect::Http { req, .. } => {
                let parsed: Value =
                    serde_json::from_slice(req.body.as_ref().unwrap().as_ref()).unwrap();
                (req.url.clone(), parsed)
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    /// vote/vote：id+indexes 进 body，postId 存在则透传。
    #[test]
    fn vote_do_body_with_optional_post_id() {
        let (url, body) = build_http(
            "im_vote_do",
            json!({ "id": "v1", "indexes": ["0", "2"], "postId": "p9" }),
        );
        assert_eq!(url, "http://localhost:3399/vote/vote");
        assert_eq!(body["id"], json!("v1"));
        assert_eq!(body["indexes"], json!(["0", "2"]));
        assert_eq!(body["postId"], json!("p9"));
    }

    /// vote/vote：postId 缺省时 body 不含该键（不写 null/空）。
    #[test]
    fn vote_do_omits_absent_post_id() {
        let (_, body) = build_http("im_vote_do", json!({ "id": "v1", "indexes": ["0"] }));
        assert!(body.get("postId").is_none(), "缺省 postId 不应入 body");
    }

    /// vote/vote：indexes 缺失/非 array → Err（边界零信任）。
    #[test]
    fn vote_do_missing_indexes_errs() {
        let corr = Correlation::from_raw(3);
        let payload = serde_json::to_vec(&json!({ "id": "v1" })).unwrap();
        assert!(handle_outbound(
            "im_vote_do",
            &payload,
            "http://h/api/cses",
            "http://localhost:3399",
            None,
            corr,
        )
        .is_err());
    }

    /// vote/closeVote：{ id } body。
    #[test]
    fn vote_close_id_body() {
        let (url, body) = build_http("im_vote_close", json!({ "id": "v7" }));
        assert_eq!(url, "http://localhost:3399/vote/closeVote");
        assert_eq!(body, json!({ "id": "v7" }));
    }

    /// average/attend：id+score(number) 进 body，postId 可选透传。
    #[test]
    fn average_attend_body_with_score() {
        let (url, body) = build_http(
            "im_average_attend",
            json!({ "id": "a1", "score": 4.5, "postId": "p3" }),
        );
        assert_eq!(url, "http://localhost:3399/average/attend");
        assert_eq!(body["id"], json!("a1"));
        assert_eq!(body["score"], json!(4.5));
        assert_eq!(body["postId"], json!("p3"));
    }

    /// average/attend：score 非 number（字符串）→ Err。
    #[test]
    fn average_attend_non_number_score_errs() {
        let corr = Correlation::from_raw(4);
        let payload = serde_json::to_vec(&json!({ "id": "a1", "score": "5" })).unwrap();
        assert!(handle_outbound(
            "im_average_attend",
            &payload,
            "http://h/api/cses",
            "http://localhost:3399",
            None,
            corr,
        )
        .is_err());
    }

    /// average/close：id 必填，postId 缺省不入 body。
    #[test]
    fn average_close_omits_absent_post_id() {
        let (url, body) = build_http("im_average_close", json!({ "id": "a9" }));
        assert_eq!(url, "http://localhost:3399/average/close");
        assert_eq!(body, json!({ "id": "a9" }));
    }

    /// average/publish：字段多 → 整 args 透传（不丢字段）。
    #[test]
    fn average_publish_passthrough_body() {
        let (url, body) = build_http(
            "im_average_publish",
            json!({
                "title": "Q3 评分",
                "content": "请评分",
                "maxScore": "100",
                "minScore": "0",
                "isDelMaxMin": true,
                "isAnonymous": false,
                "cutoff": 1700000000,
                "members": [{ "userId": "u1", "userName": "Bob" }],
            }),
        );
        assert_eq!(url, "http://localhost:3399/average/publish");
        assert_eq!(body["title"], json!("Q3 评分"));
        assert_eq!(
            body["members"],
            json!([{ "userId": "u1", "userName": "Bob" }])
        );
        assert_eq!(body["isDelMaxMin"], json!(true));
    }

    /// createVote 整 args 透传：body 逐字段等于入参（不丢字段）。
    #[test]
    fn create_vote_passthrough_body() {
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({
            "fromUserId": "u1",
            "fromUserName": "Alice",
            "title": "lunch?",
            "options": ["A", "B"],
            "isReal": false,
        }))
        .unwrap();
        let effects = handle_outbound(
            "im_vote_create",
            &payload,
            "http://h/api/cses",
            "http://localhost:3399",
            None,
            corr,
        )
        .expect("create dispatch");
        match &effects[0] {
            Effect::Http { req, .. } => {
                assert_eq!(req.url, "http://localhost:3399/vote/createVote");
                let body: Value =
                    serde_json::from_slice(req.body.as_ref().unwrap().as_ref()).unwrap();
                assert_eq!(body["fromUserName"], json!("Alice"));
                assert_eq!(body["options"], json!(["A", "B"]));
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    /// id 缺失 → Err（不 panic，边界零信任）。
    #[test]
    fn read_vote_missing_id_errs() {
        let corr = Correlation::from_raw(2);
        let payload = serde_json::to_vec(&json!({})).unwrap();
        assert!(handle_outbound(
            "im_vote_read",
            &payload,
            "http://h/api/cses",
            "http://localhost:3399",
            None,
            corr,
        )
        .is_err());
    }
}
