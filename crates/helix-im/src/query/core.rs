//! P6 投影/状态查询命令（M2 Tauri 接缝④的 helix-im 侧）。
//!
//! 接管架构表 A：现网 `imDataSource:*` 拦截垫片 + `store_get_messages` 直查语义下沉 Rust。
//! 前端退化为纯渲染（C1/C2）：不再前端 SQL 直查，改 invoke 语义命令 → core 出 `Effect` →
//! driver 兑现 → emit 投影。本文件收口三端共享的请求解析、ScanSpec 与投影 shape；
//! local-first/remote fallback 编排见 `local_first`。
//!
//! ## 命令集（对齐现网 boot.rs 真实注册的投影面）
//!
//! - `im_query_messages_by_channel`：现网 `SELECT * FROM message WHERE channel_id=? ORDER BY
//!   create_at DESC, temporary_id DESC LIMIT ?`（commands.rs，debug-only 原始行 dump）。helix 走
//!   `Scan(message, filter=channel_id, order_desc=create_at, limit)` → PortReply 回报 → 反转最近 N 条
//!   为时间正序 → emit `im:messages:query_result` 投影（`data: { channel_id, messages }`）。
//!   - scroll 游标分页（beforeId/afterId）是 `im_load_older_messages`（HTTP postContext）另一路，
//!     不在本投影命令。
//! - `im_delete_all_dialogs`：现网 `DELETE FROM channel` + `DELETE FROM channel_member`（测试用）。
//!   - **核原语缺口（已记 P7 blocker）**：core 无无条件 truncate 原语（`BatchDelete` 需 scope+IN
//!     列表，表达不了「删全表」）。本命令清 **core 内存 channel gate 状态** + emit `im:channels:loaded`
//!     刷新；物理 `DELETE FROM` 留待 truncate 原语补齐（不为测试命令现造原语，YAGNI/HX-C001）。
//!
//! `im_get_state` **不实现**：现网 boot.rs 无此命令（rv-tauri B1「漏项」claim 与 ground truth 不符，
//! 已于 P7 finalize 勘误）——凭空补会引入前端不调用的死命令。

use bytes::Bytes;
use helix_core::effect::{
    Correlation, DomainEventBytes, Effect, ScanOrder, ScanSpec, SqlValue, StorageOp,
};

use crate::error::ImError;
use crate::state::ChannelId;

/// 投影/状态查询命令名集合（dispatch 据此路由，与 outbound registry 解耦）。
pub const QUERY_COMMAND_NAMES: &[&str] = &[
    "im_query_messages_by_channel",
    "im_delete_all_dialogs",
    "im_query_dialog_list",
    "im_query_channel_sync_page",
    "im_complete_channel_sync",
    "im_query_subtopics",
    crate::older_context::LOAD_OLDER_CONTEXT,
    crate::timeline_navigation::LOAD_NEWER_CONTEXT,
    crate::timeline_navigation::LOCATE_MESSAGE,
    crate::timeline_navigation::LOCATE_CONTEXT,
];

pub fn query_command_names() -> &'static [&'static str] {
    QUERY_COMMAND_NAMES
}

pub fn is_query(name: &str) -> bool {
    QUERY_COMMAND_NAMES.contains(&name)
}

/// Timeline V3 缺省页长；所有 latest/older/newer/locate 入口共享同一策略。
pub(crate) const QUERY_MESSAGES_DEFAULT: u32 = crate::timeline_state::DEFAULT_TIMELINE_PAGE_SIZE;
/// Timeline V3 单页硬上限；非法值在产生 Effect 前拒绝而不是 clamp。

/// dialogList 投影单次上限（会话列表全量，宽于消息分页；对齐现网根群可达数千会话上限护栏）。
pub(crate) const QUERY_DIALOGS_DEFAULT: u32 = 500;
pub(crate) const QUERY_DIALOGS_MAX: u32 = 2000;

/// 三端共享的最近消息业务查询；平台 driver 不解释表名、过滤列或响应 shape。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageQueryRequest {
    pub channel_id: ChannelId,
    pub limit: u32,
    /// Timeline 查询窗口 token；缺省为 `latest`，避免把多个窗口混到 channel 级 scope。
    pub window_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogListQueryRequest {
    pub limit: u32,
}

/// 当前账号下按父频道读取 topic 的最小意图；身份只能来自 RuntimeAuth。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtopicsQueryRequest {
    pub parent_channel_id: Option<ChannelId>,
}

