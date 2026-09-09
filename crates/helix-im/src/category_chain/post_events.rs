//! Refresh only category post snapshots after the write barrier, preserving the ordered event batch.
use crate::{error::ImError, module::ImModule, state::CorrelationContext};
use helix_core::effect::{DomainEventBytes, GetSpec, SqlValue, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};
use serde_json::Value;
use std::collections::VecDeque;

#[derive(Debug, Clone, PartialEq)]
pub struct Readback {
    pub events: Vec<Value>,
    pub targets: VecDeque<(usize, String, String)>,
}

impl ImModule {
    /// Plain/text-only batches keep their original bytes; category batches release durable snapshots in order.
    pub(crate) fn release_post_events(
        &mut self,
        events: Vec<Vec<u8>>,
        category: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if !category {
            for event in events {
                out.push(Effect::Emit {
                    event: DomainEventBytes(event.into()),
                });
            }
            return Ok(());
        }
        let mut deferred = Vec::new();
        let mut targets = VecDeque::new();
        for raw in events {
            let event: Value = serde_json::from_slice(&raw)
                .map_err(|e| ImError::Parse(format!("category event: {e}")))?;
            let index = deferred.len();
            let before = targets.len();
            if let Some(data) = event.get("data") {
                collect_target(index, "/data".to_owned(), data, &mut targets);
                if let Some(posts) = data.get("posts").and_then(Value::as_array) {
                    for (i, post) in posts.iter().enumerate() {
                        collect_target(index, format!("/data/posts/{i}"), post, &mut targets);
                    }
                }
                if let Some(post) = data.get("lastPost") {
                    collect_target(index, "/data/lastPost".to_owned(), post, &mut targets);
                }
            }
            if targets.len() == before {
                // Mixed sync batches must not delay or re-encode legacy text-chain events.
                out.push(Effect::Emit {
                    event: DomainEventBytes(raw.into()),
                });
            } else {
                deferred.push(event);
            }
        }
        self.continue_category_post_events(
            Readback {
                events: deferred,
                targets,
            },
            out,
        )
    }

    /// Sequential reads keep one bounded continuation and preserve the original event ordering.
    fn continue_category_post_events(
        &mut self,
        pending: Readback,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if let Some((_, _, temporary_id)) = pending.targets.front() {
            let op = StorageOp::Get(GetSpec {
                table: "message",
                key_col: "temporary_id",
                key_val: SqlValue::Text(temporary_id.clone()),
            });
            let corr = self.alloc_corr_internal();
            tracing::debug!(
                hop = "category.post.readback",
                corr = corr.raw(),
                "refresh committed category post"
            );
            self.state.corr_map.insert(
                corr,
                CorrelationContext::CategoryPostReadback {
                    pending: Box::new(pending),
                },
            );
            out.push(Effect::Persist {
                corr,
                ops: vec![op],
            });
        } else {
            for event in pending.events {
                let bytes = serde_json::to_vec(&event) // hot-path-audit: ignore — EventSink encoding only; never written to a storage column.
                    .map_err(|e| ImError::Parse(format!("category emit: {e}")))?;
                out.push(Effect::Emit {
                    event: DomainEventBytes(bytes.into()),
                });
            }
        }
        Ok(())
    }

    /// A failed or missing durable read suppresses the batch instead of publishing pre-commit authority.
    pub(crate) fn handle_category_post_readback(
        &mut self,
        mut pending: Readback,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let PortOutcome::Ok(bytes) = outcome else {
            return Ok(());
        };
        let rows: Value = serde_json::from_slice(&bytes.0)
            .map_err(|e| ImError::Parse(format!("category row: {e}")))?;
        let shaped =
            crate::render_ready::shape_message_rows_for_viewer(&rows, &self.config.auth_user_id);
        let Some(row) = shaped
            .as_array()
            .filter(|rows| rows.len() == 1)
            .and_then(|rows| rows.first())
            .and_then(Value::as_object)
        else {
            return Ok(());
        };
        let Some((index, path, _)) = pending.targets.pop_front() else {
            return Ok(());
        };
        let Some(target) = pending.events[index]
            .pointer_mut(&path)
            .and_then(Value::as_object_mut)
        else {
            return Ok(());
        };
        // Keep event correlation/sequence metadata; replace only the existing post projection fields.
        for (key, value) in target.iter_mut() {
            if matches!(key.as_str(), "eventSeq" | "event_seq") {
                continue;
            }
            if let Some(saved) = row.get(key) {
                *value = saved.clone();
            }
        }
        self.continue_category_post_events(pending, out)
    }
}

/// Classification runs only for a batch already known to contain category posts.
fn collect_target(
    index: usize,
    path: String,
    post: &Value,
    targets: &mut VecDeque<(usize, String, String)>,
) {
    if post.get("type").and_then(Value::as_str) != Some("CATEGORY_CHAIN") {
        return;
    }
    if let Some(id) = post
        .get("temporaryId")
        .or_else(|| post.get("temporary_id"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
    {
        targets.push_back((index, path, id.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ImConfig;
    use helix_core::{Module, Tick};

    /// A failed category read in a mixed sync batch cannot delay, re-encode or suppress TEXT-chain events.
    #[test]
    fn text_chain_bytes_survive_category_read_failure() {
        let mut module = ImModule::new(ImConfig::default());
        let text =
            b"{ \"event\": \"im:post_chain:upsert\", \"data\": {\"chainId\":\"legacy\"} }".to_vec();
        let category = br#"{"event":"im:post:updated","data":{"type":"CATEGORY_CHAIN","temporaryId":"category"}}"#.to_vec();
        let mut out = EffectSink::new();
        module
            .release_post_events(vec![category, text.clone()], true, &mut out)
            .unwrap();
        assert!(out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit {event} if event.0.as_ref() == text)));
        let corr = out
            .as_slice()
            .iter()
            .find_map(|effect| match effect {
                Effect::Persist { corr, .. } => Some(*corr),
                _ => None,
            })
            .unwrap();
        out.clear();
        module
            .handle(
                &Tick::PortReply {
                    corr,
                    outcome: PortOutcome::Err(helix_core::tick::PortError::Storage(0)),
                },
                1,
                &mut out,
            )
            .unwrap();
        assert!(!out
            .as_slice()
            .iter()
            .any(|effect| matches!(effect, Effect::Emit { .. })));
        assert!(!module.state.corr_map.contains_key(&corr));
    }
}
