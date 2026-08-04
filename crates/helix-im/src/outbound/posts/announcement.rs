//! post 公告写族 outbound 命令（post/ 单数子路由，与 posts_read.rs 公告读族 #14-16 同域）。
//!
//! helix 此前仅有公告**读**族（list/detail/acceptList），写族（save/delete/read）全缺，此处补齐。
//! 三条都走 `post/announcement/*` 子路由（单数 `post/`，对齐 posts_read.rs #14-16）。
//! 各为独立命令（独立 endpoint + body + inventory 注册），默认写族 `is_read=false`。
//!
//! ## endpoint / body 真源
//! - save：MessageV3 Gate `MV3-G07b`（`im_save_announcement`）的唯一出站入口，见下节。
//! - delete：`{channelId, announcementId}`，回包由 Announcement Core 触发版本化列表重拉。
//! - read：`{postId, channelId}`（标记某公告已读）。
//!
//! ## MV3-G07b｜发布 / 修改群公告（`im_save_announcement` → 内核名 `im_announcement_save`）
//!
//! 权威参考包：`specs/003-messagev3-gate-inventory/gates/announcement/mv3-g07b/`（visual 仓）。
//!
//! - **入参**（`inbound-sample.json`）：snake_case `{channel_id, announcement_id, content}`，
//!   `announcement_id` 为 `null` / 空串 = 新建，非空 = 编辑同一条公告。
//!   `inbound-contract.json.rust_commands[0].forbid_unknown = true` → 未知顶层键一律 fail-closed。
//! - **INV-01**：`temporaryId / createAt / simpleMessage / readBits` 归 Helix 所有，
//!   Angular 一旦提交即 fail-closed（[`HELIX_OWNED_KEYS`]），不是"照单透传"。
//! - **INV-10**：公告只调用公告专用接口 `POST post/announcement/save`
//!   （`contract.json.coverage.goEndpoint`），**不得**额外调用 `posts/create`；
//!   唯一 Post 由该接口的 WS 回声 / read-back 形成。
//! - **body**：后端要**完整 Post 结构**，由 Helix 按 `contract.json.inbound.params` 组装
//!   （camelCase），不再把 args 原样透传——透传会把 snake_case 入参直接送进 Go，
//!   且让 Angular 有机会自造 `temporaryId/readBits`。
//!
//! ### temporaryId 的确定性铸造
//!
//! 公告链是 **Http → Persist → Emit**（`effects-contract.json`，与发送内核的 Persist→Http 相反），
//! 无乐观落库，`temporaryId` 只用于 WS 回声对账。Helix 由「保存意图三元组」
//! `(channel_id, announcement_id, content)` 确定性铸造它：同一保存意图重放必须收敛到
//! **同一个** `temporaryId`，否则一次重试就会在时间线上留下两条 Post，直接违反 INV-10
//! 的「唯一 Post」。铸造是纯函数（无 Clock / 无 RNG / 无模块状态），满足 sans-IO。

use serde_json::{json, Map, Value};

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

// ── MV3-G07b：im_save_announcement ─────────────────────────────────────────────

/// 公告专用接口（`contract.json.coverage.goEndpoint` = `POST /post/announcement/save`）。
/// 唯一创建入口——INV-10 禁止本 Gate 触碰 `posts/create`。
pub const ANNOUNCEMENT_SAVE_PATH: &str = "post/announcement/save";

const SAVE_COMMAND: &str = "im_announcement_save";

/// `im_save_announcement` 公开边界允许的**全部**顶层键。
///
/// 前三个是 Gate 权威业务意图（`inbound-sample.json`）；`self_id/team_id/user_name/org_name/
/// dept_name` 是宿主注入的运行时身份（沿用 `im_create_channel` / `im_team_upsert` 的
/// 「壳只传身份，不拼 wire body」约定）；`req_id` 供 `Cses-Track-Id` 头。
const SAVE_KEYS: &[&str] = &[
    "channel_id",
    "announcement_id",
    "content",
    "self_id",
    "team_id",
    "user_name",
    "org_name",
    "dept_name",
    "req_id",
];

/// INV-01 归 Helix 所有、Angular **不得**提交的字段。
///
/// 单列出来是为了把「未知字段」与「越权字段」区分成两类错误：前者多半是拼写，
/// 后者说明壳又开始自造消息事实（tasks.md 记录的 G07b 端到端漂移正是此类）。
const HELIX_OWNED_KEYS: &[&str] = &[
    "temporary_id",
    "create_at",
    "simple_message",
    "read_bits",
    "message",
    "type",
    "viewers",
    "topic_id",
    "user_snapshot",
    "post",
    "props",
];

