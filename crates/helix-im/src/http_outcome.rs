//! IM HTTP 结果到业务事件的纯策略。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, HttpResponse};
use helix_core::PortError;
use serde_json::{json, Value};

/// HTTP 鉴权失效事件。
pub const EVENT_HTTP_UNAUTHORIZED: &str = "im:http:unauthorized";
/// 网络不可用事件。
pub const EVENT_NET_OFFLINE: &str = "im:net:offline";

/// 把平台无关的 HTTP 结果映射为 IM 业务事件。
///
/// 未命中业务规则时返回 `None`；driver 只负责调用与投递，不认识事件名或 payload。
pub fn http_outcome_event(
    url: &str,
    outcome: &Result<HttpResponse, PortError>,
) -> Option<DomainEventBytes> {
    match outcome {
        Ok(response) if response.status == 401 => encode_event(
            EVENT_HTTP_UNAUTHORIZED,
            json!({ "status": response.status, "url": url }),
        ),
        Err(PortError::Transport(_)) => encode_event(EVENT_NET_OFFLINE, json!({ "url": url })),
        _ => None,
    }
}

fn encode_event(event: &'static str, data: Value) -> Option<DomainEventBytes> {
    serde_json::to_vec(&json!({ "event": event, "data": data }))
        .ok()
        .map(|bytes| DomainEventBytes(Bytes::from(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(status: u16) -> Result<HttpResponse, PortError> {
        Ok(HttpResponse {
            status,
            headers: Vec::new(),
            body: Bytes::new(),
        })
    }

    fn event_json(event: DomainEventBytes) -> Value {
        match serde_json::from_slice(event.0.as_ref()) {
            Ok(value) => value,
            Err(error) => panic!("policy event must be valid JSON: {error}"),
        }
    }

    #[test]
    fn unauthorized_maps_to_im_event() {
        let outcome = response(401);
        let Some(event) = http_outcome_event("/api/cses/posts/create", &outcome) else {
            panic!("401 must produce an event");
        };

        assert_eq!(
            event_json(event),
            json!({
                "event": EVENT_HTTP_UNAUTHORIZED,
                "data": { "status": 401, "url": "/api/cses/posts/create" }
            })
        );
    }

    #[test]
    fn transport_error_maps_to_offline_event() {
        let outcome = Err(PortError::Transport("connection refused".to_string()));
        let Some(event) = http_outcome_event("/api/cses/sync", &outcome) else {
            panic!("transport must produce an event");
        };

        assert_eq!(
            event_json(event),
            json!({
                "event": EVENT_NET_OFFLINE,
                "data": { "url": "/api/cses/sync" }
            })
        );
    }

    #[test]
    fn unrelated_outcomes_do_not_emit() {
        assert!(http_outcome_event("/forbidden", &response(403)).is_none());
        assert!(http_outcome_event(
            "/invalid",
            &Err(PortError::Http("invalid method".to_string()))
        )
        .is_none());
    }
}
