use super::*;

fn corr() -> Correlation {
    Correlation::from_raw(42)
}

fn scan_of(effect: Effect) -> Option<ScanSpec> {
    if let Effect::Persist { ops, .. } = effect {
        if let Some(StorageOp::Scan(spec)) = ops.into_iter().next() {
            return Some(spec);
        }
    }
    None
}

fn emit_json(effect: Effect) -> Option<serde_json::Value> {
    if let Effect::Emit { event } = effect {
        return serde_json::from_slice(event.0.as_ref()).ok();
    }
    None
}

#[test]
fn build_message_query_scans_message_with_channel_filter_and_desc_order() {
    let payload = serde_json::to_vec(&serde_json::json!({
        "channel_id": "ch00000000000000000000000a",
        "limit": 30
    }))
    .unwrap();
    let (cid, effect) = build_message_query(&payload, corr()).expect("build ok");
    assert_eq!(cid.as_str(), "ch00000000000000000000000a");
    let spec = scan_of(effect).expect("Persist+Scan");
    assert_eq!(spec.table, "message");
    assert_eq!(spec.limit, Some(31));
    assert!(matches!(
        &spec.filter,
        Some(("channel_id", SqlValue::Text(s))) if s == "ch00000000000000000000000a"
    ));
    assert_eq!(spec.order_by, &[ScanOrder::desc("create_at")]);
}

#[test]
/// 证明 Timeline 查询缺省 20，并对越界页长 fail closed。
fn build_message_query_defaults_and_rejects_out_of_range_limit() {
    let default_payload =
        serde_json::to_vec(&serde_json::json!({"channel_id":"ch00000000000000000000000a"}))
            .unwrap();
    let (_, default_effect) = build_message_query(&default_payload, corr()).unwrap();
    assert_eq!(
        scan_of(default_effect).expect("default scan").limit,
        Some(QUERY_MESSAGES_DEFAULT + 1)
    );

    let invalid_payload = serde_json::to_vec(
        &serde_json::json!({"channel_id":"ch00000000000000000000000a","limit":9999}),
    )
    .unwrap();
    assert!(build_message_query(&invalid_payload, corr()).is_err());
}

#[test]
fn message_query_keeps_the_explicit_timeline_window_token() {
    let payload = serde_json::to_vec(&serde_json::json!({
        "channel_id": "ch00000000000000000000000a",
        "windowToken": "locate-message-42",
    }))
    .unwrap();
    let request = parse_message_query(&payload).expect("valid timeline slot token");
    assert_eq!(request.window_token, "locate-message-42");

    let empty = serde_json::to_vec(&serde_json::json!({
        "channel_id": "ch00000000000000000000000a",
        "windowToken": "",
    }))
    .unwrap();
    assert!(matches!(
        parse_message_query(&empty),
        Err(ImError::Parse(_))
    ));
}

#[test]
fn build_message_query_rejects_missing_or_invalid_channel_id() {
    let missing = serde_json::to_vec(&serde_json::json!({"limit": 10})).unwrap();
    assert!(matches!(
        build_message_query(&missing, corr()),
        Err(ImError::Parse(_))
    ));
    assert!(matches!(
        build_message_query(b"not json{", corr()),
        Err(ImError::Parse(_))
    ));
    let invalid = serde_json::to_vec(&serde_json::json!({"channel_id": "too_short"})).unwrap();
    assert!(matches!(
        build_message_query(&invalid, corr()),
        Err(ImError::Parse(_))
    ));
}

#[test]
fn emit_message_query_result_wraps_rows_under_messages() {
    let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
    let rows = serde_json::to_vec(&serde_json::json!([
        {"temporary_id":"t-new","message":"new","create_at":200},
        {"temporary_id":"t-old","message":"old","create_at":100}
    ]))
    .unwrap();
    let value = emit_json(emit_message_query_result(&channel_id, &rows)).expect("emit");
    assert_eq!(value["event"], "im:messages:query_result");
    assert_eq!(value["data"]["channel_id"], "ch00000000000000000000000a");
    assert_eq!(value["data"]["messages"][0]["temporaryId"], "t-old");
    assert_eq!(value["data"]["messages"][0]["msgId"], "t-old");
    assert_eq!(value["data"]["messages"][0]["text"], "old");
    assert_eq!(value["data"]["messages"][1]["temporaryId"], "t-new");
    assert_eq!(value["data"]["messages"][1]["createAt"], 200);
}

#[test]
fn emit_message_query_result_invalid_reply_yields_empty_messages() {
    let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
    let value = emit_json(emit_message_query_result(&channel_id, b"garbage{")).expect("emit");
    assert_eq!(value["data"]["messages"], serde_json::json!([]));
}
