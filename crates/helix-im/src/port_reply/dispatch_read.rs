use crate::http_envelope::unwrap_sync_envelope;
use crate::module::ImModule;
use crate::older_context::LoadOlderState;
use crate::state::CorrelationContext;
use crate::{error::ImError, query::LocalStoreMode};
use helix_core::effect::{Effect, ScanOrder, ScanSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::Value;
use std::collections::HashSet;

const OLDER_NAVIGATION_ORDER: &[ScanOrder] = &[
    ScanOrder::desc("create_at"),
    ScanOrder::desc("temporary_id"),
];
const NEWER_NAVIGATION_ORDER: &[ScanOrder] =
    &[ScanOrder::asc("create_at"), ScanOrder::asc("temporary_id")];

impl ImModule {
    /// 先走权威 coverage 命中；缺 coverage 时仅发起一次 Go HTTP。
    pub(crate) fn start_timeline_navigation_or_local(
        &mut self,
        state: crate::timeline_navigation::TimelineNavigationState,
        out: &mut EffectSink,
    ) {
        // Coverage 元数据在本轮 read-back 中建立；没有完整 metadata 不能把消息数组误当命中。
        let Some(request_id) = state.request_id() else {
            self.start_timeline_navigation_http(state, out);
            return;
        };
        if !self
            .state
            .timeline_navigation_pending
            .insert(request_id.to_string())
        {
            out.push(crate::read_relay::emit_read_error(
                request_id,
                "duplicate timeline reqId",
            ));
            return;
        }
        let key = state.coverage_key();
        let Some(coverage) = self.state.timeline_navigation_coverage.get(&key).cloned() else {
            self.start_timeline_navigation_http(state, out);
            return;
        };
        let Some(coverage_rows) =
            local_timeline_navigation_rows(&state, &coverage, self.config.auth_user_id.as_str())
        else {
            tracing::debug!(
                channel_id = state.channel_id().as_str(),
                operation = state.operation(),
                "timeline navigation coverage is incomplete; falling back to authority"
            );
            self.start_timeline_navigation_http(state, out);
            return;
        };
        let shaped = crate::render_ready::shape_message_rows_for_viewer(
            &Value::Array(coverage_rows.clone()),
            self.config.auth_user_id.as_str(),
        );
        let mut body = serde_json::json!({
            "reqId": request_id,
            "channelId": state.channel_id().as_str(),
            "operation": state.operation(),
            "messages": shaped,
            "hasOlder": coverage.has_older,
            "hasNewer": coverage.has_newer,
            "pageSize": state.page_size(),
            "olderCursor": coverage.older_cursor.clone().unwrap_or(Value::Null),
            "newerCursor": coverage.newer_cursor.clone().unwrap_or(Value::Null),
        });
        if matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. }
        ) {
            body["targetPostId"] = serde_json::json!(state.anchor_message_id());
            let Some(target_index) = coverage.target_index else {
                self.start_timeline_navigation_http(state, out);
                return;
            };
            body["targetIndex"] = serde_json::json!(target_index);
        }
        self.clear_timeline_navigation_pending(&state);
        out.push(crate::read_relay::emit_read_body(request_id, body));
    }

    /// 释放已结算的 reqId，避免成功/失败后的重试被旧 pending 守卫误判为重复。
    fn clear_timeline_navigation_pending(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
    ) {
        if let Some(request_id) = state.request_id() {
            self.state.timeline_navigation_pending.remove(request_id);
        }
    }

    /// 记录已完成远端权限过滤和 durable read-back 的 coverage 元数据。
    fn remember_timeline_navigation_coverage(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        rows: &[Value],
        target_index: Option<usize>,
    ) {
        let page = state.page();
        self.state.timeline_navigation_coverage.insert(
            state.coverage_key(),
            crate::timeline_navigation::TimelineNavigationCoverage {
                channel_id: state.channel_id(),
                rows: rows.to_vec(),
                has_older: page.has_older,
                has_newer: page.has_newer,
                target_index,
                older_cursor: state.older_cursor().cloned(),
                newer_cursor: state.newer_cursor().cloned(),
            },
        );
    }

    /// 发起 Timeline V3 authority HTTP，并注册唯一 correlation continuation。
    pub(crate) fn start_timeline_navigation_http(
        &mut self,
        state: crate::timeline_navigation::TimelineNavigationState,
        out: &mut EffectSink,
    ) {
        let corr = self.alloc_corr_internal();
        out.push(crate::timeline_navigation::navigation_http(
            &state,
            self.config.api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        ));
        self.state.corr_map.insert(
            corr,
            CorrelationContext::TimelineNavigationHttp {
                state: Box::new(state),
            },
        );
    }

    /// 校验 V3 HTTP 页并把可见消息送入 matching Persist barrier。
    pub(super) fn handle_timeline_navigation_http_reply(
        &mut self,
        mut state: Box<crate::timeline_navigation::TimelineNavigationState>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // G11b-G11d 的 HTTP continuation 只由 owned corr/reqId/channel state 关联；
        // renderer 窗口 token 可能被并发 latest 刷新替换，不能据此否决仍有效的命令回包。
        let reply = match outcome {
            PortOutcome::Ok(reply) => reply,
            PortOutcome::Err(error) => {
                tracing::warn!(error = ?error, "timeline navigation HTTP failed");
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        let raw = match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(raw) => raw,
            Err(error) => {
                tracing::warn!(error = ?error, "timeline navigation envelope invalid");
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        if let Err(error) = state.ingest_http_body(&raw) {
            tracing::warn!(error = ?error, "timeline navigation body invalid");
            self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            return Ok(());
        }
        if matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. }
        ) {
            let rows = match crate::query::render_ready::locate::filter_visible_remote_rows(
                state.rows(),
                state.channel_id().as_str(),
                state.anchor_message_id(),
                self.config.auth_user_id.as_str(),
            ) {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "timeline locate target is not visible in HTTP authority"
                    );
                    self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                    return Ok(());
                }
            };
            let target_index = rows.iter().position(|row| {
                crate::timeline_navigation::row_matches_message_identity(
                    row,
                    state.anchor_message_id(),
                )
            });
            state.set_target_index(target_index);
            state.replace_rows(rows);
        }
        let (rows, ops) = match crate::query::local_first::visible_remote_rows_and_cache_ops(
            state.channel_id(),
            state.rows().to_vec(),
            &[],
            self.config.auth_user_id.as_str(),
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "timeline navigation rows could not become durable messages"
                );
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        state.replace_rows(rows);
        if self.local_store_mode == LocalStoreMode::Disabled {
            tracing::warn!(
                channel_id = state.channel_id().as_str(),
                "timeline navigation requires durable storage before event publication"
            );
            self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            return Ok(());
        }
        // 空 authority 页没有可写消息；跳过空 Persist，直接进入 bounded Scan，
        // 兼容真实 C++ storage bridge 不为零操作写集发送回执的运行时合同。
        if ops.is_empty() {
            self.schedule_timeline_navigation_readback(state, out)?;
            return Ok(());
        }
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist { corr, ops });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::TimelineNavigationCache { state });
        Ok(())
    }

    /// Publishes a bounded failed delta for the existing cursor-based Timeline window.
    pub(super) fn emit_timeline_navigation_failed_event(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        self.clear_timeline_navigation_pending(state);
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
            if let Some(request_id) = state.request_id() {
                let message = if direction == "older" {
                    "timeline older query failed before durable result"
                } else {
                    "timeline newer query failed before durable result"
                };
                out.push(crate::read_relay::emit_read_error(request_id, message));
                return Ok(());
            }
            let page = state.page();
            out.push(
                crate::event::timeline::page(
                    state.channel_id().as_str(),
                    state.window_token(),
                    direction,
                    "failed",
                    Vec::new(),
                    page.has_older,
                    page.has_newer,
                    Some(anchor_post_id),
                )?
                .into_effect(),
            );
            return Ok(());
        }
        let crate::timeline_navigation::TimelineNavigationKind::Locate {
            navigation_token, ..
        } = state.kind()
        else {
            return Ok(());
        };
        // Locate window lifecycle filtering belongs to Angular; Helix only correlates reqId.
        let _ = navigation_token;
        if let Some(request_id) = state.request_id() {
            out.push(crate::read_relay::emit_read_error(
                request_id,
                "timeline locate query failed before durable result",
            ));
        }
        Ok(())
    }

    /// PersistOk 后只读回页内权威行；该 readback 完成前不得发布成功事件。
    pub(super) fn schedule_timeline_navigation_readback(
        &mut self,
        state: Box<crate::timeline_navigation::TimelineNavigationState>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let exact_keys = timeline_navigation_readback_keys(&state);
        if !exact_keys.is_empty() {
            // V3 authority 页内每条 identity 都单独读回，禁止频道最新页冒充本次结果。
            self.schedule_timeline_navigation_exact_readback(state, exact_keys, 1, Vec::new(), out);
            return Ok(());
        }
        if matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. }
        ) {
            tracing::warn!(
                channel_id = state.channel_id().as_str(),
                "timeline locate authority page has no durable identity"
            );
            self.emit_timeline_navigation_failed_event(&state, 0, out)?;
            return Ok(());
        }
        let corr = self.alloc_corr_internal();
        let effect = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Older { .. } => {
                // Older 页面必须回读一个有界 message Scan；单行 Get 只能证明某条消息存在，
                // 无法证明返回页、去重、排序和 viewer 过滤都来自 durable storage。
                Effect::Persist {
                    corr,
                    ops: vec![StorageOp::Scan(older_navigation_scan_spec(&state))],
                }
            }
            crate::timeline_navigation::TimelineNavigationKind::Newer { .. } => {
                // Newer 同样必须回读完整的尾部 bounded Scan，不能用单行 identity 旁路结算。
                Effect::Persist {
                    corr,
                    ops: vec![StorageOp::Scan(newer_navigation_scan_spec(&state))],
                }
            }
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. } => {
                unreachable!("locate readback must use exact identity continuation")
            }
        };
        out.push(effect);
        self.state.corr_map.insert(
            corr,
            CorrelationContext::TimelineNavigationReadback { state },
        );
        Ok(())
    }

    /// 为分页页内下一个 authority identity 发出精确 durable Scan。
    fn schedule_timeline_navigation_exact_readback(
        &mut self,
        state: Box<crate::timeline_navigation::TimelineNavigationState>,
        accepted_keys: Vec<crate::timeline_navigation::TimelineNavigationReadbackKey>,
        next_index: usize,
        rows: Vec<Value>,
        out: &mut EffectSink,
    ) {
        let key = accepted_keys[next_index - 1].clone();
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Scan(timeline_navigation_exact_scan_spec(&key))],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::TimelineNavigationExactReadback {
                state,
                accepted_keys,
                next_index,
                rows,
            },
        );
    }

    /// 处理分页页内逐项 durable readback，并在最后一条 identity 后只结算一次。
    pub(super) fn handle_timeline_navigation_exact_readback_reply(
        &mut self,
        mut state: Box<crate::timeline_navigation::TimelineNavigationState>,
        accepted_keys: Vec<crate::timeline_navigation::TimelineNavigationReadbackKey>,
        next_index: usize,
        mut rows: Vec<Value>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let expected_key = accepted_keys
            .get(next_index.saturating_sub(1))
            .ok_or_else(|| {
                ImError::Parse("timeline exact readback index out of bounds".to_string())
            })?;
        let durable_rows = match outcome {
            PortOutcome::Ok(reply) => {
                match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                    Ok(rows) => rows,
                    Err(error) => {
                        tracing::warn!(
                            channel_id = state.channel_id().as_str(),
                            error = ?error,
                            "timeline exact durable read-back malformed"
                        );
                        self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                        return Ok(());
                    }
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "timeline exact durable read-back failed"
                );
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                return Ok(());
            }
        };
        let mut seen = rows
            .iter()
            .filter_map(|row| {
                accepted_keys
                    .iter()
                    .find(|key| durable_row_matches_readback_key(row, key))
                    .map(|key| (key.column, key.value.clone()))
            })
            .collect::<HashSet<_>>();
        let mut expected_found = false;
        for row in durable_rows {
            let Some(key) = accepted_keys
                .iter()
                .find(|key| durable_row_matches_readback_key(&row, key))
            else {
                continue;
            };
            expected_found |= key == expected_key;
            if seen.insert((key.column, key.value.clone())) {
                rows.push(row);
            }
        }
        if !expected_found {
            tracing::warn!(
                channel_id = state.channel_id().as_str(),
                expected_key = expected_key.value.as_str(),
                "timeline exact durable read-back missing authority identity"
            );
            self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            return Ok(());
        }

        let mut next = next_index;
        while next < accepted_keys.len() {
            if seen.contains(&(
                accepted_keys[next].column,
                accepted_keys[next].value.clone(),
            )) {
                next += 1;
            } else {
                break;
            }
        }
        if next < accepted_keys.len() {
            self.schedule_timeline_navigation_exact_readback(
                state,
                accepted_keys,
                next + 1,
                rows,
                out,
            );
            return Ok(());
        }

        let projected_rows = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Older { .. } => {
                older_navigation_rows(&state, rows, self.config.auth_user_id.as_str())
            }
            crate::timeline_navigation::TimelineNavigationKind::Newer { .. } => {
                newer_navigation_rows(&state, rows, self.config.auth_user_id.as_str())
            }
            crate::timeline_navigation::TimelineNavigationKind::Locate {
                target_message_id,
                ..
            } => match crate::query::render_ready::locate::normalize_durable_located_window(
                rows,
                state.channel_id().as_str(),
                target_message_id,
                self.config.auth_user_id.as_str(),
                state.page_size(),
            ) {
                Ok(rows) => {
                    let target_index = rows.iter().position(|row| {
                        crate::timeline_navigation::row_matches_message_identity(
                            row,
                            target_message_id,
                        )
                    });
                    if target_index.is_none() {
                        self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                        return Ok(());
                    }
                    state.set_target_index(target_index);
                    rows
                }
                Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "timeline locate exact durable projection failed closed"
                    );
                    self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                    return Ok(());
                }
            },
        };
        if !matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. }
        ) && projected_rows.len() != accepted_keys.len()
        {
            tracing::warn!(
                channel_id = state.channel_id().as_str(),
                expected = accepted_keys.len(),
                actual = projected_rows.len(),
                "timeline exact durable read-back projected an incomplete page"
            );
            self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            return Ok(());
        }
        let target_index = match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. } => {
                state.target_index()
            }
            _ => None,
        };
        self.remember_timeline_navigation_coverage(&state, &projected_rows, target_index);
        match state.kind() {
            crate::timeline_navigation::TimelineNavigationKind::Newer { .. } => {
                match self.emit_timeline_navigation_newer_result(&state, &projected_rows) {
                    Ok(effect) => out.push(effect),
                    Err(error) => {
                        tracing::warn!(
                            channel_id = state.channel_id().as_str(),
                            error = ?error,
                            "timeline newer exact durable rows could not project"
                        );
                        self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                    }
                }
            }
            crate::timeline_navigation::TimelineNavigationKind::Older { .. } => {
                out.push(self.emit_timeline_navigation_result(&state, &projected_rows)?);
            }
            crate::timeline_navigation::TimelineNavigationKind::Locate { .. } => {
                out.push(self.emit_timeline_locate_result(&state, &projected_rows)?);
            }
        }
        Ok(())
    }

    /// 匹配的本地读回存在且身份一致时，释放唯一 production Timeline Delta。
    pub(super) fn handle_timeline_navigation_readback_reply(
        &mut self,
        state: Box<crate::timeline_navigation::TimelineNavigationState>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // Durable read-back 仍须匹配 owned corr；客户端负责窗口生命周期淘汰。
        if matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Newer { .. }
        ) {
            let rows = match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                        Ok(rows) => {
                            newer_navigation_rows(&state, rows, self.config.auth_user_id.as_str())
                        }
                        Err(error) => {
                            tracing::warn!(
                                channel_id = state.channel_id().as_str(),
                                error = ?error,
                                "timeline newer durable read-back malformed"
                            );
                            self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                            return Ok(());
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "timeline newer durable read-back failed"
                    );
                    self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                    return Ok(());
                }
            };
            if newer_http_has_rows(&state) && rows.is_empty() {
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            } else {
                self.remember_timeline_navigation_coverage(&state, &rows, None);
                match self.emit_timeline_navigation_newer_result(&state, &rows) {
                    Ok(effect) => out.push(effect),
                    Err(error) => {
                        tracing::warn!(
                            channel_id = state.channel_id().as_str(),
                            error = ?error,
                            "timeline newer durable rows could not project"
                        );
                        self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
                    }
                }
            }
            return Ok(());
        }
        if matches!(
            state.kind(),
            crate::timeline_navigation::TimelineNavigationKind::Older { .. }
        ) {
            let rows = match outcome {
                PortOutcome::Ok(reply) => {
                    match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                        Ok(rows) => {
                            older_navigation_rows(&state, rows, self.config.auth_user_id.as_str())
                        }
                        Err(error) => {
                            tracing::warn!(
                                channel_id = state.channel_id().as_str(),
                                error = ?error,
                                "timeline older durable read-back malformed"
                            );
                            Vec::new()
                        }
                    }
                }
                PortOutcome::Err(error) => {
                    tracing::warn!(
                        channel_id = state.channel_id().as_str(),
                        error = ?error,
                        "timeline older durable read-back failed"
                    );
                    Vec::new()
                }
            };
            // HTTP 只建立“期望有页”的事实；若该页确实有可见 rows，却没有任何
            // matching durable row，必须失败保旧，不能旁路用 HTTP rows 结算。
            if !state.authority_rows().is_empty() && rows.is_empty() {
                self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
            } else {
                self.remember_timeline_navigation_coverage(&state, &rows, None);
                out.push(self.emit_timeline_navigation_result(&state, &rows)?);
            }
            return Ok(());
        }
        // Locate 不再有 bounded channel Scan continuation；任何误路由都失败保旧。
        self.emit_timeline_navigation_failed_event(&state, now_ms, out)?;
        Ok(())
    }

    /// 将 durable older rows 投影成同 reqId 的结构化 QueryMessagesResult。
    fn emit_timeline_navigation_result(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        rows: &[Value],
    ) -> Result<Effect, ImError> {
        self.clear_timeline_navigation_pending(state);
        let request_id = state
            .request_id()
            .ok_or_else(|| ImError::Parse("timeline older result missing req_id".to_string()))?;
        let shaped = crate::render_ready::shape_message_rows_for_viewer(
            &Value::Array(rows.to_vec()),
            self.config.auth_user_id.as_str(),
        );
        let page = state.page();
        Ok(crate::read_relay::emit_read_body(
            request_id,
            serde_json::json!({
                "reqId": request_id,
                "channelId": state.channel_id().as_str(),
                "operation": state.operation(),
                "messages": shaped,
                "hasOlder": page.has_older,
                "hasNewer": page.has_newer,
                "pageSize": state.page_size(),
                "olderCursor": Value::Null,
                "newerCursor": Value::Null,
            }),
        ))
    }

    /// 将 durable newer rows 合并到附着窗口，并以唯一 reqId 结果结算；不发布 Timeline 领域广播。
    fn emit_timeline_navigation_newer_result(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        rows: &[Value],
    ) -> Result<Effect, ImError> {
        self.clear_timeline_navigation_pending(state);
        let request_id = state
            .request_id()
            .ok_or_else(|| ImError::Parse("timeline newer result missing req_id".to_string()))?;
        let shaped = crate::render_ready::shape_message_rows_for_viewer(
            &Value::Array(rows.to_vec()),
            self.config.auth_user_id.as_str(),
        );
        let shaped_rows = shaped.as_array().ok_or_else(|| {
            ImError::Parse("timeline newer durable rows must shape to array".to_string())
        })?;
        // Angular owns ViewerLocalWindow/merge; Helix only returns the absolute page.
        let _ = shaped_rows;
        let page = state.page();
        Ok(crate::read_relay::emit_read_body(
            request_id,
            serde_json::json!({
                "reqId": request_id,
                "channelId": state.channel_id().as_str(),
                "operation": state.operation(),
                "messages": shaped,
                "hasOlder": page.has_older,
                "hasNewer": page.has_newer,
                "pageSize": state.page_size(),
                "olderCursor": Value::Null,
                "newerCursor": Value::Null,
            }),
        ))
    }

    /// 应用已读回的 locate 窗口并返回唯一 QueryMessagesResult，不发布领域广播。
    fn emit_timeline_locate_result(
        &mut self,
        state: &crate::timeline_navigation::TimelineNavigationState,
        rows: &[Value],
    ) -> Result<Effect, ImError> {
        self.clear_timeline_navigation_pending(state);
        let request_id = state
            .request_id()
            .ok_or_else(|| ImError::Parse("timeline locate result missing req_id".to_string()))?;
        let crate::timeline_navigation::TimelineNavigationKind::Locate {
            target_message_id: _,
            navigation_token: _,
        } = state.kind()
        else {
            return Err(ImError::Parse(
                "timeline locate result requires locate state".to_string(),
            ));
        };
        // Locate center-scroll and window lifecycle handling belong to Angular.
        let page = state.page();
        Ok(crate::read_relay::emit_read_body(
            request_id,
            serde_json::json!({
                "reqId": request_id,
                "channelId": state.channel_id().as_str(),
                "operation": state.operation(),
                "messages": rows,
                "hasOlder": page.has_older,
                "hasNewer": page.has_newer,
                "pageSize": state.page_size(),
                "targetPostId": state.anchor_message_id(),
                "targetIndex": state.target_index(),
                "olderCursor": state.older_cursor().cloned().unwrap_or(Value::Null),
                "newerCursor": state.newer_cursor().cloned().unwrap_or(Value::Null),
            }),
        ))
    }

    /// 置顶权威读取先过失效代次，再持久化绝对集合并回灌原请求。
    pub(super) fn handle_outbound_pinned_reply(
        &mut self,
        req_id: String,
        account_id: String,
        channel_id: crate::state::ChannelId,
        projection_key: String,
        epoch: u64,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(reply) = outcome else {
            out.push(crate::read_relay::emit_read_error(
                req_id.as_str(),
                "http request failed",
            ));
            return Ok(());
        };
        let raw_body = match unwrap_sync_envelope(reply.0.as_ref()) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(req_id, error = ?error, "pinned read envelope decode failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id.as_str(),
                    "response envelope decode failed",
                ));
                return Ok(());
            }
        };
        let current_epoch = self
            .state
            .pinned_projection_epochs
            .get(&channel_id)
            .copied()
            .unwrap_or(0);
        if current_epoch != epoch {
            out.push(crate::read_relay::emit_read_result(
                req_id.as_str(),
                raw_body.as_ref(),
            ));
            return Ok(());
        }
        let corr = self.alloc_corr_internal();
        let persist = crate::query::pinned_projection::persist_effect(
            projection_key,
            account_id,
            channel_id,
            raw_body.as_ref(),
            corr,
        )?;
        self.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::PinnedProjectionPersist {
                req_id,
                raw_body: raw_body.to_vec(),
            },
        );
        out.push(persist);
        Ok(())
    }

    /// 通用 HTTP 读回按命令身份投影，并保留原始 req_id 完成 Host 调用。
    pub(super) fn handle_outbound_read_reply(
        &mut self,
        req_id: &String,
        command: &str,
        channel_id: Option<&str>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // 私有回应结果必须经过成员投影持久化及匹配读回，不能直接透传 HTTP 成功。
        if matches!(command, "im_reaction_unread" | "im_reaction_unread_ack") {
            return self.reaction_unread_http_reply(
                req_id.clone(),
                channel_id,
                outcome,
                now_ms,
                out,
            );
        }
        if command == crate::bookmark_count::COMMAND {
            return self.bookmark_count_reply(outcome, out);
        }
        if matches!(command, "im_bookmark_create" | "im_bookmark_delete") {
            self.bookmark_write_reply(command, outcome, out)?;
        }
        // 某些读回只有公告正文/成员数组，没有 channelId；必须使用请求 correlation 冻结的频道。
        if channel_id.is_some_and(|id| !self.render_scope.visible(&self.config, id)) {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "RENDER_SCOPE_MISMATCH",
            ));
            return Ok(());
        }
        match outcome {
            PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                Ok(raw_body) => {
                    if command == "im_bookmark_load_global" {
                        let mut body: Value =
                            serde_json::from_slice(&raw_body).unwrap_or(Value::Null);
                        let page = &body["data"];
                        let valid = body["status"] == "SUCCESS"
                            && !self.config.auth_user_id.is_empty()
                            && !self.config.company_id.is_empty()
                            && page["userId"].as_str() == Some(self.config.auth_user_id.as_str())
                            && page["companyId"].as_str() == Some(self.config.company_id.as_str())
                            && page["hasMore"].is_boolean();
                        let channels: Option<HashSet<String>> =
                            page["data"].as_array().and_then(|rows| {
                                rows.iter()
                                    .map(|row| {
                                        row["channelId"]
                                            .as_str()
                                            .filter(|id| !id.is_empty())
                                            .map(str::to_owned)
                                    })
                                    .collect()
                            });
                        if let Some(channels) = channels.filter(|_| valid) {
                            body["data"]["data"] =
                                crate::render_ready::shape_message_rows_for_viewer(
                                    &body["data"]["data"],
                                    self.config.auth_user_id.as_str(),
                                );
                            self.render_scope.authorize_read(req_id, channels);
                            out.push(crate::read_relay::emit_read_body(req_id, body));
                        } else {
                            out.push(crate::read_relay::emit_read_error(
                                req_id,
                                "BOOKMARK_GLOBAL_SCOPE_OR_RESPONSE_INVALID",
                            ));
                        }
                        return Ok(());
                    }
                    if command == "im_quick_reply" {
                        let body = serde_json::from_slice::<serde_json::Value>(&raw_body);
                        if !matches!(&body, Ok(value) if value.get("status").and_then(serde_json::Value::as_str) == Some("SUCCESS"))
                        {
                            out.push(crate::read_relay::emit_read_error(
                                req_id,
                                "QUICK_REPLY_FAILED",
                            ));
                            return Ok(());
                        }
                    }
                    if command == "im_channel_query" {
                        // 频道目录行是明确的 channel schema；不要让 id/teamId 简化形状
                        // 绕过公司围栏，也不要把通用成员行的 teamId 误当频道公司。
                        let body: serde_json::Value =
                            serde_json::from_slice(&raw_body).unwrap_or(serde_json::Value::Null);
                        let scoped = body
                            .get("data")
                            .and_then(serde_json::Value::as_array)
                            .is_some_and(|rows| {
                                rows.iter().all(|row| {
                                    let tenant = row
                                        .get("teamId")
                                        .or_else(|| row.get("team_id"))
                                        .and_then(serde_json::Value::as_str);
                                    !self.config.auth_user_id.is_empty()
                                        && tenant.is_some_and(|tenant| !tenant.is_empty())
                                        && (self.config.company_id.is_empty()
                                            || tenant == Some(self.config.company_id.as_str())
                                            || row
                                                .get("id")
                                                .and_then(serde_json::Value::as_str)
                                                .is_some_and(|id| {
                                                    self.render_scope.visible(&self.config, id)
                                                }))
                                })
                            });
                        if !scoped {
                            out.push(crate::read_relay::emit_read_error(
                                req_id,
                                "RENDER_SCOPE_MISMATCH",
                            ));
                        } else {
                            out.push(crate::read_relay::emit_read_body(req_id, body));
                        }
                    } else if command == "im_get_schedule" {
                        // 允许既有裸数据响应；显式业务失败或坏 JSON 不能伪装成空列表成功。
                        let body = match serde_json::from_slice::<serde_json::Value>(&raw_body) {
                            Ok(body)
                                if body.get("status").is_none_or(|status| status == "SUCCESS") =>
                            {
                                body
                            }
                            _ => {
                                out.push(crate::read_relay::emit_read_error(
                                    req_id,
                                    "SCHEDULE_READ_FAILED",
                                ));
                                return Ok(());
                            }
                        };
                        let schedules = crate::outbound::posts::read::schedule_projections(
                            channel_id.unwrap_or_default(),
                            &body,
                        );
                        out.push(crate::read_relay::emit_read_body(
                            req_id.as_str(),
                            serde_json::json!({
                                "ok": true,
                                "schedules": schedules,
                            }),
                        ));
                    } else if command == "im_announcement_list" {
                        self.emit_announcement_list_reply(
                            req_id,
                            channel_id.unwrap_or_default(),
                            &raw_body,
                            out,
                        )?;
                    } else if command == "im_announcement_delete" {
                        self.emit_announcement_delete_reply(
                            req_id,
                            channel_id.unwrap_or_default(),
                            &raw_body,
                            out,
                        )?;
                    } else {
                        out.push(crate::read_relay::emit_read_result(
                            req_id.as_str(),
                            raw_body.as_ref(),
                        ));
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        req_id,
                        error = ?e,
                        "read reply envelope decode failed"
                    );
                    out.push(crate::read_relay::emit_read_error(
                        req_id,
                        "response envelope decode failed",
                    ));
                }
            },
            PortOutcome::Err(e) => {
                tracing::warn!(req_id = req_id.as_str(), error = ?e, "read outbound http failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "http request failed",
                ));
            }
        }
        Ok(())
    }

    /// 把公告 list 回包按频道版本做单调收敛，并同时回灌 typed command result。
    fn emit_announcement_list_reply(
        &mut self,
        req_id: &str,
        channel_id: &str,
        raw_body: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let body = match serde_json::from_slice::<serde_json::Value>(raw_body) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(req_id, error = ?error, "announcement list response JSON invalid");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "announcement list response JSON invalid",
                ));
                return Ok(());
            }
        };
        let Some(snapshot) =
            crate::outbound::posts::read_ext::announcement_list_projection(channel_id, &body)
        else {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "announcement list response missing canonical versioned snapshot",
            ));
            return Ok(());
        };
        let version = snapshot
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let Some(channel) = crate::state::ChannelId::from_str(channel_id) else {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "announcement list response has invalid channel",
            ));
            return Ok(());
        };
        let committed = self
            .state
            .announcement_versions
            .get(&channel)
            .copied()
            .unwrap_or_default();
        let pending = self
            .state
            .announcement_reload_versions
            .get(&channel)
            .copied();
        if version < committed || pending.is_some_and(|expected| version < expected) {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "stale announcement version",
            ));
            return Ok(());
        }
        let should_emit = version > committed || pending == Some(version);
        self.state
            .announcement_versions
            .insert(channel, committed.max(version));
        if pending.is_some_and(|expected| expected <= version) {
            self.state.announcement_reload_versions.remove(&channel);
        }
        if should_emit {
            out.push(crate::event::announcement::list_updated(snapshot.clone())?.into_effect());
        }
        out.push(crate::read_relay::emit_read_body(req_id, snapshot));
        Ok(())
    }

    /// 把公告删除回包投影成 typed result；幂等未命中只回灌，不启动列表回读。
    fn emit_announcement_delete_reply(
        &mut self,
        req_id: &str,
        channel_id: &str,
        raw_body: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let body = match serde_json::from_slice::<serde_json::Value>(raw_body) {
            Ok(body) => body,
            Err(error) => {
                tracing::warn!(req_id, error = ?error, "announcement delete response JSON invalid");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "announcement delete response JSON invalid",
                ));
                return Ok(());
            }
        };
        let Some(result) =
            crate::outbound::posts::read_ext::announcement_delete_projection(channel_id, &body)
        else {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "announcement delete response missing canonical version",
            ));
            return Ok(());
        };
        let Some(channel) = crate::state::ChannelId::from_str(channel_id) else {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "announcement delete response has invalid channel",
            ));
            return Ok(());
        };
        let version = result
            .get("version")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        let committed = self
            .state
            .announcement_versions
            .get(&channel)
            .copied()
            .unwrap_or_default();
        let pending = self
            .state
            .announcement_reload_versions
            .get(&channel)
            .copied();
        let no_op = result
            .get("noOp")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(version == 0);
        if no_op {
            // no-op 不携带新的列表事实：保留在途 list continuation，且只做版本 max，绝不回滚。
            if version > committed {
                self.state.announcement_versions.insert(channel, version);
            }
            out.push(crate::read_relay::emit_read_body(req_id, result));
            return Ok(());
        }
        if version < committed || pending.is_some_and(|expected| expected > version) {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "stale announcement delete version",
            ));
            return Ok(());
        }
        self.state
            .announcement_versions
            .insert(channel, committed.max(version));
        out.push(crate::read_relay::emit_read_body(req_id, result));
        self.enqueue_announcement_reload(channel, version, out)
    }

    /// 为公告变更建立去重的 list HTTP continuation；pending 版本是本频道唯一在途期望。
    fn enqueue_announcement_reload(
        &mut self,
        channel: crate::state::ChannelId,
        expected_version: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let pending = self
            .state
            .announcement_reload_versions
            .get(&channel)
            .copied()
            .unwrap_or_default();
        if pending >= expected_version {
            return Ok(());
        }
        let corr = self.alloc_corr_internal();
        let req_id = format!("announcement-refresh-{}", corr.raw());
        let payload = serde_json::to_vec(&serde_json::json!({
            "channel_id": channel.as_str(),
            "req_id": req_id,
        }))
        .map_err(|error| ImError::Parse(format!("announcement refresh payload: {error}")))?;
        let effects = crate::commands::handle_outbound(
            "im_announcement_list",
            &payload,
            self.config.api_base_url.as_str(),
            self.config.default_api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        )?;
        self.state
            .announcement_reload_versions
            .insert(channel, expected_version);
        self.state.corr_map.insert(
            corr,
            CorrelationContext::OutboundReadReply {
                req_id,
                command: "im_announcement_list".to_string(),
                channel_id: Some(channel.as_str().to_string()),
            },
        );
        for effect in effects {
            out.push(effect);
        }
        Ok(())
    }

    /// 校验 exact HTTP 回报、按 viewer 过滤并建立唯一 durable Persist barrier。
    pub(super) fn handle_exact_posts_http_reply(
        &mut self,
        req_id: &str,
        requested_ids: Vec<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        let raw_body = match outcome {
            PortOutcome::Ok(reply) => {
                match crate::http_envelope::unwrap_success_envelope(reply.0.as_ref(), "posts/get") {
                    Ok(raw) => raw,
                    Err(error) => {
                        tracing::warn!(req_id, error = ?error, "exact posts HTTP envelope invalid");
                        out.push(crate::read_relay::emit_read_error(
                            req_id,
                            "exact posts response envelope invalid",
                        ));
                        return;
                    }
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(req_id, error = ?error, "exact posts HTTP request failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "exact posts http failed",
                ));
                return;
            }
        };
        let posts = match parse_exact_posts_body(&raw_body) {
            Ok(posts) => posts,
            Err(error) => {
                tracing::warn!(req_id, error = ?error, "exact posts response body invalid");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "exact posts response body invalid",
                ));
                return;
            }
        };
        let (accepted_keys, ops) = self.collect_exact_posts_cache_ops(&requested_ids, posts);
        if !ops.is_empty() && self.local_store_mode == LocalStoreMode::Disabled {
            tracing::warn!(req_id, "exact posts requires durable local storage");
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "exact posts durable storage unavailable",
            ));
            return;
        }
        // 即使是 zero-hit，也走一次带 correlation 的空 Persist，确保 terminal 只由同一链路结算。
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist { corr, ops });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ExactPostsPersist {
                req_id: req_id.to_string(),
                accepted_keys,
            },
        );
    }

    /// 将 HTTP 可见行转成 message upsert；只接受请求 id 命中且每个请求键一次。
    pub(crate) fn collect_exact_posts_cache_ops(
        &self,
        requested_ids: &[String],
        posts: Vec<Value>,
    ) -> (Vec<String>, Vec<StorageOp>) {
        let requested = requested_ids.iter().cloned().collect::<HashSet<_>>();
        let mut matched = HashSet::new();
        let mut accepted_keys = Vec::new();
        let mut accepted_storage = HashSet::new();
        let mut ops = Vec::new();
        for post in posts {
            let fields = crate::ws::parser::extract_post_fields(&post);
            let candidates = [fields.id.as_str(), fields.temporary_id.as_str()]
                .into_iter()
                .filter(|id| !id.is_empty() && requested.contains(*id))
                .map(str::to_string)
                .collect::<Vec<_>>();
            if candidates.is_empty() || candidates.iter().all(|id| matched.contains(id)) {
                continue;
            }
            let Some(channel_id) = crate::state::ChannelId::from_str(fields.channel_id.as_str())
            else {
                tracing::warn!("exact posts row has invalid channel id; ignoring row");
                continue;
            };
            let (visible_rows, mut row_ops) =
                match crate::query::local_first::visible_remote_rows_and_cache_ops(
                    channel_id,
                    vec![post],
                    &[],
                    self.config.auth_user_id.as_str(),
                ) {
                    Ok(result) => result,
                    Err(error) => {
                        tracing::warn!(error = ?error, "exact posts row failed closed");
                        continue;
                    }
                };
            let Some(normalized) = visible_rows.into_iter().next() else {
                continue;
            };
            let normalized_fields = crate::ws::parser::extract_post_fields(&normalized);
            let Some(storage_key) = exact_storage_key(&normalized_fields) else {
                continue;
            };
            if !accepted_storage.insert(storage_key.clone()) {
                continue;
            }
            for id in candidates {
                matched.insert(id);
            }
            accepted_keys.push(storage_key);
            protect_exact_read_only_columns(&mut row_ops);
            ops.extend(row_ops);
        }
        (accepted_keys, ops)
    }

    /// PersistOk 后按 temporary_id 逐条 Scan，保证成功结果来自 durable read-back。
    pub(super) fn handle_exact_posts_persist_reply(
        &mut self,
        req_id: String,
        accepted_keys: Vec<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if let PortOutcome::Err(error) = outcome {
            tracing::warn!(%req_id, error = ?error, "exact posts persist failed");
            out.push(crate::read_relay::emit_read_error(
                req_id.as_str(),
                "exact posts persist failed",
            ));
            return;
        }
        if accepted_keys.is_empty() {
            out.push(crate::read_relay::emit_read_body(
                req_id.as_str(),
                serde_json::json!({"reqId": req_id, "posts": []}),
            ));
            return;
        }
        self.schedule_exact_posts_readback(req_id, accepted_keys, 1, Vec::new(), out);
    }

    /// 继续 exact read-back 链；只有最后一条 durable row 回报后才发唯一 terminal。
    pub(super) fn handle_exact_posts_readback_reply(
        &mut self,
        req_id: String,
        accepted_keys: Vec<String>,
        next_index: usize,
        mut rows: Vec<Value>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        let row = match outcome {
            PortOutcome::Ok(reply) => {
                match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                    Ok(rows) => rows.into_iter().next(),
                    Err(error) => {
                        tracing::warn!(%req_id, error = ?error, "exact posts durable readback malformed");
                        None
                    }
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(%req_id, error = ?error, "exact posts durable readback failed");
                None
            }
        };
        let Some(row) = row else {
            out.push(crate::read_relay::emit_read_error(
                req_id.as_str(),
                "exact posts durable readback failed",
            ));
            return;
        };
        rows.push(row);
        if next_index < accepted_keys.len() {
            self.schedule_exact_posts_readback(req_id, accepted_keys, next_index + 1, rows, out);
            return;
        }
        let projected = project_exact_posts(&accepted_keys, rows);
        out.push(crate::read_relay::emit_read_body(
            req_id.as_str(),
            serde_json::json!({"reqId": req_id, "posts": projected}),
        ));
    }

    /// 校验 G11h initial-window HTTP，并把目标首条窗口送入唯一 Persist barrier。
    pub(super) fn handle_initial_window_http_reply(
        &mut self,
        req_id: String,
        post_id: String,
        page_size: u32,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        let raw_body = match outcome {
            PortOutcome::Ok(reply) => match crate::http_envelope::unwrap_success_envelope(
                reply.0.as_ref(),
                "posts/getPostsAfterIndex",
            ) {
                Ok(raw) => raw,
                Err(error) => {
                    tracing::warn!(%req_id, error = ?error, "initial window HTTP envelope invalid");
                    out.push(crate::read_relay::emit_read_error(
                        req_id.as_str(),
                        "initial window response envelope invalid",
                    ));
                    return;
                }
            },
            PortOutcome::Err(error) => {
                tracing::warn!(%req_id, error = ?error, "initial window HTTP request failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id.as_str(),
                    "initial window http failed",
                ));
                return;
            }
        };
        let mut posts =
            match crate::query::local_first::parse_initial_window_posts(&raw_body, &post_id) {
                Ok(posts) => posts,
                Err(error) => {
                    tracing::warn!(%req_id, error = ?error, "initial window response body invalid");
                    out.push(crate::read_relay::emit_read_error(
                        req_id.as_str(),
                        "initial window target must be first",
                    ));
                    return;
                }
            };
        posts.truncate(page_size as usize);
        let (accepted_keys, ops) = match self.collect_initial_window_cache_ops(&post_id, posts) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(%req_id, error = ?error, "initial window rows failed closed");
                out.push(crate::read_relay::emit_read_error(
                    req_id.as_str(),
                    "initial window rows invalid",
                ));
                return;
            }
        };
        if !ops.is_empty() && self.local_store_mode == LocalStoreMode::Disabled {
            tracing::warn!(%req_id, "initial window requires durable local storage");
            out.push(crate::read_relay::emit_read_error(
                req_id.as_str(),
                "initial window durable storage unavailable",
            ));
            return;
        }
        // Zero-hit (missing or invisible target) still crosses one empty Persist barrier.
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist { corr, ops });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::InitialWindowPersist {
                req_id,
                post_id,
                accepted_keys,
            },
        );
    }

    /// 将 initial-window 可见 rows 转成按首条顺序排列的 durable message upsert keys。
    fn collect_initial_window_cache_ops(
        &self,
        post_id: &str,
        posts: Vec<Value>,
    ) -> Result<(Vec<String>, Vec<StorageOp>), ImError> {
        if posts.is_empty() {
            return Ok((Vec::new(), Vec::new()));
        }
        let first_fields = crate::ws::parser::extract_post_fields(&posts[0]);
        let channel_id = crate::state::ChannelId::from_str(first_fields.channel_id.as_str())
            .ok_or_else(|| ImError::Parse("initial window row missing channelId".to_string()))?;
        let (rows, mut ops) = crate::query::local_first::visible_remote_rows_and_cache_ops(
            channel_id,
            posts,
            &[],
            self.config.auth_user_id.as_str(),
        )?;
        // Target invisibility is a typed empty result; never cache the trailing rows alone.
        if rows.is_empty() || !crate::query::local_first::post_matches_identity(&rows[0], post_id) {
            return Ok((Vec::new(), Vec::new()));
        }
        let mut accepted_keys = Vec::with_capacity(rows.len());
        let mut seen = HashSet::new();
        for row in rows {
            let fields = crate::ws::parser::extract_post_fields(&row);
            let Some(key) = exact_storage_key(&fields) else {
                continue;
            };
            if seen.insert(key.clone()) {
                accepted_keys.push(key);
            }
        }
        protect_exact_read_only_columns(&mut ops);
        Ok((accepted_keys, ops))
    }

    /// PersistOk 后逐条回读 initial-window identities，空命中也只结算一次 terminal。
    pub(super) fn handle_initial_window_persist_reply(
        &mut self,
        req_id: String,
        post_id: String,
        accepted_keys: Vec<String>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if let PortOutcome::Err(error) = outcome {
            tracing::warn!(%req_id, error = ?error, "initial window persist failed");
            out.push(crate::read_relay::emit_read_error(
                req_id.as_str(),
                "initial window persist failed",
            ));
            return;
        }
        if accepted_keys.is_empty() {
            out.push(crate::read_relay::emit_read_body(
                req_id.as_str(),
                serde_json::json!({"reqId": req_id, "posts": []}),
            ));
            return;
        }
        self.schedule_initial_window_readback(req_id, post_id, accepted_keys, 1, Vec::new(), out);
    }

    /// 继续 initial-window durable read-back，最终验证 target 首条后释放唯一 Result。
    pub(super) fn handle_initial_window_readback_reply(
        &mut self,
        req_id: String,
        post_id: String,
        accepted_keys: Vec<String>,
        next_index: usize,
        mut rows: Vec<Value>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        let row = match outcome {
            PortOutcome::Ok(reply) => {
                match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                    Ok(rows) => rows.into_iter().next(),
                    Err(error) => {
                        tracing::warn!(%req_id, error = ?error, "initial window durable readback malformed");
                        None
                    }
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(%req_id, error = ?error, "initial window durable readback failed");
                None
            }
        };
        let Some(row) = row else {
            out.push(crate::read_relay::emit_read_error(
                req_id.as_str(),
                "initial window durable readback failed",
            ));
            return;
        };
        rows.push(row);
        if next_index < accepted_keys.len() {
            self.schedule_initial_window_readback(
                req_id,
                post_id,
                accepted_keys,
                next_index + 1,
                rows,
                out,
            );
            return;
        }
        if rows
            .first()
            .is_none_or(|row| !crate::query::local_first::post_matches_identity(row, &post_id))
        {
            out.push(crate::read_relay::emit_read_error(
                req_id.as_str(),
                "initial window target missing from durable readback",
            ));
            return;
        }
        let projected = project_exact_posts(&accepted_keys, rows);
        if projected
            .first()
            .is_none_or(|row| !crate::query::local_first::post_matches_identity(row, &post_id))
        {
            out.push(crate::read_relay::emit_read_error(
                req_id.as_str(),
                "initial window target is not first in durable result",
            ));
            return;
        }
        out.push(crate::read_relay::emit_read_body(
            req_id.as_str(),
            serde_json::json!({"reqId": req_id, "posts": projected}),
        ));
    }

    /// 为 initial-window 的下一个 identity 发出 bounded Scan，并绑定 continuation correlation。
    fn schedule_initial_window_readback(
        &mut self,
        req_id: String,
        post_id: String,
        accepted_keys: Vec<String>,
        next_index: usize,
        rows: Vec<Value>,
        out: &mut EffectSink,
    ) {
        let key = accepted_keys[next_index - 1].clone();
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Scan(exact_posts_scan_spec(&key))],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::InitialWindowReadback {
                req_id,
                post_id,
                accepted_keys,
                next_index,
                rows,
            },
        );
    }

    /// 为 exact read-back 发出单行 Scan，并把续接状态绑定到新 correlation。
    fn schedule_exact_posts_readback(
        &mut self,
        req_id: String,
        accepted_keys: Vec<String>,
        next_index: usize,
        rows: Vec<Value>,
        out: &mut EffectSink,
    ) {
        let key = accepted_keys[next_index - 1].clone();
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Scan(exact_posts_scan_spec(&key))],
        });
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ExactPostsReadback {
                req_id,
                accepted_keys,
                next_index,
                rows,
            },
        );
    }

    /// MV3-G02e 本地读回只按冻结的账号/频道命令投影，并通过非领域结果通道返回。
    pub(super) fn handle_draft_readback_reply(
        &self,
        command: Box<crate::draft::QueryDraftCommand>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let req_id = command.req_id.as_deref().unwrap_or_default();
        let PortOutcome::Ok(reply) = outcome else {
            out.push(crate::read_relay::emit_read_error(
                req_id,
                "draft read failed",
            ));
            return Ok(());
        };
        let rows = match helix_core::port_codec::rows_from_reply_bytes(&reply.0) {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(error = ?error, "draft readback decode failed");
                out.push(crate::read_relay::emit_read_error(
                    req_id,
                    "draft readback decode failed",
                ));
                return Ok(());
            }
        };
        out.push(command.result_event(&rows)?.into_effect());
        Ok(())
    }

    pub(super) fn handle_todo_query_reply(&self, outcome: &PortOutcome, out: &mut EffectSink) {
        match outcome {
            PortOutcome::Ok(reply) => match unwrap_sync_envelope(reply.0.as_ref()) {
                Ok(raw_body) => {
                    out.push(crate::todo::emit_todo_updated(raw_body.as_ref()));
                }
                Err(e) => {
                    tracing::warn!(error = ?e, "todo query reply envelope decode failed");
                }
            },
            PortOutcome::Err(e) => {
                tracing::warn!(error = ?e, "todo query outbound http failed");
            }
        }
    }

    pub(super) fn handle_load_older_context_reply(
        &mut self,
        mut state: Box<LoadOlderState>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        match outcome {
            PortOutcome::Ok(reply) => {
                // 剥信封 → 取 wire Post 数组；信封/base64 畸形作为 invalid round fail-closed。
                let rows = unwrap_sync_envelope(reply.0.as_ref())
                    .map(|raw| crate::older_context::extract_post_rows(&raw))
                    .unwrap_or_default();
                match state.ingest_round(&rows) {
                    crate::older_context::RoundDecision::Continue => {
                        let next_corr = self.alloc_corr_internal();
                        let (_, body) = crate::older_context::build_post_context_body(&state);
                        out.push(crate::older_context::post_context_http_tracked(
                            &self.config.api_base_url,
                            &body,
                            next_corr,
                            self.state.connection_id.as_deref(),
                            state.request_id(),
                        ));
                        self.state
                            .corr_map
                            .insert(next_corr, CorrelationContext::LoadOlderContext { state });
                    }
                    crate::older_context::RoundDecision::Done => {
                        self.finish_load_older_http(state, now_ms, out)?;
                    }
                }
            }
            PortOutcome::Err(e) => {
                tracing::warn!(error = ?e, "load_older_context postContext http failed");
                self.emit_load_older_failed_event(&state, now_ms, out)?;
            }
        }
        Ok(())
    }

    /// Converts a completed HTTP page into message writes and establishes the persistence barrier.
    fn finish_load_older_http(
        &mut self,
        state: Box<LoadOlderState>,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if state.failed() {
            return self.emit_load_older_failed_event(&state, now_ms, out);
        }
        let (_, ops) = match crate::query::local_first::visible_remote_rows_and_cache_ops(
            *state.channel_id(),
            state.older_rows(),
            &[],
            self.config.auth_user_id.as_str(),
        ) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "load_older_context response could not become durable messages"
                );
                return self.emit_load_older_failed_event(&state, now_ms, out);
            }
        };
        if ops.is_empty() {
            return self.schedule_load_older_readback(state, out);
        }
        if self.local_store_mode == LocalStoreMode::Disabled {
            tracing::warn!(
                channel_id = state.channel_id().as_str(),
                "load_older_context requires a writable store for durable timeline publication"
            );
            return self.emit_load_older_failed_event(&state, now_ms, out);
        }
        let corr = self.alloc_corr_internal();
        out.push(Effect::Persist { corr, ops });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::LoadOlderCache { state });
        Ok(())
    }

    /// 调度唯一可生成成功事件的有界存储读回。
    pub(super) fn schedule_load_older_readback(
        &mut self,
        state: Box<LoadOlderState>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let window_token = state.window_token().ok_or_else(|| {
            ImError::Parse("load older missing attached window token".to_string())
        })?;
        let request = crate::query::MessageQueryRequest {
            channel_id: *state.channel_id(),
            limit: state.readback_limit(),
            window_token: window_token.to_string(),
        };
        let corr = self.alloc_corr_internal();
        out.push(crate::query::build_message_query_from_request(
            &request, corr,
        ));
        self.state
            .corr_map
            .insert(corr, CorrelationContext::LoadOlderReadback { state });
        Ok(())
    }

    /// 从 durable readback 构造 G10a 终态事件并保留 Helix 分页事实。
    pub(super) fn handle_load_older_readback_reply(
        &mut self,
        state: Box<LoadOlderState>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut rows_desc = match outcome {
            PortOutcome::Ok(reply) => {
                match crate::query::local_first::parse_local_rows(reply.0.as_ref()) {
                    Ok(rows) => rows,
                    Err(error) => {
                        tracing::warn!(
                            channel_id = state.channel_id().as_str(),
                            error = ?error,
                            "load_older_context readback was malformed"
                        );
                        return self.emit_load_older_failed_event(&state, now_ms, out);
                    }
                }
            }
            PortOutcome::Err(error) => {
                tracing::warn!(
                    channel_id = state.channel_id().as_str(),
                    error = ?error,
                    "load_older_context readback failed"
                );
                return self.emit_load_older_failed_event(&state, now_ms, out);
            }
        };
        crate::query::local_first::sort_recent_rows_desc(&mut rows_desc);
        let window_token = state.window_token().ok_or_else(|| {
            ImError::Parse("load older missing attached window token".to_string())
        })?;
        let request = crate::query::MessageQueryRequest {
            channel_id: *state.channel_id(),
            limit: state.readback_limit(),
            window_token: window_token.to_string(),
        };
        let page = crate::timeline_state::WindowPage {
            window_token: window_token.to_string(),
            has_older: state.has_more(),
            has_newer: false,
            has_more: state.has_more(),
        };
        out.push(self.emit_timeline_snapshot_with_causation(
            &request,
            &rows_desc,
            now_ms,
            None,
            Some(page),
        )?);
        Ok(())
    }

    /// 发布上拉失败状态，但不擦除上一次成功窗口。
    pub(super) fn emit_load_older_failed_event(
        &mut self,
        state: &LoadOlderState,
        _now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        out.push(
            crate::event::timeline::page(
                state.channel_id().as_str(),
                state.window_token().unwrap_or("latest"),
                "older",
                "failed",
                Vec::new(),
                true,
                false,
                Some(state.anchor_post_id()),
            )?
            .into_effect(),
        );
        Ok(())
    }
}

