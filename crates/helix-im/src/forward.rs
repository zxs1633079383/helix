//! G-05 ID-only forwarding. Go owns immutable snapshots and stable per-target identities.

use crate::{
    state::{ChannelId, CorrelationContext},
    ImError, ImModule,
};
use helix_core::effect::{ScanSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink, TimerId};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_PENDING: usize = 64;

#[derive(Debug)]
struct Delivery {
    command_id: String,
    targets: BTreeSet<ChannelId>,
    all_targets: Vec<String>,
    timer: TimerId,
    post_count: usize,
    delivered: BTreeMap<ChannelId, BTreeSet<String>>,
}

/// One active attempt per stable command; retries never overwrite an in-flight waiter.
#[derive(Debug, Default)]
pub(crate) struct PendingForwardDeliveryLedger {
    by_request: BTreeMap<String, Delivery>,
}

impl PendingForwardDeliveryLedger {
    fn register(
        &mut self,
        req_id: &str,
        command_id: &str,
        targets: &[String],
        timer: TimerId,
        post_count: usize,
    ) -> Result<(), ImError> {
        if self.by_request.contains_key(req_id)
            || self.by_request.values().any(|d| d.command_id == command_id)
        {
            return Err(ImError::Parse("FORWARD_IN_PROGRESS".into()));
        }
        if self.by_request.len() >= MAX_PENDING {
            return Err(ImError::Parse("FORWARD_BUSY".into()));
        }
        self.by_request.insert(
            req_id.into(),
            Delivery {
                command_id: command_id.into(),
                timer,
                post_count,
                delivered: BTreeMap::new(),
                all_targets: targets.to_vec(),
                targets: targets
                    .iter()
                    .filter_map(|id| ChannelId::from_str(id))
                    .collect(),
            },
        );
        Ok(())
    }

    /// WS echo correlation is a stable command ID, separate from local attempt IDs.
    pub(crate) fn request_for_command(&self, command: &str, channel: ChannelId) -> Option<String> {
        self.by_request.iter().find_map(|(req, d)| {
            (d.command_id == command && d.targets.contains(&channel)).then(|| req.clone())
        })
    }

    /// Persist continuations retain their original attempt ID.
    pub(crate) fn matching_request(&self, req: &str, channel: ChannelId) -> Option<String> {
        self.by_request
            .get(req)
            .is_some_and(|d| d.targets.contains(&channel))
            .then(|| req.to_owned())
    }

    pub(crate) fn timer(&self, req: &str) -> Option<TimerId> {
        self.by_request.get(req).map(|d| d.timer)
    }

    pub(crate) fn is_final_post(&self, req: &str, channel: ChannelId, post_id: &str) -> bool {
        self.by_request.get(req).is_some_and(|d| {
            let seen = d.delivered.get(&channel);
            !seen.is_some_and(|ids| ids.contains(post_id))
                && seen.map_or(0, BTreeSet::len) + 1 == d.post_count
        })
    }

    fn complete_post(&mut self, req: &str, channel: ChannelId, post_id: &str) -> bool {
        let Some(delivery) = self.by_request.get_mut(req) else {
            return false;
        };
        let seen = delivery.delivered.entry(channel).or_default();
        if !seen.insert(post_id.to_owned()) || seen.len() < delivery.post_count {
            return false;
        }
        self.complete_target(req, channel)
    }

    fn complete_target(&mut self, req: &str, channel: ChannelId) -> bool {
        let Some(delivery) = self.by_request.get_mut(req) else {
            return false;
        };
        delivery.targets.remove(&channel);
        if delivery.targets.is_empty() {
            self.by_request.remove(req);
            return true;
        }
        false
    }

    fn targets(&self, req: &str) -> Vec<String> {
        self.by_request
            .get(req)
            .map(|d| d.targets.iter().map(|c| c.as_str().to_owned()).collect())
            .unwrap_or_default()
    }
}

/// Validate the only accepted forward intent; clients cannot submit source bodies or references.
pub(crate) fn request(
    value: &Value,
) -> Result<(String, String, Vec<String>, Vec<String>, &'static str), ImError> {
    let object = value
        .as_object()
        .ok_or_else(|| ImError::Parse("forward payload must be object".into()))?;
    if object.keys().any(|key| {
        ![
            "post_ids",
            "target_channel_ids",
            "mode",
            "command_id",
            "req_id",
        ]
        .contains(&key.as_str())
    }) {
        return Err(ImError::Parse(
            "forward payload has undeclared fields".into(),
        ));
    }
    let text = |key| {
        crate::query::render_ready::forward::text(value, key, 128).and_then(|s| {
            if s.is_empty() {
                Err(ImError::Parse(format!("missing forward {key}")))
            } else {
                Ok(s.to_owned())
            }
        })
    };
    let sources = ids(value, "post_ids", 100)?;
    let targets = ids(value, "target_channel_ids", 50)?;
    if targets.iter().any(|id| ChannelId::from_str(id).is_none()) {
        return Err(ImError::Parse("invalid forward target ID".into()));
    }
    let mode = match value["mode"].as_str() {
        Some("item") => "individual",
        Some("merge") => "merged",
        _ => return Err(ImError::Parse("invalid forward mode".into())),
    };
    Ok((text("req_id")?, text("command_id")?, sources, targets, mode))
}

