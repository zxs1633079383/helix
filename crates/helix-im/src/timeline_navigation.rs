//! Timeline V3 newer/locate 的 sans-IO command 与 correlation 状态。

use bytes::Bytes;
use helix_core::effect::{GetSpec, HttpRequest, SqlValue, StorageOp};
use helix_core::{Correlation, Effect};
use serde_json::{json, Value};

use crate::error::ImError;
use crate::state::ChannelId;
use crate::timeline_state::TimelinePageSize;
use crate::timeline_state::{TimelineEntityKey, WindowPage};

/// 下拉读取更新消息的正式命令。
pub const LOAD_NEWER_CONTEXT: &str = "im_load_newer_context";
/// 未缓存消息定位的正式命令。
pub const LOCATE_MESSAGE: &str = "im_locate_message";
/// 未缓存消息定位的上下文查询别名；与旧定位命令共享同一 sans-IO 链路。
pub const LOCATE_CONTEXT: &str = "im_locate_context";

/// Timeline V3 导航方向，决定 HTTP endpoint 和 Delta 合并语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineNavigationKind {
    Older {
        anchor_post_id: String,
        anchor_create_at: Option<i64>,
        cursor: TimelineEntityKey,
    },
    Newer {
        anchor_post_id: String,
        anchor_create_at: Option<i64>,
        cursor: TimelineEntityKey,
    },
    Locate {
        target_message_id: String,
        navigation_token: String,
    },
}

/// 已通过 Go 与 durable read-back 的本地完整覆盖；消息数组本身不能单独构成命中。
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineNavigationCoverage {
    pub rows: Vec<Value>,
    pub has_older: bool,
    pub has_newer: bool,
    pub target_index: Option<usize>,
}

/// 一次跨 HTTP、PersistOk 与 readback 存活的 owned 导航状态。
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineNavigationState {
    channel_id: ChannelId,
    window_token: String,
    page_size: u32,
    request_id: Option<String>,
    kind: TimelineNavigationKind,
    rows: Vec<Value>,
    page: WindowPage,
    target_index: Option<usize>,
    older_cursor: Option<Value>,
    newer_cursor: Option<Value>,
}

impl TimelineNavigationState {
    /// 创建上拉分页状态；复合游标同样只能来自 attached typed state。
    pub fn older(
        channel_id: ChannelId,
        page: WindowPage,
        page_size: u32,
        request_id: Option<String>,
        anchor_post_id: String,
        cursor: TimelineEntityKey,
    ) -> Self {
        Self {
            channel_id,
            window_token: page.window_token.clone(),
            page,
            page_size,
            request_id,
            kind: TimelineNavigationKind::Older {
                anchor_post_id,
                anchor_create_at: None,
                cursor,
            },
            rows: Vec::new(),
            target_index: None,
            older_cursor: None,
            newer_cursor: None,
        }
    }

    /// 创建不依赖视觉窗口的 older 查询；anchor 时间只作为 Go 请求输入，排序事实仍由 Go 解析。
    pub fn older_without_window(
        channel_id: ChannelId,
        page_size: u32,
        request_id: String,
        anchor_post_id: String,
        anchor_create_at: Option<i64>,
    ) -> Self {
        let correlation_token = format!("timeline:{}", request_id);
        Self {
            channel_id,
            window_token: correlation_token.clone(),
            page: WindowPage {
                window_token: correlation_token,
                ..WindowPage::default()
            },
            page_size,
            request_id: Some(request_id),
            kind: TimelineNavigationKind::Older {
                anchor_post_id,
                anchor_create_at,
                cursor: TimelineEntityKey {
                    create_at: 0,
                    temporary_id: String::new(),
                },
            },
            rows: Vec::new(),
            target_index: None,
            older_cursor: None,
            newer_cursor: None,
        }
    }