/// 构造 older durable read-back 的 bounded message Scan。
fn older_navigation_scan_spec(
    state: &crate::timeline_navigation::TimelineNavigationState,
) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(state.page_size().saturating_add(1)),
        filter: Some((
            "channel_id",
            SqlValue::Text(state.channel_id().as_str().to_string()),
        )),
        order_by: OLDER_NAVIGATION_ORDER,
    }
}

/// 提取 authority 页的实际 durable 主键列和值，并保持页内顺序去重。
fn timeline_navigation_readback_keys(
    state: &crate::timeline_navigation::TimelineNavigationState,
) -> Vec<crate::timeline_navigation::TimelineNavigationReadbackKey> {
    let mut seen = HashSet::new();
    state
        .rows()
        .iter()
        .filter_map(timeline_navigation_readback_key)
        .filter(|key| seen.insert((key.column, key.value.clone())))
        .collect()
}

/// 从 authority/durable row 解析与 event_to_upsert_op 一致的实际主键列和值。
fn timeline_navigation_readback_key(
    row: &Value,
) -> Option<crate::timeline_navigation::TimelineNavigationReadbackKey> {
    let fields = crate::ws::parser::extract_post_fields(row);
    if !fields.temporary_id.is_empty() {
        Some(crate::timeline_navigation::TimelineNavigationReadbackKey {
            column: "temporary_id",
            value: fields.temporary_id,
        })
    } else if !fields.id.is_empty() {
        Some(crate::timeline_navigation::TimelineNavigationReadbackKey {
            column: "id",
            value: fields.id,
        })
    } else {
        None
    }
}

