//! MessageV3 频道同步就绪会话与本地分页协议。
//!
//! 该模块只负责协议解析、cursor 绑定和有界投影；存储 I/O 仍由 `Effect`/`PortReply`
//! 驱动，避免 Host 侧重新解释 account、company 或分页字段。

use helix_core::effect::{
    Correlation, Effect, ScanOrder, ScanSpec, ScopedScanSpec, SqlValue, StorageOp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 跨端固定频道页长；caller 不能通过 payload 覆盖。
pub(crate) const PAGE_SIZE: u32 = 20;
/// opaque cursor 可推进的最大本地窗口；超过后由 caller 重新建立 ready session。
const MAX_OFFSET: usize = 2_000;
const MEMBER_SCAN_LIMIT: usize = 10_000;
const CURSOR_VERSION: u8 = 1;
const CHANNEL_SYNC_ORDER: &[ScanOrder] = &[
    ScanOrder::desc("is_top"),
    ScanOrder::desc("last_post_at"),
    ScanOrder::desc("created_at"),
];

/// ready 后存活的身份绑定会话；任何身份切换都会丢弃该值。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChannelSyncSession {
    pub(crate) channel_sync_session_id: String,
    pub(crate) generation: u64,
    pub(crate) account_id: String,
    pub(crate) company_id: String,
    pub(crate) completed: bool,
}

impl ChannelSyncSession {
    /// 创建一个新的本地频道分页会话。
    pub(crate) fn new(
        channel_sync_session_id: String,
        generation: u64,
        account_id: &str,
        company_id: &str,
    ) -> Self {
        Self {
            channel_sync_session_id,
            generation,
            account_id: account_id.to_string(),
            company_id: company_id.to_string(),
            completed: false,
        }
    }

    /// 校验请求是否仍属于当前 RuntimeAuth 和本次 generation。
    pub(crate) fn matches_scope(
        &self,
        session_id: &str,
        account_id: &str,
        company_id: &str,
    ) -> bool {
        !self.completed
            && self.channel_sync_session_id == session_id
            && self.account_id == account_id
            && self.company_id == company_id
    }
}

/// `im_query_channel_sync_page` 的严格请求形状。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageRequest {
    pub(crate) channel_sync_session_id: String,
    pub(crate) next_cursor: Option<String>,
    /// Host transport 注入的 request-response 关联，不属于业务分页意图。
    pub(crate) req_id: Option<String>,
}

/// `im_complete_channel_sync` 的严格请求形状。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompleteRequest {
    pub(crate) channel_sync_session_id: String,
    /// Host transport 注入的 request-response 关联。
    pub(crate) req_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct CursorBody {
    version: u8,
    session_id: String,
    generation: u64,
    offset: usize,
}

/// 解析 ready 后的分页请求；只允许 session/cursor/pageSize 三个字段。
pub(crate) fn parse_page_request(payload: &[u8]) -> Result<PageRequest, crate::ImError> {
    let value = serde_json::from_slice::<Value>(payload).map_err(|error| {
        crate::ImError::Parse(format!("im_query_channel_sync_page payload: {error}"))
    })?;
    let object = value.as_object().ok_or_else(|| {
        crate::ImError::Parse("im_query_channel_sync_page payload must be an object".to_string())
    })?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "channel_sync_session_id" | "next_cursor" | "page_size" | "req_id"
        ) {
            return Err(crate::ImError::Parse(format!(
                "im_query_channel_sync_page field is not caller-owned: {key}"
            )));
        }
    }
    let session_id = required_string(object, "channel_sync_session_id", "channel_sync_session_id")?;
    let page_size = match object.get("page_size") {
        None => PAGE_SIZE,
        Some(Value::Number(number)) if number.as_u64() == Some(PAGE_SIZE as u64) => PAGE_SIZE,
        Some(Value::Number(_)) => {
            return Err(crate::ImError::Parse(format!(
                "im_query_channel_sync_page page_size must be {PAGE_SIZE}"
            )))
        }
        Some(_) => {
            return Err(crate::ImError::Parse(
                "im_query_channel_sync_page page_size must be integer".to_string(),
            ))
        }
    };
    let _ = page_size;
    let req_id = match object.get("req_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if !value.is_empty() && value.len() <= 256 => {
            Some(value.clone())
        }
        Some(Value::String(_)) => None,
        Some(_) => {
            return Err(crate::ImError::Parse(
                "im_query_channel_sync_page req_id must be string or null".to_string(),
            ))
        }
    };
    let next_cursor = match object.get("next_cursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if value.is_empty() => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(crate::ImError::Parse(
                "im_query_channel_sync_page next_cursor must be string or null".to_string(),
            ))
        }
    };
    Ok(PageRequest {
        channel_sync_session_id: session_id,
        next_cursor,
        req_id,
    })
}

/// 解析 complete 请求；不接受 generation/account/company 等 caller-owned scope。
pub(crate) fn parse_complete_request(payload: &[u8]) -> Result<CompleteRequest, crate::ImError> {
    let value = serde_json::from_slice::<Value>(payload).map_err(|error| {
        crate::ImError::Parse(format!("im_complete_channel_sync payload: {error}"))
    })?;
    let object = value.as_object().ok_or_else(|| {
        crate::ImError::Parse("im_complete_channel_sync payload must be an object".to_string())
    })?;
    for key in object.keys() {
        if !matches!(key.as_str(), "channel_sync_session_id" | "req_id") {
            return Err(crate::ImError::Parse(format!(
                "im_complete_channel_sync field is not caller-owned: {key}"
            )));
        }
    }
    let req_id = match object.get("req_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) if !value.is_empty() && value.len() <= 256 => {
            Some(value.clone())
        }
        Some(Value::String(_)) => None,
        Some(_) => {
            return Err(crate::ImError::Parse(
                "im_complete_channel_sync req_id must be string or null".to_string(),
            ))
        }
    };
    Ok(CompleteRequest {
        channel_sync_session_id: required_string(
            object,
            "channel_sync_session_id",
            "channel_sync_session_id",
        )?,
        req_id,
    })
}