/// 当前 RuntimeAuth 的会话列表作用域；caller payload 不得覆盖这两个字段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogListScope {
    pub viewer_user_id: String,
    pub company_id: String,
}

impl DialogListScope {
    /// 从 Host 注入的身份构造不可变查询作用域。
    pub fn new(viewer_user_id: &str, company_id: &str) -> Self {
        Self {
            viewer_user_id: viewer_user_id.to_string(),
            company_id: company_id.to_string(),
        }
    }
}

/// 解析 latest Timeline 请求，并在任何 I/O Effect 产生前验证 pageSize。
pub fn parse_message_query(payload: &[u8]) -> Result<MessageQueryRequest, ImError> {
    let value: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_query_messages_by_channel payload: {e}")))?;
    let channel = value
        .get("channel_id")
        .or_else(|| value.get("channelId"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse("im_query_messages_by_channel 缺 channel_id".into()))?;
    let channel_id = ChannelId::from_str(channel)
        .ok_or_else(|| ImError::Parse(format!("非法 channel_id: {channel}")))?;
    let limit = crate::timeline_state::TimelinePageSize::parse(
        value.get("pageSize").or_else(|| value.get("limit")),
    )
    .map_err(|error| ImError::Parse(format!("im_query_messages_by_channel pageSize: {error}")))?
    .get();
    let window_token = match value
        .get("windowToken")
        .or_else(|| value.get("window_token"))
    {
        Some(serde_json::Value::String(value)) => value.as_str(),
        Some(_) => {
            return Err(ImError::Parse(
                "im_query_messages_by_channel windowToken must be string".into(),
            ));
        }
        None => "latest",
    };
    if window_token.is_empty() {
        return Err(ImError::Parse(
            "im_query_messages_by_channel windowToken must not be empty".into(),
        ));
    }
    Ok(MessageQueryRequest {
        channel_id,
        limit,
        window_token: window_token.to_string(),
    })
}

/// 为可视窗口额外读取一条 lookahead，仅用于证明本地仍有更早消息，不扩大投影页长。
pub fn message_scan_spec(request: &MessageQueryRequest) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(request.limit.saturating_add(1)),
        filter: Some((
            "channel_id",
            SqlValue::Text(request.channel_id.as_str().to_string()),
        )),
        order_by: MESSAGE_QUERY_ORDER,
    }
}

/// 解析会话列表最小意图，并拒绝由 caller 伪造的 account/team/user scope。
pub fn parse_dialog_list_query(payload: &[u8]) -> Result<DialogListQueryRequest, ImError> {
    let value = serde_json::from_slice::<serde_json::Value>(payload)
        .map_err(|error| ImError::Parse(format!("im_query_dialog_list payload: {error}")))?;
    let object = value.as_object().ok_or_else(|| {
        ImError::Parse("im_query_dialog_list payload must be an object".to_string())
    })?;
    for key in object.keys() {
        if !matches!(key.as_str(), "limit" | "req_id") {
            return Err(ImError::Parse(format!(
                "im_query_dialog_list field is not caller-owned: {key}"
            )));
        }
    }
    let limit = match object.get("limit") {
        None => QUERY_DIALOGS_DEFAULT,
        Some(serde_json::Value::Number(value)) => value
            .as_u64()
            .map(|value| value.clamp(1, u64::from(QUERY_DIALOGS_MAX)) as u32)
            .ok_or_else(|| ImError::Parse("im_query_dialog_list limit must be integer".into()))?,
        Some(_) => {
            return Err(ImError::Parse(
                "im_query_dialog_list limit must be integer".into(),
            ));
        }
    };
    if let Some(req_id) = object.get("req_id") {
        if !req_id.is_null() && !req_id.is_string() {
            return Err(ImError::Parse(
                "im_query_dialog_list req_id must be string".into(),
            ));
        }
    }
    Ok(DialogListQueryRequest { limit })
}

