//! HTTP and render-ready effects for older-context paging.

use super::{sort_key, LoadOlderState};
use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, HttpRequest};
use helix_core::{Correlation, Effect};
use serde_json::{json, Value};

/// 组装一轮 `posts/postContext` POST Effect（IM 网关 base + connectionId 身份头，与既有 outbound 同壳）。
/// 真凭据由 driver 注入（C6，core 不拼）；body 由 [`build_post_context_body`] 产。
pub fn post_context_http(
    base_url: &str,
    body: &Value,
    corr: Correlation,
    connection_id: Option<&str>,
) -> Effect {
    post_context_http_tracked(base_url, body, corr, connection_id, None)
}

/// 与 [`post_context_http`] 相同，但把 UI intent `req_id` 贯穿为 Go `Cses-Track-Id`。
/// 多轮翻页复用同一 track id，correlation 由 corr_map 继续区分每一轮 PortReply。
pub fn post_context_http_tracked(
    base_url: &str,
    body: &Value,
    corr: Correlation,
    connection_id: Option<&str>,
    request_id: Option<&str>,
) -> Effect {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::sync_http_effects::session_auth_headers(
        connection_id,
    ));
    if let Some(request_id) = request_id.filter(|value| !value.is_empty()) {
        headers.push(("Cses-Track-Id".to_string(), request_id.to_string()));
    }
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{base_url}/posts/postContext"),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(body).unwrap_or_default())),
        },
    }
}

/// emit `im:messages:older_loaded{channelId, messages[], hasMore}`（升序 wire Post 透传 + hasMore）。
///
/// `messages` = 严格更早 wire Post **升序**数组（前端 prepend），透传 camelCase（HX-C005 不改业务数据）。
/// **emit 是唯一权威排序点**（createAt 升序 + 同毫秒 temporaryId 确定化，HX-C010）——不依赖调用前
/// `stop()` 是否已排（防中途 emit / Err 收尾路径漏排）；对已排序输入幂等。空集合 → `messages:[]`，不 panic。
pub fn emit_older_loaded(state: &LoadOlderState) -> Effect {
    emit_older_loaded_for_viewer(state, "")
}

pub fn emit_older_loaded_for_viewer(state: &LoadOlderState, viewer_user_id: &str) -> Effect {
    let mut messages = state.older.clone();
    messages.sort_by_key(sort_key);
    let payload = json!({
        "event": "im:messages:older_loaded",
        "data": {
            "channelId": state.channel_id.as_str(),
            "messages": crate::render_ready::shape_message_rows_for_viewer(
                &serde_json::Value::Array(messages),
                viewer_user_id,
            ),
            "hasMore": state.has_more,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("emit_older_loaded: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}
