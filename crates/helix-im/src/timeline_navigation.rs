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

/// Timeline V3 导航方向，决定 HTTP endpoint 和 Delta 合并语义。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelineNavigationKind {
    Older {
        anchor_post_id: String,
        cursor: TimelineEntityKey,
    },
    Newer {
        anchor_post_id: String,
        cursor: TimelineEntityKey,
    },
    Locate {
        target_message_id: String,
        navigation_token: String,
    },
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
                cursor,
            },
            rows: Vec::new(),
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
                cursor,
            },
            rows: Vec::new(),
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

    /// 返回 Helix 已解析的 attached window token。
    pub fn window_token(&self) -> &str {
        self.window_token.as_str()
    }

    /// 返回 transport correlation，不把它混入业务 cursor。
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
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
        let rows = data
            .get("posts")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| {
                ImError::Parse("timeline navigation response missing posts".to_string())
            })?;
        validate_rows(&rows, self.channel_id.as_str())?;
        match &self.kind {
            TimelineNavigationKind::Older { .. } => {
                let has_more = data
                    .get("hasMore")
                    .and_then(Value::as_bool)
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

/// 解析上拉请求，并拒绝 renderer 注入复合游标。
pub fn parse_older_request(
    payload: &[u8],
) -> Result<(ChannelId, String, u32, Option<String>), ImError> {
    parse_request(
        payload,
        crate::older_context::LOAD_OLDER_CONTEXT,
        "anchor_post_id",
    )
}

/// 解析定位请求，并拒绝 renderer 注入 target 之外的实体键。
pub fn parse_locate_request(
    payload: &[u8],
) -> Result<(ChannelId, String, u32, Option<String>, Option<String>), ImError> {
    let (channel_id, message_id, page_size, request_id) =
        parse_request(payload, LOCATE_MESSAGE, "message_id")?;
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("{LOCATE_MESSAGE} payload: {error}")))?;
    let navigation_token = match value.get("navigationToken") {
        Some(Value::String(value)) if !value.is_empty() => Some(value.clone()),
        Some(_) => {
            return Err(ImError::Parse(format!(
                "{LOCATE_MESSAGE} navigationToken must be non-empty string"
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
            cursor,
        } => (
            "posts/getPostsAfterIndex",
            json!({
                "postIds": anchor_post_id,
                "cursorVersion": 1,
                "pageSize": state.page_size(),
                "direction": "older",
                "cursor": cursor,
            }),
        ),
        TimelineNavigationKind::Newer {
            anchor_post_id,
            cursor,
        } => (
            "posts/getPostsAfterIndex",
            json!({
                "postIds": anchor_post_id,
                "cursorVersion": 1,
                "pageSize": state.page_size(),
                "direction": "newer",
                "cursor": cursor,
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
    let message_id = object
        .get(message_field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{command} missing {message_field}")))?
        .to_string();
    let page_size =
        TimelinePageSize::parse(object.get("pageSize").or_else(|| object.get("page_size")))
            .map_err(|error| ImError::Parse(format!("{command} pageSize: {error}")))?
            .get();
    let request_id = match object.get("req_id") {
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