fn ids(value: &Value, key: &str, max: usize) -> Result<Vec<String>, ImError> {
    let bad = || ImError::Parse(format!("invalid forward {key}"));
    let values = value[key]
        .as_array()
        .filter(|a| !a.is_empty() && a.len() <= max)
        .ok_or_else(bad)?;
    let mut unique = BTreeSet::new();
    values
        .iter()
        .map(|v| {
            let id = v
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)
                .ok_or_else(bad)?;
            if !unique.insert(id) {
                return Err(bad());
            }
            Ok(id.to_owned())
        })
        .collect()
}

pub(crate) fn start(
    module: &mut ImModule,
    payload: &[u8],
    _now_ms: u64,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|_| ImError::Parse("invalid forward JSON".into()))?;
    let (req_id, command_id, sources, targets, mode) = request(&value)?;
    let corr = module.alloc_corr_internal();
    let effects = crate::commands::handle_outbound(
        "im_create_posts",
        payload,
        &module.config.api_base_url,
        &module.config.default_api_base_url,
        module.state.connection_id.as_deref(),
        corr,
    )?;
    let timer = module.alloc_timer();
    if let Err(error) = module.state.pending_forward_deliveries.register(
        &req_id,
        &command_id,
        &targets,
        timer,
        if mode == "merged" { 1 } else { sources.len() },
    ) {
        out.push(
            crate::event::post::batch_target_error(&req_id, &targets, &error.to_string())?
                .into_effect(),
        );
        return Ok(());
    }
    module
        .state
        .corr_map
        .insert(corr, CorrelationContext::OutboundCreatePosts { req_id });
    out.push(Effect::ScheduleTimer {
        id: timer,
        after_ms: module.config.send_timeout_ms,
    });
    for effect in effects {
        out.push(effect);
    }
    Ok(())
}

/// Retry proof reads only the durable identities declared by Go, then persists and reads them back.
#[derive(Debug, Clone, PartialEq)]
pub struct CommittedRead {
    pub req_id: String,
    pub channel_id: ChannelId,
    pub post_ids: Vec<String>,
    pub temporary_ids: Vec<String>,
    pub next: usize,
    pub rows: Vec<Value>,
}

impl ImModule {
    pub(crate) fn forward_acceptance(
        &mut self,
        req_id: &str,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if self
            .state
            .pending_forward_deliveries
            .timer(req_id)
            .is_none()
        {
            return Ok(());
        }
        let result = (|| {
            let PortOutcome::Ok(reply) = outcome else {
                return Err(ImError::Parse("forward HTTP failed".into()));
            };
            let raw = crate::http_envelope::unwrap_success_envelope(&reply.0, "posts/createPosts")?;
            let body: Value = serde_json::from_slice(&raw)
                .map_err(|_| ImError::Parse("invalid forward response".into()))?;
            if body["status"] != "SUCCESS" {
                return Err(ImError::Parse("forward rejected".into()));
            }
            let targets = body
                .pointer("/data/targets")
                .and_then(Value::as_array)
                .ok_or_else(|| ImError::Parse("missing forward targets".into()))?;
            let (expected, post_count) = self
                .state
                .pending_forward_deliveries
                .by_request
                .get(req_id)
                .map(|d| (d.all_targets.clone(), d.post_count))
                .ok_or_else(|| ImError::Parse("missing forward attempt".into()))?;
            let mut seen = BTreeSet::new();
            let mut committed = Vec::new();
            for target in targets {
                let channel = target["channelId"]
                    .as_str()
                    .filter(|c| expected.iter().any(|e| e == c))
                    .ok_or_else(|| ImError::Parse("unexpected forward target".into()))?;
                if !seen.insert(channel.to_owned()) {
                    return Err(ImError::Parse("duplicate forward target".into()));
                }
                let channel_id = ChannelId::from_str(channel)
                    .ok_or_else(|| ImError::Parse("invalid forward channel".into()))?;
                match target["status"].as_str() {
                    Some("accepted") => {}
                    Some("committed") => {
                        let post_ids = ids(target, "postIds", 100)?;
                        let temporary_ids = ids(target, "temporaryIds", 100)?;
                        if post_ids.len() != temporary_ids.len() || post_ids.len() != post_count {
                            return Err(ImError::Parse("invalid committed identities".into()));
                        }
                        committed.push(CommittedRead {
                            req_id: req_id.into(),
                            channel_id,
                            post_ids,
                            temporary_ids,
                            next: 0,
                            rows: Vec::new(),
                        });
                    }
                    Some("failed") => {}
                    _ => return Err(ImError::Parse("invalid forward acceptance".into())),
                }
            }
            if seen.len() != expected.len() {
                return Err(ImError::Parse("missing forward target".into()));
            }
            Ok((body, committed))
        })();
        let (body, committed) = match result {
            Ok(result) => result,
            Err(error) => {
                self.fail_forward(req_id, &error.to_string(), out)?;
                return Ok(());
            }
        };
        out.push(crate::event::post::batch_result_from_authority(req_id, &body)?.into_effect());
        for target in body["data"]["targets"].as_array().into_iter().flatten() {
            if target["status"] == "failed" {
                if let Some(channel) = target["channelId"].as_str().and_then(ChannelId::from_str) {
                    self.fail_forward_target(req_id, channel, out);
                }
            }
        }
        for read in committed {
            if self
                .state
                .pending_forward_deliveries
                .matching_request(req_id, read.channel_id)
                .is_none()
            {
                continue;
            }
            let corr = self.alloc_corr_internal();
            let args = json!({"post_ids":read.post_ids,"req_id":req_id});
            let effects = crate::commands::handle_outbound(
                "im_get_posts",
                args.to_string().as_bytes(),
                &self.config.api_base_url,
                &self.config.default_api_base_url,
                self.state.connection_id.as_deref(),
                corr,
            )?;
            self.state.corr_map.insert(
                corr,
                CorrelationContext::ForwardCommittedHttp {
                    read: Box::new(read),
                },
            );
            for effect in effects {
                out.push(effect);
            }
        }
        Ok(())
    }

