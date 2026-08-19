//! 文字接龙领域的命令、权威事实、持久投影和事件编码。
//!
//! 这里不执行 HTTP 或 SQLite；命令只生成 outbound 意图，PortReply/WS handler 再把 Go
//! authority 编译成 `StorageOp`。`channel_event_cursor` 仍是频道唯一 eventSeq 游标。

use crate::error::ImError;
use crate::state::Seq;
use helix_core::effect::{SqlValue, StorageOp, UpsertSpec};
use serde_json::{json, Map, Value};

/// 文字接龙公开命令闭集；别名在 client_api 边界收敛到这些 canonical 名称。
pub const COMMANDS: &[&str] = &[
    "post_chain_create",
    "post_chain_publish",
    "post_chain_append",
    "post_chain_get",
    "post_chain_reconcile",
    "post_chain_close",
    "post_chain_retract",
    "post_chain_mark_read",
];

/// 文字接龙 mutation 的本地状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainMutationState {
    Pending,
    Confirmed,
    Reconciling,
    Rejected,
}

impl ChainMutationState {
    /// 把内部状态降为跨仓合同中的稳定字符串。
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Confirmed => "CONFIRMED",
            Self::Reconciling => "RECONCILING",
            Self::Rejected => "REJECTED",
        }
    }
}

/// 一条在途文字接龙请求；异步回包只依赖这份 owned correlation context。
#[derive(Debug, Clone, PartialEq)]
pub struct ChainRequest {
    pub command: String,
    pub channel_id: String,
    pub chain_id: String,
    pub client_mutation_id: Option<String>,
    pub operation_id: Option<String>,
    pub device_id: Option<String>,
    pub payload: Value,
}

/// mutation 的内存镜像；持久化列由同一对象降维生成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainMutation {
    pub client_mutation_id: String,
    pub operation_id: String,
    pub state: ChainMutationState,
    pub entry_id: Option<String>,
    pub error_code: Option<String>,
}

impl ChainRequest {
    /// 返回该请求是否需要本地 mutation 账本。
    pub(crate) fn is_mutation(&self) -> bool {
        !matches!(
            self.command.as_str(),
            "post_chain_get" | "post_chain_mark_read"
        )
    }
}

/// 接龙摘要的 durable read model。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainSummary {
    pub chain_id: String,
    pub channel_id: String,
    pub mode: String,
    pub status: String,
    pub total_messages: i64,
    pub participant_count: i64,
    pub last_seq: i64,
    pub revision: i64,
}

/// 接龙条目的 durable read model。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainEntry {
    pub entry_id: String,
    pub chain_id: String,
    pub post_id: String,
    pub author_id: String,
    pub seq: i64,
    pub text: String,
    pub created_at: i64,
    pub status: String,
}

/// 接龙 head/tail 窗口的 durable read model。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainWindow {
    pub chain_id: String,
    pub head_ids: Vec<String>,
    pub tail_ids: Vec<String>,
    pub hidden_count: i64,
    pub middle_cursor: String,
    pub has_more: bool,
}

/// 当前 viewer 的接龙游标投影。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainViewer {
    pub chain_id: String,
    pub device_id: String,
    pub last_event_seq: i64,
    pub last_read_at: i64,
}

/// Go HTTP/WS 带回的一组已经解析的 authority facts。
#[derive(Debug, Clone, PartialEq)]
pub struct ChainAuthority {
    pub chain_id: String,
    pub channel_id: String,
    pub event_id: Option<String>,
    pub event_seq: Option<Seq>,
    pub event_type: Option<String>,
    pub summary: Option<ChainSummary>,
    pub entry: Option<ChainEntry>,
    pub entries: Vec<ChainEntry>,
    pub window: Option<ChainWindow>,
    pub viewer: Option<ChainViewer>,
    pub revision: i64,
    pub raw: Value,
}

/// 同步回放时附加到普通频道批次的接龙写集和领域事件。
///
/// 该值只在 `PersistAtomic` 事务提交后用于 Emit；它不携带任何 I/O 句柄，也不改变频道
/// `eventSeq` gate 的推进方式。
#[derive(Debug, Clone)]
pub(crate) struct SyncedChainProjection {
    pub ops: Vec<StorageOp>,
    pub event: Vec<u8>,
    /// Channel-event identity reserved before the atomic write to suppress WS/sync races.
    pub event_id: Option<String>,
}

/// 区分 CommonRes 业务拒绝和缺字段/坏 JSON，PortReply 据此选择 REJECTED 或重对账。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainReplyError {
    Rejected { code: String },
    Invalid { message: String },
}

/// 判断名称是否属于文字接龙命令闭集。
pub fn is_command(name: &str) -> bool {
    COMMANDS.contains(&name)
}

/// 从 command payload 计算稳定 operationId；不依赖时钟、随机数或 host。
pub(crate) fn stable_operation_id(command: &str, args: &Value) -> Option<String> {
    let client_mutation_id = args
        .get("client_mutation_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    let chain_id = args
        .get("chain_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if client_mutation_id.is_none() && chain_id.is_empty() {
        return None;
    }
    let seed = client_mutation_id
        .map(str::to_owned)
        .unwrap_or_else(|| serde_json::to_string(args).unwrap_or_default());
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in format!("{command}\u{1f}{chain_id}\u{1f}{seed}").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(format!("chain-op-{hash:016x}"))
}

/// 为创建意图生成跨重试稳定的 ChainId；Angular 不承担聚合身份分配。
pub(crate) fn stable_chain_id(args: &Value) -> Option<String> {
    let object = args.as_object()?;
    let mutation = object
        .get("client_mutation_id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in format!("post_chain_create\u{1f}{mutation}").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    Some(format!("chain-{hash:016x}"))
}

/// 在 module 进入 outbound registry 前注入 Helix-owned operation_id。
pub(crate) fn add_operation_id(
    payload: &[u8],
    operation_id: Option<&str>,
) -> Result<Vec<u8>, ImError> {
    let mut value: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("chain command payload: {error}")))?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| ImError::Parse("chain command payload must be object".to_string()))?;
    if let Some(operation_id) = operation_id.filter(|value| !value.is_empty()) {
        object.insert(
            "operation_id".to_string(),
            Value::String(operation_id.to_string()),
        );
    }
    serde_json::to_vec(&value).map_err(|error| ImError::Serialize(error.to_string()))
}

/// 解析已归一化的 canonical chain command，并把内部 operationId 固定进 correlation。
pub(crate) fn request_from_command(command: &str, payload: &[u8]) -> Result<ChainRequest, ImError> {
    let args: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("{command} payload: {error}")))?;
    let object = args
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{command}: payload must be object")))?;
    let channel_id = required_string(object, "channel_id", command)?;
    let chain_id = match object.get("chain_id") {
        Some(_) => required_string(object, "chain_id", command)?,
        None if command == "post_chain_create" => stable_chain_id(&args).ok_or_else(|| {
            ImError::Parse(format!(
                "{command}: client_mutation_id required to derive chain_id"
            ))
        })?,
        None => return Err(ImError::Parse(format!("{command}: missing chain_id"))),
    };
    if let Some(mode) = object.get("mode") {
        if mode.as_str().is_none_or(|value| value != "TEXT") {
            return Err(ImError::Parse(format!(
                "{command}: only TEXT mode is supported"
            )));
        }
    }
    let client_mutation_id = optional_string(object, "client_mutation_id", command)?;
    if !matches!(command, "post_chain_get" | "post_chain_mark_read") && client_mutation_id.is_none()
    {
        return Err(ImError::Parse(format!(
            "{command}: missing client_mutation_id"
        )));
    }
    let operation_id = optional_string(object, "operation_id", command)?;
    let device_id = optional_string(object, "device_id", command)?;
    validate_keys(command, object)?;
    Ok(ChainRequest {
        command: command.to_string(),
        channel_id,
        chain_id,
        client_mutation_id,
        operation_id,
        device_id,
        payload: args,
    })
}

