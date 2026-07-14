//! P6 投影/状态查询命令（M2 Tauri 接缝④的 helix-im 侧）。
//!
//! 接管架构表 A：现网 `imDataSource:*` 拦截垫片 + `store_get_messages` 直查语义下沉 Rust。
//! 前端退化为纯渲染（C1/C2）：不再前端 SQL 直查，改 invoke 语义命令 → core 出 `Effect` →
//! driver 兑现 → emit 投影。本文件是这些命令的**薄壳**（解析参数 → 产 Effect / 注册 corr 上下文），
//! 业务 endpoint 不涉（纯本地读，零 HTTP）。
//!
//! ## 命令集（对齐现网 boot.rs 真实注册的投影面）
//!
//! - `im_query_messages_by_channel`：现网 `SELECT * FROM message WHERE channel_id=? ORDER BY
//!   create_at DESC, temporary_id DESC LIMIT ?`（commands.rs，debug-only 原始行 dump）。helix 走
//!   `Scan(message, filter=channel_id, order_desc=create_at, limit)` → PortReply 回报 → 反转最近 N 条
//!   为时间正序 → emit `im:messages:query_result` 投影（`data: { channel_id, messages }`）。
//!   - **保真偏差（已记 P7 finalize）**：现网二级排序 `, temporary_id DESC` 同毫秒簇确定化未携带
//!     （`ScanSpec.order_desc_col` 当前单列）——投影 dump 用途下同毫秒次序非语义关键；scroll 游标
//!     分页（beforeId/afterId）是 `im_load_older_messages`（HTTP postContext）另一路，不在本投影命令。
//! - `im_delete_all_dialogs`：现网 `DELETE FROM channel` + `DELETE FROM channel_member`（测试用）。
//!   - **核原语缺口（已记 P7 blocker）**：core 无无条件 truncate 原语（`BatchDelete` 需 scope+IN
//!     列表，表达不了「删全表」）。本命令清 **core 内存 channel gate 状态** + emit `im:channels:loaded`
//!     刷新；物理 `DELETE FROM` 留待 truncate 原语补齐（不为测试命令现造原语，YAGNI/HX-C001）。
//!
//! `im_get_state` **不实现**：现网 boot.rs 无此命令（rv-tauri B1「漏项」claim 与 ground truth 不符，
//! 已于 P7 finalize 勘误）——凭空补会引入前端不调用的死命令。

use bytes::Bytes;
use helix_core::effect::{Correlation, DomainEventBytes, Effect, ScanSpec, SqlValue, StorageOp};

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
const QUERY_MESSAGES_DEFAULT: u32 = 50;
const QUERY_MESSAGES_MAX: u32 = 500;

/// dialogList 投影单次上限（会话列表全量，宽于消息分页；对齐现网根群可达数千会话上限护栏）。
const QUERY_DIALOGS_DEFAULT: u32 = 500;
const QUERY_DIALOGS_MAX: u32 = 2000;

