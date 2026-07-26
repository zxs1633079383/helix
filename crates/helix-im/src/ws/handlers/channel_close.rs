//! `channel_close` action handler（关闭频道：delete_at + is_active 置位，spec §S1）。
//!
//! 行为真源：现网 `handlers/channel.rs:575 handle_close` → `Channel::update_fields(delete_at,
//! is_active=false)` → emit `im:channel:closed{channelId, deleteAt}`。
//!
//! wire（`CloseChannelData` types.rs:374）：`channelId` / `deleteAt`。helix：定点列 patch
//! （`channel_set_cols`：UPDATE channel SET delete_at=?, is_active=0 WHERE id=?）。本地无此行
//! → 0 行命中 no-op（幂等）。spec §S3.4 纯本地字段 is_remove 不在此路径动。

use helix_core::effect::SqlValue;
use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const CHANNEL_CLOSE_ACTION: &str = "channel_close";

struct ChannelCloseHandler;

impl WsMessageHandler for ChannelCloseHandler {
    /// 返回关闭频道 authority 的唯一 WS action。
    fn action(&self) -> &'static str {
        CHANNEL_CLOSE_ACTION
    }

    /// 将可信 close tombstone 放入 G16 原子持久屏障，提交前不发布终态 Frame。
    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(channel_id) = data
            .get("channelId")
            .or_else(|| data.get("channel_id"))
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        let delete_at = data
            .get("deleteAt")
            .or_else(|| data.get("delete_at"))
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0);

        let corr = ctx.alloc_corr();
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ChannelLifecyclePersist {
                channel_id,
                transition: crate::state::ChannelLifecycleTransition::Closed { delete_at },
                causation_id: None,
            },
        );
        out.push(Effect::PersistAtomic {
            corr,
            ops: vec![crate::acl::to_effect_s1::channel_set_cols_op(
                channel_id,
                vec![
                    ("delete_at", SqlValue::Integer(delete_at)),
                    ("is_active", SqlValue::Integer(0)),
                ],
            )],
        });
        Ok(())
    }
}

static CHANNEL_CLOSE_HANDLER: ChannelCloseHandler = ChannelCloseHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&CHANNEL_CLOSE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: CHANNEL_CLOSE_ACTION,
        handler: &CHANNEL_CLOSE_HANDLER,
    }
}
