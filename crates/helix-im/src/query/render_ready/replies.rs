//! S8 回复族 render-ready 投影 —— `im:channel:replies`（issue #57·C013 纯渲染壳·终局切）。
//!
//! 把回复读接口整形成扁平 RenderNode：前端只维护 `messageById` 与 `replyIdsByRoot` 两个索引，
//! 不递归组树、不按 root 重扫消息数组。
//!
//! ## 形态
//! emit `im:channel:replies{reqId,channelId,rootMessageId,projectionMode,revision,
//! replyIds,nodes,replyCount,cursor,hasMore}`：
//!   - `reqId`    = 前端 bridge 注入的请求关联键（与 `im:read:result` 同一 req_id·壳认领抽屉归属）。
//!   - `replyIds` = 不含 root 的回复 id，去重保序；可 O(1) 写入 `replyIdsByRoot[rootMessageId]`。
//!   - `nodes`    = root + replies 的 render-ready message 行；可 O(n) 写入 `messageById`。
//!   - `replyCount` = 服务端总数与本页回复数的较大值。
//!
//! ## 与冻结投影的关系（契约只读·C004）
//! 本投影是 **render-ready 渲染通道**（与 `im:read:result` §1.2 / `im:channel:members` §1.5 同性质·
//! **不计入** 21 投影冻结集——前端直绑、后端响应体权威）。冻结的 `im:read:result`（`{req_id, body}`）
//! 投影**保持原样**额外 emit（UC-2.4 ② 契约面照旧裁定 verbatim body）——本投影**额外** emit，不改既有键集。

use crate::error::ImError;
use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyProjectionMode {
    Snapshot,
    Append,
}

impl ReplyProjectionMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Append => "append",
        }
    }
}

/// HTTP 发起时冻结的回复投影关联上下文。业务模式来自 command 名，不从 `reqId` 猜测。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyProjectionRequest {
    pub req_id: String,
    pub channel_id: String,
    pub root_hint: String,
    pub mode: ReplyProjectionMode,
    pub revision: u64,
    pub viewer_user_id: String,
    pub page_number: u32,
    pub page_size: u32,
}

impl ReplyProjectionRequest {
    /// 从 command payload 冻结 reqId、主锚点和页界，供 PortReply 乱序隔离使用。
    pub fn from_command(name: &str, payload: &[u8], revision: u64, viewer: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
        let req_id = value
            .get("req_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())?
            .to_string();
        let (mode, root_key) = match name {
            "im_get_replies" => (ReplyProjectionMode::Snapshot, "reply_id"),
            "im_get_reply_branch" => (ReplyProjectionMode::Append, "reply_first_level_id"),
            _ => return None,
        };
        let root_hint = match mode {
            // Branch state is keyed by the first-level reply, not the enclosing thread root.
            ReplyProjectionMode::Append => {
                value.get(root_key).or_else(|| value.get("root_message_id"))
            }
            ReplyProjectionMode::Snapshot => {
                value.get("root_message_id").or_else(|| value.get(root_key))
            }
        }
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())?
        .to_string();
        let page_number = parse_page_number(value.get("page_number")).ok()?;
        let page_size = crate::timeline_state::TimelinePageSize::parse(value.get("page_size"))
            .ok()?
            .get();
        Some(Self {
            req_id,
            channel_id: value
                .get("channel_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            root_hint,
            mode,
            revision,
            viewer_user_id: viewer.to_string(),
            page_number,
            page_size,
        })
    }
}

/// G12a/G12b 一次回复页的结构化终态，保留请求关联、根消息和 Go 页界。
#[derive(Debug, Clone, PartialEq)]
pub struct RepliesPageResult {
    pub req_id: String,
    pub root_post: serde_json::Value,
    pub replies: Vec<serde_json::Value>,
    pub has_more: bool,
    pub page_number: u32,
    pub page_size: u32,
}

impl RepliesPageResult {
    /// 转为 Tauri adapter 消费的 camelCase Command Result body。
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "reqId": self.req_id,
            "rootPost": self.root_post,
            "replies": self.replies,
            "hasMore": self.has_more,
            "page": {
                "pageNumber": self.page_number,
                "pageSize": self.page_size,
            },
        })
    }
}