/// 构造 newer durable read-back 的 bounded message Scan，尾部按复合键升序读取一条 lookahead。
fn newer_navigation_scan_spec(
    state: &crate::timeline_navigation::TimelineNavigationState,
) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(state.page_size().saturating_add(1)),
        filter: Some((
            "channel_id",
            SqlValue::Text(state.channel_id().as_str().to_string()),
        )),
        order_by: NEWER_NAVIGATION_ORDER,
    }
}

/// 验证本地 locate coverage 仍是完整的 durable viewer window，失败即回到 Go authority。
fn local_timeline_navigation_rows(
    state: &crate::timeline_navigation::TimelineNavigationState,
    coverage: &crate::timeline_navigation::TimelineNavigationCoverage,
    viewer_user_id: &str,
) -> Option<Vec<Value>> {
    match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Locate {
            target_message_id, ..
        } => {
            let rows = crate::query::render_ready::locate::normalize_durable_located_window(
                coverage.rows.clone(),
                state.channel_id().as_str(),
                target_message_id,
                viewer_user_id,
                state.page_size(),
            )
            .ok()?;
            let target_index = rows.iter().position(|row| {
                crate::timeline_navigation::row_matches_message_identity(row, target_message_id)
            })?;
            (coverage.target_index == Some(target_index)).then_some(rows)
        }
        crate::timeline_navigation::TimelineNavigationKind::Older { .. }
        | crate::timeline_navigation::TimelineNavigationKind::Newer { .. } => {
            Some(coverage.rows.clone())
        }
    }
}