    pub(crate) fn complete_forward_post(
        &mut self,
        req: &str,
        channel: ChannelId,
        post_id: &str,
        out: &mut EffectSink,
    ) {
        let timer = self.state.pending_forward_deliveries.timer(req);
        if self
            .state
            .pending_forward_deliveries
            .complete_post(req, channel, post_id)
        {
            if let Some(timer) = timer {
                out.push(Effect::CancelTimer { id: timer });
            }
            self.clear_forward_correlations(req);
        }
    }

    fn fail_forward_target(&mut self, req: &str, channel: ChannelId, out: &mut EffectSink) {
        let timer = self.state.pending_forward_deliveries.timer(req);
        if self
            .state
            .pending_forward_deliveries
            .complete_target(req, channel)
        {
            if let Some(timer) = timer {
                out.push(Effect::CancelTimer { id: timer });
            }
            self.clear_forward_correlations(req);
        }
    }

    pub(crate) fn forward_timeout(
        &mut self,
        timer: TimerId,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let req = self
            .state
            .pending_forward_deliveries
            .by_request
            .iter()
            .find_map(|(id, d)| (d.timer == timer).then(|| id.clone()));
        if let Some(req) = req {
            self.fail_forward(&req, "FORWARD_TIMEOUT", out)?;
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn cancel_forwards(&mut self, out: &mut EffectSink) -> Result<(), ImError> {
        let requests = self
            .state
            .pending_forward_deliveries
            .by_request
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for req in requests {
            self.fail_forward(&req, "CANCELLED", out)?;
        }
        Ok(())
    }

    pub(crate) fn fail_forward(
        &mut self,
        req: &str,
        reason: &str,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let targets = self.state.pending_forward_deliveries.targets(req);
        if let Some(delivery) = self.state.pending_forward_deliveries.by_request.remove(req) {
            out.push(Effect::CancelTimer { id: delivery.timer });
            self.clear_forward_correlations(req);
            out.push(crate::event::post::batch_target_error(req, &targets, reason)?.into_effect());
        }
        Ok(())
    }

    /// A WS-first success also releases its still-pending HTTP continuation.
    fn clear_forward_correlations(&mut self, req: &str) {
        self.state.corr_map.retain(|_, ctx| match ctx {
            CorrelationContext::OutboundCreatePosts { req_id } => req_id != req,
            CorrelationContext::ForwardCommittedHttp { read }
            | CorrelationContext::ForwardCommittedPersist { read }
            | CorrelationContext::ForwardCommittedReadback { read } => read.req_id != req,
            _ => true,
        });
    }

    pub(crate) fn forward_committed_http(
        &mut self,
        read: CommittedRead,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if self
            .state
            .pending_forward_deliveries
            .matching_request(&read.req_id, read.channel_id)
            .is_none()
        {
            return Ok(());
        }
        let result = (|| {
            let PortOutcome::Ok(reply) = outcome else {
                return Err(ImError::Parse("committed post lookup failed".into()));
            };
            let raw = crate::http_envelope::unwrap_success_envelope(&reply.0, "posts/get")?;
            let body: Value = serde_json::from_slice(&raw)
                .map_err(|_| ImError::Parse("invalid committed post reply".into()))?;
            if body["status"] != "SUCCESS" {
                return Err(ImError::Parse("committed post lookup rejected".into()));
            }
            let posts = body
                .pointer("/data/posts")
                .or_else(|| body.get("data"))
                .and_then(Value::as_array)
                .ok_or_else(|| ImError::Parse("missing committed posts".into()))?;
            if posts.len() != read.post_ids.len() {
                return Err(ImError::Parse("incomplete committed posts".into()));
            }
            let mut ordered = Vec::with_capacity(posts.len());
            for (id, temp) in read.post_ids.iter().zip(&read.temporary_ids) {
                let post = posts
                    .iter()
                    .find(|p| p["id"] == *id)
                    .ok_or_else(|| ImError::Parse("missing committed identity".into()))?;
                let fields = crate::ws::parser::extract_post_fields(post);
                if fields.channel_id != read.channel_id.as_str() || fields.temporary_id != *temp {
                    return Err(ImError::Parse("committed post scope mismatch".into()));
                }
                ordered.push(post.clone());
            }
            let (keys, ops) = self.collect_exact_posts_cache_ops(&read.post_ids, ordered);
            if keys.len() != read.post_ids.len()
                || self.local_store_mode == crate::query::LocalStoreMode::Disabled
            {
                return Err(ImError::Parse(
                    "committed posts unavailable for durable proof".into(),
                ));
            }
            Ok(ops)
        })();
        match result {
            Ok(ops) => {
                let corr = self.alloc_corr_internal();
                self.state.corr_map.insert(
                    corr,
                    CorrelationContext::ForwardCommittedPersist {
                        read: Box::new(read),
                    },
                );
                out.push(Effect::Persist { corr, ops });
            }
            Err(error) => self.fail_forward(&read.req_id, &error.to_string(), out)?,
        }
        Ok(())
    }

    pub(crate) fn forward_committed_persist(
        &mut self,
        read: CommittedRead,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if matches!(outcome, PortOutcome::Err(_)) {
            return self.fail_forward(&read.req_id, "committed post persist failed", out);
        }
        self.forward_readback(read, out);
        Ok(())
    }

    fn forward_readback(&mut self, read: CommittedRead, out: &mut EffectSink) {
        if self
            .state
            .pending_forward_deliveries
            .matching_request(&read.req_id, read.channel_id)
            .is_none()
        {
            return;
        }
        let corr = self.alloc_corr_internal();
        let filter = (
            "temporary_id",
            SqlValue::Text(read.temporary_ids[read.next].clone()),
        );
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ForwardCommittedReadback {
                read: Box::new(read),
            },
        );
        out.push(Effect::Persist {
            corr,
            ops: vec![StorageOp::Scan(ScanSpec {
                table: "message",
                filter: Some(filter),
                limit: Some(1),
                order_by: &[],
            })],
        });
    }

    pub(crate) fn forward_committed_readback(
        &mut self,
        mut read: CommittedRead,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if self
            .state
            .pending_forward_deliveries
            .matching_request(&read.req_id, read.channel_id)
            .is_none()
        {
            return Ok(());
        }
        let row = match outcome {
            PortOutcome::Ok(reply) => crate::query::local_first::parse_local_rows(&reply.0)
                .ok()
                .and_then(|r| r.into_iter().next()),
            _ => None,
        };
        let Some(row) = row else {
            return self.fail_forward(&read.req_id, "committed post readback failed", out);
        };
        let projected =
            crate::query::render_ready::core::shape_row(&row, &self.config.auth_user_id);
        if projected["id"] != read.post_ids[read.next]
            || projected["temporaryId"] != read.temporary_ids[read.next]
            || projected["channelId"] != read.channel_id.as_str()
        {
            return self.fail_forward(&read.req_id, "committed post readback mismatch", out);
        }
        read.rows.push(projected);
        read.next += 1;
        if read.next < read.post_ids.len() {
            self.forward_readback(read, out);
            return Ok(());
        }
        for mut row in read.rows {
            let post_id = row["id"].as_str().unwrap_or_default().to_owned();
            if self.state.pending_forward_deliveries.is_final_post(
                &read.req_id,
                read.channel_id,
                &post_id,
            ) {
                row["requestId"] = json!(read.req_id);
            }
            out.push(crate::event::post::received(row)?.into_effect());
            self.complete_forward_post(&read.req_id, read.channel_id, &post_id, out);
        }
        Ok(())
    }
}
