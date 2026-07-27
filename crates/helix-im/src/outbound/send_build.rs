//! `posts/create` 出站 body 组装（现网 `messageSend.service.ts::buildMessageObject` 编排下沉）。
//!
//! ## 为什么独立成纯函数
//!
//! 现网前端 `buildMessageObject` 把 UI 表单字段补成完整 wire body（viewers 空补、回复 root
//! 合并、userSnapshot 注入、文本截断、空消息拦截）。P1 接管把这套编排搬进 helix-im，
//! 前端退化为只发 UI 字段的语义 invoke（C1 纯渲染证伪：invoke 参数无 wire-only key）。
//!
//! 本函数是**纯函数**（无 I/O / 无 ambient 时间）：`createAt` 经 `now_ms`（Clock 注入，C3）、
//! temporary_id 由 ImModule 的确定性分配器铸造，调用方只提交发送意图。
//!
//! ## 行为真源
//!
//! - body 字段集 + casing：`docs/csesapi-migration/真机curl真源.md §1`（真抓包逐字 body）。
//! - viewers 空补 / findReplyRootId / userSnapshot 5 字段 / 2000 截断 / 空消息 trim：
//!   现网 `src/pages/message-v3/service/messageSend.service.ts`（buildMessageObject / sendMessage）。
//!
//! ## 关键不变量
//!
//! - 回声对账锚 `action=="post"`（**非 `posted`**）+ body 字段 `createAt`（**非 `createdAt`**，B3）。
//! - `temporary_id` 是主键，全程不变（乐观落库 / Http body / echo reconcile 同一值）。

use helix_core::effect::{SqlValue, StorageOp, UpsertSpec};
use serde_json::{json, Map, Value};

/// 文本最大字符数（现网 `snapshotToPlainText(snapshot, 2000)`）。
///
/// 注：现网在前端把富文本 snapshot 渲染成纯文本时截断 2000；helix 接管后前端只传纯文本，
/// 故此处对入站纯文本做同样的 2000 字符上界兜底（防超长 body）。截断按 **char**（Unicode
/// scalar）计数而非 byte，与 JS `String.slice(0, 2000)` 的 UTF-16 语义近似（CJK 同为 1）。
pub const MAX_MESSAGE_CHARS: usize = 2000;

/// 当前登录用户身份（host 经 ImConfig 注入，确定性身份；core/im 不自取登录态）。
///
/// `org_name` / `dept_name` / `user_name` 是 ucenter 跨 feature 读依赖（现网
/// `this.ctx.userInfo`）——P1 由 host 经 ImConfig 占位注入（未注入则空串 mock），
/// 缺这三非阻断字段不影响 Go 落库 / 推回声（主键身份是 userId/teamId）。
pub struct UserIdentity<'a> {
    pub user_id: &'a str,
    pub team_id: &'a str,
    pub user_name: &'a str,
    pub org_name: &'a str,
    pub dept_name: &'a str,
}

/// 被回复消息的本地引用字段（findReplyRootId 的输入，对齐现网 `localRepliedMessage`）。
///
/// 全部 owned/借用纯数据，无生命周期进 async（HX-C002）。`None` = 非回复消息。
pub struct RepliedRef<'a> {
    /// 被回复消息自身的 id（现网 `message.id`）。
    pub id: &'a str,
    /// 被回复消息的 replyId（它是否回复过别人）。
    pub reply_id: &'a str,
    /// 被回复消息的 replyRootId。
    pub reply_root_id: &'a str,
    /// 被回复消息的 replyFirstLevelId。
    pub reply_first_level_id: &'a str,
}

