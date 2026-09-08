//! 恢复流控：HTTP 每页最多64份频道投影，PersistOk 是下一页的唯一放行信号。
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext, Seq};
use helix_core::effect::HttpRequest;
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, Effect, EffectSink};
use serde_json::json;
use std::collections::{HashMap, HashSet, VecDeque};

const PAGE_SIZE: usize = 64;

/// 单连接单恢复流只保留一次 manifest 和 cursor 索引，以及一个在途 HTTP/持久化关联。
#[derive(Debug)]
pub(crate) struct IncrementPull {
    connection_id: String,
    timestamp: i64,
    cursors: HashMap<ChannelId, Seq>,
    remaining: VecDeque<ChannelId>,
    requested: Option<Vec<ChannelId>>,
    inflight: Correlation,
    pending: Vec<crate::sync_session::IncrementChannel>,
}

impl ImModule {
    /// 将启动扫描补偿合并到现有恢复流；不并行建立第二个全量请求。
    pub(crate) fn start_increment_pull(
        &mut self,
        timestamp: i64,
        cursors: Vec<(ChannelId, Seq)>,
        out: &mut EffectSink,
    ) {
        if self.state.increment_pull.is_some() {
            return;
        }
        let Some(connection_id) = self.state.connection_id.clone() else {
            return;
        };
        let corr = self.alloc_corr_internal();
        self.state.increment_pull = Some(IncrementPull {
            connection_id,
            timestamp,
            cursors: cursors.into_iter().collect(),
            remaining: VecDeque::new(),
            requested: None,
            inflight: corr,
            pending: Vec::new(),
        });
        self.send_increment_pull_page(corr, out);
    }

