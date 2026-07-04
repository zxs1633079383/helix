use helix_core::effect::{Row, ScanSpec, SqlValue};
use serde_json::Value;

use crate::ImError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MobileQueryShape {
    MessagesByChannel { channel_id: String },
    DialogList,
}

#[derive(Debug, Clone)]
pub enum MobileQueryPlan {
    Scan {
        spec: ScanSpec,
        shape: MobileQueryShape,
    },
}

const QUERY_MESSAGES_DEFAULT: u32 = 50;
const QUERY_MESSAGES_MAX: u32 = 500;
const QUERY_DIALOGS_DEFAULT: u32 = 500;
const QUERY_DIALOGS_MAX: u32 = 2000;

pub fn plan(name: &str, payload: &[u8]) -> Result<MobileQueryPlan, ImError> {
    match name {
        "im_query_messages_by_channel" => message_query_plan(payload),
        "im_query_dialog_list" => dialog_list_plan(payload),
        _ => Err(ImError::Parse(format!(
            "unsupported mobile sync query '{name}'"
        ))),
    }
}

pub fn shape_json(shape: MobileQueryShape, rows: Vec<Row>) -> Vec<u8> {
    let rows_json = rows_to_json(rows);
    let payload = match shape {
        MobileQueryShape::MessagesByChannel { channel_id } => serde_json::json!({
            "event": "im:messages:query_result",
            "data": {
                "channel_id": channel_id,
                "messages": crate::render_ready::shape_message_rows(&rows_json),
            }
        }),
        MobileQueryShape::DialogList => serde_json::json!({
            "event": "im:channels:projection",
            "data": {
                "dialogList": shape_dialog_rows(rows_json),
            }
        }),
    };
    serde_json::to_vec(&payload).expect("mobile query JSON shape must serialize")
}

fn message_query_plan(payload: &[u8]) -> Result<MobileQueryPlan, ImError> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_query_messages_by_channel payload: {e}")))?;
    let channel_id = value
        .get("channelId")
        .or_else(|| value.get("channel_id"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ImError::Parse("im_query_messages_by_channel 缺 channelId".into()))?
        .to_string();
    let limit = value
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| (n as u32).clamp(1, QUERY_MESSAGES_MAX))
        .unwrap_or(QUERY_MESSAGES_DEFAULT);

    Ok(MobileQueryPlan::Scan {
        spec: ScanSpec {
            table: "message",
            limit: Some(limit),
            filter: Some(("channel_id", SqlValue::Text(channel_id.clone()))),
            order_desc_col: Some("create_at"),
        },
        shape: MobileQueryShape::MessagesByChannel { channel_id },
    })
}

fn dialog_list_plan(payload: &[u8]) -> Result<MobileQueryPlan, ImError> {
    let limit = if payload.is_empty() {
        QUERY_DIALOGS_DEFAULT
    } else {
        serde_json::from_slice::<Value>(payload)
            .ok()
            .and_then(|value| value.get("limit").and_then(Value::as_u64))
            .map(|n| (n as u32).clamp(1, QUERY_DIALOGS_MAX))
            .unwrap_or(QUERY_DIALOGS_DEFAULT)
    };

    Ok(MobileQueryPlan::Scan {
        spec: ScanSpec {
            table: "channel",
            limit: Some(limit),
            filter: None,
            order_desc_col: Some("last_post_at"),
        },
        shape: MobileQueryShape::DialogList,
    })
}

fn rows_to_json(rows: Vec<Row>) -> Value {
    Value::Array(
        rows.into_iter()
            .map(|row| {
                let mut object = serde_json::Map::with_capacity(row.len());
                for (key, value) in row {
                    object.insert(key, sql_value_to_json(value));
                }
                Value::Object(object)
            })
            .collect(),
    )
}

fn shape_dialog_rows(rows: Value) -> Value {
    match rows {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(crate::query::dialog_render::render_ready_dialog_row)
                .collect(),
        ),
        _ => Value::Array(Vec::new()),
    }
}