/// 组装 registry 使用的 snake_case→Go lowerCamelCase body，并拒绝 category 字段渗入 TEXT。
pub(crate) fn wire_body(command: &str, args: &Value) -> Result<(&'static str, Value), ImError> {
    let request = request_from_command(
        command,
        &serde_json::to_vec(args)
            .map_err(|error| ImError::Serialize(format!("{command} args: {error}")))?,
    )?;
    let object = request
        .payload
        .as_object()
        .ok_or_else(|| ImError::Parse(format!("{command}: payload must be object")))?;
    let mut body = Map::new();
    body.insert("channelId".to_string(), Value::String(request.channel_id));
    body.insert("chainId".to_string(), Value::String(request.chain_id));
    if let Some(device_id) = request.device_id {
        body.insert("deviceId".to_string(), Value::String(device_id));
    }
    if let Some(client_mutation_id) = request.client_mutation_id {
        body.insert(
            "clientMutationId".to_string(),
            Value::String(client_mutation_id),
        );
    }
    if let Some(operation_id) = request.operation_id {
        body.insert("operationId".to_string(), Value::String(operation_id));
    }
    let path = match command {
        "post_chain_create" => {
            body.insert("mode".to_string(), Value::String("TEXT".to_string()));
            copy_optional(&mut body, object, "title", "title");
            copy_optional(&mut body, object, "description", "description");
            copy_optional(&mut body, object, "deadline_at", "deadlineAt");
            "post-chain/create"
        }
        "post_chain_publish" => {
            copy_optional(&mut body, object, "expected_revision", "expectedRevision");
            copy_optional(&mut body, object, "deadline_at", "deadlineAt");
            "post-chain/publish"
        }
        "post_chain_append" => {
            copy_required(&mut body, object, "text", "text", command)?;
            let attachments = object
                .get("attachments")
                .cloned()
                .unwrap_or_else(|| json!([]));
            if !attachments.is_array() {
                return Err(ImError::Parse(format!(
                    "{command}: attachments must be array"
                )));
            }
            body.insert("attachments".to_string(), attachments);
            "post-chain/append"
        }
        "post_chain_get" => {
            copy_optional(&mut body, object, "middle_cursor", "middleCursor");
            if let Some(limit) = object.get("limit") {
                if !limit.is_u64() && !limit.is_i64() {
                    return Err(ImError::Parse(format!("{command}: limit must be integer")));
                }
                body.insert("limit".to_string(), limit.clone());
            }
            "post-chain/get"
        }
        "post_chain_reconcile" => "post-chain/reconcile",
        "post_chain_close" => {
            copy_optional(&mut body, object, "expected_revision", "expectedRevision");
            "post-chain/close"
        }
        "post_chain_retract" => {
            copy_optional(&mut body, object, "expected_revision", "expectedRevision");
            copy_required(&mut body, object, "entry_id", "entryId", command)?;
            "post-chain/retract"
        }
        "post_chain_mark_read" => {
            copy_optional(&mut body, object, "last_event_seq", "lastEventSeq");
            copy_optional(&mut body, object, "last_read_at", "lastReadAt");
            copy_optional(&mut body, object, "middle_cursor", "middleCursor");
            "post-chain/mark-read"
        }
        _ => return Err(ImError::Parse(format!("unknown chain command {command}"))),
    };
    Ok((path, Value::Object(body)))
}

/// 解析 HTTP PortReply 中的 CommonRes/ADR-007 envelope，业务拒绝不伪造 authority。
pub(crate) fn decode_http_authority(
    reply: &[u8],
    request: &ChainRequest,
) -> Result<ChainAuthority, ChainReplyError> {
    let raw = if let Ok(envelope) = serde_json::from_slice::<Value>(reply) {
        if envelope.get("body").and_then(Value::as_str).is_some()
            && envelope.get("status").and_then(Value::as_u64).is_some()
        {
            crate::http_envelope::unwrap_sync_envelope(reply).map_err(|error| {
                ChainReplyError::Invalid {
                    message: error.to_string(),
                }
            })?
        } else {
            reply.to_vec()
        }
    } else {
        reply.to_vec()
    };
    let root: Value = serde_json::from_slice(&raw).map_err(|error| ChainReplyError::Invalid {
        message: format!("JSON parse: {error}"),
    })?;
    check_business_status(&root)?;
    let data = root.get("data").unwrap_or(&root);
    parse_authority(
        data,
        request,
        root.get("eventId").and_then(Value::as_str),
        None,
    )
}

/// 从 WS root/data authority frame 编译同一套 chain projection，避免 HTTP/WS 两套 reducer。
pub(crate) fn authority_from_ws(
    root: &Value,
    event_seq: Option<Seq>,
) -> Result<ChainAuthority, ChainReplyError> {
    let data = root.get("data").unwrap_or(root);
    // Go 的 channel_event payload 将身份嵌套在 chainSummary/chainEntry；不能只读 data 根，
    // 否则 publish/close 这类没有 entry 的事件会在 WS 边界被静默丢弃。
    let channel_id = find_string(data, &["channelId", "channel_id"])
        .or_else(|| {
            find_nested_string(
                data,
                &["chainSummary", "chain_summary", "chainEntry", "chain_entry"],
                &["channelId", "channel_id"],
            )
        })
        .unwrap_or_default();
    let chain_id = find_string(data, &["chainId", "chain_id"])
        .or_else(|| {
            find_nested_string(
                data,
                &["chainSummary", "chain_summary", "chainEntry", "chain_entry"],
                &["chainId", "chain_id"],
            )
        })
        .unwrap_or_default();
    if channel_id.is_empty() || chain_id.is_empty() {
        return Err(ChainReplyError::Invalid {
            message: "WS authority missing channelId/chainId".to_string(),
        });
    }
    let request = ChainRequest {
        command: "post_chain_event".to_string(),
        channel_id,
        chain_id,
        client_mutation_id: find_string(data, &["clientMutationId", "client_mutation_id"]),
        operation_id: find_string(data, &["operationId", "operation_id"]),
        device_id: None,
        payload: data.clone(),
    };
    let mut authority = parse_authority(
        data,
        &request,
        root.get("eventId")
            .and_then(Value::as_str)
            .or_else(|| data.get("eventId").and_then(Value::as_str)),
        event_seq,
    )?;
    authority.event_type = find_string(root, &["eventType", "event_type", "action"])
        .or_else(|| find_string(data, &["eventType", "event_type", "action"]))
        .or(authority.event_type);
    Ok(authority)
}

