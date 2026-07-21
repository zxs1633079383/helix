use crate::error::ImError;
use crate::state::{ChannelId, Seq};
use crate::sync_session::EventEnvelope;

use super::fields::extract_post_fields;
use super::kind::parse_event_kind;

/// 解析 WS 入站帧为 EventEnvelope。
///
/// 期望 JSON 格式：
/// ```json
/// {
///   "event": "posted",
///   "data": {
///     "channel_id": 123,
///     "seq": 11,
///     "event_type": 1,
///     "msg_id": "msg_11",
///     "actor_id": "user_1",
///     "created_at": 1000000,
///     "payload": null
///   }
/// }
/// ```
///
/// 失败时返回 ImError::Parse（外部输入 fail-fast，不 unwrap）。
pub fn parse_inbound(bytes: &[u8]) -> Result<EventEnvelope, ImError> {
    let v: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| ImError::Parse(format!("invalid JSON: {}", e)))?;
    parse_inbound_value(&v)
}

/// `parse_inbound` 的内核：从**已解析好的 root `Value`**（非 `data` 子树）提取 legacy 数值帧
/// 的 `EventEnvelope`。`ws::handlers::legacy_event`（无 action 的 legacy 数值帧）复用此函数，
/// 复用 `WsFrame::parse` 已解析的 root Value，消除第二趟 `from_slice`（痛点 1）。
///
/// **边界等价铁律（重构 A 不可漂移）**：取数源逐字保持 `parse_inbound` 历史行为——
///   - `data` 硬要求存在（缺 → Err），channel_id / event_seq 等取数全部基于 `data`；
///   - `channel_id` 仅 `data["channel_id"]`（snake_case，**无 camelCase fallback**）；
///   - event_seq 四级 fallback：① `data.event_seq` ② `data.props.channel_event_seq`
///     ③ **顶层** `v["channel_event_seq"]`（第③级必须取 root，故入参 = root） ④ `data.seq`；
///   - `extract_post_fields(v)` 传 **root**（不是 data）。
/// 任何「顺手统一成 parse_post_frame 风格」都会让 increment_parse_test 的 fallback 断言语义漂移。
pub fn parse_inbound_value(v: &serde_json::Value) -> Result<EventEnvelope, ImError> {
    let data = v
        .get("data")
        .ok_or_else(|| ImError::Parse("missing 'data' field".to_string()))?;

    let channel_id = data["channel_id"]
        .as_str()
        .and_then(ChannelId::from_str)
        .ok_or_else(|| ImError::Parse("missing/invalid data.channel_id".to_string()))?;

    // event_seq 三级 fallback（设计文档 §1.5，后端注入位置不统一）：
    //   ① data.event_seq（权威，真 Go int64 数值）
    //   ② per-post props.channel_event_seq（posted 帧冗余）
    //   ③ 顶层 channel_event_seq（legacy）
    // 兼容兜底：legacy data.seq（无 action 抽象 wire / 既有测试帧）。
    // 真 event_seq 是 int64 数值（Seq(u64)），非 26 字符 id；零信任只取 as_u64。
    let seq = data["event_seq"]
        .as_u64()
        .or_else(|| data["props"]["channel_event_seq"].as_u64())
        .or_else(|| v["channel_event_seq"].as_u64())
        .or_else(|| data["seq"].as_u64())
        .ok_or_else(|| {
            ImError::Parse(
                "missing event_seq (data.event_seq / props.channel_event_seq / channel_event_seq / data.seq)"
                    .to_string(),
            )
        })?;

    let event_type = data["event_type"].as_u64().unwrap_or(1) as u8;

    if event_type == 7 {
        return Err(ImError::Parse(
            "terminal event type 7 is accepted only through strict sync wire".to_string(),
        ));
    }

    let kind = parse_event_kind(event_type)?;
    // HX-C005：字段提取上移此处（`v` 已解析在手），落库路径零再解析、零回序列化。
    // 边界等价铁律：传 ROOT（不是 data）——与历史 parse_inbound 一字不动。
    let fields = extract_post_fields(v);
    // WS 帧 wire `msg_id`（兼容 msgId）；缺省 None（with_msg_id 内部回退 fields 派生值）。
    let msg_id = data
        .get("msg_id")
        .or_else(|| data.get("msgId"))
        .and_then(|m| m.as_str())
        .map(str::to_string);

    Ok(EventEnvelope::new(channel_id, Seq(seq), kind, fields).with_msg_id(msg_id))
}
