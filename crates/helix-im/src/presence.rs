//! Ephemeral, bounded presence observations. All transport and time come from Effects/Ticks.
use crate::{error::ImError, module::ImModule};
use bytes::Bytes;
use helix_core::effect::{TimerId, TransportId};
use helix_core::tick::PortOutcome;
use helix_core::{Correlation, Effect, EffectSink, Tick};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(crate) const COMMAND: &str = "im_presence_observe";
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Observation {
    scope_id: String,
    user_ids: Vec<String>,
    focused_user_ids: Vec<String>,
}
struct Cached {
    status: String,
    sampled: u64,
    expires: u64,
}
struct Pending {
    ids: Vec<String>,
    started: u64,
}
#[derive(Default)]
struct Retry {
    failures: u32,
    not_before: u64,
}
#[derive(Default)]
pub(crate) struct Presence {
    scopes: BTreeMap<String, Observation>,
    users: BTreeSet<String>,
    focused: BTreeSet<String>,
    cache: BTreeMap<String, Cached>,
    pending: HashMap<Correlation, Pending>,
    last_attempt: BTreeMap<String, u64>,
    retries: BTreeMap<String, Retry>,
    generation: u64,
    timer: Option<TimerId>,
    next_poll: u64,
    next_watch: u64,
    last_checked_at: Option<u64>,
    clock_offset_upper: Option<i128>,
    emitted: Option<BTreeMap<String, String>>,
}