/// 返回日志使用的导航方向，不把完整 payload 写入日志。
fn timeline_navigation_direction(
    state: &crate::timeline_navigation::TimelineNavigationState,
) -> &'static str {
    match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Older { .. } => "older",
        crate::timeline_navigation::TimelineNavigationKind::Newer { .. } => "newer",
        crate::timeline_navigation::TimelineNavigationKind::Locate { .. } => "locate",
    }
}

/// 过滤、去重、排序 durable older rows，避免 HTTP rows 成为成功结果旁路。
fn older_navigation_rows(
    state: &crate::timeline_navigation::TimelineNavigationState,
    rows: Vec<Value>,
    auth_user_id: &str,
) -> Vec<Value> {
    // Go 的空页/hasMore=false 是该次 older authority 的边界；不能用旧 cache
    // 中恰好位于 anchor 前的行重新推导一页，避免 stale storage 旁路污染结果。
    if state.rows().is_empty() {
        return Vec::new();
    }
    let cursor = match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Older { cursor, .. }
            if !state.is_windowless() =>
        {
            Some(cursor)
        }
        crate::timeline_navigation::TimelineNavigationKind::Older { .. } => None,
        _ => return Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut filtered = rows
        .into_iter()
        .filter_map(|row| {
            let fields = crate::ws::parser::extract_post_fields(&row);
            if fields.channel_id != state.channel_id().as_str()
                || !crate::channel_write::post_updates_from_fields(
                    state.channel_id(),
                    &fields,
                    auth_user_id,
                )
                .visible
            {
                return None;
            }
            let key = durable_navigation_key(&row)?;
            if let Some(cursor) = cursor {
                if key.0 > cursor.create_at
                    || (key.0 == cursor.create_at && key.1.as_str() >= cursor.temporary_id.as_str())
                {
                    return None;
                }
            }
            let identity = if !fields.id.is_empty() {
                format!("id:{}", fields.id)
            } else if !fields.temporary_id.is_empty() {
                format!("tmp:{}", fields.temporary_id)
            } else {
                return None;
            };
            seen.insert(identity).then_some((key, row))
        })
        .collect::<Vec<_>>();
    filtered.sort_by(|left, right| left.0.cmp(&right.0));
    filtered
        .into_iter()
        .take(state.page_size() as usize)
        .map(|(_, row)| row)
        .collect()
}

