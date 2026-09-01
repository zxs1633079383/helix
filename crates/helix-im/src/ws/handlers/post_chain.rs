//! `post_chain` WS authority handler：与 HTTP 使用同一 chain projection/persist barrier。

use crate::chain::{self, ChainMutationState, ChainRequest};
use crate::error::ImError;
use crate::state::{ChannelId, CorrelationContext};
use helix_core::effect::Effect;
use helix_core::EffectSink;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const POST_CHAIN_ACTION: &str = "post_chain";

struct PostChainHandler;

impl WsMessageHandler for PostChainHandler {
    /// 返回接龙 authority 的唯一 WS action。
    fn action(&self) -> &'static str {
        POST_CHAIN_ACTION
    }

    /// 只接受连续 eventSeq，并把 authority facts 放入 matching PersistAtomic。
    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let authority = match chain::authority_from_ws(frame.root(), frame.event_seq()) {
            Ok(authority) => authority,
            Err(error) => {
                tracing::warn!(error = ?error, "invalid post_chain WS authority");
                return Ok(());
            }
        };
        if authority.event_id.as_ref().is_some_and(|event_id| {
            ctx.state.seen_chain_event_ids.contains(event_id)
                || ctx.state.pending_chain_event_ids.contains(event_id)
        }) {
            return Ok(());
        }
        let channel_id = ChannelId::from_str(&authority.channel_id)
            .ok_or_else(|| ImError::Parse("post_chain WS invalid channel_id".to_string()))?;
        if let Some(event_seq) = authority.event_seq {
            let channel = ctx
                .state
                .channels
                .entry(channel_id)
                .or_insert_with(|| crate::channel::Channel::new(channel_id, 0));
            // The generic canonical stream can commit the same sequence first; chain tables and
            // the chain event still must be projected even when the shared cursor is already there.
            if event_seq > channel.cursor.value() && !channel.admit_chain_event_seq(event_seq, out)
            {
                return Ok(());
            }
        }
        let command = chain::command_for_event(authority.event_type.as_deref());
        let request = ChainRequest {
            command: command.to_string(),
            channel_id: authority.channel_id.clone(),
            chain_id: authority.chain_id.clone(),
            client_mutation_id: authority
                .raw
                .get("clientMutationId")
                .or_else(|| authority.raw.get("client_mutation_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            operation_id: authority
                .raw
                .get("operationId")
                .or_else(|| authority.raw.get("operation_id"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string),
            device_id: None,
            payload: authority.raw.clone(),
        };
        let mut ops = chain::persist_ops(
            &authority,
            &request,
            request
                .client_mutation_id
                .as_ref()
                .map(|_| ChainMutationState::Confirmed),
            None,
        );
        if let Some(event_seq) = authority.event_seq {
            ops.push(crate::acl::to_effect::advance_cursor_op(
                channel_id, event_seq,
            ));
        }
        if ops.is_empty() {
            return Ok(());
        }
        if let Some(event_id) = authority.event_id.as_ref() {
            ctx.state.pending_chain_event_ids.insert(event_id.clone());
        }
        let event_name = authority
            .event_type
            .clone()
            .unwrap_or_else(|| command.to_string());
        let corr = ctx.alloc_corr();
        ctx.state.corr_map.insert(
            corr,
            CorrelationContext::ChainPersist {
                request: Box::new(request),
                authority: Box::new(authority),
                event_name,
                mutation_state: Some(ChainMutationState::Confirmed),
                error_code: None,
            },
        );
        out.push(Effect::PersistAtomic { corr, ops });
        Ok(())
    }
}

static POST_CHAIN_HANDLER: PostChainHandler = PostChainHandler;

#[cfg(target_arch = "wasm32")]
/// 在 wasm inventory 不可自动发现时保留静态 handler。
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&POST_CHAIN_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: POST_CHAIN_ACTION,
        handler: &POST_CHAIN_HANDLER,
    }
}
