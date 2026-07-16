//! `render_ready_members` 单测（TDD·issue #56 S7）。移出 src 守 ≤300 行硬顶。

use super::*;
use crate::acl::to_effect_s1::MemberRow;

fn emit_json(e: Effect) -> serde_json::Value {
    match e {
        Effect::Emit { event } => serde_json::from_slice(event.0.as_ref()).unwrap(),
        _ => panic!("expected Emit"),
    }
}

#[test]
fn role_admin_rule_covers_raw_and_normalized() {
    // raw wire（byIds body）
    assert!(role_is_admin("CREATOR"));
    assert!(role_is_admin("MANGER"));
    assert!(role_is_admin("MANAGER"));
    assert!(role_is_admin("ADMIN"));
    // normalized（collect_members）
    assert!(role_is_admin("OWNER"));
    // 非 admin（对齐原壳·BOSS/MEMBER 不判 admin）
    assert!(!role_is_admin("BOSS"));
    assert!(!role_is_admin("MEMBER"));
    assert!(!role_is_admin(""));
}

#[test]
fn members_from_rows_maps_admin_and_fields() {
    let rows = vec![
        MemberRow {
            user_id: "u1".into(),
            team_id: "t".into(),
            role: "OWNER".into(),
            nick_name: "小王".into(),
        },
        MemberRow {
            user_id: "u2".into(),
            team_id: "t".into(),
            role: "MEMBER".into(),
            nick_name: "".into(),
        },
    ];
    let out = members_from_rows(rows);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].member_id, "u1");
    assert_eq!(out[0].nickname, "小王");
    assert!(out[0].admin);
    assert_eq!(out[1].member_id, "u2");
    assert!(!out[1].admin);
}

#[test]
fn byids_body_parses_per_channel_with_admin_and_dedup() {
    let body = serde_json::json!({
        "status": "SUCCESS",
        "data": {
            "ch1": [
                {"id": "u1", "role": "CREATOR", "nickName": "群主"},
                {"id": "u2", "role": "MEMBER", "nickName": ""},
                {"id": "u1", "role": "MEMBER"},          // 重复 id → 去重保首
                {"role": "MEMBER"},                       // 缺 id → 跳过
            ],
            "ch2": []
        }
    });
    let mut out = members_from_byids_body(&body);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].0, "ch1");
    assert_eq!(out[0].1.len(), 2); // u1（保首·admin）+ u2
    assert!(out[0].1[0].admin);
    assert_eq!(out[0].1[0].nickname, "群主");
    assert!(!out[0].1[1].admin);
    assert_eq!(out[1].0, "ch2");
    assert!(out[1].1.is_empty());
}

#[test]
fn byids_body_missing_data_yields_empty() {
    assert!(members_from_byids_body(&serde_json::json!({"status": "OK"})).is_empty());
    assert!(members_from_byids_body(&serde_json::Value::Null).is_empty());
}

#[test]
fn emit_channel_members_shape_camelcase() {
    let e = emit_channel_members(
        "ch1",
        vec![RenderMember {
            member_id: "u1".into(),
            nickname: "n".into(),
            admin: true,
        }],
        vec!["u9".into()],
        MemberProjectionMode::Incremental,
    );
    let v = emit_json(e);
    assert_eq!(v["event"], "im:channel:members");
    assert_eq!(v["data"]["channelId"], "ch1");
    assert_eq!(v["data"]["memberCount"], 1);
    assert_eq!(v["data"]["projectionMode"], "incremental");
    assert_eq!(v["data"]["members"][0]["memberId"], "u1");
    assert_eq!(v["data"]["members"][0]["nickname"], "n");
    assert_eq!(v["data"]["members"][0]["admin"], true);
    assert_eq!(v["data"]["leaves"][0], "u9");
}

#[test]
fn member_projection_mode_is_explicit_for_snapshot_and_incremental() {
    for (mode, expected) in [
        (MemberProjectionMode::Snapshot, "snapshot"),
        (MemberProjectionMode::Incremental, "incremental"),
    ] {
        let value = emit_json(emit_channel_members("ch1", Vec::new(), Vec::new(), mode));
        assert_eq!(value["data"]["projectionMode"], expected);
    }
}
