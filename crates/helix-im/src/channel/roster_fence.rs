//! 成员角色的独立持久水位；不借用消息游标、未读或共享频道属性版本。
use crate::state::ChannelId;
use helix_core::effect::{SqlValue, StorageOp, UpsertGuard, UpsertSpec};

/// 只接收服务端明确给出的频道版本；旧协议为0，不能覆盖已版本化的成员事实。
pub(crate) fn revision(data: &serde_json::Value) -> i64 {
    data.get("updateAt")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
        .max(0)
}

/// 频道标量PATCH不获得成员写权限，也不推进成员水位。
pub(crate) fn has_roster(data: &serde_json::Value) -> bool {
    ["members", "memberChange", "adminUsers", "boss", "owner"]
        .iter()
        .any(|key| data.get(*key).is_some())
}

/// 版本冲突必须使整批事务回滚；主键guard不改变受众，仅启用已有driver的冲突错误合同。
pub(crate) fn write(channel_id: ChannelId, revision: i64) -> StorageOp {
    StorageOp::BatchUpsert(UpsertSpec {
        table: "channel_member_roster_fence",
        rows: vec![vec![
            (
                "channel_id".into(),
                SqlValue::Text(channel_id.as_str().into()),
            ),
            ("revision".into(), SqlValue::Integer(revision)),
        ]],
        conflict_key: Some("channel_id"),
        exclude_from_update: vec![],
        update_guard: Some(UpsertGuard {
            column: "channel_id",
            expected: SqlValue::Text(channel_id.as_str().into()),
        }),
        version_column: Some("revision"),
    })
}
