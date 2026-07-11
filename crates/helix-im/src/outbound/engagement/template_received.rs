//! posts 模板已收到 outbound 命令（P2 新增）：templateReceived。
//!
//! ⚠️ endpoint 前缀是 `/post`（单数）非 `/posts`——真源 `posts.go:53`
//! `api.BaseRoutes.Post.Handle("/templateReceived", ...)`（Post = `/post` subrouter，api.go:171）。
//! body 真源 `posts.go:721` 匿名 struct `{postId}`；Go 校验 `postId == ""` → 必填。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{
    require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS,
};

/// POST /api/cses/post/templateReceived — 模板消息已收到回执。真源 `{postId}`。
/// 路径前缀 `post`（单数），勿写成 `posts`（命名陷阱：与同文件其余 posts/* 不同前缀）。
///
/// 公共命令先经过 `mobile_contract::normalize_command_payload`，顶层 camelCase 会归一为
/// snake_case，因此 builder 读取 `post_id`，wire body 再映射回 Go 所需的 `postId`。
struct TemplateReceivedCommand;
impl OutboundCommand for TemplateReceivedCommand {
    fn name(&self) -> &'static str {
        "im_template_received"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        Ok(("post/templateReceived", json!({ "postId": post_id })))
    }
}
static TEMPLATE_RECEIVED: TemplateReceivedCommand = TemplateReceivedCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static TEMPLATE_RECEIVED_REG: OutboundRegistration = OutboundRegistration {
    name: "im_template_received",
    command: &TEMPLATE_RECEIVED,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalized_public_payload_builds_go_wire_body() {
        let normalized = crate::mobile_contract::normalize_command_payload(
            "im_template_received",
            br#"{"postId":"post-1"}"#,
        )
        .expect("normalize public payload");
        let args: Value = serde_json::from_slice(&normalized).expect("decode normalized payload");
        let (path, body) = TEMPLATE_RECEIVED
            .build(&args)
            .expect("build outbound request");
        assert_eq!(path, "post/templateReceived");
        assert_eq!(body, json!({ "postId": "post-1" }));
    }
}