/// 从 `/channel/sync/notify` 的 `ChannelEvent.payload` 编译接龙投影。
///
/// 普通消息仍由 sync flow 的既有写集处理；只有带 canonical `im:post_chain:*` action 的
/// payload 才进入这里。接龙 payload 一旦声明为该领域但缺少 authority，直接返回错误，避免
/// cursor 已前进而 `chain_summary/chain_entry/textWindow` 缺失。
pub(crate) fn synced_projection(
    event: &crate::sync::session::EventEnvelope,
) -> Result<Option<SyncedChainProjection>, ImError> {
    if event.event_payload.trim().is_empty() {
        return Ok(None);
    }
    // Legacy/non-chain channel events may carry an opaque payload. Only JSON objects can expose
    // the chain action; leave opaque payloads to the existing generic message replay path.
    let Ok(payload) = serde_json::from_str::<Value>(&event.event_payload) else {
        return Ok(None);
    };
    let root_action = find_string(&payload, &["action", "eventType", "event_type"]);
    let action = root_action.clone().or_else(|| {
        payload
            .get("data")
            .and_then(|data| find_string(data, &["action", "eventType", "event_type"]))
    });
    let Some(event_name) = normalize_sync_event_name(action.as_deref()) else {
        return Ok(None);
    };
    // `event.channel_id` 已由 SyncBatchFacts 校验过，允许稀疏历史 payload 省略重复的
    // channelId；其余 chain identity 仍必须来自 authority payload，不能由客户端臆造。
    let mut authority_payload = if root_action.is_none() {
        payload
            .get("data")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| payload.clone())
    } else {
        payload.clone()
    };
    if let Some(object) = authority_payload.as_object_mut() {
        object
            .entry("channelId".to_string())
            .or_insert_with(|| json!(event.channel_id.as_str()));
    }
    let root = json!({
        "action": event_name,
        "eventId": event.event_id,
        "eventSeq": event.seq.0,
        "data": authority_payload,
    });
    let authority = authority_from_ws(&root, Some(event.seq)).map_err(|error| {
        ImError::Parse(format!(
            "chain sync authority seq {}: {:?}",
            event.seq.0, error
        ))
    })?;
    if authority.channel_id != event.channel_id.as_str() {
        return Err(ImError::Parse(format!(
            "chain sync channel mismatch seq {}: {} != {}",
            event.seq.0,
            authority.channel_id,
            event.channel_id.as_str()
        )));
    }
    let request = ChainRequest {
        command: command_for_event(Some(event_name)).to_string(),
        channel_id: authority.channel_id.clone(),
        chain_id: authority.chain_id.clone(),
        client_mutation_id: find_string(
            &authority_payload,
            &["clientMutationId", "client_mutation_id"],
        ),
        operation_id: find_string(&authority_payload, &["operationId", "operation_id"]),
        device_id: None,
        payload: authority.raw.clone(),
    };
    let mutation_state = request
        .client_mutation_id
        .as_ref()
        .filter(|_| request.is_mutation())
        .map(|_| ChainMutationState::Confirmed);
    let ops = persist_ops(&authority, &request, mutation_state, None);
    let event = event_bytes(event_name, &request, Some(&authority), mutation_state, None)?;
    Ok(Some(SyncedChainProjection {
        ops,
        event,
        event_id: authority.event_id,
    }))
}

/// 将历史 snake/canonical action 收敛为唯一的跨端事件名称。
fn normalize_sync_event_name(event_type: Option<&str>) -> Option<&'static str> {
    match event_type.unwrap_or_default() {
        "im:post_chain:draft" | "post_chain_create" => Some("im:post_chain:draft"),
        "im:post_chain:publish" | "post_chain_publish" => Some("im:post_chain:publish"),
        "im:post_chain:upsert" | "post_chain_append" => Some("im:post_chain:upsert"),
        "im:post_chain:append_rejected" => Some("im:post_chain:append_rejected"),
        "im:post_chain:reconcile" | "post_chain_reconcile" => Some("im:post_chain:reconcile"),
        "im:post_chain:close" | "post_chain_close" => Some("im:post_chain:close"),
        "im:post_chain:retract" | "post_chain_retract" => Some("im:post_chain:retract"),
        "im:post_chain:read_cursor" | "post_chain_mark_read" => Some("im:post_chain:read_cursor"),
        _ => None,
    }
}

/// 生成 ChainSummary/Entry/window/viewer/mutation 的同一事务写集。
pub(crate) fn persist_ops(
    authority: &ChainAuthority,
    request: &ChainRequest,
    mutation_state: Option<ChainMutationState>,
    error_code: Option<&str>,
) -> Vec<StorageOp> {
    let mut ops = Vec::new();
    if let Some(summary) = &authority.summary {
        ops.push(StorageOp::BatchUpsert(UpsertSpec::new(
            "chain_summary",
            vec![vec![
                ("chain_id".to_string(), text(&summary.chain_id)),
                ("channel_id".to_string(), text(&summary.channel_id)),
                ("mode".to_string(), text(&summary.mode)),
                ("status".to_string(), text(&summary.status)),
                (
                    "total_messages".to_string(),
                    integer(summary.total_messages),
                ),
                (
                    "participant_count".to_string(),
                    integer(summary.participant_count),
                ),
                ("last_seq".to_string(), integer(summary.last_seq)),
                ("revision".to_string(), integer(summary.revision)),
            ]],
            Some("chain_id"),
        )));
    }
    let entries = if authority.entries.is_empty() {
        authority.entry.iter().collect::<Vec<_>>()
    } else {
        authority.entries.iter().collect::<Vec<_>>()
    };
    if !entries.is_empty() {
        ops.push(StorageOp::BatchUpsert(UpsertSpec::new(
            "chain_entry",
            entries
                .iter()
                .map(|entry| {
                    vec![
                        ("entry_id".to_string(), text(&entry.entry_id)),
                        ("chain_id".to_string(), text(&entry.chain_id)),
                        ("post_id".to_string(), text(&entry.post_id)),
                        ("author_id".to_string(), text(&entry.author_id)),
                        ("seq".to_string(), integer(entry.seq)),
                        ("text".to_string(), text(&entry.text)),
                        ("created_at".to_string(), integer(entry.created_at)),
                        ("status".to_string(), text(&entry.status)),
                    ]
                })
                .collect(),
            Some("entry_id"),
        )));
    }
    if let Some(window) = &authority.window {
        ops.push(StorageOp::BatchUpsert(UpsertSpec::new(
            "chain_window",
            vec![vec![
                ("chain_id".to_string(), text(&window.chain_id)),
                ("head_ids".to_string(), json_text(&window.head_ids)),
                ("tail_ids".to_string(), json_text(&window.tail_ids)),
                ("hidden_count".to_string(), integer(window.hidden_count)),
                ("middle_cursor".to_string(), text(&window.middle_cursor)),
                ("has_more".to_string(), integer(i64::from(window.has_more))),
            ]],
            Some("chain_id"),
        )));
    }
    if let Some(viewer) = &authority.viewer {
        ops.push(StorageOp::BatchUpsert(UpsertSpec::new(
            "chain_viewer",
            vec![vec![
                (
                    "viewer_key".to_string(),
                    text(&format!("{}:{}", viewer.chain_id, viewer.device_id)),
                ),
                ("chain_id".to_string(), text(&viewer.chain_id)),
                ("device_id".to_string(), text(&viewer.device_id)),
                ("last_event_seq".to_string(), integer(viewer.last_event_seq)),
                ("last_read_at".to_string(), integer(viewer.last_read_at)),
            ]],
            Some("viewer_key"),
        )));
    }
    if let (Some(client_mutation_id), Some(state)) = (&request.client_mutation_id, mutation_state) {
        ops.push(mutation_op(
            request,
            client_mutation_id,
            state,
            authority
                .entry
                .as_ref()
                .map(|entry| entry.entry_id.as_str()),
            error_code,
        ));
    }
    ops
}

