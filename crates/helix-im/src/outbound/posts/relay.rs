//! Server-authoritative forwarding: only IDs cross the creation boundary.

use crate::{
    outbound::registry::{OutboundCommand, OutboundRegistration},
    ImError,
};
use serde_json::{json, Value};

struct CreatePostsCommand;
impl OutboundCommand for CreatePostsCommand {
    fn name(&self) -> &'static str {
        "im_create_posts"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        let (_, command_id, sources, targets, mode) = crate::forward::request(args)?;
        Ok((
            "posts/createPosts",
            json!({"postIds":sources,"targetChannelIds":targets,"mode":mode,"commandId":command_id}),
        ))
    }
}
static CREATE_POSTS: CreatePostsCommand = CreatePostsCommand;
inventory::submit! { OutboundRegistration { name:"im_create_posts", command:&CREATE_POSTS } }

struct ForwardDetailCommand;
impl OutboundCommand for ForwardDetailCommand {
    fn name(&self) -> &'static str {
        "im_forward_detail"
    }
    fn is_read(&self) -> bool {
        true
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        Ok((
            "posts/forwardDetail",
            crate::query::forward_detail::Request::parse(args)?.body(),
        ))
    }
}
static FORWARD_DETAIL: ForwardDetailCommand = ForwardDetailCommand;
inventory::submit! { OutboundRegistration { name:"im_forward_detail", command:&FORWARD_DETAIL } }
