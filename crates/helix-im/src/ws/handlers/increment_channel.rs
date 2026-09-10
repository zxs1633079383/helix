use helix_core::{Correlation, Effect, EffectSink};

use crate::channel::Channel;
use crate::error::ImError;
use crate::state::Seq;
use crate::sync_session::IncrementChannel;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const INCREMENT_CHANNEL_ACTION: &str = "increment_channel";

pub(crate) fn apply_increment(
    ctx: &mut ImWsContext<'_>,
    inc: &IncrementChannel,
    out: &mut EffectSink,
) {
    // 每个新 increment 都重新打开当前批次；global end 只封口一次，避免后续批次被旧 ready 闸门吞掉。
    ctx.state.channel_sync_batch_pending = true;
    if let Ok(data) = serde_json::from_slice::<serde_json::Value>(inc.raw.as_ref()) {
        if let Some(batch_id) = data
            .get("batchId")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
        {
            ctx.state.pending_increment_batch_id = Some(batch_id.to_string());
        }
    }
    for effect in apply_increment_effects(ctx, inc) {
        match effect {
            Effect::PersistFire { ops } => ctx.state.pending_increment_ops.extend(ops),
            other => out.push(other),
        }
    }
    ctx.state
        .pending_increment_projections
        .push((inc.channel_id, inc.raw.as_ref().to_vec()));
}

/// UC-4.5 HTTP 单频道 hydration：复用与 WS `increment_channel` 完全相同的解析后应用规则，
/// 仅把其中幂等 `PersistFire` 写合并提升为带 corr 的 `PersistAtomic`。调用方据此等待
/// channel/channel_member 同一原子屏障确认落库后再启动 sync，避免 HTTP 与本地写并发导致投影读回竞态。
pub(crate) fn apply_increment_hydration(
    ctx: &mut ImWsContext<'_>,
    inc: &IncrementChannel,
    corr: Correlation,
    out: &mut EffectSink,
) -> bool {
    let mut persist_ops = Vec::new();
    for effect in apply_increment_effects(ctx, inc) {
        match effect {
            Effect::PersistFire { ops } => persist_ops.extend(ops),
            other => out.push(other),
        }
    }
    let has_persist = !persist_ops.is_empty();
    if has_persist {
        out.push(Effect::PersistAtomic {
            corr,
            ops: persist_ops,
        });
    }
    has_persist
}

/// 将单个 increment 快照编译为频道写与当前 viewer 的单调成员态写。
fn apply_increment_effects(ctx: &mut ImWsContext<'_>, inc: &IncrementChannel) -> Vec<Effect> {
    commit_increment_state(ctx, inc);
    compile_increment_effects(ctx, inc)
}

/// HTTP 分页在 PersistOk 后才应用内存事实；旧 WS 路径保持既有接收时应用顺序。
pub(crate) fn commit_increment_state(ctx: &mut ImWsContext<'_>, inc: &IncrementChannel) {
    let channel_id = inc.channel_id;
    // 输入批次仅由apply_increment打开；单群hydration/持久化后的状态应用保持已有批次边界。
    // 冷启动真新 channel（DB 无 cursor，on_start Scan 未载入）首次注册时，内存 cursor 种子
    // 必须是 0（= 本地未确认任何 event），不是帧的 lastEventSeq（服务端水位）。
    // 误种水位 → from_seq=cursor=max → Go no_change → 离线 backfill 永不触发，且 B2 heal 的
    // cursor<target 永假（cursor==target==水位）连带废（真源 todo#1 / B2，post.rs）。
    // lastEventSeq 只喂下面的 increment_target（gap / B2-heal stuck 检测），与 cursor 严格解耦。
    ctx.state
        .channels
        .entry(channel_id)
        .or_insert_with(|| Channel::new(channel_id, 0));

    // 增量帧可能早于启动后的 channel 投影扫描到达；先用同一帧的删除标记封口，
    // 让本批 `increment_channel_end` 和 PongGap 都跳过已删除/已关闭频道。
    if is_terminal_projection(inc.raw.as_ref()) {
        if let Some(channel) = ctx.state.channels.get_mut(&channel_id) {
            channel
                .mark_projection_terminal(Seq(inc.last_event_seq.0.max(channel.cursor.value().0)));
        }
    }

    if ctx.state.increment_fetched.insert(channel_id) {
        ctx.state.increment_order.push(channel_id);
    }
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

    // UC-10：收集本群「about-me」post id（mention + urgent）累入会话缓冲——global-end 收尾时
    // build queryTodoList 拉待办内容（真源 channel.rs:142-149）。零信任：缺/坏字段 → 空，不报错。
    if let Ok(data) = serde_json::from_slice::<serde_json::Value>(inc.raw.as_ref()) {
        ctx.state
            .about_me_post_ids
            .extend(crate::todo::collect_about_me_ids(&data));
    }
}

