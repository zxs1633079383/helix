//! posts 快捷回复 outbound 命令（P2 新增）：quickReply（emoji 表情回复）。
//!
//! endpoint 真源 `posts.go:51`；body 真源为 `{postId, emoji}`。
//! actor 必须由 Go 鉴权 session 注入，不能由薄客户端重复下发或伪造。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

/// POST /api/cses/posts/quickReply — emoji 快捷回复。真源 `{postId, emoji}`；actor 由会话注入。
struct QuickReplyCommand;
impl OutboundCommand for QuickReplyCommand {
    fn name(&self) -> &'static str {
        "im_quick_reply"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_exact_keys(args, &["post_id", "emoji", "req_id"], self.name())?;
        let post_id = require_str(args, "post_id", self.name())?;
        let emoji = require_str(args, "emoji", self.name())?;
        require_optional_request_id(args, self.name())?;
        Ok((
            "posts/quickReply",
            json!({ "postId": post_id, "emoji": emoji }),
        ))
    }
}
static QUICK_REPLY: QuickReplyCommand = QuickReplyCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_quick_reply",
        command: &QUICK_REPLY,
    }
}

fn require_exact_keys(args: &Value, allowed: &[&str], cmd: &str) -> Result<(), ImError> {
    let object = args
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{cmd}: payload 必须是 object")))?;
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ImError::Parse(format!(
            "{cmd}: 未知或非 canonical 字段 '{unknown}'"
        )));
    }
    Ok(())
}

fn require_optional_request_id(args: &Value, cmd: &str) -> Result<(), ImError> {
    if let Some(value) = args.get("req_id") {
        if value.as_str().filter(|value| !value.is_empty()).is_none() {
            return Err(ImError::Parse(format!("{cmd}: req_id 必须是非空字符串")));
        }
    }
    Ok(())
}
