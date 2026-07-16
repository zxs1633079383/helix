//! 显式离线同步应用命令。
//!
//! 平台只提交频道意图；cursor 始终来自 Helix 本地状态，避免 UI 注入或回退业务水位。

use crate::channel::Channel;
use crate::error::ImError;
use crate::module::ImModule;
use crate::state::ChannelId;
use helix_core::EffectSink;

pub(crate) fn handle(
    module: &mut ImModule,
    payload: &[u8],
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let value: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("im_sync_channels payload: {error}")))?;
    let channel_id = value
        .get("channel_id")
        .and_then(serde_json::Value::as_str)
        .and_then(ChannelId::from_str)
        .ok_or_else(|| ImError::Parse("im_sync_channels: 缺/非法 channel_id".to_string()))?;

    module
        .state
        .channels
        .entry(channel_id)
        .or_insert_with(|| Channel::new(channel_id, 0));

    let api_base_url = module.config.api_base_url.clone();
    module.with_state_and_corr_allocator(|state, alloc_corr| {
        crate::sync_scheduler::enqueue_and_drain(
            state,
            api_base_url.as_str(),
            &[channel_id],
            alloc_corr,
            out,
        );
    });
    Ok(())
}
