//! 成员来源按字段合并：当前 members > 当前角色摘要 > 历史 join。
//! 未提供字段保留 presence；显式空昵称允许清空。离场只由明确 leave 表达。

use crate::acl::to_effect_s1::MemberRow;

/// 当前成员字段优先于精简角色摘要和历史 join；O(n) 按用户合并。
pub fn collect_members(data: &serde_json::Value) -> Vec<MemberRow> {
    let mut out = Vec::new();
    collect_member_change_join(data, &mut out);
    collect_array(data.get("adminUsers"), "ADMIN", &mut out);
    let mut boss_leaves = std::collections::HashSet::new();
    collect_boss_array(data.get("boss"), &mut out, &mut boss_leaves);
    collect_owner(data.get("owner"), &mut out);
    let mut current = Vec::new();
    collect_array(data.get("members"), "MEMBER", &mut current);
    for member in &mut current {
        // 明确撤销 BOSS + 仍在当前成员名单中：缺 role 表达降为普通成员。
        // 当前名单显式 role 仍拥有最高优先级，不能凭 leave 创建新成员。
        if boss_leaves.contains(&member.user_id) && member.omitted_fields.contains(&"role") {
            member.role = "MEMBER".to_string();
            member.omitted_fields.retain(|field| *field != "role");
        }
    }
    out.extend(current);
    merge_present_fields(out)
}

/// `memberChange.join[]` 增量成员 → MemberRow（真源 `apply_member_change_tx:247`）。
///
/// 每元素角色取 `role` ?? `MEMBER`（经 `normalize_role` 归一）；**跳过** `user_id == owner_id`
/// 且归一角色为 MEMBER 的条目——owner 已由四源 `owner{}` 以 OWNER 入集，不让 join 把它降级。
/// owner_id 取 `owner.id` ?? `owner.userId`（兼容两 wire 拼写；缺省空串 = 不跳过任何条目）。
fn collect_member_change_join(data: &serde_json::Value, out: &mut Vec<MemberRow>) {
    let Some(join) = data
        .get("memberChange")
        .and_then(|m| m.get("join"))
        .and_then(serde_json::Value::as_array)
    else {
        return;
    };
    let owner_id = data
        .get("owner")
        .and_then(|o| o.get("id").or_else(|| o.get("userId")))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    for item in join {
        let Some(row) = member_row_from_value(item, "MEMBER") else {
            continue;
        };
        // owner-as-MEMBER 跳过（owner 已在四源以 OWNER 入集，join 不降级）。
        if row.user_id == owner_id && row.role == "MEMBER" {
            continue;
        }
        out.push(row);
    }
}

