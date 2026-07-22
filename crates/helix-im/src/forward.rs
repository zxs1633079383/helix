//! G-05 转发编排：UI 只传源消息 ID、目标频道和模式，Helix 从本地消息表装配完整 Post。

use helix_core::effect::{Effect, GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::EffectSink;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::error::ImError;
use crate::module::ImModule;
use crate::state::{ChannelId, CorrelationContext};

const MAX_FORWARD_POSTS: usize = 100;
const MAX_FORWARD_TARGETS: usize = 50;
const MAX_PENDING_FORWARD_REQUESTS: usize = 64;

/// Bounded request-to-target authority ledger for forward delivery.
///
/// HTTP acceptance never completes a target. A target remains eligible only until a trusted
/// `envelope.tracing.csesTrackId` post is durably projected with the same request id.
#[derive(Debug, Default)]
pub(crate) struct PendingForwardDeliveryLedger {
    targets_by_request: BTreeMap<String, BTreeSet<ChannelId>>,
    insertion_order: VecDeque<String>,
}

impl PendingForwardDeliveryLedger {
    pub(crate) fn register(&mut self, request_id: String, targets: &[String]) {
        let targets = targets
            .iter()
            .filter_map(|target| ChannelId::from_str(target))
            .collect::<BTreeSet<_>>();
        if targets.is_empty() {
            return;
        }
        if !self.targets_by_request.contains_key(&request_id) {
            while self.targets_by_request.len() >= MAX_PENDING_FORWARD_REQUESTS {
                let Some(oldest) = self.insertion_order.pop_front() else {
                    break;
                };
                self.targets_by_request.remove(&oldest);
            }
            self.insertion_order.push_back(request_id.clone());
        }
        self.targets_by_request.insert(request_id, targets);
    }

    pub(crate) fn matching_request(
        &self,
        request_id: &str,
        channel_id: ChannelId,
    ) -> Option<String> {
        self.targets_by_request
            .get(request_id)
            .is_some_and(|targets| targets.contains(&channel_id))
            .then(|| request_id.to_string())
    }

    pub(crate) fn retain_http_accepted_targets(&mut self, request_id: &str, body: &Value) {
        let Some(targets) = self.targets_by_request.get_mut(request_id) else {
            return;
        };
        let Some(results) = body
            .get("data")
            .and_then(|data| data.get("targets"))
            .and_then(Value::as_array)
        else {
            return;
        };
        for result in results {
            if result.get("status").and_then(Value::as_str) == Some("accepted") {
                continue;
            }
            if let Some(channel_id) = result
                .get("channelId")
                .and_then(Value::as_str)
                .and_then(ChannelId::from_str)
            {
                targets.remove(&channel_id);
            }
        }
        self.remove_if_empty(request_id);
    }

    /// Returns true only when this target completed the final still-accepted target.
    pub(crate) fn complete_target(&mut self, request_id: &str, channel_id: ChannelId) -> bool {
        let existed = self.targets_by_request.contains_key(request_id);
        if let Some(targets) = self.targets_by_request.get_mut(request_id) {
            targets.remove(&channel_id);
        }
        let completed = existed
            && self
                .targets_by_request
                .get(request_id)
                .is_some_and(BTreeSet::is_empty);
        self.remove_if_empty(request_id);
        completed
    }

    pub(crate) fn cancel(&mut self, request_id: &str) {
        self.targets_by_request.remove(request_id);
        self.insertion_order
            .retain(|candidate| candidate != request_id);
    }

    fn remove_if_empty(&mut self, request_id: &str) {
        if self
            .targets_by_request
            .get(request_id)
            .is_some_and(BTreeSet::is_empty)
        {
            self.cancel(request_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_target_ledger_completes_only_on_the_last_remaining_target() {
        let first = crate::state::test_channel_id(50);
        let last = crate::state::test_channel_id(51);
        let mut ledger = PendingForwardDeliveryLedger::default();
        ledger.register(
            "forward-1".to_string(),
            &[first.as_str().to_string(), last.as_str().to_string()],
        );

        assert!(!ledger.complete_target("forward-1", first));
        assert!(ledger.matching_request("forward-1", last).is_some());
        assert!(ledger.complete_target("forward-1", last));
        assert!(ledger.matching_request("forward-1", last).is_none());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardMode {
    Individual,
    Merged,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ForwardAssembly {
    pub(crate) req_id: String,
    pub(crate) source_post_ids: Vec<String>,
    pub(crate) target_channel_ids: Vec<String>,
    pub(crate) mode: ForwardMode,
    pub(crate) next_index: usize,
    pub(crate) rows: Vec<Value>,
    pub(crate) now_ms: u64,
}

pub(crate) fn is_source_id_request(payload: &[u8]) -> bool {
    serde_json::from_slice::<Value>(payload)
        .ok()
        .and_then(|value| value.get("post_ids").and_then(Value::as_array).cloned())
        .is_some()
}

pub(crate) fn start(
    module: &mut ImModule,
    payload: &[u8],
    now_ms: u64,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let value: Value = serde_json::from_slice(payload)
        .map_err(|error| ImError::Parse(format!("im_relay_messages payload: {error}")))?;
    require_exact_keys(
        &value,
        &["post_ids", "target_channel_ids", "mode", "req_id"],
    )?;
    let source_post_ids = required_strings(&value, "post_ids", MAX_FORWARD_POSTS)?;
    let target_channel_ids = required_strings(&value, "target_channel_ids", MAX_FORWARD_TARGETS)?;
    if target_channel_ids
        .iter()
        .any(|id| ChannelId::from_str(id).is_none())
    {
        return Err(ImError::Parse(
            "im_relay_messages: target_channel_ids 含非法频道 ID".to_string(),
        ));
    }
    let mode = match value
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("individual")
    {
        "individual" => ForwardMode::Individual,
        "merged" => ForwardMode::Merged,
        other => {
            return Err(ImError::Parse(format!(
                "im_relay_messages: 不支持的 mode '{other}'"
            )))
        }
    };
    let req_id = operation_req_id(module, &value, now_ms)?;
    let assembly = ForwardAssembly {
        req_id,
        source_post_ids,
        target_channel_ids,
        mode,
        next_index: 0,
        rows: Vec::new(),
        now_ms,
    };
    module
        .state
        .pending_forward_deliveries
        .register(assembly.req_id.clone(), &assembly.target_channel_ids);
    schedule_next_lookup(module, assembly, out);
    Ok(())
}

pub(crate) fn handle_lookup_reply(
    module: &mut ImModule,
    mut assembly: ForwardAssembly,
    outcome: &PortOutcome,
    out: &mut EffectSink,
) -> Result<(), ImError> {
    let source_id = assembly
        .source_post_ids
        .get(assembly.next_index)
        .cloned()
        .ok_or_else(|| ImError::Parse("im_relay_messages: 本地查询状态越界".to_string()))?;
    let row = match outcome {
        PortOutcome::Ok(reply) => first_row(reply.0.as_ref()),
        PortOutcome::Err(_) => None,
    };
    let Some(row) = row else {
        module
            .state
            .pending_forward_deliveries
            .cancel(&assembly.req_id);
        out.push(crate::query::render_ready::batch_result::emit_target_error(
            &assembly.req_id,
            &assembly.target_channel_ids,
            &format!("source post not found: {source_id}"),
        ));
        return Ok(());
    };
    assembly.rows.push(row);
    assembly.next_index += 1;
    if assembly.next_index < assembly.source_post_ids.len() {
        schedule_next_lookup(module, assembly, out);
        return Ok(());
    }

    let posts = crate::forward_shape::assemble_posts(module, &assembly)?;
    let args = json!({
        "req_id": assembly.req_id,
        "posts": posts,
        "channel_ids": assembly.target_channel_ids,
    });
    let payload =
        serde_json::to_vec(&args).map_err(|error| ImError::Serialize(error.to_string()))?;
    let corr = module.alloc_corr_internal();
    let effects = crate::commands::handle_outbound(
        "im_create_posts",
        &payload,
        module.config.api_base_url.as_str(),
        module.config.default_api_base_url.as_str(),
        module.state.connection_id.as_deref(),
        corr,
    )?;
    module.state.corr_map.insert(
        corr,
        CorrelationContext::OutboundCreatePosts {
            req_id: assembly.req_id,
        },
    );
    for effect in effects {
        out.push(effect);
    }
    Ok(())
}

fn schedule_next_lookup(module: &mut ImModule, assembly: ForwardAssembly, out: &mut EffectSink) {
    let source_id = assembly.source_post_ids[assembly.next_index].clone();
    let corr = module.alloc_corr_internal();
    module.state.corr_map.insert(
        corr,
        CorrelationContext::ForwardSourceLookup {
            assembly: Box::new(assembly),
        },
    );
    out.push(Effect::Persist {
        corr,
        ops: vec![StorageOp::Get(GetSpec {
            table: "message",
            key_col: "id",
            key_val: SqlValue::Text(source_id),
        })],
    });
}

fn first_row(reply: &[u8]) -> Option<Value> {
    serde_json::from_slice::<Value>(reply)
        .ok()?
        .as_array()?
        .first()
        .filter(|value| value.is_object())
        .cloned()
}

fn operation_req_id(module: &mut ImModule, value: &Value, now_ms: u64) -> Result<String, ImError> {
    match value.get("req_id") {
        None => {
            let sequence = module.alloc_corr_internal().raw();
            Ok(format!("relay-{now_ms}-{sequence}"))
        }
        Some(Value::String(req_id)) if !req_id.is_empty() => Ok(req_id.clone()),
        Some(_) => Err(ImError::Parse(
            "im_relay_messages: 'req_id' 必须为非空字符串".to_string(),
        )),
    }
}

fn required_strings(value: &Value, key: &str, max: usize) -> Result<Vec<String>, ImError> {
    let values = value
        .get(key)
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= max)
        .ok_or_else(|| {
            ImError::Parse(format!(
                "im_relay_messages: '{key}' 必须为 1..={max} 个字符串"
            ))
        })?;
    let values: Vec<String> = values
        .iter()
        .map(|item| {
            item.as_str()
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .ok_or_else(|| ImError::Parse(format!("im_relay_messages: '{key}' 含空值")))
        })
        .collect::<Result<_, _>>()?;
    let mut unique = BTreeSet::new();
    if values.iter().any(|item| !unique.insert(item.as_str())) {
        return Err(ImError::Parse(format!(
            "im_relay_messages: '{key}' 不允许重复值"
        )));
    }
    Ok(values)
}

fn require_exact_keys(value: &Value, allowed: &[&str]) -> Result<(), ImError> {
    let object = value
        .as_object()
        .ok_or_else(|| ImError::Parse("im_relay_messages: payload 必须是 object".to_string()))?;
    if let Some(unknown) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ImError::Parse(format!(
            "im_relay_messages: 未知或非 canonical 字段 '{unknown}'"
        )));
    }
    Ok(())
}
