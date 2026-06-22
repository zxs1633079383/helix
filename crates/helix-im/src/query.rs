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
//!   `Scan(message, filter=channel_id, order_desc=create_at, limit)` → PortReply 回报 → emit
//!   `im:messages:query_result` 投影（data 数组，driver `{items:[]}` 包装）。
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
pub fn is_query(name: &str) -> bool {
    matches!(
        name,
        "im_query_messages_by_channel" | "im_delete_all_dialogs"
    )
}

/// 投影查询单条上限（对齐现网 `clamp(1, 500)`）。
const QUERY_MESSAGES_DEFAULT: u32 = 50;
const QUERY_MESSAGES_MAX: u32 = 500;

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
/// driver-host codec）。整组行作 `data`（数组）→ driver `guard_bus_envelope` 把数组包成 `{items:[]}`
/// （M4 横切，本文件不做包装——单一包装点在 driver 出口）。零信任：非法 JSON → 空数组投影（不 panic）。
pub fn emit_message_query_result(channel_id: &ChannelId, reply_bytes: &[u8]) -> Effect {
    let rows: serde_json::Value =
        serde_json::from_slice(reply_bytes).unwrap_or_else(|_| serde_json::json!([]));
    let payload = serde_json::json!({
        "event": "im:messages:query_result",
        "data": {
            "channel_id": channel_id.as_str(),
            "messages": rows,
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
mod tests {
    use super::*;

    fn corr() -> Correlation {
        Correlation::from_raw(42)
    }

    /// 取 Effect::Persist 的第一条 Scan spec（Option：非该形态 → None，调用方 `.expect`）。
    /// 不含 never-taken panic 分支——以 `matches!`+`if let` 收敛，covered。
    fn scan_of(effect: Effect) -> Option<ScanSpec> {
        if let Effect::Persist { ops, .. } = effect {
            if let Some(StorageOp::Scan(s)) = ops.into_iter().next() {
                return Some(s);
            }
        }
        None
    }

    /// 取 Effect::Emit 载荷并解成 JSON（Option：非 Emit → None，调用方 `.expect`）。
    fn emit_json(effect: Effect) -> Option<serde_json::Value> {
        if let Effect::Emit { event } = effect {
            return serde_json::from_slice(event.0.as_ref()).ok();
        }
        None
    }

    /// 覆盖 helper 的 None 分支（消除 never-taken 行）：传错 variant → None。
    #[test]
    fn helpers_return_none_on_wrong_variant() {
        // emit_dialogs_cleared 是 Emit，非 Persist → scan_of None。
        assert!(scan_of(emit_dialogs_cleared()).is_none());
        // build_message_query 产 Persist，非 Emit → emit_json None。
        let p = serde_json::to_vec(&serde_json::json!({"channel_id":"ch00000000000000000000000a"}))
            .unwrap();
        let persist = build_message_query(&p, corr()).unwrap().1;
        assert!(emit_json(persist).is_none());
        // Persist 但首 op 非 Scan（空 ops）→ scan_of None（覆盖内层 if let 假分支）。
        let empty_persist = Effect::Persist {
            corr: corr(),
            ops: vec![],
        };
        assert!(scan_of(empty_persist).is_none());
    }

    #[test]
    fn is_query_recognizes_projection_commands() {
        assert!(is_query("im_query_messages_by_channel"));
        assert!(is_query("im_delete_all_dialogs"));
        assert!(!is_query("im_send_message"));
        assert!(!is_query("im_create_channel"));
    }

    #[test]
    fn build_message_query_scans_message_with_channel_filter_and_desc_order() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "channel_id": "ch00000000000000000000000a",
            "limit": 30
        }))
        .unwrap();
        let (cid, effect) = build_message_query(&payload, corr()).expect("build ok");
        assert_eq!(cid.as_str(), "ch00000000000000000000000a");
        let spec = scan_of(effect).expect("Persist+Scan");
        assert_eq!(spec.table, "message");
        assert_eq!(spec.limit, Some(30));
        // 过滤列 = channel_id（等值绑定）；降序列 = create_at（取最近 N 条，对齐现网 ORDER BY DESC）。
        assert!(matches!(
            &spec.filter,
            Some(("channel_id", SqlValue::Text(s))) if s == "ch00000000000000000000000a"
        ));
        assert_eq!(spec.order_desc_col, Some("create_at"));
    }

    #[test]
    fn build_message_query_defaults_and_clamps_limit() {
        // 缺 limit → 默认 50。
        let p1 =
            serde_json::to_vec(&serde_json::json!({"channel_id":"ch00000000000000000000000a"}))
                .unwrap();
        let (_, e1) = build_message_query(&p1, corr()).unwrap();
        assert_eq!(
            scan_of(e1).expect("scan e1").limit,
            Some(QUERY_MESSAGES_DEFAULT)
        );
        // limit 超 500 → clamp 500。
        let p2 = serde_json::to_vec(
            &serde_json::json!({"channel_id":"ch00000000000000000000000a","limit":9999}),
        )
        .unwrap();
        let (_, e2) = build_message_query(&p2, corr()).unwrap();
        assert_eq!(
            scan_of(e2).expect("scan e2").limit,
            Some(QUERY_MESSAGES_MAX)
        );
    }

    #[test]
    fn build_message_query_rejects_missing_channel_id() {
        let payload = serde_json::to_vec(&serde_json::json!({"limit": 10})).unwrap();
        assert!(matches!(
            build_message_query(&payload, corr()),
            Err(ImError::Parse(_))
        ));
        // 非法 JSON → Err 不 panic（边界零信任）。
        assert!(matches!(
            build_message_query(b"not json{", corr()),
            Err(ImError::Parse(_))
        ));
        // 非空但非法 Id26（长度不符）→ from_str None → Err（覆盖「非法 channel_id」分支）。
        let bad = serde_json::to_vec(&serde_json::json!({"channel_id": "too_short"})).unwrap();
        assert!(matches!(
            build_message_query(&bad, corr()),
            Err(ImError::Parse(_))
        ));
    }

    #[test]
    fn emit_message_query_result_wraps_rows_under_messages() {
        let cid = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let rows = serde_json::to_vec(&serde_json::json!([
            {"temporary_id":"t1","message":"hi"},
            {"temporary_id":"t2","message":"yo"}
        ]))
        .unwrap();
        let v = emit_json(emit_message_query_result(&cid, &rows)).expect("emit");
        assert_eq!(v["event"], "im:messages:query_result");
        assert_eq!(v["data"]["channel_id"], "ch00000000000000000000000a");
        assert_eq!(v["data"]["messages"][0]["temporary_id"], "t1");
        assert_eq!(v["data"]["messages"][1]["message"], "yo");
    }

    #[test]
    fn emit_message_query_result_invalid_reply_yields_empty_messages() {
        let cid = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let v = emit_json(emit_message_query_result(&cid, b"garbage{")).expect("emit");
        assert_eq!(v["data"]["messages"], serde_json::json!([]));
    }

    #[test]
    fn emit_dialogs_cleared_signals_channels_loaded() {
        let v = emit_json(emit_dialogs_cleared()).expect("emit");
        assert_eq!(v["event"], "im:channels:loaded");
        assert_eq!(v["data"]["reason"], "delete_all_dialogs");
    }
}
