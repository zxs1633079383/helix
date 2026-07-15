//! 最近消息 local-first 状态机。
//!
//! 这里只编排业务意图：本地覆盖判定、远端 `getLatestPost`、合并、缓存与统一投影。
//! Storage/HTTP 的物理执行仍由平台 driver 兑现。

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::Value;

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};

use super::MessageQueryRequest;

mod data;

#[cfg(test)]
use data::server_id;
use data::{
    classify_local_read, dedup_recent_rows, merge_recent_rows, message_key, message_result_effect,
    parse_latest_posts_reply, parse_local_rows, sort_recent_rows_desc,
    stale_local_server_rows_delete_op, visible_remote_rows_and_cache_ops,
};
#[cfg(test)]
pub(crate) use data::parse_latest_posts_body;

/// 平台提供的本地数据能力。它描述物理数据寿命，不承诺某次查询已经完整。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum LocalStoreMode {
    #[default]
    Durable,
    Session,
    Disabled,
}

/// 某次本地读相对“最近消息窗口”的业务覆盖结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalReadCoverage {
    Complete,
    Partial,
    Miss,
    Unsupported,
}

/// `getLatestPost` 的权威基础窗口。服务端可能为同 segment 补回更多行，但不会因此证明更早窗口。
const REMOTE_RECENT_WINDOW: usize = 20;

/// 当前连接会话内、成功远端读取且成功缓存后的覆盖证明。
///
/// 只记消息身份，不拿“本地有 N 行”冒充完整。hello/disconnect 会清空；cache Persist 失败不记录。
#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct RecentMessageCoverage {
    remote_keys_desc: Vec<String>,
    remote_exhausted: bool,
}

impl RecentMessageCoverage {
    fn from_remote(rows_desc: &[Value], received_count: usize) -> Option<Self> {
        let remote_exhausted = received_count < REMOTE_RECENT_WINDOW;
        // getLatestPost 满 20 后可能把同 segment 一并补回；多出来的行可用于本次展示，
        // 但不能把基础 20 窗口扩张成“服务端证明了 21+ 条完整”。
        let proven_len = if remote_exhausted {
            rows_desc.len()
        } else {
            rows_desc.len().min(REMOTE_RECENT_WINDOW)
        };
        let remote_keys_desc: Vec<String> = rows_desc
            .iter()
            .take(proven_len)
            .filter_map(message_key)
            .collect();
        if remote_keys_desc.is_empty() {
            return None;
        }
        Some(Self {
            remote_keys_desc,
            remote_exhausted,
        })
    }
}

