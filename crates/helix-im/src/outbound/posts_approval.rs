//! posts 审批 outbound 命令（HTTP-coverage 补齐，与 posts_existing/posts_announcement 零交集）。
//!
//! #32 POST /api/cses/post/approval/approval — 消息审批（approvalPost → ApprovePost）。
//! 真源 full-map/partials/1 §32：内联 `{postId}`（post_approval.go L16-18）。
//! 审批通过常伴 `post_update` WS 回声（视实现）→ 写族（不读回灌），HTTP 响应无 data。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use super::registry::{require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};

/// #32 post/approval/approval — 审批一条待审消息。body = `{postId}`（camelCase 单字段）。
struct ApprovalPostCommand;
impl OutboundCommand for ApprovalPostCommand {
    fn name(&self) -> &'static str {
        "im_approval_post"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        Ok(("post/approval/approval", json!({ "postId": post_id })))
    }
}
#[distributed_slice(OUTBOUND_COMMANDS)]
static APPROVAL_POST_REG: OutboundRegistration = OutboundRegistration {
    name: "im_approval_post",
    command: &ApprovalPostCommand,
};

#[cfg(test)]
mod tests {
    use helix_core::effect::Effect;
    use helix_core::Correlation;
    use serde_json::json;

    use super::super::registry::{handle_outbound, is_outbound};

    #[test]
    fn approval_post_endpoint_and_body() {
        assert!(is_outbound("im_approval_post"));
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({ "post_id": "p1" })).unwrap();
        let effects = handle_outbound(
            "im_approval_post",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr,
        )
        .expect("approval should dispatch");
        match &effects[0] {
            Effect::Http { req, .. } => {
                assert!(
                    req.url.ends_with("/post/approval/approval"),
                    "url={}",
                    req.url
                );
                let body: serde_json::Value =
                    serde_json::from_slice(req.body.as_ref().unwrap()).unwrap();
                assert_eq!(body["postId"], "p1");
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn approval_post_missing_post_id_errors() {
        let corr = Correlation::from_raw(2);
        let payload = serde_json::to_vec(&json!({})).unwrap();
        assert!(handle_outbound(
            "im_approval_post",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr
        )
        .is_err());
    }
}
