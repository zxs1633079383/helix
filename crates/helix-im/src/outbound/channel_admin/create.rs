//! channel 建群族 outbound 命令（P3b，parallelGroup=G1）：createSpecifyOwner。
//!
//! `channel/create`（owner=session）已在 `channel_existing.rs`（P2 迁移）；本文件补 P3b 漏项
//! `channel/createSpecifyOwner`（指定 owner，#8）——两者共用 Go `CreateCsesChannel`，仅 owner
//! 解析不同（create=session / createSpecifyOwner=param.Owner，channel.go:69/283）。
//!
//! ## endpoint / body 真源（mattermost csesapi 逐字，不自造 fixture，C5）
//! - endpoint `channel/createSpecifyOwner`（api.go BaseRoutes.Channel = `/api/cses/channel`）。
//! - body = `entity.CreateChannelSpecifyOwner`（内嵌 `*ent.Channel` + `users[]` + `forceCreate` +
//!   `owner`，create_channel.go:20-25）。字段繁多（含 picture/source/props map）且
//!   **`direct_key` 是 snake_case**（ent.Channel json tag 异常，#7-8 迁移注意点 3）——由前端直供
//!   raw body 透传（helix-im 只认领 endpoint + 校验最小必填），透传保真 → snake_case `direct_key`
//!   不被改写为 camelCase（与 `channel/create` 同策略，避免 helix 重拼复杂嵌套结构丢字段）。

use serde_json::Value;

use crate::error::ImError;

use crate::outbound::registry::{require_str, OutboundCommand, OutboundRegistration};

/// #8 POST /api/cses/channel/createSpecifyOwner — 指定 owner 建群 →
/// channel_created + channel_member_update + post（系统加群消息）。
///
/// 真源 `CreateChannelSpecifyOwner`（内嵌 ent.Channel + Users[]ChannelMemberMinimal + Owner）。
/// 字段多 + 嵌套（含 picture/source/props map + **snake_case `direct_key`**），由前端直供 raw body
/// 透传；至少校验 `displayName` 非空（Go Validate：Channel 非 nil + Users ≥1，displayName 是建群
/// 最小可识别字段，边界零信任拦空发）。
struct CreateSpecifyOwnerCommand;
impl OutboundCommand for CreateSpecifyOwnerCommand {
    fn name(&self) -> &'static str {
        "im_create_specify_owner"
    }
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
        // 最小必填校验（边界零信任，避免空发被服务端拒）：displayName 非空。
        // 其余字段（users/owner/picture/direct_key/forceCreate）由前端直供，透传保真。
        require_str(args, "displayName", self.name())?;
        Ok(("channel/createSpecifyOwner", args.clone()))
    }
}
static CREATE_SPECIFY_OWNER: CreateSpecifyOwnerCommand = CreateSpecifyOwnerCommand;
inventory::submit! {
    OutboundRegistration {
        name: "im_create_specify_owner",
        command: &CREATE_SPECIFY_OWNER,
    }
}
