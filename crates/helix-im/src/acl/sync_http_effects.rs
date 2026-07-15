//! Sync HTTP Effect factories.

use crate::state::{ChannelId, Seq};
use bytes::Bytes;
use helix_core::effect::HttpRequest;
use helix_core::{AuthKind, Correlation, Effect, AUTH_KIND_HEADER};

/// 触发 /sync/notify HTTP 请求（pull 模型，设计文档 §1.4）。
pub fn sync_notify(
    base_url: &str,
    channel_id: ChannelId,
    from_seq: Seq,
    corr: Correlation,
    connection_id: Option<&str>,
) -> Effect {
    let body = serde_json::json!({
        "cursors": [
            { "channelId": channel_id.as_str(), "fromSeq": from_seq.0 }
        ]
    });
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(session_auth_headers(connection_id));
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{}/channel/sync/notify", base_url),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).expect(
                "sync_notify: static JSON shape must not fail to serialize",
            ))),
        },
    }
}

/// B-rest：触发 increment HTTP（push 模型，设计文档 §1.3 / §3 Phase 2）。
pub fn increment_http_trigger(
    base_url: &str,
    cursors: &[(ChannelId, Seq)],
    corr: Correlation,
    connection_id: Option<&str>,
) -> Effect {
    let cursor_json: Vec<serde_json::Value> = cursors
        .iter()
        .map(
            |(ch, from_seq)| serde_json::json!({ "channelId": ch.as_str(), "fromSeq": from_seq.0 }),
        )
        .collect();
    let body = serde_json::json!({
        "timestamp": 0,
        "cursors": cursor_json,
    });
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(session_auth_headers(connection_id));
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{}/channels/load/increment", base_url),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).expect(
                "increment_http_trigger: static JSON shape must not fail to serialize",
            ))),
        },
    }
}

/// A3b：把 connectionId（若 Some）组装成 IM HTTP 身份头列表。
pub fn connection_id_headers(connection_id: Option<&str>) -> Vec<(String, String)> {
    match connection_id {
        Some(id) => vec![("connectionId".to_string(), id.to_string())],
        None => vec![],
    }
}

/// 需要用户会话的 IM HTTP 请求统一携带内部 auth intent；driver 出网前替换成真实凭据。
pub fn session_auth_headers(connection_id: Option<&str>) -> Vec<(String, String)> {
    let mut headers = connection_id_headers(connection_id);
    headers.push((
        AUTH_KIND_HEADER.to_string(),
        AuthKind::Session.as_str().to_string(),
    ));
    headers
}