/// 生成只更新 mutation 账本的原子写，用于 PENDING 之后的 RECONCILING/REJECTED。
pub(crate) fn mutation_op(
    request: &ChainRequest,
    client_mutation_id: &str,
    state: ChainMutationState,
    entry_id: Option<&str>,
    error_code: Option<&str>,
) -> StorageOp {
    StorageOp::BatchUpsert(UpsertSpec::new(
        "chain_mutation",
        vec![vec![
            ("client_mutation_id".to_string(), text(client_mutation_id)),
            (
                "operation_id".to_string(),
                text(request.operation_id.as_deref().unwrap_or_default()),
            ),
            ("state".to_string(), text(state.as_str())),
            ("entry_id".to_string(), text(entry_id.unwrap_or_default())),
            (
                "error_code".to_string(),
                text(error_code.unwrap_or_default()),
            ),
        ]],
        Some("client_mutation_id"),
    ))
}

/// 仅在同一 PersistAtomic 成功后编码 canonical `im:post_chain:*` 事件。
pub(crate) fn event_bytes(
    event_name: &str,
    request: &ChainRequest,
    authority: Option<&ChainAuthority>,
    state: Option<ChainMutationState>,
    error_code: Option<&str>,
) -> Result<Vec<u8>, ImError> {
    if event_name_static(event_name).is_none() {
        return Err(ImError::Parse(format!("unknown chain event {event_name}")));
    }
    let mut data = Map::new();
    data.insert("channelId".to_string(), json!(request.channel_id));
    if let Some(value) = &request.client_mutation_id {
        data.insert("clientMutationId".to_string(), json!(value));
    }
    if let Some(value) = &request.operation_id {
        data.insert("operationId".to_string(), json!(value));
    }
    if let Some(state) = state {
        data.insert("state".to_string(), json!(state.as_str()));
    }
    if let Some(error_code) = error_code.filter(|value| !value.is_empty()) {
        data.insert("errorCode".to_string(), json!(error_code));
    }
    if let Some(authority) = authority {
        if let Some(event_id) = &authority.event_id {
            data.insert("eventId".to_string(), json!(event_id));
        }
        if let Some(event_seq) = authority.event_seq {
            data.insert("eventSeq".to_string(), json!(event_seq.0));
        }
        if let Some(summary) = &authority.summary {
            data.insert("chainSummary".to_string(), summary_value(summary));
        }
        if let Some(entry) = &authority.entry {
            data.insert("chainEntry".to_string(), entry_value(entry));
        }
        if authority.entries.len() > 1 {
            data.insert(
                "chainEntries".to_string(),
                Value::Array(authority.entries.iter().map(entry_value).collect()),
            );
        }
        if let Some(window) = &authority.window {
            data.insert("textWindow".to_string(), window_value(window));
        }
        if let Some(viewer) = &authority.viewer {
            data.insert("viewer".to_string(), viewer_value(viewer));
        }
    } else {
        // Rejected/reconciling events still carry identity under mutation, never as a bare chainId.
        data.insert(
            "mutation".to_string(),
            json!({
                "chainId": request.chain_id,
                "clientMutationId": request.client_mutation_id,
                "operationId": request.operation_id,
                "state": state.map(ChainMutationState::as_str),
                "errorCode": error_code,
            }),
        );
    }
    let bytes = serde_json::to_vec(&json!({
        "event": event_name,
        "data": Value::Object(data),
    }))
    .map_err(|error| ImError::Serialize(error.to_string()))?;
    Ok(bytes)
}

/// 由 command 决定 HTTP 成功后的 canonical 事件名。
pub(crate) fn success_event_name(command: &str) -> &'static str {
    match command {
        "post_chain_create" => "im:post_chain:draft",
        "post_chain_publish" => "im:post_chain:publish",
        "post_chain_append" => "im:post_chain:upsert",
        "post_chain_get" => "im:post_chain:upsert",
        "post_chain_reconcile" => "im:post_chain:reconcile",
        "post_chain_close" => "im:post_chain:close",
        "post_chain_retract" => "im:post_chain:retract",
        "post_chain_mark_read" => "im:post_chain:read_cursor",
        _ => "im:post_chain:reconcile",
    }
}

/// 由 command 决定业务拒绝后的事件名；只有 append 使用专门的 rejected 事件。
pub(crate) fn rejected_event_name(command: &str) -> &'static str {
    if command == "post_chain_append" {
        "im:post_chain:append_rejected"
    } else {
        success_event_name(command)
    }
}

/// 把 WS eventType 映射到同一套 command 事件语义，避免 HTTP/WS reducer 分叉。
pub(crate) fn command_for_event(event_type: Option<&str>) -> &'static str {
    match event_type.unwrap_or_default() {
        "im:post_chain:draft" | "post_chain_create" => "post_chain_create",
        "im:post_chain:publish" | "post_chain_publish" => "post_chain_publish",
        "im:post_chain:close" | "post_chain_close" => "post_chain_close",
        "im:post_chain:retract" | "post_chain_retract" => "post_chain_retract",
        "im:post_chain:read_cursor" | "post_chain_mark_read" => "post_chain_mark_read",
        "im:post_chain:reconcile" | "post_chain_reconcile" => "post_chain_reconcile",
        _ => "post_chain_append",
    }
}

/// 返回事件名的静态集合，确保跨端 event envelope 不接受用户控制字符串。
fn event_name_static(event_name: &str) -> Option<&'static str> {
    match event_name {
        "im:post_chain:draft" => Some("im:post_chain:draft"),
        "im:post_chain:publish" => Some("im:post_chain:publish"),
        "im:post_chain:upsert" => Some("im:post_chain:upsert"),
        "im:post_chain:append_rejected" => Some("im:post_chain:append_rejected"),
        "im:post_chain:reconcile" => Some("im:post_chain:reconcile"),
        "im:post_chain:close" => Some("im:post_chain:close"),
        "im:post_chain:retract" => Some("im:post_chain:retract"),
        "im:post_chain:read_cursor" => Some("im:post_chain:read_cursor"),
        _ => None,
    }
}

