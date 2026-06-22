//! posts props 更新 outbound 命令（P2 新增）：updatePostProps（投票等 props 改写）。
//!
//! endpoint 真源 `posts.go:52`；body 真源 `posts.go:750` 匿名 struct
//! `{postId, props map[string]interface{}}`。Go 校验 `postId == "" || props == nil` → 必填。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use super::registry::{require_str, OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};

/// POST /api/cses/posts/updatePostProps — 更新 post props（投票态等）。
/// 真源 `{postId, props}`；props 必须是非 null object（Go `props == nil` 拒）。
struct UpdatePostPropsCommand;
impl OutboundCommand for UpdatePostPropsCommand {
    fn name(&self) -> &'static str {
        "im_update_post_props"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        // props 必须是 object（Go `map[string]interface{}` 且非 nil）；边界零信任校验。
        let props = args
            .get("props")
            .filter(|v| v.is_object())
            .ok_or_else(|| ImError::Parse(format!("{}: 缺/坏 props（须为对象）", self.name())))?;
        Ok((
            "posts/updatePostProps",
            json!({ "postId": post_id, "props": props }),
        ))
    }
}
static UPDATE_POST_PROPS: UpdatePostPropsCommand = UpdatePostPropsCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static UPDATE_POST_PROPS_REG: OutboundRegistration = OutboundRegistration {
    name: "im_update_post_props",
    command: &UPDATE_POST_PROPS,
};
