//! S3 成员收集 — `collect_members`：increment_channel / channel_created / channel_member_update
//! 帧的 `members/owner/adminUsers/boss` **四源** + `memberChange.join` 增量 → `channel_member`
//! 行（复合 PK，三路径同口径）。
//!
//! 行为真源 = 现网 `repo/channel_member_repo.rs`：
//! - 四源全量（`collect_members_from_channel_json:303`，`tables/channel.rs:333` upsert_channel_tx
//!   内调用 → `replace_channel_members_tx`）：
//!   - `members[]`   → 默认角色 `MEMBER`
//!   - `adminUsers[]`→ 默认角色 `ADMIN`
//!   - `boss[]`      → 默认角色 `BOSS`
//!   - `owner{}`     → 角色 `OWNER`（单对象，非数组）
//! - 增量 join（`apply_member_change_tx:247`）：`memberChange.join[]` 逐元素 upsert，
//!   角色取元素 `role` ?? `MEMBER`；**跳过** `user_id == owner_id && normalize_role==MEMBER`
//!   的条目（owner 已由四源 `owner{}` 以 OWNER 角色入集，不被降级为 MEMBER）。
//!
//! 每元素 `userId` ?? `id`（缺 / 空 → 跳该条，零信任）；显式 `role` 字段覆盖默认角色（经
//! `normalize_role` 归一）；`teamId` / `nickName` 透传。dedup **保留最后出现**（owner/boss/admin
//! /join 覆盖 member——对齐现网 `dedupe_members` rev 语义；join 排在四源后，相同 user_id 时
//! join 的角色为准，与现网增量后到达即覆盖一致）。
//!
//! ⚠️ **A1 修复**：此前 `increment_channel.rs::apply_increment` 只读四源（`members[]` 等常缺省）
//! → 真 Go 增量帧成员经 `memberChange.join` 增量交付时全丢（43 人群仅落 3 行）。三路径统一改走
//! 本函数（四源 + join），消除 `channel_member_update.rs::parse_member_join`「只读 join 不读四源」
//! 与本函数「只读四源不读 join」的同仓不一致。
//!
//! ✅ **memberChange.leave 物理删（已接通）**：现网 `apply_member_change_tx:270` 对 leave 走物理
//! `DELETE FROM channel_member WHERE channel_id=? AND user_id IN(…)`。helix-core `StorageOp` 已加
//! **`BatchDelete` 变体**（`effect.rs:101` + `BatchDeleteSpec`，scope 等值约束防跨群误删），三端
//! driver 已适配。leave 删除**两路真接通**：`collect_member_leaves` 解析离场 user_id 集 →
//! `acl::to_effect_s1::delete_channel_members` → `StorageOp::BatchDelete`，由 `increment_channel.rs:69`
//! 与 `channel_member_update.rs:114` 两个 handler 调用。本函数（`collect_members`）只管**收集应在册
//! 成员**（join + 四源 upsert）；离场删除是独立 `collect_member_leaves` 路径，二者分工互补。

use crate::acl::to_effect_s1::MemberRow;

/// 帧 data 的四源 + `memberChange.join` → 去重后的 `channel_member` 行（角色映射 + dedup）。
///
/// 顺序：members→adminUsers→boss→owner→memberChange.join，末尾去重保留后者（角色升级 / 增量覆盖
/// 语义，对齐现网 `dedupe_members` rev + 「增量后到达即覆盖」）。空集 → 空 `Vec`（调用方据此跳过
/// 写库）。复杂度 O(n)（n=四源 + join 元素总数；冷路径，非每事件热路径）。
///
/// leave 不在此处理（离场删除走独立 `collect_member_leaves` → `BatchDelete` 路径，见模块头）；
/// 本函数只收集应在册成员（join + 四源 upsert）。
pub fn collect_members(data: &serde_json::Value) -> Vec<MemberRow> {
    let mut out: Vec<MemberRow> = Vec::new();
    collect_array(data.get("members"), "MEMBER", &mut out);
    collect_array(data.get("adminUsers"), "ADMIN", &mut out);
    collect_array(data.get("boss"), "BOSS", &mut out);
    collect_owner(data.get("owner"), &mut out);
    collect_member_change_join(data, &mut out);
    dedupe_keep_last(out)
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
    })
}

/// 角色归一（真源 `channel_member_repo::normalize_role`）：OWNER/CREATOR→OWNER，
/// ADMIN/MANAGER→ADMIN，BOSS→BOSS，其余→MEMBER。
fn normalize_role(role: &str) -> &'static str {
    match role {
        "OWNER" | "CREATOR" => "OWNER",
        "ADMIN" | "MANAGER" => "ADMIN",
        "BOSS" => "BOSS",
        _ => "MEMBER",
    }
}

/// 去重 by user_id，**保留最后出现**（owner/boss/admin 覆盖 member 角色升级）。
///
/// 真源 `dedupe_members`：rev 遍历入 seen，再 reverse 恢复顺序——等价「同 user_id 取后者」。
/// 复杂度 O(n)（HashSet 一遍）。
fn dedupe_keep_last(members: Vec<MemberRow>) -> Vec<MemberRow> {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::with_capacity(members.len());
    let mut result: Vec<MemberRow> = Vec::with_capacity(members.len());
    for m in members.into_iter().rev() {
        if seen.insert(m.user_id.clone()) {
            result.push(m);
        }
    }
    result.reverse();
    result
}

// 回归测试见 `tests/channel_member_collect_test.rs`（四源 + memberChange.join + leave；
// 仅用公开 API `collect_members`/`collect_member_leaves`，移出 src 以守 ≤300 行硬顶）。