/// 验证 command 顶层字段只来自该命令合同，防止 Angular 把业务事实塞入 Helix builder。
fn validate_keys(command: &str, object: &Map<String, Value>) -> Result<(), ImError> {
    let allowed: &[&str] = match command {
        "post_chain_create" => &[
            "channel_id",
            "chain_id",
            "mode",
            "title",
            "description",
            "deadline_at",
            "client_mutation_id",
            "device_id",
            "operation_id",
            "req_id",
        ],
        "post_chain_publish" => &[
            "channel_id",
            "chain_id",
            "expected_revision",
            "deadline_at",
            "client_mutation_id",
            "device_id",
            "operation_id",
            "req_id",
        ],
        "post_chain_append" => &[
            "channel_id",
            "chain_id",
            "text",
            "attachments",
            "client_mutation_id",
            "device_id",
            "operation_id",
            "req_id",
        ],
        "post_chain_get" => &[
            "channel_id",
            "chain_id",
            "middle_cursor",
            "limit",
            "device_id",
            "req_id",
        ],
        "post_chain_reconcile" => &[
            "channel_id",
            "chain_id",
            "client_mutation_id",
            "operation_id",
            "device_id",
            "req_id",
        ],
        "post_chain_close" => &[
            "channel_id",
            "chain_id",
            "expected_revision",
            "client_mutation_id",
            "device_id",
            "operation_id",
            "req_id",
        ],
        "post_chain_retract" => &[
            "channel_id",
            "chain_id",
            "entry_id",
            "expected_revision",
            "client_mutation_id",
            "device_id",
            "operation_id",
            "req_id",
        ],
        "post_chain_mark_read" => &[
            "channel_id",
            "chain_id",
            "last_event_seq",
            "last_read_at",
            "middle_cursor",
            "client_mutation_id",
            "operation_id",
            "device_id",
            "req_id",
        ],
        _ => return Err(ImError::Parse(format!("unknown chain command {command}"))),
    };
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ImError::Parse(format!("{command}: unknown field {key}")));
    }
    Ok(())
}

/// 解析 envelope 的业务状态；HTTP 200 不等于 CommonRes 成功。
fn check_business_status(root: &Value) -> Result<(), ChainReplyError> {
    if let Some(status) = root.get("status").and_then(Value::as_str) {
        if !matches!(
            status.to_ascii_uppercase().as_str(),
            "SUCCESS" | "OK" | "TRUE"
        ) {
            let code = root
                .get("code")
                .or_else(|| root.get("data").and_then(|data| data.get("code")))
                .map(value_code)
                .unwrap_or_else(|| status.to_string());
            return Err(ChainReplyError::Rejected { code });
        }
    }
    if let Some(code) = root.get("code") {
        if let Some(number) = code.as_i64() {
            if number != 0 {
                return Err(ChainReplyError::Rejected {
                    code: number.to_string(),
                });
            }
        } else if let Some(value) = code.as_str() {
            let upper = value.to_ascii_uppercase();
            if !matches!(upper.as_str(), "" | "0" | "SUCCESS" | "OK") {
                return Err(ChainReplyError::Rejected {
                    code: value.to_string(),
                });
            }
        }
    }
    Ok(())
}

/// 从 authority 节点提取摘要、entry、窗口和 viewer，并要求至少一类事实存在。
fn parse_authority(
    value: &Value,
    request: &ChainRequest,
    response_event_id: Option<&str>,
    response_event_seq: Option<Seq>,
) -> Result<ChainAuthority, ChainReplyError> {
    let chain_id =
        find_string(value, &["chainId", "chain_id"]).unwrap_or_else(|| request.chain_id.clone());
    let channel_id = find_string(value, &["channelId", "channel_id"])
        .unwrap_or_else(|| request.channel_id.clone());
    let summary = find_object(value, &["chainSummary", "chain_summary"])
        .or_else(|| find_summary_like(value))
        .and_then(|node| parse_summary(node, &chain_id, &channel_id));
    let entries = find_entries_like(value, &chain_id);
    let entry = find_object(value, &["chainEntry", "chain_entry", "entry"])
        .or_else(|| find_entry_like(value))
        .and_then(|node| parse_entry(node, &chain_id));
    let window = find_object(
        value,
        &[
            "textWindow",
            "text_window",
            "window",
            "chainWindow",
            "chain_window",
        ],
    )
    .and_then(|node| parse_window(node, &chain_id))
    .or_else(|| parse_window_from_result(value, &chain_id));
    let viewer = find_object(value, &["viewer", "chainViewer", "chain_viewer"])
        .and_then(|node| parse_viewer(node, &chain_id));
    if summary.is_none()
        && entry.is_none()
        && entries.is_empty()
        && window.is_none()
        && viewer.is_none()
    {
        return Err(ChainReplyError::Invalid {
            message: "authority contains no chain projection".to_string(),
        });
    }
    let event = value
        .get("event")
        .or_else(|| value.get("data").and_then(|data| data.get("event")));
    let event_id = event
        .and_then(|item| find_local_string(item, &["eventId", "event_id"]))
        .or_else(|| response_event_id.map(str::to_string));
    let event_seq = event
        .and_then(|item| find_u64(item, &["eventSeq", "event_seq"]).map(Seq))
        .or(response_event_seq)
        .or_else(|| find_u64(value, &["eventSeq", "event_seq"]).map(Seq));
    let event_type = event
        .and_then(|item| find_local_string(item, &["action", "eventType", "event_type"]))
        .or_else(|| find_string(value, &["eventType", "event_type", "action"]));
    let revision = summary.as_ref().map(|item| item.revision).unwrap_or(0);
    Ok(ChainAuthority {
        chain_id,
        channel_id,
        event_id: event_id.or_else(|| find_string(value, &["eventId", "event_id"])),
        event_seq,
        event_type,
        summary,
        entry: entry.or_else(|| entries.last().cloned()),
        entries,
        window,
        viewer,
        revision,
        raw: value.clone(),
    })
}

/// 返回指定名称的 object 节点，兼容 data/chain 的一层嵌套。
fn find_object<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    let candidates = [
        Some(value),
        value.get("data"),
        value.get("event"),
        value.get("event").and_then(|event| event.get("data")),
        value.get("data").and_then(|data| data.get("event")),
        value
            .get("data")
            .and_then(|data| data.get("event"))
            .and_then(|event| event.get("data")),
        value.get("chain"),
        value.get("chainEntry"),
        value.get("chain_entry"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|node| keys.iter().find_map(|key| node.get(*key)))
}