/// 从 command payload 解析缺省为 1、显式必须为正整数的页码。
fn parse_page_number(value: Option<&serde_json::Value>) -> Result<u32, ImError> {
    let Some(value) = value else {
        return Ok(1);
    };
    let raw = value
        .as_u64()
        .ok_or_else(|| ImError::Parse("reply pageNumber must be a positive integer".to_string()))?;
    let page = u32::try_from(raw)
        .map_err(|_| ImError::Parse("reply pageNumber is out of range".to_string()))?;
    if page == 0 {
        return Err(ImError::Parse(
            "reply pageNumber must be a positive integer".to_string(),
        ));
    }
    Ok(page)
}

/// 将 Go getReplies authority body转换为 root-isolated、viewer-filtered 的单页结果。
///
/// 该转换只遍历一次根与当前页（O(pageSize)），绝不发起额外 `getPosts`；缺失
/// `hasMore`、页界或回复数组均 fail-closed，避免把未知分页误当成 `false`。
pub fn project_replies_page(
    request: &ReplyProjectionRequest,
    body: &serde_json::Value,
    viewer_user_id: &str,
) -> Result<RepliesPageResult, ImError> {
    let payload = reply_payload(body)?;
    let has_more = payload
        .get("hasMore")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| ImError::Parse("reply result missing boolean hasMore".to_string()))?;
    let (page_number, page_size) = match payload.get("page") {
        Some(page) => {
            let page = page
                .as_object()
                .ok_or_else(|| ImError::Parse("reply result page must be an object".to_string()))?;
            let page_number = parse_page_number(page.get("pageNumber"))?;
            let page_size = crate::timeline_state::TimelinePageSize::parse(page.get("pageSize"))
                .map_err(|error| ImError::Parse(format!("reply result pageSize: {error}")))?
                .get();
            if page_number != request.page_number || page_size != request.page_size {
                return Err(ImError::Parse(
                    "reply result page does not match request".to_string(),
                ));
            }
            (page_number, page_size)
        }
        None if request.mode == ReplyProjectionMode::Append => {
            // Go getReplyBranch returns HasMorePage{data,hasMore}; page state stays in request.
            (request.page_number, request.page_size)
        }
        None => return Err(ImError::Parse("reply result missing page".to_string())),
    };
    let replies = payload
        .get(if request.mode == ReplyProjectionMode::Append {
            "data"
        } else {
            "replies"
        })
        .and_then(serde_json::Value::as_array)
        .or_else(|| {
            // Accept an adapter that has already renamed HasMorePage.data to `replies`.
            (request.mode == ReplyProjectionMode::Append)
                .then(|| payload.get("replies").and_then(serde_json::Value::as_array))
                .flatten()
        })
        .ok_or_else(|| ImError::Parse("reply result missing reply data array".to_string()))?;

    let root_post = payload
        .get("rootPost")
        .filter(|_| request.mode == ReplyProjectionMode::Snapshot)
        .filter(|post| {
            is_visible_to_viewer(post, viewer_user_id)
                && post_identity_matches(post, request.root_hint.as_str())
        })
        .cloned()
        .unwrap_or(serde_json::Value::Null);

    let mut seen_ids = HashSet::with_capacity(replies.len());
    let mut seen_temporary_ids = HashSet::with_capacity(replies.len());
    let mut projected_replies = Vec::with_capacity(replies.len());
    for reply in replies {
        if !is_visible_to_viewer(reply, viewer_user_id)
            || !reply_belongs_to_root_for_mode(reply, request.root_hint.as_str(), request.mode)
        {
            continue;
        }
        let Some((id, temporary_id)) = reply_identity(reply) else {
            continue;
        };
        if seen_ids.contains(&id) || seen_temporary_ids.contains(&temporary_id) {
            continue;
        }
        seen_ids.insert(id);
        seen_temporary_ids.insert(temporary_id);
        projected_replies.push(reply.clone());
    }

    Ok(RepliesPageResult {
        req_id: request.req_id.clone(),
        root_post,
        replies: projected_replies,
        has_more,
        page_number,
        page_size,
    })
}

/// 解开 Go CommonRes data 外壳，保留缺失字段的显式错误语义。
fn reply_payload(body: &serde_json::Value) -> Result<&serde_json::Value, ImError> {
    let payload = body
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(body);
    if !payload.is_object() {
        return Err(ImError::Parse("reply result must be an object".to_string()));
    }
    Ok(payload)
}

