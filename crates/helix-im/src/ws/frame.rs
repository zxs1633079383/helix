use bytes::Bytes;

use crate::error::ImError;
use crate::state::Seq;

pub(crate) const LEGACY_EVENT_ACTION: &str = "__legacy_event__";

pub(crate) struct WsFrame {
    raw: Bytes,
    root: serde_json::Value,
    action: String,
}

impl WsFrame {
    pub(crate) fn parse(raw: &[u8]) -> Result<Self, ImError> {
        let root: serde_json::Value = serde_json::from_slice(raw)
            .map_err(|err| ImError::InvalidWsFrame(format!("invalid JSON: {err}")))?;
        let action = root
            .get("action")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                root.get("payload")
                    .and_then(|payload| payload.get("action"))
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or(LEGACY_EVENT_ACTION)
            .to_string();

        Ok(Self {
            raw: Bytes::copy_from_slice(raw),
            root,
            action,
        })
    }

    pub(crate) fn action(&self) -> &str {
        &self.action
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
        self.root
            .get("eventSeq")
            .and_then(serde_json::Value::as_u64)
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
            .or_else(|| {
                self.data()
                    .and_then(|data| data.get("event_seq"))
                    .and_then(serde_json::Value::as_u64)
            })
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

        assert_eq!(frame.action(), "posted");
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
    fn event_seq_uses_event_seq_then_event_dot_seq_then_payload_event_seq() {
        let data_seq = WsFrame::parse(
            br#"{"action":"posted","eventSeq":2,"event":{"seq":3},"payload":{"eventSeq":4}}"#,
        )
        .unwrap();
        assert_eq!(data_seq.event_seq(), Some(Seq(2)));

        let event_seq = WsFrame::parse(
            br#"{"action":"post_update","event":{"seq":3},"payload":{"eventSeq":4}}"#,
        )
        .unwrap();
        assert_eq!(event_seq.event_seq(), Some(Seq(3)));

        let payload_seq =
            WsFrame::parse(br#"{"action":"posts_update","payload":{"eventSeq":4}}"#).unwrap();
        assert_eq!(payload_seq.event_seq(), Some(Seq(4)));
    }

    #[test]
    fn event_seq_keeps_legacy_fallback_after_acceptance_fields() {
        let frame = WsFrame::parse(
            br#"{"action":"post_update","channel_event_seq":1,"data":{"props":{"channel_event_seq":3}}}"#,
        )
        .unwrap();

        assert_eq!(frame.event_seq(), Some(Seq(3)));
    }
}
