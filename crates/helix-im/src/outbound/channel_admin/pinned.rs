//! 消息置顶族 outbound 命令：messageTop（置顶）/ messageUntop（取消置顶）。
//!
//! ⚠️ **勿与对话置顶混淆**：本族是 **post 置顶**（一条消息钉在频道顶部，`channel/*/postPinned`），
//! 与 `channel_change.rs` 的 `im_channel_change_top`（**per-member 对话置顶** `channel/change/top`）
//! 是两套机制，键名/路由都不同——前者作用于单条 post，后者作用于整个频道在我侧的排序。
//!
//! 两条命令各独立 endpoint + body + inventory 注册（C4 收口，与其他 outbound 文件零交集）。
//! 写族默认 is_read=false（写经 WS 回声对账，不回灌 HTTP 响应）。
//!
//! ## endpoint / body 真源
//! 子路由前缀 `channel`（BaseRoutes.Channel = `/api/cses/channel`）。
//! - set   → POST `channel/add/postPinned`    body = `{channelId, postId}`
//! - unset → POST `channel/remove/postPinned` body = `{channelId, postId}`
//!
//! **casing 陷阱**：body 键 camelCase 原样透传（`channelId`/`postId`，非 snake_case）——
//! endpoint 是 passthrough 域，键形状须对齐 cses params。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

/// POST /api/cses/channel/add/postPinned — 置顶一条消息。
/// body = `{channelId, postId}`（两者必填非空，边界零信任 HX-C 不变量 4）。
struct SetMessageTopCommand;
impl OutboundCommand for SetMessageTopCommand {
    fn name(&self) -> &'static str {
        "im_set_message_top"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let post_id = require_str(args, "post_id", self.name())?;
        Ok((
            "channel/add/postPinned",
            json!({ "channelId": channel_id, "postId": post_id }),
        ))
    }
}
static SET_MESSAGE_TOP: SetMessageTopCommand = SetMessageTopCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_set_message_top",
        command: &SET_MESSAGE_TOP,
    }
}

/// POST /api/cses/channel/remove/postPinned — 取消置顶一条消息。
/// body = `{channelId, postId}`（两者必填非空，边界零信任 HX-C 不变量 4）。
struct SetMessageUntopCommand;
impl OutboundCommand for SetMessageUntopCommand {
    fn name(&self) -> &'static str {
        "im_set_message_untop"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let channel_id = require_str(args, "channel_id", self.name())?;
        let post_id = require_str(args, "post_id", self.name())?;
        Ok((
            "channel/remove/postPinned",
            json!({ "channelId": channel_id, "postId": post_id }),
        ))
    }
}
static SET_MESSAGE_UNTOP: SetMessageUntopCommand = SetMessageUntopCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_set_message_untop",
        command: &SET_MESSAGE_UNTOP,
    }
}

#[cfg(test)]
mod tests {
    use helix_core::effect::Effect;
    use helix_core::Correlation;
    use serde_json::json;

    use crate::outbound::registry::{handle_outbound, is_outbound};

    /// im_set_message_top 正常路径：endpoint = channel/add/postPinned，
    /// body camelCase 透传（channelId/postId）。
    #[test]
    fn set_message_top_dispatch_and_body() {
        assert!(is_outbound("im_set_message_top"));
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({ "channel_id": "c1", "post_id": "p1" })).unwrap();
        let effects = handle_outbound(
            "im_set_message_top",
            &payload,
            "http://h/api",
            "http://h",
            Some("conn1"),
            corr,
        )
        .expect("set_message_top should dispatch");
        match &effects[0] {
            Effect::Http { req, .. } => {
                assert!(
                    req.url.ends_with("/channel/add/postPinned"),
                    "url={}",
                    req.url
                );
                let body: serde_json::Value =
                    serde_json::from_slice(req.body.as_ref().expect("body")).unwrap();
                assert_eq!(body["channelId"], "c1");
                assert_eq!(body["postId"], "p1");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    /// im_set_message_untop 正常路径：endpoint = channel/remove/postPinned，
    /// body camelCase 透传（channelId/postId）。
    #[test]
    fn set_message_untop_dispatch_and_body() {
        assert!(is_outbound("im_set_message_untop"));
        let corr = Correlation::from_raw(2);
        let payload = serde_json::to_vec(&json!({ "channel_id": "c2", "post_id": "p2" })).unwrap();
        let effects = handle_outbound(
            "im_set_message_untop",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr,
        )
        .expect("set_message_untop should dispatch");
        match &effects[0] {
            Effect::Http { req, .. } => {
                assert!(
                    req.url.ends_with("/channel/remove/postPinned"),
                    "url={}",
                    req.url
                );
                let body: serde_json::Value =
                    serde_json::from_slice(req.body.as_ref().expect("body")).unwrap();
                assert_eq!(body["channelId"], "c2");
                assert_eq!(body["postId"], "p2");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    /// 缺 post_id → Err（边界零信任，不 panic）。
    #[test]
    fn set_message_top_missing_post_id_errors() {
        let corr = Correlation::from_raw(3);
        let payload = serde_json::to_vec(&json!({ "channel_id": "c1" })).unwrap();
        assert!(handle_outbound(
            "im_set_message_top",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr
        )
        .is_err());
    }
}
