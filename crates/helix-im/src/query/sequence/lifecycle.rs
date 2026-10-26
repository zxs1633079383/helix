use super::*;
use crate::{state::CorrelationContext, ImModule};
use bytes::Bytes;
use helix_core::effect::{HttpRequest, StorageOp};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink};
use serde_json::json;

impl ImModule {
    pub(crate) fn sequence_command(
        &mut self,
        name: &str,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let request = Request::parse(name, payload)?;
        let count = self
            .state
            .corr_map
            .values()
            .filter(|context| matches!(context, CorrelationContext::SequenceQuery { .. }))
            .count();
        if self.state.corr_map.values().any(|context|matches!(context,CorrelationContext::SequenceQuery{pending} if pending.req_id()==request.req_id)) { return Err(error("DUPLICATE_REQ_ID")); }
        if count >= MAX_PENDING {
            return Err(error("BUSY"));
        }
        if self.local_store_mode == crate::query::LocalStoreMode::Disabled {
            return self.handle_query_command(name, request.legacy.to_string().as_bytes(), out);
        }
        let timer = self.alloc_timer();
        let pending = Box::new(Pending {
            request,
            timer,
            query_session_epoch: self.state.query_session_epoch,
            stage: Stage::Metadata,
            meta: None,
            direction: Direction::Older,
            next: 0,
            lo: 0,
            hi: 0,
            accepted_lo: None,
            accepted_hi: None,
            scanned: 0,
            http: 0,
            restarts: 0,
            locate_pass: LocatePass::Before,
            slots: BTreeMap::new(),
            previous_keys: BTreeMap::new(),
            previous_revisions: BTreeMap::new(),
            bodies: BTreeMap::new(),
            rows: BTreeMap::new(),
        });
        out.push(Effect::ScheduleTimer {
            id: timer,
            after_ms: TIMEOUT_MS,
        });
        self.sequence_http(pending, None, out);
        Ok(())
    }

