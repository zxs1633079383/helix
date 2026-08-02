//! 最近消息 local-first 状态机。
//!
//! 这里只编排业务意图：本地覆盖判定、远端 `getLatestPost`、合并、缓存与统一投影。
//! Storage/HTTP 的物理执行仍由平台 driver 兑现。

use helix_core::effect::Effect;
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::Value;

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};

use super::MessageQueryRequest;

mod data;

use data::{
    classify_local_read, dedup_recent_rows, message_key, parse_latest_posts_reply, server_id,
    stale_local_server_rows_delete_op,
};
pub(crate) use data::{parse_local_rows, sort_recent_rows_desc, visible_remote_rows_and_cache_ops};

/// 解析 Go G11h initial-window 业务体，并保证非空窗口的首条就是请求目标。
pub(crate) fn parse_initial_window_posts(
    raw_body: &[u8],
    target_post_id: &str,
) -> Result<Vec<Value>, ImError> {
    let root: Value = serde_json::from_slice(raw_body)
        .map_err(|error| ImError::Parse(format!("getPostsAfterIndex body: {error}")))?;
    let status = root
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse("getPostsAfterIndex body missing string status".into()))?;
    if !status.eq_ignore_ascii_case("SUCCESS") {
        return Err(ImError::Parse(format!(
            "getPostsAfterIndex backend status {status}"
        )));
    }
    let payload = root
        .pointer("/data/posts")
        .or_else(|| root.get("data"))
        .ok_or_else(|| {
            ImError::Parse("getPostsAfterIndex response missing posts array".to_string())
        })?;
    if payload.is_null() {
        return Ok(Vec::new());
    }
    let rows = payload.as_array().ok_or_else(|| {
        ImError::Parse("getPostsAfterIndex response posts must be array".to_string())
    })?;
    if rows.iter().any(|row| !row.is_object()) {
        return Err(ImError::Parse(
            "getPostsAfterIndex posts must be objects".to_string(),
        ));
    }
    let rows = rows.clone();
    if rows.is_empty() {
        return Ok(rows);
    }
    if !post_matches_identity(&rows[0], target_post_id) {
        return Err(ImError::Parse(
            "getPostsAfterIndex target must be first row".to_string(),
        ));
    }
    Ok(rows)
}

/// 比较 canonical server id 与 temporaryId，兼容只知任一 post identity 的 caller。
pub(crate) fn post_matches_identity(row: &Value, target_post_id: &str) -> bool {
    ["id", "postId", "temporaryId", "temporary_id"]
        .iter()
        .filter_map(|key| row.get(*key).and_then(Value::as_str))
        .any(|value| value == target_post_id)
}

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

/// 将已附着窗口大小收敛到 Go timeline V3 接受的 1..=60 页长。
fn authoritative_readback_page_size(visible_items: Option<usize>) -> u32 {
    visible_items
        .filter(|visible| *visible > 0)
        .and_then(|visible| u32::try_from(visible).ok())
        .map(|visible| visible.saturating_add(1))
        .unwrap_or(super::QUERY_MESSAGES_DEFAULT)
        .min(crate::timeline_state::MAX_TIMELINE_PAGE_SIZE)
}

#[cfg(test)]
mod authoritative_readback_tests {
    use super::*;

    /// 空窗口沿用时间线默认页长，避免发出显式零值。
    #[test]
    fn authoritative_readback_defaults_for_empty_window() {
        assert_eq!(
            authoritative_readback_page_size(None),
            super::super::QUERY_MESSAGES_DEFAULT
        );
        assert_eq!(
            authoritative_readback_page_size(Some(0)),
            super::super::QUERY_MESSAGES_DEFAULT
        );
    }

    /// 大窗口必须截断到 Go timeline V3 的 60 条硬上限。
    #[test]
    fn authoritative_readback_caps_large_window_at_go_contract_limit() {
        assert_eq!(authoritative_readback_page_size(Some(59)), 60);
        assert_eq!(
            authoritative_readback_page_size(Some(60)),
            crate::timeline_state::MAX_TIMELINE_PAGE_SIZE
        );
        assert_eq!(
            authoritative_readback_page_size(Some(500)),
            crate::timeline_state::MAX_TIMELINE_PAGE_SIZE
        );
    }