    /// 创建下拉分页状态；复合游标只能来自 Helix attached typed state。
    pub fn newer(
        channel_id: ChannelId,
        page: WindowPage,
        page_size: u32,
        request_id: Option<String>,
        anchor_post_id: String,
        cursor: TimelineEntityKey,
    ) -> Self {
        Self {
            channel_id,
            window_token: page.window_token.clone(),
            page,
            page_size,
            request_id,
            kind: TimelineNavigationKind::Newer {
                anchor_post_id,
                anchor_create_at: None,
                cursor,
            },
            rows: Vec::new(),
            target_index: None,
            older_cursor: None,
            newer_cursor: None,
        }
    }

    /// 创建不依赖视觉窗口的 newer 查询；Helix 不保存或读取 TimelineWindow。
    pub fn newer_without_window(
        channel_id: ChannelId,
        page_size: u32,
        request_id: String,
        anchor_post_id: String,
        anchor_create_at: Option<i64>,
    ) -> Self {
        let correlation_token = format!("timeline:{}", request_id);
        Self {
            channel_id,
            window_token: correlation_token.clone(),
            page: WindowPage {
                window_token: correlation_token,
                ..WindowPage::default()
            },
            page_size,
            request_id: Some(request_id),
            kind: TimelineNavigationKind::Newer {
                anchor_post_id,
                anchor_create_at,
                cursor: TimelineEntityKey {
                    create_at: 0,
                    temporary_id: String::new(),
                },
            },
            rows: Vec::new(),
            target_index: None,
            older_cursor: None,
            newer_cursor: None,
        }
    }

    /// 创建定位状态；navigation token 仅由命令关联生成，不接受 Range 或排序输入。
    pub fn locate(
        channel_id: ChannelId,
        window_token: String,
        page_size: u32,
        request_id: Option<String>,
        target_message_id: String,
        navigation_token: String,
    ) -> Self {
        Self {
            channel_id,
            page: WindowPage {
                window_token: window_token.clone(),
                has_older: true,
                has_newer: true,
                has_more: true,
            },
            window_token,
            page_size,
            request_id,
            kind: TimelineNavigationKind::Locate {
                target_message_id,
                navigation_token,
            },
            rows: Vec::new(),
            target_index: None,
            older_cursor: None,
            newer_cursor: None,
        }
    }

    /// 返回导航所属频道。
    pub const fn channel_id(&self) -> ChannelId {
        self.channel_id
    }

    /// 返回已验证页长。
    pub const fn page_size(&self) -> u32 {
        self.page_size
    }

    /// 返回一次查询的瞬时 correlation token；它不是视觉窗口或 attach 前置。
    pub fn window_token(&self) -> &str {
        self.window_token.as_str()
    }

    /// 判断是否只使用瞬时 correlation token，避免按视觉窗口游标过滤 Go 权威页。
    pub fn is_windowless(&self) -> bool {
        self.window_token.starts_with("timeline:")
    }

