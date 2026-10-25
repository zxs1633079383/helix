//! Opt-in fixed-sequence paging. Event gates/cursors remain owned by the existing sync path.
use crate::{state::ChannelId, ImError};
use helix_core::TimerId;
use serde_json::Value;
use std::collections::BTreeMap;

mod data;
mod lifecycle;

const RANGE_SIZE: i64 = 128;
const MAX_SCANNED: usize = 2048;
const MAX_HTTP: usize = 16;
const TIMEOUT_MS: u64 = 10_000;
const MAX_PENDING: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Direction {
    Older,
    Newer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum LocatePass {
    Before,
    After,
    Fill,
}

#[derive(Clone, Debug, PartialEq)]
struct Request {
    name: String,
    legacy: Value,
    channel: ChannelId,
    req_id: String,
    size: usize,
    operation: &'static str,
    anchor: String,
    forward: bool,
    attach: bool,
    window_token: Option<String>,
    continuation: Option<Cursor>,
}

#[derive(Clone, Debug, PartialEq)]
struct Cursor {
    direction: Direction,
    next: i64,
    head: i64,
    epoch: i64,
    revision: i64,
}

#[derive(Clone, Debug, PartialEq)]
struct Metadata {
    first: i64,
    head: i64,
    epoch: i64,
    revision: i64,
    anchor: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
struct Slot {
    seq: i64,
    key: Option<String>,
    event_seq: i64,
}

#[derive(Clone, Debug, PartialEq)]
enum Stage {
    Metadata,
    Slots,
    Bodies,
    Range(i64, i64),
    Write,
    ReadbackSlots,
}

/// Owned continuation is carried by exactly one correlation until terminal/cancel/timeout.
#[derive(Clone, Debug, PartialEq)]
pub struct Pending {
    request: Request,
    timer: TimerId,
    query_session_epoch: u64,
    stage: Stage,
    meta: Option<Metadata>,
    direction: Direction,
    next: i64,
    lo: i64,
    hi: i64,
    accepted_lo: Option<i64>,
    accepted_hi: Option<i64>,
    scanned: usize,
    http: usize,
    restarts: usize,
    locate_pass: LocatePass,
    slots: BTreeMap<i64, Slot>,
    previous_keys: BTreeMap<i64, String>,
    previous_revisions: BTreeMap<i64, i64>,
    bodies: BTreeMap<String, Value>,
    rows: BTreeMap<i64, Value>,
}

impl Pending {
    pub(crate) fn req_id(&self) -> &str {
        &self.request.req_id
    }
}

/// Only an explicit opt-in can select the additive protocol; old payloads retain their routes.
pub(crate) fn enabled(name: &str, payload: &[u8]) -> bool {
    matches!(
        name,
        "im_load_older_context"
            | "im_load_newer_context"
            | "im_locate_message"
            | "im_locate_context"
            | "im_query_messages_by_channel"
    ) && serde_json::from_slice::<Value>(payload)
        .ok()
        .is_some_and(|v| {
            v.get("sequence_pagination")
                .or_else(|| v.get("sequencePagination"))
                == Some(&Value::Bool(true))
        })
}

fn error(reason: &str) -> ImError {
    ImError::Parse(format!("SEQUENCE_PAGINATION_{reason}"))
}

/// Wire int64 never passes through a JavaScript number (including cursor JSON).
fn decimal(value: &Value) -> Result<i64, ImError> {
    let s = value
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 19 && s.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| error("INVALID_SEQUENCE"))?;
    let n = s.parse::<i64>().map_err(|_| error("INVALID_SEQUENCE"))?;
    if n.to_string() != s {
        return Err(error("INVALID_SEQUENCE"));
    }
    Ok(n)
}

fn field<'a>(value: &'a Value, snake: &str, camel: &str) -> &'a Value {
    value
        .get(snake)
        .or_else(|| value.get(camel))
        .unwrap_or(&Value::Null)
}

