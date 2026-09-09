//! Sync HTTP Effect factories.

use crate::state::{ChannelId, Seq};
use bytes::Bytes;
use helix_core::effect::{HttpRequest, ScanOrder, ScanSpec, StorageOp};
use helix_core::{AuthKind, Correlation, Effect, AUTH_KIND_HEADER};

const CSES_TRACK_ID_HEADER: &str = "Cses-Track-Id";

pub(crate) fn sync_track_id(corr: Correlation) -> String {
    format!("helix-sync:{}", corr.raw())
}

const INCREMENT_MESSAGE_TIMESTAMP_ORDER: &[ScanOrder] = &[ScanOrder::desc("update_at")];

/// 读取本地 message 表的最大 update_at，作为下一次 increment 的时间边界。
pub fn increment_message_timestamp_scan(corr: Correlation) -> Effect {
    Effect::Persist {
        corr,
        ops: vec![StorageOp::Scan(ScanSpec {
            table: "message",
            limit: Some(1),
            filter: None,
            order_by: INCREMENT_MESSAGE_TIMESTAMP_ORDER,
        })],
    }
}

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
    headers.push((CSES_TRACK_ID_HEADER.to_string(), sync_track_id(corr)));
    if let Some(id) = connection_id {
        headers.push(("X-CSES-Sync-Session-Id".into(), id.to_string()));
    }
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
    timestamp: i64,
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
        "timestamp": timestamp,
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

#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::effect::{ScanOrder, StorageOp};

    #[test]
    fn sync_notify_carries_corr_derived_track_id() {
        let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let corr = Correlation::from_raw(42);
        let Effect::Http {
            corr: effect_corr,
            req,
        } = sync_notify(
            "http://localhost/api/cses",
            channel_id,
            Seq(7),
            corr,
            Some("conn-1"),
        )
        else {
            panic!("sync_notify must emit Http");
        };

        assert_eq!(effect_corr, corr);
        assert!(req
            .headers
            .iter()
            .any(|(key, value)| key == CSES_TRACK_ID_HEADER && value == "helix-sync:42"));
    }

    #[test]
    fn increment_message_timestamp_scan_reads_latest_message_update_at() {
        let corr = Correlation::from_raw(42);
        let Effect::Persist {
            corr: effect_corr,
            ops,
        } = increment_message_timestamp_scan(corr)
        else {
            panic!("increment timestamp lookup must use Persist");
        };

        assert_eq!(effect_corr, corr);
        let [StorageOp::Scan(spec)] = ops.as_slice() else {
            panic!("increment timestamp lookup must scan exactly one operation");
        };
        assert_eq!(spec.table, "message");
        assert_eq!(spec.limit, Some(1));
        assert!(spec.filter.is_none());
        assert_eq!(spec.order_by, &[ScanOrder::desc("update_at")]);
    }

    #[test]
    fn increment_http_trigger_carries_timestamp_without_changing_cursors() {
        let channel_id = ChannelId::from_str("ch00000000000000000000000a").unwrap();
        let corr = Correlation::from_raw(42);
        let Effect::Http { req, .. } = increment_http_trigger(
            "http://localhost/api/cses",
            1_726_000_123,
            &[(channel_id, Seq(17))],
            corr,
            Some("conn-1"),
        ) else {
            panic!("increment trigger must emit Http");
        };

        let body: serde_json::Value =
            serde_json::from_slice(req.body.as_ref().expect("increment body")).unwrap();
        assert_eq!(body["timestamp"], 1_726_000_123);
        assert_eq!(body["cursors"][0]["channelId"], channel_id.as_str());
        assert_eq!(body["cursors"][0]["fromSeq"], 17);
    }
}
