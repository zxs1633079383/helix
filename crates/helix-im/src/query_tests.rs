//! `query.rs` 投影/状态查询命令单测外提 sibling（structure-gate「测试外提」范本，
//! 对齐 `state_tests.rs` / `module_tests.rs`）。
//!
//! 从 `query.rs` 抽出守 ≤300 硬顶（结构闸 §1）——UC-3 `im_query_dialog_list` 投影命令
//! 加入后内联 tests 撑爆 388 行；drift-review 收敛铁律：**绝不上调 baseline 抹 FAIL**，
//! 测试外提回正。`super::` 指向 `query` 模块，故 `is_query`/`build_*`/`emit_*` 路径与内联时一致。

use super::*;

fn corr() -> Correlation {
    Correlation::from_raw(42)
}

/// 取 Effect::Persist 的第一条 Scan spec（Option：非该形态 → None，调用方 `.expect`）。
/// 不含 never-taken panic 分支——以 `matches!`+`if let` 收敛，covered。
fn scan_of(effect: Effect) -> Option<ScanSpec> {
    if let Effect::Persist { ops, .. } = effect {
        if let Some(StorageOp::Scan(s)) = ops.into_iter().next() {
            return Some(s);
        }
    }
    None
}

/// 取 Effect::Emit 载荷并解成 JSON（Option：非 Emit → None，调用方 `.expect`）。
fn emit_json(effect: Effect) -> Option<serde_json::Value> {
    if let Effect::Emit { event } = effect {
        return serde_json::from_slice(event.0.as_ref()).ok();
    }
    None
}

/// 覆盖 helper 的 None 分支（消除 never-taken 行）：传错 variant → None。
#[test]
fn helpers_return_none_on_wrong_variant() {
    // emit_dialogs_cleared 是 Emit，非 Persist → scan_of None。
    assert!(scan_of(emit_dialogs_cleared()).is_none());
    // build_message_query 产 Persist，非 Emit → emit_json None。
    let p = serde_json::to_vec(&serde_json::json!({"channel_id":"ch00000000000000000000000a"}))
        .unwrap();
    let persist = build_message_query(&p, corr()).unwrap().1;
    assert!(emit_json(persist).is_none());
    // Persist 但首 op 非 Scan（空 ops）→ scan_of None（覆盖内层 if let 假分支）。
    let empty_persist = Effect::Persist {
        corr: corr(),
        ops: vec![],
    };
    assert!(scan_of(empty_persist).is_none());
}

#[test]
fn is_query_recognizes_projection_commands() {
    assert!(is_query("im_query_messages_by_channel"));
    assert!(is_query("im_delete_all_dialogs"));
    assert!(is_query("im_query_dialog_list"));
    assert!(!is_query("im_send_message"));
    assert!(!is_query("im_create_channel"));
}

#[test]
fn build_dialog_list_query_scans_channel_table_desc_by_last_post_at() {
    // 带 limit。
    let payload = serde_json::to_vec(&serde_json::json!({ "limit": 100 })).unwrap();
    let effect = build_dialog_list_query(&payload, corr()).expect("build ok");
    let spec = scan_of(effect).expect("Persist+Scan");
    assert_eq!(spec.table, "channel");
    assert_eq!(spec.limit, Some(100));
    // 全量会话：无 channel_id 过滤；按 last_post_at 降序（最近活跃排前）。
    assert!(spec.filter.is_none());
    assert_eq!(spec.order_desc_col, Some("last_post_at"));
}

#[test]
fn build_dialog_list_query_defaults_and_clamps_limit() {
    // 缺 limit / 空 payload → 默认 500。
    let e1 = build_dialog_list_query(b"{}", corr()).unwrap();
    assert_eq!(
        scan_of(e1).expect("scan e1").limit,
        Some(QUERY_DIALOGS_DEFAULT)
    );
    // 非法 JSON（零信任）→ 不 panic，走默认 limit。
    let e2 = build_dialog_list_query(b"not json{", corr()).unwrap();
    assert_eq!(
        scan_of(e2).expect("scan e2").limit,
        Some(QUERY_DIALOGS_DEFAULT)
    );
    // 超 2000 → clamp 2000。
    let p3 = serde_json::to_vec(&serde_json::json!({ "limit": 99999 })).unwrap();
    let e3 = build_dialog_list_query(&p3, corr()).unwrap();
    assert_eq!(scan_of(e3).expect("scan e3").limit, Some(QUERY_DIALOGS_MAX));
}

