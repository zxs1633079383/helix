use std::collections::BTreeMap;

use helix_core::effect::{SqlValue, StorageOp};
use helix_core::Effect;
use serde_json::{json, Value};

const CHANNEL_ID: &str = "00000000000000000000000001";

pub fn canonical_effect_bytes(effects: &[Effect], sequence: u64) -> Result<Vec<u8>, String> {
    let [message_effect, cursor_effect, emit_effect] = effects else {
        return Err(format!(
            "PERF_CORRECTNESS_VIOLATION: seq={sequence} effect count/order mismatch"
        ));
    };
    let message = canonical_message_effect(message_effect, sequence)?;
    let cursor = canonical_cursor_effect(cursor_effect, sequence)?;
    let emit = canonical_emit_effect(emit_effect, sequence)?;
    serde_json::to_vec(&json!([message, cursor, emit]))
        .map_err(|error| format!("PERF_EVIDENCE_CORRUPT: canonical effect: {error}"))
}

fn canonical_message_effect(effect: &Effect, sequence: u64) -> Result<Value, String> {
    let Effect::PersistFire { ops } = effect else {
        return Err(mismatch(sequence, "effect[0] tag != PersistFire"));
    };
    let [StorageOp::BatchUpsert(spec)] = ops.as_slice() else {
        return Err(mismatch(sequence, "effect[0] op != BatchUpsert"));
    };
    if spec.table != "message" || spec.conflict_key != Some("temporary_id") {
        return Err(mismatch(sequence, "message table/conflict key mismatch"));
    }
    let [row] = spec.rows.as_slice() else {
        return Err(mismatch(sequence, "message upsert row count mismatch"));
    };
    require_text(row, "temporary_id", &format!("perf-{sequence}"), sequence)?;
    require_text(row, "channel_id", CHANNEL_ID, sequence)?;
    require_text(row, "send_status", "sent", sequence)?;
    require_integer(row, "create_at", 1_000_000 + sequence as i64, sequence)?;
    Ok(json!({
        "tag": "PersistFire",
        "op": "BatchUpsert",
        "table": spec.table,
        "conflictKey": spec.conflict_key,
        "excludeFromUpdate": spec.exclude_from_update,
        "rows": spec.rows.iter().map(|row| canonical_row(row)).collect::<Vec<_>>()
    }))
}

fn canonical_cursor_effect(effect: &Effect, sequence: u64) -> Result<Value, String> {
    let Effect::PersistFire { ops } = effect else {
        return Err(mismatch(sequence, "effect[1] tag != PersistFire"));
    };
    let [StorageOp::MonotonicUpsert(spec)] = ops.as_slice() else {
        return Err(mismatch(sequence, "effect[1] op != MonotonicUpsert"));
    };
    if spec.table != "channel_event_cursor"
        || spec.key_col != "channel_id"
        || spec.value_col != "last_event_seq"
        || spec.scope_key != CHANNEL_ID
        || spec.value != sequence as i64
    {
        return Err(mismatch(sequence, "cursor key/value payload mismatch"));
    }
    Ok(json!({
        "tag": "PersistFire",
        "op": "MonotonicUpsert",
        "table": spec.table,
        "keyCol": spec.key_col,
        "valueCol": spec.value_col,
        "touchCol": spec.touch_col,
        "scopeKey": spec.scope_key,
        "value": spec.value
    }))
}

fn canonical_emit_effect(effect: &Effect, sequence: u64) -> Result<Value, String> {
    let Effect::Emit { event } = effect else {
        return Err(mismatch(sequence, "effect[2] tag != Emit"));
    };
    let payload: Value = serde_json::from_slice(&event.0)
        .map_err(|error| mismatch(sequence, &format!("emit JSON invalid: {error}")))?;
    let expected_msg_id = format!("perf-{sequence}");
    if payload.get("event").and_then(Value::as_str) != Some("im:post:received")
        || payload.pointer("/data/channel_id").and_then(Value::as_str) != Some(CHANNEL_ID)
        || payload.pointer("/data/event_seq").and_then(Value::as_u64) != Some(sequence)
        || payload.pointer("/data/msg_id").and_then(Value::as_str) != Some(expected_msg_id.as_str())
    {
        return Err(mismatch(sequence, "emit event/key/payload mismatch"));
    }
    Ok(json!({"tag": "Emit", "payload": canonical_json(payload)}))
}

fn canonical_row(row: &[(String, SqlValue)]) -> Value {
    Value::Array(
        row.iter()
            .map(|(key, value)| json!({"key": key, "value": sql_value(value)}))
            .collect(),
    )
}

fn sql_value(value: &SqlValue) -> Value {
    match value {
        SqlValue::Text(value) => json!({"text": value}),
        SqlValue::Integer(value) => json!({"integer": value}),
        SqlValue::Real(value) => json!({"real": value}),
        SqlValue::Blob(value) => json!({"blob": value}),
        SqlValue::Null => Value::Null,
    }
}

fn canonical_json(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_json).collect()),
        value => value,
    }
}

fn require_text(
    row: &[(String, SqlValue)],
    key: &str,
    expected: &str,
    sequence: u64,
) -> Result<(), String> {
    if matches!(row.iter().find(|(name, _)| name == key), Some((_, SqlValue::Text(value))) if value == expected)
    {
        Ok(())
    } else {
        Err(mismatch(sequence, &format!("message.{key} mismatch")))
    }
}

fn require_integer(
    row: &[(String, SqlValue)],
    key: &str,
    expected: i64,
    sequence: u64,
) -> Result<(), String> {
    if matches!(row.iter().find(|(name, _)| name == key), Some((_, SqlValue::Integer(value))) if *value == expected)
    {
        Ok(())
    } else {
        Err(mismatch(sequence, &format!("message.{key} mismatch")))
    }
}

fn mismatch(sequence: u64, detail: &str) -> String {
    format!("PERF_CORRECTNESS_VIOLATION: seq={sequence} {detail}")
}
