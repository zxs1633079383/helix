//! `channel_write::collect_members` / `collect_member_leaves` 纯计算回归测试。
//!
//! 回归锚: A1 —— increment / channel_created / channel_member_update 三路径统一的成员收集：
//! 四源（members/adminUsers/boss/owner）+ `memberChange.join` 增量 → `channel_member` 行；
//! `memberChange.leave` 解析出离场 user_id 集，调用方接成 `StorageOp::BatchDelete`。
//!
//! 行为真源：现网 cses-client `repo/channel_member_repo.rs`
//! （`collect_members_from_channel_json:303` 四源 + `apply_member_change_tx:247` join/leave）。

use helix_im::channel_write::{collect_member_leaves, collect_members};

/// 测试辅助：按 user_id 取角色。
fn role_of<'a>(rows: &'a [helix_im::acl::to_effect_s1::MemberRow], uid: &str) -> Option<&'a str> {
    rows.iter()
        .find(|r| r.user_id == uid)
        .map(|r| r.role.as_str())
}

/// 回归锚: A1 —— 四源成员收集 + 角色映射 + dedup 升级（保留最后出现）。
#[test]
fn collect_members_maps_roles_and_dedups() {
    let mut data = serde_json::json!({
        "members": [{ "id": "u_member" }, { "id": "u_dup" }],
        "adminUsers": [{ "id": "u_admin" }],
        "boss": [{ "id": "u_boss" }],
        "owner": { "id": "u_owner", "teamId": "t1", "nickName": "boss-nick" },
    });
    // u_dup 同时在 members 与 adminUsers → 取后者 ADMIN（角色升级）。
    data["adminUsers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "id": "u_dup" }));

    let rows = collect_members(&data);
    assert_eq!(role_of(&rows, "u_member"), Some("MEMBER"));
    assert_eq!(role_of(&rows, "u_admin"), Some("ADMIN"));
    assert_eq!(role_of(&rows, "u_boss"), Some("BOSS"));
    assert_eq!(role_of(&rows, "u_owner"), Some("OWNER"));
    assert_eq!(
        role_of(&rows, "u_dup"),
        Some("ADMIN"),
        "members+admin 去重取 ADMIN（后者）"
    );
    assert_eq!(
        rows.iter().filter(|r| r.user_id == "u_dup").count(),
        1,
        "u_dup 去重后仅一行"
    );
    let owner = rows.iter().find(|r| r.user_id == "u_owner").unwrap();
    assert_eq!(owner.team_id, "t1");
    assert_eq!(owner.nick_name, "boss-nick");
}

/// 回归锚: A1 —— 显式 role 字段覆盖默认角色 + normalize（MANAGER→ADMIN、CREATOR→OWNER）。
#[test]
fn collect_members_explicit_role_normalized() {
    let data = serde_json::json!({
        "members": [{ "id": "u1", "role": "MANAGER" }, { "userId": "u2", "role": "CREATOR" }],
    });
    let rows = collect_members(&data);
    assert_eq!(role_of(&rows, "u1"), Some("ADMIN"), "MANAGER→ADMIN");
    assert_eq!(
        role_of(&rows, "u2"),
        Some("OWNER"),
        "CREATOR→OWNER；userId 键也认"
    );
}

/// 回归锚: A1 —— 字符串化数组（旧 server tag）兼容 + 空源 → 空集。
#[test]
fn collect_members_stringified_and_empty() {
    let data = serde_json::json!({
        "members": "[{\"id\":\"u_str\"}]",
        "adminUsers": "",
        "boss": [],
    });
    let rows = collect_members(&data);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].user_id, "u_str");

    assert!(
        collect_members(&serde_json::json!({})).is_empty(),
        "无成员源 → 空集"
    );
}

