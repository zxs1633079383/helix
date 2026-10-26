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
use std::collections::BTreeSet;

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

const MAX_URGENT_TARGETS: usize = 100;

/// POST /api/cses/posts/urgentPost — 消息加急 → post_update。
/// 真源 UrgentPostVO：targetIds 非空校验在 Go `Validate()`（"请选择用户后再操作"）→ 本地也校验
/// targetIds 非空数组（边界零信任，避免空发被服务端拒）。message 可选。
struct UrgentPostCommand;
impl OutboundCommand for UrgentPostCommand {
    fn name(&self) -> &'static str {
        "im_urgent_post"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_exact_keys(
            args,
            &["post_id", "target_ids", "message", "req_id", "channel_id"],
            self.name(),
        )?;
        let post_id = require_str(args, "post_id", self.name())?;
        let target_values = args
            .get("target_ids")
            .and_then(Value::as_array)
            .filter(|items| !items.is_empty() && items.len() <= MAX_URGENT_TARGETS)
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "{}: target_ids 必须为 1..={MAX_URGENT_TARGETS} 个非空字符串",
                    self.name()
                ))
            })?;
        let mut unique = BTreeSet::new();
        let mut target_ids = Vec::with_capacity(target_values.len());
        for target in target_values {
            let target = target
                .as_str()
                .filter(|target| !target.is_empty())
                .ok_or_else(|| ImError::Parse(format!("{}: target_ids 含空值", self.name())))?;
            if !unique.insert(target) {
                return Err(ImError::Parse(format!(
                    "{}: target_ids 不允许重复值",
                    self.name()
                )));
            }
            target_ids.push(target);
        }
        require_optional_request_id(args, self.name())?;
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
        require_exact_keys(args, &["post_id", "req_id", "channel_id"], self.name())?;
        let post_id = require_str(args, "post_id", self.name())?;
        require_optional_request_id(args, self.name())?;
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
        require_exact_keys(args, &["post_id", "req_id", "channel_id"], self.name())?;
        let post_id = require_str(args, "post_id", self.name())?;
        require_optional_request_id(args, self.name())?;
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
