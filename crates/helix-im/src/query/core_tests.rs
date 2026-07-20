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

fn rendered_dialog_list_for_test(reply_bytes: &[u8]) -> serde_json::Value {
    let rows = serde_json::from_slice::<serde_json::Value>(reply_bytes)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .map(crate::query::dialog_render::render_ready_dialog_row)
        .collect::<Vec<_>>();
    serde_json::json!({ "data": { "dialogList": rows } })
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
fn build_dialog_list_query_sorts_top_activity_and_creation_before_limit() {
    // 带 limit。
    let payload = serde_json::to_vec(&serde_json::json!({ "limit": 100 })).unwrap();
    let effect = build_dialog_list_query(&payload, corr()).expect("build ok");
    let spec = scan_of(effect).expect("Persist+Scan");
    assert_eq!(spec.table, "channel");
    assert_eq!(spec.limit, Some(100));
    // 全量会话：置顶优先；同组按活跃、创建时间降序。
    assert!(spec.filter.is_none());
    assert_eq!(
        spec.order_by,
        &[
            ScanOrder::desc("is_top"),
            ScanOrder::desc("last_post_at"),
            ScanOrder::desc("created_at"),
        ]
    );
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
fn render_ready_dialog_rows_keep_full_fields_before_frame_serialization() {
    // 模拟 channel 行扫描结果：完整 DialogItem 字段（含 unread_count 等，前端零计算）。
    let rows = serde_json::to_vec(&serde_json::json!([
        {"id":"ch1","display_name":"群A","unread_count":3,"mention_count":1,"last_post":"hi","last_event_seq":42},
        {"id":"ch2","display_name":"群B","unread_count":0,"mention_count":0,"last_post":"","last_event_seq":7}
    ]))
    .unwrap();
    let v = rendered_dialog_list_for_test(&rows);
    // dialogList 携完整可渲染行（含 unread_count，前端不再自算）。
    assert_eq!(v["data"]["dialogList"][0]["id"], "ch1");
    assert_eq!(v["data"]["dialogList"][0]["unread_count"], 3);
    assert_eq!(v["data"]["dialogList"][0]["mention_count"], 1);
    assert_eq!(v["data"]["dialogList"][1]["unread_count"], 0);
}

#[test]
fn emit_dialog_list_frame_result_projects_scoped_render_ready_snapshot() {
    let rows = serde_json::to_vec(&serde_json::json!([
        {
            "id":"ch-group","display_name":"产品设计群","type":"P","member_count":3,
            "unread_count":4,"mention_count":1,"urgent_count":2,"is_top":1,
            "last_post_at":1700000000000_i64,
            "last_post":"{\"id\":\"post-1\",\"message\":\"交互稿已上传\",\"userId\":\"u-1\",\"type\":\"TEXT\",\"createAt\":1700000000000}"
        },
        {"id":"ch-direct","display_name":"张英","type":"D","member_count":2,"unread_count":0,"last_post_at":1}
    ])).unwrap();
    let mut state = crate::dialog_frame::DialogListFrameState::default();
    let value = emit_json(emit_dialog_list_frame_result(
        &rows,
        crate::dialog_frame::DialogListScope::new("444", "team-a"),
        &mut state,
    ))
    .expect("frame emit");

    assert_eq!(value["event"], "im:interaction:frame");
    assert_eq!(value["data"]["schemaVersion"], 1);
    let mutation = &value["data"]["mutations"][0];
    assert_eq!(mutation["surface"], "message.left-dialog-list");
    assert_eq!(mutation["scope"]["userId"], "444");
    assert_eq!(mutation["mutation"]["kind"], "dialogListSnapshot");
    assert_eq!(
        mutation["mutation"]["view"]["sections"]
            .as_array()
            .expect("dialog sections")
            .iter()
            .flat_map(|section| {
                section["items"]
                    .as_array()
                    .expect("section rows")
                    .iter()
                    .map(|row| row["channelId"].clone())
            })
            .collect::<Vec<_>>(),
        vec![
            serde_json::json!("ch-group"),
            serde_json::json!("ch-direct")
        ]
    );
    assert_eq!(
        mutation["mutation"]["view"]["sections"][0]["items"][0]["unreadCount"],
        4
    );
    assert_eq!(
        mutation["mutation"]["view"]["sections"][0]["items"][0]["preview"]["text"],
        "交互稿已上传"
    );
    assert!(mutation["mutation"]["view"]["sections"][0]["items"][0]
        .get("topicRows")
        .is_none());
}

#[test]
fn render_ready_dialog_rows_keep_terminal_fields_s5() {
    // S5（issue #54）：CL 行升 render-ready 终态——前端零 normalize/零 ++。
    let rows = serde_json::to_vec(&serde_json::json!([
        {
            "id":"ch1","display_name":"群A","is_top":1,"create_at":1700000000000_i64,
            "type":"P","team_id":"team-1","root_id":"","root_post_id":"",
            "unread_count":3,"unread_post_id":"post-unread","mention_count":1,
            "mention_list":"[\"u1\"]","urgent_post_list":"[\"p1\"]",
            "member_count":9,"topic_msg_count":2,"last_event_seq":42,
            "has_schedule_post":1,"props":"{\"canEdit\":true}",
            // last_post = 整帧 post JSON（含 message + 加急标）
            "last_post":"{\"message\":\"最后一条\",\"props\":{\"expedite\":\"m1\"}}",
            "notice":"{\"text\":\"公告内容\"}"
        },
        {
            "id":"ch2","display_name":"群B","is_top":0,"create_at":1700000000001_i64,
            "unread_count":0,"mention_count":0,"mention_count_root":0,
            "last_post":"纯文本兜底","notice":"裸文本公告"
        }
    ]))
    .unwrap();
    let v = rendered_dialog_list_for_test(&rows);
    let r0 = &v["data"]["dialogList"][0];
    // ch1：render-ready 终态键齐全。
    assert_eq!(r0["displayName"], "群A");
    assert_eq!(r0["notice"], "公告内容"); // {text} 归一抽取
    assert_eq!(r0["isTop"], true); // is_top:1 → bool
    assert_eq!(r0["createAt"], 1700000000000_i64);
    assert_eq!(r0["unread"], 3); // 终值（前端不 ++）
    assert_eq!(r0["unreadCount"], 3);
    assert_eq!(r0["mentionCount"], 1);
    assert_eq!(r0["urgentCount"], 0);
    assert_eq!(r0["top"], true);
    assert_eq!(r0["mention"], true); // mention_count>0
    assert_eq!(r0["lastMessage"], "最后一条"); // last_post.message 预览
    assert_eq!(r0["urgent"], true); // props.expedite 命中
                                    // SQLite snake_case 行必须在 Helix 边界升成完整 camelCase dialog，UI 不做 casing/JSON 猜测。
    assert_eq!(r0["channelId"], "ch1");
    assert_eq!(r0["group"], "group");
    assert_eq!(r0["channelType"], "P");
    assert_eq!(r0["channelKind"], "group");
    assert_eq!(r0["teamId"], "team-1");
    assert_eq!(r0["unreadPostId"], "post-unread");
    assert_eq!(r0["mentionList"], serde_json::json!(["u1"]));
    assert_eq!(r0["urgentPostList"], serde_json::json!(["p1"]));
    assert_eq!(r0["memberCount"], 9);
    assert_eq!(r0["topicCount"], 2);
    assert_eq!(r0["eventSeq"], 42);
    assert_eq!(r0["hasSchedulePost"], true);
    assert_eq!(r0["props"], serde_json::json!({"canEdit": true}));
    // 原始列保留（向后兼容）。
    assert_eq!(r0["unread_count"], 3);
    let r1 = &v["data"]["dialogList"][1];
    assert_eq!(r1["isTop"], false); // is_top:0
    assert_eq!(r1["unread"], 0);
    assert_eq!(r1["mention"], false);
    assert_eq!(r1["lastMessage"], "纯文本兜底"); // 非 JSON 兜底整串
    assert_eq!(r1["urgent"], false);
    assert_eq!(r1["notice"], "裸文本公告"); // 非 JSON notice 原样透传
}

#[test]
fn render_ready_dialog_rows_do_not_treat_inactive_as_closed_after_restart() {
    // 回归：新建频道落库默认 is_active=0；冷启动 Scan 时不能因此从 dialogList 消失。
    // 解散(delete_at)和自己退群(is_remove)仍保持两个独立事实。
    let rows = serde_json::to_vec(&serde_json::json!([
        {"id":"ch-new","is_active":0,"is_remove":0,"delete_at":0},
        {"id":"ch-left","is_active":0,"is_remove":1,"delete_at":0},
        {"id":"ch-closed","is_active":0,"is_remove":0,"delete_at":1700000000000_i64}
    ]))
    .unwrap();

    let v = rendered_dialog_list_for_test(&rows);
    let dialogs = v["data"]["dialogList"].as_array().expect("dialogList");

    assert_eq!(dialogs[0]["closed"], false);
    assert_eq!(dialogs[0]["isActive"], 0);
    assert_eq!(dialogs[1]["closed"], false);
    assert_eq!(dialogs[1]["isRemove"], 1);
    assert_eq!(dialogs[2]["closed"], true);
}

#[test]
fn render_ready_dialog_rows_empty_last_post_and_notice_yield_null_s5() {
    // 边界：last_post/notice 空 → lastMessage/notice = null（前端行不带该属性）。
    let rows = serde_json::to_vec(&serde_json::json!([
        {"id":"ch3","display_name":"群C","last_post":"","notice":"","unread_count":0}
    ]))
    .unwrap();
    let v = rendered_dialog_list_for_test(&rows);
    let r = &v["data"]["dialogList"][0];
    assert!(r["lastMessage"].is_null());
    assert!(r["notice"].is_null());
    assert_eq!(r["urgent"], false);
    assert_eq!(r["mention"], false);
}

#[test]
fn render_ready_dialog_rows_invalid_reply_yields_empty_list() {
    let v = rendered_dialog_list_for_test(b"garbage{");
    assert_eq!(v["data"]["dialogList"], serde_json::json!([]));
}

#[test]
fn rendered_dialog_rows_preserve_all_channel_types_and_kinds() {
    let rows = serde_json::to_vec(&serde_json::json!([
        {"id":"topic","type":"T"},
        {"id":"company","type":"O"},
        {"id":"group","type":"P"},
        {"id":"direct","type":"D"}
    ]))
    .unwrap();
    let value = rendered_dialog_list_for_test(&rows);
    let dialogs = value["data"]["dialogList"].as_array().expect("dialogs");
    let expected = [
        ("T", "topic"),
        ("O", "company"),
        ("P", "group"),
        ("D", "direct"),
    ];
    for (dialog, (channel_type, channel_kind)) in dialogs.iter().zip(expected) {
        assert_eq!(dialog["channelType"], channel_type);
        assert_eq!(dialog["channelKind"], channel_kind);
        assert_eq!(dialog["group"], channel_kind);
    }
}

#[test]
fn emit_dialogs_cleared_signals_channels_loaded() {
    let v = emit_json(emit_dialogs_cleared()).expect("emit");
    assert_eq!(v["event"], "im:channels:loaded");
    assert_eq!(v["data"]["reason"], "delete_all_dialogs");
}