#[test]
fn emit_dialog_list_result_wraps_rows_under_dialoglist_with_full_fields() {
    // 模拟 channel 行扫描结果：完整 DialogItem 字段（含 unread_count 等，前端零计算）。
    let rows = serde_json::to_vec(&serde_json::json!([
        {"id":"ch1","display_name":"群A","unread_count":3,"mention_count":1,"last_post":"hi","last_event_seq":42},
        {"id":"ch2","display_name":"群B","unread_count":0,"mention_count":0,"last_post":"","last_event_seq":7}
    ]))
    .unwrap();
    let v = emit_json(emit_dialog_list_result(&rows)).expect("emit");
    assert_eq!(v["event"], "im:channels:projection");
    // dialogList 携完整可渲染行（含 unread_count，前端不再自算）。
    assert_eq!(v["data"]["dialogList"][0]["id"], "ch1");
    assert_eq!(v["data"]["dialogList"][0]["unread_count"], 3);
    assert_eq!(v["data"]["dialogList"][0]["mention_count"], 1);
    assert_eq!(v["data"]["dialogList"][1]["unread_count"], 0);
}

#[test]
fn emit_dialog_list_result_invalid_reply_yields_empty_dialoglist() {
    let v = emit_json(emit_dialog_list_result(b"garbage{")).expect("emit");
    assert_eq!(v["event"], "im:channels:projection");
    assert_eq!(v["data"]["dialogList"], serde_json::json!([]));
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
    assert_eq!(spec.limit, Some(30));
    // 过滤列 = channel_id（等值绑定）；降序列 = create_at（取最近 N 条，对齐现网 ORDER BY DESC）。
    assert!(matches!(
        &spec.filter,
        Some(("channel_id", SqlValue::Text(s))) if s == "ch00000000000000000000000a"
    ));
    assert_eq!(spec.order_desc_col, Some("create_at"));
}

#[test]
fn build_message_query_defaults_and_clamps_limit() {
    // 缺 limit → 默认 50。
    let p1 = serde_json::to_vec(&serde_json::json!({"channel_id":"ch00000000000000000000000a"}))
        .unwrap();
    let (_, e1) = build_message_query(&p1, corr()).unwrap();
    assert_eq!(
        scan_of(e1).expect("scan e1").limit,
        Some(QUERY_MESSAGES_DEFAULT)
    );
    // limit 超 500 → clamp 500。
    let p2 = serde_json::to_vec(
        &serde_json::json!({"channel_id":"ch00000000000000000000000a","limit":9999}),
    )
    .unwrap();
    let (_, e2) = build_message_query(&p2, corr()).unwrap();
    assert_eq!(
        scan_of(e2).expect("scan e2").limit,
        Some(QUERY_MESSAGES_MAX)
    );
}

#[test]
fn build_message_query_rejects_missing_channel_id() {
    let payload = serde_json::to_vec(&serde_json::json!({"limit": 10})).unwrap();
    assert!(matches!(
        build_message_query(&payload, corr()),
        Err(ImError::Parse(_))
    ));
    // 非法 JSON → Err 不 panic（边界零信任）。
    assert!(matches!(
        build_message_query(b"not json{", corr()),
        Err(ImError::Parse(_))
    ));
    // 非空但非法 Id26（长度不符）→ from_str None → Err（覆盖「非法 channel_id」分支）。
    let bad = serde_json::to_vec(&serde_json::json!({"channel_id": "too_short"})).unwrap();
    assert!(matches!(
        build_message_query(&bad, corr()),
        Err(ImError::Parse(_))
    ));
}

#[test]
fn emit_message_query_result_wraps_rows_under_messages() {
    let cid = ChannelId::from_str("ch00000000000000000000000a").unwrap();
    let rows = serde_json::to_vec(&serde_json::json!([
        {"temporary_id":"t1","message":"hi"},
        {"temporary_id":"t2","message":"yo"}
    ]))
    .unwrap();
    let v = emit_json(emit_message_query_result(&cid, &rows)).expect("emit");
    assert_eq!(v["event"], "im:messages:query_result");
    assert_eq!(v["data"]["channel_id"], "ch00000000000000000000000a");
    assert_eq!(v["data"]["messages"][0]["temporary_id"], "t1");
    assert_eq!(v["data"]["messages"][1]["message"], "yo");
}

#[test]
fn emit_message_query_result_invalid_reply_yields_empty_messages() {
    let cid = ChannelId::from_str("ch00000000000000000000000a").unwrap();
    let v = emit_json(emit_message_query_result(&cid, b"garbage{")).expect("emit");
    assert_eq!(v["data"]["messages"], serde_json::json!([]));
}

#[test]
fn emit_dialogs_cleared_signals_channels_loaded() {
    let v = emit_json(emit_dialogs_cleared()).expect("emit");
    assert_eq!(v["event"], "im:channels:loaded");
    assert_eq!(v["data"]["reason"], "delete_all_dialogs");
}