/// 判定消息是否属于当前 viewer；空受众沿用 Helix 的未绑定 viewer 宽容语义。
fn is_visible_to_viewer(post: &serde_json::Value, viewer_user_id: &str) -> bool {
    if post.get("type").and_then(serde_json::Value::as_str) == Some("NOTICE")
        || viewer_user_id.is_empty()
    {
        return true;
    }
    let Some(viewers) = post.get("viewers") else {
        return true;
    };
    if let Some(items) = viewers.as_array() {
        return items
            .iter()
            .filter_map(serde_json::Value::as_str)
            .any(|id| id == "all" || id == viewer_user_id);
    }
    viewers
        .as_str()
        .and_then(|raw| serde_json::from_str::<Vec<String>>(raw).ok())
        .is_some_and(|items| items.iter().any(|id| id == "all" || id == viewer_user_id))
}

/// 读取消息的 server/temporary 双锚，供 root 校验与页内幂等去重共用。
fn reply_identity(post: &serde_json::Value) -> Option<(String, String)> {
    let id = post
        .get("id")
        .or_else(|| post.get("msgId"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let temporary_id = post
        .get("temporaryId")
        .or_else(|| post.get("temporary_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    match (id, temporary_id) {
        (Some(id), Some(temporary_id)) => Some((id, temporary_id)),
        (Some(id), None) => Some((id.clone(), id)),
        (None, Some(temporary_id)) => Some((temporary_id.clone(), temporary_id)),
        (None, None) => None,
    }
}

/// 判断 rootPost 是否与请求根同一 server/temporary 锚。
fn post_identity_matches(post: &serde_json::Value, root_hint: &str) -> bool {
    reply_identity(post)
        .is_some_and(|(id, temporary_id)| id == root_hint || temporary_id == root_hint)
}

/// 只接受当前 endpoint 的主锚关系，避免根线程与一级分支互相污染。
fn reply_belongs_to_root_for_mode(
    post: &serde_json::Value,
    root_hint: &str,
    mode: ReplyProjectionMode,
) -> bool {
    let keys: &[&str] = match mode {
        ReplyProjectionMode::Snapshot => &[
            "rootId",
            "root_id",
            "replyRootId",
            "reply_root_id",
            "replyId",
            "reply_id",
            "parentId",
            "parent_id",
        ],
        ReplyProjectionMode::Append => &[
            "replyFirstLevelId",
            "reply_first_level_id",
            "parentId",
            "parent_id",
        ],
    };
    keys.iter()
        .find_map(|key| post.get(*key).and_then(serde_json::Value::as_str))
        .is_some_and(|relation| relation == root_hint)
}

/// 将一次成功的回复页以唯一 `im:read:result` terminal 交给 Host/Adapter。
pub fn emit_replies_page_result(result: &RepliesPageResult) -> Effect {
    crate::read_relay::emit_read_body(&result.req_id, result.to_value())
}

#[derive(Debug, Clone, PartialEq)]
pub struct FlatReplyProjection {
    pub channel_id: String,
    pub root_message_id: String,
    pub projection_mode: ReplyProjectionMode,
    pub revision: u64,
    pub reply_ids: Vec<String>,
    pub nodes: Vec<serde_json::Value>,
    pub reply_count: usize,
    pub has_more: bool,
}

impl FlatReplyProjection {
    pub fn empty() -> Self {
        Self {
            channel_id: String::new(),
            root_message_id: String::new(),
            projection_mode: ReplyProjectionMode::Snapshot,
            revision: 0,
            reply_ids: Vec::new(),
            nodes: Vec::new(),
            reply_count: 0,
            has_more: false,
        }
    }
}

/// 从读族透传 body 防御性抽 Post id 列表（render-ready·下沉自前端 `read-result-extract.ts`）。
///
/// 探针顺序兼容两 endpoint 不同壳形态：
///  - getReplies：`{rootPost, replies:[Post]}`（partial 1 §15 GetRepliesResp）
///  - getReplyBranch：HasMorePage[Post]（`{data:[Post]}` / `{list:[Post]}` / 顶层 `[Post]`）
/// 取 rootPost.id + replies[].id + data[].id + list[].id + 顶层数组[].id。去重保序·仅非空 string。
/// body 缺/非对象（失败回灌 `{req_id, error}` 或畸形）→ 返空数组（前端 reject 语义·清抽屉）。
/// `viewer_user_id` 是请求发起时冻结的 host 身份，保证回复读投影与 WS/首屏投影使用同一作者视角。
pub fn extract_flat_replies(body: &serde_json::Value, viewer_user_id: &str) -> FlatReplyProjection {
    // Go HTTP 统一 CommonRes 会把业务结果包在 `data`；先剥一层信封，再按两类业务结果抽取。
    // getReplyBranch 的业务结果自身仍有 `data:[Post]`，下方循环继续处理该数组。
    let raw_body = body;
    let body = raw_body
        .get("data")
        .filter(|value| value.is_object() || value.is_array())
        .unwrap_or(raw_body);
    let mut projection = FlatReplyProjection::empty();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |post: &serde_json::Value, is_root: bool| {
        if !is_visible_to_viewer(post, viewer_user_id) {
            return;
        }
        let id = post
            .get("id")
            .or_else(|| post.get("msgId"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if id.is_empty() || !seen.insert(id.to_string()) {
            return;
        }
        if is_root {
            projection.root_message_id = id.to_string();
        } else {
            projection.reply_ids.push(id.to_string());
        }
        projection
            .nodes
            .push(super::core::shape_row(post, viewer_user_id));
    };
    if let Some(obj) = body.as_object() {
        if let Some(root) = obj.get("rootPost") {
            push(root, true);
        }
        if let Some(arr) = ["replies", "data", "list"]
            .iter()
            .find_map(|key| obj.get(*key).and_then(serde_json::Value::as_array))
        {
            for post in arr {
                push(post, false);
            }
        }
    } else if let Some(arr) = body.as_array() {
        for post in arr {
            push(post, false);
        }
    }
    drop(push);

    if projection.root_message_id.is_empty() {
        projection.root_message_id = projection
            .nodes
            .first()
            .and_then(|node| {
                ["replyRootId", "replyId", "msgId"]
                    .iter()
                    .find_map(|key| node.get(*key).and_then(serde_json::Value::as_str))
            })
            .unwrap_or("")
            .to_string();
    }
    let server_count = projection
        .nodes
        .first()
        .and_then(|node| node.get("replyCount"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default() as usize;
    projection.reply_count = server_count.max(projection.reply_ids.len());
    projection.channel_id = projection
        .nodes
        .iter()
        .find_map(|node| {
            node.get("channelId")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("")
        .to_string();
    projection.has_more = raw_body
        .get("hasMore")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| body.get("hasMore").and_then(serde_json::Value::as_bool))
        .unwrap_or(false);
    projection
}

/// 把 command 语义和响应数据合并成稳定投影；缺失的服务端游标明确冻结为 `null`。
pub fn finalize_projection(
    request: &ReplyProjectionRequest,
    mut projection: FlatReplyProjection,
) -> FlatReplyProjection {
    projection.projection_mode = request.mode;
    projection.revision = request.revision;
    if projection.channel_id.is_empty() {
        projection.channel_id.clone_from(&request.channel_id);
    }
    if request.mode == ReplyProjectionMode::Append && !request.root_hint.is_empty() {
        // Branch replies are keyed by first-level id even though every row also carries root id.
        projection.root_message_id.clone_from(&request.root_hint);
        projection.nodes.retain(|node| {
            reply_belongs_to_root_for_mode(node, request.root_hint.as_str(), request.mode)
        });
        let mut seen = HashSet::with_capacity(projection.nodes.len());
        projection.reply_ids = projection
            .nodes
            .iter()
            .filter_map(reply_identity)
            .map(|(id, _)| id)
            .filter(|id| seen.insert(id.clone()))
            .collect();
        projection.reply_count = projection.reply_ids.len();
    } else if projection.root_message_id.is_empty() {
        projection.root_message_id.clone_from(&request.root_hint);
    }
    if !projection_matches_request(request, &projection) {
        // Reject the complete authority batch instead of leaking a mixed-channel or unrelated
        // reply relation to a renderer. Preserve request identity only so pending UI can resolve.
        projection.channel_id.clone_from(&request.channel_id);
        projection.root_message_id.clone_from(&request.root_hint);
        projection.reply_ids.clear();
        projection.nodes.clear();
        projection.reply_count = 0;
        projection.has_more = false;
    }
    projection
}

/// Validate reply authority against the request context captured when HTTP began.
pub fn projection_matches_request(
    request: &ReplyProjectionRequest,
    projection: &FlatReplyProjection,
) -> bool {
    if !request.channel_id.is_empty()
        && !projection.channel_id.is_empty()
        && projection.channel_id != request.channel_id
    {
        return false;
    }
    if request.mode == ReplyProjectionMode::Snapshot
        && !request.root_hint.is_empty()
        && !projection.root_message_id.is_empty()
        && projection.root_message_id != request.root_hint
    {
        return false;
    }
    projection.nodes.iter().all(|node| {
        let channel = node
            .get("channelId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if !request.channel_id.is_empty() && !channel.is_empty() && channel != request.channel_id {
            return false;
        }
        let id = node
            .get("msgId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        if id == projection.root_message_id {
            return true;
        }
        let relation = reply_relation(node, request.mode).unwrap_or("");
        relation.is_empty()
            || relation == projection.root_message_id.as_str()
            || (request.mode == ReplyProjectionMode::Append
                && relation == request.root_hint.as_str())
    })
}

/// 读取当前投影模式的主关联字段；数组首命中规则拒绝两个锚点的模糊混用。
fn reply_relation(post: &serde_json::Value, mode: ReplyProjectionMode) -> Option<&str> {
    let keys: &[&str] = match mode {
        ReplyProjectionMode::Snapshot => &["replyRootId", "reply_root_id", "replyId", "reply_id"],
        ReplyProjectionMode::Append => &[
            "replyFirstLevelId",
            "reply_first_level_id",
            "parentId",
            "parent_id",
        ],
    };
    keys.iter()
        .find_map(|key| post.get(*key).and_then(serde_json::Value::as_str))
        .filter(|value| !value.is_empty())
}

/// 在 Helix 内完成乱序抑制和 append 跨页去重，Angular 不解释 revision/模式。
///
/// - snapshot：权威替换已见 id 集；
/// - append：只下发从未出现的 id，保持本次服务端顺序；
/// - 旧 revision：不 emit render-ready 投影，避免覆盖更新快照。
pub fn accept_projection(
    projection: &mut FlatReplyProjection,
    revisions: &mut HashMap<String, u64>,
    seen_ids: &mut HashMap<String, HashSet<String>>,
) -> bool {
    let root = projection.root_message_id.as_str();
    if root.is_empty() {
        return true;
    }
    if revisions
        .get(root)
        .is_some_and(|latest| *latest > projection.revision)
    {
        return false;
    }
    revisions.insert(root.to_string(), projection.revision);

    match projection.projection_mode {
        ReplyProjectionMode::Snapshot => {
            seen_ids.insert(
                root.to_string(),
                projection.reply_ids.iter().cloned().collect(),
            );
        }
        ReplyProjectionMode::Append => {
            let seen = seen_ids.entry(root.to_string()).or_default();
            let accepted: HashSet<String> = projection
                .reply_ids
                .iter()
                .filter(|id| !seen.contains(*id))
                .cloned()
                .collect();
            projection.reply_ids.retain(|id| accepted.contains(id));
            projection.nodes.retain(|node| {
                let id = node
                    .get("msgId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                id.is_empty() || id == root || accepted.contains(id)
            });
            seen.extend(accepted);
        }
    }
    true
}

/// emit 显式快照/增量语义；`cursor:null` 表示当前 Go 接口未提供可恢复 opaque cursor。
///
/// 静态 shape（序列化失败=编程错·边界输入零信任已在 `extract_flat_replies` 完成）。
pub fn emit_channel_replies(req_id: &str, projection: FlatReplyProjection) -> Effect {
    let payload = serde_json::json!({
        "event": "im:channel:replies",
        "data": {
            "reqId": req_id,
            "channelId": projection.channel_id,
            "rootMessageId": projection.root_message_id,
            "projectionMode": projection.projection_mode.as_str(),
            "revision": projection.revision,
            "replyIds": projection.reply_ids,
            "nodes": projection.nodes,
            "replyCount": projection.reply_count,
            "cursor": serde_json::Value::Null,
            "hasMore": projection.has_more,
        },
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_replies: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}
