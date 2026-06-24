//! `__pong__` 合成 action handler——心跳 pong gap 补偿消费侧（UC-4.4，Round-3 偏差修复）。
//!
//! 行为真源（现网 source of truth）：cses-client `im_ws_client.rs:44-87 handle_pong_compensation`
//! → `sync_compensation.rs:31-85 compensate_from_pong`。pong 是 `status==OK` ACK，被 runtime 心跳
//! 校验吞掉不进 handler，故现网走 `ack_observer` 观察 `data.{gaps, hashMismatch}`。
//!
//! helix sans-IO 无 runtime 吞帧：pong 作 Text JSON 经 `Tick::Inbound` 进 core，`WsFrame::action()`
//! 把无 action 的 pong 帧合成 `PONG_ACTION` → 本 handler 解析 `gaps[].channelId` + `hashMismatch`
//! → `pong_compensate::compensate_from_pong`（5s 节流去重 + 全局 sync 窗口）。

use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::error::ImError;

use super::super::{
    ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, PONG_ACTION, WS_HANDLERS,
};

struct PongHandler;

impl WsMessageHandler for PongHandler {
    fn action(&self) -> &'static str {
        PONG_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // pong data 缺失 → 无补偿信息，no-op（零信任，不报错）。
        let Some(data) = frame.data() else {
            return Ok(());
        };

        let gap_channels = crate::pong_compensate::parse_gap_channels(data);
        let hash_mismatch = crate::pong_compensate::parse_hash_mismatch(data);

        // 无 gap 且无 hashMismatch → 纯 ACK，无补偿动作（提前返回，省 split-borrow）。
        if gap_channels.is_empty() && !hash_mismatch {
            return Ok(());
        }

        let now_ms = ctx.now_ms;
        let api_base_url = ctx.api_base_url.to_string();
        // 拆借：同拿 &mut ImState + corr 分配器（enqueue_and_drain 需要二者）。
        let (state, alloc_corr) = ctx.split_state_alloc();
        crate::pong_compensate::compensate_from_pong(
            state,
            &api_base_url,
            now_ms,
            &gap_channels,
            hash_mismatch,
            alloc_corr,
            out,
        );
        Ok(())
    }
}

static PONG_HANDLER: PongHandler = PongHandler;

#[distributed_slice(WS_HANDLERS)]
static PONG_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: PONG_ACTION,
    handler: &PONG_HANDLER,
};