    /// 返回 transport correlation，不把它混入业务 cursor。
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    /// 返回稳定的业务操作名，供终态合同回显。
    pub const fn operation(&self) -> &'static str {
        match self.kind {
            TimelineNavigationKind::Older { .. } => "older",
            TimelineNavigationKind::Newer { .. } => "newer",
            TimelineNavigationKind::Locate { .. } => "locate",
        }
    }

    /// 返回与视觉生命周期无关的本地 coverage key。
    pub fn coverage_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.channel_id.as_str(),
            match &self.kind {
                TimelineNavigationKind::Older { .. } => "older",
                TimelineNavigationKind::Newer { .. } => "newer",
                TimelineNavigationKind::Locate { .. } => "locate",
            },
            self.anchor_message_id(),
            self.page_size
        )
    }

    /// 返回导航类别。
    pub fn kind(&self) -> &TimelineNavigationKind {
        &self.kind
    }

    /// Returns the existing attached message that owns this navigation transaction.
    pub fn anchor_message_id(&self) -> &str {
        match &self.kind {
            TimelineNavigationKind::Older { anchor_post_id, .. }
            | TimelineNavigationKind::Newer { anchor_post_id, .. } => anchor_post_id,
            TimelineNavigationKind::Locate {
                target_message_id, ..
            } => target_message_id,
        }
    }

    /// 返回成功 HTTP 页建立的边界。
    pub fn page(&self) -> WindowPage {
        self.page.clone()
    }

    /// 返回通过权限和 wire 校验的原始消息页。
    pub fn rows(&self) -> &[Value] {
        &self.rows
    }

    /// 用权限过滤且补齐 wire 字段的同源行替换原始 HTTP 页。
    pub fn replace_rows(&mut self, rows: Vec<Value>) {
        self.rows = rows;
    }

    /// 返回 Go authority 校验过的 locate targetIndex。
    pub const fn target_index(&self) -> Option<usize> {
        self.target_index
    }

    /// 返回 authority 提供的向前游标；缺省时以 null 结算，不在 Helix 侧合成。
    pub fn older_cursor(&self) -> Option<&Value> {
        self.older_cursor.as_ref()
    }

    /// 返回 authority 提供的向后游标；缺省时以 null 结算，不在 Helix 侧合成。
    pub fn newer_cursor(&self) -> Option<&Value> {
        self.newer_cursor.as_ref()
    }

    /// 返回用于持久化后单行 readback 的权威 server id。
    pub fn readback_message_id(&self) -> Option<&str> {
        let target = match &self.kind {
            TimelineNavigationKind::Locate {
                target_message_id, ..
            } => self
                .rows
                .iter()
                .find(|row| row.get("id").and_then(Value::as_str) == Some(target_message_id)),
            TimelineNavigationKind::Older { .. } => self.rows.first(),
            TimelineNavigationKind::Newer { .. } => self.rows.last(),
        }?;
        target
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
    }

    /// 吃入 V3 CommonRes，并冻结本次页的 rows 与双向边界。
    pub fn ingest_http_body(&mut self, raw_body: &[u8]) -> Result<(), ImError> {
        let root: Value = serde_json::from_slice(raw_body)
            .map_err(|error| ImError::Parse(format!("timeline navigation response: {error}")))?;
        if root.get("status").and_then(Value::as_str) != Some("SUCCESS") {
            return Err(ImError::Parse(
                "timeline navigation response status must be SUCCESS".to_string(),
            ));
        }
        let data = root.get("data").and_then(Value::as_object).ok_or_else(|| {
            ImError::Parse("timeline navigation response missing data".to_string())
        })?;
        // Go CommonRes uses `count:0, posts:null` for an empty navigation page;
        // accept that explicit zero-card shape without weakening non-empty validation.
        let (rows, empty_authority_page) = match data.get("posts") {
            Some(Value::Array(rows)) => {
                let rows = rows.clone();
                (
                    rows.clone(),
                    data.get("count").and_then(Value::as_u64) == Some(0) && rows.is_empty(),
                )
            }
            Some(Value::Null) if data.get("count").and_then(Value::as_u64) == Some(0) => {
                (Vec::new(), true)
            }
            _ => {
                return Err(ImError::Parse(
                    "timeline navigation response missing posts".to_string(),
                ))
            }
        };
        validate_rows(&rows, self.channel_id.as_str())?;
        match &self.kind {
            TimelineNavigationKind::Older { .. } => {
                let has_more = data
                    .get("hasMore")
                    .and_then(Value::as_bool)
                    .or_else(|| empty_authority_page.then_some(false))
                    .ok_or_else(|| {
                        ImError::Parse(
                            "timeline older response missing boolean hasMore".to_string(),
                        )
                    })?;
                self.page.has_older = has_more;
                self.page.has_more = has_more || self.page.has_newer;
            }
            TimelineNavigationKind::Newer { .. } => {
                let has_more = data
                    .get("hasMore")
                    .and_then(Value::as_bool)
                    .or_else(|| empty_authority_page.then_some(false))
                    .ok_or_else(|| {
                        ImError::Parse(
                            "timeline newer response missing boolean hasMore".to_string(),
                        )
                    })?;
                self.page.has_newer = has_more;
                self.page.has_more = self.page.has_older || has_more;
            }
            TimelineNavigationKind::Locate {
                target_message_id, ..
            } => {
                let target_index = data
                    .get("targetIndex")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| {
                        ImError::Parse("timeline locate response missing targetIndex".to_string())
                    })?;
                if rows
                    .get(target_index)
                    .and_then(|row| row.get("id"))
                    .and_then(Value::as_str)
                    != Some(target_message_id.as_str())
                {
                    return Err(ImError::Parse(
                        "timeline locate targetIndex does not identify target".to_string(),
                    ));
                }
                self.target_index = Some(target_index);
                self.page.has_older =
                    data.get("hasOlder")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| {
                            ImError::Parse(
                                "timeline locate response missing boolean hasOlder".to_string(),
                            )
                        })?;
                self.page.has_newer =
                    data.get("hasNewer")
                        .and_then(Value::as_bool)
                        .ok_or_else(|| {
                            ImError::Parse(
                                "timeline locate response missing boolean hasNewer".to_string(),
                            )
                        })?;
                self.page.has_more = self.page.has_older || self.page.has_newer;
                self.older_cursor = data
                    .get("olderCursor")
                    .or_else(|| data.get("older_cursor"))
                    .cloned()
                    .filter(|value| !value.is_null());
                self.newer_cursor = data
                    .get("newerCursor")
                    .or_else(|| data.get("newer_cursor"))
                    .cloned()
                    .filter(|value| !value.is_null());
            }
        }
        self.rows = rows;
        Ok(())
    }
}

