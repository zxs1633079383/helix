use bytes::Bytes;

use crate::error::ImError;
use crate::state::Seq;

pub(crate) const LEGACY_EVENT_ACTION: &str = "__legacy_event__";

/// 合成 action：心跳 pong ACK 帧（`status==OK` + 带 `gaps`/`hashMismatch`，**无** `action` 字段）。
///
/// 现网 cses-client 的 pong 走 runtime `ack_observer`（status==OK 被心跳校验吞，不进 handler）。
/// helix sans-IO 无 runtime 吞帧——pong 作 Text JSON 经 `Tick::Inbound` 进 core。为复用 registry
/// 的 by-action 路由（不污染 dispatch），`WsFrame::action()` 把这类无 action 的 pong 帧合成此
/// 常量 → `ws::handlers::pong` 接管 gap 补偿（Round-3 UC-4.4 消费侧补齐）。
pub(crate) const PONG_ACTION: &str = "__pong__";

pub(crate) struct WsFrame {
    raw: Bytes,
    root: serde_json::Value,
}

impl WsFrame {
    pub(crate) fn parse(raw: &[u8]) -> Result<Self, ImError> {
        let root: serde_json::Value = serde_json::from_slice(raw)
            .map_err(|err| ImError::InvalidWsFrame(format!("invalid JSON: {err}")))?;

        Ok(Self {
            raw: Bytes::copy_from_slice(raw),
            root,
        })
    }

    /// WS-1：惰性从已解析 root 取 action `&str`——删 action 字段，不每帧堆分配 String（HX-C005
    /// `&'static str` 字面纪律）。借用 `&self.root`，唯一消费方在同步 dispatch 内，借用安全。
    ///
    /// 显式 `action` 字段优先；**无 action** 时若帧形如心跳 pong ACK（`status=="OK"` 且 `data`
    /// 带 `gaps`/`hashMismatch`）→ 合成 `PONG_ACTION`（UC-4.4 消费侧路由）；其余无 action 帧
    /// 仍回退 `LEGACY_EVENT_ACTION`（legacy 数值帧）。
    pub(crate) fn action(&self) -> &str {
        if let Some(action) = self.root.get("action").and_then(serde_json::Value::as_str) {
            return action;
        }
        if self.is_pong_frame() {
            return PONG_ACTION;
        }
        LEGACY_EVENT_ACTION
    }

    /// 心跳 pong ACK 判定（零信任）：`status=="OK"` 且 `data` 含 `gaps` 或 `hashMismatch`。
    /// 严格判定避免误吞普通 status==OK 裸 ACK（无 gap 信息的 pong 无补偿动作，路由到此也是
    /// no-op，但显式带补偿字段才合成 action，语义更收敛）。
    fn is_pong_frame(&self) -> bool {
        let status_ok = self.root.get("status").and_then(serde_json::Value::as_str) == Some("OK");
        if !status_ok {
            return false;
        }
        self.data()
            .map(|d| d.get("gaps").is_some() || d.get("hashMismatch").is_some())
            .unwrap_or(false)
    }

    pub(crate) fn raw(&self) -> &[u8] {
        &self.raw
    }

    pub(crate) fn root(&self) -> &serde_json::Value {
        &self.root
    }

    pub(crate) fn data(&self) -> Option<&serde_json::Value> {
        self.root.get("data")
    }

    pub(crate) fn data_required(&self) -> Result<&serde_json::Value, ImError> {
        self.data()
            .ok_or_else(|| ImError::InvalidWsFrame("missing data".to_string()))
    }

