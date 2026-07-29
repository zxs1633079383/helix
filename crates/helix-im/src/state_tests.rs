//! `state.rs` 的 Id26 测试与同 crate fixture。

use super::{ChannelId, Id26, ServerId};

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

/// 验证测试 ID 工厂稳定且互不碰撞。
#[test]
fn test_id_factories_are_deterministic() {
    assert_eq!(test_channel_id(1), test_channel_id(1));
    assert_ne!(test_channel_id(1), test_channel_id(2));
    assert_eq!(test_server_id(1), test_server_id(1));
}