/// 解析下拉请求，并拒绝 renderer 注入 cursor/order/range。
pub fn parse_newer_request(
    payload: &[u8],
) -> Result<(ChannelId, String, u32, Option<String>), ImError> {
    let parsed = parse_request(payload, LOAD_NEWER_CONTEXT, "anchor_post_id")?;
    if parsed.3.is_none() {
        return Err(ImError::Parse(format!(
            "{LOAD_NEWER_CONTEXT} req_id is required"
        )));
    }
    Ok(parsed)
}

/// 解析 newer 的 V3 anchor；旧 tuple API 仍由测试和兼容 caller 使用。
pub fn parse_newer_request_with_anchor(
    payload: &[u8],
) -> Result<(ChannelId, String, Option<i64>, u32, String), ImError> {
    parse_v3_request(payload, LOAD_NEWER_CONTEXT, "anchor_post_id")
}

/// 解析上拉请求，并拒绝 renderer 注入复合游标。
pub fn parse_older_request(
    payload: &[u8],
) -> Result<(ChannelId, String, u32, Option<String>), ImError> {
    let parsed = parse_request(
        payload,
        crate::older_context::LOAD_OLDER_CONTEXT,
        "anchor_post_id",
    )?;
    if parsed.3.is_none() {
        return Err(ImError::Parse(format!(
            "{} req_id is required",
            crate::older_context::LOAD_OLDER_CONTEXT
        )));
    }
    Ok(parsed)
}

/// 解析 older 的 V3 anchor；Go 负责 postId→temporary_id 与排序事实。
pub fn parse_older_request_with_anchor(
    payload: &[u8],
) -> Result<(ChannelId, String, Option<i64>, u32, String), ImError> {
    parse_v3_request(
        payload,
        crate::older_context::LOAD_OLDER_CONTEXT,
        "anchor_post_id",
    )
}

/// 解析定位请求，并拒绝 renderer 注入 target 之外的实体键。
pub fn parse_locate_request(
    payload: &[u8],
) -> Result<(ChannelId, String, u32, Option<String>, Option<String>), ImError> {
    parse_locate_request_for_command(payload, LOCATE_MESSAGE)
}

/// 解析定位别名请求，并把命令名带入错误，避免两条入口产生不同校验。
pub fn parse_locate_request_for_command(
    payload: &[u8],
    command: &str,
) -> Result<(ChannelId, String, u32, Option<String>, Option<String>), ImError> {
    let (channel_id, message_id, page_size, request_id) =
        parse_request(payload, command, "message_id")?;
    if request_id.is_none() {
        return Err(ImError::Parse(format!("{command} req_id is required")));
    }
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("{command} payload: {error}")))?;
    let navigation_token = match value.get("navigationToken") {
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        Some(_) => {
            return Err(ImError::Parse(format!(
                "{command} navigationToken must be non-empty string"
            )))
        }
        None => None,
    };
    Ok((
        channel_id,
        message_id,
        page_size,
        request_id,
        navigation_token,
    ))
}

