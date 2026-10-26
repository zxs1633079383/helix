//! Viewer-local reaction targets; versions stay decimal strings across JS boundaries.
use crate::error::ImError;
use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};
use serde_json::{json, Value};

struct ReactionUnreadCommand {
    ack: bool,
}
impl OutboundCommand for ReactionUnreadCommand {
    fn name(&self) -> &'static str {
        if self.ack {
            "im_reaction_unread_ack"
        } else {
            "im_reaction_unread"
        }
    }
    fn is_read(&self) -> bool {
        true
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let allowed: &[&str] = if self.ack {
            &["channel_id", "post_id", "observed_revision", "req_id"]
        } else {
            &["channel_id", "req_id", "post_ids"]
        };
        let object = args
            .as_object()
            .ok_or_else(|| ImError::Parse("reaction request must be an object".into()))?;
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(ImError::Parse(
                "reaction request contains unknown fields".into(),
            ));
        }
        require_str(args, "req_id", self.name())?;
        let channel = require_str(args, "channel_id", self.name())?;
        if crate::state::ChannelId::from_str(channel).is_none() {
            return Err(ImError::Parse(
                "reaction request has invalid channel".into(),
            ));
        }
        let mut body = json!({"channelId":channel});
        if let Some(ids) = args.get("post_ids") {
            let ids = ids
                .as_array()
                .ok_or_else(|| ImError::Parse("post_ids must be an array".into()))?;
            let mut unique = std::collections::BTreeSet::new();
            if ids.len() > 100
                || ids.iter().any(|id| {
                    id.as_str().is_none_or(|id| {
                        id.trim().is_empty() || id != id.trim() || !unique.insert(id)
                    })
                })
            {
                return Err(ImError::Parse(
                    "post_ids must contain at most 100 unique nonempty IDs".into(),
                ));
            }
            body["postIds"] = json!(ids);
        }
        if self.ack {
            let post = require_str(args, "post_id", self.name())?;
            let revision = require_str(args, "observed_revision", self.name())?;
            if !valid_revision(revision) {
                return Err(ImError::Parse(
                    "reaction revision must be a positive decimal i64 string".into(),
                ));
            }
            body["postId"] = json!(post);
            body["observedRevision"] = json!(revision);
        }
        Ok((
            if self.ack {
                "posts/reactionUnread/ack"
            } else {
                "posts/reactionUnread"
            },
            body,
        ))
    }
}
pub(crate) fn valid_revision(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<i64>().is_ok_and(|revision| revision > 0)
}
static QUERY: ReactionUnreadCommand = ReactionUnreadCommand { ack: false };
static ACK: ReactionUnreadCommand = ReactionUnreadCommand { ack: true };
inventory::submit! { OutboundRegistration { name: "im_reaction_unread", command: &QUERY } }
inventory::submit! { OutboundRegistration { name: "im_reaction_unread_ack", command: &ACK } }
