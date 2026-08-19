use bytes::Bytes;

use crate::error::ImError;
use crate::state::Seq;

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
    /// 显式 `action` 字段优先；接龙 action 归一化到共享 handler，无 action 的心跳 ACK 合成
    /// `PONG_ACTION`，其余帧拒绝。
    pub(crate) fn action(&self) -> Result<&str, ImError> {
        if let Some(action) = self.root.get("action").and_then(serde_json::Value::as_str) {
            if action.starts_with("im:post_chain:") {
                return Ok("post_chain");
            }
            return Ok(action);
        }
        if self
            .root
            .get("eventType")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|event_type| event_type.starts_with("im:post_chain:"))
        {
            return Ok("post_chain");
        }
        if self.is_pong_frame() {
            return Ok(PONG_ACTION);
        }
        Err(ImError::InvalidWsFrame("missing action".to_string()))
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

    /// Request correlation is trusted only from the server envelope. Post payload/props values
    /// are deliberately ignored so renderer-controlled content cannot forge action completion.
    pub(crate) fn cses_track_id(&self) -> Option<&str> {
        let value = self
            .root
            .get("tracing")?
            .get("csesTrackId")?
            .as_str()?
            .trim();
        (!value.is_empty() && value.len() <= 64).then_some(value)
    }

    pub(crate) fn event_seq(&self) -> Option<Seq> {
        self.data()
            .and_then(|data| data.get("event_seq"))
            .and_then(serde_json::Value::as_u64)
            .or_else(|| {
                self.data()
                    .and_then(|data| data.get("eventSeq"))
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
            .or_else(|| {
                self.data()
                    .and_then(|data| data.get("seq"))
                    .and_then(serde_json::Value::as_u64)
            })
            .or_else(|| self.root.get("seq").and_then(serde_json::Value::as_u64))
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

    /// Go 的 canonical chain envelope 带 im:post_chain:* action 时必须路由到共享 handler。
    #[test]
    fn normalizes_post_chain_action_and_camel_event_seq() {
        let frame = WsFrame::parse(br#"{"action":"im:post_chain:upsert","data":{"eventSeq":17}}"#)
            .expect("chain frame");

        assert_eq!(frame.action().expect("chain action"), "post_chain");
        assert_eq!(frame.event_seq(), Some(Seq(17)));
    }

    /// WebSocketEventJSON 的 envelope sequence 位于根 seq 时仍必须进入 channel gate。
    #[test]
    fn reads_root_sequence_from_canonical_event_envelope() {
        let frame =
            WsFrame::parse(br#"{"action":"im:post_chain:upsert","seq":23}"#).expect("chain frame");

        assert_eq!(frame.event_seq(), Some(Seq(23)));
    }
}
