//! vote/score 第二网关 outbound 命令族（spec06 方案 A，5 命令）。
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbound::registry::{handle_outbound, is_outbound, is_read};
    use helix_core::{Correlation, Effect};

    /// 5 命令全注册 + 读写族正确（read=readVote/average-read；write=create/delete×2）。
    #[test]
    fn five_commands_registered_with_read_flags() {
        for n in [
            "im_vote_create",
            "im_vote_read",
            "im_vote_delete",
            "im_average_read",
            "im_average_delete",
        ] {
            assert!(is_outbound(n), "{n} 应注册");
        }
        assert!(is_read("im_vote_read"));
        assert!(is_read("im_average_read"));
        assert!(!is_read("im_vote_create"));
        assert!(!is_read("im_vote_delete"));
        assert!(!is_read("im_average_delete"));
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
