use helix_core::effect::{Row, ScanSpec, SqlValue};
use serde_json::Value;

use crate::ImError;

/// A5 删除同步 ABI 前的兼容名；真实 shape 定义只在 `query::core`。
pub type MobileQueryShape = crate::query::QueryResponseShape;

#[derive(Debug, Clone)]
pub enum MobileQueryPlan {
    Scan {
        spec: ScanSpec,
        shape: MobileQueryShape,
    },
}

pub fn supports(name: &str) -> bool {
    matches!(
        name,
        "im_query_messages_by_channel" | "im_query_dialog_list"
    )
}

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
    let reply_bytes =
        serde_json::to_vec(&rows_json).expect("SQL rows converted to JSON Value must serialize");
    match shape {
        MobileQueryShape::MessagesByChannel { channel_id } => {
            crate::query::core::message_query_result_bytes(&channel_id, &reply_bytes, "")
        }
        MobileQueryShape::DialogList => crate::query::core::dialog_list_result_bytes(&reply_bytes),
    }
}

fn message_query_plan(payload: &[u8]) -> Result<MobileQueryPlan, ImError> {
    let request = crate::query::parse_message_query(payload)?;

    Ok(MobileQueryPlan::Scan {
        spec: crate::query::message_scan_spec(&request),
        shape: MobileQueryShape::MessagesByChannel {
            channel_id: request.channel_id.as_str().to_string(),
        },
    })
}

fn dialog_list_plan(payload: &[u8]) -> Result<MobileQueryPlan, ImError> {
    let request = crate::query::parse_dialog_list_query(payload);

    Ok(MobileQueryPlan::Scan {
        spec: crate::query::dialog_list_scan_spec(request),
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
        let channel_id = "ch00000000000000000000000a";
        let plan = plan(
            "im_query_messages_by_channel",
            format!(r#"{{"channelId":"{channel_id}","limit":25}}"#).as_bytes(),
        )
        .expect("plan");
        match plan {
            MobileQueryPlan::Scan { spec, shape } => {
                assert_eq!(spec.table, "message");
                assert_eq!(spec.limit, Some(25));
                assert_eq!(
                    shape,
                    MobileQueryShape::MessagesByChannel {
                        channel_id: channel_id.into()
                    }
                );
                assert!(matches!(
                    spec.filter,
                    Some(("channel_id", SqlValue::Text(ref s))) if s == channel_id
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
                assert_eq!(spec.limit, Some(500));
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

    #[test]
    fn compatibility_shape_is_byte_identical_to_async_query_truth() {
        let channel_id = "ch00000000000000000000000a";
        let rows = vec![vec![
            ("id".into(), SqlValue::Text("server-new".into())),
            ("temporary_id".into(), SqlValue::Text("tmp-new".into())),
            ("channel_id".into(), SqlValue::Text(channel_id.into())),
            ("message".into(), SqlValue::Text("new".into())),
            ("create_at".into(), SqlValue::Integer(200)),
        ]];
        let compatibility = shape_json(
            MobileQueryShape::MessagesByChannel {
                channel_id: channel_id.into(),
            },
            rows.clone(),
        );
        let reply = serde_json::to_vec(&rows_to_json(rows)).unwrap();
        let async_truth = crate::query::core::message_query_result_bytes(channel_id, &reply, "");
        assert_eq!(compatibility, async_truth);
    }
}
