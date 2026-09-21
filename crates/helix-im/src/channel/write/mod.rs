//! channel 表三路径写的**纯计算**层（spec §S3）。
//!
//! 行为真源 = 现网 cses-client `domain/channel_state.rs` + `repo/channel_repo.rs` +
//! `services/message_service.rs`；逐列对照见 `docs/csesapi-migration/channel-52col-mapping.md`。
//!
//! 三路径（皆同步纯函数，副作用经调用方 `out.push(Effect)`）：
//! - **path1** `program_full`（`program.rs`）：WS `increment_channel` 全量 upsert（54 列）。
//! - **path2** `collect_present`（`patch.rs`）：WS `update_channel` PATCH（仅 `Some` 字段进 SET）。
//! - **path3** `post_updates`（`post.rs`）：新消息触发 2-5 列（未读 +1 走 SQL 自增，HX-C005）。
//!
//! helix sans-IO 约束下的两处对现网的**有意偏离**（皆行为等价 / 更优，注释留证）：
//! 1. path3 未读 +1：现网应用层 `local.unread+1`（RMW）→ helix `GuardedBump`（SQL `col=col+1`，O(1)）。
//! 2. path1 守卫列（unread/unread_post_id/purpose）：现网读本地回退 → helix 用「server None 时
//!    **不进 ON CONFLICT 更新集**」表达（INSERT 取默认 / UPDATE 保留既有 = 等价"回退本地"）。
//!
//! 按 §1 单一职责拆分（src ≤300 行硬顶）：mod 持共享类型 + 白名单；三路径各一文件。

mod members;
mod patch;
mod post;
mod program;
mod reaction;
mod revoke;
mod schedule;
mod template;
mod urgent;

use helix_core::effect::SqlValue;

pub use members::{collect_member_leaves, collect_members};
pub use patch::collect_present;
pub use post::{
    message_class, message_v3_commit_ops, message_v3_member_read_op, post_updates,
    post_updates_from_fields, PostChannelUpdate,
};
pub(crate) use post::{message_v3_post_mutation_ops, PostUpsertSource};
pub use program::program_full;
pub use reaction::message_v3_reaction_op;
pub use revoke::message_v3_revoke_channel_ops;
pub(crate) use schedule::ScheduleFact;
pub use template::message_v3_template_op;
pub use urgent::message_v3_urgent_op;

/// 一列（列名 `&'static str` 编译期常量，值 owned）。
pub type ChannelCol = (&'static str, SqlValue);

/// path2/3 共用白名单（对齐现网 `channel_repo.rs:25-77 ALLOWED_COLUMNS`，防越权 / SQL 注入）。
///
/// 含义 = 「PATCH 路径允许写的列」。**不含** `member_count` / `subtopics_loaded_at`（本地维护列，
/// partial 路径受保护，与 path1 一致）。owner/admin_users/boss 在现网经此白名单但 update_partial
/// 内被 filter 转 replace_roles——helix 当前 partial 路径不写这三列（成员走独立 channel_member 表）。
pub const ALLOWED_COLUMNS: &[&str] = &[
    "display_name",
    "type",
    "team_id",
    "create_by",
    "update_by",
    "role",
    "orient",
    "purpose",
    "header",
    "root_id",
    "root_post_id",
    "notify_props",
    "mention_permission",
    "notice_permission",
    "top_permission",
    "last_event_seq",
    "urgent_current_name",
    "picture_type",
    "last_post_at",
    "delete_at",
    "is_closed",
    "created_at",
    "updated_at",
    "last_root_post_at",
    "unread_count",
    "unread_post_id",
    "mention_count",
    "urgent_count",
    "thread_count",
    "top_count",
    "topic_msg_count",
    "admin_max_count",
    "mention_count_root",
    "is_active",
    "is_top",
    "has_more",
    "has_urgent_post",
    "has_schedule_post",
    "last_post",
    "mention_list",
    "urgent_post_list",
    "source",
    "props",
    "picture",
    "target_users",
];