/// 构造 V3 newer/locate HTTP Effect，路由和 body 与 Go opt-in handler 一致。
pub fn navigation_http(
    state: &TimelineNavigationState,
    base_url: &str,
    connection_id: Option<&str>,
    corr: Correlation,
) -> Effect {
    let (path, body) = match state.kind() {
        TimelineNavigationKind::Older {
            anchor_post_id,
            anchor_create_at,
            cursor,
        } => (
            "posts/getPostsAfterIndex",
            json!({
                "postIds": anchor_post_id,
                "cursorVersion": 1,
                "pageSize": state.page_size(),
                "direction": "older",
                "anchor": {
                    "postId": anchor_post_id,
                    "createAt": anchor_create_at,
                },
                "cursor": if anchor_create_at.is_some() { Value::Null } else { json!(cursor) },
            }),
        ),
        TimelineNavigationKind::Newer {
            anchor_post_id,
            anchor_create_at,
            cursor,
        } => (
            "posts/getPostsAfterIndex",
            json!({
                "postIds": anchor_post_id,
                "cursorVersion": 1,
                "pageSize": state.page_size(),
                "direction": "newer",
                "anchor": {
                    "postId": anchor_post_id,
                    "createAt": anchor_create_at,
                },
                "cursor": if anchor_create_at.is_some() { Value::Null } else { json!(cursor) },
            }),
        ),
        TimelineNavigationKind::Locate {
            target_message_id, ..
        } => (
            "posts/postContext",
            json!({
                "postId": target_message_id,
                "cursorVersion": 1,
                "pageSize": state.page_size(),
            }),
        ),
    };
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::sync_http_effects::session_auth_headers(
        connection_id,
    ));
    if let Some(request_id) = state.request_id() {
        headers.push(("Cses-Track-Id".to_string(), request_id.to_string()));
    }
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{base_url}/{path}"),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).unwrap_or_default())),
        },
    }
}

/// 构造导航持久化后的单行读回；定位入口本身直接使用远端 context authority。
pub fn locate_probe_effect(target_message_id: &str, corr: Correlation) -> Effect {
    Effect::Persist {
        corr,
        ops: vec![StorageOp::Get(GetSpec {
            table: "message",
            key_col: "id",
            key_val: SqlValue::Text(target_message_id.to_string()),
        })],
    }
}

/// 构造定位专用的单行读回；分页 newer/older 必须由 dispatch 生成 bounded Scan。
pub fn navigation_readback_effect(
    state: &TimelineNavigationState,
    corr: Correlation,
) -> Option<Effect> {
    if !matches!(state.kind(), TimelineNavigationKind::Locate { .. }) {
        return None;
    }
    state
        .readback_message_id()
        .map(|message_id| locate_probe_effect(message_id, corr))
}

/// 解析共享请求字段并在任何 I/O 前验证 pageSize。
fn parse_request(
    payload: &[u8],
    command: &str,
    message_field: &str,
) -> Result<(ChannelId, String, u32, Option<String>), ImError> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("{command} payload: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{command} payload must be object")))?;
    if let Some(unknown) = object.keys().find(|key| {
        !matches!(
            key.as_str(),
            "channel_id"
                | "channelId"
                | "anchor_post_id"
                | "message_id"
                | "pageSize"
                | "page_size"
                | "req_id"
                | "reqId"
                | "operation"
                | "targetPostId"
                | "target_post_id"
                | "anchor"
                | "navigationToken"
        )
    }) {
        return Err(ImError::Parse(format!(
            "{command} unknown field: {unknown}"
        )));
    }
    let channel = object
        .get("channel_id")
        .or_else(|| object.get("channelId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{command} missing channel_id")))?;
    let channel_id = ChannelId::from_str(channel)
        .ok_or_else(|| ImError::Parse(format!("invalid channel_id: {channel}")))?;
    let anchor_post_id = object
        .get("anchor")
        .and_then(Value::as_object)
        .and_then(|anchor| anchor.get("postId").or_else(|| anchor.get("post_id")))
        .and_then(Value::as_str);
    let message_id = object
        .get(message_field)
        .or_else(|| object.get("targetPostId"))
        .or_else(|| object.get("target_post_id"))
        .and_then(Value::as_str)
        .or(anchor_post_id)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{command} missing {message_field}")))?
        .to_string();
    let page_size =
        TimelinePageSize::parse(object.get("pageSize").or_else(|| object.get("page_size")))
            .map_err(|error| ImError::Parse(format!("{command} pageSize: {error}")))?
            .get();
    let request_id = match object.get("req_id").or_else(|| object.get("reqId")) {
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        Some(_) => {
            return Err(ImError::Parse(format!(
                "{command} req_id must be non-empty string"
            )))
        }
        None => None,
    };
    Ok((channel_id, message_id, page_size, request_id))
}

