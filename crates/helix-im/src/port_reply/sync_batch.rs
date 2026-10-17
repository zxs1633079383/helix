use crate::{error::ImError, module::ImModule, state::ChannelId};
use helix_core::{
    tick::{PortError, PortOutcome},
    Correlation, EffectSink,
};
use std::collections::HashSet;

// Validate the complete response before applying any group: a missing entry is not no_change.
fn validate_batch(
    requests: &[(Correlation, ChannelId)],
    outcome: &PortOutcome,
) -> Result<(), String> {
    let PortOutcome::Ok(reply) = outcome else {
        return Ok(());
    };
    let raw =
        crate::http_envelope::unwrap_sync_envelope(reply.0.as_ref()).map_err(|e| e.to_string())?;
    let value: serde_json::Value = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let entries = value
        .pointer("/data/entries")
        .and_then(|v| v.as_array())
        .ok_or("missing sync entries")?;
    let expected: HashSet<_> = requests.iter().map(|(_, id)| id.as_str()).collect();
    let mut seen = HashSet::new();
    for entry in entries {
        let id = entry
            .get("channelId")
            .and_then(|v| v.as_str())
            .ok_or("missing channel identity")?;
        if !expected.contains(id) || !seen.insert(id) {
            return Err("unexpected or duplicate sync entry".into());
        }
    }
    if seen.len() != expected.len() {
        return Err("incomplete sync batch".into());
    }
    for (_, id) in requests {
        crate::parser::parse_sync_response(&raw, *id).map_err(|e| e.to_string())?;
    }
    Ok(())
}

impl ImModule {
    pub(super) fn handle_sync_batch_reply(
        &mut self,
        requests: Vec<(Correlation, ChannelId)>,
        outcome: &PortOutcome,
        now_ms: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let invalid = validate_batch(&requests, outcome).err().map(|e| {
            tracing::warn!(reason = %e, "invalid sync batch");
            PortOutcome::Err(PortError::Other(1))
        });
        let outcome = invalid.as_ref().unwrap_or(outcome);
        // Refill the window once, after every child settles; otherwise each reply creates a singleton.
        self.state.sync_scheduler.applying_batch = true;
        let mut error = None;
        for (corr, _) in requests {
            if !self.state.corr_map.contains_key(&corr) {
                continue;
            }
            if let Err(e) = self.dispatch_port_reply(corr, outcome, now_ms, out) {
                error.get_or_insert(e);
            }
        }
        self.state.sync_scheduler.applying_batch = false;
        if let Some(error) = error {
            return Err(error);
        }
        self.drain_sync_queue(out);
        Ok(())
    }
}