/// 构造 `im_query_messages_by_channel` 的 `Scan(message WHERE channel_id ORDER BY create_at DESC)`。
///
/// 返回 `(channel_id, Effect::Persist{Scan})`——调用方（module.rs）alloc corr + 注册
/// `CorrelationContext::MessageQuery{channel_id}`，PortReply 回报后 [`emit_message_query_result`]
/// 把行投影成 emit。`limit` 取 payload `limit`（缺省 50，clamp ≤500）。
pub fn build_message_query(
    payload: &[u8],
    corr: Correlation,
) -> Result<(ChannelId, Effect), ImError> {
    let v: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_query_messages_by_channel payload: {e}")))?;
    let channel_str = v
        .get("channel_id")
        .and_then(|c| c.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ImError::Parse("im_query_messages_by_channel 缺 channel_id".into()))?;
    let channel_id = ChannelId::from_str(channel_str)
        .ok_or_else(|| ImError::Parse(format!("非法 channel_id: {channel_str}")))?;
    let limit = v
        .get("limit")
        .and_then(|l| l.as_u64())
        .map(|l| (l as u32).clamp(1, QUERY_MESSAGES_MAX))
        .unwrap_or(QUERY_MESSAGES_DEFAULT);

    let effect = Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(ScanSpec {
            table: "message",
            limit: Some(limit),
            // 等值过滤 channel_id（绑定参数，防注入）+ create_at 降序（取最近 N 条，对齐现网）。
            filter: Some(("channel_id", SqlValue::Text(channel_str.to_string()))),
            order_desc_col: Some("create_at"),
        })],
    };
    Ok((channel_id, effect))
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
            "channel_id": channel_id.as_str(),
            "messages": crate::render_ready::shape_message_rows_for_viewer(&rows, viewer_user_id),
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_message_query_result: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 构造 `im_query_dialog_list` 的 `Scan(channel ORDER BY last_post_at DESC LIMIT)`。
///
/// 返回 `Effect::Persist{Scan}`——调用方（module.rs）alloc corr + 注册
/// `CorrelationContext::DialogListQuery`，PortReply 回报后 [`emit_dialog_list_result`] 把整份
/// channel 行投影成完整 dialogList emit。`limit` 取 payload `limit`（缺省 500，clamp ≤2000）。
///
/// **无 channel_id 过滤**（会话列表是全量，非按群）：scan 整张 channel 表。`last_post_at DESC`
/// 让最近活跃会话排前（对齐现网 dialogList 排序），前端零再排序。
///
/// **HX-C005**：低频按需调用（off hot-path），整表 scan 合规——见模块头注与 spec06 §2 边界。
pub fn build_dialog_list_query(payload: &[u8], corr: Correlation) -> Result<Effect, ImError> {
    // 零信任：payload 可缺省（前端 invoke 可不带参全量拉）。非法 JSON → 走默认 limit（不 panic）。
    let limit = serde_json::from_slice::<serde_json::Value>(payload)
        .ok()
        .and_then(|v| v.get("limit").and_then(|l| l.as_u64()))
        .map(|l| (l as u32).clamp(1, QUERY_DIALOGS_MAX))
        .unwrap_or(QUERY_DIALOGS_DEFAULT);

    Ok(Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(ScanSpec {
            table: "channel",
            limit: Some(limit),
            // 全量会话列表：无等值过滤；按最近活跃（last_post_at）降序，前端零再排序。
            filter: None,
            order_desc_col: Some("last_post_at"),
        })],
    })
}

/// PortReply 回报（`DialogListQuery` 上下文 Ok）→ emit `im:channels:projection` 完整 dialogList 投影。
///
/// `reply_bytes` 是 engine 把 Scan rows 序列化的 JSON 数组 `[{col:val}, ...]`（channel 行，
/// 列名 = projection-schema §4 DB 列）。整组行作 `data.dialogList`——**完整 DialogItem 行，含
/// `unread_count`/`mention_count`/`mention_count_root`/`last_post`/`unread_post_id`/`last_event_seq`
/// 等渲染所需全字段**，前端直接渲染会话列表 + badge，**零计算零回读**（UC-3 unread 计算下放达成）。
///
/// 零信任：非法 JSON → 空 dialogList（前端拿空列表，不卡渲染）。
///
/// S5（issue #54·C013 业务规则下沉 helix·前端纯绑定）：每行升 **render-ready 终态 CL 行**——
/// 在保留原始 DB 列（向后兼容）之上，补 8 个 render-ready 派生键供前端 1:1 绑定：
/// `displayName`/`notice`(归一)/`isTop`(bool)/`createAt`/`unread`(终值)/`mention`(bool)/
/// `lastMessage`(预览)/`urgent`(bool)。前端壳删 normalizeNotice/normalizeIsTop/unread++ shaping。
pub fn emit_dialog_list_result(reply_bytes: &[u8]) -> Effect {
    let raw: serde_json::Value =
        serde_json::from_slice(reply_bytes).unwrap_or_else(|_| serde_json::json!([]));
    let rows: serde_json::Value = match raw {
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .into_iter()
                .map(crate::query::dialog_render::render_ready_dialog_row)
                .collect(),
        ),
        _ => serde_json::json!([]),
    };
    let payload = serde_json::json!({
        "event": "im:channels:projection",
        "data": {
            "dialogList": rows,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_dialog_list_result: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 单频道 hydration 的即时 render-ready 投影。
///
/// `incrementByChannelId` 已返回完整 channel wire 数据时，先发布这一行解除前端 unknown 状态；
/// terminal sync 后仍会执行 [`emit_dialog_list_result`] 全量读回，以本地持久化终态覆盖即时值。
pub fn emit_hydrated_dialog_result(channel_bytes: &[u8]) -> Option<Effect> {
    let channel: serde_json::Value = serde_json::from_slice(channel_bytes).ok()?;
    let dialog = crate::query::dialog_wire::render_ready_dialog_wire_row(channel)?;
    let payload = serde_json::json!({
        "event": "im:channels:projection",
        "data": {
            "dialogList": [dialog],
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_hydrated_dialog_result: static JSON shape must serialize"),
    );
    Some(Effect::Emit {
        event: DomainEventBytes(bytes),
    })
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
