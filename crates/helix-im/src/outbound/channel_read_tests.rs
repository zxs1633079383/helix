//! channel 读族补全命令测试：endpoint + camelCase body + is_read=true + 必填零信任。

use helix_core::effect::Effect;
use helix_core::Correlation;
use serde_json::json;

use super::super::registry::{handle_outbound, is_outbound, is_read};

fn dispatch(name: &str, args: serde_json::Value) -> (String, serde_json::Value) {
    let corr = Correlation::from_raw(1);
    let payload = serde_json::to_vec(&args).unwrap();
    let effects = handle_outbound(name, &payload, "http://h/api", "http://h", Some("c1"), corr)
        .unwrap_or_else(|e| panic!("{name} should dispatch: {e:?}"));
    assert_eq!(effects.len(), 1);
    match &effects[0] {
        Effect::Http { req, .. } => {
            let body: serde_json::Value =
                serde_json::from_slice(req.body.as_ref().expect("body")).unwrap();
            (req.url.clone(), body)
        }
        other => panic!("expected Http, got {other:?}"),
    }
}

#[test]
fn all_read_commands_marked_is_read() {
    for n in [
        "im_channel_load_notice",
        "im_channel_load_post_pinned",
        "im_channel_load_admin",
        "im_channel_load_increment_by_channel_id",
        "im_channel_query",
        "im_channel_online_status",
        "im_channels_members_by_ids",
        "im_channel_member_snapshot",
    ] {
        assert!(is_outbound(n), "{n} 应被认领");
        assert!(
            is_read(n),
            "{n} 应标 is_read=true（无 WS 回声，HTTP 体即数据）"
        );
    }
}

#[test]
fn load_notice_admin_pinned_increment_channel_id_body() {
    for (name, ep) in [
        ("im_channel_load_notice", "/channel/load/notice"),
        ("im_channel_load_admin", "/channel/load/admin"),
        ("im_channel_load_post_pinned", "/channel/load/postPinned"),
        (
            "im_channel_load_increment_by_channel_id",
            "/channel/load/incrementByChannelId",
        ),
    ] {
        let (url, body) = dispatch(name, json!({ "channel_id": "c1" }));
        assert!(url.ends_with(ep), "{name} url={url}");
        assert_eq!(body["channelId"], "c1", "{name} channelId");
    }
}

#[test]
fn online_status_and_members_by_ids_channel_ids_array() {
    let (url, body) = dispatch(
        "im_channel_online_status",
        json!({ "channel_ids": ["c1", "c2"] }),
    );
    assert!(url.ends_with("/channel/onlineStatus"), "url={url}");
    assert_eq!(body["channelIds"], json!(["c1", "c2"]));

    let (url2, body2) = dispatch(
        "im_channels_members_by_ids",
        json!({ "channel_ids": ["c1"] }),
    );
    assert!(url2.ends_with("/channels/member/byIds"), "url={url2}");
    assert_eq!(body2["channelIds"], json!(["c1"]));
}

#[test]
fn member_snapshot_int_time_range() {
    let (url, body) = dispatch(
        "im_channel_member_snapshot",
        json!({ "channel_id": "c1", "start_time": 1000_i64, "end_time": 2000_i64 }),
    );
    assert!(url.ends_with("/channel/member/snapshot"), "url={url}");
    assert_eq!(body["channelId"], "c1");
    assert_eq!(body["startTime"], 1000);
    assert_eq!(body["endTime"], 2000);
}

#[test]
fn channel_query_merges_condition_and_paging() {
    let (url, body) = dispatch(
        "im_channel_query",
        json!({
            "condition": { "name": "测试群", "teamId": "t1" },
            "page_number": 1,
            "page_size": 20,
            "offset": 0
        }),
    );
    assert!(url.ends_with("/channel/query"), "url={url}");
    // 条件平铺 + 分页字段同层（匿名 struct embed Channel + PageOpts）。
    assert_eq!(body["name"], "测试群");
    assert_eq!(body["teamId"], "t1");
    assert_eq!(body["pageNumber"], 1);
    assert_eq!(body["pageSize"], 20);
    assert_eq!(body["offset"], 0);
}

#[test]
fn channel_query_defaults_paging_zero_when_absent() {
    let (_, body) = dispatch("im_channel_query", json!({ "condition": { "name": "x" } }));
    assert_eq!(body["pageNumber"], 0);
    assert_eq!(body["pageSize"], 0);
    assert_eq!(body["offset"], 0);
}

#[test]
fn read_missing_required_errors() {
    let corr = Correlation::from_raw(2);
    let p = serde_json::to_vec(&json!({})).unwrap();
    assert!(handle_outbound(
        "im_channel_load_notice",
        &p,
        "http://h/api",
        "http://h",
        None,
        corr
    )
    .is_err());
    // snapshot 缺 time
    let p2 = serde_json::to_vec(&json!({ "channel_id": "c1" })).unwrap();
    assert!(handle_outbound(
        "im_channel_member_snapshot",
        &p2,
        "http://h/api",
        "http://h",
        None,
        corr
    )
    .is_err());
    // online_status 空数组
    let p3 = serde_json::to_vec(&json!({ "channel_ids": [] })).unwrap();
    assert!(handle_outbound(
        "im_channel_online_status",
        &p3,
        "http://h/api",
        "http://h",
        None,
        corr
    )
    .is_err());
}