/// 回归锚: A1 —— **核心修复**：增量帧成员仅经 `memberChange.join` 交付（`members[]` 稀疏/缺省）
/// → 必须全员落 channel_member 行（此前漏合并 join → 43 人群仅落稀疏 members[] 的少数行）。
#[test]
fn collect_members_merges_member_change_join() {
    // 真 Go 增量形态：full members[] 仅含建群少数成员，新进成员经 memberChange.join 增量交付。
    let data = serde_json::json!({
        "members": [{ "id": "u_seed" }],
        "owner": { "id": "u_owner" },
        "memberChange": {
            "join": [
                { "id": "u_join1", "teamId": "tj", "nickName": "nj1" },
                { "id": "u_join2", "role": "ADMIN" },
                { "userId": "u_join3" }
            ]
        }
    });
    let rows = collect_members(&data);

    assert_eq!(
        role_of(&rows, "u_seed"),
        Some("MEMBER"),
        "四源 members 仍在"
    );
    assert_eq!(role_of(&rows, "u_owner"), Some("OWNER"), "四源 owner 仍在");
    assert_eq!(
        role_of(&rows, "u_join1"),
        Some("MEMBER"),
        "join 缺省角色 MEMBER"
    );
    assert_eq!(
        role_of(&rows, "u_join2"),
        Some("ADMIN"),
        "join 显式 role 生效"
    );
    assert_eq!(
        role_of(&rows, "u_join3"),
        Some("MEMBER"),
        "join userId 键也认"
    );

    let j1 = rows.iter().find(|r| r.user_id == "u_join1").unwrap();
    assert_eq!(j1.team_id, "tj");
    assert_eq!(j1.nick_name, "nj1");

    // 全员（2 四源 + 3 join）= 5 行，无丢失。
    assert_eq!(rows.len(), 5, "四源 + join 全员落库，无成员丢失");
}

/// 回归锚: A1 —— join 与四源同 user_id 时：join 排在后 → dedup 取 join 角色（增量后到达即覆盖）。
#[test]
fn collect_members_join_overrides_four_source_role() {
    let data = serde_json::json!({
        "members": [{ "id": "u_x" }],
        "memberChange": { "join": [{ "id": "u_x", "role": "ADMIN" }] }
    });
    let rows = collect_members(&data);
    assert_eq!(rows.len(), 1, "同 user_id 去重为一行");
    assert_eq!(
        role_of(&rows, "u_x"),
        Some("ADMIN"),
        "join 在四源后 → 增量角色覆盖"
    );
}

/// 回归锚: A1 —— owner 经 join 以 MEMBER 角色出现时跳过（owner 已在四源以 OWNER 入集，不降级）。
#[test]
fn collect_members_join_skips_owner_as_member() {
    let data = serde_json::json!({
        "owner": { "id": "u_owner" },
        "memberChange": {
            "join": [
                { "id": "u_owner", "role": "MEMBER" },   // owner-as-MEMBER → 跳过
                { "id": "u_owner", "role": "OWNER" },     // 非 MEMBER → 不跳过（与四源同角色，dedup 合一）
                { "id": "u_normal", "role": "MEMBER" }
            ]
        }
    });
    let rows = collect_members(&data);
    assert_eq!(
        role_of(&rows, "u_owner"),
        Some("OWNER"),
        "owner 保持 OWNER（join MEMBER 条目被跳过）"
    );
    assert_eq!(role_of(&rows, "u_normal"), Some("MEMBER"));
    // owner 去重后一行 + u_normal 一行。
    assert_eq!(rows.iter().filter(|r| r.user_id == "u_owner").count(), 1);
}

/// boss leave 不创建幽灵成员；若 members 仍含该用户，角色自然恢复为 MEMBER。
#[test]
fn collect_members_boss_leave_restores_member_and_leave_wins() {
    let data = serde_json::json!({
        "members": [{ "id": "u_boss" }, { "id": "u_left" }],
        "boss": [{ "id": "u_boss", "option": "leave" }],
        "memberChange": { "leave": [{ "id": "u_left" }] }
    });
    let rows = collect_members(&data);
    assert_eq!(role_of(&rows, "u_boss"), Some("MEMBER"));
    assert_eq!(collect_member_leaves(&data), vec!["u_left".to_string()]);

    let join_only = serde_json::json!({
        "boss": [{ "id": "u_boss", "option": "leave" }]
    });
    assert!(
        collect_members(&join_only).is_empty(),
        "boss leave 不应凭空插入成员"
    );
}

/// 回归锚: A1 —— `memberChange.leave` 解析出离场 user_id 集（缺 id 跳过；id ?? userId）。
/// 调用方将此集接成 `StorageOp::BatchDelete(channel_member, scope=channel_id, key=user_id)`。
#[test]
fn collect_member_leaves_extracts_user_ids() {
    let data = serde_json::json!({
        "memberChange": {
            "leave": [
                { "id": "u_left1" },
                { "userId": "u_left2" },
                { "id": "" },        // 空 id 跳过
                { "role": "MEMBER" } // 无 id 跳过
            ]
        }
    });
    let leaves = collect_member_leaves(&data);
    assert_eq!(leaves, vec!["u_left1".to_string(), "u_left2".to_string()]);

    // 无 memberChange / 无 leave → 空集。
    assert!(collect_member_leaves(&serde_json::json!({})).is_empty());
    assert!(collect_member_leaves(&serde_json::json!({ "memberChange": {} })).is_empty());
}
