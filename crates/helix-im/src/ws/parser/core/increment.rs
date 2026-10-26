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
///   - `unread_post_id` / `last_read_seq` / `projection_revision`：当前 viewer 的可选成员投影，
///     同一帧带正 revision 时由 strict member guard 共同落库；显式非法整帧拒绝，避免退化为 legacy。
///   - `raw`：`serde_json::to_vec(data)`（供 emit im:channel:increment 透传 mentionList/urgentPostList）。
///
/// `id` 非法（缺 / 非 26 字符）或显式 projection 整数字段非法 → None（边界零信任，不 panic）；
/// 其余字段缺省回退默认值。
pub fn parse_increment_channel(data: &serde_json::Value) -> Option<IncrementChannel> {
    let channel_id = data
        .get("id")
        .and_then(|v| v.as_str())
        .and_then(ChannelId::from_str)?;
    // Validate every supplied alias before any channel/member persistence is compiled.
    for key in ["unreadCount", "unread_count"] {
        if let Some(value) = data.get(key) {
            if value.as_i64().filter(|count| *count >= 0).is_none() {
                return None;
            }
        }
    }
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
    let unread_post_id = ["unread_post_id", "unreadPostId", "unReadPostId"]
        .into_iter()
        .find_map(|key| data.get(key))
        .and_then(|value| value.as_str().map(str::to_owned));
    let last_read_seq = match ["last_read_seq", "lastReadSeq"]
        .into_iter()
        .find_map(|key| data.get(key))
    {
        None => None,
        Some(value) => {
            let read_seq = parse_i64(value)?;
            if read_seq < 0 {
                return None;
            }
            Some(read_seq)
        }
    };
    let projection_revision = match ["projection_revision", "projectionRevision"]
        .into_iter()
        .find_map(|key| data.get(key))
    {
        None => None,
        Some(value) => Some(parse_u64(value)?),
    };
    let raw = bytes::Bytes::from(serde_json::to_vec(data).ok()?);

    Some(IncrementChannel {
        channel_id,
        last_event_seq,
        need_sync,
        unread_post_id,
        last_read_seq,
        projection_revision,
        raw,
    })
}

/// Decode an optional wire integer without turning malformed input into a legacy omission.
fn parse_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|number| number.parse::<i64>().ok()))
}

/// Decode a non-negative wire integer; callers use the outer `Option` to reject malformed keys.
fn parse_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|number| number.parse::<u64>().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_canonical_unread_projection_fields_and_preserves_empty_anchor() {
        let data = serde_json::json!({
            "id": "bi643wys7fgy9pbai9wjfusnfa",
            "lastEventSeq": 151,
            "needSync": false,
            "unread_post_id": "",
            "last_read_seq": 41,
            "projection_revision": 7
        });
        let parsed = parse_increment_channel(&data).expect("valid increment frame");
        assert_eq!(parsed.unread_post_id.as_deref(), Some(""));
        assert_eq!(parsed.last_read_seq, Some(41));
        assert_eq!(parsed.projection_revision, Some(7));
    }

    #[test]
    fn malformed_versioned_integer_is_rejected_instead_of_becoming_legacy() {
        let data = serde_json::json!({
            "id": "bi643wys7fgy9pbai9wjfusnfa",
            "projectionRevision": "not-a-number"
        });
        assert!(parse_increment_channel(&data).is_none());
    }

    /// Invalid absolute counts must not reach either channel or member persistence.
    #[test]
    fn projection_boundary_rejects_invalid_unread_counts() {
        for key in ["unreadCount", "unread_count"] {
            for value in [
                serde_json::json!(-1),
                serde_json::json!("bad"),
                serde_json::Value::Null,
            ] {
                let mut data = serde_json::json!({"id": "bi643wys7fgy9pbai9wjfusnfa"});
                data[key] = value;
                assert!(parse_increment_channel(&data).is_none(), "{data}");
            }
        }
    }
}
