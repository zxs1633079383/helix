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

// 10 命令注册 / body / 读写族单测外提到 sibling（守 ≤300 硬顶；测试外提范本）。
#[cfg(test)]
#[path = "vote_score_tests.rs"]
mod tests;