/// buildMessageObject 的输入（UI 表单字段 + 注入身份/时间）。
pub struct BuildInput<'a> {
    pub channel_id: &'a str,
    pub temporary_id: &'a str,
    /// 消息类型（`entity.Post.Type`，wire `type`）。默认 `"TEXT"`；DOCUMENT 等富媒体传真值——硬编 `"TEXT"` 降级纯文本（UC-1.2）。
    pub msg_type: &'a str,
    /// 消息 id（`entity.Post.ID`，wire `id`）。默认空串 = 新建；编辑传被编辑消息真 post id（非空 → Go update 而非 create，UC-1.6）。
    pub post_id: &'a str,
    /// UI 纯文本（已是 snapshotToPlainText 结果；此处再做 2000 兜底截断）。
    pub text: &'a str,
    /// UI 选的可见性（空 → 补 `["all"]`）。
    pub viewers: Vec<String>,
    /// @ 提及列表（透传，默认空数组）。
    pub mentions: Value,
    /// 富载荷 props（透传，默认空对象）。
    pub props: Value,
    /// 话题 id（默认空串）。
    pub topic_id: &'a str,
    /// 被回复消息引用（None = 普通消息）。
    pub replied: Option<RepliedRef<'a>>,
    /// Clock 注入的当前毫秒时间（C3：禁 ambient now）。
    pub now_ms: u64,
    /// host 注入的登录身份。
    pub identity: UserIdentity<'a>,
}

/// findReplyRootId（现网 `utils/index.ts::findReplyRootId`）。
///
/// 输入被回复消息的引用，输出本条新消息的 `{replyId, replyRootId, replyFirstLevelId}`
/// 三字段（合并进 body）。四分支严格对齐源：
/// 1. 被回复消息没回复过别人（`reply_id` 空）→ 它就是根：`replyId=replyRootId=id`，firstLevel 空。
/// 2. 被回复消息回复过、且有 firstLevel → 继承 root/firstLevel，`replyId=id`。
/// 3. 被回复消息回复过、但无 firstLevel（直接回根）→ `replyId=id`，`firstLevel=id`，继承 root。
///
/// 返回 `(reply_id, reply_root_id, reply_first_level_id)`，调用方按非空写入 body。
fn find_reply_root_id(replied: &RepliedRef<'_>) -> (String, String, String) {
    let id = replied.id;
    if replied.reply_id.is_empty() {
        // 分支 1：被回复消息是根消息
        return (id.to_string(), id.to_string(), String::new());
    }
    if !replied.reply_first_level_id.is_empty() {
        // 分支 2：被回复消息回复过、且曾回复一级回复
        return (
            id.to_string(),
            replied.reply_root_id.to_string(),
            replied.reply_first_level_id.to_string(),
        );
    }
    // 分支 3：被回复消息直接回复了根
    (
        id.to_string(),
        replied.reply_root_id.to_string(),
        id.to_string(),
    )
}

/// 按 char 截断到 `MAX_MESSAGE_CHARS`（对齐 snapshotToPlainText 的 2000 上界）。
fn truncate_chars(text: &str) -> String {
    if text.chars().count() <= MAX_MESSAGE_CHARS {
        return text.to_string();
    }
    text.chars().take(MAX_MESSAGE_CHARS).collect()
}

/// 空消息拦截：现网 `sendMessage` 过滤 `message.trim()` 为空 **且** props 为空对象的消息。
///
/// 返回 `true` = 应拦截（不发）。`text` 去首尾空白后为空、且 `props` 不是非空对象 → 拦截。
fn should_intercept_empty(text: &str, props: &Value) -> bool {
    let has_valid_message = !text.trim().is_empty();
    let has_valid_props = props.as_object().map(|m| !m.is_empty()).unwrap_or(false);
    !(has_valid_message || has_valid_props)
}

fn canonical_message_type(msg_type: &str) -> &str {
    match msg_type {
        "rich" | "IMAGE" => "IMAGE",
        "file" | "FILE" => "FILE",
        "" => "TEXT",
        other => other,
    }
}