/// 判断 HTTP newer 页是否包含锚点之后的 authority 行；anchor-only 页仍是合法 empty terminal。
fn newer_http_has_rows(state: &crate::timeline_navigation::TimelineNavigationState) -> bool {
    let Some(cursor) = (match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Newer { cursor, .. } => Some(cursor),
        _ => None,
    }) else {
        return false;
    };
    state
        .authority_rows()
        .iter()
        .filter_map(durable_navigation_key)
        .any(|key| {
            key.0 > cursor.create_at
                || (key.0 == cursor.create_at && key.1.as_str() > cursor.temporary_id.as_str())
        })
}

/// 过滤、去重、排序 durable newer rows，只保留锚点之后的页内可见消息。
fn newer_navigation_rows(
    state: &crate::timeline_navigation::TimelineNavigationState,
    rows: Vec<Value>,
    auth_user_id: &str,
) -> Vec<Value> {
    // HTTP empty/anchor-only 页是 Go 的明确终态，不能被旧 cache 中恰好存在的
    // 新侧行重新推导成一页，避免 stale storage 旁路污染 empty result。
    if !newer_http_has_rows(state) {
        return Vec::new();
    }
    let cursor = match state.kind() {
        crate::timeline_navigation::TimelineNavigationKind::Newer { cursor, .. }
            if !state.is_windowless() =>
        {
            Some(cursor)
        }
        crate::timeline_navigation::TimelineNavigationKind::Newer { .. } => None,
        _ => return Vec::new(),
    };
    let mut seen = HashSet::new();
    let mut filtered = rows
        .into_iter()
        .filter_map(|row| {
            let fields = crate::ws::parser::extract_post_fields(&row);
            if fields.channel_id != state.channel_id().as_str()
                || !crate::channel_write::post_updates_from_fields(
                    state.channel_id(),
                    &fields,
                    auth_user_id,
                )
                .visible
            {
                return None;
            }
            let key = durable_navigation_key(&row)?;
            if let Some(cursor) = cursor {
                if key.0 < cursor.create_at
                    || (key.0 == cursor.create_at && key.1.as_str() <= cursor.temporary_id.as_str())
                {
                    return None;
                }
            }
            let identity = if !fields.id.is_empty() {
                format!("id:{}", fields.id)
            } else if !fields.temporary_id.is_empty() {
                format!("tmp:{}", fields.temporary_id)
            } else {
                return None;
            };
            seen.insert(identity).then_some((key, row))
        })
        .collect::<Vec<_>>();
    filtered.sort_by(|left, right| left.0.cmp(&right.0));
    filtered
        .into_iter()
        .take(state.page_size() as usize)
        .map(|(_, row)| row)
        .collect()
}

