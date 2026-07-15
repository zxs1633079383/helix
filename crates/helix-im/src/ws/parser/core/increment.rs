use crate::state::{ChannelId, Seq};
use crate::sync_session::IncrementChannel;

/// 解析 `increment_channel` 帧的 `data` → `IncrementChannel`（边界零信任，非法→None）。
///
/// 按设计文档 §1.3 解析真 Go `IncrementChannel.ToMap()` 的 data：
///   - `id`：26 字符 base32 channelId → `ChannelId::from_str`（**唯一必填**，cursor/路由锚；
///     零信任，非 26 字符→None）。
///   - `lastEventSeq`：int64 服务端当前 max event_seq → `Seq`（作离线同步 v2 cursor 种子）。
///     **真 Go 为 Optional**（types.rs:257 `Option<i64>`，legacy mattermost 9.7.3 的 GetMaxSeq
///     降级场景帧不带此字段，channel_state.rs:345 `unwrap_or(0)`）——缺省回退 `Seq(0)`，**绝不**
///     因缺失整帧丢弃（P0：实测 650 increment 帧因 `?` 早退被全静默丢、落库 0 行）。
///   - `needSync`：bool，**缺省 true**（多拉不漏；false=已追平可省 sync）。
///   - `raw`：`serde_json::to_vec(data)`（供 emit im:channel:increment 透传 mentionList/urgentPostList）。
///
/// 仅 `id` 非法（缺 / 非 26 字符）→ None（边界零信任，不 panic / unwrap）；其余字段缺省回退默认值。
pub fn parse_increment_channel(data: &serde_json::Value) -> Option<IncrementChannel> {
    let channel_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(ChannelId::from_str)?;
    // lastEventSeq 真 Go Optional（GetMaxSeq 降级场景缺失）→ 缺省 Seq(0)，不早退丢帧。
    let last_event_seq = Seq(data
        .get("lastEventSeq")
        .and_then(|v| v.as_u64())
        .unwrap_or(0));
    // needSync 缺省 true：宁可多拉一次 sync，也不漏增量（§1.3 防前端永久 loading）。
    let need_sync = data
        .get("needSync")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let raw = bytes::Bytes::from(serde_json::to_vec(data).ok()?);

    Some(IncrementChannel {
        channel_id,
        last_event_seq,
        need_sync,
        raw,
    })
}
