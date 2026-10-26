//! `state.rs` 的 Id26 测试与同 crate fixture。

use super::{is_canonical_post_id, ChannelId, Id26, ImState, ServerId};
use crate::timeline_navigation::TimelineNavigationCoverage;

/// 生成确定性的 26 位测试频道 ID。
pub(crate) fn test_channel_id(n: u64) -> ChannelId {
    let value = format!("chfixx{n:020x}");
    match ChannelId::from_str(&value) {
        Some(id) => id,
        None => panic!("test channel id fixture must be valid"),
    }
}

/// 生成确定性的 26 位测试消息 ID。
pub(crate) fn test_server_id(n: u64) -> ServerId {
    let value = format!("srvfix{n:020x}");
    match ServerId::from_str(&value) {
        Some(id) => id,
        None => panic!("test server id fixture must be valid"),
    }
}

/// 验证 26 位 ASCII ID 可无损往返。
#[test]
fn id26_accepts_exact_ascii_length() {
    let value = "5ffd465e263864787b87aa0123";
    let parsed = Id26::from_str(value);
    assert_eq!(parsed.as_ref().map(Id26::as_str), Some(value));
}

/// 验证错误长度与非 ASCII 输入被拒绝。
#[test]
fn id26_rejects_wrong_length_and_non_ascii() {
    assert!(Id26::from_str("").is_none());
    assert!(Id26::from_str("abc").is_none());
    assert!(Id26::from_str(&"a".repeat(27)).is_none());
    assert!(Id26::from_str("你好你好你好你好你好你好你").is_none());
}

/// 验证 ID 字母表只接受小写字母与数字。
#[test]
fn id26_rejects_invalid_ascii_alphabet() {
    for value in [
        "5ffd465e263864787b87aa012!",
        "5ffd465e263864787b87aa012 ",
        "5ffd465e263864787b87aa012_",
        "5ffd465e263864787b87aa012A",
    ] {
        assert!(Id26::from_str(value).is_none(), "must reject {value:?}");
    }
}

/// Java cses 的 24 位 post 与 Go/Mattermost 的 26 位 post 都属于可上行主键。
#[test]
fn canonical_post_id_accepts_java_24_and_go_26() {
    assert!(is_canonical_post_id("6a80e31f9aac148da3e2c219"));
    assert!(is_canonical_post_id("5ffd465e263864787b87aa0123"));
}

/// 临时键、错误长度和非小写字母数字必须继续被已读边界拒绝。
#[test]
fn canonical_post_id_rejects_temporary_or_invalid_values() {
    let invalid = ["helix_tmp_0000000000000001_0000000000000001"];
    for value in invalid {
        assert!(!is_canonical_post_id(value), "must reject {value:?}");
    }
    for value in ["a".repeat(23), "a".repeat(25), "A".repeat(24)] {
        assert!(
            !is_canonical_post_id(value.as_str()),
            "must reject {value:?}"
        );
    }
}

/// 验证测试 ID 工厂稳定且互不碰撞。
#[test]
fn test_id_factories_are_deterministic() {
    assert_eq!(test_channel_id(1), test_channel_id(1));
    assert_ne!(test_channel_id(1), test_channel_id(2));
    assert_eq!(test_server_id(1), test_server_id(1));
}

/// 验证消息突变只失效所属频道的 locate/page coverage，不波及其他频道窗口。
#[test]
fn timeline_navigation_coverage_invalidation_is_channel_scoped() {
    let changed = test_channel_id(21);
    let untouched = test_channel_id(22);
    let coverage = |channel_id| TimelineNavigationCoverage {
        channel_id,
        rows: vec![serde_json::json!({"id": "root", "topic": {}})],
        has_older: false,
        has_newer: false,
        target_index: Some(0),
        older_cursor: None,
        newer_cursor: None,
    };
    let mut state = ImState::new();
    state
        .timeline_navigation_coverage
        .insert("changed:locate:root:60".to_string(), coverage(changed));
    state
        .timeline_navigation_coverage
        .insert("untouched:locate:root:60".to_string(), coverage(untouched));

    state.invalidate_timeline_navigation_coverage(changed);

    assert_eq!(state.timeline_navigation_coverage.len(), 1);
    assert!(state
        .timeline_navigation_coverage
        .values()
        .all(|entry| entry.channel_id == untouched));
}
