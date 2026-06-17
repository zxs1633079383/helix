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
        let frame = WsFrame::parse(br#"{"action":"posted","data":{"event_seq":7}}"#).unwrap();

        assert_eq!(frame.action(), "posted");
        assert_eq!(
            frame.raw(),
            br#"{"action":"posted","data":{"event_seq":7}}"#
        );
        assert_eq!(frame.data().unwrap()["event_seq"], 7);
    }

    #[test]
    fn parse_legacy_frame_uses_legacy_action_key() {
        let frame = WsFrame::parse(br#"{"data":{"event_seq":8}}"#).unwrap();

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
    fn event_seq_uses_data_then_props_then_root() {
        let data_seq = WsFrame::parse(
            br#"{"action":"posted","channel_event_seq":1,"data":{"event_seq":2,"props":{"channel_event_seq":3}}}"#,
        )
        .unwrap();
        assert_eq!(data_seq.event_seq(), Some(Seq(2)));

        let props_seq = WsFrame::parse(
            br#"{"action":"post_update","channel_event_seq":1,"data":{"props":{"channel_event_seq":3}}}"#,
        )
        .unwrap();
        assert_eq!(props_seq.event_seq(), Some(Seq(3)));

        let root_seq =
            WsFrame::parse(br#"{"action":"posts_update","channel_event_seq":1,"data":{}}"#)
                .unwrap();
        assert_eq!(root_seq.event_seq(), Some(Seq(1)));
    }
}