    /// 已提交的 optimistic corr 被消费后，durable authority 仍可独立结算 retry。
    #[test]
    fn authoritative_settle_does_not_require_live_optimistic_correlation() {
        let mut module = ImModule::new(crate::module::ImConfig::default());
        let temporary_id = crate::state::TemporaryId("tmp-authority-no-corr".to_string());
        let mut pending = crate::pending_send::PendingSend::new(
            temporary_id.clone(),
            helix_core::TimerId::from_raw(41),
            None,
        );
        pending.persist_corr = None;
        module
            .state
            .pending_sends
            .insert(temporary_id.clone(), pending);
        let continuation = CorrelationContext::AuthoritativeSendReconcilePersist {
            temporary_id: temporary_id.clone(),
        };
        let mut out = EffectSink::new();

        assert!(module.settle_authoritative_send(
            &temporary_id,
            crate::state::test_server_id(41),
            continuation,
            &mut out,
        ));
        assert!(!module.state.pending_sends.contains_key(&temporary_id));
        assert!(out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Persist { .. })));
        assert!(out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::CancelTimer { .. })));
    }
}

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
    /// 记录服务端已证明的最近窗口；是否到顶由本地与远端联合判断后显式传入。
    fn from_remote(rows_desc: &[Value], remote_exhausted: bool) -> Option<Self> {
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

/// 短远端窗口只有在本地不存在更早服务端消息时，才可证明全局历史到顶。
fn recent_reply_proves_history_exhausted(
    local_rows_desc: &[Value],
    remote_rows_desc: &[Value],
    received_count: usize,
) -> bool {
    if received_count >= REMOTE_RECENT_WINDOW {
        return false;
    }
    let oldest_remote = remote_rows_desc.iter().filter_map(message_create_at).min();
    !oldest_remote.is_some_and(|oldest| {
        local_rows_desc.iter().any(|row| {
            !server_id(row).is_empty()
                && message_create_at(row).is_some_and(|create_at| create_at < oldest)
        })
    })
}

/// 兼容本地 snake_case 与 render-ready camelCase 行，统一提取消息时间。
fn message_create_at(row: &Value) -> Option<i64> {
    row.get("create_at")
        .or_else(|| row.get("createAt"))
        .or_else(|| row.get("createdAt"))
        .and_then(Value::as_i64)
}

impl ImModule {
    /// 用当前 RuntimeAuth 构造本地 dialog Scan，不把 company/user 接受为 payload 意图。
    pub(crate) fn build_dialog_list_query_for_runtime(
        &self,
        payload: &[u8],
        corr: helix_core::Correlation,
    ) -> Result<Effect, ImError> {
        let scope = super::DialogListScope::new(
            self.config.auth_user_id.as_str(),
            self.config.company_id.as_str(),
        );
        super::build_dialog_list_query_for_scope(payload, corr, &scope)
    }

    /// 将 dialog Scan 回报交给唯一 typed Result 通道，失败仍返回空 items。
    pub(crate) fn emit_dialog_list_result_for_runtime(
        &self,
        req_id: Option<&str>,
        reply_bytes: &[u8],
    ) -> Effect {
        let scope = super::DialogListScope::new(
            self.config.auth_user_id.as_str(),
            self.config.company_id.as_str(),
        );
        super::emit_dialog_list_result(req_id.unwrap_or_default(), reply_bytes, &scope)
    }

    /// 显式 retry 在 HTTP admission 后强制读取 Go 最新消息完成幂等对账。
    pub(crate) fn start_authoritative_send_readback(
        &mut self,
        channel_id: ChannelId,
        window_token: Option<&str>,
        causation_id: Option<String>,
        temporary_id: crate::state::TemporaryId,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let window_token = window_token
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                self.state
                    .timeline_state
                    .unique_attached_window_for_channel(channel_id.as_str())
                    .map(|(token, _)| token)
            })
            .unwrap_or_else(|| "latest".to_string());
        let scope = crate::timeline_state::TimelineScope {
            channel_id: channel_id.as_str().to_string(),
            window_token: window_token.clone(),
        };
        let limit = authoritative_readback_page_size(
            self.state
                .timeline_state
                .current_view(&scope)
                .map(|view| view.items.len()),
        );
        let request = MessageQueryRequest {
            channel_id,
            limit,
            window_token,
        };
        let query_generation = self
            .state
            .begin_message_query_generation(channel_id, request.window_token.as_str());
        self.start_remote_message_query(
            request,
            Vec::new(),
            query_generation,
            causation_id,
            None,
            Some(temporary_id),
            out,
        )
    }

    /// A durable mutation may change an already attached latest timeline without
    /// another renderer intent. Re-enter the existing local-first projector
    /// after the write acknowledgement instead of leaking a legacy `im:post:*`
    /// payload to a client-side reducer.
    pub(crate) fn refresh_attached_latest_timeline(
        &mut self,
        channel_id: ChannelId,
        causation_id: Option<String>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.refresh_attached_latest_timeline_with_deferred_send(
            channel_id,
            causation_id,
            None,
            out,
        )
        .map(|_| ())
    }

    /// 对已附着的指定窗口执行权威读回。
    pub(crate) fn refresh_attached_timeline(
        &mut self,
        channel_id: ChannelId,
        window_token: &str,
        causation_id: Option<String>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.refresh_attached_timeline_window_with_deferred_send(
            channel_id,
            window_token,
            causation_id,
            None,
            out,
        )
        .map(|_| ())
    }

    /// 已附着窗口先读回持久事实，再释放普通消息 HTTP。
    pub(crate) fn refresh_attached_latest_timeline_with_deferred_send(
        &mut self,
        channel_id: ChannelId,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let Some((window_token, _)) = self
            .state
            .timeline_state
            .unique_attached_window_for_channel(channel_id.as_str())
        else {
            return Ok(false);
        };
        self.refresh_attached_timeline_window_with_deferred_send(
            channel_id,
            window_token.as_str(),
            causation_id,
            deferred_send_http,
            out,
        )
    }

    /// 写后回读为新事实预留一个槽位，避免短窗口用固定页长挤掉仍可见的旧消息。
    fn refresh_attached_timeline_window_with_deferred_send(
        &mut self,
        channel_id: ChannelId,
        window_token: &str,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let scope = crate::timeline_state::TimelineScope {
            channel_id: channel_id.as_str().to_string(),
            window_token: window_token.to_string(),
        };
        if !self.state.timeline_state.is_attached(&scope) {
            return Ok(false);
        }
        let visible_limit = self
            .state
            .timeline_state
            .current_view(&scope)
            .map(|view| view.items.len())
            .filter(|visible| *visible > 0)
            .and_then(|visible| u32::try_from(visible).ok())
            .map(|visible| {
                visible
                    .saturating_add(1)
                    .min(crate::timeline_state::MAX_TIMELINE_WINDOW_ITEMS as u32)
            })
            .unwrap_or(super::QUERY_MESSAGES_DEFAULT);
        let payload = serde_json::json!({
            "channel_id": channel_id.as_str(),
            "window_token": window_token,
            "limit": visible_limit,
        });
        let bytes =
            serde_json::to_vec(&payload) // hot-path-audit: ignore - HTTP wire body，不写入单列。
                .map_err(|error| {
                    ImError::Serialize(format!("attached timeline refresh: {error}"))
                })?;
        self.dispatch_message_query_with_causation_and_deferred_send(
            &bytes,
            false,
            causation_id,
            deferred_send_http,
            out,
        )?;
        Ok(true)
    }

    /// `im_query_messages_by_channel` 的唯一异步入口。
    pub(crate) fn dispatch_message_query(
        &mut self,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.dispatch_message_query_with_causation_and_deferred_send(payload, true, None, None, out)
    }

    /// 执行 local-first 查询并保留内部因果键与发送 continuation。
    fn dispatch_message_query_with_causation_and_deferred_send(
        &mut self,
        payload: &[u8],
        allow_remote_fallback: bool,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let request = super::parse_message_query(payload)?;
        let query_generation = self
            .state
            .begin_message_query_generation(request.channel_id, request.window_token.as_str());
        let corr = self.alloc_corr_internal();
        out.push(super::build_message_query_from_request(&request, corr));
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageQueryLocal {
                request: Box::new(request),
                query_session_epoch: self.state.query_session_epoch,
                query_generation,
                allow_remote_fallback,
                causation_id,
                deferred_send_http,
            },
        );
        Ok(())
    }

    pub(crate) fn handle_message_query_local_reply(
        &mut self,
        request: MessageQueryRequest,
        query_session_epoch: u64,
        query_generation: u64,
        allow_remote_fallback: bool,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        now_ms: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !self.is_current_message_query(
            request.channel_id,
            &request.window_token,
            query_session_epoch,
            query_generation,
        ) {
            return Ok(());
        }
        let mut local_rows_desc = match outcome {
            PortOutcome::Ok(reply) => match parse_local_rows(reply.0.as_ref()) {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        channel_id = request.channel_id.as_str(),
                        error = ?error,
                        allow_remote_fallback,
                        "message query local scan reply malformed"
                    );
                    if !allow_remote_fallback {
                        out.push(self.emit_timeline_failed(&request, now_ms, causation_id)?);
                        self.emit_deferred_posts_create_after_timeline_event(
                            deferred_send_http,
                            out,
                        )?;
                        return Ok(());
                    }
                    Vec::new()
                }
            },
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    allow_remote_fallback,
                    "message query local scan failed"
                );
                if !allow_remote_fallback {
                    out.push(self.emit_timeline_failed(&request, now_ms, causation_id)?);
                    self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
                    return Ok(());
                }
                Vec::new()
            }
        };
        sort_recent_rows_desc(&mut local_rows_desc);

        if !allow_remote_fallback {
            out.push(self.emit_timeline_snapshot_with_causation(
                &request,
                &local_rows_desc,
                now_ms,
                causation_id,
                None,
            )?);
            self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
            return Ok(());
        }

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
            out.push(self.emit_timeline_snapshot_with_causation(
                &request,
                &local_rows_desc,
                now_ms,
                causation_id,
                None,
            )?);
            self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
            return Ok(());
        }
        self.start_remote_message_query(
            request,
            local_rows_desc,
            query_generation,
            causation_id,
            deferred_send_http,
            None,
            out,
        )
    }

    /// 解析 Go 权威最近消息并落 durable cache；最终窗口必须等待后续 Scan read-back。
    pub(crate) fn handle_message_query_remote_reply(
        &mut self,
        request: MessageQueryRequest,
        local_rows_desc: Vec<Value>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        authoritative_send_readback: Option<crate::state::TemporaryId>,
        now_ms: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !self.is_current_message_query(
            request.channel_id,
            &request.window_token,
            query_session_epoch,
            query_generation,
        ) {
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
                out.push(self.emit_timeline_failed(&request, now_ms, causation_id)?);
                self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
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
                out.push(self.emit_timeline_failed(&request, now_ms, causation_id)?);
                self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
                return Ok(());
            }
        };
        let received_count = remote_posts.len();
        if let Some(temporary_id) = authoritative_send_readback {
            self.reconcile_authoritative_send_readback(
                request.channel_id,
                &temporary_id,
                &remote_posts,
                out,
            );
        }
        let (mut remote_rows_desc, mut cache_ops) = match visible_remote_rows_and_cache_ops(
            request.channel_id,
            remote_posts,
            &local_rows_desc,
            self.config.auth_user_id.as_str(),
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query remote fallback contained invalid posts"
                );
                out.push(self.emit_timeline_failed(&request, now_ms, causation_id)?);
                self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
                return Ok(());
            }
        };
        dedup_recent_rows(&mut remote_rows_desc);
        sort_recent_rows_desc(&mut remote_rows_desc);

        let remote_exhausted = recent_reply_proves_history_exhausted(
            &local_rows_desc,
            &remote_rows_desc,
            received_count,
        );
        let coverage = RecentMessageCoverage::from_remote(&remote_rows_desc, remote_exhausted);
        if remote_exhausted {
            if let Some(delete) = stale_local_server_rows_delete_op(
                request.channel_id,
                &local_rows_desc,
                &remote_rows_desc,
            ) {
                cache_ops.push(delete);
            }
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
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
            },
        );
        Ok(())
    }

    /// cache Persist 成功后只发起 durable message Scan，失败则不触碰已附着窗口。
    pub(crate) fn handle_message_query_cache_reply(
        &mut self,
        request: MessageQueryRequest,
        coverage: Option<RecentMessageCoverage>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        now_ms: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !self.is_current_message_query(
            request.channel_id,
            &request.window_token,
            query_session_epoch,
            query_generation,
        ) {
            return Ok(());
        }
        if let PortOutcome::Err(error) = outcome {
            tracing::warn!(
                channel_id = request.channel_id.as_str(),
                error = ?error,
                "message query remote cache failed; preserving previous timeline"
            );
            out.push(self.emit_timeline_failed(&request, now_ms, causation_id)?);
            self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
            return Ok(());
        }

        // Cache success is only a barrier; the next Scan is the sole source of render rows.
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![helix_core::effect::StorageOp::Scan(
                super::message_scan_spec(&request),
            )],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::MessageQueryReadback {
                request: Box::new(request),
                coverage,
                query_session_epoch,
                query_generation,
                causation_id,
                deferred_send_http,
            },
        );
        Ok(())
    }

    /// 读取 cache Persist 后的 durable message rows，并发布唯一 timeline 终态。
    pub(crate) fn handle_message_query_readback_reply(
        &mut self,
        request: MessageQueryRequest,
        coverage: Option<RecentMessageCoverage>,
        query_session_epoch: u64,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        now_ms: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !self.is_current_message_query(
            request.channel_id,
            &request.window_token,
            query_session_epoch,
            query_generation,
        ) {
            return Ok(());
        }
        let mut rows_desc = match outcome {
            PortOutcome::Ok(reply) => match parse_local_rows(reply.0.as_ref()) {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        channel_id = request.channel_id.as_str(),
                        error = ?error,
                        "message query durable read-back malformed"
                    );
                    out.push(self.emit_timeline_failed(&request, now_ms, causation_id)?);
                    self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
                    return Ok(());
                }
            },
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = request.channel_id.as_str(),
                    error = ?error,
                    "message query durable read-back failed; preserving previous timeline"
                );
                out.push(self.emit_timeline_failed(&request, now_ms, causation_id)?);
                self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
                return Ok(());
            }
        };
        sort_recent_rows_desc(&mut rows_desc);
        if let Some(coverage) = coverage {
            self.state
                .recent_message_coverage
                .insert(request.channel_id, coverage);
        }
        out.push(self.emit_timeline_snapshot_with_causation(
            &request,
            &rows_desc,
            now_ms,
            causation_id,
            None,
        )?);
        self.emit_deferred_posts_create_after_timeline_event(deferred_send_http, out)?;
        Ok(())
    }

    /// 发起单次 Go `getLatestPost` 查询并登记其 query/retry continuation。
    fn start_remote_message_query(
        &mut self,
        request: MessageQueryRequest,
        local_rows_desc: Vec<Value>,
        query_generation: u64,
        causation_id: Option<String>,
        deferred_send_http: Option<crate::state::TemporaryId>,
        authoritative_send_readback: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let payload = serde_json::to_vec(&serde_json::json!({
            "channel_id": request.channel_id.as_str(),
            "timestamp": 0,
            "cursor_version": 1,
            "page_size": request.limit,
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
                query_generation,
                causation_id,
                deferred_send_http,
                authoritative_send_readback,
            },
        );
        Ok(())
    }

    /// 用 Go 历史中的同 temporaryId + 合法 server id 结算 retry，并撤销其超时令牌。
    fn reconcile_authoritative_send_readback(
        &mut self,
        channel_id: ChannelId,
        temporary_id: &crate::state::TemporaryId,
        posts: &[Value],
        out: &mut EffectSink,
    ) {
        let server_id = posts.iter().find_map(|post| {
            let fields = crate::ws::parser::extract_post_fields(post);
            (fields.temporary_id == temporary_id.0)
                .then(|| crate::state::ServerId::from_str(fields.id.as_str()))
                .flatten()
        });
        let Some(server_id) = server_id else {
            return;
        };
        let context = CorrelationContext::AuthoritativeSendReconcilePersist {
            temporary_id: temporary_id.clone(),
        };
        if !self.settle_authoritative_send(temporary_id, server_id, context, out) {
            return;
        }
        tracing::info!(
            channel_id = channel_id.as_str(),
            tmp_id = temporary_id.0.as_str(),
            "explicit retry settled from authoritative history"
        );
    }

    /// 将已验证 server id 持久化到同一 pending 聚合，并登记后续 continuation。
    fn settle_authoritative_send(
        &mut self,
        temporary_id: &crate::state::TemporaryId,
        server_id: crate::state::ServerId,
        continuation: CorrelationContext,
        out: &mut EffectSink,
    ) -> bool {
        let Some(persist_corr) = self
            .state
            .pending_sends
            .get(temporary_id)
            .map(|pending| pending.persist_corr)
        else {
            tracing::warn!(
                tmp_id = temporary_id.0.as_str(),
                "authoritative retry matched durable authority without a pending aggregate"
            );
            return false;
        };
        let reconcile_corr = self.alloc_corr_internal();
        if let Some(pending) = self.state.pending_sends.get_mut(temporary_id) {
            pending.reconcile(server_id, reconcile_corr, out);
        }
        self.state.pending_sends.remove(temporary_id);
        if let Some(persist_corr) = persist_corr {
            self.state.corr_map.remove(&persist_corr);
        }
        self.state.corr_map.insert(reconcile_corr, continuation);
        true
    }

    fn emit_deferred_posts_create_after_timeline_event(
        &mut self,
        deferred_send_http: Option<crate::state::TemporaryId>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Some(temporary_id) = deferred_send_http else {
            return Ok(());
        };
        let (channel_id, body) = self
            .state
            .pending_sends
            .get(&temporary_id)
            .and_then(|pending| {
                pending.body.as_ref().and_then(|body| {
                    body.get("channelId")
                        .and_then(serde_json::Value::as_str)
                        .and_then(ChannelId::from_str)
                        .map(|channel_id| (channel_id, body.clone()))
                })
            })
            .ok_or_else(|| {
                ImError::Parse(format!(
                    "deferred posts/create missing pending send body: {}",
                    temporary_id.0
                ))
            })?;
        self.emit_posts_create_http(channel_id, temporary_id, &body, out)
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

    fn is_current_message_query(
        &self,
        channel_id: ChannelId,
        window_token: &str,
        query_session_epoch: u64,
        query_generation: u64,
    ) -> bool {
        query_session_epoch == self.state.query_session_epoch
            && self.state.is_current_message_query_generation(
                channel_id,
                window_token,
                query_generation,
            )
    }

    /// local-first 已完成排序、去重、权限与 render-ready shaping 后的唯一 timeline 出口。
    /// `rows_desc` 可含一条本地 lookahead；这里只投影请求页长，并把额外行转成分页事实。
    pub(crate) fn emit_timeline_snapshot_with_causation(
        &mut self,
        request: &MessageQueryRequest,
        rows_desc: &[Value],
        _now_ms: u64,
        causation_id: Option<String>,
        page_override: Option<crate::timeline_state::WindowPage>,
    ) -> Result<Effect, ImError> {
        let terminal_request = causation_id.clone();
        let scope = crate::timeline_state::TimelineScope {
            channel_id: request.channel_id.as_str().to_string(),
            window_token: request.window_token.to_string(),
        };
        let current_view = self.state.timeline_state.current_view(&scope);
        let anchored_window = current_view.is_some_and(|view| {
            view.anchor.mode == crate::timeline_state::TimelineAnchorMode::Locate
        });
        let anchored_create_at = current_view.and_then(|view| {
            let anchor_id = view.anchor.message_id.as_deref()?;
            view.items
                .iter()
                .find(|item| item.id == anchor_id)
                .map(|item| item.created_at)
        });
        let anchored_page_bounds = current_view
            .filter(|_| anchored_window)
            .map(|view| (view.page.has_older, view.page.has_newer, view.page.has_more));
        let had_attached_window = current_view.is_some();
        let visible_len = current_view
            .filter(|view| view.page.has_older)
            .map_or(request.limit as usize, |view| view.items.len());
        let has_local_older = rows_desc.len() > visible_len;
        let rows_asc = Value::Array(rows_desc.iter().take(visible_len).rev().cloned().collect());
        let shaped = crate::render_ready::shape_message_rows_for_viewer(
            &rows_asc,
            self.config.auth_user_id.as_str(),
        );
        let rows = shaped.as_array().ok_or_else(|| {
            ImError::Parse("render-ready timeline rows must be an array".to_string())
        })?;
        let mut timeline_request =
            crate::timeline_state::TimelineWindowRequest::latest_with_window_token(
                request.channel_id.as_str(),
                request.window_token.as_str(),
            );
        timeline_request.page_size = visible_len as u32;
        if let Some(page) = page_override {
            timeline_request.page = page;
        } else if let Some((has_older, has_newer, has_more)) = anchored_page_bounds {
            timeline_request.page.has_older = has_older;
            timeline_request.page.has_newer = has_newer;
            timeline_request.page.has_more = has_more;
        } else if has_local_older
            || self
                .state
                .recent_message_coverage
                .get(&request.channel_id)
                .is_some_and(|coverage| !coverage.remote_exhausted)
        {
            // A full authoritative recent window proves only that older rows
            // may exist. Preserve that proof in the render contract so shells
            // can expose the bounded load-older action without guessing from
            // the visible row count.
            timeline_request.page.has_older = true;
            timeline_request.page.has_more = true;
        }
        let has_older = timeline_request.page.has_older;
        let has_newer = timeline_request.page.has_newer;
        if anchored_window && timeline_request.target_message_id.is_none() {
            // 离底窗口收到 durable 新消息时只向新侧扩展；不得用 latest scan 隐式重定位。
            self.state.timeline_state.patch_page_from_render_ready(
                timeline_request,
                rows,
                crate::timeline_state::TimelinePageMutation::Newer,
                causation_id,
            )
        } else if self.state.timeline_state.current_view(&scope).is_some() {
            self.state
                .timeline_state
                .patch_from_render_ready(timeline_request, rows, causation_id)
        } else {
            self.state
                .timeline_state
                .snapshot_from_render_ready_with_causation(timeline_request, rows, causation_id)
        }
        .map_err(|error| ImError::Parse(format!("timeline state: {error}")))?;
        let event_rows = rows_asc.as_array().cloned().unwrap_or_default();
        let anchor_post_id = self
            .state
            .timeline_state
            .current_view(&scope)
            .and_then(|view| view.anchor.message_id.as_deref());
        let effect = if !had_attached_window {
            crate::event::timeline::window(
                request.channel_id.as_str(),
                request.window_token.as_str(),
                "ready",
                event_rows,
                has_older,
                has_newer,
                None,
            )?
        } else if anchored_window {
            let anchor_create_at = event_rows
                .iter()
                .find(|row| row.get("id").and_then(Value::as_str) == anchor_post_id)
                .and_then(|row| row.get("create_at").or_else(|| row.get("createAt")))
                .and_then(Value::as_i64)
                .or(anchored_create_at);
            let newer_count = event_rows
                .iter()
                .filter(|row| {
                    let create_at = row
                        .get("create_at")
                        .or_else(|| row.get("createAt"))
                        .and_then(Value::as_i64);
                    create_at.zip(anchor_create_at).is_some_and(
                        |(message_create_at, anchor_create_at)| {
                            message_create_at > anchor_create_at
                        },
                    )
                })
                .count();
            crate::event::timeline::anchored_update(
                request.channel_id.as_str(),
                request.window_token.as_str(),
                "ready",
                event_rows,
                has_older,
                has_newer,
                anchor_post_id,
                newer_count,
            )?
        } else {
            crate::event::timeline::page(
                request.channel_id.as_str(),
                request.window_token.as_str(),
                "append",
                "ready",
                event_rows,
                has_older,
                has_newer,
                anchor_post_id,
            )?
        }
        .into_effect();
        if let Some(request_id) = terminal_request {
            self.state
                .pending_forward_deliveries
                .complete_target(&request_id, request.channel_id);
        }
        Ok(effect)
    }

    /// 把已持久化并读回确认的 V3 导航页投影为同一 attached slot 的原子 Delta。
    pub(crate) fn emit_timeline_navigation_page(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        _now_ms: u64,
    ) -> Result<Effect, ImError> {
        let rows = Value::Array(state.rows().to_vec());
        let shaped = crate::render_ready::shape_message_rows_for_viewer(
            &rows,
            self.config.auth_user_id.as_str(),
        );
        let rows = shaped.as_array().ok_or_else(|| {
            ImError::Parse("timeline navigation rows must shape to array".to_string())
        })?;
        let mut request = crate::timeline_state::TimelineWindowRequest::latest_with_window_token(
            state.channel_id().as_str(),
            state.window_token(),
        );
        request.page_size = state.page_size();
        request.page = state.page();
        let locate_is_current = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Locate {
                navigation_token, ..
            } => self.state.timeline_state.is_current_locate_navigation(
                state.channel_id().as_str(),
                state.window_token(),
                navigation_token,
            ),
            _ => true,
        };
        let page_mutation = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Older { .. } => {
                crate::timeline_state::TimelinePageMutation::Older
            }
            crate::timeline_navigation::TimelineNavigationKind::Newer { .. } => {
                crate::timeline_state::TimelinePageMutation::Newer
            }
            crate::timeline_navigation::TimelineNavigationKind::Locate {
                target_message_id,
                navigation_token,
            } => crate::timeline_state::TimelinePageMutation::Locate {
                target_message_id: target_message_id.to_string(),
                navigation_token: navigation_token.to_string(),
                activate: locate_is_current,
            },
        };
        self.state
            .timeline_state
            .patch_page_from_render_ready(
                request,
                rows,
                page_mutation,
                state.request_id().map(str::to_string),
            )
            .map_err(|error| ImError::Parse(format!("timeline navigation projection: {error}")))?;
        let page_direction = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Older {
                anchor_post_id, ..
            } => Some(("older", anchor_post_id.as_str())),
            crate::timeline_navigation::TimelineNavigationKind::Newer {
                anchor_post_id, ..
            } => Some(("newer", anchor_post_id.as_str())),
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. } => None,
        };
        if let Some((direction, anchor_post_id)) = page_direction {
            let page = state.page();
            let messages = state
                .rows()
                .iter()
                .filter(|row| row.get("id").and_then(Value::as_str) != Some(anchor_post_id))
                .cloned()
                .collect();
            return Ok(crate::event::timeline::page(
                state.channel_id().as_str(),
                state.window_token(),
                direction,
                "ready",
                messages,
                page.has_older,
                page.has_newer,
                Some(anchor_post_id),
            )?
            .into_effect());
        }
        let crate::timeline_navigation::TimelineNavigationKind::Locate {
            target_message_id,
            navigation_token,
        } = state.kind()
        else {
            return Err(ImError::Parse(
                "timeline navigation kind changed after page dispatch".to_string(),
            ));
        };
        let page = state.page();
        if !locate_is_current {
            return Ok(crate::event::timeline::page(
                state.channel_id().as_str(),
                state.window_token(),
                "merge",
                "stale",
                state.rows().to_vec(),
                page.has_older,
                page.has_newer,
                Some(target_message_id),
            )?
            .into_effect());
        }
        Ok(crate::event::timeline::located(serde_json::json!({
            "channelId": state.channel_id().as_str(),
            "windowToken": state.window_token(),
            "state": "ready",
            "messages": state.rows(),
            "hasOlder": page.has_older,
            "hasNewer": page.has_newer,
            "targetMessageId": target_message_id,
            "anchorPostId": target_message_id,
            "revealPostId": target_message_id,
            "navigationToken": navigation_token,
        }))?
        .into_effect())
    }

    fn emit_timeline_failed(
        &mut self,
        request: &MessageQueryRequest,
        _now_ms: u64,
        _causation_id: Option<String>,
    ) -> Result<Effect, ImError> {
        Ok(crate::event::timeline::window(
            request.channel_id.as_str(),
            request.window_token.as_str(),
            "failed",
            Vec::new(),
            false,
            false,
            None,
        )?
        .into_effect())
    }
}
