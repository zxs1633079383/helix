//! Non-durable translation read lifecycle.
//!
//! The module owns request correlation and terminal events. It never persists message content,
//! mutates message projections, or sends a websocket frame.

use bytes::Bytes;
use helix_core::effect::HttpRequest;
use helix_core::tick::{PortError, PortOutcome};
use helix_core::{Correlation, Effect, EffectSink, TimerId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::ImError;
use crate::state::CorrelationContext;
use crate::ImModule;

pub(crate) const SERVICE_URL: &str = "service:translation";
pub(crate) const TRANSLATION_TIMEOUT_MS: u64 = 30_000;
pub(crate) const MAX_PENDING_TRANSLATIONS: usize = 32;
const MAX_TEXT_CHARS: usize = 4_000;
const MAX_TEXT_BYTES: usize = 16_000;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub req_id: String,
    pub channel_id: String,
    pub message_key: String,
    pub source_revision: String,
    pub from: String,
    pub to: String,
    pub text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CancelRequest {
    req_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    text: String,
    from: String,
    to: String,
}

pub(crate) fn is_command(name: &str) -> bool {
    matches!(name, "im_translate_message" | "im_cancel_translation")
}

impl Request {
    fn parse(payload: &Value) -> Result<Self, ImError> {
        let request: Self = serde_json::from_value(payload.clone())
            .map_err(|error| ImError::Parse(format!("invalid translation request: {error}")))?;
        validate_request(&request)?;
        let bytes = serde_json::to_vec(&request)
            .map_err(|error| ImError::Serialize(format!("translation request: {error}")))?;
        if bytes.len() > MAX_TEXT_BYTES {
            return Err(ImError::Parse("TRANSLATION_REQUEST_TOO_LARGE".to_string()));
        }
        Ok(request)
    }
}

fn validate_request(request: &Request) -> Result<(), ImError> {
    if request.req_id.is_empty() || request.req_id.chars().count() > 128 {
        return Err(ImError::Parse("TRANSLATION_INVALID_REQUEST_ID".into()));
    }
    for (name, value) in [
        ("channel_id", request.channel_id.as_str()),
        ("message_key", request.message_key.as_str()),
        ("source_revision", request.source_revision.as_str()),
    ] {
        if value.is_empty() || value.chars().count() > 256 {
            return Err(ImError::Parse(format!("TRANSLATION_INVALID_{name}")));
        }
    }
    if !is_supported_language(&request.from) || !is_supported_language(&request.to) {
        return Err(ImError::Parse("TRANSLATION_INVALID_LANGUAGE".into()));
    }
    if request.from == request.to {
        return Err(ImError::Parse("TRANSLATION_LANGUAGE_PAIR_UNCHANGED".into()));
    }
    if request.text.is_empty() {
        return Err(ImError::Parse("TRANSLATION_TEXT_EMPTY".into()));
    }
    if request.text.chars().count() > MAX_TEXT_CHARS || request.text.len() > MAX_TEXT_BYTES {
        return Err(ImError::Parse("TRANSLATION_TEXT_TOO_LARGE".into()));
    }
    Ok(())
}

impl ImModule {
    /// Dispatch the two public translation commands and register one bounded corr/timer pair.
    pub(crate) fn translation_command(
        &mut self,
        name: &str,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let value: Value = serde_json::from_slice(payload)
            .map_err(|error| ImError::Parse(format!("translation JSON: {error}")))?;
        if name == "im_cancel_translation" {
            let cancel: CancelRequest = serde_json::from_value(value).map_err(|error| {
                ImError::Parse(format!("invalid translation cancellation: {error}"))
            })?;
            if cancel.req_id.is_empty() || cancel.req_id.chars().count() > 128 {
                return Err(ImError::Parse("TRANSLATION_INVALID_REQUEST_ID".into()));
            }
            self.cancel_translations(Some(cancel.req_id.as_str()), true, out)?;
            return Ok(());
        }

        let request = Request::parse(&value)?;
        let pending = self.translation_contexts();
        if let Some((_, existing)) = pending
            .iter()
            .find(|(_, existing)| existing.req_id == request.req_id)
        {
            if **existing == request {
                // A repeated request with the same req_id and body is idempotent while pending.
                return Ok(());
            }
            self.emit_terminal(
                &request,
                "error",
                None,
                Some("TRANSLATION_REQUEST_CONFLICT"),
                out,
            )?;
            return Ok(());
        }
        if pending.len() >= MAX_PENDING_TRANSLATIONS {
            self.emit_terminal(&request, "error", None, Some("TRANSLATION_BUSY"), out)?;
            return Ok(());
        }

        let body = serde_json::to_vec(&request)
            .map_err(|error| ImError::Serialize(format!("translation request: {error}")))?;
        let corr = self.alloc_corr_internal();
        let timer = self.alloc_timer();
        self.state.corr_map.insert(
            corr,
            CorrelationContext::Translation {
                request: Box::new(request),
                timer,
            },
        );
        out.push(Effect::ScheduleTimer {
            id: timer,
            after_ms: TRANSLATION_TIMEOUT_MS,
        });
        out.push(Effect::Http {
            corr,
            req: HttpRequest {
                method: "POST".to_string(),
                url: SERVICE_URL.to_string(),
                headers: vec![("content-type".to_string(), "application/json".to_string())],
                body: Some(Bytes::from(body)),
            },
        });
        Ok(())
    }

    /// Resolve one HTTP reply into exactly one non-durable terminal event.
    pub(crate) fn translation_reply(
        &mut self,
        request: Request,
        timer: TimerId,
        outcome: &PortOutcome,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        out.push(Effect::CancelTimer { id: timer });
        match outcome {
            PortOutcome::Err(error) => {
                let error_code = error_code(error);
                self.emit_terminal(&request, "error", None, Some(error_code), out)?;
            }
            PortOutcome::Ok(reply) => {
                let result = crate::http_envelope::unwrap_transport_envelope(reply.0.as_ref())
                    .map_err(|_| "TRANSLATION_HTTP_ERROR")
                    .and_then(|raw| {
                        let response: Response = serde_json::from_slice(&raw)
                            .map_err(|_| "TRANSLATION_INVALID_RESPONSE")?;
                        validate_response(&request, &response)
                            .map_err(|_| "TRANSLATION_INVALID_RESPONSE")?;
                        Ok(response)
                    });
                match result {
                    Ok(response) => {
                        self.emit_terminal(
                            &request,
                            "success",
                            Some(response.text.as_str()),
                            None,
                            out,
                        )?;
                    }
                    Err(error_code) => {
                        self.emit_terminal(&request, "error", None, Some(error_code), out)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Expire a translation timer and remove its corr before emitting the timeout terminal.
    pub(crate) fn translation_timeout(
        &mut self,
        timer: TimerId,
        out: &mut EffectSink,
    ) -> Result<bool, ImError> {
        let corr = self.state.corr_map.iter().find_map(|(corr, context)| {
            matches!(
                context,
                CorrelationContext::Translation { timer: active, .. } if *active == timer
            )
            .then_some(*corr)
        });
        let Some(corr) = corr else {
            return Ok(false);
        };
        let Some(CorrelationContext::Translation { request, .. }) =
            self.state.corr_map.remove(&corr)
        else {
            return Ok(false);
        };
        self.emit_terminal(&request, "error", None, Some("TRANSLATION_TIMEOUT"), out)?;
        Ok(true)
    }

    /// Cancel active translations; lifecycle shutdown suppresses terminal events for old epochs.
    pub(crate) fn cancel_translations(
        &mut self,
        req_id: Option<&str>,
        emit_terminal: bool,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut correlations = self
            .state
            .corr_map
            .iter()
            .filter_map(|(corr, context)| match context {
                CorrelationContext::Translation { request, .. }
                    if req_id.is_none_or(|id| id == request.req_id) =>
                {
                    Some(*corr)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        correlations.sort_unstable_by_key(|corr| corr.raw());
        for corr in correlations {
            let Some(CorrelationContext::Translation { request, timer }) =
                self.state.corr_map.remove(&corr)
            else {
                continue;
            };
            out.push(Effect::CancelTimer { id: timer });
            if emit_terminal {
                self.emit_terminal(&request, "cancelled", None, None, out)?;
            }
        }
        Ok(())
    }

    fn translation_contexts(&self) -> Vec<(Correlation, &Request)> {
        self.state
            .corr_map
            .iter()
            .filter_map(|(corr, context)| match context {
                CorrelationContext::Translation { request, .. } => Some((*corr, request.as_ref())),
                _ => None,
            })
            .collect()
    }

    fn emit_terminal(
        &self,
        request: &Request,
        status: &str,
        translated_text: Option<&str>,
        error_code: Option<&str>,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let mut data = json!({
            "req_id": request.req_id,
            "channel_id": request.channel_id,
            "message_key": request.message_key,
            "source_revision": request.source_revision,
            "from": request.from,
            "to": request.to,
            "status": status,
        });
        let object = data
            .as_object_mut()
            .expect("translation terminal data is always an object");
        if let Some(translated_text) = translated_text {
            object.insert(
                "translated_text".into(),
                Value::String(translated_text.into()),
            );
        }
        if let Some(error_code) = error_code {
            object.insert("error_code".into(), Value::String(error_code.into()));
        }
        out.push(crate::event::MessageV3Event::new("im:translation:result", data)?.into_effect());
        Ok(())
    }
}

fn validate_response(request: &Request, response: &Response) -> Result<(), ()> {
    if response.from != request.from || response.to != request.to {
        return Err(());
    }
    if response.text.chars().count() > MAX_TEXT_CHARS || response.text.len() > MAX_TEXT_BYTES {
        return Err(());
    }
    Ok(())
}

fn is_supported_language(language: &str) -> bool {
    matches!(language, "zh" | "en")
}

fn error_code(error: &PortError) -> &'static str {
    match error {
        PortError::Timeout => "TRANSLATION_TIMEOUT",
        PortError::Network => "TRANSLATION_NETWORK_ERROR",
        PortError::Http(_) => "TRANSLATION_HTTP_ERROR",
        PortError::Other(_) => "TRANSLATION_FAILED",
        PortError::Storage(_) => "TRANSLATION_FAILED",
    }
}