/// 从 snake/camel durable row 提取稳定分页键。
fn durable_navigation_key(row: &Value) -> Option<(i64, String)> {
    let create_at = row
        .get("create_at")
        .or_else(|| row.get("createAt"))
        .and_then(Value::as_i64)?;
    let temporary_id = row
        .get("temporary_id")
        .or_else(|| row.get("temporaryId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())?;
    Some((create_at, temporary_id.to_string()))
}

/// 按本次 Scan 的实际列和值匹配 durable row，避免 id 回退行被 temporary_id 覆盖。
fn durable_row_matches_readback_key(
    row: &Value,
    key: &crate::timeline_navigation::TimelineNavigationReadbackKey,
) -> bool {
    let fields = crate::ws::parser::extract_post_fields(row);
    match key.column {
        "temporary_id" => fields.temporary_id == key.value,
        "id" => fields.id == key.value,
        _ => false,
    }
}

/// 解析 Go `posts/get` 的 SUCCESS 信封业务体；只接受数组，空数组表示合法零命中。
pub(super) fn parse_exact_posts_body(raw: &[u8]) -> Result<Vec<Value>, ImError> {
    let root: Value = serde_json::from_slice(raw)
        .map_err(|error| ImError::Parse(format!("posts/get body: {error}")))?;
    let status = root
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| ImError::Parse("posts/get body missing string status".to_string()))?;
    if !status.eq_ignore_ascii_case("SUCCESS") {
        return Err(ImError::Parse(format!("posts/get backend status {status}")));
    }
    let rows = root
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ImError::Parse("posts/get response missing data array".to_string()))?;
    if rows.iter().any(|row| !row.is_object()) {
        return Err(ImError::Parse(
            "posts/get data rows must be objects".to_string(),
        ));
    }
    Ok(rows.clone())
}

