//! channel 读族补全 outbound 命令（HTTP-coverage 补齐，与 channel_change*/posts_read 零交集）。
//!
//! 纯**读** request/response：helix-im 只 build 正确 wire body；读族无 WS 回声、HTTP 响应体即数据
//! → `is_read=true`（spec06 缺陷A），dispatch 注册 `OutboundReadReply` 透传回灌前端（read_relay）。
//!
//! ## endpoint / body 真源（mattermost csesapi 逐字，full-map partials/2，不自造 fixture，C5）
//! - load/notice #23 `command.ChannelIdCommand`：`{channelId}` → data=ChannelNoticeSlice。
//! - load/postPinned #24 `command.ChannelPostPinnedCommand`（实际只用 ChannelId）：`{channelId}`
//!   → data=[]PostPinnedRes（嵌 ent.Post ToMap 全字段）。
//! - load/admin #25 `entity.ChannelIdStruct`：`{channelId}` → data=ChannelMemberEntitySlice。
//! - query #26 内联匿名 `entity.Channel + entity.PageOpts`：透传前端给的 channel 条件 + 分页 →
//!   data=[]ent.Channel。
//! - onlineStatus #28 内联 `{channelIds:[]string}` → data=[]ChannelOnlineStatusGroup。
//! - member/snapshot #6 `entity.GetMembersSnapshotParam`：`{channelId, startTime:int, endTime:int}`
//!   → data=[]GetMembersSnapshotDto。
//! - channels/member/byIds #5 内联 `{channelIds:[]string}`（len≤200）→ data=map[channelId][]IdWithCompanyExt。
//! - load/incrementByChannelId #2 内联 `{channelId}` → data=*IncrementChannel（单条，HTTP 直返不推送）。
//!
//! **casing 陷阱**：全部 camelCase（channelId/channelIds/startTime/endTime）。member/snapshot 的
//! startTime/endTime 是 int64（前端传毫秒数，非 string）。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

/// 把 Host 的 snake_case 条件键还原为 Go `entity.Channel` 使用的 lowerCamelCase wire 键。
fn channel_condition_wire_key(key: &str) -> String {
    let mut output = String::with_capacity(key.len());
    let mut uppercase_next = false;
    for character in key.chars() {
        if character == '_' {
            uppercase_next = true;
        } else if uppercase_next {
            output.extend(character.to_uppercase());
            uppercase_next = false;
        } else {
            output.push(character);
        }
    }
    output
}

/// 规范化频道查询条件的协议键；条件值保持调用方原值，不在 Helix 推导业务条件。
fn channel_query_condition(args: &Value) -> Value {
    let Some(condition) = args.get("condition").and_then(Value::as_object) else {
        return json!({});
    };
    Value::Object(
        condition
            .iter()
            .map(|(key, value)| (channel_condition_wire_key(key), value.clone()))
            .collect(),
    )
}

/// 取必填字符串数组（channelIds 类，非空、元素全 string）。
fn require_str_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 {key}（非空字符串数组）")))
}

/// 注册读命令样板（对齐 posts_read.rs::read_cmd!）：struct/impl/slice 三件套，`is_read=true` 全读族。
macro_rules! read_cmd {
    ($cmd_struct:ident, $reg:ident, $name:literal, $build:expr) => {
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
                true
            }
        }
        inventory::submit! {
            OutboundRegistration {
                name: $name,
                command: &$cmd_struct,
            }
        }
    };
}

// #23 load/notice：加载群公告。真源 ChannelIdCommand{channelId}。
read_cmd!(
    LoadNoticeCommand,
    LOAD_NOTICE_REG,
    "im_channel_load_notice",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        Ok(("channel/load/notice", json!({ "channelId": channel_id })))
    }
);

// #24 load/postPinned：加载置顶消息列表。真源 ChannelPostPinnedCommand（只用 channelId）。
read_cmd!(
    LoadPostPinnedCommand,
    LOAD_POST_PINNED_REG,
    "im_channel_load_post_pinned",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        Ok((
            "channel/load/postPinned",
            json!({ "channelId": channel_id }),
        ))
    }
);

// #25 load/admin：加载群管理员。真源 ChannelIdStruct{channelId}（userId/newCreator 此读路径不填）。
read_cmd!(
    LoadAdminCommand,
    LOAD_ADMIN_REG,
    "im_channel_load_admin",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        Ok(("channel/load/admin", json!({ "channelId": channel_id })))
    }
);

