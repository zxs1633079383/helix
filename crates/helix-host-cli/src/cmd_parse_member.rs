//! 成员管理类 repl 命令解析（nickname / role / notify）。
//!
//! 从 `cmd_parse.rs` 抽出（结构闸 ≤300 行）。各映射一个 helix-im outbound 命令，
//! 业务契约/endpoint/body 真源在 `helix-im::outbound`（channel_existing / channel_member_admin），
//! 本文件只把用户参数翻成 payload JSON。子模块经 `super::{Parsed, command}` 复用父构造器。

use super::{command, Parsed};

/// `nickname <channelId> <新昵称...>` → im_update_member_nickname。
/// 昵称允许空（清空昵称，Go 侧 trim 空→nil）；userId 缺省 → Go 用 session 兜底。
pub(super) fn parse_nickname(rest: &str) -> Parsed {
    let mut it = rest.splitn(2, char::is_whitespace);
    let Some(channel) = it.next().filter(|s| !s.is_empty()) else {
        return Parsed::Bad("用法 nickname <channelId> <新昵称>".into());
    };
    let nick = it.next().unwrap_or("").trim();
    Parsed::Command(command(
        "im_update_member_nickname",
        serde_json::json!({ "channel_id": channel, "nickname": nick }),
    ))
}

/// `role <channelId> <userId> <role>` → im_channel_member_role。
/// 真源 body `{channelId, userIds:[]string(非空), role}`；repl 单 userId → 包成单元素数组。
pub(super) fn parse_role(rest: &str) -> Parsed {
    let mut it = rest.split_whitespace();
    let (Some(channel), Some(user), Some(role)) = (it.next(), it.next(), it.next()) else {
        return Parsed::Bad("用法 role <channelId> <userId> <role>".into());
    };
    Parsed::Command(command(
        "im_channel_member_role",
        serde_json::json!({ "channel_id": channel, "user_ids": [user], "role": role }),
    ))
}

/// `notify <channelId> <notify>` → im_channel_member_notify。
/// 真源 body `{channelId, notify}`，UserId 后端 session 覆盖。notify 透传（字符串/枚举枚举值
/// NORMAL/STRONG/IGNORE，非法值 Go ent validator 拒，full-map channelmember 枚举真源）。
pub(super) fn parse_notify(rest: &str) -> Parsed {
    let mut it = rest.split_whitespace();
    let (Some(channel), Some(notify)) = (it.next(), it.next()) else {
        return Parsed::Bad("用法 notify <channelId> <notify>（NORMAL/STRONG/IGNORE）".into());
    };
    Parsed::Command(command(
        "im_channel_member_notify",
        serde_json::json!({ "channel_id": channel, "notify": notify }),
    ))
}

#[cfg(test)]
mod tests {
    use super::super::Parsed;
    use helix_core::Tick;

    fn unwrap_cmd(p: Parsed) -> (String, serde_json::Value) {
        match p {
            Parsed::Command(Tick::Command(c)) => {
                let v: serde_json::Value = serde_json::from_slice(&c.payload).unwrap();
                (c.name.into_owned(), v)
            }
            _ => panic!("expected Command"),
        }
    }

    #[test]
    fn role_wraps_single_user_into_array() {
        let (name, v) = unwrap_cmd(super::parse_role("ch_9 678 MEMBER"));
        assert_eq!(name, "im_channel_member_role");
        assert_eq!(v["channel_id"], "ch_9");
        assert_eq!(v["user_ids"], serde_json::json!(["678"]));
        assert_eq!(v["role"], "MEMBER");
    }

    #[test]
    fn role_missing_arg_is_bad() {
        assert!(matches!(super::parse_role("ch_9 678"), Parsed::Bad(_)));
    }

    #[test]
    fn notify_passes_enum_value() {
        let (name, v) = unwrap_cmd(super::parse_notify("ch_9 IGNORE"));
        assert_eq!(name, "im_channel_member_notify");
        assert_eq!(v["channel_id"], "ch_9");
        assert_eq!(v["notify"], "IGNORE");
    }
}