/// 解析 `querySubtopics` 意图；空 parent 作为本地 typed-empty，不触发 I/O。
pub fn parse_subtopics_query(payload: &[u8]) -> Result<SubtopicsQueryRequest, ImError> {
    let value = serde_json::from_slice::<serde_json::Value>(payload)
        .map_err(|error| ImError::Parse(format!("im_query_subtopics payload: {error}")))?;
    let object = value.as_object().ok_or_else(|| {
        ImError::Parse("im_query_subtopics payload must be an object".to_string())
    })?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "parentChannelId" | "parent_channel_id" | "req_id"
        ) {
            return Err(ImError::Parse(format!(
                "im_query_subtopics field is not caller-owned: {key}"
            )));
        }
    }
    let parent = object
        .get("parentChannelId")
        .or_else(|| object.get("parent_channel_id"));
    let parent_channel_id = match parent {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(value)) if value.is_empty() => None,
        Some(serde_json::Value::String(value)) => {
            ChannelId::from_str(value).map(Some).ok_or_else(|| {
                ImError::Parse("im_query_subtopics parentChannelId is invalid".into())
            })?
        }
        Some(_) => {
            return Err(ImError::Parse(
                "im_query_subtopics parentChannelId must be string".into(),
            ));
        }
    };
    if let Some(req_id) = object.get("req_id") {
        if !req_id.is_null() && !req_id.is_string() {
            return Err(ImError::Parse(
                "im_query_subtopics req_id must be string".into(),
            ));
        }
    }
    Ok(SubtopicsQueryRequest { parent_channel_id })
}

pub fn dialog_list_scan_spec(request: DialogListQueryRequest) -> ScanSpec {
    ScanSpec {
        table: "channel",
        limit: Some(request.limit),
        filter: None,
        order_by: DIALOG_QUERY_ORDER,
    }
}

/// 构造带 RuntimeAuth 租户过滤的本地 channel Scan。
pub fn dialog_list_scan_spec_for_scope(
    request: DialogListQueryRequest,
    scope: &DialogListScope,
) -> Result<ScanSpec, ImError> {
    if scope.company_id.is_empty() || scope.viewer_user_id.is_empty() {
        return Err(ImError::Parse(
            "im_query_dialog_list requires RuntimeAuth user and company".into(),
        ));
    }
    Ok(ScanSpec {
        table: "channel",
        limit: Some(request.limit),
        filter: Some(("team_id", SqlValue::Text(scope.company_id.clone()))),
        order_by: DIALOG_QUERY_ORDER,
    })
}

/// 构造当前 RuntimeAuth 租户的本地 topic Scan；父频道可见性在同一回包中验证。
pub fn subtopics_scan_spec_for_scope(scope: &DialogListScope) -> Result<ScanSpec, ImError> {
    if scope.company_id.is_empty() || scope.viewer_user_id.is_empty() {
        return Err(ImError::Parse(
            "im_query_subtopics requires RuntimeAuth user and company".into(),
        ));
    }
    Ok(ScanSpec {
        table: "channel",
        limit: Some(QUERY_DIALOGS_MAX),
        filter: Some(("team_id", SqlValue::Text(scope.company_id.clone()))),
        order_by: DIALOG_QUERY_ORDER,
    })
}

const MESSAGE_QUERY_ORDER: &[ScanOrder] = &[
    ScanOrder::desc("create_at"),
    ScanOrder::desc("temporary_id"),
];
pub(crate) const DIALOG_QUERY_ORDER: &[ScanOrder] = &[
    ScanOrder::desc("is_top"),
    ScanOrder::desc("last_post_at"),
    ScanOrder::desc("created_at"),
];

/// 构造 `im_query_messages_by_channel` 的 `Scan(message WHERE channel_id ORDER BY create_at DESC)`。
///
/// 返回 `(channel_id, Effect::Persist{Scan})`；异步业务入口会注册 local-first correlation，
/// 根据显式 coverage 决定本地投影或 `getLatestPost` fallback。页长缺省 20，合法范围 1..=60。
pub fn build_message_query(
    payload: &[u8],
    corr: Correlation,
) -> Result<(ChannelId, Effect), ImError> {
    let request = parse_message_query(payload)?;
    let effect = build_message_query_from_request(&request, corr);
    Ok((request.channel_id, effect))
}

pub(crate) fn build_message_query_from_request(
    request: &MessageQueryRequest,
    corr: Correlation,
) -> Effect {
    Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(message_scan_spec(request))],
    }
}