impl Presence {
    // Derive a repeatable phase from host/server identity; no ambient randomness in the kernel.
    fn phase(module: &ImModule) -> u64 {
        module
            .config
            .auth_user_id
            .bytes()
            .chain(module.state.connection_id.as_deref().unwrap_or("").bytes())
            .fold(0u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(16_777_619)
            })
            % 5_001
    }

    fn failed(&mut self, pending: &Pending) {
        for id in &pending.ids {
            let retry = self.retries.entry(id.clone()).or_default();
            retry.failures = retry.failures.saturating_add(1).min(5);
            let delay = (30_000u64 << (retry.failures - 1)).min(300_000);
            retry.not_before = pending.started.saturating_add(delay);
        }
    }

    fn emit(&mut self, now: u64, out: &mut EffectSink) {
        let requested: BTreeSet<_> = self
            .scopes
            .values()
            .flat_map(|s| s.user_ids.iter().chain(&s.focused_user_ids))
            .collect();
        if requested.is_empty() && self.emitted.is_none() {
            return;
        }
        let statuses: BTreeMap<_, _> = requested
            .into_iter()
            .map(|id| {
                let status = self
                    .cache
                    .get(id)
                    .filter(|c| c.expires > now)
                    .map(|c| c.status.as_str())
                    .unwrap_or("unknown");
                (id.clone(), status.to_owned())
            })
            .collect();
        if self.emitted.as_ref() == Some(&statuses) {
            return;
        }
        self.emitted = Some(statuses.clone());
        if let Ok(event) =
            crate::event::MessageV3Event::new("im:presence:update", json!({"statuses":statuses}))
        {
            out.push(event.into_effect());
        }
    }

    fn arm(&mut self, module: &mut ImModule, now: u64, out: &mut EffectSink) {
        if self.timer.is_none() && !self.users.is_empty() {
            let id = module.alloc_timer();
            self.timer = Some(id);
            let after_ms = self
                .cache
                .values()
                .map(|c| c.expires.saturating_sub(now))
                .min()
                .unwrap_or(5_000)
                .min(self.next_poll.saturating_sub(now))
                .clamp(1, 5_000);
            out.push(Effect::ScheduleTimer { id, after_ms });
        }
    }

    fn watch(&mut self, module: &mut ImModule, now: u64, out: &mut EffectSink) {
        if module.state.connection_id.is_none() {
            return;
        }
        let frame = json!({"action":"presence_watch", "seq": module.state.next_client_seq(),
            "data":{"userIds":self.focused,"generation":self.generation}});
        out.push(Effect::Send {
            transport: TransportId::from_raw(0),
            frame: Bytes::from(frame.to_string()),
        });
        self.next_watch = now.saturating_add(20_000);
    }

    fn query(
        &mut self,
        module: &mut ImModule,
        now: u64,
        snapshot: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let ids: Vec<_> = self
            .users
            .iter()
            .filter(|id| snapshot || !self.focused.contains(*id))
            .filter(|id| {
                self.retries
                    .get(*id)
                    .is_none_or(|retry| now >= retry.not_before)
            })
            .filter(|id| {
                !snapshot
                    || ((!self.cache.get(*id).is_some_and(|c| c.expires > now)
                        || !self.last_attempt.contains_key(*id))
                        && !self
                            .last_attempt
                            .get(*id)
                            .is_some_and(|at| now.saturating_sub(*at) < 30_000))
            })
            .cloned()
            .collect();
        for chunk in ids.chunks(100) {
            // At most one request per user remains in flight; timeout is handled by the presence timer.
            let chunk: Vec<_> = chunk
                .iter()
                .filter(|id| !self.pending.values().any(|p| p.ids.contains(id)))
                .cloned()
                .collect();
            if chunk.is_empty() {
                continue;
            }
            let corr = module.alloc_corr();
            let effects = crate::outbound::handle_outbound(
                "im_get_user_statuses",
                json!({"user_ids":chunk,"include_metadata":true})
                    .to_string()
                    .as_bytes(),
                &module.config.api_base_url,
                &module.config.default_api_base_url,
                module.state.connection_id.as_deref(),
                corr,
            )?;
            for id in &chunk {
                self.last_attempt.insert(id.clone(), now);
            }
            self.pending.insert(
                corr,
                Pending {
                    ids: chunk,
                    started: now,
                },
            );
            for effect in effects {
                out.push(effect);
            }
        }
        if self.next_poll == 0 {
            self.next_poll = now.saturating_add(30_000 + Self::phase(module));
        } else if !snapshot {
            self.next_poll = now.saturating_add(30_000);
        }
        Ok(())
    }

    fn observe(
        &mut self,
        module: &mut ImModule,
        payload: &[u8],
        now: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let observation: Observation =
            serde_json::from_slice(payload).map_err(|e| ImError::Parse(e.to_string()))?;
        if observation.scope_id.is_empty()
            || observation.scope_id.len() > 256
            || observation.user_ids.len() > 200
            || observation.focused_user_ids.len() > 200
            || observation
                .user_ids
                .iter()
                .chain(&observation.focused_user_ids)
                .any(|id| id.is_empty() || id.len() > 128)
        {
            return Err(ImError::Parse("invalid presence observation".into()));
        }
        let mut scopes = self.scopes.clone();
        if observation.user_ids.is_empty() && observation.focused_user_ids.is_empty() {
            scopes.remove(&observation.scope_id);
        } else {
            scopes.insert(observation.scope_id.clone(), observation);
        }
        let users: BTreeSet<_> = scopes
            .values()
            .flat_map(|s| s.user_ids.iter().chain(&s.focused_user_ids))
            .cloned()
            .collect();
        if scopes.len() > 64 || observation_size(&scopes) {
            return Err(ImError::Parse("presence observation limit exceeded".into()));
        }
        let all_focused: BTreeSet<_> = scopes
            .values()
            .flat_map(|s| s.focused_user_ids.iter())
            .cloned()
            .collect();
        let focused: BTreeSet<_> = all_focused.iter().take(10).cloned().collect();
        let users: BTreeSet<_> = all_focused
            .iter()
            .chain(users.iter().filter(|id| !all_focused.contains(*id)))
            .take(200)
            .cloned()
            .collect();
        if !users.is_empty() && module.config.api_base_url.trim().is_empty() {
            return Err(ImError::Parse(
                "presence requires the authenticated API base URL".into(),
            ));
        }
        self.scopes = scopes;
        if !self.scopes.is_empty() {
            self.emitted = None;
        }
        if users == self.users && focused == self.focused {
            self.query(module, now, true, out)?;
            self.emit(now, out);
            return Ok(());
        }
        let focus_changed = focused != self.focused;
        if focus_changed {
            self.generation = self.generation.saturating_add(1);
            self.last_checked_at = None;
        }
        // Keep intersection requests alive; removing an ID severs its old correlation even if it is re-added later.
        self.pending.retain(|_, pending| {
            pending.ids.retain(|id| users.contains(id));
            !pending.ids.is_empty()
        });
        self.last_attempt.retain(|id, _| users.contains(id));
        self.retries.retain(|id, _| users.contains(id));
        self.cache.retain(|id, _| users.contains(id));
        // Refresh mode changes without blanking a still-valid sample or extending its lifetime.
        for id in focused.symmetric_difference(&self.focused) {
            self.last_attempt.remove(id);
            self.retries.remove(id);
        }
        for id in focused.difference(&self.focused) {
            if let Some(cached) = self.cache.get_mut(id) {
                cached.expires = cached.expires.min(now.saturating_add(15_000));
            }
        }
        self.users = users;
        self.focused = focused;
        if focus_changed {
            self.watch(module, now, out);
        }
        self.query(module, now, true, out)?;
        if self.users.is_empty() {
            self.next_poll = 0;
            if let Some(id) = self.timer.take() {
                out.push(Effect::CancelTimer { id });
            }
        } else {
            self.arm(module, now, out);
        }
        self.emit(now, out);
        Ok(())
    }

    fn apply(&mut self, ids: &[String], statuses: &Value, sampled: u64, expires: u64) {
        for id in ids {
            if !self.users.contains(id) || self.cache.get(id).is_some_and(|c| c.sampled > sampled) {
                continue;
            }
            let status = statuses
                .get(id)
                .and_then(Value::as_str)
                .filter(|s| matches!(*s, "online" | "offline"))
                .unwrap_or("unknown");
            self.cache.insert(
                id.clone(),
                Cached {
                    status: status.into(),
                    sampled,
                    expires,
                },
            );
        }
    }
}