fn simple_message(msg_type: &str, message: &str, props: &Value) -> String {
    let (prefix, content) = match msg_type {
        "IMAGE" => ("[图片]", message),
        "FILE" => (
            "[文件]",
            props
                .pointer("/file/name")
                .or_else(|| props.pointer("/file/mediaInput/fileName"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .unwrap_or(message),
        ),
        "AUDIO" => ("[音频]", message),
        "VIDEO" => ("[视频]", message),
        _ => ("", message),
    };
    prefix.chars().chain(content.chars()).take(50).collect()
}

/// 组装 `posts/create` body（现网 buildMessageObject + sendMessage 空消息拦截）。
///
/// 返回 `None` = 空消息被拦截（不产出 Http）；`Some(body)` = 完整 camelCase wire body。
///
/// body 字段集逐字对齐真抓包真源（§1）：`viewers/message/mentions/temporaryId/type/`
/// `simpleMessage/channelId/userId/teamId/userSnapshot/id/props/topicId/revoke` + 注入
/// `createAt`（Clock）+ 回复三字段（findReplyRootId 命中时）。
pub fn build_message_object(input: &BuildInput<'_>) -> Option<Value> {
    // 空消息拦截（trim 空 + props 空 → 不发）。
    if should_intercept_empty(input.text, &input.props) {
        return None;
    }

    let message = truncate_chars(input.text);
    let msg_type = canonical_message_type(input.msg_type);
    // 会话列表摘要沿用现网媒体前缀；按 char 保持总长不超过 50。
    let simple_message = simple_message(msg_type, &message, &input.props);

    // viewers 空补 ["all"]（现网 `if (message.viewers.length === 0) viewers = ['all']`）。
    let viewers: Vec<String> = if input.viewers.is_empty() {
        vec!["all".to_string()]
    } else {
        input.viewers.clone()
    };

    let id = &input.identity;
    let mut body: Map<String, Value> = Map::new();
    body.insert("viewers".to_string(), json!(viewers));
    body.insert("message".to_string(), json!(message));
    body.insert("mentions".to_string(), input.mentions.clone());
    body.insert("temporaryId".to_string(), json!(input.temporary_id));
    // type：默认 "TEXT"，文档/图片等富媒体透传真值（UC-1.2）。空串容错为 "TEXT"。
    body.insert("type".to_string(), json!(msg_type));
    body.insert("simpleMessage".to_string(), json!(simple_message));
    body.insert("channelId".to_string(), json!(input.channel_id));
    body.insert("userId".to_string(), json!(id.user_id));
    body.insert("teamId".to_string(), json!(id.team_id));
    body.insert(
        "userSnapshot".to_string(),
        json!({
            "orgName": id.org_name,
            "deptName": id.dept_name,
            "userName": id.user_name,
            "userId": id.user_id,
            "teamId": id.team_id,
        }),
    );
    // id：新建时空串（现网 self message `id: ''`）；编辑时传被编辑消息真 post id（非空 →
    // Go 当 update 而非 create，UC-1.6）。透传 `input.post_id`，默认空串。
    body.insert("id".to_string(), json!(input.post_id));
    body.insert("props".to_string(), input.props.clone());
    body.insert("topicId".to_string(), json!(input.topic_id));
    body.insert("revoke".to_string(), json!(false));
    // C3：createAt 经 Clock 注入（**非 createdAt**，B3）。
    body.insert("createAt".to_string(), json!(input.now_ms));

    // 回复 root 合并（findReplyRootId 命中时写三字段，对齐 `...(localRepliedMessage ? ... : {})`）。
    if let Some(replied) = &input.replied {
        let (reply_id, reply_root_id, reply_first_level_id) = find_reply_root_id(replied);
        body.insert("replyId".to_string(), json!(reply_id));
        body.insert("replyRootId".to_string(), json!(reply_root_id));
        // firstLevel 可能为空串（分支 1）——现网原样写入（不省略 key）。
        body.insert("replyFirstLevelId".to_string(), json!(reply_first_level_id));
    }

    Some(Value::Object(body))
}

/// 乐观落库 `Effect::Persist` 的 message 表 BatchUpsert op（status=sending，PK=temporary_id）。
///
/// 单行 INSERT/UPDATE WHERE PK（HX-C005 热路径 O(1)，禁 RMW）。msgId/channelId 是 26 字符
/// base32 → Text（ADR-009）。`text` 已是 2000 截断后的纯文本（与 body.message 同源）。
pub fn optimistic_persist_op(temporary_id: &str, channel_id: &str, text: String) -> StorageOp {
    StorageOp::BatchUpsert(UpsertSpec {
        table: "message",
        rows: vec![vec![
            (
                "temporary_id".to_string(),
                SqlValue::Text(temporary_id.to_string()),
            ),
            (
                "channel_id".to_string(),
                SqlValue::Text(channel_id.to_string()),
            ),
            ("message".to_string(), SqlValue::Text(text)),
            (
                "send_status".to_string(),
                SqlValue::Text("sending".to_string()),
            ),
        ]],
        conflict_key: Some("temporary_id"),
        exclude_from_update: Vec::new(),
    })
}

/// 从 `im_send_message` 命令载荷 + 注入身份/时间组装 body（边界解析 + 调 `build_message_object`）。
///
/// 命令载荷字段（前端只传 UI 字段，C1 纯渲染）：`text`（纯文本）/ `viewers`（空数组省略亦可）/
/// `mentions` / `props` / `topic_id` / `type`（默认 `"TEXT"`，DOCUMENT 等富媒体）/ `id`（默认空串，
/// 编辑时传被编辑消息真 post id）/ `replied{id,reply_id,reply_root_id,reply_first_level_id}`。
/// 返回 `None` = 空消息拦截（不发）。`channel_id` / `temporary_id` 由调用方先校验（typed PK）后传入。
pub fn build_from_command(
    cmd: &Value,
    channel_id: &str,
    temporary_id: &str,
    now_ms: u64,
    identity: &UserIdentity<'_>,
) -> Option<Value> {
    let text = cmd["text"].as_str().unwrap_or("");
    // 零信任 default：缺 type → "TEXT"；缺 id → ""（新建）。前端 DOCUMENT/编辑时显式传。
    let msg_type = cmd["type"].as_str().unwrap_or("TEXT");
    let post_id = cmd["id"].as_str().unwrap_or("");
    let viewers: Vec<String> = cmd["viewers"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let mentions = cmd.get("mentions").cloned().unwrap_or_else(|| json!([]));
    let props = cmd.get("props").cloned().unwrap_or_else(|| json!({}));
    let topic_id = cmd["topic_id"].as_str().unwrap_or("");
    // 被回复消息引用（前端按本地库取出 localRepliedMessage 的 4 字段透传）。
    let replied_owned = cmd.get("replied").and_then(|r| {
        let id = r["id"].as_str()?.to_string();
        Some((
            id,
            r["reply_id"].as_str().unwrap_or("").to_string(),
            r["reply_root_id"].as_str().unwrap_or("").to_string(),
            r["reply_first_level_id"].as_str().unwrap_or("").to_string(),
        ))
    });
    let replied = replied_owned
        .as_ref()
        .map(|(id, rid, rroot, rfirst)| RepliedRef {
            id,
            reply_id: rid,
            reply_root_id: rroot,
            reply_first_level_id: rfirst,
        });
    build_message_object(&BuildInput {
        channel_id,
        temporary_id,
        msg_type,
        post_id,
        text,
        viewers,
        mentions,
        props,
        topic_id,
        replied,
        now_ms,
        identity: UserIdentity {
            user_id: identity.user_id,
            team_id: identity.team_id,
            user_name: identity.user_name,
            org_name: identity.org_name,
            dept_name: identity.dept_name,
        },
    })
}
