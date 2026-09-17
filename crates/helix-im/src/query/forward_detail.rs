//! Remote immutable snapshot pages. Cancellation releases module state, not the HTTP socket.

use crate::{state::CorrelationContext, ImError, ImModule};
use helix_core::tick::PortOutcome;
use helix_core::{Effect, EffectSink, TimerId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const MAX_PENDING: usize = 32;
const PAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub root_post_id: String,
    pub path: Vec<String>,
    pub offset: usize,
    pub req_id: String,
}

impl Request {
    pub(crate) fn parse(value: &Value) -> Result<Self, ImError> {
        let request: Self = serde_json::from_value(value.clone())
            .map_err(|_| ImError::Parse("invalid forward detail request".into()))?;
        if request.root_post_id.is_empty()
            || request.root_post_id.len() > 128
            || request.req_id.is_empty()
            || request.req_id.len() > 128
            || request.path.len() > 31
            || request
                .path
                .iter()
                .any(|key| key.is_empty() || key.len() > 128)
            || request.offset > 100
        {
            return Err(ImError::Parse("invalid forward detail bounds".into()));
        }
        Ok(request)
    }

    pub(crate) fn body(&self) -> Value {
        json!({"rootPostId":self.root_post_id,"path":self.path,"offset":self.offset})
    }
}

/// Both commands use this registry predicate at accepts and dispatch.
pub(crate) fn is_command(name: &str) -> bool {
    matches!(name, "im_forward_detail" | "im_cancel_forward_detail")
}