impl ImModule {
    /// Return true only for a presence-owned command, reply or timer.
    pub(crate) fn presence_tick(
        &mut self,
        tick: &Tick,
        now: u64,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let owned = match tick {
            Tick::Command(c) => c.name.as_ref() == COMMAND,
            Tick::PortReply { corr, .. } => self.presence.pending.contains_key(corr),
            Tick::Timer(id) => self.presence.timer == Some(*id),
            _ => false,
        };
        if !owned {
            return Ok(false);
        }
        let mut presence = std::mem::take(&mut self.presence);
        tracing::debug!(target: "helix::presence", hop = "presence_tick", generation = presence.generation, "presence input");
        let result = (|| {
            match tick {
                Tick::Command(c) => presence.observe(self, &c.payload, now, out)?,
                Tick::PortReply { corr, outcome } => {
                    if let Some(pending) = presence.pending.remove(corr) {
                        let mut succeeded = false;
                        if now.saturating_sub(pending.started) < 15_000 {
                            let statuses = match outcome {
                                PortOutcome::Ok(reply) => {
                                    crate::http_envelope::unwrap_sync_envelope(&reply.0)
                                        .ok()
                                        .and_then(|raw| serde_json::from_slice::<Value>(&raw).ok())
                                        .filter(|v| {
                                            v.get("status")
                                                .is_none_or(|s| s.as_str() == Some("SUCCESS"))
                                        })
                                        .map(|v| v.get("data").cloned().unwrap_or(v))
                                }
                                _ => None,
                            };
                            // Failed requests cannot extend the freshness of an old observation.
                            if let Some(metadata) = statuses {
                                if let (Some(checked_at), Some(statuses)) = (
                                    metadata.get("checkedAt").and_then(Value::as_u64),
                                    metadata.get("statuses").filter(|v| v.is_object()),
                                ) {
                                    // The sample happened after request dispatch; this upper bound needs no shared client/server clock.
                                    let offset =
                                        i128::from(checked_at) - i128::from(pending.started);
                                    presence.clock_offset_upper = Some(
                                        presence
                                            .clock_offset_upper
                                            .map_or(offset, |old| old.min(offset)),
                                    );
                                    succeeded = true;
                                    for id in &pending.ids {
                                        presence.retries.remove(id);
                                    }
                                    let (focused, ordinary): (Vec<_>, Vec<_>) = pending
                                        .ids
                                        .iter()
                                        .cloned()
                                        .filter(|id| {
                                            !presence
                                                .cache
                                                .get(id)
                                                .is_some_and(|c| c.sampled >= checked_at)
                                        })
                                        .partition(|id| presence.focused.contains(id));
                                    presence.apply(
                                        &focused,
                                        statuses,
                                        checked_at,
                                        pending.started.saturating_add(15_000),
                                    );
                                    presence.apply(
                                        &ordinary,
                                        statuses,
                                        checked_at,
                                        pending.started.saturating_add(45_000),
                                    );
                                }
                            }
                            presence.emit(now, out);
                        }
                        if !succeeded {
                            presence.failed(&pending);
                        }
                    }
                }
                Tick::Timer(_) => {
                    presence.timer = None;
                    let expired: Vec<_> = presence
                        .pending
                        .iter()
                        .filter(|(_, p)| now.saturating_sub(p.started) >= 15_000)
                        .map(|(corr, _)| *corr)
                        .collect();
                    for corr in expired {
                        if let Some(pending) = presence.pending.remove(&corr) {
                            presence.failed(&pending);
                        }
                    }
                    presence.cache.retain(|_, c| c.expires > now);
                    if now >= presence.next_poll {
                        presence.query(self, now, false, out)?;
                    }
                    if now >= presence.next_watch && !presence.focused.is_empty() {
                        presence.watch(self, now, out);
                        if presence.clock_offset_upper.is_none() {
                            presence.query(self, now, true, out)?;
                        }
                    }
                    presence.emit(now, out);
                    presence.arm(self, now, out);
                }
                _ => {}
            }
            Ok(true)
        })();
        self.presence = presence;
        result
    }

