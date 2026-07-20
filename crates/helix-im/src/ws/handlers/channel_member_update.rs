//! `channel_member_update` action handler（成员变更：path1 全量 upsert + 成员表写，spec §S1/§S3）。
//!
//! 行为真源：现网 `handlers/channel.rs:602 handle_member_update` → `channel_service::process_and_save`
//! （path1 全量 upsert）→ emit `im:channel:member-updated`（带 memberChange）。
//!
//! helix：`program_full`（全 54 列）→ `upsert_channel_full`；成员经 `channel_write::collect_members`
//! 统一收集 **四源（members/adminUsers/boss/owner）+ memberChange.join** → `channel_member` 表
//! （复合 PK，`upsert_channel_members`），与 `increment_channel` / `channel_created` 三路径同口径。
//!
//! ✅ **memberChange.leave 物理删（已接通）**：现网 `apply_member_change_tx:270` 对 leave 走物理
//! `DELETE FROM channel_member`。helix-core `StorageOp` 已加 `BatchDelete` 变体；本 handler 在
//! :113-114 调 `collect_member_leaves` → `delete_channel_members` → `StorageOp::BatchDelete`
//! 物理删离场行（与 `increment_channel.rs:69` 同口径）。注：helix 成员是 **upsert（非 replace）**，
//! 离场行只能靠此显式 BatchDelete 清除，不会被后续全量帧覆盖掉——故必须走删除路径。

use helix_core::EffectSink;

use crate::error::ImError;
use crate::state::{ChannelId, Seq};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const CHANNEL_MEMBER_UPDATE_ACTION: &str = "channel_member_update";

struct ChannelMemberUpdateHandler;

impl WsMessageHandler for ChannelMemberUpdateHandler {
    fn action(&self) -> &'static str {
        CHANNEL_MEMBER_UPDATE_ACTION
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
        let Some(channel_id) = data
            .get("id")
            .and_then(serde_json::Value::as_str)
            .and_then(ChannelId::from_str)
        else {
            return Ok(());
        };

        apply_channel_full_and_members(ctx, channel_id, data, out);

        let raw = serde_json::to_vec(data).unwrap_or_default();
        out.push(crate::acl::to_effect_s1::emit_channel_member_updated(
            channel_id, &raw,
        ));

        // S7（issue #56·C013 纯渲染壳）：额外 emit render-ready `im:channel:members`——把
        // `role→admin` 业务规则 + 四源/join 合并下沉 helix，前端 `applyChannelMembers` 退纯绑定
        // （keyed upsert + leaves 删行）。冻结的 `im:channel:member-updated`{channel_id,channel}
        // 上面照旧 emit（UC-6.1 ② 契约面不变）；本投影是额外 render-ready 渲染通道。
        let members = crate::render_ready_members::members_from_rows(
            crate::channel_write::collect_members(data),
        );
        let leaves = crate::channel_write::collect_member_leaves(data);
        out.push(crate::render_ready_members::emit_channel_members(
            channel_id.as_str(),
            members,
            leaves,
            crate::render_ready_members::MemberProjectionMode::Incremental,
        ));
        if let Some(scope) = ctx.state.dialog_list_frame.scope().cloned() {
            if let Some(frame) = ctx
                .state
                .dialog_list_frame
                .patch_from_channel_update(scope, data)
            {
                out.push(frame);
            }
        }
        Ok(())
    }
}

/// S3 path1 全量 upsert + memberChange.join 成员行写（channel_created / channel_member_update
/// 共用）。channel 内存态确保已注册（与 `increment_channel` 同口径）。
pub(crate) fn apply_channel_full_and_members(
    ctx: &mut ImWsContext<'_>,
    channel_id: ChannelId,
    data: &serde_json::Value,
    out: &mut EffectSink,
) {
    crate::channel_capabilities::remember_viewer_role(
        ctx.state,
        channel_id,
        data,
        ctx.auth_user_id,
    );
    // HX-C008：内存 cursor 是「本地已确认处理的 event_seq」，**不是**服务端水位。
    // 冷启动真新 channel（DB 无 cursor → on_start Scan 未载入 → or_insert_with 触发）必须种子 0，
    // 绝不拿帧的 lastEventSeq（服务端 max）当种子——与 increment_channel.rs:34 完全对齐。
    // 已由 handle_scan_reply 载入 DB 持久 cursor 的 channel 早在 channels 里，or_insert_with 不触发。
    //
    // 此前误种子 seed=lastEventSeq 的后果（与本次回炉修复的 increment_channel 同形态 HX-C008 违反）：
    // 若某真新 channel 首帧走本 handler（channel_member_update / channel_created），cursor 被钉死到
    // max → 后续 increment_channel 的 or_insert_with no-op（已存在）→ increment_channel_end 的
    // from_seq=cursor=max → Go no_change → 离线 backfill 丢失；且 B2 heal 的 cursor<target 永假
    // （cursor==target==lastEventSeq）→ heal 连带废。
    ctx.state
        .channels
        .entry(channel_id)
        .or_insert_with(|| crate::channel::Channel::new(channel_id, 0));

    // lastEventSeq → increment_target（服务端水位，B2 heal 的 stuck 判定锚点），与内存 cursor 严格解耦
    // （cursor=本地确认，target=服务端水位）——对齐 increment_channel.rs:43-50。
    // 真 Go wire：channel_member_update / channel_created 的 data 是 incrementData.ToMap()，含 lastEventSeq
    // （见 docs full-map partials/5）；缺省 0（不早退）。
    if let Some(last_event_seq) = data.get("lastEventSeq").and_then(serde_json::Value::as_u64) {
        let target = ctx
            .state
            .increment_target
            .entry(channel_id)
            .or_insert(Seq(0));
        if Seq(last_event_seq) > *target {
            *target = Seq(last_event_seq);
        }
    }

    // path1 全 54 列 upsert（user_id 写 host 注入真身份 ctx.auth_user_id；now_ms host 经 Clock 注入）。
    if let Some((cols, exclude)) =
        crate::channel_write::program_full(data, ctx.auth_user_id, ctx.now_ms)
    {
        out.push(crate::acl::to_effect::upsert_channel_full(cols, exclude));
    }

    // 四源（members/adminUsers/boss/owner）+ memberChange.join → channel_member 行（复合 PK upsert）。
    // 与 increment_channel / channel_created 三路径同口径（`channel_write::collect_members`）。
    let members = crate::channel_write::collect_members(data);
    if let Some(eff) = crate::acl::to_effect_s1::upsert_channel_members(channel_id, members) {
        out.push(eff);
    }

    // memberChange.leave → 从 channel_member 删离场成员（复合 PK 作用域删，scope=channel_id 不跨群）。
    let leaves = crate::channel_write::collect_member_leaves(data);
    if let Some(eff) = crate::acl::to_effect_s1::delete_channel_members(channel_id, leaves) {
        out.push(eff);
    }
}

static CHANNEL_MEMBER_UPDATE_HANDLER: ChannelMemberUpdateHandler = ChannelMemberUpdateHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&CHANNEL_MEMBER_UPDATE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: CHANNEL_MEMBER_UPDATE_ACTION,
        handler: &CHANNEL_MEMBER_UPDATE_HANDLER,
    }
}
