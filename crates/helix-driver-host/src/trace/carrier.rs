use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const TRACE_CARRIER_JSON_MAX_BYTES: usize = 16 * 1024;
pub const TRACEPARENT_BYTES: usize = 55;
pub const TRACE_BAGGAGE_MAX_BYTES: usize = 8 * 1024;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TraceCarrier {
    pub traceparent: Option<String>,
    pub baggage: Option<String>,
    pub raw_json: Option<String>,
}

impl TraceCarrier {
    pub fn from_json_str(raw: &str) -> Result<Self, TraceCarrierError> {
        if raw.len() > TRACE_CARRIER_JSON_MAX_BYTES {
            return Err(TraceCarrierError::TooLarge {
                field: "trace_json",
                max: TRACE_CARRIER_JSON_MAX_BYTES,
                actual: raw.len(),
            });
        }
        let wire: TraceCarrierWire =
            serde_json::from_str(raw).map_err(|e| TraceCarrierError::Parse(e.to_string()))?;
        let traceparent = match wire.traceparent.filter(|v| !v.is_empty()) {
            Some(value) if value.len() > TRACEPARENT_BYTES => {
                return Err(TraceCarrierError::TooLarge {
                    field: "traceparent",
                    max: TRACEPARENT_BYTES,
                    actual: value.len(),
                });
            }
            Some(value) if parse_trace_id(&value).is_none() => {
                return Err(TraceCarrierError::InvalidTraceparent);
            }
            value => value,
        };
        let baggage = match wire.baggage.filter(|v| !v.is_empty()) {
            Some(value) if value.len() > TRACE_BAGGAGE_MAX_BYTES => {
                return Err(TraceCarrierError::TooLarge {
                    field: "baggage",
                    max: TRACE_BAGGAGE_MAX_BYTES,
                    actual: value.len(),
                });
            }
            value => value,
        };
        Ok(Self {
            traceparent,
            baggage,
            raw_json: Some(raw.to_string()),
        })
    }

    pub fn from_headers(headers: &[(String, String)]) -> Option<Self> {
        let traceparent = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("traceparent"))
            .and_then(|(_, value)| bounded_traceparent(value));
        let baggage = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("baggage"))
            .and_then(|(_, value)| bounded_baggage(value));

        if traceparent.is_none() && baggage.is_none() {
            return None;
        }

        Some(Self {
            traceparent,
            baggage,
            raw_json: None,
        })
    }

    /// 从 Go HTTP 响应的 `X-CSES-Traceparent` 读取服务端 Span 上下文。
    pub fn from_response_headers(headers: &[(String, String)]) -> Option<Self> {
        let traceparent = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("x-cses-traceparent"))
            .and_then(|(_, value)| bounded_traceparent(value));
        let baggage = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("baggage"))
            .and_then(|(_, value)| bounded_baggage(value));

        if traceparent.is_none() && baggage.is_none() {
            return None;
        }

        Some(Self {
            traceparent,
            baggage,
            raw_json: None,
        })
    }

    /// 从 Go WS envelope 的顶层 `tracing` carrier 提取 W3C 上下文。
    ///
    /// 只解析有界 JSON 和标准 traceparent/baggage；业务 data、track id 与消息正文不进入
    /// OTel carrier，避免把业务关联字段误当成 trace context。
    pub fn from_ws_frame(frame: &[u8]) -> Option<Self> {
        if frame.len() > TRACE_CARRIER_JSON_MAX_BYTES {
            return None;
        }
        let envelope = serde_json::from_slice::<WsEnvelopeWire>(frame).ok()?;
        let tracing = envelope.tracing?;
        if tracing.traceparent.is_none() && tracing.baggage.is_none() {
            return None;
        }
        let raw = serde_json::to_string(&tracing).ok()?;
        Self::from_json_str(&raw).ok()
    }
}

#[derive(Debug, Error)]
pub enum TraceCarrierError {
    #[error("trace sidecar JSON parse failed: {0}")]
    Parse(String),
    #[error("{field} exceeds {max} bytes: {actual}")]
    TooLarge {
        field: &'static str,
        max: usize,
        actual: usize,
    },
    #[error("traceparent is not valid W3C version 00 format")]
    InvalidTraceparent,
}

#[derive(Debug, Deserialize, Serialize)]
struct TraceCarrierWire {
    #[serde(default)]
    traceparent: Option<String>,
    #[serde(default)]
    baggage: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WsEnvelopeWire {
    #[serde(default)]
    tracing: Option<TraceCarrierWire>,
}

pub(crate) fn parse_trace_id(traceparent: &str) -> Option<String> {
    if !is_valid_w3c_traceparent(traceparent) {
        return None;
    }
    Some(traceparent[3..35].to_string())
}

fn bounded_traceparent(value: &str) -> Option<String> {
    if value.len() > TRACEPARENT_BYTES || parse_trace_id(value).is_none() {
        return None;
    }
    Some(value.to_string())
}

fn bounded_baggage(value: &str) -> Option<String> {
    if value.is_empty() || value.len() > TRACE_BAGGAGE_MAX_BYTES {
        return None;
    }
    Some(value.to_string())
}

fn is_valid_w3c_traceparent(value: &str) -> bool {
    if value.len() != TRACEPARENT_BYTES {
        return false;
    }
    let bytes = value.as_bytes();
    if &bytes[0..2] != b"00" || bytes[2] != b'-' || bytes[35] != b'-' || bytes[52] != b'-' {
        return false;
    }

    let trace_id = &value[3..35];
    let parent_id = &value[36..52];
    let flags = &value[53..55];
    all_lower_hex(trace_id)
        && all_lower_hex(parent_id)
        && all_lower_hex(flags)
        && !all_zero(trace_id)
        && !all_zero(parent_id)
}

fn all_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn all_zero(value: &str) -> bool {
    value.bytes().all(|b| b == b'0')
}

#[derive(Clone, Debug, Default)]
pub struct CommandTraceQueue {
    inner: Arc<Mutex<VecDeque<Option<TraceCarrier>>>>,
}

impl CommandTraceQueue {
    pub fn push_slot(&self, carrier: Option<TraceCarrier>) {
        self.inner.lock().push_back(carrier);
    }

    pub fn rollback_last(&self) {
        let _ = self.inner.lock().pop_back();
    }

    pub fn pop_next(&self) -> Option<TraceCarrier> {
        self.inner.lock().pop_front().flatten()
    }

    #[cfg(test)]
    pub(super) fn len_for_test(&self) -> usize {
        self.inner.lock().len()
    }
}