/// PortReply 回报（`MessageQuery` 上下文 Ok）→ emit `im:messages:query_result` 投影。
///
/// `reply_bytes` 是 engine 把 Scan rows 序列化的 JSON 数组 `[{col:val}, ...]`（rows↔bytes 约定，
/// driver-host codec）。存储层按 `create_at DESC` 高效截取最近 N 条，投影层反转为时间正序后，
/// 再作为 `messages` 字段装入 `data` 对象。最终 emit 形状固定为
/// `data: { channel_id, messages }`。零信任：非法 JSON → 空数组投影（不 panic）。
pub fn emit_message_query_result(channel_id: &ChannelId, reply_bytes: &[u8]) -> Effect {
    emit_message_query_result_for_viewer(channel_id, reply_bytes, "")
}

pub fn emit_message_query_result_for_viewer(
    channel_id: &ChannelId,
    reply_bytes: &[u8],
    viewer_user_id: &str,
) -> Effect {
    Effect::Emit {
        event: DomainEventBytes(Bytes::from(message_query_result_bytes_for_viewer(
            channel_id,
            reply_bytes,
            viewer_user_id,
        ))),
    }
}

pub(crate) fn message_query_result_bytes_for_viewer(
    channel_id: &ChannelId,
    reply_bytes: &[u8],
    viewer_user_id: &str,
) -> Vec<u8> {
    message_query_result_bytes(channel_id.as_str(), reply_bytes, viewer_user_id)
}

pub(crate) fn message_query_result_bytes(
    channel_id: &str,
    reply_bytes: &[u8],
    viewer_user_id: &str,
) -> Vec<u8> {
    let rows = match serde_json::from_slice::<serde_json::Value>(reply_bytes) {
        Ok(serde_json::Value::Array(mut items)) => {
            items.reverse();
            serde_json::Value::Array(items)
        }
        _ => serde_json::json!([]),
    };
    let payload = serde_json::json!({
        "event": "im:messages:query_result",
        // messages 元素整形成 render-ready 终态行（S6·issue #55·C013）：前端 1:1 绑定，
        // 不再壳内抽 DB snake 列 / 归一 readBits / dedup。外层键集 {channel_id, messages} 不变。
        "data": {
            "channel_id": channel_id,
            "messages": crate::render_ready::shape_message_rows_for_viewer(&rows, viewer_user_id),
        }
    });
    serde_json::to_vec(&payload).expect("message query static JSON shape must serialize")
}

/// 构造兼容旧内部 caller 的无 scope `Scan(channel ORDER BY is_top,last_post_at,created_at DESC LIMIT)`。
///
/// 返回 `Effect::Persist{Scan}`——调用方（module.rs）alloc corr + 注册
/// `CorrelationContext::DialogListQuery`，PortReply 回报由 runtime wrapper 过滤并封装 typed
/// Result。生产入口必须使用 `build_dialog_list_query_for_scope`，避免无租户 Scan。
///
/// **无 channel_id 过滤**（会话列表是全量，非按群）：scan 整张 channel 表。置顶优先；同组按
/// `last_post_at DESC, create_at DESC`，所以新建且尚无消息的会话也能稳定进入 LIMIT 窗口。
///
/// **HX-C005**：低频按需调用（off hot-path），整表 scan 合规——见模块头注与 spec06 §2 边界。
pub fn build_dialog_list_query(payload: &[u8], corr: Correlation) -> Result<Effect, ImError> {
    let request = parse_dialog_list_query(payload)?;

    Ok(Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(dialog_list_scan_spec(request))],
    })
}

/// 构造当前 RuntimeAuth 的 account-local dialog Scan，禁止从 payload 取租户。
pub fn build_dialog_list_query_for_scope(
    payload: &[u8],
    corr: Correlation,
    scope: &DialogListScope,
) -> Result<Effect, ImError> {
    let request = parse_dialog_list_query(payload)?;
    let scan = dialog_list_scan_spec_for_scope(request, scope)?;
    Ok(Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(scan)],
    })
}

/// 构造 `im_query_subtopics` 的单次租户内 channel Scan。
pub fn build_subtopics_query_for_scope(
    request: &SubtopicsQueryRequest,
    corr: Correlation,
    scope: &DialogListScope,
) -> Result<Effect, ImError> {
    if request.parent_channel_id.is_none() {
        return Err(ImError::Parse(
            "im_query_subtopics requires parentChannelId".into(),
        ));
    }
    let scan = subtopics_scan_spec_for_scope(scope)?;
    Ok(Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(scan)],
    })
}

