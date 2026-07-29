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

use helix_core::{Effect, EffectSink};

use crate::error::ImError;
use crate::state::{ChannelId, Seq};

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const CHANNEL_MEMBER_UPDATE_ACTION: &str = "channel_member_update";

struct ChannelMemberUpdateHandler;

impl WsMessageHandler for ChannelMemberUpdateHandler {
    /// 返回 registry 使用的稳定 WS action 名。
    fn action(&self) -> &'static str {
        CHANNEL_MEMBER_UPDATE_ACTION
    }

    /// 将 member update 编译为原子持久写，回执前不发布 roster 终态。
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

        let event_seq = frame.event_seq();
        if event_seq.is_some_and(|seq| {
            ctx.state
                .committed_member_update_seqs
                .contains(&(channel_id, seq))
                || ctx
                    .state
                    .inflight_member_update_seqs
                    .contains(&(channel_id, seq))
        }) {
            return Ok(());
        }
        let ops = channel_full_and_member_ops(ctx, channel_id, data);
        if ops.is_empty() {
            return Ok(());
        }
        let raw_channel = serde_json::to_vec(data).unwrap_or_default();
        let members = crate::render_ready_members::members_from_rows(
            crate::channel_write::collect_members(data),
        );
        let leaves = crate::channel_write::collect_member_leaves(data);
        if let Some(seq) = event_seq {
            ctx.state
                .inflight_member_update_seqs
                .insert((channel_id, seq));
        }
        let corr = ctx.alloc_corr();
        ctx.state.corr_map.insert(
            corr,
            crate::state::CorrelationContext::ChannelMemberUpdatePersist {
                channel_id,
                event_seq,
                raw_channel,
                members,
                leaves,
                causation_id: event_seq.map(|seq| format!("channel-member-update:{}", seq.0)),
            },
        );
        out.push(Effect::PersistAtomic { corr, ops });
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
    let ops = channel_full_and_member_ops(ctx, channel_id, data);
    if !ops.is_empty() {
        out.push(Effect::PersistFire { ops });
    }
}

/// 为 G-15a 合并 HTTP/WS authority，单一 PersistAtomic 成功前不发布终态事件。
pub(crate) fn queue_channel_create_persist(
    ctx: &mut ImWsContext<'_>,
    channel_id: ChannelId,
    mut channel: serde_json::Value,
    causation_id: Option<String>,
    out: &mut EffectSink,
) {
    if ctx.state.committed_channel_creates.contains(&channel_id)
        || ctx.state.inflight_channel_creates.contains(&channel_id)
    {
        return;
    }
    let member_rows = crate::channel_write::collect_members(&channel)
        .into_iter()
        .map(|member| {
            serde_json::json!({
                "channel_id": channel_id.as_str(),
                "user_id": member.user_id,
                "team_id": member.team_id,
                "role": member.role,
                "nick_name": member.nick_name,
            })
        })
        .collect::<Vec<_>>();
    if let Some(object) = channel.as_object_mut() {
        object.insert(
            "memberCount".to_string(),
            serde_json::Value::from(member_rows.len() as u64),
        );
    }
    let ops = channel_full_and_member_ops(ctx, channel_id, &channel);
    if ops.is_empty() {
        return;
    }
    let corr = ctx.alloc_corr();
    ctx.state.inflight_channel_creates.insert(channel_id);
    ctx.state.corr_map.insert(
        corr,
        crate::state::CorrelationContext::ChannelCreatePersist {
            channel_id,
            channel: Box::new(channel),
            member_rows,
            causation_id,
        },
    );
    out.push(Effect::PersistAtomic { corr, ops });
}

/// 将 channel 与 roster 权威事实编译为可原子提交的 StorageOp 批次。
fn channel_full_and_member_ops(
    ctx: &mut ImWsContext<'_>,
    channel_id: ChannelId,
    data: &serde_json::Value,
) -> Vec<helix_core::effect::StorageOp> {
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
    let mut ops = Vec::new();
    if let Some((cols, exclude)) =
        crate::channel_write::program_full(data, ctx.auth_user_id, ctx.now_ms)
    {
        ops.push(crate::acl::to_effect::upsert_channel_full_op(cols, exclude));
    }

    // 四源（members/adminUsers/boss/owner）+ memberChange.join → channel_member 行（复合 PK upsert）。
    // 与 increment_channel / channel_created 三路径同口径（`channel_write::collect_members`）。
    let members = crate::channel_write::collect_members(data);
    if let Some(op) = crate::acl::to_effect_s1::upsert_channel_members_op(channel_id, members) {
        ops.push(op);
    }

    // memberChange.leave → 从 channel_member 删离场成员（复合 PK 作用域删，scope=channel_id 不跨群）。
    let leaves = crate::channel_write::collect_member_leaves(data);
    if let Some(op) = crate::acl::to_effect_s1::delete_channel_members_op(channel_id, leaves) {
        ops.push(op);
    }
    ops
}

static CHANNEL_MEMBER_UPDATE_HANDLER: ChannelMemberUpdateHandler = ChannelMemberUpdateHandler;
#[cfg(target_arch = "wasm32")]
/// 在 wasm inventory 不可自动发现时保留静态 handler。
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&CHANNEL_MEMBER_UPDATE_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: CHANNEL_MEMBER_UPDATE_ACTION,
        handler: &CHANNEL_MEMBER_UPDATE_HANDLER,
    }
}