impl ImModule {
    /// `im_query_messages_by_channel` 的唯一异步入口。
    pub(crate) fn dispatch_message_query(
        &mut self,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let request = super::parse_message_query(payload)?;
        if self.local_store_mode == LocalStoreMode::Disabled {
            return self.start_remote_message_query(request, Vec::new(), out);
        }

        let corr = self.alloc_corr_internal();
        out.push(super::build_message_query_from_request(&request, corr));
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageQueryLocal {
                request: Box::new(request),
                query_session_epoch: self.state.query_session_epoch,
            },
        );
        Ok(())
    }

    pub(crate) fn handle_message_query_local_reply(
        &mut self,
        request: MessageQueryRequest,
        query_session_epoch: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if query_session_epoch != self.state.query_session_epoch {
            return Ok(());
        }
        let mut local_rows_desc = match outcome {
            PortOutcome::Ok(reply) => match parse_local_rows(reply.0.as_ref()) {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        channel_id = request.channel_id.as_str(),
                        error = ?error,
                        "message query local scan reply malformed; falling back to remote"
                    );
                    Vec::new()
                }
            },
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query local scan failed; falling back to remote"
                );
                Vec::new()
            }
        };
        sort_recent_rows_desc(&mut local_rows_desc);

        let coverage = classify_local_read(
            self.local_store_mode,
            &request,
            &local_rows_desc,
            self.state.recent_message_coverage.get(&request.channel_id),
            self.message_query_has_known_gap(request.channel_id),
        );
        tracing::debug!(
            channel_id = request.channel_id.as_str(),
            ?coverage,
            local_rows = local_rows_desc.len(),
            "message query local coverage classified"
        );

        if coverage == LocalReadCoverage::Complete {
            out.push(message_result_effect(
                &request,
                &local_rows_desc,
                self.config.auth_user_id.as_str(),
            )?);
            return Ok(());
        }
        self.start_remote_message_query(request, local_rows_desc, out)
    }

    pub(crate) fn handle_message_query_remote_reply(
        &mut self,
        request: MessageQueryRequest,
        local_rows_desc: Vec<Value>,
        query_session_epoch: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if query_session_epoch != self.state.query_session_epoch {
            return Ok(());
        }
        let reply = match outcome {
            PortOutcome::Ok(reply) => reply,
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query remote fallback failed"
                );
                out.push(emit_message_query_failed(
                    request.channel_id,
                    format!("remote request failed: {error:?}"),
                ));
                return Ok(());
            }
        };

        let remote_posts = match parse_latest_posts_reply(reply) {
            Ok(posts) => posts,
            Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query remote fallback returned invalid response"
                );
                out.push(emit_message_query_failed(
                    request.channel_id,
                    error.to_string(),
                ));
                return Ok(());
            }
        };
        let received_count = remote_posts.len();
        let (mut remote_rows_desc, mut cache_ops) = match visible_remote_rows_and_cache_ops(
            request.channel_id,
            remote_posts,
            self.config.auth_user_id.as_str(),
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query remote fallback contained invalid posts"
                );
                out.push(emit_message_query_failed(
                    request.channel_id,
                    error.to_string(),
                ));
                return Ok(());
            }
        };
        dedup_recent_rows(&mut remote_rows_desc);
        sort_recent_rows_desc(&mut remote_rows_desc);

        let remote_exhausted = received_count < REMOTE_RECENT_WINDOW;
        let coverage = RecentMessageCoverage::from_remote(&remote_rows_desc, received_count);
        if remote_exhausted {
            if let Some(delete) = stale_local_server_rows_delete_op(
                request.channel_id,
                &local_rows_desc,
                &remote_rows_desc,
            ) {
                cache_ops.push(delete);
            }
        }
        let merged_rows_desc = merge_recent_rows(
            local_rows_desc,
            remote_rows_desc,
            remote_exhausted,
            request.limit as usize,
        );

        if self.local_store_mode == LocalStoreMode::Disabled || cache_ops.is_empty() {
            out.push(message_result_effect(
                &request,
                &merged_rows_desc,
                self.config.auth_user_id.as_str(),
            )?);
            return Ok(());
        }

        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: cache_ops,
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageQueryCache {
                request: Box::new(request),
                merged_rows_desc: Box::new(merged_rows_desc),
                coverage,
                query_session_epoch,
            },
        );
        Ok(())
    }

    pub(crate) fn handle_message_query_cache_reply(
        &mut self,
        request: MessageQueryRequest,
        merged_rows_desc: Vec<Value>,
        coverage: Option<RecentMessageCoverage>,
        query_session_epoch: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if query_session_epoch != self.state.query_session_epoch {
            return Ok(());
        }
        match outcome {
            PortOutcome::Ok(_) => {
                if query_session_epoch == self.state.query_session_epoch {
                    if let Some(coverage) = coverage {
                        self.state
                            .recent_message_coverage
                            .insert(request.channel_id, coverage);
                    }
                }
            }
            PortOutcome::Err(error) => tracing::warn!(
                channel_id = request.channel_id.as_str(),
                error = ?error,
                "message query remote cache failed; returning remote result without coverage"
            ),
        }
        out.push(message_result_effect(
            &request,
            &merged_rows_desc,
            self.config.auth_user_id.as_str(),
        )?);
        Ok(())
    }

    fn start_remote_message_query(
        &mut self,
        request: MessageQueryRequest,
        local_rows_desc: Vec<Value>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let payload = serde_json::to_vec(&serde_json::json!({
            "channel_id": request.channel_id.as_str(),
            "timestamp": 0,
        }))
        .map_err(|error| ImError::Serialize(error.to_string()))?;
        let mut effects = crate::commands::handle_outbound(
            "im_get_latest_post",
            &payload,
            self.config.api_base_url.as_str(),
            self.config.default_api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        )?;
        if effects.len() != 1 {
            return Err(ImError::Parse(format!(
                "im_get_latest_post expected one HTTP effect, got {}",
                effects.len()
            )));
        }
        let effect = effects
            .pop()
            .ok_or_else(|| ImError::Parse("im_get_latest_post produced no effect".to_string()))?;
        if !matches!(&effect, Effect::Http { .. }) {
            return Err(ImError::Parse(
                "im_get_latest_post did not produce Effect::Http".to_string(),
            ));
        }
        out.push(effect);
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageQueryRemote {
                request: Box::new(request),
                local_rows_desc: Box::new(local_rows_desc),
                query_session_epoch: self.state.query_session_epoch,
            },
        );
        Ok(())
    }

    fn message_query_has_known_gap(&self, channel_id: ChannelId) -> bool {
        let Some(channel) = self.state.channels.get(&channel_id) else {
            return false;
        };
        let behind_target = self
            .state
            .increment_target
            .get(&channel_id)
            .is_some_and(|target| channel.cursor.value() < *target);
        behind_target || channel.inflight_sync.is_some() || !channel.buffer.is_empty()
    }
}

/// 最近消息查询的显式失败状态；与成功投影分离，禁止用空列表伪装失败。
pub fn emit_message_query_failed(channel_id: ChannelId, error: String) -> Effect {
    let payload = serde_json::json!({
        "event": "im:post:increment-failed",
        "data": {
            "channelId": channel_id.as_str(),
            "error": error,
        }
    });
    let bytes = serde_json::to_vec(&payload).unwrap_or_else(|_| {
        br#"{"event":"im:post:increment-failed","data":{"channelId":"","error":"serialize failure"}}"#
            .to_vec()
    });
    Effect::Emit {
        event: DomainEventBytes(Bytes::from(bytes)),
    }
}

#[cfg(test)]
#[path = "local_first_tests.rs"]
mod tests;
