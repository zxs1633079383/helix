//! posts 加急族 outbound 命令（P2 新增）：urgentPost / urgentConfirm / urgentCancel。
//!
//! 三条都走 WS 推 **`post_update`** 复用（现网 `cses_post.go:149`
//! `NewWebSocketEventForUsers(model.WebsocketEventPostUpdate, ...)`），故归一个语义族一文件，
//! 但各为独立命令（独立 endpoint + body + 注册）。
//!
//! endpoint 真源 `posts.go:24/28/29`；body 真源：
//! - urgentPost = `entity.UrgentPostVO`（post.go:504）`{postId, targetIds[], message?}`
//! - urgentConfirm/urgentCancel = `{postId}`（Go handler `urgentConfirmParam`）
//!
//! Go 从 `postId` 对应的权威 post 行 O(1) 解析频道，并对显式冲突 fail-closed；Helix 因而
//! 不接管频道推导，也不把旧调用方携带的 `channel_id` 继续发到 wire。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

/// POST /api/cses/posts/urgentPost — 消息加急 → post_update。
/// 真源 UrgentPostVO：targetIds 非空校验在 Go `Validate()`（"请选择用户后再操作"）→ 本地也校验
/// targetIds 非空数组（边界零信任，避免空发被服务端拒）。message 可选。
struct UrgentPostCommand;
impl OutboundCommand for UrgentPostCommand {
    fn name(&self) -> &'static str {
        "im_urgent_post"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        let target_ids = args
            .get("target_ids")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty() && a.iter().all(|v| v.is_string()))
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{}: 缺/空 target_ids（非空字符串数组）",
                    self.name()
                ))
            })?;
        let mut b = json!({
            "postId": post_id,
            "targetIds": target_ids,
        });
        // message 可选（UrgentPostVO.Message *string，omitempty 语义）。
        if let Some(msg) = args.get("message").and_then(Value::as_str) {
            b["message"] = json!(msg);
        }
        Ok(("posts/urgentPost", b))
    }
}
static URGENT_POST: UrgentPostCommand = UrgentPostCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_urgent_post",
        command: &URGENT_POST,
    }
}

/// POST /api/cses/posts/urgentConfirm — 确认加急（被加急者点确认）→ post_update。
/// Go 按 `postId` 权威解析频道；调用方无需重复提供频道。
struct UrgentConfirmCommand;
impl OutboundCommand for UrgentConfirmCommand {
    fn name(&self) -> &'static str {
        "im_urgent_confirm"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        Ok(("posts/urgentConfirm", json!({ "postId": post_id })))
    }
}
static URGENT_CONFIRM: UrgentConfirmCommand = UrgentConfirmCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_urgent_confirm",
        command: &URGENT_CONFIRM,
    }
}

/// POST /api/cses/posts/urgentCancel — 取消加急（发起者撤）→ post_update。
/// Go 按 `postId` 权威解析频道；调用方无需重复提供频道。
struct UrgentCancelCommand;
impl OutboundCommand for UrgentCancelCommand {
    fn name(&self) -> &'static str {
        "im_urgent_cancel"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        Ok(("posts/urgentCancel", json!({ "postId": post_id })))
    }
}
static URGENT_CANCEL: UrgentCancelCommand = UrgentCancelCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_urgent_cancel",
        command: &URGENT_CANCEL,
    }
}