/// 在根/data/chain 节点中找一类字段。
fn find_string(value: &Value, keys: &[&str]) -> Option<String> {
    let candidates = [Some(value), value.get("data"), value.get("chain")];
    candidates
        .into_iter()
        .flatten()
        .find_map(|node| {
            keys.iter()
                .find_map(|key| node.get(*key))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 在根/data 节点中找一个非负整数。
fn find_u64(value: &Value, keys: &[&str]) -> Option<u64> {
    let candidates = [Some(value), value.get("data"), value.get("chain")];
    candidates.into_iter().flatten().find_map(|node| {
        keys.iter()
            .find_map(|key| node.get(*key))
            .and_then(Value::as_u64)
    })
}

/// 从嵌套 authority 节点提取 identity 字段。
fn find_nested_string(value: &Value, nodes: &[&str], keys: &[&str]) -> Option<String> {
    nodes.iter().find_map(|node| {
        value
            .get(*node)
            .and_then(|child| find_local_string(child, keys))
    })
}

/// 找到包含摘要绝对字段的节点。
fn find_summary_like(value: &Value) -> Option<&Value> {
    let candidates = [Some(value), value.get("data"), value.get("chain")];
    candidates.into_iter().flatten().find(|node| {
        node.get("totalMessages")
            .or_else(|| node.get("total_messages"))
            .is_some()
            && node
                .get("lastSeq")
                .or_else(|| node.get("last_seq"))
                .is_some()
    })
}

/// 找到包含 entry identity 的节点。
fn find_entry_like(value: &Value) -> Option<&Value> {
    let candidates = [Some(value), value.get("data"), value.get("chain")];
    candidates.into_iter().flatten().find(|node| {
        node.get("entryId")
            .or_else(|| node.get("entry_id"))
            .is_some()
            && node.get("seq").is_some()
    })
}

/// 从 Go 查询结果的 head/tail/entries 数组提取全部 Entry，保持服务端顺序。
fn find_entries_like(value: &Value, chain_id: &str) -> Vec<ChainEntry> {
    let candidates = [Some(value), value.get("data")];
    candidates
        .into_iter()
        .flatten()
        .flat_map(|node| {
            let keys: &[&str] =
                if node.get("chainEntries").is_some() || node.get("chain_entries").is_some() {
                    &["chainEntries", "chain_entries"]
                } else {
                    &["entries", "headEntries", "tailEntries"]
                };
            keys.iter()
                .copied()
                .filter_map(move |key| node.get(key))
                .filter_map(Value::as_array)
                .flat_map(|items| items.iter())
                .filter_map(|item| parse_entry(item, chain_id))
        })
        .collect()
}

/// 兼容 Go `WindowResult` 的扁平字段，生成 Helix 的统一窗口 read model。
fn parse_window_from_result(value: &Value, chain_id: &str) -> Option<ChainWindow> {
    let node = [Some(value), value.get("data")]
        .into_iter()
        .flatten()
        .find(|candidate| {
            candidate.get("headEntries").is_some()
                || candidate.get("tailEntries").is_some()
                || candidate.get("entries").is_some()
                || candidate.get("hiddenCount").is_some()
        })?;
    let ids = |key: &str| {
        node.get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        item.get("entryId")
                            .or_else(|| item.get("entry_id"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    Some(ChainWindow {
        chain_id: chain_id.to_string(),
        head_ids: ids("headEntries"),
        tail_ids: ids("tailEntries"),
        hidden_count: integer_or(node, &["hiddenCount", "hidden_count"], 0),
        middle_cursor: string_or(node, &["middleCursor", "middle_cursor"], ""),
        has_more: bool_or(node, &["hasMore", "has_more"], false),
    })
}

/// 解析链摘要的 camelCase/snake_case 双形态字段。
fn parse_summary(value: &Value, chain_id: &str, channel_id: &str) -> Option<ChainSummary> {
    Some(ChainSummary {
        chain_id: string_or(value, &["chainId", "chain_id"], chain_id),
        channel_id: string_or(value, &["channelId", "channel_id"], channel_id),
        mode: string_or(value, &["mode"], "TEXT"),
        status: string_or(value, &["status"], "DRAFT"),
        total_messages: integer_or(value, &["totalMessages", "total_messages"], 0),
        participant_count: integer_or(value, &["participantCount", "participant_count"], 0),
        last_seq: integer_or(value, &["lastSeq", "last_seq"], 0),
        revision: integer_or(value, &["revision"], 0),
    })
}

/// 解析单个接龙条目。
fn parse_entry(value: &Value, chain_id: &str) -> Option<ChainEntry> {
    let entry_id = find_local_string(value, &["entryId", "entry_id"])?;
    Some(ChainEntry {
        entry_id,
        chain_id: string_or(value, &["chainId", "chain_id"], chain_id),
        post_id: string_or(value, &["postId", "post_id"], ""),
        author_id: string_or(value, &["authorId", "author_id", "userId", "user_id"], ""),
        seq: integer_or(value, &["seq"], 0),
        text: string_or(value, &["text", "message"], ""),
        created_at: integer_or(value, &["createdAt", "created_at"], 0),
        status: string_or(value, &["status"], "ACTIVE"),
    })
}

/// 解析 head/tail 窗口；列表字段只接受字符串数组，坏字段按空窗口处理。
fn parse_window(value: &Value, chain_id: &str) -> Option<ChainWindow> {
    Some(ChainWindow {
        chain_id: string_or(value, &["chainId", "chain_id"], chain_id),
        head_ids: string_array(
            value,
            &[
                "headEntryIds",
                "head_entry_ids",
                "headIds",
                "head_ids",
                "headEntries",
            ],
        ),
        tail_ids: string_array(
            value,
            &[
                "tailEntryIds",
                "tail_entry_ids",
                "tailIds",
                "tail_ids",
                "tailEntries",
            ],
        ),
        hidden_count: integer_or(value, &["hiddenCount", "hidden_count"], 0),
        middle_cursor: string_or(value, &["middleCursor", "middle_cursor"], ""),
        has_more: bool_or(value, &["hasMore", "has_more"], false),
    })
}

/// 解析 viewer 游标。
fn parse_viewer(value: &Value, chain_id: &str) -> Option<ChainViewer> {
    Some(ChainViewer {
        chain_id: string_or(value, &["chainId", "chain_id"], chain_id),
        device_id: string_or(value, &["deviceId", "device_id"], ""),
        last_event_seq: integer_or(value, &["lastEventSeq", "last_event_seq", "lastReadSeq"], 0),
        last_read_at: integer_or(value, &["lastReadAt", "last_read_at"], 0),
    })
}

/// 摘要投影为事件 payload。
fn summary_value(value: &ChainSummary) -> Value {
    json!({
        "chainId": value.chain_id,
        "channelId": value.channel_id,
        "mode": value.mode,
        "status": value.status,
        "totalMessages": value.total_messages,
        "participantCount": value.participant_count,
        "lastSeq": value.last_seq,
        "revision": value.revision,
    })
}

/// entry 投影为事件 payload。
fn entry_value(value: &ChainEntry) -> Value {
    json!({
        "entryId": value.entry_id,
        "chainId": value.chain_id,
        "postId": value.post_id,
        "authorId": value.author_id,
        "seq": value.seq,
        "text": value.text,
        "createdAt": value.created_at,
        "status": value.status,
    })
}

/// window 投影为事件 payload。
fn window_value(value: &ChainWindow) -> Value {
    json!({
        "chainId": value.chain_id,
        "headEntryIds": value.head_ids,
        "tailEntryIds": value.tail_ids,
        "hiddenCount": value.hidden_count,
        "middleCursor": value.middle_cursor,
        "hasMore": value.has_more,
    })
}

/// viewer 投影为事件 payload。
fn viewer_value(value: &ChainViewer) -> Value {
    json!({
        "chainId": value.chain_id,
        "deviceId": value.device_id,
        "lastEventSeq": value.last_event_seq,
        "lastReadAt": value.last_read_at,
    })
}

/// 取必填非空字符串。
fn required_string(
    object: &Map<String, Value>,
    key: &str,
    command: &str,
) -> Result<String, ImError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ImError::Parse(format!("{command}: missing {key}")))
}

/// 取可选字符串并拒绝错误类型。
fn optional_string(
    object: &Map<String, Value>,
    key: &str,
    command: &str,
) -> Result<Option<String>, ImError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => Ok(None),
        Some(_) => Err(ImError::Parse(format!("{command}: {key} must be string"))),
    }
}

/// 复制可选字段，不把 null 写进 wire body。
fn copy_optional(body: &mut Map<String, Value>, object: &Map<String, Value>, from: &str, to: &str) {
    if let Some(value) = object.get(from).filter(|value| !value.is_null()) {
        body.insert(to.to_string(), value.clone());
    }
}

/// 复制必填字段并拒绝缺失。
fn copy_required(
    body: &mut Map<String, Value>,
    object: &Map<String, Value>,
    from: &str,
    to: &str,
    command: &str,
) -> Result<(), ImError> {
    let value = object
        .get(from)
        .filter(|value| !value.is_null())
        .cloned()
        .ok_or_else(|| ImError::Parse(format!("{command}: missing {from}")))?;
    body.insert(to.to_string(), value);
    Ok(())
}

/// 字符串 SQL 值 helper。
fn text(value: &str) -> SqlValue {
    SqlValue::Text(value.to_string())
}

/// 整数 SQL 值 helper。
fn integer(value: i64) -> SqlValue {
    SqlValue::Integer(value)
}

/// JSON 数组 SQL 值 helper。
fn json_text<T: serde::Serialize>(value: &T) -> SqlValue {
    SqlValue::Text(serde_json::to_string(value).unwrap_or_else(|_| "[]".to_string()))
}

/// 从字段候选中取本地 string。
fn find_local_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 从字段候选中取 string 或默认值。
fn string_or(value: &Value, keys: &[&str], default: &str) -> String {
    find_local_string(value, keys).unwrap_or_else(|| default.to_string())
}

/// 从字段候选中取整数或默认值。
fn integer_or(value: &Value, keys: &[&str], default: i64) -> i64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_i64))
        .unwrap_or(default)
}

