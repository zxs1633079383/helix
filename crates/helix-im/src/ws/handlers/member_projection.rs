//! Canonical viewer-local absolute member projection.

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const MEMBER_PROJECTION_ACTION: &str = "member_projection";

struct MemberProjectionHandler;

impl WsMessageHandler for MemberProjectionHandler {
    fn action(&self) -> &'static str {
        MEMBER_PROJECTION_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        queue_member_projection(ctx, frame.data_required()?, out)
    }
}

pub(crate) fn queue_member_projection(
    ctx: &mut ImWsContext<'_>,
    data: &serde_json::Value,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let Some(channel_id) = data
        .get("channelId")
        .or_else(|| data.get("channel_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
    else {
        return Ok(());
    };
    let Some(user_id) = data
        .get("userId")
        .or_else(|| data.get("user_id"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty() && *value == ctx.auth_user_id)
    else {
        return Ok(());
    };
    let Some((row, projection)) = crate::channel_update::member_channel_from_update_channel(
        data,
        channel_id,
        ctx.auth_user_id,
        ctx.now_ms,
    ) else {
        return Ok(());
    };
    let (revision, effect_id) = crate::channel_update::require_member_projection_identity(
        &projection,
        "member projection",
    )?;

    let corr = ctx.alloc_corr();
    ctx.state.corr_map.insert(
        corr,
        crate::state::CorrelationContext::MemberProjectionPersist {
            channel_id,
            expected_revision: revision,
            expected_effect_id: effect_id.to_string(),
            expected_projection: Box::new(projection),
        },
    );
    out.push(helix_core::Effect::PersistAtomic {
        corr,
        ops: crate::acl::to_effect::canonical_member_projection_ops(
            channel_id, user_id, revision, row,
        ),
    });
    Ok(())
}

pub(crate) fn emit_committed_projection(
    reply: &[u8],
    channel_id: ChannelId,
    expected: &crate::channel_update::MemberChannelUpdate,
    auth_user_id: &str,
    now_ms: u64,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let Some(projection) =
        crate::channel_update::member_channel_from_reply(reply, channel_id, auth_user_id, now_ms)
    else {
        return Ok(());
    };
    let Some(expected_revision) = expected.projection_revision else {
        return Ok(());
    };
    if projection.projection_revision != Some(expected_revision)
        || projection.effect_id != expected.effect_id
        || !crate::channel_update::member_projection_matches(expected, &projection)
    {
        return Ok(());
    }
    let data = crate::acl::to_effect::member_channel_update_data(
        channel_id,
        &projection,
        "member_projection",
    );
    let mut update = data
        .get("dialogPatch")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(object) = update.as_object_mut() {
        object.insert(
            "channelId".to_string(),
            serde_json::Value::String(channel_id.as_str().to_string()),
        );
    }
    out.push(crate::event::channel::update(update)?.into_effect());
    out.push(
        crate::event::read::channel_from_member_projection(channel_id.as_str(), &data)?
            .into_effect(),
    );
    Ok(())
}

static MEMBER_PROJECTION_HANDLER: MemberProjectionHandler = MemberProjectionHandler;

#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&MEMBER_PROJECTION_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: MEMBER_PROJECTION_ACTION,
        handler: &MEMBER_PROJECTION_HANDLER,
    }
}