/// 纯粹编译频道和成员写操作，不修改恢复集合或频道内存态。
pub(crate) fn compile_increment_effects(
    ctx: &mut ImWsContext<'_>,
    inc: &IncrementChannel,
) -> Vec<Effect> {
    let mut effects = Vec::with_capacity(4);
    let channel_id = inc.channel_id;
    // S3 path1：全量 upsert（52 列）落库。从帧 data（`inc.raw`，parser 已零信任校验）逐列计算。
    // BLOCKING-1：user_id 列写 host 注入的真身份（ctx.auth_user_id，非硬编码空串）；空串=无身份退化。
    // now_ms 由 host 经 Clock 注入（确定性）。非法 data（缺 id）→ program_full None → 跳落库（仍 emit）。
    if let Ok(data) = serde_json::from_slice::<serde_json::Value>(inc.raw.as_ref()) {
        if let Some((cols, exclude)) =
            crate::channel_write::program_full(&data, ctx.auth_user_id, ctx.now_ms)
        {
            effects.push(crate::acl::to_effect::upsert_channel_full(cols, exclude));
        }

        // 只有快照明确携带 unread 绝对值时，它才是当前 viewer 的成员态权威；缺字段时保留
        // 既有 member/guard，后续 HydrationHistory 再按实际 type1 事件重建，禁止用默认值猜测。
        let unread_authority = data
            .get("unreadCount")
            .or_else(|| data.get("unread_count"))
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            });
        let event_seq = i64::try_from(inc.last_event_seq.0).ok();
        if let (Some(unread_count), Some(event_seq)) = (unread_authority, event_seq) {
            if let Some((mut row, _)) = crate::channel_update::member_channel_from_update_channel(
                &data,
                channel_id,
                ctx.auth_user_id,
                ctx.now_ms,
            )
            .filter(|_| !ctx.auth_user_id.is_empty())
            {
                if let Some((_, user_id)) = row.iter_mut().find(|(column, _)| column == "user_id") {
                    *user_id = helix_core::effect::SqlValue::Text(ctx.auth_user_id.to_string());
                }
                row.push((
                    "last_unread_event_seq".to_string(),
                    helix_core::effect::SqlValue::Integer(event_seq),
                ));
                let seed =
                    helix_core::effect::StorageOp::BatchUpsert(helix_core::effect::UpsertSpec {
                        version_column: None,
                        update_guard: None,
                        table: "channel_member",
                        rows: vec![row],
                        conflict_key: Some("channel_id,user_id"),
                        exclude_from_update: vec!["unread_count", "last_unread_event_seq"],
                    });
                // `last_post_at + 0` 只充当通用 ScopedGuardedBump 的中性载体；真正写入的是
                // unread 绝对值与 per-viewer sequence guard，旧/重放快照因严格 `>` 不会回退。
                let guarded_set = helix_core::effect::StorageOp::ScopedGuardedBump(
                    helix_core::effect::ScopedGuardedBumpSpec {
                        table: "channel_member",
                        scope_col: "channel_id",
                        scope_val: helix_core::effect::SqlValue::Text(
                            channel_id.as_str().to_string(),
                        ),
                        key_col: "user_id",
                        key_val: helix_core::effect::SqlValue::Text(ctx.auth_user_id.to_string()),
                        bump_col: "last_post_at",
                        bump_delta: 0,
                        set_cols: vec![
                            (
                                "unread_count".to_string(),
                                helix_core::effect::SqlValue::Integer(unread_count),
                            ),
                            (
                                "last_unread_event_seq".to_string(),
                                helix_core::effect::SqlValue::Integer(event_seq),
                            ),
                        ],
                        guard_col: "last_unread_event_seq",
                        guard_val: event_seq,
                    },
                );
                effects.push(Effect::PersistFire {
                    ops: vec![seed, guarded_set],
                });
            }
        }

        // A1 修复：从帧四源 `members/owner/adminUsers/boss` **+ `memberChange.join`** 写
        // channel_member 表（复合 PK）。真源 `tables/channel.rs:333 collect_members_from_channel_json`
        // （四源全量）+ `apply_member_change_tx:247`（join 增量）→ 成员真值进独立 channel_member 表；
        // 读路径 assemble_channel 再从该表重建主表 members/owner/admin_users/boss JSON 列。
        // 此前 `collect_members` 只读四源 → 真 Go 增量帧成员经 memberChange.join 交付时全丢
        // （43 人群仅落稀疏 members[] 的 3 行）；现 collect_members 已合并 join（members.rs）。
        // leave 物理删已接通（core 有 BatchDelete，见下 :67 + members.rs 模块头）。
        let members = crate::channel_write::collect_members(&data);
        if let Some(eff) = crate::acl::to_effect_s1::upsert_channel_members(channel_id, members) {
            effects.push(eff);
        }

        // memberChange.leave → 删离场成员（复合 PK 作用域删，scope=channel_id 不跨群）。
        let leaves = crate::channel_write::collect_member_leaves(&data);
        if let Some(eff) = crate::acl::to_effect_s1::delete_channel_members(channel_id, leaves) {
            effects.push(eff);
        }
    }

    effects
}