fn sql_value_to_json(value: SqlValue) -> Value {
    match value {
        SqlValue::Text(text) => Value::String(text),
        SqlValue::Integer(number) => Value::from(number),
        SqlValue::Real(number) => Value::from(number),
        SqlValue::Blob(bytes) => Value::Array(bytes.into_iter().map(Value::from).collect()),
        SqlValue::Null => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn message_query_plan_keeps_schema_inside_helix_im() {
        let plan = plan(
            "im_query_messages_by_channel",
            br#"{"channelId":"ch1","limit":25}"#,
        )
        .expect("plan");
        match plan {
            MobileQueryPlan::Scan { spec, shape } => {
                assert_eq!(spec.table, "message");
                assert_eq!(spec.limit, Some(25));
                assert_eq!(
                    shape,
                    MobileQueryShape::MessagesByChannel {
                        channel_id: "ch1".into()
                    }
                );
                assert!(matches!(
                    spec.filter,
                    Some(("channel_id", SqlValue::Text(ref s))) if s == "ch1"
                ));
            }
        }
    }

    #[test]
    fn dialog_query_plan_defaults_limit_and_hides_schema_in_helix_im() {
        let plan = plan("im_query_dialog_list", br#"{}"#).expect("plan");
        match plan {
            MobileQueryPlan::Scan { spec, shape } => {
                assert_eq!(spec.table, "channel");
                assert_eq!(spec.limit, Some(QUERY_DIALOGS_DEFAULT));
                assert!(spec.filter.is_none());
                assert_eq!(spec.order_desc_col, Some("last_post_at"));
                assert_eq!(shape, MobileQueryShape::DialogList);
            }
        }
    }

    #[test]
    fn shape_json_shapes_messages_and_dialogs_into_contract_events() {
        let message_bytes = shape_json(
            MobileQueryShape::MessagesByChannel {
                channel_id: "ch1".into(),
            },
            vec![vec![
                ("temporary_id".into(), SqlValue::Text("tmp-1".into())),
                ("message".into(), SqlValue::Text("hello".into())),
                ("channel_id".into(), SqlValue::Text("ch1".into())),
                ("create_at".into(), SqlValue::Integer(123)),
            ]],
        );
        let message_value: Value = serde_json::from_slice(&message_bytes).expect("json");
        assert_eq!(message_value["event"], "im:messages:query_result");
        assert_eq!(message_value["data"]["channel_id"], "ch1");
        assert_eq!(message_value["data"]["messages"][0]["temporaryId"], "tmp-1");
        assert_eq!(message_value["data"]["messages"][0]["text"], "hello");

        let dialog_bytes = shape_json(
            MobileQueryShape::DialogList,
            vec![vec![
                ("id".into(), SqlValue::Text("ch1".into())),
                ("display_name".into(), SqlValue::Text("General".into())),
                ("unread_count".into(), SqlValue::Integer(3)),
                ("mention_count".into(), SqlValue::Integer(1)),
            ]],
        );
        let dialog_value: Value = serde_json::from_slice(&dialog_bytes).expect("json");
        assert_eq!(dialog_value["event"], "im:channels:projection");
        assert_eq!(dialog_value["data"]["dialogList"][0]["id"], "ch1");
        assert_eq!(
            dialog_value["data"]["dialogList"][0]["displayName"],
            "General"
        );
        assert_eq!(dialog_value["data"]["dialogList"][0]["unread"], 3);
        assert_eq!(dialog_value["data"]["dialogList"][0]["mention"], true);
        assert_eq!(
            dialog_value["data"]["dialogList"][0]["lastMessage"],
            Value::Null
        );
        assert_eq!(dialog_value["data"]["dialogList"][0]["notice"], Value::Null);
        assert_eq!(dialog_value["data"]["dialogList"][0]["urgent"], false);
        assert_eq!(
            dialog_value["data"]["dialogList"][0]["mention_count"],
            json!(1)
        );
    }
}