impl Request {
    fn parse(name: &str, payload: &[u8]) -> Result<Self, ImError> {
        let mut v: Value = serde_json::from_slice(payload).map_err(|_| error("INVALID_JSON"))?;
        let channel = field(&v, "channel_id", "channelId")
            .as_str()
            .and_then(ChannelId::from_str)
            .ok_or_else(|| error("INVALID_CHANNEL"))?;
        let req_id = field(&v, "req_id", "reqId")
            .as_str()
            .filter(|v| !v.is_empty() && v.len() <= 128)
            .ok_or_else(|| error("MISSING_REQ_ID"))?
            .to_owned();
        let size = crate::timeline_state::TimelinePageSize::parse(
            v.get("page_size")
                .or_else(|| v.get("pageSize"))
                .or_else(|| v.get("limit")),
        )
        .map_err(|_| error("INVALID_PAGE_SIZE"))?
        .get() as usize;
        let operation = match name {
            "im_load_older_context" => "older",
            "im_load_newer_context" => "newer",
            "im_query_messages_by_channel" => "latest",
            _ => "locate",
        };
        let anchor = field(&v, "anchor_post_id", "anchorPostId")
            .as_str()
            .or_else(|| field(&v, "target_post_id", "targetPostId").as_str())
            .or_else(|| field(&v, "message_id", "messageId").as_str())
            .or_else(|| field(&v["anchor"], "post_id", "postId").as_str())
            .unwrap_or("")
            .to_owned();
        // Legacy tuple cursors remain usable through their explicit post anchor during rollout.
        let continuation = v
            .get("cursor")
            .filter(|v| !v.is_null())
            .filter(|cursor| {
                !(cursor.get("version").is_none()
                    && field(cursor, "create_at", "createAt")
                        .as_i64()
                        .is_some_and(|n| n > 0)
                    && field(cursor, "temporary_id", "temporaryId")
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                    && !anchor.is_empty())
            })
            .map(|cursor| {
                if cursor["version"] != 2
                    || field(cursor, "channel_id", "channelId") != channel.as_str()
                {
                    return Err(error("INVALID_CURSOR"));
                }
                let direction = match cursor["direction"].as_str() {
                    Some("older") if operation == "older" => Direction::Older,
                    Some("newer") if operation == "newer" => Direction::Newer,
                    _ => return Err(error("INVALID_CURSOR")),
                };
                Ok(Cursor {
                    direction,
                    next: decimal(field(cursor, "next_seq", "nextSeq"))?,
                    head: decimal(field(cursor, "head_seq", "headSeq"))?,
                    epoch: decimal(field(cursor, "membership_epoch", "membershipEpoch"))?,
                    revision: decimal(&cursor["revision"])?,
                })
            })
            .transpose()?;
        if operation != "latest" && anchor.is_empty() && continuation.is_none() {
            return Err(error("MISSING_ANCHOR"));
        }
        if anchor.len() > 128 {
            return Err(error("INVALID_ANCHOR"));
        }
        let forward = match v.get("window").and_then(Value::as_str) {
            None | Some("center") => false,
            Some("forward") => true,
            _ => return Err(error("INVALID_WINDOW")),
        };
        let attach = match v.get("attach") {
            None => true,
            Some(Value::Bool(false)) if operation == "locate" => false,
            _ => return Err(error("INVALID_ATTACH")),
        };
        let window_token = match field(&v, "window_token", "windowToken") {
            Value::Null => None,
            Value::String(token) if !token.is_empty() && token.len() <= 256 => Some(token.clone()),
            _ => return Err(error("INVALID_WINDOW_TOKEN")),
        };
        let object = v.as_object_mut().ok_or_else(|| error("INVALID_JSON"))?;
        for key in [
            "sequence_pagination",
            "sequencePagination",
            "cursor",
            "window",
            "attach",
        ] {
            object.remove(key);
        }
        if operation == "latest" {
            object.insert("limit".into(), Value::from(size));
        } else {
            for key in ["window_token", "windowToken", "limit"] {
                object.remove(key);
            }
            object.insert("page_size".into(), Value::from(size));
        }
        Ok(Self {
            name: name.to_owned(),
            legacy: v,
            channel,
            req_id,
            size,
            operation,
            anchor,
            forward,
            attach,
            window_token,
            continuation,
        })
    }
}