/// `im_save_announcement` 的已校验入参（Gate 权威业务意图 + 宿主身份）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveAnnouncementCommand {
    pub channel_id: String,
    /// 空串 = 新建；非空 = 编辑该 id 的公告（`contract.json`：区分新建 vs 编辑的唯一依据）。
    pub announcement_id: String,
    pub content: String,
    pub self_id: String,
    pub team_id: String,
    pub user_name: String,
    pub org_name: String,
    pub dept_name: String,
}

impl SaveAnnouncementCommand {
    /// 解析并校验保存意图；未知键 / INV-01 越权键 / 缺必填 → `Err`（fail closed，零 panic）。
    pub fn parse(args: &Value) -> Result<Self, ImError> {
        let obj = args
            .as_object()
            .ok_or_else(|| ImError::Parse(format!("{SAVE_COMMAND}: payload 必须是 JSON 对象")))?;
        for key in obj.keys() {
            if HELIX_OWNED_KEYS.contains(&key.as_str()) {
                return Err(ImError::Parse(format!(
                    "{SAVE_COMMAND}: 字段 {key} 由 Helix 铸造，客户端不得提交（INV-01）"
                )));
            }
            if !SAVE_KEYS.contains(&key.as_str()) {
                return Err(ImError::Parse(format!("{SAVE_COMMAND}: 未知字段 {key}")));
            }
        }
        Ok(Self {
            channel_id: require_str(args, "channel_id", SAVE_COMMAND)?.to_string(),
            announcement_id: optional_announcement_id(obj)?,
            content: require_str(args, "content", SAVE_COMMAND)?.to_string(),
            self_id: optional_str(obj, "self_id", SAVE_COMMAND)?,
            team_id: optional_str(obj, "team_id", SAVE_COMMAND)?,
            user_name: optional_str(obj, "user_name", SAVE_COMMAND)?,
            org_name: optional_str(obj, "org_name", SAVE_COMMAND)?,
            dept_name: optional_str(obj, "dept_name", SAVE_COMMAND)?,
        })
    }

    /// 保存意图的确定性对账键：同一 `(channel_id, announcement_id, content)` 恒为同一值。
    ///
    /// 见模块文档「temporaryId 的确定性铸造」。FNV-1a 64 位，纯函数、零依赖。
    pub fn temporary_id(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for part in [
            self.channel_id.as_str(),
            self.announcement_id.as_str(),
            self.content.as_str(),
        ] {
            for byte in part.as_bytes().iter().chain(std::iter::once(&0x1fu8)) {
                hash ^= *byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        format!("ann{hash:016x}")
    }

    /// 组装公告专用接口的完整 Post body（camelCase，逐字段对齐 `contract.json.inbound.params`）。
    pub fn wire_body(&self) -> Value {
        json!({
            "temporaryId": self.temporary_id(),
            "channelId": self.channel_id,
            "userId": self.self_id,
            "teamId": self.team_id,
            "topicId": "",
            "type": "ANNOUNCEMENT",
            // 公告正文既是消息体也是摘要（`outbound-sample.json` 两者同值）。
            "message": self.content,
            "simpleMessage": self.content,
            // 公告不走消息位图已读（`contract.json`：readBits 常量 ''）。
            "readBits": "",
            // 公告对全群可见，不支持定向（空 viewers 会让公告对任何人都不可见）。
            "viewers": ["all"],
            "userSnapshot": {
                "userId": self.self_id,
                "teamId": self.team_id,
                "userName": self.user_name,
                "orgName": self.org_name,
                "deptName": self.dept_name,
            },
            "props": {
                "announcement": {
                    "announcementId": self.announcement_id,
                    "content": self.content,
                }
            },
        })
    }
}

/// `announcement_id`：缺省 / `null` / 空串 = 新建（归一为空串）；非空字符串 = 编辑。
fn optional_announcement_id(obj: &Map<String, Value>) -> Result<String, ImError> {
    match obj.get("announcement_id") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(ImError::Parse(format!(
            "{SAVE_COMMAND}: announcement_id 必须是字符串或 null"
        ))),
    }
}

/// 可选字符串（缺省 / `null` → 空串；类型错 → `Err`，不静默吞掉）。
fn optional_str(obj: &Map<String, Value>, key: &str, cmd: &str) -> Result<String, ImError> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(value)) => Ok(value.clone()),
        Some(_) => Err(ImError::Parse(format!("{cmd}: {key} 必须是字符串"))),
    }
}

/// 公告删除只接受频道与公告身份；请求关联键不进入 Go body。
fn require_delete_keys(args: &Value, cmd: &str) -> Result<(), ImError> {
    let object = args
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{cmd}: payload 必须是 object")))?;
    if let Some(unknown) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "channel_id" | "announcement_id" | "req_id"))
    {
        return Err(ImError::Parse(format!(
            "{cmd}: 未知或非 canonical 字段 '{unknown}'"
        )));
    }
    if let Some(req_id) = object.get("req_id") {
        if req_id.as_str().is_none_or(str::is_empty) {
            return Err(ImError::Parse(format!("{cmd}: req_id 必须是非空字符串")));
        }
    }
    Ok(())
}

