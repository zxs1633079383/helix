//! 分类公开投影只走已有WS入口，私有参与状态只允许认证HTTP回包。
use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};
use crate::category_chain::{self, runtime, wire, Request};
use crate::error::ImError;
use crate::state::Seq;
use helix_core::EffectSink;

struct CategoryChainHandler;
impl WsMessageHandler for CategoryChainHandler {
    /// 精确action注册，未知schema不会改写成文字接龙。
    fn action(&self) -> &'static str {
        category_chain::EVENT
    }

    /// 保持字符串eventSeq精度，权威字段经同一durable串行门处理。
    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let data = frame
            .root()
            .get("data")
            .ok_or_else(|| ImError::Parse("category WS data missing".to_owned()))?;
        let channel = category_chain::string(data, "channelId")
            .ok_or_else(|| ImError::Parse("category WS channel missing".to_owned()))?;
        let chain = category_chain::string(data, "chainId")
            .ok_or_else(|| ImError::Parse("category WS chain missing".to_owned()))?;
        if let Err(error) = wire::validate_authority(data, &channel, Some(&chain), false) {
            tracing::warn!(error=%error,"category WS authority rejected");
            return Ok(());
        }
        let seq = data
            .get("eventSeq")
            .and_then(serde_json::Value::as_str)
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|n| *n > 0)
            .map(Seq)
            .ok_or_else(|| ImError::Parse("category WS eventSeq invalid".to_owned()))?;
        let request = Request {
            command: "category_chain_projection".to_owned(),
            channel,
            chain: Some(chain),
            viewer: ctx.auth_user_id.to_owned(),
            req_id: None,
            mutation: None,
            query_scope: "public-ws".to_owned(),
            temporary_id: None,
        };
        let work = runtime::make_work(request, data.to_owned(), Some(seq), true)?;
        let (state, alloc) = ctx.split_state_alloc();
        runtime::enqueue(state, alloc, work, out)
    }
}
inventory::submit! { WsHandlerRegistration { action: category_chain::EVENT, handler:&CategoryChainHandler } }
/// Web链接锚确保inventory注册保留。
pub(crate) fn inventory_link_anchor() {}
