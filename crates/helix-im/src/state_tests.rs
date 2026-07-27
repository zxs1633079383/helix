//! `state.rs` 的 Id26 / fixture 单测（经 `#[cfg(test)] #[path] mod id26_tests;` 挂回）。
//!
//! 从 `state.rs` 抽出守 ≤300 软目标 / ≤378 baseline（结构闸 §1）——drift-review 收敛：
//! WHITELIST baseline 不得靠「上调数字」抹掉 FAIL（#1/#7 后门第二形态），src 测试外提回正。
//! `super::` 指向 `state` 模块，故 `test_channel_id`/`Id26`/`ChannelId` 路径与内联时一致。

use super::{ChannelId, Id26, ImState, ServerId};
use crate::interaction_frame::{ActionBinding, SurfaceId, SurfaceScope};
use crate::projector::action::{ActionBindingGrant, ActionDispatch};

/// 测试 fixture：产 26 位 `[a-z0-9]` 合法 ChannelId（`chfixx` 前缀 + 20 位 hex）。
/// 从 `state.rs` 外提至此守 ≤351 baseline（结构闸 §1，drift-review 后门第二形态收敛）；
/// state.rs 经 `#[cfg(test)] pub(crate) use id26_tests::{...}` 把路径 `crate::state::test_channel_id`
/// 重导出回原位 → 同 crate 其余测试（channel_tests/sync_scheduler/ws::registry…）调用点零改动。
pub(crate) fn test_channel_id(n: u64) -> ChannelId {
    let s = format!("chfixx{n:020x}");
    ChannelId::from_str(&s).expect("test channel id fixture must be valid Id26")
}

pub(crate) fn test_server_id(n: u64) -> ServerId {
    let s = format!("srvfix{n:020x}");
    ServerId::from_str(&s).expect("test server id fixture must be valid Id26")
}

#[test]
fn from_str_accepts_exactly_26_ascii_and_roundtrips() {
    let s = "5ffd465e263864787b87aa01"; // 24 字符——非法
    assert!(Id26::from_str(s).is_none(), "24 字符应拒绝");
    let s26 = "5ffd465e263864787b87aa0123"; // 26 字符
    assert_eq!(s26.len(), 26);
    let id = Id26::from_str(s26).expect("26 ASCII 应接受");
    assert_eq!(id.as_str(), s26, "as_str 必须原样回放");
}

#[test]
fn from_str_rejects_wrong_len_and_non_ascii() {
    assert!(Id26::from_str("").is_none());
    assert!(Id26::from_str("abc").is_none()); // 太短
    assert!(Id26::from_str(&"a".repeat(27)).is_none()); // 太长
    assert!(Id26::from_str("你好你好你好你好你好你好你").is_none()); // 非 ASCII（字节数也≠26）
}

#[test]
fn from_str_rejects_invalid_ascii_alphabet() {
    let invalid = [
        "5ffd465e263864787b87aa012!",
        "5ffd465e263864787b87aa012 ",
        "5ffd465e263864787b87aa012_",
        "5ffd465e263864787b87aa012A",
    ];
    for s in invalid {
        assert_eq!(s.len(), 26);
        assert!(Id26::from_str(s).is_none(), "must reject {s:?}");
    }
}

#[test]
fn test_channel_id_is_26_alnum_and_distinct() {
    let a = test_channel_id(1);
    let b = test_channel_id(2);
    assert_eq!(a.as_str().len(), 26);
    assert_ne!(a, b);
    assert_eq!(a, test_channel_id(1), "确定性");
    // 经 from_str 再解析应等价（26 位 [a-z0-9] fixture）
    assert_eq!(super::ChannelId::from_str(a.as_str()), Some(a));
}

#[test]
fn hash_u64_deterministic() {
    let a = test_channel_id(7);
    assert_eq!(a.hash_u64(), a.hash_u64());
    assert_ne!(test_channel_id(7).hash_u64(), test_channel_id(8).hash_u64());
}

#[test]
/// 证明重连 hello 的批次 reset 只失效 query correlation，不撤销 renderer 当前 live grant。
fn reconnect_batch_reset_preserves_live_action_grant() {
    let mut state = ImState::new();
    let scope = SurfaceScope::Timeline {
        channel_id: "channel-1".to_string(),
        window_token: "latest".to_string(),
    };
    state
        .action_registry
        .grant(
            ActionBindingGrant {
                actor_id: "viewer-1".to_string(),
                surface: SurfaceId::MessageTimeline,
                scope,
                surface_epoch: 3,
                revision: 7,
                expires_at_ms: 60_000,
                binding: ActionBinding {
                    action_ref: "act.v1.transport-live".to_string(),
                    kind: "retry".to_string(),
                    input_shape: "object".to_string(),
                    surface_epoch: 3,
                    enabled: true,
                    disabled_reason: None,
                },
                dispatch: ActionDispatch::RetryMessage {
                    channel_id: "channel-1".to_string(),
                    message_id: "message-1".to_string(),
                    window_token: "latest".to_string(),
                },
            },
            1,
        )
        .expect("live retry grant must register");

    state.reset_increment_batch();

    assert_eq!(state.action_registry.len(), 1);
}
