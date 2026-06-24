//! P6 투영/状态命令分发（从 module.rs 外提 sibling，守「只减不增」基线）。
//!
//! `handle_query_command` 薄壳，契约在 `crate::query`。接受显式参数替代 `&mut ImModule`
//! 以规避 borrow-split 限制，module.rs 保留同名 1-line 委派方法。

use crate::error::ImError;
use crate::state::{CorrelationContext, ImState};
use helix_core::EffectSink;

/// P6 投影/状态命令分发（薄壳，契约在 `crate::query`）。
///
/// - `im_query_messages_by_channel`：吐 `Scan(message)` + 注册 `MessageQuery` corr 上下文；
///   PortReply 回报后 `handle_port_reply` 的 MessageQuery 臂 emit 投影。
/// - `im_delete_all_dialogs`：清内存 channel gate + emit 刷新（物理 truncate 缺原语，见 query.rs）。
pub(crate) fn handle_query_command(
    next_corr: &mut u64,
    state: &mut ImState,
    api_base_url: &str,
    name: &str,
    payload: &[u8],
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let mut alloc = || {
        let c = helix_core::Correlation::from_raw(*next_corr);
        *next_corr += 1;
        c
    };
    match name {
        "im_query_messages_by_channel" => {
            let corr = alloc();
            let (channel_id, effect) = crate::query::build_message_query(payload, corr)?;
            state
                .corr_map
                .insert(corr, CorrelationContext::MessageQuery { channel_id });
            out.push(effect);
        }
        "im_query_dialog_list" => {
            // UC-3：低频按需拉全量会话列表（off hot-path）→ Scan channel + 注册 DialogListQuery；
            // PortReply 回报后 emit `im:channels:projection` 完整 dialogList（前端零计算）。
            let corr = alloc();
            let effect = crate::query::build_dialog_list_query(payload, corr)?;
            state
                .corr_map
                .insert(corr, CorrelationContext::DialogListQuery);
            out.push(effect);
        }
        "im_delete_all_dialogs" => {
            // 内存 channel gate 清空（core 拥有的状态；物理 DELETE 留待 truncate 原语，query.rs）。
            state.channels.clear();
            out.push(crate::query::emit_dialogs_cleared());
        }
        // D3 上拉翻页（NEED_HELIX_FIX_PULL_REFRESH）：解析锚点 → 发首轮 postContext Http +
        // 注册 LoadOlderContext 续接态。多轮编排在 handle_port_reply 的 LoadOlderContext 臂（每轮
        // 回报推进 anchor，凑够/耗尽 → emit im:messages:older_loaded）。
        n if n == crate::older_context::LOAD_OLDER_CONTEXT => {
            let st = crate::older_context::parse_request(payload)?;
            let corr = alloc();
            let (_, body) = crate::older_context::build_post_context_body(&st);
            out.push(crate::older_context::post_context_http(
                api_base_url,
                &body,
                corr,
                state.connection_id.as_deref(),
            ));
            state.corr_map.insert(
                corr,
                CorrelationContext::LoadOlderContext {
                    state: Box::new(st),
                },
            );
        }
        _ => tracing::debug!("im: query dispatch miss '{}'", name),
    }
    Ok(())
}
