//! G-05 转发消息的 Post 装配纯函数；与本地查询状态机分离。

use serde_json::{json, Value};

use crate::error::ImError;
use crate::forward::{ForwardAssembly, ForwardMode};
use crate::module::ImModule;

pub(crate) fn assemble_posts(
    module: &ImModule,
    assembly: &ForwardAssembly,
) -> Result<Vec<Value>, ImError> {
    match assembly.mode {
        ForwardMode::Individual => assembly
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| individual_post(module, assembly, row, index))
            .collect(),
        ForwardMode::Merged => Ok(vec![merged_post(module, assembly)?]),
    }
}

fn individual_post(
    module: &ImModule,
    assembly: &ForwardAssembly,
    row: &Value,
    index: usize,
) -> Result<Value, ImError> {
    let message = string_at(row, "message");
    let simple_message = nonempty_string_at(row, "simple_message")
        .unwrap_or_else(|| message.chars().take(50).collect());
    Ok(base_post(
        module,
        assembly,
        index,
        string_or(row, "type", "TEXT"),
        message,
        simple_message,
        json_column(row, "props", json!({})),
        json_column(row, "mentions", json!([])),
    ))
}

/// 合并 Post 封装最多一批来源行，并把权威频道标题固化进 durable props。
fn merged_post(module: &ImModule, assembly: &ForwardAssembly) -> Result<Value, ImError> {
    let source_channel_id = assembly
        .source_channel_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("merged forward omitted source channel".to_string()))?;
    let source_channel_title = assembly
        .source_channel_title
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("merged forward omitted source channel title".to_string()))?;
    let items: Vec<Value> = assembly
        .rows
        .iter()
        .map(|row| {
            json!({
                "id": string_at(row, "id"),
                "temporaryId": string_at(row, "temporary_id"),
                "userId": string_at(row, "user_id"),
                "props": json_column(row, "props", json!({})),
                "simpleMessage": string_at(row, "simple_message"),
                "type": string_or(row, "type", "TEXT"),
                "sendStatus": "sended",
                "userSnapshot": json_column(row, "user_snapshot", json!({})),
                "viewers": json_column(row, "viewers", json!(["all"])),
                "message": string_at(row, "message"),
                "createAt": row.get("create_at").and_then(Value::as_i64).unwrap_or_default(),
            })
        })
        .collect();
    let merge_message =
        serde_json::to_string(&items).map_err(|error| ImError::Serialize(error.to_string()))?;
    Ok(base_post(
        module,
        assembly,
        0,
        "MULTIPLY".to_string(),
        String::new(),
        "[聊天记录]".to_string(),
        json!({
            "forwardTitle": format!("{source_channel_title}的聊天记录"),
            "sourceChannelId": source_channel_id,
            "sourceChannelName": source_channel_title,
            "mergeMessage": merge_message,
        }),
        json!([]),
    ))
}

fn base_post(
    module: &ImModule,
    assembly: &ForwardAssembly,
    index: usize,
    msg_type: String,
    message: String,
    simple_message: String,
    props: Value,
    mentions: Value,
) -> Value {
    let identity = module.config.user_identity();
    json!({
        "id": "",
        "temporaryId": format!("relay-{}-{index}", assembly.req_id),
        "channelId": "",
        "userId": identity.user_id,
        "teamId": identity.team_id,
        "userSnapshot": {
            "userId": identity.user_id,
            "teamId": identity.team_id,
            "userName": identity.user_name,
            "orgName": identity.org_name,
            "deptName": identity.dept_name,
        },
        "type": msg_type,
        "message": message,
        "simpleMessage": simple_message,
        "props": props,
        "mentions": mentions,
        "viewers": ["all"],
        "topicId": "",
        "revoke": false,
        "createAt": assembly.now_ms.saturating_add(index as u64),
    })
}

fn string_at(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn nonempty_string_at(row: &Value, key: &str) -> Option<String> {
    row.get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn string_or(row: &Value, key: &str, fallback: &str) -> String {
    nonempty_string_at(row, key).unwrap_or_else(|| fallback.to_string())
}

fn json_column(row: &Value, key: &str, fallback: Value) -> Value {
    match row.get(key) {
        Some(Value::String(raw)) if !raw.is_empty() => {
            serde_json::from_str(raw).unwrap_or(fallback)
        }
        Some(value @ Value::Array(_)) | Some(value @ Value::Object(_)) => value.clone(),
        _ => fallback,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::module::ImConfig;

    /// 合并转发标题必须来自已读取的来源频道，且每项保留稳定消息与作者标识。
    #[test]
    fn merged_forward_persists_authoritative_source_title_and_item_ids() {
        let module = ImModule::new(ImConfig::default());
        let assembly = ForwardAssembly {
            req_id: "forward-1".to_string(),
            source_post_ids: vec!["message-1".to_string()],
            target_channel_ids: vec!["ch00000000000000000000000a".to_string()],
            mode: ForwardMode::Merged,
            next_index: 1,
            rows: vec![json!({
                "id":"message-1",
                "channel_id":"ch00000000000000000000000b",
                "user_id":"user-1",
                "type":"TEXT",
                "message":"正文",
                "simple_message":"正文",
                "create_at":10
            })],
            source_channel_id: Some("ch00000000000000000000000b".to_string()),
            source_channel_title: Some("研发群".to_string()),
            now_ms: 20,
        };

        let posts = assemble_posts(&module, &assembly).expect("merged forward closes");
        let props = &posts[0]["props"];
        assert_eq!(props["forwardTitle"], "研发群的聊天记录");
        assert_eq!(props["sourceChannelName"], "研发群");
        let items: Value = serde_json::from_str(props["mergeMessage"].as_str().unwrap()).unwrap();
        assert_eq!(items[0]["id"], "message-1");
        assert_eq!(items[0]["userId"], "user-1");
    }
}
