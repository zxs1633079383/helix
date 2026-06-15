//! Shared row reply serialization for driver PortReply payloads.

use bytes::Bytes;
use helix_core::effect::{Row, SqlValue};
use serde_json::{json, Value as JsonValue};

/// `&[Row]` -> JSON array bytes.
///
/// Schema:
/// `[{"col1": "str", "col2": 42, "col3": 3.14, "col4": null}, ...]`
pub fn rows_to_reply_bytes(rows: &[Row]) -> Bytes {
    let json_rows: Vec<JsonValue> = rows
        .iter()
        .map(|row| {
            let obj: serde_json::Map<String, JsonValue> = row
                .iter()
                .map(|(k, v)| {
                    let jv = match v {
                        SqlValue::Text(s) => JsonValue::String(s.clone()),
                        SqlValue::Integer(i) => json!(i),
                        SqlValue::Real(f) => json!(f),
                        // M2：Blob 编为 JSON 字节数组（与 Text/Number/Null 不歧义）
                        SqlValue::Blob(b) => json!(b),
                        SqlValue::Null => JsonValue::Null,
                    };
                    (k.clone(), jv)
                })
                .collect();
            JsonValue::Object(obj)
        })
        .collect();

    serde_json::to_vec(&json_rows)
        .map(Bytes::from)
        .unwrap_or_default()
}

/// JSON bytes -> `Vec<Row>` (`rows_to_reply_bytes` inverse).
pub fn rows_from_reply_bytes(bytes: &Bytes) -> Result<Vec<Row>, String> {
    let json: Vec<serde_json::Map<String, JsonValue>> =
        serde_json::from_slice(bytes).map_err(|e| e.to_string())?;

    Ok(json
        .into_iter()
        .map(|obj| {
            obj.into_iter()
                .map(|(k, v)| {
                    let sv = match v {
                        JsonValue::String(s) => SqlValue::Text(s),
                        JsonValue::Number(n) => {
                            if let Some(i) = n.as_i64() {
                                SqlValue::Integer(i)
                            } else {
                                SqlValue::Real(n.as_f64().unwrap_or(0.0))
                            }
                        }
                        JsonValue::Null => SqlValue::Null,
                        // M2：JSON 数组 = Blob 字节序列（rows_to_reply_bytes 的逆）
                        JsonValue::Array(a) => SqlValue::Blob(
                            a.iter()
                                .filter_map(|x| x.as_u64().map(|n| n as u8))
                                .collect(),
                        ),
                        other => SqlValue::Text(other.to_string()),
                    };
                    (k, sv)
                })
                .collect()
        })
        .collect())
}