/// 将本地 channel rows 过滤为当前 account 的 typed `items` 结果。
pub fn project_dialog_list_items(
    reply_bytes: &[u8],
    scope: &DialogListScope,
) -> Vec<serde_json::Value> {
    let Ok(serde_json::Value::Array(rows)) = serde_json::from_slice(reply_bytes) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|row| dialog_row_visible(row, scope))
        .filter_map(|row| {
            let id = row
                .get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())?;
            seen.insert(id.to_string())
                .then(|| super::render_ready::channel::shape_channel_row(&row))
        })
        .collect()
}

/// 把单次 local Scan 包成唯一的 `im:read:result` command Result。
pub fn emit_dialog_list_result(
    req_id: &str,
    reply_bytes: &[u8],
    scope: &DialogListScope,
) -> Effect {
    crate::read_relay::emit_read_body(
        req_id,
        serde_json::json!({
            "items": project_dialog_list_items(reply_bytes, scope),
        }),
    )
}

/// 将 local channel rows 投影为指定可见父频道下的 topic typed items。
pub fn project_subtopic_items(
    reply_bytes: &[u8],
    scope: &DialogListScope,
    parent_channel_id: Option<&str>,
) -> Vec<serde_json::Value> {
    let Some(parent_channel_id) = parent_channel_id.filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Array(rows)) = serde_json::from_slice(reply_bytes) else {
        return Vec::new();
    };
    let parent_visible = rows.iter().any(|row| {
        channel_row_id(row) == Some(parent_channel_id) && dialog_row_visible(row, scope)
    });
    if !parent_visible {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    rows.into_iter()
        .filter(|row| dialog_row_visible(row, scope))
        .filter(|row| channel_row_type(row) == Some("T"))
        .filter(|row| channel_row_root_id(row) == Some(parent_channel_id))
        .filter_map(|row| {
            let id = channel_row_id(&row)?;
            seen.insert(id.to_string())
                .then(|| render_subtopic_row(row))
        })
        .collect()
}

/// 把 topic Scan 回报封装为唯一 `im:read:result` command Result。
pub fn emit_subtopics_result(
    req_id: &str,
    reply_bytes: &[u8],
    scope: &DialogListScope,
    parent_channel_id: Option<&str>,
) -> Effect {
    crate::read_relay::emit_read_body(
        req_id,
        serde_json::json!({
            "items": project_subtopic_items(reply_bytes, scope, parent_channel_id),
        }),
    )
}