impl ImModule {
    pub(crate) fn forward_detail_command(
        &mut self,
        name: &str,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let value: Value = serde_json::from_slice(payload)
            .map_err(|_| ImError::Parse("invalid forward detail JSON".into()))?;
        if name == "im_cancel_forward_detail" {
            let req_id = super::render_ready::forward::text(&value, "req_id", 128)?;
            if req_id.is_empty() || value.as_object().is_none_or(|v| v.len() != 1) {
                return Err(ImError::Parse("invalid forward detail cancellation".into()));
            }
            self.cancel_forward_details(Some(req_id), "CANCELLED", out);
            return Ok(());
        }
        let result = self.start_forward_detail(&value, out);
        if let Err(error) = result {
            if let Some(req_id) = value["req_id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 128)
            {
                let active = self.state.corr_map.values().any(|ctx| {
                    matches!(ctx,
                    CorrelationContext::ForwardDetail { request, .. } if request.req_id == req_id)
                });
                if active {
                    self.cancel_forward_details(Some(req_id), &error.to_string(), out);
                } else {
                    out.push(crate::read_relay::emit_read_error(
                        req_id,
                        &error.to_string(),
                    ));
                }
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }

    fn start_forward_detail(&mut self, value: &Value, out: &mut EffectSink) -> Result<(), ImError> {
        let request = Request::parse(value)?;
        let pending = self
            .state
            .corr_map
            .values()
            .filter_map(|ctx| match ctx {
                CorrelationContext::ForwardDetail { request, .. } => Some(request),
                _ => None,
            })
            .collect::<Vec<_>>();
        if let Some(old) = pending.iter().find(|old| old.req_id == request.req_id) {
            if ***old != request {
                self.cancel_forward_details(
                    Some(&request.req_id),
                    "FORWARD_DETAIL_REQUEST_CONFLICT",
                    out,
                );
            }
            return Ok(());
        }
        if pending.len() >= MAX_PENDING {
            return Err(ImError::Parse("FORWARD_DETAIL_BUSY".into()));
        }
        let corr = self.alloc_corr_internal();
        let effects = crate::commands::handle_outbound(
            "im_forward_detail",
            value.to_string().as_bytes(),
            &self.config.api_base_url,
            &self.config.default_api_base_url,
            self.state.connection_id.as_deref(),
            corr,
        )?;
        let timer = self.alloc_timer();
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ForwardDetail {
                request: Box::new(request),
                timer,
            },
        );
        out.push(Effect::ScheduleTimer {
            id: timer,
            after_ms: self.config.send_timeout_ms,
        });
        for effect in effects {
            out.push(effect);
        }
        Ok(())
    }

    pub(crate) fn forward_detail_reply(
        &mut self,
        request: Request,
        timer: TimerId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) {
        out.push(Effect::CancelTimer { id: timer });
        let result = (|| {
            let PortOutcome::Ok(reply) = outcome else {
                return Err(ImError::Parse("FORWARD_DETAIL_HTTP_FAILED".into()));
            };
            let raw =
                crate::http_envelope::unwrap_success_envelope(&reply.0, "posts/forwardDetail")?;
            if raw.len() > PAGE_BYTES + 64 * 1024 {
                return Err(ImError::Parse("FORWARD_DETAIL_PAGE_TOO_LARGE".into()));
            }
            let body: Value = serde_json::from_slice(&raw)
                .map_err(|_| ImError::Parse("FORWARD_DETAIL_INVALID_JSON".into()))?;
            if body["status"] != "SUCCESS" {
                return Err(ImError::Parse("FORWARD_DETAIL_REJECTED".into()));
            }
            project_page(&request, &body["data"], &self.config.auth_user_id)
        })();
        match result {
            Ok(page) => out.push(crate::read_relay::emit_read_body(&request.req_id, page)),
            Err(error) => out.push(crate::read_relay::emit_read_error(
                &request.req_id,
                &error.to_string(),
            )),
        }
    }

    /// Identity change, stop and explicit cancellation share one bounded cleanup path.
    pub(crate) fn cancel_forward_details(
        &mut self,
        req_id: Option<&str>,
        reason: &str,
        out: &mut EffectSink,
    ) {
        let mut keys = self
            .state
            .corr_map
            .iter()
            .filter_map(|(corr, ctx)| match ctx {
                CorrelationContext::ForwardDetail { request, .. }
                    if req_id.is_none_or(|id| id == request.req_id) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        keys.sort_unstable_by_key(|corr| corr.raw());
        for corr in keys {
            if let Some(CorrelationContext::ForwardDetail { request, timer }) =
                self.state.corr_map.remove(&corr)
            {
                out.push(Effect::CancelTimer { id: timer });
                out.push(crate::read_relay::emit_read_error(&request.req_id, reason));
            }
        }
    }

    pub(crate) fn forward_detail_timeout(&mut self, timer: TimerId, out: &mut EffectSink) -> bool {
        let req_id = self.state.corr_map.values().find_map(|ctx| match ctx {
            CorrelationContext::ForwardDetail {
                request,
                timer: active,
            } if *active == timer => Some(request.req_id.clone()),
            _ => None,
        });
        if let Some(req_id) = req_id {
            self.cancel_forward_details(Some(&req_id), "FORWARD_DETAIL_TIMEOUT", out);
            return true;
        }
        false
    }
}

/// Reject wrong roots/paths, corrupt pagination and oversized items before projection.
fn project_page(request: &Request, body: &Value, viewer: &str) -> Result<Value, ImError> {
    let fail = || ImError::Parse("FORWARD_DETAIL_INVALID_PAGE".into());
    if body["rootPostId"] != request.root_post_id || body["path"] != json!(request.path) {
        return Err(fail());
    }
    let snapshot = super::render_ready::forward::text(body, "snapshotId", 128)?;
    let title = super::render_ready::forward::text(body, "title", 128)?;
    let count = body["itemCount"]
        .as_u64()
        .filter(|v| (1..=100).contains(v))
        .ok_or_else(fail)? as usize;
    let items = body["items"]
        .as_array()
        .filter(|v| v.len() <= 20)
        .ok_or_else(fail)?;
    let end = request.offset + items.len();
    if snapshot.is_empty()
        || end > count
        || (items.is_empty() && request.offset != count)
        || body.get("nextOffset").is_none()
        || (end == count && !body["nextOffset"].is_null())
        || (end < count && body["nextOffset"].as_u64() != Some(end as u64))
    {
        return Err(fail());
    }
    let mut bytes = 0;
    let mut keys = BTreeSet::new();
    let mut projected = Vec::with_capacity(items.len());
    for item in items {
        let len = serde_json::to_vec(item).map_err(|_| fail())?.len();
        bytes += len;
        if len > 256 * 1024
            || bytes > PAGE_BYTES
            || !keys.insert(item["itemKey"].as_str().ok_or_else(fail)?)
        {
            return Err(fail());
        }
        projected.push(super::render_ready::forward::item(snapshot, item, viewer)?);
    }
    Ok(
        json!({"rootPostId":request.root_post_id,"path":request.path,"snapshotId":snapshot,
        "title":title,"itemCount":count,"items":projected,"nextOffset":body["nextOffset"]}),
    )
}