/// 解析无 window 的 Timeline 请求，要求 reqId 并提取 `{createAt,postId}` anchor。
fn parse_v3_request(
    payload: &[u8],
    command: &str,
    message_field: &str,
) -> Result<(ChannelId, String, Option<i64>, u32, String), ImError> {
    let parsed = parse_request(payload, command, message_field)?;
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("{command} payload: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{command} payload must be object")))?;
    let anchor = object.get("anchor").and_then(Value::as_object);
    let anchor_create_at = anchor
        .and_then(|value| value.get("createAt").or_else(|| value.get("create_at")))
        .and_then(Value::as_i64);
    let request_id = parsed
        .3
        .ok_or_else(|| ImError::Parse(format!("{command} req_id is required")))?;
    Ok((parsed.0, parsed.1, anchor_create_at, parsed.2, request_id))
}

/// 校验远端页严格按 `(createAt, temporaryId)` 升序且属于请求频道。
fn validate_rows(rows: &[Value], channel_id: &str) -> Result<(), ImError> {
    let mut previous: Option<(i64, &str)> = None;
    for row in rows {
        let row_channel = row
            .get("channelId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ImError::Parse("timeline row missing channelId".to_string()))?;
        let create_at = row
            .get("createAt")
            .and_then(Value::as_i64)
            .filter(|value| *value > 0)
            .ok_or_else(|| ImError::Parse("timeline row missing createAt".to_string()))?;
        let temporary_id = row
            .get("temporaryId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ImError::Parse("timeline row missing temporaryId".to_string()))?;
        if row_channel != channel_id {
            return Err(ImError::Parse("timeline row channel mismatch".to_string()));
        }
        if previous.is_some_and(|key| key >= (create_at, temporary_id)) {
            return Err(ImError::Parse(
                "timeline rows must be strictly ordered by composite cursor".to_string(),
            ));
        }
        previous = Some((create_at, temporary_id));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::TimelineNavigationState;

    /// Go 空页的显式 `count:0, posts:null` 必须结算为合法 older 终态。
    #[test]
    fn empty_authority_page_accepts_null_posts() {
        let mut state = TimelineNavigationState::older_without_window(
            crate::state::test_channel_id(11),
            20,
            "req-empty-older".to_string(),
            "srvfix00000000000000000001".to_string(),
            None,
        );

        state
            .ingest_http_body(br#"{"status":"SUCCESS","data":{"count":0,"posts":null}}"#)
            .expect("explicit zero-card page is a successful empty result");

        assert!(state.rows().is_empty());
        assert!(!state.page().has_older);
        assert!(!state.page().has_more);
    }

    /// 非零 count 与 null posts 仍然失败，避免把畸形后端响应伪装为空页。
    #[test]
    fn non_empty_count_rejects_null_posts() {
        let mut state = TimelineNavigationState::older_without_window(
            crate::state::test_channel_id(12),
            20,
            "req-invalid-older".to_string(),
            "srvfix00000000000000000001".to_string(),
            None,
        );

        assert!(state
            .ingest_http_body(br#"{"status":"SUCCESS","data":{"count":1,"posts":null}}"#)
            .is_err());
    }
}
