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
    crate::older_context::LOAD_OLDER_CONTEXT,
];

pub fn query_command_names() -> &'static [&'static str] {
    QUERY_COMMAND_NAMES
}

pub fn is_query(name: &str) -> bool {
    QUERY_COMMAND_NAMES.contains(&name)
}

/// 投影查询单条上限（对齐现网 `clamp(1, 500)`）。
pub(crate) const QUERY_MESSAGES_DEFAULT: u32 = 50;
pub(crate) const QUERY_MESSAGES_MAX: u32 = 500;

/// dialogList 投影单次上限（会话列表全量，宽于消息分页；对齐现网根群可达数千会话上限护栏）。
pub(crate) const QUERY_DIALOGS_DEFAULT: u32 = 500;
pub(crate) const QUERY_DIALOGS_MAX: u32 = 2000;

/// 三端共享的最近消息业务查询；平台 driver 不解释表名、过滤列或响应 shape。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageQueryRequest {
    pub channel_id: ChannelId,
    pub limit: u32,
    /// SurfaceRuntime 的 timeline slot token。旧调用未提供时固定为 `latest`，使兼容入口仍落到
    /// 一个明确、可 attach 的有界窗口，而不是把所有时间线混到 channel 级 scope。
    pub window_token: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DialogListQueryRequest {
    pub limit: u32,
}

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
    let limit = value
        .get("limit")
        .and_then(serde_json::Value::as_u64)
        .map(|value| value.clamp(1, u64::from(QUERY_MESSAGES_MAX)) as u32)
        .unwrap_or(QUERY_MESSAGES_DEFAULT);
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

pub fn message_scan_spec(request: &MessageQueryRequest) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(request.limit),
        filter: Some((
            "channel_id",
            SqlValue::Text(request.channel_id.as_str().to_string()),
        )),
        order_by: MESSAGE_QUERY_ORDER,
    }
}

pub fn parse_dialog_list_query(payload: &[u8]) -> DialogListQueryRequest {
    let limit = serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|value| value.get("limit").and_then(serde_json::Value::as_u64))
        .map(|value| value.clamp(1, u64::from(QUERY_DIALOGS_MAX)) as u32)
        .unwrap_or(QUERY_DIALOGS_DEFAULT);
    DialogListQueryRequest { limit }
}

pub fn dialog_list_scan_spec(request: DialogListQueryRequest) -> ScanSpec {
    ScanSpec {
        table: "channel",
        limit: Some(request.limit),
        filter: None,
        order_by: DIALOG_QUERY_ORDER,
    }
}

const MESSAGE_QUERY_ORDER: &[ScanOrder] = &[ScanOrder::desc("create_at")];
pub(crate) const DIALOG_QUERY_ORDER: &[ScanOrder] = &[
    ScanOrder::desc("is_top"),
    ScanOrder::desc("last_post_at"),
    ScanOrder::desc("created_at"),
];

/// 构造 `im_query_messages_by_channel` 的 `Scan(message WHERE channel_id ORDER BY create_at DESC)`。
///
/// 返回 `(channel_id, Effect::Persist{Scan})`；异步业务入口会注册 local-first correlation，
/// 根据显式 coverage 决定本地投影或 `getLatestPost` fallback。`limit` 取 payload `limit`
/// （缺省 50，clamp ≤500）。
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

/// 构造 `im_query_dialog_list` 的 `Scan(channel ORDER BY is_top,last_post_at,created_at DESC LIMIT)`。
///
/// 返回 `Effect::Persist{Scan}`——调用方（module.rs）alloc corr + 注册
/// `CorrelationContext::DialogListQuery`，PortReply 回报后把整份 channel 行投影为完整
/// `message.left-dialog-list` snapshot frame。`limit` 取 payload `limit`（缺省 500，clamp ≤2000）。
///
/// **无 channel_id 过滤**（会话列表是全量，非按群）：scan 整张 channel 表。置顶优先；同组按
/// `last_post_at DESC, create_at DESC`，所以新建且尚无消息的会话也能稳定进入 LIMIT 窗口。
///
/// **HX-C005**：低频按需调用（off hot-path），整表 scan 合规——见模块头注与 spec06 §2 边界。
pub fn build_dialog_list_query(payload: &[u8], corr: Correlation) -> Result<Effect, ImError> {
    let request = parse_dialog_list_query(payload);

    Ok(Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(dialog_list_scan_spec(request))],
    })
}

/// `im_query_dialog_list` 的唯一生产消费面：把本地 channel scan 提升为完整的
/// `message.left-dialog-list` snapshot frame。行内所有展示判定均在 Helix 完成；客户
/// 端只核验 scope/revision 并按 `orderedChannelIds` 绑定。
pub fn emit_dialog_list_frame_result(
    reply_bytes: &[u8],
    scope: crate::dialog_frame::DialogListScope,
    state: &mut crate::dialog_frame::DialogListFrameState,
) -> Option<Effect> {
    emit_dialog_list_frame_result_with_terminal_channels(
        reply_bytes,
        scope,
        state,
        &std::collections::HashSet::new(),
    )
}

/// Same dialog snapshot projection with the durable type7 terminal marker overlaid from the
/// channel aggregate. The `channel` scan intentionally remains a raw storage read; terminal
/// truth lives in the cursor projection and is applied in Helix before the renderer receives the
/// frame, so a restart cannot reopen a locally confirmed closed channel.
pub fn emit_dialog_list_frame_result_with_terminal_channels(
    reply_bytes: &[u8],
    scope: crate::dialog_frame::DialogListScope,
    state: &mut crate::dialog_frame::DialogListFrameState,
    terminal_channel_ids: &std::collections::HashSet<String>,
) -> Option<Effect> {
    let mut rows = crate::dialog_frame::rows_from_dialog_scan(reply_bytes)?;
    for row in &mut rows {
        if terminal_channel_ids.contains(&row.channel_id) {
            row.state.closed = true;
        }
    }
    state.snapshot(scope, rows)
}

/// 单频道 hydration 写入成功后，仅在已有左侧列表 snapshot 时发布同一 surface 的属性 patch。
/// 没有 snapshot 时不伪造半份列表；下次 `im_query_dialog_list` 会给出权威 snapshot。
pub fn patch_hydrated_dialog_frame(
    channel_bytes: &[u8],
    state: &mut crate::dialog_frame::DialogListFrameState,
) -> Option<Effect> {
    let channel: serde_json::Value = serde_json::from_slice(channel_bytes).ok()?;
    let scope = state.scope()?.clone();
    state.patch_from_channel_update(scope, &channel)
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
#[path = "core_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "core_message_tests.rs"]
mod message_tests;