/// 判断 increment 投影是否已经是删除/关闭终态，兼容 Go/SQLite 的字段形态。
fn is_terminal_projection(raw: &[u8]) -> bool {
    let Ok(data) = serde_json::from_slice::<serde_json::Value>(raw) else {
        return false;
    };
    let delete_at = data
        .get("deleteAt")
        .or_else(|| data.get("delete_at"))
        .and_then(projection_i64)
        .unwrap_or(0);
    let is_remove = data
        .get("isRemove")
        .or_else(|| data.get("is_remove"))
        .and_then(projection_bool)
        .unwrap_or(false);
    delete_at > 0 || is_remove
}

/// 读取删除标记中的整数值，拒绝浮点和任意字符串转换。
fn projection_i64(value: &serde_json::Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|v| i64::try_from(v).ok()))
}

/// 读取删除标记中的布尔值，兼容 SQLite 0/1 和 JSON 布尔。
fn projection_bool(value: &serde_json::Value) -> Option<bool> {
    value
        .as_bool()
        .or_else(|| value.as_i64().map(|v| v != 0))
        .or_else(|| {
            value.as_str().and_then(|v| match v {
                "1" | "true" | "TRUE" => Some(true),
                "0" | "false" | "FALSE" => Some(false),
                _ => None,
            })
        })
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
            tracing::warn!(
                "channel-sync-ready 未触发：increment_channel 帧缺少 data，未进入批次缓冲"
            );
            return Ok(());
        };
        let Some(inc) = crate::ws::parser::parse_increment_channel(data) else {
            tracing::warn!(
                "channel-sync-ready 未触发：increment_channel 帧解析失败，未进入批次缓冲"
            );
            return Ok(());
        };

        apply_increment(ctx, &inc, out);
        Ok(())
    }
}

static INCREMENT_CHANNEL_HANDLER: IncrementChannelHandler = IncrementChannelHandler;
#[cfg(target_arch = "wasm32")]
pub(super) fn inventory_link_anchor() {
    std::hint::black_box(&INCREMENT_CHANNEL_HANDLER);
}

inventory::submit! {
    WsHandlerRegistration {
        action: INCREMENT_CHANNEL_ACTION,
        handler: &INCREMENT_CHANNEL_HANDLER,
    }
}