/// 取 durable message 的实际冲突主键；与 event_to_upsert_op 的三级回退保持一致。
fn exact_storage_key(fields: &crate::sync_session::PostFields) -> Option<String> {
    if !fields.temporary_id.is_empty() {
        Some(fields.temporary_id.clone())
    } else if !fields.id.is_empty() {
        Some(fields.id.clone())
    } else {
        None
    }
}

/// exact 查询只缓存内容，不得借回填改写已有消息的 create_at/read_bits/event_seq 本地事实。
fn protect_exact_read_only_columns(ops: &mut Vec<StorageOp>) {
    for op in ops {
        if let StorageOp::BatchUpsert(spec) = op {
            for column in ["create_at", "read_bits", "event_seq"] {
                if !spec.exclude_from_update.contains(&column) {
                    spec.exclude_from_update.push(column);
                }
            }
        }
    }
}

/// 为 Timeline 导航的单个 exact identity 构造有界 message Scan，禁止频道全表扫描。
fn timeline_navigation_exact_scan_spec(
    key: &crate::timeline_navigation::TimelineNavigationReadbackKey,
) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(1),
        filter: Some((key.column, SqlValue::Text(key.value.clone()))),
        order_by: &[],
    }
}

/// 为旧 exact-posts API 按 temporary_id 构造单行 message Scan，保持其既有合同。
fn exact_posts_scan_spec(key: &str) -> ScanSpec {
    ScanSpec {
        table: "message",
        limit: Some(1),
        filter: Some(("temporary_id", SqlValue::Text(key.to_string()))),
        order_by: &[],
    }
}