    /// 只携带当前页涉及的 cursor，避免每页重复发送全量 cursor 形成二次方负载。
    fn send_increment_pull_page(&mut self, corr: Correlation, out: &mut EffectSink) {
        let Some(pull) = self.state.increment_pull.as_ref() else {
            return;
        };
        let mut cursors: Vec<_> = match &pull.requested {
            Some(ids) => ids
                .iter()
                .filter_map(|id| pull.cursors.get(id).map(|seq| (*id, *seq)))
                .collect(),
            None => pull.cursors.iter().map(|(id, seq)| (*id, *seq)).collect(),
        };
        cursors.sort_unstable_by_key(|(id, _)| *id);
        let mut body = json!({"timeStamp":pull.timestamp,"cursors":cursors.iter().map(|(id,seq)|json!({"channelId":id.as_str(),"fromSeq":seq.0})).collect::<Vec<_>>()});
        if let Some(ids) = &pull.requested {
            body["channelIds"] = json!(ids.iter().map(ChannelId::as_str).collect::<Vec<_>>());
        }
        self.diagnose(crate::diagnostics::Observation {
            event: "increment_page_requested",
            stage: "http",
            corr: Some(corr.raw()),
            count: pull.requested.as_ref().map_or(0, Vec::len),
            has_more: !pull.remaining.is_empty(),
            ..Default::default()
        });
        let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
        headers.extend(crate::acl::sync_http_effects::session_auth_headers(Some(
            &pull.connection_id,
        )));
        out.push(Effect::Http {
            corr,
            req: HttpRequest {
                method: "POST".into(),
                url: format!("{}/channels/load/increment/page", self.config.api_base_url),
                headers,
                body: Some(body.to_string().into()),
            },
        });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::IncrementPullHttp);
    }

    /// 先验证整页及清单，再复用 increment reducer；非法/旧连接响应不能留下部分副作用。
    pub(crate) fn handle_increment_pull_http(
        &mut self,
        corr: Correlation,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) {
        if !self.increment_pull_matches(corr) {
            return;
        }
        let parsed = (|| {
            let PortOutcome::Ok(reply) = outcome else {
                return None;
            };
            let raw = crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref()).ok()?;
            let value: serde_json::Value = serde_json::from_slice(&raw).ok()?;
            if value.get("status")?.as_str()? != "SUCCESS" {
                return None;
            }
            let data = value.get("data")?;
            let channels = data.get("channels")?.as_array()?;
            if channels.len() > PAGE_SIZE {
                return None;
            }
            let pull = self.state.increment_pull.as_ref()?;
            let manifest = if pull.requested.is_none() {
                let mut seen = HashSet::new();
                let ids = data
                    .get("manifest")?
                    .as_array()?
                    .iter()
                    .map(|value| {
                        let id = ChannelId::from_str(value.as_str()?)?;
                        seen.insert(id).then_some(id)
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(ids)
            } else {
                None
            };
            let expected: HashSet<_> = manifest
                .as_ref()
                .map(|ids| ids.iter().take(PAGE_SIZE).copied().collect())
                .unwrap_or_else(|| {
                    pull.requested
                        .as_ref()
                        .into_iter()
                        .flatten()
                        .copied()
                        .collect()
                });
            let mut seen = HashSet::new();
            let increments = channels
                .iter()
                .map(|value| {
                    let inc = crate::ws::parser::parse_increment_channel(value)?;
                    (expected.contains(&inc.channel_id) && seen.insert(inc.channel_id))
                        .then_some(inc)
                })
                .collect::<Option<Vec<_>>>()?;
            // 首批和 manifest 同一次查询产生，不允许漏项；后续已退出的频道可以被鉴权过滤。
            if manifest.is_some() && increments.len() != expected.len() {
                return None;
            }
            Some((manifest, increments))
        })();
        let Some((manifest, increments)) = parsed else {
            self.fail_increment_pull("invalid_or_failed_http");
            return;
        };
        if let Some(manifest) = manifest {
            if let Some(pull) = self.state.increment_pull.as_mut() {
                pull.remaining = manifest.into_iter().skip(PAGE_SIZE).collect();
            }
        }
        let mut ops = Vec::new();
        let base = self.config.api_base_url.clone();
        let actor = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx = crate::ws::ImWsContext::new(state, now_ms, &base, &actor, alloc);
            for inc in &increments {
                for effect in
                    crate::ws::handlers::increment_channel::compile_increment_effects(&mut ctx, inc)
                {
                    if let Effect::PersistFire { ops: writes } = effect {
                        ops.extend(writes);
                    }
                }
            }
        });
        if let Some(pull) = self.state.increment_pull.as_mut() {
            pull.pending = increments;
        }
        if ops.is_empty() {
            self.advance_increment_pull(out);
            return;
        }
        let persist = self.alloc_corr_internal();
        if let Some(pull) = self.state.increment_pull.as_mut() {
            pull.inflight = persist;
        }
        self.state
            .corr_map
            .insert(persist, CorrelationContext::IncrementPullPersist);
        out.push(Effect::PersistAtomic { corr: persist, ops });
    }

    /// 成功落库才释放下一页；失败和迟到回执均不能发 loaded 或再取数据。
    pub(crate) fn handle_increment_pull_persist(
        &mut self,
        corr: Correlation,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if !self.increment_pull_matches(corr) {
            return;
        }
        self.diagnose(crate::diagnostics::Observation {
            event: "increment_page_persisted",
            stage: "persist",
            corr: Some(corr.raw()),
            count: self
                .state
                .increment_pull
                .as_ref()
                .map_or(0, |pull| pull.pending.len()),
            result: if matches!(outcome, PortOutcome::Ok(_)) {
                "success"
            } else {
                "failed"
            },
            ..Default::default()
        });
        if matches!(outcome, PortOutcome::Ok(_)) {
            self.advance_increment_pull(out);
        } else {
            self.fail_increment_pull("persist_failed");
        }
    }

    /// 单一关联和活跃连接共同隔离重复、旧连接和断线后的异步回包。
    fn increment_pull_matches(&self, corr: Correlation) -> bool {
        self.state.conn == crate::state::ConnState::Connected
            && self.state.increment_pull.as_ref().is_some_and(|pull| {
                pull.inflight == corr
                    && self.state.connection_id.as_deref() == Some(pull.connection_id.as_str())
            })
    }

    /// 消费冻结清单的一页；清单耗尽后复用原 global-end 的 sync 与最终持久化门槛。
    fn advance_increment_pull(&mut self, out: &mut EffectSink) {
        let pending = match self.state.increment_pull.as_mut() {
            Some(pull) => std::mem::take(&mut pull.pending),
            None => return,
        };
        let base = self.config.api_base_url.clone();
        let actor = self.config.auth_user_id.clone();
        self.with_state_and_corr_allocator(|state, alloc| {
            let mut ctx = crate::ws::ImWsContext::new(state, 0, &base, &actor, alloc);
            for inc in &pending {
                crate::ws::handlers::increment_channel::commit_increment_state(&mut ctx, inc);
            }
        });
        for inc in &pending {
            self.diagnose(crate::diagnostics::Observation {
                event: "channel_inventory_item_committed",
                stage: "persist",
                result: "success",
                channel: inc.channel_id.as_str(),
                count: 1,
                ..Default::default()
            });
            self.diagnose_checkpoint(inc.channel_id, "inventory_committed");
        }
        let Some(pull) = self.state.increment_pull.as_mut() else {
            return;
        };
        if pull.remaining.is_empty() {
            self.state.increment_pull = None;
            if self.state.increment_fetched.is_empty() {
                // 空清单也经过宿主持久化屏障，才能发布本地查询 ready，避免永久 loading。
                let corr = self.alloc_corr_internal();
                self.state.channel_sync_batch_pending = false;
                self.state.channel_sync_persist_inflight += 1;
                self.state.corr_map.insert(
                    corr,
                    CorrelationContext::IncrementBatchPersist {
                        projections: Vec::new(),
                        batch_id: None,
                    },
                );
                out.push(Effect::PersistAtomic {
                    corr,
                    ops: Vec::new(),
                });
            } else {
                self.ingest_increment_end(None, out);
            }
            return;
        }
        pull.requested = Some(
            (0..PAGE_SIZE)
                .filter_map(|_| pull.remaining.pop_front())
                .collect(),
        );
        let corr = self.alloc_corr_internal();
        if let Some(pull) = self.state.increment_pull.as_mut() {
            pull.inflight = corr;
        }
        self.send_increment_pull_page(corr, out);
    }

    /// 恢复失败关闭本轮完成门槛，下一次连接恢复可从已持久化页重新校验。
    fn fail_increment_pull(&mut self, reason: &'static str) {
        self.state.increment_pull = None;
        self.state.channel_sync_batch_pending = false;
        self.state.recovery_session.phase = crate::sync_session::RecoveryPhase::Failed;
        tracing::warn!(reason, "increment pull stopped before completion");
    }
}
