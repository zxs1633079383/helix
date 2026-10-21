//! 共享频道元数据以服务端 updateAt 排序；成员、消息计数与本地时间不借用此水位。
use helix_core::effect::{GuardComparison, Row, ScopedGuardedBumpSpec, SqlValue, StorageOp};

/// 同属 channel 实体快照的字段；viewer/计数/调度独立版本字段故意不包含在此集合。
pub(crate) fn is_shared(column: &str) -> bool {
    matches!(
        column,
        "display_name"
            | "type"
            | "team_id"
            | "is_active"
            | "delete_at"
            | "is_closed"
            | "props"
            | "created_at"
            | "updated_at"
            | "owner"
            | "source"
            | "target_users"
            | "picture"
            | "picture_type"
            | "orient"
            | "purpose"
            | "header"
            | "root_id"
            | "root_post_id"
            | "create_by"
            | "update_by"
            | "mention_permission"
            | "notice_permission"
            | "top_permission"
            | "admin_max_count"
    )
}

/// 只接收显式服务端时间；版本0兼容旧帧，但不能覆盖已有版本化共享属性。
pub(crate) fn revision(row: &Row) -> i64 {
    row.iter()
        .find_map(|(key, value)| {
            if key == "snapshot_updated_at" {
                if let SqlValue::Integer(v) = value {
                    return Some((*v).max(0));
                }
            }
            None
        })
        .unwrap_or_else(|| {
            row.iter()
                .find_map(|(key, value)| {
                    if key == "updated_at" {
                        if let SqlValue::Integer(v) = value {
                            return Some((*v).max(0));
                        }
                    }
                    None
                })
                .unwrap_or(0)
        })
}

/// 单条主键 UPDATE 原子比较版本与写共享字段；缺省字段不写，缺频道不创建。
pub(crate) fn shared_patch(id: SqlValue, mut patch: Row, version: i64) -> StorageOp {
    patch.push(("snapshot_updated_at".into(), SqlValue::Integer(version)));
    StorageOp::ScopedGuardedBump(ScopedGuardedBumpSpec {
        table: "channel",
        scope_col: "id",
        scope_val: id.clone(),
        key_col: "id",
        key_val: id,
        bump_col: "snapshot_updated_at",
        bump_delta: 0,
        set_cols: patch,
        guard_col: "snapshot_updated_at",
        comparison: GuardComparison::GreaterOrEqual,
        guard_val: version,
        additional_guard: None,
    })
}
