//! `channel_created` action handler（S3 path1：新建 channel 全量 upsert，spec §S1/§S3.1）。
//!
//! 行为真源：现网 `handlers/channel.rs:482 handle_create` → `channel_service::process_and_save`
//! （path1 全 52 列 upsert）→ emit `im:channel:created`。建话题 makeTopic 也发 channel_created。
//!
//! helix：`channel_write::program_full`（全 52 列 + JSON rename + 守卫）→ `upsert_channel_full`。
//! 成员（memberChange.join）同步写 `channel_member` 表（复合 PK，与 channel_member_update 同口径）。
//! 非法 data（缺 id / 非 26 字符）→ program_full None → 跳落库（仍 best-effort emit 由 channel_id
//! gate 决定；缺 id 则整体 noop，边界零信任 helix-im 不变量 4）。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::ChannelId;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const CHANNEL_CREATED_ACTION: &str = "channel_created";

struct ChannelCreatedHandler;

impl WsMessageHandler for ChannelCreatedHandler {
    /// 返回新建频道 authority 的唯一 WS action。
    fn action(&self) -> &'static str {
        CHANNEL_CREATED_ACTION
    }

    /// 把 channel_created 权威帧放入 G-15a 原子持久屏障。
    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        // id 硬 gate（缺/非 26 字符 → 整体 noop）。
        let Some(channel_id) = data
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        let causation_id = frame
            .cses_track_id()
            .or_else(|| {
                data.get("reqId")
                    .or_else(|| data.get("req_id"))
                    .and_then(serde_json::Value::as_str)
            })
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        super::channel_member_update::queue_channel_create_persist(
            ctx,
            channel_id,
            data.clone(),
            causation_id,
            out,
        );
        Ok(())
    }
}

static CHANNEL_CREATED_HANDLER: ChannelCreatedHandler = ChannelCreatedHandler;
#[cfg(target_arch = "wasm32")]
/// 在 wasm inventory 不可自动发现时保留静态 handler。
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&CHANNEL_CREATED_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: CHANNEL_CREATED_ACTION,
        handler: &CHANNEL_CREATED_HANDLER,
    }
}
