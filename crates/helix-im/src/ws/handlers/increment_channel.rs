use helix_core::EffectSink;
use linkme::distributed_slice;

use crate::channel::Channel;
use crate::error::ImError;
use crate::state::Seq;
use crate::sync_session::IncrementChannel;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler, WS_HANDLERS};

const INCREMENT_CHANNEL_ACTION: &str = "increment_channel";

pub(crate) fn apply_increment(
    ctx: &mut ImWsContext<'_>,
    inc: &IncrementChannel,
    out: &mut EffectSink,
) {
    let channel_id = inc.channel_id;
    // 冷启动真新 channel（DB 无 cursor，on_start Scan 未载入）首次注册时，内存 cursor 种子
    // 必须是 0（= 本地未确认任何 event），不是帧的 lastEventSeq（服务端水位）。
    // 误种水位 → from_seq=cursor=max → Go no_change → 离线 backfill 永不触发，且 B2 heal 的
    // cursor<target 永假（cursor==target==水位）连带废（真源 todo#1 / B2，post.rs）。
    // lastEventSeq 只喂下面的 increment_target（gap / B2-heal stuck 检测），与 cursor 严格解耦。
    ctx.state
        .channels
        .entry(channel_id)
        .or_insert_with(|| Channel::new(channel_id, 0));

    ctx.state.increment_fetched.insert(channel_id);
    if !inc.need_sync {
        ctx.state.need_sync_skip.insert(channel_id);
    }

    // lastEventSeq → increment_target（服务端水位，算 gap / B2 heal 的 stuck 判定），
    // 与内存 cursor 严格解耦（cursor=本地确认，target=服务端水位）。
    let target = ctx
        .state
        .increment_target
        .entry(channel_id)
        .or_insert(Seq(0));
    if inc.last_event_seq > *target {
        *target = inc.last_event_seq;
    }

    // S3 path1：全量 upsert（52 列）落库。从帧 data（`inc.raw`，parser 已零信任校验）逐列计算。
    // BLOCKING-1：user_id 列写 host 注入的真身份（ctx.auth_user_id，非硬编码空串）；空串=无身份退化。
    // now_ms 由 host 经 Clock 注入（确定性）。非法 data（缺 id）→ program_full None → 跳落库（仍 emit）。
    if let Ok(data) = serde_json::from_slice::<serde_json::Value>(inc.raw.as_ref()) {
        if let Some((cols, exclude)) =
            crate::channel_write::program_full(&data, ctx.auth_user_id, ctx.now_ms)
        {
            out.push(crate::acl::to_effect::upsert_channel_full(cols, exclude));
        }

        // A1 修复：从帧四源 `members/owner/adminUsers/boss` **+ `memberChange.join`** 写
        // channel_member 表（复合 PK）。真源 `tables/channel.rs:333 collect_members_from_channel_json`
        // （四源全量）+ `apply_member_change_tx:247`（join 增量）→ 成员真值进独立 channel_member 表；
        // 读路径 assemble_channel 再从该表重建主表 members/owner/admin_users/boss JSON 列。
        // 此前 `collect_members` 只读四源 → 真 Go 增量帧成员经 memberChange.join 交付时全丢
        // （43 人群仅落稀疏 members[] 的 3 行）；现 collect_members 已合并 join（members.rs）。
        // leave 偏离（core 无 Delete 变体，不删离场成员）见 members.rs 模块头 + ROADMAP-P。
        let members = crate::channel_write::collect_members(&data);
        if let Some(eff) = crate::acl::to_effect_s1::upsert_channel_members(channel_id, members) {
            out.push(eff);
        }

        // memberChange.leave → 删离场成员（复合 PK 作用域删，scope=channel_id 不跨群）。
        let leaves = crate::channel_write::collect_member_leaves(&data);
        if let Some(eff) = crate::acl::to_effect_s1::delete_channel_members(channel_id, leaves) {
            out.push(eff);
        }
    }

    // 前端仍要 emit（透传 mentionList/urgentPostList，B-rest 契约不变）。
    out.push(crate::acl::to_effect::emit_channel_increment(
        channel_id,
        inc.raw.as_ref(),
    ));
}

struct IncrementChannelHandler;

impl WsMessageHandler for IncrementChannelHandler {
    fn action(&self) -> &'static str {
        INCREMENT_CHANNEL_ACTION
    }

    fn handle(
        &self,
        ctx: &mut ImWsContext<'_>,
        frame: &WsFrame,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let Ok(data) = frame.data_required() else {
            return Ok(());
        };
        let Some(inc) = crate::parser::parse_increment_channel(data) else {
            return Ok(());
        };

        apply_increment(ctx, &inc, out);
        Ok(())
    }
}

static INCREMENT_CHANNEL_HANDLER: IncrementChannelHandler = IncrementChannelHandler;

#[distributed_slice(WS_HANDLERS)]
static INCREMENT_CHANNEL_REGISTRATION: WsHandlerRegistration = WsHandlerRegistration {
    action: INCREMENT_CHANNEL_ACTION,
    handler: &INCREMENT_CHANNEL_HANDLER,
};
