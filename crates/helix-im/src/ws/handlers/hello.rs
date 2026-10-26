use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::{ChannelId, ConnState, CorrelationContext, Seq};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const HELLO_ACTION: &str = "hello";

struct HelloHandler;

impl WsMessageHandler for HelloHandler {
    fn action(&self) -> &'static str {
        HELLO_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(connection_id) = frame
            .data()
            .and_then(|data| data.get("connectionId"))
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(());
        };

        // 同一活跃物理连接的重复握手不重置恢复；断线后的同 ID 重连仍须重新初始化。
        if ctx.state.conn == ConnState::Connected
            && ctx.state.connection_id.as_deref() == Some(connection_id)
        {
            return Ok(());
        }

        ctx.state.conn = ConnState::Connected;
        ctx.state.connection_id = Some(connection_id.to_string());
        // This registry handler is the production hello entrypoint. Establish
        // the recovery epoch here; the legacy ImModule helper is not invoked by
        // registry dispatch.
        ctx.state.recovery_session.begin(ctx.auth_user_id);
        ctx.state.reset_increment_batch();
        ctx.state.increment_page_supported = frame
            .data()
            .and_then(|data| data.get("incrementPageVersion"))
            .and_then(serde_json::Value::as_u64)
            == Some(1);
        out.push(crate::acl::to_effect::emit_connection_established(
            connection_id,
        ));

        // 共库的频道归属扫描尚未完成时，空集合不是“当前公司没有本地消息”。
        // finish_startup_scan 会用完整公司集合计算 create_at，再发唯一 increment 请求。
        if ctx.state.corr_map.values().any(|context| {
            matches!(
                context,
                CorrelationContext::ScanCursors | CorrelationContext::ScanChannelProjections
            )
        }) {
            ctx.state.pending_increment_bootstrap_after_scan = true;
            return Ok(());
        }

        // 启动 channel 投影扫描完成前不发送任何 active cursor；否则已删除频道会在
        // hello 竞态窗口重新进入 Go 的增量/补偿链路。扫描成功后仍排除 terminal 聚合。
        let mut cursors: Vec<(ChannelId, Seq)> = if ctx.state.startup_channel_projection_ready
            && !ctx
                .state
                .corr_map
                .values()
                .any(|context| matches!(context, CorrelationContext::ScanChannelProjections))
        {
            ctx.state
                .channels
                .iter()
                .filter(|(_, channel)| !channel.is_terminal())
                .map(|(&id, ch)| (id, ch.cursor.value()))
                .collect()
        } else {
            Vec::new()
        };
        cursors.sort_unstable_by_key(|(id, _)| *id);
        ctx.state.pending_increment_bootstrap_after_scan = cursors.is_empty();
        let timestamp_scan_corr = ctx.alloc_corr();
        ctx.state.corr_map.insert(
            timestamp_scan_corr,
            CorrelationContext::IncrementMessageTimestampScan {
                connection_id: Some(connection_id.to_string()),
                cursors,
            },
        );
        out.push(
            crate::acl::to_effect::increment_message_timestamp_scan_for_channels(
                timestamp_scan_corr,
                ctx.scoped_channel_ids.unwrap_or(&[]),
            ),
        );
        tracing::info!(
            "helix-im: hello handshake complete, local message watermark scan started before increment HTTP"
        );

        Ok(())
    }
}

static HELLO_HANDLER: HelloHandler = HelloHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&HELLO_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: HELLO_ACTION,
        handler: &HELLO_HANDLER,
    }
}
