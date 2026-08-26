//! Thin, pass-through, and control projection emit factories.

use crate::state::{ChannelId, Seq};
use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;

/// G-15 request lifecycle signal. This reports only the command/remote stage;
/// UI completion still requires the matching channel-created and root-post frames.
pub fn emit_topic_operation_status(
    req_id: &str,
    root_message_id: &str,
    phase: &str,
    topic_channel_id: Option<&str>,
    failure_code: Option<&str>,
    retryable: bool,
) -> Effect {
    let payload = serde_json::json!({
        "event": "im:operation:status",
        "data": {
            "reqId": req_id,
            "kind": "topic-create",
            "phase": phase,
            "rootMessageId": root_message_id,
            "topicChannelId": topic_channel_id,
            "failureCode": failure_code,
            "retryable": retryable,
        },
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_topic_operation_status: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 发送 im:sync:too_long 事件。
pub fn emit_sync_too_long(channel_id: ChannelId, reset_to: Seq) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:sync:too_long",
        "data": {
            "channelId": channel_id.as_str(),
            "resetTo": reset_to.0,
            "mode": "drop_and_reload",
            "dropLocalMessages": true,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_sync_too_long: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// G-S3：同步事务成功提交后的 render-ready 状态。HTTP 成功或解析成功均不得直接调用。
pub fn emit_sync_state(channel_id: ChannelId, committed_seq: Seq) -> Effect {
    emit_sync_state_with_trigger(
        channel_id,
        committed_seq,
        crate::state::SyncTrigger::Routine,
    )
}

pub fn emit_sync_state_with_trigger(
    channel_id: ChannelId,
    committed_seq: Seq,
    trigger: crate::state::SyncTrigger,
) -> Effect {
    use serde_json::json;
    let mut payload = json!({
        "event": "im:sync:state",
        "data": {
            "state": "committed",
            "recovery": {
                "channelId": channel_id.as_str(),
                "committedSeq": committed_seq.0,
            }
        }
    });
    if let (Some(label), Some(data)) = (
        trigger.projection_label(),
        payload
            .get_mut("data")
            .and_then(serde_json::Value::as_object_mut),
    ) {
        data.insert("trigger".to_string(), label.into());
    }
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_sync_state: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// A4：发送 im:connection:established 事件（hello 握手完成信号）。
pub fn emit_connection_established(connection_id: &str) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:connection:established",
        "data": {
            "connectionId": connection_id,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_connection_established: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// B-rest：发送 im:channel:update 事件（子 topic 批次结束）。
pub fn emit_channel_update(channel_id: ChannelId) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channel:update",
        "data": { "channel_id": channel_id.as_str() }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_update: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 连接控制：`im_reconnect` → `im:net:reconnect_requested` 信号。
pub fn emit_reconnect_requested() -> Effect {
    let payload = serde_json::json!({ "event": "im:net:reconnect_requested", "data": {} });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("emit_reconnect_requested: static JSON must not fail"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// `posts_update`（批量编辑/撤回）按当前 viewer 投影为 `im:post:batch-updated`。
pub fn emit_post_batch_updated(
    channel_id: ChannelId,
    posts: &serde_json::Value,
    viewer_user_id: &str,
) -> Effect {
    use serde_json::json;
    let posts = if posts.is_array() {
        crate::render_ready::shape_message_rows_for_viewer(posts, viewer_user_id)
    } else {
        json!([])
    };
    let payload = json!({
        "event": "im:post:batch-updated",
        "data": {
            "channel_id": channel_id.as_str(),
            "posts": posts,
        }
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_batch_updated: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

mod dialog;
pub use dialog::{member_channel_update_data, post_channel_update_data};

#[cfg(test)]
#[path = "projection_control_effects_tests.rs"]
mod tests;