/// 构造 account-local channel Scan；driver 只执行结构化 filter/order。
pub(crate) fn page_scan_effect(
    corr: Correlation,
    company_id: &str,
    user_id: &str,
    offset: usize,
) -> Result<Effect, crate::ImError> {
    if user_id.is_empty() {
        return Err(crate::ImError::Parse(
            "im_query_channel_sync_page requires RuntimeAuth user".to_string(),
        ));
    }
    if offset > MAX_OFFSET || offset % PAGE_SIZE as usize != 0 {
        return Err(crate::ImError::Parse(
            "channel sync page offset is out of range".to_string(),
        ));
    }
    // ScanSpec 当前只有 limit 没有 offset；因此按 cursor 扫描有界前缀并在 Helix 侧裁页。
    // 首页为 21 行（20 + lookahead），后续页为 offset + 21，保持连续页确定性。
    let limit = offset
        .saturating_add(PAGE_SIZE as usize + 1)
        .min(MAX_OFFSET) as u32;
    Ok(Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(ScanSpec {
            table: "channel",
            limit: Some(limit),
            filter: Some({
                let (column, value) =
                    crate::identity::AccountScope::new(user_id, company_id).channel_filter();
                (column, SqlValue::Text(value.to_string()))
            }),
            order_by: CHANNEL_SYNC_ORDER,
        })],
    })
}

/// 构造当前频道页的成员快照 Scan；作用域只来自当前页的真实频道 ID。
///
/// 成员所属公司 (`channel_member.team_id`) 不是频道可见性的拒绝条件：同一用户可
/// 作为外部公司的成员出现在当前公司的频道中。频道公司先由 channel rows 约束，viewer
/// membership 由投影阶段判断；完整同步页仍需其他成员生成 roster/memberCount。
pub(crate) fn member_scan_effect(
    corr: Correlation,
    channel_rows: &[Value],
    company_id: &str,
    user_id: &str,
) -> Result<Effect, crate::ImError> {
    member_scan_effect_with_filter(corr, channel_rows, company_id, user_id, None)
}

/// Dialog 与同步页共享完整成员读取，避免首次读取缺失角色容器。
pub(crate) fn dialog_member_scan_effect(
    corr: Correlation,
    channel_rows: &[Value],
    company_id: &str,
    user_id: &str,
) -> Result<Effect, crate::ImError> {
    member_scan_effect(corr, channel_rows, company_id, user_id)
}

/// 按去重且可见的频道集合估计读取容量；5000 是基线，不是成员数量上限。
fn member_read_limit(rows: &[Value], company_id: &str, user_id: &str) -> usize {
    let mut seen = std::collections::HashSet::new();
    let capacity = rows
        .iter()
        .filter(|row| !crate::query::channel_row_is_terminal(row))
        .filter(|row| {
            crate::identity::AccountScope::new(user_id, company_id).accepts_channel_tenant(
                row.get("team_id")
                    .or_else(|| row.get("teamId"))
                    .and_then(Value::as_str),
            )
        })
        .filter_map(|row| {
            let id = row
                .get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(Value::as_str)?;
            if !seen.insert(id) {
                return None;
            }
            let count = row
                .get("member_count")
                .or_else(|| row.get("memberCount"))
                .and_then(projection_i64)
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(0);
            Some(count.max(5_000))
        })
        .fold(0usize, usize::saturating_add);
    capacity.max(MEMBER_SCAN_LIMIT).saturating_add(1)
}