    /// A successful server sample is a full snapshot; omitted/revoked IDs immediately become unknown.
    pub(crate) fn presence_inbound(
        &mut self,
        data: Option<&Value>,
        now: u64,
        out: &mut EffectSink,
    ) {
        let Some(data) = data else { return };
        if data.get("generation").and_then(Value::as_u64) != Some(self.presence.generation) {
            return;
        }
        let Some(checked_at) = data.get("checkedAt").and_then(Value::as_u64) else {
            return;
        };
        if self
            .presence
            .last_checked_at
            .is_some_and(|last| checked_at <= last)
        {
            return;
        }
        let Some(statuses) = data.get("statuses").filter(|s| s.is_object()) else {
            return;
        };
        tracing::debug!(target: "helix::presence", hop = "presence_inbound", generation = self.presence.generation, "presence snapshot");
        self.presence.last_checked_at = Some(checked_at);
        let ids: Vec<_> = self.presence.focused.iter().cloned().collect();
        let Some(offset) = self.presence.clock_offset_upper else {
            return;
        };
        let expires = ((i128::from(checked_at) - offset + 15_000).clamp(0, i128::from(u64::MAX))
            as u64)
            .min(now.saturating_add(15_000));
        if expires <= now {
            return;
        }
        self.presence.apply(&ids, statuses, checked_at, expires);
        self.presence.emit(now, out);
    }

    /// Reconnect gets a new generation so a delayed frame cannot renew the old observation.
    pub(crate) fn presence_connected(
        &mut self,
        now: u64,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut presence = std::mem::take(&mut self.presence);
        presence.generation = presence.generation.saturating_add(1);
        presence.last_checked_at = None;
        presence.clock_offset_upper = None;
        presence.pending.clear();
        presence.last_attempt.clear();
        presence.retries.clear();
        presence.next_poll = 0;
        presence.cache.clear();
        if !presence.focused.is_empty() {
            presence.watch(self, now, out);
        }
        let result = presence.query(self, now, true, out);
        presence.arm(self, now, out);
        presence.emit(now, out);
        self.presence = presence;
        result
    }

    /// No persistent presence survives disconnect, identity changes or module shutdown.
    pub(crate) fn presence_clear(&mut self, release: bool, now: u64, out: &mut EffectSink) {
        let mut presence = std::mem::take(&mut self.presence);
        presence.generation = presence.generation.saturating_add(1);
        presence.last_checked_at = None;
        presence.clock_offset_upper = None;
        presence.cache.clear();
        presence.pending.clear();
        presence.last_attempt.clear();
        presence.retries.clear();
        presence.next_poll = 0;
        if let Some(id) = presence.timer.take() {
            out.push(Effect::CancelTimer { id });
        }
        presence.emit(now, out);
        if release {
            let had_focus = !presence.focused.is_empty();
            presence.scopes.clear();
            presence.users.clear();
            presence.focused.clear();
            if had_focus {
                presence.watch(self, now, out);
            }
        }
        self.presence = presence;
    }
}

// Individual scopes are bounded before cross-window observations are prioritized.
fn observation_size(scopes: &BTreeMap<String, Observation>) -> bool {
    scopes.values().any(|s| {
        s.user_ids
            .iter()
            .chain(&s.focused_user_ids)
            .collect::<BTreeSet<_>>()
            .len()
            > 200
    })
}