/// `memberChange.leave[]` → 离场成员的 user_id 集（边界零信任，缺 id 跳该条）。
///
/// 真源 `apply_member_change_tx:270` 的 `leave_list.filter_map(|item| item.id)`。
/// ✅ **已接通**：消费方 `delete_channel_members` → `StorageOp::BatchDelete`，由
/// `increment_channel.rs:69` + `channel_member_update.rs:114` 两 handler 调用（物理删离场行）。
pub fn collect_member_leaves(data: &serde_json::Value) -> Vec<String> {
    data.get("memberChange")
        .and_then(|m| m.get("leave"))
        .and_then(serde_json::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("id")
                        .or_else(|| item.get("userId"))
                        .and_then(serde_json::Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 数组源（members / adminUsers / boss）→ 逐元素 MemberRow（缺/空 userId 跳过）。
///
/// 兼容真 wire 两形态（现网 `collect_array_value`）：
///   - 直接 JSON 数组（`[{...}]`）；
///   - 字符串化数组（`"[{...}]"`，旧 server json tag）——再解析一层。
fn collect_array(value: Option<&serde_json::Value>, default_role: &str, out: &mut Vec<MemberRow>) {
    let Some(value) = value else { return };
    // 字符串化数组：先内层解析（失败 → 跳过，零信任不 panic）。
    let parsed;
    let value = match value.as_str() {
        Some(s) if s.trim().is_empty() => return,
        Some(s) => {
            parsed =
                serde_json::from_str::<serde_json::Value>(s).unwrap_or(serde_json::Value::Null);
            &parsed
        }
        None => value,
    };
    let Some(arr) = value.as_array() else { return };
    for item in arr {
        if let Some(row) = member_row_from_value(item, default_role) {
            out.push(row);
        }
    }
}

/// boss 数组中的 leave 只撤销 BOSS 角色；成员是否仍在群由 members/其它权威源决定。
fn collect_boss_array(
    value: Option<&serde_json::Value>,
    out: &mut Vec<MemberRow>,
    leaves: &mut std::collections::HashSet<String>,
) {
    let Some(value) = value else { return };
    let parsed;
    let value = match value.as_str() {
        Some(s) if s.trim().is_empty() => return,
        Some(s) => {
            parsed =
                serde_json::from_str::<serde_json::Value>(s).unwrap_or(serde_json::Value::Null);
            &parsed
        }
        None => value,
    };
    let Some(arr) = value.as_array() else { return };
    for item in arr {
        if item.get("option").and_then(serde_json::Value::as_str) == Some("leave") {
            if let Some(id) = item
                .get("userId")
                .or_else(|| item.get("id"))
                .and_then(serde_json::Value::as_str)
            {
                leaves.insert(id.to_string());
            }
            continue;
        }
        if let Some(row) = member_row_from_value(item, "BOSS") {
            out.push(row);
        }
    }
}

/// owner 源（单对象）→ MemberRow（角色 OWNER）。兼容字符串化对象（旧 server tag）。
fn collect_owner(value: Option<&serde_json::Value>, out: &mut Vec<MemberRow>) {
    let Some(value) = value else { return };
    let parsed;
    let value = match value.as_str() {
        Some(s) if s.trim().is_empty() => return,
        Some(s) => {
            parsed =
                serde_json::from_str::<serde_json::Value>(s).unwrap_or(serde_json::Value::Null);
            &parsed
        }
        None => value,
    };
    if value.is_object() {
        if let Some(row) = member_row_from_value(value, "OWNER") {
            out.push(row);
        }
    }
}

/// 单成员对象 → MemberRow（真源 `member_input_from_value`）。
///
/// `userId` ?? `id`（缺 / 空 → None 跳该条）；显式 `role` 覆盖 `default_role`（经 normalize_role
/// 归一为 OWNER/ADMIN/BOSS/MEMBER 四值）；`teamId` / `nickName` 透传缺省空串。
fn member_row_from_value(value: &serde_json::Value, default_role: &str) -> Option<MemberRow> {
    let obj = value.as_object()?;
    let user_id = obj
        .get("userId")
        .or_else(|| obj.get("id"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())?
        .to_string();
    let team_id = obj
        .get("teamId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let role = obj
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or(default_role);
    if obj.get("role").is_some_and(|v| {
        !v.is_null()
            && !matches!(
                v.as_str(),
                Some("OWNER" | "CREATOR" | "ADMIN" | "MANAGER" | "MANGER" | "BOSS" | "MEMBER")
            )
    }) {
        return None;
    }
    let nick_name = obj
        .get("nickName")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(MemberRow {
        user_id,
        team_id,
        role: normalize_role(role).to_string(),
        nick_name,
        omitted_fields: [
            (
                "team_id",
                obj.get("teamId")
                    .and_then(|v| v.as_str())
                    .filter(|v| !v.is_empty())
                    .is_none(),
            ),
            (
                "nick_name",
                obj.get("nickName").and_then(|v| v.as_str()).is_none(),
            ),
            (
                "role",
                default_role == "MEMBER" && obj.get("role").and_then(|v| v.as_str()).is_none(),
            ),
        ]
        .into_iter()
        .filter_map(|(field, omitted)| omitted.then_some(field))
        .collect(),
    })
}

/// 角色归一（真源 `channel_member_repo::normalize_role`）：OWNER/CREATOR→OWNER，
/// ADMIN/MANAGER→ADMIN，BOSS→BOSS，其余→MEMBER。
fn normalize_role(role: &str) -> &'static str {
    match role {
        "OWNER" | "CREATOR" => "OWNER",
        "ADMIN" | "MANAGER" | "MANGER" => "ADMIN",
        "BOSS" => "BOSS",
        _ => "MEMBER",
    }
}

/// 同一用户只覆盖来源实际提供的字段；稳定输入顺序、时间空间 O(n)。
fn merge_present_fields(members: Vec<MemberRow>) -> Vec<MemberRow> {
    let mut positions = std::collections::HashMap::new();
    let mut result: Vec<MemberRow> = Vec::new();
    for member in members {
        if let Some(&index) = positions.get(&member.user_id) {
            let current: &mut MemberRow = &mut result[index];
            for field in ["team_id", "role", "nick_name"] {
                if member.omitted_fields.contains(&field) {
                    continue;
                }
                match field {
                    "team_id" => current.team_id.clone_from(&member.team_id),
                    "role" => current.role.clone_from(&member.role),
                    _ => current.nick_name.clone_from(&member.nick_name),
                }
                current.omitted_fields.retain(|omitted| *omitted != field);
            }
        } else {
            positions.insert(member.user_id.clone(), result.len());
            result.push(member);
        }
    }
    result
}
