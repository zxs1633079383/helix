//! 独立增量迁移：不得拼入旧 IM_SCHEMA 指纹而重放已有正文清洗。

/// 宿主在原库可用后单独事务执行；失败只关闭可选的 seq 查询。
pub const IM_TIMELINE_SEQ_SCHEMA_VERSION: u32 = 1;

/// 每个序号都保留一个 viewer-local slot；不可见项不保存身份或正文。
pub const IM_TIMELINE_SEQ_SCHEMA: &[&str] = &["CREATE TABLE IF NOT EXISTS message_sequence_slot (\
        slot_key TEXT PRIMARY KEY, channel_id TEXT NOT NULL, seq INTEGER NOT NULL, \
        is_visible INTEGER NOT NULL CHECK(is_visible IN (0,1)), temporary_id TEXT, \
        membership_epoch INTEGER NOT NULL, revision INTEGER NOT NULL, \
        body_event_seq INTEGER NOT NULL DEFAULT 0, \
        CHECK(is_visible = 1 OR temporary_id IS NULL), UNIQUE(channel_id, seq))"];
