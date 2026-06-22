//! `posts/createPosts` 出站 — 批量转发消息（forward to N channels）。
//!
//! ## 行为真源（证据链，frontend-source-derived = 等价抓包）
//!
//! 现网前端 `message.service.ts::sendRelayMessages(params)` 把 `params` **逐字透传**到
//! `POST /posts/createPosts`（无任何字段重命名）。两个真实 caller 都构造 **camelCase**：
//! - `chat-content/chat-content-base.component.ts:967`：`sendRelayMessages({ posts, channelIds })`
//! - `messageContainerShared.service.ts:245`：`sendRelayMessages({ posts: [post], channelIds: [channelId] })`
//!
//! 后端 `server/channels/csesapi/posts.go::createPosts` 解构体：
//! ```go
//! var param struct {
//!     Posts      *entity.Posts   // entity.Posts = []*Post（数组）
//!     ChannelIds []string
//! }
//! ```
//! **两字段均无 json tag** → Go `encoding/json` case-insensitive 匹配，camelCase
//! `posts`/`channelIds` 与 PascalCase `Posts`/`ChannelIds` **都能解**。但**真实 wire = 前端
//! 实际构造的 camelCase**（PascalCase 只是 Go 字段名，从未上线）——故 helix 按 camelCase 拼装。
//!
//! > 标注：本结论 **frontend-source-derived 非 live 抓包**（`真机curl真源.md` 未含 createPosts
//! > 抓包）；但前端构造的 body 就是上线的真实 wire（HTTP body 逐字透传），等价抓包强度。
//!
//! ## wire 形态
//!
//! `{"posts":[<Post>...],"channelIds":[<channelId>...]}` —— `posts` 是 Post 对象数组
//! （`type Posts []*Post`），`channelIds` 是目标 channel 字符串数组。App 层
//! `CreateCsesPosts` 遍历 channelIds × posts 在每个目标 channel 建消息。
//!
//! ## 接管入参约定（C1 纯渲染：前端只传 UI 字段 + 已构造好的 post 对象数组）
//!
//! - `posts`：必填，**非空 JSON 数组**（每元素 = 待转发的 Post 对象，前端已从本地库取出）。
//! - `channel_ids`：必填，**非空字符串数组**（目标 channel）。snake_case UI param（C1），
//!   翻成 wire `channelIds`。

use linkme::distributed_slice;
use serde_json::{json, Value};

use crate::error::ImError;

use super::registry::{OutboundCommand, OutboundRegistration, OUTBOUND_COMMANDS};

/// 取必填**非空数组**字段，缺/类型错/空 → `ImError::Parse`（边界零信任，HX-C 不变量 4）。
fn require_nonempty_array<'a>(
    args: &'a Value,
    key: &str,
    cmd: &str,
) -> Result<&'a Vec<Value>, ImError> {
    args.get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空必填数组 '{key}'")))
}

/// POST /api/cses/posts/createPosts — 批量转发 → 每个目标 channel 推 `post`（逐 channel WS new_post）。
///
/// 真源：前端 `sendRelayMessages` camelCase 透传 + Go `createPosts` 无 tag case-insensitive 解构。
/// wire body camelCase：`{"posts":[...],"channelIds":[...]}`。
struct CreatePostsCommand;
impl OutboundCommand for CreatePostsCommand {
    fn name(&self) -> &'static str {
        "im_create_posts"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        // 入参 UI 字段（snake_case，C1）→ wire camelCase。两者均必填非空数组。
        let posts = require_nonempty_array(args, "posts", self.name())?;
        let channel_ids = require_nonempty_array(args, "channel_ids", self.name())?;
        Ok((
            "posts/createPosts",
            json!({ "posts": posts, "channelIds": channel_ids }),
        ))
    }
}
static CREATE_POSTS: CreatePostsCommand = CreatePostsCommand;
#[distributed_slice(OUTBOUND_COMMANDS)]
static CREATE_POSTS_REG: OutboundRegistration = OutboundRegistration {
    name: "im_create_posts",
    command: &CREATE_POSTS,
};
