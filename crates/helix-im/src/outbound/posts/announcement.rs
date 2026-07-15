//! post 公告写族 outbound 命令（post/ 单数子路由，与 posts_read.rs 公告读族 #14-16 同域）。
//!
//! helix 此前仅有公告**读**族（list/detail/acceptList），写族（save/delete/read）全缺，此处补齐。
//! 三条都走 `post/announcement/*` 子路由（单数 `post/`，对齐 posts_read.rs #14-16）。
//! 各为独立命令（独立 endpoint + body + inventory 注册），默认写族 `is_read=false`。
//!
//! ## endpoint / body 真源
//! - save：后端要**完整 Post 结构**，body = 整个 args 透传（args.clone()），不裁字段。
//! - delete：`{postIds:[], postId:[]}`（两字段同值数组，兼容后端单/复数命名差异）。
//! - read：`{postId, channelId}`（标记某公告已读）。

use serde_json::{json, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

// ── 本文件局部 helper（边界零信任：缺/类型错 → Err，不 panic）──────────────────────

/// 取必填字符串数组（非空、元素全 string）。供 delete 的 postIds 复用。
fn require_str_array(args: &Value, key: &str, cmd: &str) -> Result<Value, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty() && a.iter().all(Value::is_string))
        .cloned()
        .map(Value::Array)
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空 {key}（非空字符串数组）")))
}

/// POST .../post/announcement/save — 发布/保存公告。
/// 后端要完整 Post 结构 → body = 整个 args 透传（不裁字段，build 直回 args.clone()）。
struct AnnouncementSaveCommand;
impl OutboundCommand for AnnouncementSaveCommand {
    fn name(&self) -> &'static str {
        "im_announcement_save"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        Ok(("post/announcement/save", args.clone()))
    }
}
static ANNOUNCEMENT_SAVE: AnnouncementSaveCommand = AnnouncementSaveCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_announcement_save",
        command: &ANNOUNCEMENT_SAVE,
    }
}

/// POST .../post/announcement/delete — 删除公告。
/// body `{postIds:[...], postId:[...]}` 两字段同值数组（兼容后端单/复数命名差异）。
struct AnnouncementDeleteCommand;
impl OutboundCommand for AnnouncementDeleteCommand {
    fn name(&self) -> &'static str {
        "im_announcement_delete"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_ids = require_str_array(args, "post_ids", self.name())?;
        Ok((
            "post/announcement/delete",
            json!({ "postIds": post_ids, "postId": post_ids }),
        ))
    }
}
static ANNOUNCEMENT_DELETE: AnnouncementDeleteCommand = AnnouncementDeleteCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_announcement_delete",
        command: &ANNOUNCEMENT_DELETE,
    }
}

/// POST .../post/announcement/read — 标记公告已读。
/// body `{postId, channelId}`。
struct AnnouncementReadCommand;
impl OutboundCommand for AnnouncementReadCommand {
    fn name(&self) -> &'static str {
        "im_announcement_read"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let post_id = require_str(args, "post_id", self.name())?;
        let channel_id = require_str(args, "channel_id", self.name())?;
        Ok((
            "post/announcement/read",
            json!({ "postId": post_id, "channelId": channel_id }),
        ))
    }
}
static ANNOUNCEMENT_READ: AnnouncementReadCommand = AnnouncementReadCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_announcement_read",
        command: &ANNOUNCEMENT_READ,
    }
}

#[cfg(test)]
mod tests {
    use crate::outbound::handle_outbound;
    use helix_core::effect::Effect;
    use helix_core::Correlation;
    use serde_json::{json, Value};

    /// 解析 handle_outbound 产出的首个 Effect::Http → (url, body json)。
    fn dispatch(name: &str, args: &Value) -> (String, Value) {
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(args).unwrap();
        let effects = handle_outbound(name, &payload, "http://h/api", "http://h", Some("c1"), corr)
            .expect("should dispatch");
        match &effects[0] {
            Effect::Http { req, .. } => {
                let body: Value =
                    serde_json::from_slice(req.body.as_ref().unwrap()).expect("body json");
                (req.url.clone(), body)
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    /// save：整个 args 透传（含任意 Post 字段，camelCase 原样）。
    #[test]
    fn announcement_save_passthrough() {
        let args = json!({
            "id": "p1",
            "channelId": "c1",
            "message": "公告内容",
            "createAt": 123,
        });
        let (url, body) = dispatch("im_announcement_save", &args);
        assert_eq!(url, "http://h/api/post/announcement/save");
        // 整体透传：字段逐字保留。
        assert_eq!(body, args);
    }

    /// delete：postIds + postId 两字段同值数组。
    #[test]
    fn announcement_delete_dual_field() {
        let args = json!({ "post_ids": ["p1", "p2"] });
        let (url, body) = dispatch("im_announcement_delete", &args);
        assert_eq!(url, "http://h/api/post/announcement/delete");
        assert_eq!(body["postIds"], json!(["p1", "p2"]));
        assert_eq!(body["postId"], json!(["p1", "p2"]));
    }

    /// delete：缺/空 post_ids → Err（不 panic）。
    #[test]
    fn announcement_delete_rejects_empty() {
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({ "post_ids": [] })).unwrap();
        assert!(handle_outbound(
            "im_announcement_delete",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr
        )
        .is_err());
    }

    /// read：{postId, channelId}。
    #[test]
    fn announcement_read_body() {
        let args = json!({ "post_id": "p1", "channel_id": "c1" });
        let (url, body) = dispatch("im_announcement_read", &args);
        assert_eq!(url, "http://h/api/post/announcement/read");
        assert_eq!(body, json!({ "postId": "p1", "channelId": "c1" }));
    }

    /// read：缺 channel_id → Err。
    #[test]
    fn announcement_read_rejects_missing() {
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({ "post_id": "p1" })).unwrap();
        assert!(handle_outbound(
            "im_announcement_read",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr
        )
        .is_err());
    }
}
