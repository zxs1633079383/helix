//! MessageV3 频道同步就绪会话与本地分页协议。
//!
//! 该模块只负责协议解析、cursor 绑定和有界投影；存储 I/O 仍由 `Effect`/`PortReply`
//! 驱动，避免 Host 侧重新解释 account、company 或分页字段。

use helix_core::effect::{Correlation, Effect, ScanOrder, ScanSpec, SqlValue, StorageOp};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 跨端固定频道页长；caller 不能通过 payload 覆盖。
pub(crate) const PAGE_SIZE: u32 = 20;
/// opaque cursor 可推进的最大本地窗口；超过后由 caller 重新建立 ready session。
const MAX_OFFSET: usize = 2_000;
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
    offset: usize,
) -> Result<Effect, crate::ImError> {
    if company_id.is_empty() {
        return Err(crate::ImError::Parse(
            "im_query_channel_sync_page requires RuntimeAuth company".to_string(),
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
            filter: Some(("team_id", SqlValue::Text(company_id.to_string()))),
            order_by: CHANNEL_SYNC_ORDER,
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

/// 从本地 Scan rows 生成严格有界的 absolute channel page。
pub(crate) fn project_page(
    reply_bytes: &[u8],
    scope: &crate::query::DialogListScope,
    offset: usize,
) -> (Vec<Value>, bool) {
    let items = crate::query::project_dialog_list_items(reply_bytes, scope);
    let has_more = items.len() > offset.saturating_add(PAGE_SIZE as usize);
    let page = items
        .into_iter()
        .skip(offset)
        .take(PAGE_SIZE as usize)
        .collect();
    (page, has_more)
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

    fn session() -> ChannelSyncSession {
        ChannelSyncSession::new("session-a".to_string(), 7, "user-a", "company-a")
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
}