/// POST .../post/announcement/save — 发布 / 修改群公告（Gate `MV3-G07b`）。
///
/// 见模块文档；body 由 [`SaveAnnouncementCommand::wire_body`] 组装，**不**透传 args。
struct AnnouncementSaveCommand;
impl OutboundCommand for AnnouncementSaveCommand {
    fn name(&self) -> &'static str {
        SAVE_COMMAND
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let command = SaveAnnouncementCommand::parse(args)?;
        Ok((ANNOUNCEMENT_SAVE_PATH, command.wire_body()))
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
/// body `{channelId, announcementId}`；`req_id` 只用于读回关联，不进入 Go body。
struct AnnouncementDeleteCommand;
impl OutboundCommand for AnnouncementDeleteCommand {
    fn name(&self) -> &'static str {
        "im_announcement_delete"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        require_delete_keys(args, self.name())?;
        let channel_id = require_str(args, "channel_id", self.name())?;
        let announcement_id = require_str(args, "announcement_id", self.name())?;
        Ok((
            "post/announcement/delete",
            json!({ "channelId": channel_id, "announcementId": announcement_id }),
        ))
    }
    fn is_read(&self) -> bool {
        // G07c 的 HTTP 回包是结构化删除结果，后续由 Helix 触发同版本列表重拉。
        true
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

    /// save：Gate 权威 snake_case 意图 → 完整 camelCase Post body（**不**透传 args）。
    ///
    /// 旧断言 `body == args`（整体透传）随 MV3-G07b 落地作废：透传会把 snake_case 入参
    /// 直接送进 Go，并把 `temporaryId/readBits/simpleMessage` 的所有权让回客户端（违反 INV-01）。
    #[test]
    fn announcement_save_builds_full_post_body() {
        let args = json!({
            "channel_id": "c1",
            "announcement_id": null,
            "content": "公告内容",
            "self_id": "u1",
            "team_id": "t1",
        });
        let (url, body) = dispatch("im_announcement_save", &args);
        assert_eq!(url, "http://h/api/post/announcement/save");
        assert_eq!(body["type"], "ANNOUNCEMENT");
        assert_eq!(body["channelId"], "c1");
        assert_eq!(body["userId"], "u1");
        assert_eq!(body["message"], "公告内容");
        assert_eq!(body["simpleMessage"], "公告内容");
        assert_eq!(body["readBits"], "");
        assert_eq!(body["viewers"], json!(["all"]));
        assert_eq!(body["props"]["announcement"]["announcementId"], "");
        assert_eq!(body["props"]["announcement"]["content"], "公告内容");
        // snake_case 入参不得泄漏到 wire。
        assert!(body.get("channel_id").is_none());
        assert!(body.get("self_id").is_none());
    }

    /// save：客户端自造 Helix 所有权字段 → Err（INV-01 fail closed）。
    #[test]
    fn announcement_save_rejects_client_minted_fields() {
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({
            "channel_id": "c1",
            "content": "公告内容",
            "temporary_id": "client-minted",
        }))
        .unwrap();
        assert!(handle_outbound(
            "im_announcement_save",
            &payload,
            "http://h/api",
            "http://h",
            None,
            corr
        )
        .is_err());
    }

    /// delete：频道与公告身份进入 wire，req_id 不泄漏到 Go body。
    #[test]
    fn announcement_delete_builds_channel_announcement_body() {
        let args = json!({
            "channel_id": "c1",
            "announcement_id": "a1",
            "req_id": "rq-delete"
        });
        let (url, body) = dispatch("im_announcement_delete", &args);
        assert_eq!(url, "http://h/api/post/announcement/delete");
        assert_eq!(body, json!({ "channelId": "c1", "announcementId": "a1" }));
    }

    /// delete：缺身份、旧字段或未知字段 → Err（不 panic）。
    #[test]
    fn announcement_delete_rejects_legacy_or_invalid_fields() {
        let corr = Correlation::from_raw(1);
        for args in [
            json!({ "post_ids": ["p1"] }),
            json!({ "channel_id": "c1" }),
            json!({ "channel_id": "c1", "announcement_id": "a1", "user_id": "u1" }),
            json!({ "channel_id": "c1", "announcement_id": "a1", "req_id": "" }),
        ] {
            let payload = serde_json::to_vec(&args).unwrap();
            assert!(
                handle_outbound(
                    "im_announcement_delete",
                    &payload,
                    "http://h/api",
                    "http://h",
                    None,
                    corr
                )
                .is_err(),
                "args={args}"
            );
        }
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
