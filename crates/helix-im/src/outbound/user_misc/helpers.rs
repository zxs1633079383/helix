//! `user_misc.rs` 的 build 层 helper（从 user_misc.rs 外提，收口 user_misc.rs 行数基线）。
//!
//! 纯 build helper：承载必填字符串数组校验与 Search Gate wire body 组装，供 user_misc.rs
//! 命令闭包复用。边界零信任（缺/类型错/越权字段 → Err，不 panic）。

use serde_json::{json, Value};

use crate::error::ImError;

/// 取必填字符串数组（非空、元素全 string）。供 `{userIds:[]}` 类命令复用。
pub(crate) fn require_str_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 {key}（非空字符串数组）")))
}

const MAX_SEARCH_PAGE_SIZE: u64 = 50;
const MAX_SEARCH_CURSOR_BYTES: usize = 4096;
const MAX_SEARCH_REQ_ID_BYTES: usize = 128;
const MAX_SEARCH_TIMESTAMP: u64 = i64::MAX as u64;
const MAX_SEARCH_KEYWORD_CHARS: usize = 512;
const MAX_SEARCH_FILTER_VALUES: usize = 100;
const MAX_SEARCH_IDENTIFIER_BYTES: usize = 128;

/// 校验搜索 correlation 与服务端权威字段，防止前端越权指定用户或租户范围。
fn validate_search_meta(args: &Value, cmd: &str) -> Result<(), ImError> {
    args.get("req_id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.len() <= MAX_SEARCH_REQ_ID_BYTES)
        .ok_or_else(|| {
            ImError::Parse(format!(
                "{cmd}: req_id 必须为 1..={MAX_SEARCH_REQ_ID_BYTES} 字节的非空字符串"
            ))
        })?;

    for field in ["user_id", "team_id", "company_id", "sort"] {
        if args.get(field).is_some() {
            return Err(ImError::Parse(format!(
                "{cmd}: {field} 由服务端权威决定，禁止前端下发"
            )));
        }
    }
    Ok(())
}

/// 把可选 cursor/page_size 校验后写成 Go wire 的 camelCase 字段。
fn append_search_page(args: &Value, cmd: &str, body: &mut Value) -> Result<(), ImError> {
    if let Some(cursor) = args.get("cursor") {
        let cursor = cursor
            .as_str()
            .filter(|value| !value.is_empty() && value.len() <= MAX_SEARCH_CURSOR_BYTES)
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{cmd}: cursor 必须为 1..={MAX_SEARCH_CURSOR_BYTES} 字节的字符串"
                ))
            })?;
        body["cursor"] = json!(cursor);
    }
    if let Some(page_size) = args.get("page_size") {
        let page_size = page_size
            .as_u64()
            .filter(|value| (1..=MAX_SEARCH_PAGE_SIZE).contains(value))
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{cmd}: page_size 必须为 1..={MAX_SEARCH_PAGE_SIZE} 的整数"
                ))
            })?;
        body["pageSize"] = json!(page_size);
    }
    Ok(())
}

/// 读取非空字符串数组；字段存在时禁止空数组、空元素和非字符串元素。
fn optional_search_str_array(args: &Value, key: &str, cmd: &str) -> Result<Option<Value>, ImError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_array()
        .filter(|items| {
            !items.is_empty()
                && items.len() <= MAX_SEARCH_FILTER_VALUES
                && items
                    .iter()
                    .all(|item| {
                        item.as_str().is_some_and(|text| {
                            let text = text.trim();
                            !text.is_empty() && text.len() <= MAX_SEARCH_IDENTIFIER_BYTES
                        })
                    })
        })
        .cloned()
        .map(Value::Array)
        .map(Some)
        .ok_or_else(|| {
            ImError::Parse(format!(
                "{cmd}: {key} 必须为 1..={MAX_SEARCH_FILTER_VALUES} 个非空字符串，单值不超过 {MAX_SEARCH_IDENTIFIER_BYTES} 字节"
            ))
        })
}

fn validate_search_keyword(keyword: &str, cmd: &str) -> Result<(), ImError> {
    if keyword.chars().count() > MAX_SEARCH_KEYWORD_CHARS {
        return Err(ImError::Parse(format!(
            "{cmd}: keyword 不得超过 {MAX_SEARCH_KEYWORD_CHARS} 个字符"
        )));
    }
    Ok(())
}

/// Go 搜索接口使用 int64 时间戳；在 Helix 边界拒绝 u64 可表示但 Go 无法接收的值。
fn optional_search_timestamp(args: &Value, key: &str, cmd: &str) -> Result<Option<u64>, ImError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    value
        .as_u64()
        .filter(|value| *value <= MAX_SEARCH_TIMESTAMP)
        .map(Some)
        .ok_or_else(|| {
            ImError::Parse(format!(
                "{cmd}: {key} 必须为 0..={MAX_SEARCH_TIMESTAMP} 的整数"
            ))
        })
}

