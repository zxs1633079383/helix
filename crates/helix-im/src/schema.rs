//! IM 业务 schema —— DDL 当数据，host-cli `execute_raw` 逐条幂等执行。
//!
//! ## 为什么 DDL 在 helix-im 而不在 helix-core
//!
//! helix-core 永远 schema-agnostic（HX-C001）：业务表名（message/channel/...）
//! 绝不进 core。落 DDL 的唯一干净接缝 = helix-im 把 DDL 当**数据**暴露为
//! `&[&'static str]`，host-cli 启动 migrate 后 loop `NativeStorage::execute_raw(sql)`
//! 叠加建表。
//!
//! ## 幂等 & 'static 约束
//!
//! - 每条都与现网 678 真库
//!   `~/Library/Application Support/com.jinqidongli.cses.dev/data/678/tenant.db`
//!   的 `.schema` 字符级幂等对齐（`CREATE TABLE IF NOT EXISTS` /
//!   `CREATE INDEX IF NOT EXISTS`）—— M2 接 cses-client 既有 tenant.db 时不冲突。
//! - `execute_raw` 签名是 `&'static str`，故每个数组元素都是 string literal
//!   （'static 生命周期天然满足，无堆分配）。

/// IM 业务 schema（DDL 当数据）。host-cli 启动 migrate 后 loop execute_raw 叠加。
///
/// 与现网 678 `~/Library/Application Support/com.jinqidongli.cses.dev/data/678/tenant.db`
/// 的 `.schema` 幂等对齐 —— M2 接 cses-client 既有 tenant.db 时 IF NOT EXISTS 不冲突。
///
/// **678 对齐的有意例外（ADR-013）**：`message.segment_id` 列 + `idx_message_segment_id`
/// 索引**不再镜像** —— bitmap segment 增量同步是现网 cses-client 的离线 backfill 机制，
/// helix sync-v2 已由 `event_seq` cursor 取代（ADR-008），segment_id 在 helix 从无写路径
/// （恒空）。M2 共享既有 678 tenant.db 时该列仍由 cses-client 持有、IF NOT EXISTS 跳过、
/// helix 永不触碰，故移除零功能影响；fresh helix-only 库则不再建该列与无用索引。
pub const IM_SCHEMA: &[&'static str] = &[
    // ── message（发送路径权威表；PK = temporary_id，对齐 678，除 segment_id 见上注）──
    "CREATE TABLE IF NOT EXISTS message (\
        temporary_id TEXT PRIMARY KEY,\
        id TEXT NOT NULL DEFAULT '',\
        channel_id TEXT NOT NULL DEFAULT '',\
        user_id TEXT NOT NULL DEFAULT '',\
        type TEXT NOT NULL DEFAULT '',\
        message TEXT NOT NULL DEFAULT '',\
        props TEXT NOT NULL DEFAULT '',\
        viewers TEXT NOT NULL DEFAULT '',\
        user_snapshot TEXT NOT NULL DEFAULT '',\
        mentions TEXT NOT NULL DEFAULT '',\
        replied_message TEXT NOT NULL DEFAULT '',\
        reply_messages TEXT NOT NULL DEFAULT '',\
        reply_id TEXT NOT NULL DEFAULT '',\
        reply_root_id TEXT NOT NULL DEFAULT '',\
        reply_first_level_id TEXT NOT NULL DEFAULT '',\
        expedite_map TEXT NOT NULL DEFAULT '',\
        topic TEXT NOT NULL DEFAULT '',\
        quick_reply TEXT NOT NULL DEFAULT '',\
        simple_message TEXT NOT NULL DEFAULT '',\
        read_bits TEXT NOT NULL DEFAULT '',\
        send_status TEXT NOT NULL DEFAULT '',\
        revoke INTEGER NOT NULL DEFAULT 0,\
        create_at INTEGER NOT NULL DEFAULT 0,\
        update_at INTEGER NOT NULL DEFAULT 0\
    )",
    "CREATE INDEX IF NOT EXISTS idx_message_id ON message (id)",
    "CREATE INDEX IF NOT EXISTS idx_message_channel_id ON message (channel_id)",
    // ── channel（M2 共享 tenant.db 必须存在；本轮 helix-im 无写路径，仅建表占位对齐）──
    // 注：channel 表 54 列，本轮 helix-im 不写它（OUT），但建表使 M2 接 cses-client 不缺表。
    // 完整 54 列与 678 对齐（此处全量落地）。
    "CREATE TABLE IF NOT EXISTS channel (\
        id TEXT PRIMARY KEY,\
        display_name TEXT NOT NULL DEFAULT '',\
        type TEXT NOT NULL DEFAULT '',\
        user_id TEXT NOT NULL DEFAULT '',\
        team_id TEXT NOT NULL DEFAULT '',\
        is_active INTEGER NOT NULL DEFAULT 0,\
        is_remove INTEGER NOT NULL DEFAULT 0,\
        is_top INTEGER NOT NULL DEFAULT 0,\
        unread_count INTEGER NOT NULL DEFAULT 0,\
        unread_post_id TEXT NOT NULL DEFAULT '',\
        mention_count INTEGER NOT NULL DEFAULT 0,\
        mention_list TEXT NOT NULL DEFAULT '',\
        mention_user TEXT NOT NULL DEFAULT '',\
        urgent_count INTEGER NOT NULL DEFAULT 0,\
        urgent_post_list TEXT NOT NULL DEFAULT '',\
        last_post TEXT NOT NULL DEFAULT '',\
        last_post_at INTEGER NOT NULL DEFAULT 0,\
        delete_at INTEGER NOT NULL DEFAULT 0,\
        props TEXT NOT NULL DEFAULT '',\
        created_at INTEGER NOT NULL DEFAULT 0,\
        updated_at INTEGER NOT NULL DEFAULT 0,\
        members TEXT NOT NULL DEFAULT '',\
        owner TEXT NOT NULL DEFAULT '',\
        admin_users TEXT NOT NULL DEFAULT '',\
        boss TEXT NOT NULL DEFAULT '',\
        source TEXT NOT NULL DEFAULT '',\
        target_users TEXT NOT NULL DEFAULT '',\
        picture TEXT NOT NULL DEFAULT '',\
        picture_type TEXT NOT NULL DEFAULT '',\
        orient TEXT NOT NULL DEFAULT '',\
        purpose TEXT NOT NULL DEFAULT '',\
        draft TEXT NOT NULL DEFAULT '',\
        role TEXT NOT NULL DEFAULT '',\
        root_id TEXT NOT NULL DEFAULT '',\
        root_post_id TEXT NOT NULL DEFAULT '',\
        notify_props TEXT NOT NULL DEFAULT '',\
        mention_permission TEXT NOT NULL DEFAULT '',\
        notice_permission TEXT NOT NULL DEFAULT '',\
        top_permission TEXT NOT NULL DEFAULT '',\
        last_event_seq INTEGER NOT NULL DEFAULT 0,\
        thread_count INTEGER NOT NULL DEFAULT 0,\
        top_count INTEGER NOT NULL DEFAULT 0,\
        topic_msg_count INTEGER NOT NULL DEFAULT 0,\
        admin_max_count INTEGER NOT NULL DEFAULT 0,\
        mention_count_root INTEGER NOT NULL DEFAULT 0,\
        has_more INTEGER NOT NULL DEFAULT 0,\
        has_urgent_post INTEGER NOT NULL DEFAULT 0,\
        has_schedule_post INTEGER NOT NULL DEFAULT 0,\
        create_by TEXT NOT NULL DEFAULT '',\
        update_by TEXT NOT NULL DEFAULT '',\
        last_root_post_at INTEGER NOT NULL DEFAULT 0,\
        urgent_current_name TEXT NOT NULL DEFAULT '',\
        member_count INTEGER NOT NULL DEFAULT 0,\
        subtopics_loaded_at INTEGER NOT NULL DEFAULT 0\
    )",
    // ── channel_member（复合 PK，对齐 678）────────────────────────────────
    "CREATE TABLE IF NOT EXISTS channel_member (\
        channel_id TEXT NOT NULL,\
        user_id TEXT NOT NULL,\
        team_id TEXT,\
        user_name TEXT,\
        role TEXT NOT NULL DEFAULT 'MEMBER',\
        nick_name TEXT,\
        joined_at INTEGER NOT NULL DEFAULT 0,\
        updated_at INTEGER NOT NULL DEFAULT 0,\
        PRIMARY KEY (channel_id, user_id)\
    )",
    "CREATE INDEX IF NOT EXISTS idx_channel_member_user ON channel_member(user_id)",
    "CREATE INDEX IF NOT EXISTS idx_channel_member_role ON channel_member(channel_id, role)",
    // ── channel_event_cursor（增量同步游标，S2）─────────────────────────────────
    // 全量替换要求 byte 对齐现网 678 读路径 → IM cursor 写 `channel_event_cursor`
    // （三列 channel_id PK / last_event_seq / updated_at），与现网
    // `tables/channel_event_cursor.rs:28-37 migrate` 字符级幂等（IF NOT EXISTS）。
    // 写路径：`acl/to_effect::advance_cursor` 走 `MonotonicUpsert{table=channel_event_cursor,
    //   key_col=channel_id, value_col=last_event_seq, touch_col=updated_at}`——表名/列名作
    //   数据传给 core（守 HX-C001，core 不解释）；MAX guard 由 core MonotonicUpsert 语义保证
    //   （value 严格 > 才前进，cursor 不回退）；updated_at 由 driver ambient 时钟写穿。
    // 读路径：on_start 吐 `Scan(channel_event_cursor)`，handle_scan_reply 解析
    //   channel_id（TEXT 26 字符）+ last_event_seq（INTEGER）重建内存 cursor。
    // cursor 单一真源仍是 helix gate（channel gate state），此表仅持久化镜像。
    // 注：core 通用单调表 `_helix_monotonic` 由 driver migrate() 自建（generic primitive，
    //   stress/conformance 测试用），与本业务 cursor 表互不影响（additive）。
    "CREATE TABLE IF NOT EXISTS channel_event_cursor (\
        channel_id TEXT NOT NULL PRIMARY KEY,\
        last_event_seq INTEGER NOT NULL DEFAULT 0,\
        updated_at INTEGER NOT NULL DEFAULT 0\
    )",
];
