//! posts 快捷回复 outbound 命令（P2 新增）：quickReply（emoji 表情回复）。
//!
//! endpoint 真源 `posts.go:51`；body 真源 `posts.go:181` 匿名 struct
//! `{postId, userId, emoji}`。userId 可选（Go 侧用传入 userId；空时由 App 层语义决定）。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

/// POST /api/cses/posts/quickReply — emoji 快捷回复。真源 `{postId, userId, emoji}`。
struct QuickReplyCommand;
impl OutboundCommand for QuickReplyCommand {
    fn name(&self) -> &'static str {
        "im_quick_reply"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        let emoji = require_str(args, "emoji", self.name())?;
        let mut b = json!({ "postId": post_id, "emoji": emoji });
        // userId 可选（Go param.UserId 直传；缺省则交由服务端 session 语义）。
        if let Some(uid) = args.get("user_id").and_then(Value::as_str) {
            b["userId"] = json!(uid);
        }
        Ok(("posts/quickReply", b))
    }
}
static QUICK_REPLY: QuickReplyCommand = QuickReplyCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_quick_reply",
        command: &QUICK_REPLY,
    }
}