/// 构造消息搜索 wire body；权限范围留给 Go session，Helix 只传合法筛选与 opaque cursor。
fn post_search_body(args: &Value, cmd: &str) -> Result<Value, ImError> {
    if let Some(scope) = args.get("scope") {
        scope
            .as_str()
            .filter(|scope| matches!(*scope, "channel" | "global"))
            .ok_or_else(|| ImError::Parse(format!("{cmd}: scope 必须为 channel 或 global")))?;
    }
    let channel_id = match args.get("channel_id") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty() && value.len() <= MAX_SEARCH_IDENTIFIER_BYTES)
                .ok_or_else(|| {
                    ImError::Parse(format!(
                        "{cmd}: channel_id 必须为 1..={MAX_SEARCH_IDENTIFIER_BYTES} 字节的字符串"
                    ))
                })?,
        ),
    };
    let keyword = match args.get("keyword") {
        None => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or_else(|| ImError::Parse(format!("{cmd}: keyword 必须为字符串")))?
                .trim(),
        )
        .filter(|value| !value.is_empty()),
    };
    if let Some(keyword) = keyword {
        validate_search_keyword(keyword, cmd)?;
    }
    let sender_ids = optional_search_str_array(args, "sender_ids", cmd)?;
    let content_types = optional_search_str_array(args, "content_types", cmd)?;
    let start_at = optional_search_timestamp(args, "start_at", cmd)?;
    let end_at = optional_search_timestamp(args, "end_at", cmd)?;
    if start_at.zip(end_at).is_some_and(|(start, end)| start > end) {
        return Err(ImError::Parse(format!("{cmd}: start_at 不得晚于 end_at")));
    }
    if keyword.is_none()
        && channel_id.is_none()
        && sender_ids.is_none()
        && content_types.is_none()
        && start_at.is_none()
        && end_at.is_none()
    {
        return Err(ImError::Parse(format!(
            "{cmd}: keyword 为空时至少提供 sender_ids/content_types/时间筛选之一"
        )));
    }

    let mut body = json!({});
    if let Some(value) = keyword {
        body["keyword"] = json!(value);
    }
    if let Some(value) = channel_id {
        body["channelId"] = json!(value);
    }
    if let Some(value) = sender_ids {
        body["senderUserIds"] = value;
    }
    if let Some(value) = content_types {
        body["contentTypes"] = value;
    }
    if let Some(value) = start_at {
        body["startAt"] = json!(value);
    }
    if let Some(value) = end_at {
        body["endAt"] = json!(value);
    }
    append_search_page(args, cmd, &mut body)?;
    Ok(body)
}

/// 构造 user/channel/do 搜索 wire body；三者共享必填 keyword 与有界游标合同。
fn keyword_search_body(args: &Value, cmd: &str) -> Result<Value, ImError> {
    let keyword = args
        .get("keyword")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{cmd}: keyword 必须为非空字符串")))?;
    validate_search_keyword(keyword, cmd)?;
    let mut body = json!({ "keyword": keyword });
    append_search_page(args, cmd, &mut body)?;
    Ok(body)
}

/// 构造聚合搜索 wire body；三段结果各自翻页，禁止误用单 cursor。
fn global_search_body(args: &Value, cmd: &str) -> Result<Value, ImError> {
    if args.get("cursor").is_some() {
        return Err(ImError::Parse(format!(
            "{cmd}: 聚合搜索禁止 cursor，请使用三段独立 cursor"
        )));
    }
    let keyword = args
        .get("keyword")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{cmd}: keyword 必须为非空字符串")))?;
    validate_search_keyword(keyword, cmd)?;
    let mut body = json!({ "keyword": keyword });
    append_search_page(args, cmd, &mut body)?;
    for (field, wire) in [
        ("user_cursor", "userCursor"),
        ("channel_cursor", "channelCursor"),
        ("message_group_cursor", "messageGroupCursor"),
    ] {
        if let Some(cursor) = args.get(field) {
            let cursor = cursor
                .as_str()
                .filter(|value| !value.is_empty() && value.len() <= MAX_SEARCH_CURSOR_BYTES)
                .ok_or_else(|| {
                    ImError::Parse(format!(
                        "{cmd}: {field} 必须为 1..={MAX_SEARCH_CURSOR_BYTES} 字节的字符串"
                    ))
                })?;
            body[wire] = json!(cursor);
        }
    }
    Ok(body)
}

/// 组装四个 Search Gate 的 Go wire body；`req_id` 仅用于 Helix 读回 correlation，不下发。
pub(crate) fn do_search_body(args: &Value, cmd: &str) -> Result<Value, ImError> {
    validate_search_meta(args, cmd)?;
    match cmd {
        "im_search_post" => post_search_body(args, cmd),
        "im_search_user" | "im_search_channel" => keyword_search_body(args, cmd),
        "im_search_do" => global_search_body(args, cmd),
        _ => Err(ImError::Parse(format!("{cmd}: 未知搜索命令"))),
    }
}
