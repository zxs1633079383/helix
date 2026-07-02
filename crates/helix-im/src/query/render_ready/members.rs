//! S7 成员族 render-ready 投影 —— `im:channel:members`（issue #56·C013 纯渲染壳）。
//!
//! 把**成员渲染整形**（`role→admin` 业务规则 + 四源/byIds 解析重组）从渲染壳下沉 helix-im：
//! 前端 `applyChannelMembers` 退化为**纯绑定**（按 `memberId` keyed upsert + `leaves` 删行，
//! 零 extract / 零 role 判定 / 零四源合并）。
//!
//! ## 形态
//! emit `im:channel:members{channelId, members:[{memberId, nickname, admin}], leaves:[memberId]}`：
//!   - `members` = 本帧应在册成员的 render-ready 终态行（`admin` 成品·helix 已判 role→admin）。
//!   - `leaves`  = 本帧离场成员 id 集（壳据此删 MB 区成员行）。
//!
//! ## 与冻结投影的关系（契约只读·C004）
//! 本投影是 **render-ready 渲染通道**（与 `im:read:result` §1.2 同性质·**不计入** 21 投影冻结集——
//! 前端直绑、后端权威态由 `channel_member` DB 表兑现）。冻结的 `im:channel:member-updated`
//! （`{channel_id, channel}`）/ `im:read:result`（`{req_id, body}`）/ `im:channel:memberNickname`
//! 三投影**保持原样**，其 ② 契约面照旧裁定——本投影**额外**emit，不改既有键集。

use crate::acl::to_effect_s1::MemberRow;
use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};

/// render-ready 成员行（`admin` 成品·`role→admin` 已在 helix 判）。
pub struct RenderMember {
    pub member_id: String,
    pub nickname: String,
    pub admin: bool,
}

/// `role→admin` 业务规则（下沉 helix·壳禁判）。
///
/// 统一覆盖两种取值来源：byIds 响应体的 **raw wire role**（`CREATOR`/`MANGER`/`MANAGER`/`ADMIN`）
/// 与 `collect_members` 的 `MemberRow.role`（已 `normalize_role` 归一为 `OWNER`/`ADMIN`/`BOSS`/`MEMBER`）。
/// `BOSS` / `MEMBER` → 非 admin（对齐原壳 `applyMembersSnapshot` 仅 CREATOR/MANGER/ADMIN 判 admin·不含 BOSS）。
pub fn role_is_admin(role: &str) -> bool {
    matches!(role, "CREATOR" | "OWNER" | "ADMIN" | "MANAGER" | "MANGER")
}

/// `collect_members` 的 `MemberRow`（四源 + memberChange.join·已 dedup·role 已归一）→ render-ready 成员集。
pub fn members_from_rows(rows: Vec<MemberRow>) -> Vec<RenderMember> {
    rows.into_iter()
        .map(|m| RenderMember {
            admin: role_is_admin(&m.role),
            member_id: m.user_id,
            nickname: m.nick_name,
        })
        .collect()
}

/// byIds 响应体 `map[channelId][]IdWithCompanyExt` → 逐 channel 的 render-ready 成员集。
///
/// `body` = `{status, data:{channelId:[{id, role, nickName, ...}]}}`（透传响应体·见 channel_read.rs §5）。
/// 每元素 `id→memberId` · `nickName→nickname` · `role→admin`（raw role 判）。缺/空 id 跳过·channel 内去重保首。
/// `data` 缺 / 非对象 → 空（边界零信任·老群 444 非成员 `data:{}` → 该 channel 空成员集）。
pub fn members_from_byids_body(body: &serde_json::Value) -> Vec<(String, Vec<RenderMember>)> {
    let Some(data) = body.get("data").and_then(serde_json::Value::as_object) else {
        return Vec::new();
    };
    let mut out: Vec<(String, Vec<RenderMember>)> = Vec::new();
    for (channel_id, arr) in data {
        let Some(arr) = arr.as_array() else { continue };
        let mut members: Vec<RenderMember> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for item in arr {
            let Some(obj) = item.as_object() else { continue };
            let Some(id) = obj
                .get("id")
                .and_then(serde_json::Value::as_str)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            if !seen.insert(id.to_string()) {
                continue;
            }
            let nickname = obj
                .get("nickName")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string();
            let role = obj.get("role").and_then(serde_json::Value::as_str).unwrap_or("");
            members.push(RenderMember {
                member_id: id.to_string(),
                nickname,
                admin: role_is_admin(role),
            });
        }
        out.push((channel_id.clone(), members));
    }
    out
}

/// emit `im:channel:members{channelId, members:[{memberId,nickname,admin}], leaves:[memberId]}`。
///
/// 静态 shape（序列化失败=编程错·边界输入零信任已在上游解析完成）。
pub fn emit_channel_members(
    channel_id: &str,
    members: Vec<RenderMember>,
    leaves: Vec<String>,
) -> Effect {
    let members_json: Vec<serde_json::Value> = members
        .into_iter()
        .map(|m| {
            serde_json::json!({
                "memberId": m.member_id,
                "nickname": m.nickname,
                "admin": m.admin,
            })
        })
        .collect();
    let payload = serde_json::json!({
        "event": "im:channel:members",
        "data": { "channelId": channel_id, "members": members_json, "leaves": leaves },
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload).expect("emit_channel_members: static JSON shape must serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

#[cfg(test)]
#[path = "members_tests.rs"]
mod tests;
