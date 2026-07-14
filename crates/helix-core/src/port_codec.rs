//! Pure, runtime-free codecs shared by every platform driver.
//!
//! Drivers own I/O execution, while this module owns the bytes fed back through
//! `Tick::PortReply`. Keeping the wire shape here prevents native, FFI and Web
//! adapters from inventing subtly different HTTP or storage reply envelopes.

use bytes::Bytes;
use serde_json::{json, Value as JsonValue};

use crate::effect::{HttpResponse, Row, SqlValue};
use crate::error::PortError as DriverPortError;
use crate::tick::{PortError, PortOutcome, ReplyBytes};

/// Convert one completed HTTP port call into the canonical module-visible reply.
///
/// HTTP 4xx/5xx remain successful port calls. Their status is carried inside
/// the envelope so the business module, rather than the driver, decides how to
/// interpret the response.
pub fn http_result_to_outcome(result: Result<HttpResponse, DriverPortError>) -> PortOutcome {
    match result {
        Ok(response) => PortOutcome::Ok(ReplyBytes(encode_http_response(&response))),
        Err(error) => PortOutcome::Err(classify_port_error(error)),
    }
}

/// Encode the canonical HTTP response envelope.
///
/// Shape: `{"status":u16,"headers":[[name,value],...],"body":"base64"}`.
pub fn encode_http_response(response: &HttpResponse) -> Bytes {
    let value = json!({
        "status": response.status,
        "headers": response.headers,
        "body": base64_encode(&response.body),
    });
    serde_json::to_vec(&value)
        .map(Bytes::from)
        .unwrap_or_default()
}

/// Encode storage rows as the canonical JSON array used by `PortReply`.
pub fn rows_to_reply_bytes(rows: &[Row]) -> Bytes {
    let json_rows: Vec<JsonValue> = rows
        .iter()
        .map(|row| {
            let object = row
                .iter()
                .map(|(key, value)| {
                    let value = match value {
                        SqlValue::Text(value) => JsonValue::String(value.clone()),
                        SqlValue::Integer(value) => json!(value),
                        SqlValue::Real(value) => json!(value),
                        SqlValue::Blob(value) => json!(value),
                        SqlValue::Null => JsonValue::Null,
                    };
                    (key.clone(), value)
                })
                .collect();
            JsonValue::Object(object)
        })
        .collect();

    serde_json::to_vec(&json_rows)
        .map(Bytes::from)
        .unwrap_or_default()
}

/// Decode [`rows_to_reply_bytes`] output for driver compatibility paths.
pub fn rows_from_reply_bytes(bytes: &Bytes) -> Result<Vec<Row>, String> {
    let json: Vec<serde_json::Map<String, JsonValue>> =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;

    Ok(json
        .into_iter()
        .map(|object| {
            object
                .into_iter()
                .map(|(key, value)| {
                    let value = match value {
                        JsonValue::String(value) => SqlValue::Text(value),
                        JsonValue::Number(value) => value.as_i64().map_or_else(
                            || SqlValue::Real(value.as_f64().unwrap_or(0.0)),
                            SqlValue::Integer,
                        ),
                        JsonValue::Null => SqlValue::Null,
                        JsonValue::Array(values) => SqlValue::Blob(
                            values
                                .iter()
                                .filter_map(|value| value.as_u64().map(|value| value as u8))
                                .collect(),
                        ),
                        other => SqlValue::Text(other.to_string()),
                    };
                    (key, value)
                })
                .collect()
        })
        .collect())
}

/// Convert a driver-facing port failure into the stable module-visible class.
pub fn classify_port_error(error: DriverPortError) -> PortError {
    match &error {
        DriverPortError::Storage(_) => PortError::Storage(0),
        DriverPortError::Transport(message) if message.starts_with("timeout") => PortError::Timeout,
        DriverPortError::Transport(_) => PortError::Network,
        DriverPortError::Http(message) => PortError::Http(
            message
                .split_whitespace()
                .find_map(|word| word.parse::<u16>().ok())
                .unwrap_or(0),
        ),
        _ => PortError::Other(0),
    }
}

/// Standard padded base64 used by the HTTP response envelope.
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or_default() as u32;
        let third = chunk.get(2).copied().unwrap_or_default() as u32;
        let value = (first << 16) | (second << 8) | third;
        output.push(TABLE[((value >> 18) & 63) as usize] as char);
        output.push(TABLE[((value >> 12) & 63) as usize] as char);
        output.push(
            chunk
                .get(1)
                .map_or('=', |_| TABLE[((value >> 6) & 63) as usize] as char),
        );
        output.push(
            chunk
                .get(2)
                .map_or('=', |_| TABLE[(value & 63) as usize] as char),
        );
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_envelope_keeps_binary_body_and_repeated_headers() {
        let encoded = encode_http_response(&HttpResponse {
            status: 418,
            headers: vec![
                ("set-cookie".into(), "a=1".into()),
                ("set-cookie".into(), "b=2".into()),
            ],
            body: Bytes::from_static(&[0, 1, 2, 255]),
        });
        let value: JsonValue = serde_json::from_slice(&encoded).expect("valid envelope");
        assert_eq!(value["status"], 418);
        assert_eq!(value["headers"].as_array().map(Vec::len), Some(2));
        assert_eq!(value["body"], "AAEC/w==");
    }

    #[test]
    fn row_codec_round_trips_every_sql_value() {
        let rows = vec![vec![
            ("text".into(), SqlValue::Text("helix".into())),
            ("integer".into(), SqlValue::Integer(7)),
            ("real".into(), SqlValue::Real(1.5)),
            ("blob".into(), SqlValue::Blob(vec![0, 127, 255])),
            ("null".into(), SqlValue::Null),
        ]];
        let mut decoded = rows_from_reply_bytes(&rows_to_reply_bytes(&rows)).expect("row decode");
        let mut expected = rows;
        for row in &mut decoded {
            row.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        }
        for row in &mut expected {
            row.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        }
        assert_eq!(format!("{decoded:?}"), format!("{expected:?}"));
    }

    #[test]
    fn error_mapping_is_shared_by_all_drivers() {
        assert_eq!(
            classify_port_error(DriverPortError::Transport("timeout: slow".into())),
            PortError::Timeout
        );
        assert_eq!(
            classify_port_error(DriverPortError::Http("http status 503".into())),
            PortError::Http(503)
        );
    }
}
