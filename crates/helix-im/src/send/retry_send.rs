use crate::module::ImModule;
use crate::state::{ChannelId, SendStatus, TemporaryId};
use crate::ImError;
use helix_core::{Effect, EffectSink};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RetrySendCommand {
    #[serde(alias = "messageId")]
    message_id: String,
}

pub(crate) fn handle_retry_send(
    module: &mut ImModule,
    payload: &[u8],
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let cmd: RetrySendCommand = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("im_retry_send payload: {e}")))?;
    let temporary_id = TemporaryId(cmd.message_id);
    let body = {
        let pending = module
            .state
            .pending_sends
            .get(&temporary_id)
            .ok_or_else(|| ImError::Parse("im_retry_send missing pending send".to_string()))?;
        if pending.upload_failed {
            return Err(ImError::Parse(
                "im_retry_send cannot replace im_retry_upload".to_string(),
            ));
        }
        if pending.status != SendStatus::UnSend {
            return Err(ImError::Parse(format!(
                "im_retry_send requires failed status, got {:?}",
                pending.status
            )));
        }
        pending
            .body
            .clone()
            .ok_or_else(|| ImError::Parse("im_retry_send missing cached send body".to_string()))?
    };
    let channel_id = body
        .get("channelId")
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
        .ok_or_else(|| ImError::Parse("im_retry_send cached body missing channelId".to_string()))?;
    let body_temporary_id = body
        .get("temporaryId")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            ImError::Parse("im_retry_send cached body missing temporaryId".to_string())
        })?;
    if body_temporary_id != temporary_id.0 {
        return Err(ImError::Parse(
            "im_retry_send cached body correlation mismatch".to_string(),
        ));
    }

    let timeout_timer = module.alloc_timer();
    if let Some(pending) = module.state.pending_sends.get_mut(&temporary_id) {
        pending.status = SendStatus::Sending;
        pending.timeout_timer = timeout_timer;
    }
    out.push(Effect::PersistFire {
        ops: vec![crate::pending_send::send_status_persist_op(
            &temporary_id,
            "sending",
        )],
    });
    out.push(crate::acl::to_effect::emit_post_sending(
        channel_id,
        &temporary_id.0,
        body.get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        body.get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("TEXT"),
    ));
    module.emit_posts_create_http(channel_id, temporary_id, &body, out)
}