/// 读取 channel row 的稳定 id，兼容本地 snake_case 与 render-ready camelCase。
fn channel_row_id(row: &serde_json::Value) -> Option<&str> {
    row.get("id")
        .or_else(|| row.get("channel_id"))
        .or_else(|| row.get("channelId"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
}

/// 读取 topic 类型标记，严格要求服务端的 `T` 类型。
fn channel_row_type(row: &serde_json::Value) -> Option<&str> {
    row.get("type")
        .or_else(|| row.get("channel_type"))
        .or_else(|| row.get("channelType"))
        .and_then(serde_json::Value::as_str)
}

/// 读取 topic 的 rootId 关联，兼容 snake_case 与 camelCase。
fn channel_row_root_id(row: &serde_json::Value) -> Option<&str> {
    row.get("root_id")
        .or_else(|| row.get("rootId"))
        .and_then(serde_json::Value::as_str)
}

/// 将 topic 行转换为 render-ready row，并补齐 creator 到 participants 集合。
fn render_subtopic_row(row: serde_json::Value) -> serde_json::Value {
    let owner_id = row
        .get("user_id")
        .or_else(|| row.get("userId"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let mut rendered = render_dialog_row(row);
    let Some(owner_id) = owner_id else {
        return rendered;
    };
    let mut members = rendered
        .get("members")
        .map(parse_json_column)
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    if !members.iter().any(|member| {
        member
            .get("userId")
            .or_else(|| member.get("user_id"))
            .or_else(|| member.get("id"))
            .and_then(serde_json::Value::as_str)
            == Some(owner_id.as_str())
    }) {
        members.push(serde_json::json!({ "userId": owner_id }));
    }
    if let Some(object) = rendered.as_object_mut() {
        object.insert("members".to_string(), serde_json::Value::Array(members));
        let member_count = object
            .get("members")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        object.insert(
            "memberCount".to_string(),
            serde_json::Value::from(member_count as u64),
        );
        if object.contains_key("member_count") {
            object.insert(
                "member_count".to_string(),
                serde_json::Value::from(member_count as u64),
            );
        }
    }
    rendered
}

/// 仅允许租户内、当前 viewer 所属的 channel 行进入结果。
fn dialog_row_visible(row: &serde_json::Value, scope: &DialogListScope) -> bool {
    let Some(object) = row.as_object() else {
        return false;
    };
    let team_id = object
        .get("team_id")
        .or_else(|| object.get("teamId"))
        .and_then(serde_json::Value::as_str);
    if team_id != Some(scope.company_id.as_str()) {
        return false;
    }
    let owner_id = object
        .get("user_id")
        .or_else(|| object.get("userId"))
        .and_then(serde_json::Value::as_str);
    if owner_id == Some(scope.viewer_user_id.as_str()) {
        return true;
    }
    ["members", "target_users", "targetUsers"]
        .iter()
        .filter_map(|key| object.get(*key))
        .map(parse_json_column)
        .any(|members| member_list_contains(&members, scope.viewer_user_id.as_str()))
}

/// 读取 channel JSON text 列；driver 已结构化时保持原值。
fn parse_json_column(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::String(raw) => {
            serde_json::from_str(raw).unwrap_or_else(|_| value.clone())
        }
        _ => value.clone(),
    }
}

/// 检查成员数组中是否包含当前 RuntimeAuth viewer。
fn member_list_contains(value: &serde_json::Value, viewer_user_id: &str) -> bool {
    value.as_array().is_some_and(|members| {
        members.iter().any(|member| {
            member
                .get("userId")
                .or_else(|| member.get("user_id"))
                .or_else(|| member.get("id"))
                .and_then(serde_json::Value::as_str)
                == Some(viewer_user_id)
        })
    })
}

/// 将 channel storage 行补齐最小 camelCase command-result 字段。
fn render_dialog_row(row: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(mut object) = row else {
        return serde_json::Value::Null;
    };
    const ALIASES: &[(&str, &str)] = &[
        ("display_name", "displayName"),
        ("user_id", "userId"),
        ("team_id", "teamId"),
        ("root_id", "rootId"),
        ("member_count", "memberCount"),
        ("unread_count", "unreadCount"),
        ("mention_count", "mentionCount"),
        ("mention_count_root", "mentionCountRoot"),
        ("last_post", "lastPost"),
        ("last_post_at", "lastPostAt"),
        ("created_at", "createdAt"),
        ("is_top", "isTop"),
        ("admin_users", "adminUsers"),
        ("target_users", "targetUsers"),
    ];
    for (source, target) in ALIASES {
        if let Some(value) = object.get(*source).cloned() {
            object.insert((*target).to_string(), parse_json_column(&value));
        }
    }
    for key in [
        "members",
        "admin_users",
        "adminUsers",
        "boss",
        "owner",
        "target_users",
    ] {
        if let Some(value) = object.get(key).cloned() {
            object.insert(key.to_string(), parse_json_column(&value));
        }
    }
    serde_json::Value::Object(object)
}

/// `im_delete_all_dialogs`（测试用）：emit `im:channels:loaded` 刷新信号。
///
/// 见模块头注「核原语缺口」：core 无 truncate 原语，物理 `DELETE FROM channel/channel_member`
/// 留待补齐。本命令保证命令名被识别（前端不报 command not found）+ 触发前端清空重载。
/// 内存 channel gate 清理由调用方（module.rs）做（需 `&mut state`）；本函数只产 emit。
pub fn emit_dialogs_cleared() -> Effect {
    let payload = serde_json::json!({
        "event": "im:channels:loaded",
        "data": { "reason": "delete_all_dialogs" }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_dialogs_cleared: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

#[cfg(test)]
#[path = "core_message_tests.rs"]
mod message_tests;

#[cfg(test)]
mod phase2_contract_tests {
    /// 冻结 Phase 2 时间线页长边界，确保默认 40 与最大 60 在 I/O 前可验证。
    #[test]
    fn timeline_page_size_contract_accepts_twenty_and_sixty() {
        assert_eq!(
            crate::timeline_state::TimelinePageSize::new(20)
                .expect("default page size is valid")
                .get(),
            20
        );
        assert_eq!(
            crate::timeline_state::TimelinePageSize::new(60)
                .expect("maximum page size is valid")
                .get(),
            60
        );
        assert!(crate::timeline_state::TimelinePageSize::new(0).is_err());
        assert!(crate::timeline_state::TimelinePageSize::new(61).is_err());
    }
}