/// 把落库字符串字段恢复为对象；畸形扩展字段收敛为空对象，不暴露原始编码。
fn exact_json_object(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}))
}

/// 被引用消息是可空对象；无引用不能借用 props 等扩展字段的空对象默认值。
fn exact_replied_message(raw: &str) -> Value {
    serde_json::from_str::<Value>(raw)
        .ok()
        .filter(Value::is_object)
        .unwrap_or(Value::Null)
}

/// 生成 G11g 严格消息投影；排除 readBits、eventSeq、cursor 与 expedite 原始位图。
fn project_exact_posts(accepted_keys: &[String], rows: Vec<Value>) -> Vec<Value> {
    let accepted = accepted_keys.iter().collect::<HashSet<_>>();
    let mut seen = HashSet::new();
    let mut projected = Vec::new();
    for row in rows {
        // Reuse the DB-row decoder before the wire parser; SQLite JSON columns
        // are text, and parsing them as wire values double-encodes card props.
        let row = crate::query::render_ready::core::shape_row(&row, "");
        let fields = crate::ws::parser::extract_post_fields(&row);
        let Some(key) = exact_storage_key(&fields) else {
            continue;
        };
        if !accepted.contains(&key) || !seen.insert(key) {
            continue;
        }
        let mut projected_post = serde_json::json!({
            "id": if fields.id.is_empty() { fields.temporary_id.clone() } else { fields.id.clone() },
            "temporaryId": fields.temporary_id,
            "channelId": fields.channel_id,
            // 后续读侧可能再次经过 prepare_post，保留作用域/撤回事实避免摘要漂移。
            "teamId": row.get("teamId").cloned().unwrap_or_else(|| Value::String(fields.team_id.clone())),
            "userId": fields.user_id,
            "userSnapshot": exact_json_object(&fields.user_snapshot),
            "type": fields.msg_type,
            "message": fields.message,
            "simpleMessage": fields.simple_message,
            "createAt": fields.create_at,
            "updateAt": fields.update_at,
            "sendStatus": "sent",
            "viewers": fields.viewers,
            "mentions": fields.mentions,
            "props": exact_json_object(&fields.props),
            "topic": exact_json_object(&fields.topic),
            "replyId": fields.reply_id,
            "replyRootId": fields.reply_root_id,
            "replyFirstLevelId": fields.reply_first_level_id,
            "replyCount": fields.reply_count,
            "repliedMessage": exact_replied_message(&fields.replied_message),
            "replyMessages": exact_json_object(&fields.reply_messages),
            "revoke": row.get("revoke").cloned().unwrap_or(Value::Bool(false)),
        });
        crate::message_summary::attach_parts(&mut projected_post);
        projected.push(projected_post);
    }
    projected
}

#[cfg(test)]
mod member_notice_tests {
    use super::project_exact_posts;
    use serde_json::json;

    #[test]
    fn exact_posts_rebuild_keeps_parts_and_drops_them_for_revoke() {
        let row = json!({
            "id": "post-304",
            "temporary_id": "tmp-304",
            "channel_id": "channel-304",
            "team_id": "company-a",
            "user_id": "operator",
            "type": "NOTICE",
            "message": "member notice",
            "simple_message": "旧摘要",
            "props": {
                "type": "join",
                "operator": {"id": "operator"},
                "users": [{"id": "target", "name": "旧目标名"}]
            },
            "revoke": 0
        });
        let projected = project_exact_posts(&["tmp-304".to_string()], vec![row]);
        assert_eq!(projected.len(), 1);
        assert_eq!(
            projected[0]["summaryParts"],
            json!([
                {"kind":"user","userId":"operator","companyId":"company-a"},
                {"kind":"text","text":"邀请"},
                {"kind":"user","userId":"target","companyId":"company-a"},
                {"kind":"text","text":"加入群聊"}
            ])
        );
        assert!(!projected[0]["summaryParts"].to_string().contains("旧"));
        assert_eq!(projected[0]["teamId"], "company-a");
        assert_eq!(projected[0]["revoke"], false);
        let reread = crate::message_summary::prepare_post(&projected[0]);
        assert_eq!(reread["summaryParts"][0]["companyId"], "company-a");

        let mut revoked = projected[0].clone();
        revoked["revoke"] = json!(true);
        let revoked = project_exact_posts(&["tmp-304".to_string()], vec![revoked]);
        assert_eq!(revoked.len(), 1);
        assert!(revoked[0].get("summaryParts").is_none());
        assert_eq!(revoked[0]["revoke"], true);
        assert!(crate::message_summary::prepare_post(&revoked[0])
            .get("summaryParts")
            .is_none());
    }
}