    pub(crate) fn event_seq(&self) -> Option<Seq> {
        self.data()
            .and_then(|data| data.get("event_seq"))
            .and_then(serde_json::Value::as_u64)
            .or_else(|| {
                self.data()
                    .and_then(|data| data.get("props"))
                    .and_then(|props| props.get("channel_event_seq"))
                    .and_then(serde_json::Value::as_u64)
            })
            .or_else(|| {
                self.root
                    .get("channel_event_seq")
                    .and_then(serde_json::Value::as_u64)
            })
            .or_else(|| {
                self.data()
                    .and_then(|data| data.get("seq"))
                    .and_then(serde_json::Value::as_u64)
            })
            .or_else(|| {
                self.root
                    .get("eventSeq")
                    .and_then(serde_json::Value::as_u64)
            })
            .or_else(|| {
                self.root
                    .get("event")
                    .and_then(|event| event.get("seq"))
                    .and_then(serde_json::Value::as_u64)
            })
            .or_else(|| {
                self.root
                    .get("payload")
                    .and_then(|payload| payload.get("eventSeq"))
                    .and_then(serde_json::Value::as_u64)
            })
            .map(Seq)
    }

    pub(crate) fn event_seq_required(&self) -> Result<Seq, ImError> {
        self.event_seq()
            .ok_or_else(|| ImError::InvalidWsFrame("missing event_seq".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_action_frame_reads_top_level_action() {
        let frame = WsFrame::parse(br#"{"action":"posted","eventSeq":7}"#).unwrap();

        assert_eq!(frame.action(), "posted");
        assert_eq!(frame.raw(), br#"{"action":"posted","eventSeq":7}"#);
    }

    #[test]
    fn parse_payload_action_when_top_level_action_missing() {
        let frame = WsFrame::parse(br#"{"payload":{"action":"posted"}}"#).unwrap();

        assert_eq!(frame.action(), LEGACY_EVENT_ACTION);
    }

    #[test]
    fn parse_legacy_frame_uses_legacy_action_key() {
        let frame = WsFrame::parse(br#"{"payload":{"eventSeq":8}}"#).unwrap();

        assert_eq!(frame.action(), LEGACY_EVENT_ACTION);
    }

    #[test]
    fn parse_invalid_json_returns_invalid_ws_frame() {
        let err = match WsFrame::parse(br#"{"action":"posted""#) {
            Ok(_) => panic!("invalid JSON should fail"),
            Err(err) => err,
        };

        assert!(matches!(err, ImError::InvalidWsFrame(_)));
    }

    #[test]
    fn event_seq_uses_legacy_wire_priority_before_compatibility_fields() {
        let data_seq = WsFrame::parse(
            br#"{"action":"posted","eventSeq":2,"event":{"seq":3},"payload":{"eventSeq":4},"channel_event_seq":5,"data":{"event_seq":6,"props":{"channel_event_seq":7}}}"#,
        )
        .unwrap();
        assert_eq!(data_seq.event_seq(), Some(Seq(6)));

        let props_seq = WsFrame::parse(
            br#"{"action":"post_update","eventSeq":2,"event":{"seq":3},"payload":{"eventSeq":4},"channel_event_seq":5,"data":{"props":{"channel_event_seq":7}}}"#,
        )
        .unwrap();
        assert_eq!(props_seq.event_seq(), Some(Seq(7)));

        let root_seq = WsFrame::parse(
            br#"{"action":"posts_update","eventSeq":2,"event":{"seq":3},"payload":{"eventSeq":4},"channel_event_seq":5,"data":{}}"#,
        )
        .unwrap();
        assert_eq!(root_seq.event_seq(), Some(Seq(5)));
    }

    #[test]
    fn event_seq_uses_compatibility_fields_after_legacy_wire_fields() {
        let event_seq = WsFrame::parse(
            br#"{"action":"post_update","eventSeq":2,"event":{"seq":3},"payload":{"eventSeq":4}}"#,
        )
        .unwrap();
        assert_eq!(event_seq.event_seq(), Some(Seq(2)));

        let event_dot_seq = WsFrame::parse(
            br#"{"action":"post_update","event":{"seq":3},"payload":{"eventSeq":4}}"#,
        )
        .unwrap();
        assert_eq!(event_dot_seq.event_seq(), Some(Seq(3)));

        let payload_seq =
            WsFrame::parse(br#"{"action":"posts_update","payload":{"eventSeq":4}}"#).unwrap();
        assert_eq!(payload_seq.event_seq(), Some(Seq(4)));
    }
}