    fn sequence_http(
        &mut self,
        mut p: Box<Pending>,
        range: Option<(i64, i64)>,
        out: &mut EffectSink,
    ) {
        p.http += 1;
        let mut body = json!({"channelId":p.request.channel.as_str()});
        if let Some((lo, hi)) = range {
            let Some(meta) = &p.meta else {
                self.sequence_fail(&p, "MISSING_METADATA", out);
                return;
            };
            body["fromSeq"] = json!(lo.to_string());
            body["toSeq"] = json!(hi.to_string());
            body["snapshotHeadSeq"] = json!(meta.head.to_string());
            body["membershipEpoch"] = json!(meta.epoch.to_string());
            body["expectedRevision"] = json!(meta.revision.to_string());
            p.stage = Stage::Range(lo, hi);
        } else {
            if p.request.continuation.is_none() && !p.request.anchor.is_empty() {
                body["anchorPostId"] = json!(p.request.anchor);
            }
            if let Some(cursor) = &p.request.continuation {
                body["snapshotHeadSeq"] = json!(cursor.head.to_string());
            }
            p.stage = Stage::Metadata;
        }
        let corr = self.alloc_corr_internal();
        let mut headers = vec![
            ("Content-Type".into(), "application/json".into()),
            ("Cses-Track-Id".into(), p.request.req_id.clone()),
        ];
        headers.extend(crate::acl::sync_http_effects::session_auth_headers(
            self.state.connection_id.as_deref(),
        ));
        out.push(Effect::Http {
            corr,
            req: HttpRequest {
                method: "POST".into(),
                url: format!(
                    "{}/posts/sequenceRange",
                    self.config.api_base_url.trim_end_matches('/')
                ),
                headers,
                body: Some(Bytes::from(body.to_string())),
            },
        });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::SequenceQuery { pending: p });
    }

    fn sequence_storage(
        &mut self,
        mut p: Box<Pending>,
        stage: Stage,
        ops: Vec<StorageOp>,
        atomic: bool,
        out: &mut EffectSink,
    ) {
        p.stage = stage;
        let corr = self.alloc_corr_internal();
        out.push(if atomic {
            Effect::PersistAtomic { corr, ops }
        } else {
            Effect::Persist { corr, ops }
        });
        self.state
            .corr_map
            .insert(corr, CorrelationContext::SequenceQuery { pending: p });
    }

    fn sequence_fail(&self, p: &Pending, reason: &str, out: &mut EffectSink) {
        out.push(Effect::CancelTimer { id: p.timer });
        out.push(crate::read_relay::emit_read_error(
            &p.request.req_id,
            reason,
        ));
    }

    fn sequence_finish(&mut self, p: &Pending, out: &mut EffectSink) -> Result<(), ImError> {
        let result = data::result(p, &self.config.auth_user_id)?;
        if p.request.attach && matches!(p.request.operation, "latest" | "locate") {
            // Keep the existing live/readback subscription; history reads never advance sync state.
            let token = p
                .request
                .window_token
                .clone()
                .or_else(|| {
                    self.state
                        .timeline_state
                        .unique_attached_window_for_channel(p.request.channel.as_str())
                        .map(|(token, _)| token)
                })
                .unwrap_or_else(|| format!("timeline:{}", p.request.channel.as_str()));
            let mut request =
                crate::timeline_state::TimelineWindowRequest::latest_with_window_token(
                    p.request.channel.as_str(),
                    token,
                );
            request.page_size = p.request.size as u32;
            request.page.has_older = result["hasOlder"] == true;
            request.page.has_newer = result["hasNewer"] == true;
            request.page.has_more = request.page.has_older;
            if p.request.operation == "locate" {
                request.target_message_id = Some(p.request.anchor.clone());
            }
            let rows = result["messages"]
                .as_array()
                .ok_or_else(|| error("INVALID_ROWS"))?;
            self.state
                .timeline_state
                .replace(request, rows)
                .map_err(|_| error("INVALID_ROWS"))?;
            self.observe_rows_create_at(p.request.channel, rows);
        }
        self.render_scope.authorize_read(
            &p.request.req_id,
            [p.request.channel.as_str().to_owned()]
                .into_iter()
                .collect(),
        );
        out.push(Effect::CancelTimer { id: p.timer });
        out.push(crate::read_relay::emit_read_body(&p.request.req_id, result));
        Ok(())
    }

    /// The dispatcher has already removed this correlation, so duplicate/late replies are inert.
    pub(crate) fn sequence_reply(
        &mut self,
        mut p: Box<Pending>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        if let Err(reason) = self.sequence_reply_inner(&mut p, outcome, out) {
            self.sequence_fail(&p, &reason.to_string(), out);
        }
    }

    fn sequence_reply_inner(
        &mut self,
        p: &mut Box<Pending>,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        if p.query_session_epoch != self.state.query_session_epoch {
            return Err(error("SESSION_CHANGED"));
        }
        if self
            .state
            .channels
            .get(&p.request.channel)
            .is_some_and(|channel| channel.is_terminal())
        {
            return Err(error("ACCESS_REVOKED"));
        }
        if p.meta.as_ref().is_some_and(|meta| {
            self.state
                .sequence_authority_versions
                .get(&p.request.channel)
                .is_some_and(|version| *version != (meta.epoch, meta.revision))
        }) {
            return Err(error("SNAPSHOT_CHANGED"));
        }
        let PortOutcome::Ok(reply) = outcome else {
            return Err(error("PORT_FAILED"));
        };
        match p.stage {
            Stage::Metadata | Stage::Range(_, _) => {
                let envelope: Value =
                    serde_json::from_slice(&reply.0).map_err(|_| error("INVALID_ENVELOPE"))?;
                if matches!(p.stage, Stage::Metadata) && envelope["status"].as_u64() == Some(404) {
                    if p.request.continuation.is_some() {
                        return Err(error("CURSOR_UNSUPPORTED"));
                    }
                    out.push(Effect::CancelTimer { id: p.timer });
                    return self.handle_query_command(
                        &p.request.name,
                        p.request.legacy.to_string().as_bytes(),
                        out,
                    );
                }
                let raw = crate::http_envelope::unwrap_transport_envelope(&reply.0)?;
                let body: Value =
                    serde_json::from_slice(&raw).map_err(|_| error("INVALID_JSON"))?;
                // The old Go router reports only this exact route-missing envelope with HTTP 200.
                if matches!(p.stage, Stage::Metadata)
                    && body["status"] == "failed"
                    && body["message"] == "not found"
                    && body["data"].is_null()
                {
                    out.push(Effect::CancelTimer { id: p.timer });
                    if p.request.continuation.is_some() {
                        return Err(error("CURSOR_UNSUPPORTED"));
                    }
                    return self.handle_query_command(
                        &p.request.name,
                        p.request.legacy.to_string().as_bytes(),
                        out,
                    );
                }
                if body["status"] != "SUCCESS" {
                    if body["data"]["code"] == "SEQUENCE_SNAPSHOT_CHANGED"
                        && p.restarts < 2
                        && p.http < MAX_HTTP
                    {
                        p.restarts += 1;
                        p.meta = None;
                        p.rows.clear();
                        p.slots.clear();
                        p.bodies.clear();
                        p.accepted_lo = None;
                        p.accepted_hi = None;
                        p.scanned = 0;
                        p.locate_pass = LocatePass::Before;
                        self.sequence_http(p.clone(), None, out);
                        return Ok(());
                    }
                    return Err(error("AUTHORITY_REJECTED"));
                }
                let data = &body["data"];
                if matches!(p.stage, Stage::Metadata) {
                    if data["supported"] == false
                        || (data["supported"] == true && data["ready"] == false)
                    {
                        out.push(Effect::CancelTimer { id: p.timer });
                        if p.request.continuation.is_some() {
                            return Err(error("CURSOR_UNSUPPORTED"));
                        }
                        return self.handle_query_command(
                            &p.request.name,
                            p.request.legacy.to_string().as_bytes(),
                            out,
                        );
                    }
                    if data["supported"] != true || data["ready"] != true {
                        return Err(error("INVALID_CAPABILITY"));
                    }
                    let meta = data::metadata(data)?;
                    if self
                        .state
                        .sequence_authority_versions
                        .get(&p.request.channel)
                        .is_some_and(|(epoch, revision)| {
                            meta.epoch < *epoch || meta.revision < *revision
                        })
                    {
                        return Err(error("SNAPSHOT_CHANGED"));
                    }
                    self.state
                        .sequence_authority_versions
                        .insert(p.request.channel, (meta.epoch, meta.revision));
                    if let Some(cursor) = &p.request.continuation {
                        if cursor.epoch != meta.epoch
                            || cursor.revision != meta.revision
                            || cursor.head != meta.head
                            || cursor.next > meta.head.saturating_add(1)
                        {
                            return Err(error("CURSOR_EXPIRED"));
                        }
                        p.direction = cursor.direction.clone();
                        p.next = cursor.next;
                    } else {
                        let anchor = meta.anchor;
                        match p.request.operation {
                            "latest" => {
                                p.direction = Direction::Older;
                                p.next = meta.head;
                            }
                            "older" => {
                                p.direction = Direction::Older;
                                p.next = anchor.ok_or_else(|| error("ANCHOR_UNAVAILABLE"))? - 1;
                            }
                            "newer" => {
                                p.direction = Direction::Newer;
                                p.next = anchor
                                    .ok_or_else(|| error("ANCHOR_UNAVAILABLE"))?
                                    .saturating_add(1);
                            }
                            _ if p.request.forward => {
                                p.direction = Direction::Newer;
                                p.next = anchor.ok_or_else(|| error("ANCHOR_UNAVAILABLE"))?;
                            }
                            _ => {
                                p.direction = Direction::Older;
                                p.next = anchor.ok_or_else(|| error("ANCHOR_UNAVAILABLE"))? - 1;
                            }
                        }
                    }
                    p.meta = Some(meta);
                    self.sequence_next(p.clone(), out)?;
                } else if let Stage::Range(lo, hi) = p.stage {
                    let ops = data::range_ops(p, data, lo, hi, &self.config.auth_user_id)?;
                    self.sequence_storage(p.clone(), Stage::Write, ops, true, out);
                }
            }
            Stage::Slots | Stage::ReadbackSlots => {
                data::ingest_slots(p, &reply.0)?;
                let op = data::body_scan(p);
                self.sequence_storage(p.clone(), Stage::Bodies, vec![op], false, out);
            }
            Stage::Bodies => {
                data::ingest_bodies(p, &reply.0)?;
                if let Some(range) = data::missing(p) {
                    if p.http >= MAX_HTTP {
                        return self.sequence_budget(p, out);
                    }
                    self.sequence_http(p.clone(), Some(range), out);
                } else {
                    self.sequence_accept(p)?;
                    self.sequence_next(p.clone(), out)?;
                }
            }
            Stage::Write => {
                let op = data::slot_scan(p);
                self.sequence_storage(p.clone(), Stage::ReadbackSlots, vec![op], false, out);
            }
        }
        Ok(())
    }

    fn sequence_budget(&mut self, p: &Pending, out: &mut EffectSink) -> Result<(), ImError> {
        if p.accepted_lo.is_none() {
            return Err(error("SCAN_BUDGET_RETRY_REQUIRED"));
        }
        self.sequence_finish(p, out)
    }

    fn sequence_accept(&self, p: &mut Pending) -> Result<(), ImError> {
        let center_older = p.request.operation == "locate"
            && !p.request.forward
            && p.locate_pass == LocatePass::Before;
        let goal = if center_older {
            p.request.size / 2
        } else {
            p.request.size
        };
        let sequence: Box<dyn Iterator<Item = i64>> = if p.direction == Direction::Older {
            Box::new((p.lo..=p.hi).rev())
        } else {
            Box::new(p.lo..=p.hi)
        };
        for seq in sequence {
            if p.rows.len() >= goal {
                break;
            }
            let slot = p.slots.get(&seq).ok_or_else(|| error("INCOMPLETE_RANGE"))?;
            if let Some(key) = &slot.key {
                let row = p.bodies.get(key).ok_or_else(|| error("MISSING_BODY"))?;
                let fields = crate::ws::parser::extract_post_fields(row);
                // Revocation keeps a cleared, visible shell; only viewer permissions exclude it.
                if crate::channel_write::post_updates_from_fields(
                    p.request.channel,
                    &fields,
                    &self.config.auth_user_id,
                )
                .visible
                {
                    p.rows.insert(slot.seq, row.clone());
                }
            }
            p.accepted_lo = Some(p.accepted_lo.map_or(seq, |old| old.min(seq)));
            p.accepted_hi = Some(p.accepted_hi.map_or(seq, |old| old.max(seq)));
            p.scanned += 1;
            p.next = if p.direction == Direction::Older {
                seq - 1
            } else {
                seq.saturating_add(1)
            };
        }
        Ok(())
    }

    fn sequence_next(&mut self, mut p: Box<Pending>, out: &mut EffectSink) -> Result<(), ImError> {
        let meta = p.meta.as_ref().ok_or_else(|| error("MISSING_METADATA"))?;
        let center = p.request.operation == "locate" && !p.request.forward;
        let center_older = center && p.locate_pass == LocatePass::Before;
        if center_older && (p.rows.len() >= p.request.size / 2 || p.next < meta.first) {
            p.locate_pass = LocatePass::After;
            p.direction = Direction::Newer;
            p.next = meta.anchor.ok_or_else(|| error("ANCHOR_UNAVAILABLE"))?;
        }
        let newer_end = p.direction == Direction::Newer
            && (p.accepted_hi == Some(meta.head)
                || p.next > meta.head
                || (p.accepted_hi.is_none()
                    && p.request.operation == "newer"
                    && p.request.continuation.is_none()
                    && meta.anchor == Some(meta.head)));
        if center
            && p.locate_pass == LocatePass::After
            && newer_end
            && p.rows.len() < p.request.size
        {
            p.locate_pass = LocatePass::Fill;
            p.direction = Direction::Older;
            p.next = p.accepted_lo.unwrap_or(meta.first) - 1;
        }
        if p.rows.len() >= p.request.size
            || meta.head == 0
            || p.next < meta.first
            || p.next > meta.head
            || (newer_end && p.direction == Direction::Newer)
        {
            return self.sequence_finish(&p, out);
        }
        if p.scanned >= MAX_SCANNED {
            return self.sequence_budget(&p, out);
        }
        let remaining = p.request.size.saturating_sub(p.rows.len()).max(1);
        let desired = if p.scanned == 0 {
            remaining
        } else if p.rows.is_empty() {
            RANGE_SIZE as usize
        } else {
            remaining.saturating_mul(p.scanned).div_ceil(p.rows.len())
        };
        let width = (desired as i64)
            .min(RANGE_SIZE)
            .min((MAX_SCANNED - p.scanned) as i64)
            .max(1);
        (p.lo, p.hi) = if p.direction == Direction::Older {
            ((p.next - width + 1).max(meta.first), p.next)
        } else {
            (p.next, p.next.saturating_add(width - 1).min(meta.head))
        };
        p.slots.clear();
        p.bodies.clear();
        let op = data::slot_scan(&p);
        self.sequence_storage(p, Stage::Slots, vec![op], false, out);
        Ok(())
    }

    /// Runtime stop/account switch removes every owned continuation before late transport replies.
    pub(crate) fn cancel_sequence_queries(
        &mut self,
        timer: Option<TimerId>,
        out: &mut EffectSink,
    ) -> bool {
        let mut keys = self
            .state
            .corr_map
            .iter()
            .filter_map(|(corr, context)| match context {
                CorrelationContext::SequenceQuery { pending }
                    if timer.is_none_or(|id| id == pending.timer) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        keys.sort_unstable_by_key(|corr| corr.raw());
        let found = !keys.is_empty();
        for corr in keys {
            if let Some(CorrelationContext::SequenceQuery { pending }) =
                self.state.corr_map.remove(&corr)
            {
                self.sequence_fail(
                    &pending,
                    if timer.is_some() {
                        "SEQUENCE_TIMEOUT"
                    } else {
                        "CANCELLED"
                    },
                    out,
                );
            }
        }
        found
    }
}