/// 两条查询共享频道租户与集合约束，成员过滤按调用方的实际投影需求选择。
fn member_scan_effect_with_filter(
    corr: Correlation,
    channel_rows: &[Value],
    company_id: &str,
    user_id: &str,
    filter: Option<(&'static str, SqlValue)>,
) -> Result<Effect, crate::ImError> {
    if user_id.is_empty() {
        return Err(crate::ImError::Parse(
            "im_query_channel_sync_page requires RuntimeAuth user".to_string(),
        ));
    }
    let mut channel_ids = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in channel_rows {
        if crate::query::channel_row_is_terminal(row) {
            continue;
        }
        let Some(object) = row.as_object() else {
            continue;
        };
        if !crate::identity::AccountScope::new(user_id, company_id)
            .accepts_channel_tenant(text_field(object, &["team_id", "teamId"]))
        {
            continue;
        }
        let Some(channel_id) = text_field(object, &["id", "channel_id", "channelId"]) else {
            continue;
        };
        if seen.insert(channel_id.to_string()) {
            channel_ids.push(SqlValue::Text(channel_id.to_string()));
        }
    }
    // 按当前频道集合容量分配读取上限，多个 5000 人群不能共享固定 10000 行截断。
    let member_limit = member_read_limit(channel_rows, company_id, user_id);
    Ok(Effect::Persist {
        corr,
        ops: vec![StorageOp::ScopedScan(ScopedScanSpec {
            table: "channel_member",
            scope_col: "channel_id",
            scope_values: channel_ids,
            filter,
            limit: member_limit,
        })],
    })
}

/// 将 opaque cursor 绑定到 session/generation，拒绝跨会话伪造或越界 offset。
pub(crate) fn encode_cursor(session: &ChannelSyncSession, offset: usize) -> String {
    let body = CursorBody {
        version: CURSOR_VERSION,
        session_id: session.channel_sync_session_id.clone(),
        generation: session.generation,
        offset,
    };
    let bytes = serde_json::to_vec(&body).expect("channel sync cursor is serializable");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// 解码并校验 opaque cursor；坏 cursor 必须 fail-closed，不触发分页事件。
pub(crate) fn decode_cursor(
    cursor: &str,
    session: &ChannelSyncSession,
) -> Result<usize, crate::ImError> {
    if cursor.is_empty() || cursor.len() > 4096 || cursor.len() % 2 != 0 {
        return Err(crate::ImError::Parse(
            "invalid channel sync cursor".to_string(),
        ));
    }
    let mut bytes = Vec::with_capacity(cursor.len() / 2);
    for pair in cursor.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(pair)
            .map_err(|_| crate::ImError::Parse("invalid channel sync cursor".to_string()))?;
        let byte = u8::from_str_radix(text, 16)
            .map_err(|_| crate::ImError::Parse("invalid channel sync cursor".to_string()))?;
        bytes.push(byte);
    }
    let body = serde_json::from_slice::<CursorBody>(&bytes)
        .map_err(|_| crate::ImError::Parse("invalid channel sync cursor".to_string()))?;
    if body.version != CURSOR_VERSION
        || body.session_id != session.channel_sync_session_id
        || body.generation != session.generation
        || body.offset > MAX_OFFSET
        || body.offset % PAGE_SIZE as usize != 0
    {
        return Err(crate::ImError::Parse(
            "channel sync cursor scope mismatch".to_string(),
        ));
    }
    Ok(body.offset)
}

/// 先按原始 Scan 游标裁页，再过滤不可见频道；空页仍保留后续扫描信息。
pub(crate) fn project_page(
    reply_bytes: &[u8],
    scope: &crate::query::DialogListScope,
    offset: usize,
) -> (Vec<Value>, bool) {
    let Ok(Value::Array(rows)) = serde_json::from_slice(reply_bytes) else {
        return (Vec::new(), false);
    };
    let has_more = rows.len() > offset.saturating_add(PAGE_SIZE as usize);
    let window: Vec<_> = rows
        .into_iter()
        .skip(offset)
        .take(PAGE_SIZE as usize)
        .collect();
    let items = crate::query::project_dialog_list_items(
        &serde_json::to_vec(&window).unwrap_or_default(),
        scope,
    );
    (items, has_more)
}

/// 把 channel 页和一次租户成员快照合成为绝对频道分页投影。
///
/// `channel_member` 是成员权威表；成功读回的成员快照缺少频道行时直接 fail-closed，
/// 绝不从 channel 行旧占位字段恢复可见性。返回 `None` 表示任一回包不是严格数组，
/// 调用方必须保留上一页而不能发布伪造空成员投影。游标按原始行推进，删除态在成员校验前过滤。
pub(crate) fn project_page_with_members(
    channel_bytes: &[u8],
    member_bytes: &[u8],
    scope: &crate::query::DialogListScope,
    offset: usize,
) -> Option<(Vec<Value>, bool)> {
    let Value::Array(channel_rows) = serde_json::from_slice(channel_bytes).ok()? else {
        return None;
    };
    let has_more = channel_rows.len() > offset.saturating_add(PAGE_SIZE as usize);
    let rows = channel_rows
        .into_iter()
        .skip(offset)
        .take(PAGE_SIZE as usize)
        .collect();
    Some((project_authority_rows(rows, member_bytes, scope)?, has_more))
}

/// 全部读取入口共享频道与成员权威组合；整形只在组合完成后进行一次。
pub(crate) fn project_authority_rows(
    channel_rows: Vec<Value>,
    member_bytes: &[u8],
    scope: &crate::query::DialogListScope,
) -> Option<Vec<Value>> {
    let Value::Array(member_rows) = serde_json::from_slice(member_bytes).ok()? else {
        return None;
    };
    let channel_ids: std::collections::HashSet<_> = channel_rows
        .iter()
        .filter(|row| !crate::query::channel_row_is_terminal(row))
        .filter(|row| {
            crate::identity::AccountScope::new(&scope.viewer_user_id, &scope.company_id)
                .accepts_channel_tenant(
                    row.get("team_id")
                        .or_else(|| row.get("teamId"))
                        .and_then(Value::as_str),
                )
        })
        .filter_map(|row| {
            row.get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .collect();
    let member_rows: Vec<_> = member_rows
        .into_iter()
        .filter(|row| {
            row.get("channel_id")
                .or_else(|| row.get("channelId"))
                .and_then(Value::as_str)
                .is_some_and(|id| channel_ids.contains(id))
        })
        .collect();
    // 多读一行作截断哨兵；未读完整的集合不得宣称为绝对名单。
    if member_rows.len()
        >= member_read_limit(&channel_rows, &scope.company_id, &scope.viewer_user_id)
    {
        return None;
    }
    let mut member_ids = std::collections::HashSet::new();
    let mut by_channel: std::collections::HashMap<String, Vec<Value>> =
        std::collections::HashMap::new();
    for row in member_rows {
        let object = row.as_object()?;
        let channel_id = text_field(object, &["channel_id", "channelId"])?;
        if !channel_ids.contains(channel_id) {
            continue;
        }
        let user_id = text_field(object, &["user_id", "userId", "id"])?;
        let team_id = object
            .get("team_id")
            .or_else(|| object.get("teamId"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !member_ids.insert((channel_id.to_owned(), user_id.to_owned())) {
            return None;
        }
        let source_role = text_field(object, &["role"]).unwrap_or("");
        let nickname = text_field(object, &["nick_name", "nickName", "nickname"]).unwrap_or("");
        let mut authority_member = object.clone();
        authority_member.insert("id".to_string(), Value::String(user_id.to_string()));
        authority_member.insert("userId".to_string(), Value::String(user_id.to_string()));
        authority_member.insert("teamId".to_string(), Value::String(team_id.to_string()));
        authority_member.insert("nickName".to_string(), Value::String(nickname.to_string()));
        authority_member.insert("nickname".to_string(), Value::String(nickname.to_string()));
        authority_member.insert("role".to_string(), Value::String(source_role.to_string()));
        by_channel
            .entry(channel_id.to_string())
            .or_default()
            .push(Value::Object(authority_member));
    }

    let mut seen = std::collections::HashSet::new();
    let mut invalid_viewer_projection = false;
    let page = channel_rows
        .into_iter()
        .filter(|row| !crate::query::channel_row_is_terminal(row))
        .filter_map(|row| {
            let channel_id = row
                .get("id")
                .or_else(|| row.get("channel_id"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())?
                .to_string();
            if !crate::identity::AccountScope::new(&scope.viewer_user_id, &scope.company_id)
                .accepts_channel_tenant(
                    row.get("team_id")
                        .or_else(|| row.get("teamId"))
                        .and_then(Value::as_str),
                )
            {
                return None;
            }
            // 成功读回的租户快照是成员权威；缺少该频道成员行时必须 fail-closed，不能回退到
            // channel.members 旧占位字段把已离群用户重新显示出来。
            let Ok((row, visible)) =
                attach_member_projection(row, by_channel.get(&channel_id), scope)
            else {
                invalid_viewer_projection = true;
                return None;
            };
            if visible && seen.insert(channel_id) {
                Some(crate::query::render_ready::channel::shape_channel_row(&row))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if invalid_viewer_projection {
        return None;
    }
    Some(page)
}

/// 将成员权威行分桶并覆盖频道的四组成员投影；没有该频道成员行时拒绝可见性。
fn attach_member_projection(
    mut row: Value,
    members: Option<&Vec<Value>>,
    scope: &crate::query::DialogListScope,
) -> Result<(Value, bool), ()> {
    let Some(members) = members else {
        return Ok((row, false));
    };
    if members.is_empty() {
        return Ok((row, false));
    }
    let mut regular = Vec::new();
    let mut admins = Vec::new();
    let mut bosses = Vec::new();
    let mut owner = Value::Null;
    let mut visible = false;
    let mut viewer_role = None;
    for member in members {
        let Some(source_role) = member.get("role").and_then(Value::as_str) else {
            continue;
        };
        let role = normalize_projection_role(source_role);
        let projected_member = serde_json::json!({
            "id": member.get("id").and_then(Value::as_str).unwrap_or(""),
            "userId": member.get("userId").and_then(Value::as_str).unwrap_or(""),
            "teamId": member.get("teamId").and_then(Value::as_str).unwrap_or(""),
            "nickName": member.get("nickName").and_then(Value::as_str).unwrap_or(""),
            "nickname": member.get("nickname").and_then(Value::as_str).unwrap_or(""),
            "role": role,
        });
        let is_viewer = member
            .get("userId")
            .and_then(Value::as_str)
            .is_some_and(|id| id == scope.viewer_user_id);
        visible |= is_viewer;
        if is_viewer {
            crate::event::read::overlay_viewer_dialog_projection(&mut row, member)
                .map_err(|_| ())?;
            // Keep the raw authority value for capability resolution; the normalized
            // display role must not turn an unknown or missing source role into MEMBER.
            viewer_role = canonical_projection_role(source_role);
        }
        match normalize_projection_role(role) {
            "OWNER" if owner.is_null() => owner = projected_member.clone(),
            "ADMIN" => admins.push(projected_member.clone()),
            "BOSS" => bosses.push(projected_member.clone()),
            _ => regular.push(projected_member),
        }
    }
    let Some(object) = row.as_object_mut() else {
        return Ok((row, false));
    };
    object.insert("members".to_string(), Value::Array(regular));
    object.insert("admin_users".to_string(), Value::Array(admins.clone()));
    object.insert("adminUsers".to_string(), Value::Array(admins));
    object.insert("boss".to_string(), Value::Array(bosses));
    object.insert("owner".to_string(), owner);
    object.insert("memberCount".to_string(), Value::from(members.len() as u64));
    // The channel row does not own viewer identity; carry the scoped member role into the
    // render-ready row so capability fallback follows the same authority as Go increment.
    if let Some(role) = viewer_role {
        object.insert("role".to_string(), Value::String(role.to_string()));
    }
    if object.contains_key("member_count") {
        object.insert(
            "member_count".to_string(),
            Value::from(members.len() as u64),
        );
    }
    Ok((row, visible))
}

/// Overlay the viewer's durable member authority onto a channel row before rendering.
///
/// A member row carrying any unread projection column is authoritative for that viewer. The
/// count/anchor pair is validated together: positive counts require a non-empty string anchor,
/// while zero explicitly clears it. Malformed authority fails the whole page instead of falling
/// back to a stale channel column.
pub(crate) fn overlay_viewer_unread_projection(
    channel: &mut Value,
    member: &Value,
) -> Result<(), ()> {
    let Some(member_object) = member.as_object() else {
        return Err(());
    };
    if let Some(value) = first_value(
        member_object,
        &["reaction_unread_count", "reactionUnreadCount"],
    ) {
        let count = projection_i64(value)
            .filter(|count| *count >= 0)
            .ok_or(())?;
        let object = channel.as_object_mut().ok_or(())?;
        object.insert("reactionUnreadCount".to_string(), count.into());
        object.insert("reaction_unread_count".to_string(), count.into());
    }
    let has_projection = [
        "unread_count",
        "unreadCount",
        "unread_post_id",
        "unreadPostId",
        "unReadPostId",
        "last_read_seq",
        "lastReadSeq",
        "projection_revision",
        "projectionRevision",
    ]
    .iter()
    .any(|key| member_object.contains_key(*key));
    if !has_projection {
        return Ok(());
    }
    let Some(count_value) = first_value(member_object, &["unread_count", "unreadCount"]) else {
        return Err(());
    };
    let Some(count) = projection_i64(count_value) else {
        return Err(());
    };
    if count < 0 {
        return Err(());
    }
    let read_seq = match first_value(member_object, &["last_read_seq", "lastReadSeq"]) {
        None => None,
        Some(value) => Some(
            projection_i64(value)
                .filter(|value| *value >= 0)
                .ok_or(())?,
        ),
    };
    let revision = match first_value(
        member_object,
        &["projection_revision", "projectionRevision"],
    ) {
        None => None,
        Some(value) => Some(
            projection_i64(value)
                .filter(|value| *value >= 0)
                .ok_or(())?,
        ),
    };
    if revision.is_some_and(|value| value > 0) && read_seq.is_none() {
        return Err(());
    }
    let anchor = if count == 0 {
        String::new()
    } else {
        let Some(anchor) = first_value(
            member_object,
            &["unread_post_id", "unreadPostId", "unReadPostId"],
        )
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty()) else {
            return Err(());
        };
        anchor.to_string()
    };
    let Some(channel_object) = channel.as_object_mut() else {
        return Err(());
    };
    channel_object.insert("unread_count".to_string(), Value::from(count));
    channel_object.insert("unreadCount".to_string(), Value::from(count));
    channel_object.insert("unread_post_id".to_string(), Value::String(anchor.clone()));
    channel_object.insert("unreadPostId".to_string(), Value::String(anchor));
    if let Some(read_seq) = read_seq {
        channel_object.insert("last_read_seq".to_string(), Value::from(read_seq));
        channel_object.insert("lastReadSeq".to_string(), Value::from(read_seq));
    }
    // A legacy partial cache has no effect identity: do not let UI claim its revision
    // before a complete authority response repairs the same durable row.
    if let Some(revision) = revision.filter(|value| *value > 0).filter(|_| {
        first_value(member_object, &["effect_id", "effectId"])
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
    }) {
        channel_object.insert("projection_revision".to_string(), Value::from(revision));
        channel_object.insert("projectionRevision".to_string(), Value::from(revision));
    } else {
        channel_object.remove("projection_revision");
        channel_object.remove("projectionRevision");
    }
    Ok(())
}

fn first_value<'a>(object: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

fn projection_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
}

/// 读取 storage row 的非空文本列。
fn text_field<'a>(object: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
}

/// 对齐 Go/成员写路径的四种持久角色。
fn normalize_projection_role(role: &str) -> &'static str {
    match role {
        "OWNER" | "CREATOR" => "OWNER",
        "ADMIN" | "MANAGER" | "MANGER" => "ADMIN",
        "BOSS" => "BOSS",
        _ => "MEMBER",
    }
}

/// Convert a persisted member role to the canonical Go capability role, failing closed for unknown input.
fn canonical_projection_role(role: &str) -> Option<&'static str> {
    match role.trim().to_ascii_uppercase().as_str() {
        "OWNER" | "CREATOR" => Some("CREATOR"),
        "ADMIN" | "MANAGER" | "MANGER" => Some("MANAGER"),
        "BOSS" => Some("BOSS"),
        "MEMBER" => Some("MEMBER"),
        _ => None,
    }
}

/// 从 JSON object 读取非空字符串字段。
fn required_string(
    object: &serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<String, crate::ImError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_string)
        .ok_or_else(|| crate::ImError::Parse(format!("{label} must be a non-empty string")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 个人同步只扫描当前账户，并以频道成员事实投影跨公司会话。
    #[test]
    fn personal_sync_has_no_company_filter_but_requires_membership() {
        let channels = json!([
            {"id":"company-group","team_id":"company-a","user_id":"personal"},
            {"id":"personal-group","team_id":"other-person","user_id":"personal"},
            {"id":"not-member","team_id":"company-a","user_id":"personal"}
        ]);
        let members = json!([
            {"channel_id":"company-group","user_id":"personal","team_id":"company-a","role":"MEMBER"},
            {"channel_id":"personal-group","user_id":"personal","team_id":"other-person","role":"ADMIN"}
        ]);
        let scope = crate::query::DialogListScope::new("personal", "");
        let (page, _) = project_page_with_members(
            &serde_json::to_vec(&channels).unwrap(),
            &serde_json::to_vec(&members).unwrap(),
            &scope,
            0,
        )
        .unwrap();
        assert_eq!(
            page.iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["company-group", "personal-group"]
        );
        let Effect::Persist { ops, .. } =
            page_scan_effect(Correlation::from_raw(1), "", "personal", 0).unwrap()
        else {
            panic!("scan required")
        };
        assert!(
            matches!(&ops[0], StorageOp::Scan(spec) if matches!(&spec.filter, Some(("user_id", SqlValue::Text(id))) if id == "personal"))
        );
        let Effect::Persist { ops, .. } = member_scan_effect(
            Correlation::from_raw(2),
            channels.as_array().unwrap(),
            "",
            "personal",
        )
        .unwrap() else {
            panic!("scan required")
        };
        assert!(
            matches!(&ops[0], StorageOp::ScopedScan(spec) if spec.scope_values.len() == 3 && spec.limit == 15_001)
        );
        assert!(page_scan_effect(Correlation::from_raw(1), "", "", 0).is_err());
        assert!(member_scan_effect(Correlation::from_raw(1), &[], "", "").is_err());
    }

    fn session() -> ChannelSyncSession {
        ChannelSyncSession::new("session-a".to_string(), 7, "user-a", "company-a")
    }

    /// 移除别名先于成员版本校验执行，旧零版本成员不能污染整个同步页。
    #[test]
    fn terminal_channels_are_filtered_before_member_projection() {
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        for marker in [
            json!({"is_remove": 1}),
            json!({"isRemove": true}),
            json!({"is_remove": "true"}),
        ] {
            let mut terminal = json!({"id":"deleted", "team_id":"company-a"});
            terminal
                .as_object_mut()
                .unwrap()
                .extend(marker.as_object().unwrap().clone());
            let channels = json!([terminal, {"id":"live", "team_id":"company-a"}]);
            let members = json!([
                {"channel_id":"deleted","user_id":"user-a","team_id":"company-a","role":"OWNER",
                 "unread_count":-1,"projection_revision":0},
                {"channel_id":"live","user_id":"user-a","team_id":"company-a","role":"MEMBER"}
            ]);
            let (page, more) = project_page_with_members(
                &serde_json::to_vec(&channels).unwrap(),
                &serde_json::to_vec(&members).unwrap(),
                &scope,
                0,
            )
            .expect("terminal row is excluded before malformed member");
            assert_eq!(page.len(), 1);
            assert_eq!(page[0]["id"], "live");
            assert!(!more);
            let mut owned = channels.clone();
            for row in owned.as_array_mut().unwrap() {
                row["user_id"] = json!("user-a");
            }
            let ordinary = crate::query::project_dialog_list_items(
                &serde_json::to_vec(&owned).unwrap(),
                &scope,
            );
            assert_eq!(ordinary.len(), 1);
            assert_eq!(ordinary[0]["id"], "live");
            let Effect::Persist { ops, .. } = member_scan_effect(
                Correlation::from_raw(1),
                channels.as_array().unwrap(),
                "company-a",
                "user-a",
            )
            .unwrap() else {
                panic!("member scan")
            };
            let StorageOp::ScopedScan(scan) = &ops[0] else {
                panic!("scoped scan")
            };
            assert!(matches!(scan.scope_values.as_slice(), [SqlValue::Text(id)] if id == "live"));
        }
    }

    /// 正式同步页须带回已关闭群的归档事实及当前 viewer 成员投影。
    #[test]
    fn archived_channel_survives_member_scan_and_page_projection() {
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let channels = json!([{"id":"archive", "team_id":"company-a", "type":"P",
            "is_closed":0, "delete_at":1789974402636_i64}]);
        let members = json!([{"channel_id":"archive", "user_id":"user-a",
            "team_id":"company-a", "role":"MEMBER"}]);
        let Effect::Persist { ops, .. } = member_scan_effect(
            Correlation::from_raw(1),
            channels.as_array().unwrap(),
            "company-a",
            "user-a",
        )
        .unwrap() else {
            panic!("member scan")
        };
        let StorageOp::ScopedScan(scan) = &ops[0] else {
            panic!("scoped scan")
        };
        assert!(matches!(scan.scope_values.as_slice(), [SqlValue::Text(id)] if id == "archive"));
        let (page, more) = project_page_with_members(
            &serde_json::to_vec(&channels).unwrap(),
            &serde_json::to_vec(&members).unwrap(),
            &scope,
            0,
        )
        .expect("archived member projection");
        assert!(!more);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0]["close"], false);
        assert_eq!(page[0]["deleteAt"], 1789974402636_i64);
    }

    /// 删除态占原始扫描位置，空页不能丢掉下一窗口或重复前一页。
    #[test]
    fn filtered_pages_preserve_raw_scan_cursor_and_tail() {
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let channels: Vec<_> = (0..41)
            .map(|i| {
                json!({"id":format!("c-{i}"),
            "team_id":"company-a", "is_remove":if i < 20 {1} else {0}})
            })
            .collect();
        let members: Vec<_> = (0..41)
            .map(|i| {
                json!({"channel_id":format!("c-{i}"),
            "user_id":"user-a", "team_id":"company-a", "role":"MEMBER"})
            })
            .collect();
        let member_bytes = serde_json::to_vec(&members).unwrap();
        let mut seen = Vec::new();
        for (offset, expected_len, expected_more) in [(0, 0, true), (20, 20, true), (40, 1, false)]
        {
            let prefix = &channels[..channels.len().min(offset + 21)];
            let (page, more) = project_page_with_members(
                &serde_json::to_vec(prefix).unwrap(),
                &member_bytes,
                &scope,
                offset,
            )
            .unwrap();
            assert_eq!(page.len(), expected_len, "offset={offset}");
            assert_eq!(more, expected_more, "offset={offset}");
            seen.extend(
                page.iter()
                    .map(|row| row["id"].as_str().unwrap().to_owned()),
            );
        }
        assert_eq!(seen, (20..41).map(|i| format!("c-{i}")).collect::<Vec<_>>());
    }

    #[test]
    fn page_request_has_fixed_page_size_and_strict_fields() {
        let request = parse_page_request(
            br#"{"channel_sync_session_id":"session-a","next_cursor":null,"page_size":20,"req_id":"req-1"}"#,
        )
        .expect("fixed page size accepted");
        assert_eq!(request.channel_sync_session_id, "session-a");
        assert!(request.next_cursor.is_none());
        assert_eq!(request.req_id.as_deref(), Some("req-1"));
        assert!(
            parse_page_request(br#"{"channel_sync_session_id":"session-a","page_size":21}"#)
                .is_err()
        );
        assert!(
            parse_page_request(br#"{"channel_sync_session_id":"session-a","extra":1}"#).is_err()
        );
    }

    #[test]
    fn cursor_is_bound_to_session_generation_and_offset() {
        let current = session();
        let cursor = encode_cursor(&current, 20);
        assert_eq!(decode_cursor(&cursor, &current).ok(), Some(20));
        let other = ChannelSyncSession::new("session-b".to_string(), 7, "user-a", "company-a");
        assert!(decode_cursor(&cursor, &other).is_err());
        assert!(decode_cursor("nope", &current).is_err());
    }

    /// 同步分页还要生成完整 roster/memberCount，dialog 的 viewer 过滤不得污染此入口。
    #[test]
    fn channel_sync_page_keeps_full_roster_scan() {
        use helix_core::tick::{AppCommand, PortOutcome, ReplyBytes};
        use helix_core::{EffectSink, Module, Tick};
        let mut module = crate::ImModule::new(crate::ImConfig {
            auth_user_id: "user-a".into(),
            company_id: "company-a".into(),
            ..Default::default()
        });
        module.state.channel_sync_session = Some(session());
        let mut out = EffectSink::new();
        module
            .handle(
                &Tick::Command(AppCommand::new(
                    "im_query_channel_sync_page",
                    serde_json::to_vec(&serde_json::json!({
                        "channel_sync_session_id":"session-a", "req_id":"roster-page"
                    }))
                    .unwrap(),
                )),
                1000,
                &mut out,
            )
            .unwrap();
        let corr = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .expect("channel scan");
        out.clear();
        module.handle(&Tick::PortReply { corr, outcome: PortOutcome::Ok(ReplyBytes(
            serde_json::to_vec(&serde_json::json!([{
                "id":"01hq0000000000000000000000", "team_id":"company-a", "user_id":"user-a"
            }])).unwrap().into())) }, 1001, &mut out).unwrap();
        let spec = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { ops, .. } => ops.iter().find_map(|op| match op {
                    StorageOp::ScopedScan(spec) => Some(spec),
                    _ => None,
                }),
                _ => None,
            })
            .expect("roster member scan");
        assert!(
            spec.filter.is_none(),
            "sync page needs all members for roster/memberCount"
        );
    }

    #[test]
    fn member_scan_effect_scopes_to_deduplicated_current_company_channels() {
        let rows = vec![
            serde_json::json!({"id": "channel-a", "team_id": "company-a"}),
            serde_json::json!({"id": "channel-a", "team_id": "company-a"}),
            serde_json::json!({"id": "other-company", "team_id": "company-b"}),
            serde_json::json!({"id": "channel-c", "teamId": "company-a"}),
            serde_json::json!({"id": "missing-team"}),
        ];
        let effect = member_scan_effect(Correlation::from_raw(1), &rows, "company-a", "user-a")
            .expect("company scope is required");
        let Effect::Persist { ops, .. } = effect else {
            panic!("member snapshot must be a persist effect");
        };
        assert_eq!(ops.len(), 1);
        let StorageOp::ScopedScan(spec) = &ops[0] else {
            panic!("member snapshot must use a scoped scan");
        };
        assert_eq!(spec.table, "channel_member");
        assert_eq!(spec.scope_col, "channel_id");
        assert_eq!(spec.limit, 10_001usize);
        let ids = spec
            .scope_values
            .iter()
            .map(|value| match value {
                SqlValue::Text(value) => value.as_str(),
                _ => panic!("channel scope must use text IDs"),
            })
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["channel-a", "channel-c"]);
    }

    #[test]
    fn projection_is_bounded_to_twenty_rows() {
        let rows = (0..21)
            .map(|index| {
                serde_json::json!({
                    "id": format!("channel-{index}"),
                    "team_id": "company-a",
                    "user_id": "user-a",
                    "type": "D",
                })
            })
            .collect::<Vec<_>>();
        let bytes = serde_json::to_vec(&rows).unwrap();
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let (page, has_more) = project_page(&bytes, &scope, 0);
        assert_eq!(page.len(), 20);
        assert!(has_more);
        let (tail, has_more) = project_page(&bytes, &scope, 20);
        assert_eq!(tail.len(), 1);
        assert!(!has_more);
    }

    #[test]
    fn projection_keeps_continuous_prefix_pages_for_forty_one_rows() {
        let rows = (0..41)
            .map(|index| {
                serde_json::json!({
                    "id": format!("channel-{index}"),
                    "team_id": "company-a",
                    "user_id": "user-a",
                    "type": "D",
                })
            })
            .collect::<Vec<_>>();
        let bytes = serde_json::to_vec(&rows).unwrap();
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let (first, first_more) = project_page(&bytes, &scope, 0);
        let (second, second_more) = project_page(&bytes, &scope, 20);
        let (last, last_more) = project_page(&bytes, &scope, 40);
        assert_eq!(first.len(), 20);
        assert_eq!(second.len(), 20);
        assert_eq!(last.len(), 1);
        assert!(first_more && second_more);
        assert!(!last_more);
        assert_ne!(first[0]["id"], second[0]["id"]);
        assert_ne!(second[0]["id"], last[0]["id"]);
    }

    #[test]
    fn member_projection_reads_authoritative_roles_and_stays_bounded() {
        let channels = (0..21)
            .map(|index| {
                serde_json::json!({
                    "id": format!("channel-{index}"),
                    "team_id": "company-a",
                    "display_name": format!("群 {index}"),
                    "type": "P",
                })
            })
            .collect::<Vec<_>>();
        let mut members = Vec::new();
        for index in 0..21 {
            members.push(serde_json::json!({
                "channel_id": format!("channel-{index}"),
                "user_id": "user-a",
                "team_id": "company-a",
                "role": if index == 0 { "CREATOR" } else { "MEMBER" },
                "nick_name": format!("成员 {index}"),
            }));
        }
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let (page, has_more) = project_page_with_members(
            &serde_json::to_vec(&channels).unwrap(),
            &serde_json::to_vec(&members).unwrap(),
            &scope,
            0,
        )
        .expect("valid channel/member snapshots");
        assert_eq!(page.len(), 20);
        assert!(has_more);
        assert_eq!(page[0]["owner"]["userId"], "user-a");
        assert_eq!(page[0]["memberCount"], 1);
        assert_eq!(page[0]["owner"]["nickName"], "成员 0");
        assert_eq!(page[0]["role"], "CREATOR");
        assert_eq!(page[0]["canEditChannelSettings"], true);
        assert_eq!(page[0]["canManageMembers"], true);
    }

    #[test]
    fn member_projection_keeps_external_member_company_but_rejects_other_channels() {
        let channels = serde_json::json!([
            {
                "id": "channel-a",
                "team_id": "company-a",
                "type": "P",
            },
            {
                "id": "channel-b",
                "team_id": "company-b",
                "type": "P",
            },
            {
                "id": "channel-nonmember",
                "team_id": "company-a",
                "type": "P",
            }
        ]);
        let members = serde_json::json!([
            {
                "channel_id": "channel-a",
                "user_id": "user-a",
                "team_id": "company-b",
                "role": "OWNER",
            },
            {
                "channel_id": "channel-b",
                "user_id": "user-a",
                "team_id": "company-b",
                "role": "MEMBER",
            },
            {
                "channel_id": "channel-nonmember",
                "user_id": "user-b",
                "team_id": "company-b",
                "role": "MEMBER",
            }
        ]);
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let (page, has_more) = project_page_with_members(
            &serde_json::to_vec(&channels).unwrap(),
            &serde_json::to_vec(&members).unwrap(),
            &scope,
            0,
        )
        .expect("valid snapshots");
        assert_eq!(page.len(), 1);
        assert!(!has_more);
        assert_eq!(page[0]["id"], "channel-a");
        assert_eq!(page[0]["owner"]["teamId"], "company-b");
    }

    #[test]
    fn successful_empty_member_snapshot_does_not_fallback_to_channel_placeholder() {
        let channels = serde_json::json!([{
            "id": "channel-1",
            "team_id": "company-a",
            "members": [{"userId": "user-a"}],
            "type": "P",
        }]);
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let (page, has_more) =
            project_page_with_members(&serde_json::to_vec(&channels).unwrap(), b"[]", &scope, 0)
                .expect("valid snapshots");
        assert!(page.is_empty());
        assert!(!has_more);
    }

    /// Unknown viewer roles remain visible as a member row but never grant a capability.
    #[test]
    fn member_projection_fails_closed_for_unknown_viewer_role() {
        let channels = serde_json::json!([{
            "id": "channel-unknown-role",
            "team_id": "company-a",
            "type": "O",
            "noticePermission": "MEMBER",
            "topPermission": "MEMBER"
        }]);
        let members = serde_json::json!([{
            "channel_id": "channel-unknown-role",
            "user_id": "user-a",
            "team_id": "company-a",
            "role": "UNKNOWN"
        }]);
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let (page, has_more) = project_page_with_members(
            &serde_json::to_vec(&channels).unwrap(),
            &serde_json::to_vec(&members).unwrap(),
            &scope,
            0,
        )
        .expect("valid snapshots");
        assert!(!has_more);
        assert_eq!(page.len(), 1);
        assert_eq!(page[0]["canManageMembers"], false);
        assert_eq!(page[0]["canPublishNotice"], false);
        assert_eq!(page[0]["canPinMessage"], false);
    }

    /// The emitted channel takes only its viewer authority and keeps member identities public-only.
    #[test]
    fn viewer_member_unread_projection_overlays_channel_and_validates_anchor_contract() {
        let channels = serde_json::json!([{
            "id": "channel-unread",
            "team_id": "company-a",
            "unread_count": 99,
            "unread_post_id": "stale-channel-anchor",
            "type": "P"
        }]);
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        let member = |count: Value, anchor: Value| {
            serde_json::json!([{
                "channel_id": "channel-unread",
                "user_id": "user-a",
                "team_id": "company-a",
                "role": "MEMBER",
                "unread_count": count,
                "reaction_unread_count": 2,
                "unread_post_id": anchor,
                "last_read_seq": 41,
                "projection_revision": 7,
                "effect_id": "complete-7"
            }])
        };

        let (page, _) = project_page_with_members(
            &serde_json::to_vec(&channels).unwrap(),
            &serde_json::to_vec(&member(json!(3), json!("member-anchor"))).unwrap(),
            &scope,
            0,
        )
        .expect("valid member authority");
        assert_eq!(page[0]["unreadCount"], 3);
        assert_eq!(page[0]["reactionUnreadCount"], 2);
        assert!(page[0]["members"][0].get("reaction_unread_count").is_none());
        assert_eq!(page[0]["unreadPostId"], "member-anchor");
        assert_eq!(page[0]["lastReadSeq"], 41);
        assert_eq!(page[0]["projectionRevision"], 7);
        assert!(page[0]["members"][0].get("unread_count").is_none());
        assert!(page[0]["members"][0].get("last_read_seq").is_none());

        let (page, _) = project_page_with_members(
            &serde_json::to_vec(&channels).unwrap(),
            &serde_json::to_vec(&member(json!(0), json!("stale-member-anchor"))).unwrap(),
            &scope,
            0,
        )
        .expect("zero count explicitly clears the anchor");
        assert_eq!(page[0]["unreadCount"], 0);
        assert_eq!(page[0]["unreadPostId"], "");

        for (count, anchor) in [
            (json!(2), Value::Null),
            (json!(2), json!(17)),
            (json!(2), json!("")),
        ] {
            assert!(project_page_with_members(
                &serde_json::to_vec(&channels).unwrap(),
                &serde_json::to_vec(&member(count, anchor)).unwrap(),
                &scope,
                0,
            )
            .is_none());
        }
    }
    /// 三个 5000 人群的完整名单不能被固定 10000 行上限截断。
    #[test]
    fn full_roster_projection_preserves_three_large_groups() {
        let channels: Vec<_> = (0..3).map(|index| json!({"id":format!("large-{index}"),"team_id":"company-a","user_id":"user-a","admin_users":"[]","member_count":5000})).collect();
        let members: Vec<_> = (0..3).flat_map(|index| (0..5000).map(move |member| json!({
            "channel_id":format!("large-{index}"),"user_id":if member == 0 {"user-a".to_owned()} else {format!("member-{member}")},
            "team_id":"company-a","role":if member == 0 {"OWNER"} else if member == 1 {"ADMIN"} else {"MEMBER"}
        }))).collect();
        let page = project_authority_rows(
            channels,
            &serde_json::to_vec(&members).unwrap(),
            &crate::query::DialogListScope::new("user-a", "company-a"),
        )
        .unwrap();
        assert_eq!(page.len(), 3);
        for row in page {
            assert_eq!(row["memberCount"], 5000);
            assert_eq!(row["members"].as_array().unwrap().len(), 4998);
            assert_eq!(row["adminUsers"].as_array().unwrap().len(), 1);
            assert_eq!(row["canManageMembers"], true);
        }
    }

    /// 重复成员行与截断哨兵不允许形成可缓存的部分名单。
    #[test]
    fn full_roster_rejects_duplicate_viewer_and_truncation_sentinel() {
        let rows = vec![json!({"id":"one","team_id":"company-a"})];
        let member =
            json!({"channel_id":"one","user_id":"user-a","team_id":"company-a","role":"OWNER"});
        let scope = crate::query::DialogListScope::new("user-a", "company-a");
        assert!(project_authority_rows(
            rows.clone(),
            &serde_json::to_vec(&vec![member.clone(), member.clone()]).unwrap(),
            &scope
        )
        .is_none());
        let truncated: Vec<_> = (0..10_001).map(|index| json!({"channel_id":"one","user_id":format!("member-{index}"),"team_id":"company-a","role":"MEMBER"})).collect();
        assert!(
            project_authority_rows(rows, &serde_json::to_vec(&truncated).unwrap(), &scope)
                .is_none()
        );
    }
    /// 已知的大群人数大于设计基线时，读取不能在 5000 人处截断。
    #[test]
    fn member_scan_capacity_respects_larger_durable_member_count() {
        let rows = vec![json!({"id":"large","team_id":"company-a","member_count":12000})];
        let Effect::Persist { ops, .. } =
            member_scan_effect(Correlation::from_raw(1), &rows, "company-a", "user-a").unwrap()
        else {
            panic!("member scan");
        };
        let StorageOp::ScopedScan(spec) = &ops[0] else {
            panic!("scoped member scan");
        };
        assert_eq!(spec.limit, 12001);
    }
}