// #2 load/incrementByChannelId：单频道增量同步（HTTP 直返单条 IncrementChannel）。真源内联 {channelId}。
read_cmd!(
    LoadIncrementByChannelIdCommand,
    LOAD_INCREMENT_BY_CHANNEL_ID_REG,
    "im_channel_load_increment_by_channel_id",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        Ok((
            "channel/load/incrementByChannelId",
            json!({ "channelId": channel_id }),
        ))
    }
);

// #2b channels/load/increment：按父群请求话题频道增量，HTTP 只表示受理，结果经 WS end 收口。
read_cmd!(
    RequestChannelIncrementCommand,
    REQUEST_CHANNEL_INCREMENT_REG,
    "im_request_channel_increment",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        let request_id = args.get("sync_request_id").and_then(Value::as_str);
        let cursors = args.get("cursors").cloned().unwrap_or_else(|| json!([]));
        if !cursors.is_array() {
            return Err(ImError::Parse(format!("{cmd}: cursors 必须是数组")));
        }
        Ok((
            "channels/load/increment",
            json!({ "timeStamp": 0, "channelId": channel_id, "cursors": cursors, "syncRequestId": request_id }),
        ))
    }
);

// #26 query：条件分页查询频道。透传前端给的 channel 条件 map（已 camelCase）+ pageOpts。
// 真源内联匿名 struct{entity.Channel; entity.PageOpts} → 同层 merge（channel 字段与分页字段平铺）。
read_cmd!(
    ChannelQueryCommand,
    CHANNEL_QUERY_REG,
    "im_channel_query",
    |args, cmd| {
        // condition：前端构造的频道查询条件（object，平铺进顶层；空则不带条件字段）。
        let mut body = channel_query_condition(args);
        // 分页（pageNumber/pageSize/offset，缺省 0；前端 PageOpts 直传）。
        let obj = body
            .as_object_mut()
            .ok_or_else(|| ImError::Parse(format!("{cmd}: condition 必须是 object")))?;
        obj.insert(
            "pageNumber".to_string(),
            json!(args.get("page_number").and_then(Value::as_i64).unwrap_or(0)),
        );
        obj.insert(
            "pageSize".to_string(),
            json!(args.get("page_size").and_then(Value::as_i64).unwrap_or(0)),
        );
        obj.insert(
            "offset".to_string(),
            json!(args.get("offset").and_then(Value::as_i64).unwrap_or(0)),
        );
        Ok(("channel/query", body))
    }
);

// #28 onlineStatus：批量查频道在线状态。真源内联 {channelIds:[]string}。
read_cmd!(
    ChannelOnlineStatusCommand,
    CHANNEL_ONLINE_STATUS_REG,
    "im_channel_online_status",
    |args, cmd| {
        let channel_ids = require_str_array(args, "channel_ids", cmd)?;
        Ok(("channel/onlineStatus", json!({ "channelIds": channel_ids })))
    }
);

// #5 channels/member/byIds：批量按 channelIds 拉成员（len≤200）。真源内联 {channelIds:[]string}。
read_cmd!(
    MembersByIdsCommand,
    MEMBERS_BY_IDS_REG,
    "im_channels_members_by_ids",
    |args, cmd| {
        let channel_ids = require_str_array(args, "channel_ids", cmd)?;
        Ok((
            "channels/member/byIds",
            json!({ "channelIds": channel_ids }),
        ))
    }
);

// #6 member/snapshot：成员快照（时间范围）。真源 GetMembersSnapshotParam{channelId,startTime,endTime}。
// startTime/endTime int64 必填（Go *int64 Validate 非 nil；前端传毫秒数）。
read_cmd!(
    MemberSnapshotCommand,
    MEMBER_SNAPSHOT_REG,
    "im_channel_member_snapshot",
    |args, cmd| {
        let channel_id = require_str(args, "channel_id", cmd)?;
        let start_time = args
            .get("start_time")
            .and_then(Value::as_i64)
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺 start_time（int 毫秒）")))?;
        let end_time = args
            .get("end_time")
            .and_then(Value::as_i64)
            .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺 end_time（int 毫秒）")))?;
        Ok((
            "channel/member/snapshot",
            json!({ "channelId": channel_id, "startTime": start_time, "endTime": end_time }),
        ))
    }
);

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