/// 从字段候选中取 bool 或默认值。
fn bool_or(value: &Value, keys: &[&str], default: bool) -> bool {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_bool))
        .unwrap_or(default)
}

/// 从字段候选中取字符串数组。
fn string_array(value: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| {
            value.get(*key).and_then(Value::as_array).map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
        })
        .unwrap_or_default()
}

/// 读取 CommonRes code 的字符串表示。
fn value_code(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .unwrap_or_else(|| "UNKNOWN".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::StorageOp;

    /// 文字接龙 operationId 对同一 mutation 意图保持稳定。
    #[test]
    fn operation_id_is_stable_for_same_mutation() {
        let args = json!({"chain_id":"chain-1","client_mutation_id":"m-1"});
        assert_eq!(
            stable_operation_id("post_chain_append", &args),
            stable_operation_id("post_chain_append", &args)
        );
    }

    /// 创建意图缺少 chainId 时由 Helix 生成可重试复用的聚合身份。
    #[test]
    fn create_derives_chain_id_without_renderer_field() {
        let args = json!({
            "channel_id": "channel-1",
            "client_mutation_id": "m-1",
            "mode": "TEXT"
        });
        let request =
            request_from_command("post_chain_create", &serde_json::to_vec(&args).unwrap())
                .expect("create request");
        assert!(request.chain_id.starts_with("chain-"));
        let (_, body) = wire_body("post_chain_create", &args).expect("create body");
        assert_eq!(body["chainId"], request.chain_id);
    }

    /// TEXT append 的 wire body 只发送最小业务字段并拒绝 categoryIds。
    #[test]
    fn append_wire_body_is_text_only() {
        let args = json!({
            "channel_id":"channel-1",
            "chain_id":"chain-1",
            "text":"hello",
            "attachments":[],
            "client_mutation_id":"m-1",
            "operation_id":"op-1"
        });
        let (_, body) = wire_body("post_chain_append", &args).expect("append body");
        assert_eq!(body["operationId"], "op-1");
        assert!(body.get("categoryIds").is_none());
    }

    /// 发布和已读的可选游标缺省时仍必须生成合法 Go endpoint body。
    #[test]
    fn optional_publish_and_read_cursors_are_not_required() {
        let publish_args = json!({
            "channel_id": "channel-1",
            "chain_id": "chain-1",
            "client_mutation_id": "m-publish"
        });
        let (_, publish_body) =
            wire_body("post_chain_publish", &publish_args).expect("publish body");
        assert_eq!(publish_body["channelId"], "channel-1");
        assert!(publish_body.get("expectedRevision").is_none());

        let read_args = json!({
            "channel_id": "channel-1",
            "chain_id": "chain-1",
            "device_id": "device-1"
        });
        let (_, read_body) = wire_body("post_chain_mark_read", &read_args).expect("read body");
        assert_eq!(read_body["deviceId"], "device-1");
        assert!(read_body.get("lastEventSeq").is_none());
    }

    /// authority 编译必须同时落摘要、entry、window、viewer 与 mutation rows。
    #[test]
    fn authority_compiles_to_chain_projection_ops() {
        let request = ChainRequest {
            command: "post_chain_append".to_string(),
            channel_id: "channel-1".to_string(),
            chain_id: "chain-1".to_string(),
            client_mutation_id: Some("m-1".to_string()),
            operation_id: Some("op-1".to_string()),
            device_id: Some("device-1".to_string()),
            payload: json!({}),
        };
        let authority = parse_authority(
            &json!({
                "chainSummary":{"chainId":"chain-1","channelId":"channel-1","totalMessages":1,"participantCount":1,"lastSeq":1,"revision":1},
                "chainEntry":{"entryId":"entry-1","chainId":"chain-1","seq":1,"text":"hello"},
                "window":{"chainId":"chain-1","headIds":["entry-1"],"tailIds":[],"hiddenCount":0,"hasMore":false},
                "viewer":{"chainId":"chain-1","deviceId":"device-1","lastEventSeq":2,"lastReadAt":3}
            }),
            &request,
            Some("event-1"),
            Some(Seq(2)),
        )
        .expect("authority");
        let ops = persist_ops(
            &authority,
            &request,
            Some(ChainMutationState::Confirmed),
            None,
        );
        assert_eq!(ops.len(), 5);
        assert!(ops
            .iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "chain_summary")));
        let bytes = event_bytes(
            "im:post_chain:upsert",
            &request,
            Some(&authority),
            Some(ChainMutationState::Confirmed),
            None,
        )
        .expect("event bytes");
        let event: Value = serde_json::from_slice(&bytes).expect("event JSON");
        assert_eq!(event["event"], "im:post_chain:upsert");
        assert_eq!(event["data"]["eventId"], "event-1");
        assert_eq!(
            event["data"]["textWindow"]["headEntryIds"],
            json!(["entry-1"])
        );
        assert_eq!(event["data"]["textWindow"]["tailEntryIds"], json!([]));
        assert!(event["data"].get("window").is_none());
    }

    /// 拒绝事件不能写裸 chainId，但必须在 mutation 下保留可路由的聚合身份。
    #[test]
    fn rejected_event_keeps_nested_chain_identity() {
        let request = ChainRequest {
            command: "post_chain_append".to_string(),
            channel_id: "channel-1".to_string(),
            chain_id: "chain-1".to_string(),
            client_mutation_id: Some("m-1".to_string()),
            operation_id: Some("op-1".to_string()),
            device_id: None,
            payload: json!({}),
        };
        let bytes = event_bytes(
            "im:post_chain:append_rejected",
            &request,
            None,
            Some(ChainMutationState::Rejected),
            Some("CHAIN_CLOSED"),
        )
        .expect("rejected event bytes");
        let event: Value = serde_json::from_slice(&bytes).expect("event JSON");
        assert!(event["data"].get("chainId").is_none());
        assert_eq!(event["data"]["mutation"]["chainId"], "chain-1");
        assert_eq!(event["data"]["mutation"]["state"], "REJECTED");
    }

    /// CommonRes 失败码位于 data.code 时仍必须归类为业务拒绝。
    #[test]
    fn nested_common_response_code_is_rejected() {
        let request = ChainRequest {
            command: "post_chain_append".to_string(),
            channel_id: "channel-1".to_string(),
            chain_id: "chain-1".to_string(),
            client_mutation_id: Some("m-1".to_string()),
            operation_id: Some("op-1".to_string()),
            device_id: None,
            payload: json!({}),
        };
        let error = decode_http_authority(
            br#"{"status":"failed","data":{"code":"CHAIN_NOT_PUBLISHED"}}"#,
            &request,
        )
        .expect_err("failed CommonRes must reject");
        assert_eq!(
            error,
            ChainReplyError::Rejected {
                code: "CHAIN_NOT_PUBLISHED".to_string()
            }
        );
    }

    /// WS/HTTP 嵌套 event 节点必须保留 eventId、eventSeq 与 action。
    #[test]
    fn nested_event_authority_preserves_identity_and_action() {
        let authority = authority_from_ws(
            &json!({
                "data": {
                    "channelId": "channel-1",
                    "chainId": "chain-1",
                    "event": {"eventId": "event-1", "eventSeq": 7, "action": "im:post_chain:upsert"},
                    "chainSummary": {"totalMessages": 1, "lastSeq": 1, "revision": 1}
                }
            }),
            None,
        )
        .expect("nested authority");
        assert_eq!(authority.event_id.as_deref(), Some("event-1"));
        assert_eq!(authority.event_seq, Some(Seq(7)));
        assert_eq!(
            authority.event_type.as_deref(),
            Some("im:post_chain:upsert")
        );
    }

    /// Go publish/close 事件把身份放在 chainSummary 时仍必须进入 WS authority gate。
    #[test]
    fn summary_nested_event_authority_preserves_identity() {
        let authority = authority_from_ws(
            &json!({
                "action": "im:post_chain:publish",
                "seq": 9,
                "data": {
                    "eventId": "event-publish-1",
                    "eventSeq": 9,
                    "action": "im:post_chain:publish",
                    "chainSummary": {
                        "chainId": "chain-1",
                        "channelId": "channel-1",
                        "status": "PUBLISHED",
                        "revision": 2
                    }
                }
            }),
            None,
        )
        .expect("summary nested authority");
        assert_eq!(authority.chain_id, "chain-1");
        assert_eq!(authority.channel_id, "channel-1");
        assert_eq!(authority.event_seq, Some(Seq(9)));
        assert_eq!(
            authority.event_type.as_deref(),
            Some("im:post_chain:publish")
        );
    }

    /// Go append/retract 事件携带的 canonical textWindow 必须进入 Helix durable window projection。
    #[test]
    fn canonical_text_window_authority_accepts_entry_id_boundaries() {
        let authority = authority_from_ws(
            &json!({
                "action": "im:post_chain:upsert",
                "data": {
                    "channelId": "channel-1",
                    "chainId": "chain-1",
                    "eventSeq": 8,
                    "event": {
                        "data": {
                            "textWindow": {
                                "chainId": "chain-1",
                                "headEntryIds": ["entry-1"],
                                "tailEntryIds": ["entry-3"],
                                "hiddenCount": 1,
                                "middleCursor": "1",
                                "hasMore": true
                            }
                        }
                    }
                }
            }),
            None,
        )
        .expect("canonical window authority");

        assert_eq!(authority.event_seq, Some(Seq(8)));
        let window = authority.window.expect("window projection");
        assert_eq!(window.head_ids, vec!["entry-1"]);
        assert_eq!(window.tail_ids, vec!["entry-3"]);
        assert_eq!(window.hidden_count, 1);
    }

    /// eventSeq 缺口回放必须把接龙事实和普通消息放进同一个持久写批次。
    #[test]
    fn synced_chain_payload_compiles_projection_and_emit() {
        use crate::state::ChannelId;
        use crate::sync::session::{EventEnvelope, EventKind, PostFields};

        let channel_id = "a9h5hrdsy3873dmg375a6ntqiw";
        let event = EventEnvelope::new(
            ChannelId::from_str(channel_id).expect("valid channel id"),
            Seq(8),
            EventKind::PostUpsert,
            PostFields::default(),
        )
        .with_event_identity(
            Some("event-8".to_string()),
            None,
            1_700_000_000,
            serde_json::json!({
                "data": {
                    "action": "im:post_chain:upsert",
                    "chainId": "chain-1",
                    "clientMutationId": "mutation-8",
                    "operationId": "operation-8",
                    "chainSummary": {
                        "chainId": "chain-1",
                        "mode": "TEXT",
                        "status": "PUBLISHED",
                        "totalMessages": 1,
                        "participantCount": 1,
                        "lastSeq": 1,
                        "revision": 2
                    },
                    "chainEntry": {
                        "entryId": "entry-1",
                        "chainId": "chain-1",
                        "postId": "post-1",
                        "authorId": "user-1",
                        "seq": 1,
                        "text": "hello",
                        "createdAt": 1_700_000_000,
                        "status": "ACTIVE"
                    },
                    "textWindow": {
                        "chainId": "chain-1",
                        "headEntryIds": ["entry-1"],
                        "tailEntryIds": [],
                        "hiddenCount": 0,
                        "middleCursor": "",
                        "hasMore": false
                    }
                }
            })
            .to_string(),
        );

        let projection = synced_projection(&event)
            .expect("chain payload is valid")
            .expect("chain action is recognized");
        assert!(projection
            .ops
            .iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "chain_summary")));
        assert!(projection
            .ops
            .iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "chain_entry")));
        assert!(projection
            .ops
            .iter()
            .any(|op| matches!(op, StorageOp::BatchUpsert(spec) if spec.table == "chain_window")));
        let emitted: Value = serde_json::from_slice(&projection.event).expect("event JSON");
        assert_eq!(emitted["event"], "im:post_chain:upsert");
        assert_eq!(emitted["data"]["eventSeq"], 8);
        assert_eq!(
            emitted["data"]["textWindow"]["headEntryIds"],
            json!(["entry-1"])
        );
    }

    /// 非接龙 channel event 仍由既有消息回放路径处理，不能被 Chain reducer 拦截。
    #[test]
    fn synced_non_chain_payload_is_ignored() {
        use crate::state::ChannelId;
        use crate::sync::session::{EventEnvelope, EventKind, PostFields};

        let event = EventEnvelope::new(
            ChannelId::from_str("a9h5hrdsy3873dmg375a6ntqiw").expect("valid channel id"),
            Seq(9),
            EventKind::PostUpsert,
            PostFields::default(),
        )
        .with_event_identity(None, None, 0, json!({"action": "post"}).to_string());
        assert!(synced_projection(&event)
            .expect("ordinary payload is valid JSON")
            .is_none());
    }

    /// 接龙 action 缺少 authority 时必须失败关闭，不能只推进 channel cursor。
    #[test]
    fn synced_malformed_chain_payload_fails_closed() {
        use crate::state::ChannelId;
        use crate::sync::session::{EventEnvelope, EventKind, PostFields};

        let event = EventEnvelope::new(
            ChannelId::from_str("a9h5hrdsy3873dmg375a6ntqiw").expect("valid channel id"),
            Seq(10),
            EventKind::PostUpsert,
            PostFields::default(),
        )
        .with_event_identity(
            None,
            None,
            0,
            json!({
                "action": "im:post_chain:upsert",
                "channelId": "a9h5hrdsy3873dmg375a6ntqiw"
            })
            .to_string(),
        );
        assert!(synced_projection(&event).is_err());
    }
}
