//! A3/CAP-9 sync 未读落 DB 工厂（to_effect.rs 存量债外提 sibling，守「只减不增」基线）。
//!
//! 与 `to_effect.rs` 分文件（§1 单文件 ≤300 行硬顶；to_effect.rs 已是存量超限文件「只减不增」）。
//! 职责同 ACL-1：把 IM 概念翻译成 core 通用 StorageOp——不含 I/O、不认识 tokio/sqlx。

use crate::acl::to_effect::bump_channel_unread_op;
use crate::sync_session::EventEnvelope;
use helix_core::effect::StorageOp;

/// A3/CAP-9：离线 sync 应用的可见 type1 新消息 → channel 未读 +1 **落 DB**（不只 emit）。
///
/// 真源 post.rs:1234 离线 sync apply 显式调 `update_channel_from_post`（内含 sender 豁免 +
/// 可见性门控）把 `unread_count` +1 落库。phantom（msg_id ∉ messages）= 不可见内容 → 不 bump。
/// 复杂度：每事件 O(1)（HashMap 查 + 谓词 + 单 GuardedBump 构造）。
pub fn sync_unread_bump_ops(
    events: &[EventEnvelope],
    messages: &std::collections::HashMap<String, crate::sync_session::PostFields>,
    auth_user_id: &str,
) -> Vec<StorageOp> {
    use crate::sync_session::EventKind;

    let mut ops = Vec::new();
    for ev in events {
        if !matches!(ev.kind, EventKind::PostUpsert) {
            continue;
        }
        // 仅可见 type1（命中 messages map）才参与未读决策——与落库 / emit 同 phantom 口径。
        let Some(fields) = ev.msg_id.as_deref().and_then(|id| messages.get(id)) else {
            continue;
        };
        let upd =
            crate::channel_write::post_updates_from_fields(ev.channel_id, fields, auth_user_id);
        // delta==0（自己 / 不可见）仍可能写 lastPost 组；GuardedBump 守卫幂等，安全并入批。
        ops.push(bump_channel_unread_op(&upd));
    }
    ops
}
