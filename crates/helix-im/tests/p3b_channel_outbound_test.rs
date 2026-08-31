//! P3b — channel 写族 outbound（createSpecifyOwner / change info·source·picture·notice·top /
//! member role·notify）+ P0-2 系统消息消费编排核验 集成测试。
//!
//! 回归锚: P3b
//!
//! 覆盖维度（02 文档 P3b gate + 03 测试矩阵 P3b 行）：
//! - 每命令 1 正常路径：endpoint url + body casing 断言（锚现网 Go 真源 cses_channel.go /
//!   command/channel*.go 逐字 json tag，**不自造 fixture**，C5）
//! - **quirk casing 真 Go fixture 比对**（decode 静默失败防线）：
//!   · createSpecifyOwner 透传 raw body → `direct_key` **snake_case** 不被改写（ent.Channel tag）
//!   · change/source·picture·notice 用 **`id`** 作 channelId 键（command json tag），info/top 用 `channelId`
//!   · member/notify request body 用 `channelId`（WS 响应才是 `id`——不在 outbound 断言）
//! - G-15/G-15a canonical adapter：只做 snake_case command 到 camelCase HTTP body 的机械映射，
//!   身份、团队、角色与 root post 派生由 Go boundary 负责
//! - **#15 change/props 故意不广播**：im_update_channel_props **未注册**（不该有此 outbound 命令）
//!   且无 update_channel WS handler 凭空广播——断言 registry 不认领该名
//! - P0-2 系统消息消费编排（已在 main 落地，本 Phase 核验不重造）：
//!   · **反 RMW 性质（B5）**：他人 `post` 帧 → unread bump 产**单条** `GuardedBump`（SET col=col+1），
//!     **不含** SELECT（GuardedBump 非 read-modify-write）、**不含**全列 upsert
//!   · channel handler 只触发不自拼 post 帧（DDD 聚合根解耦）：channel outbound 命令产 Effect::Http
//!     **不产** Persist / 不自造 post 帧
//! - registry 性质：新增命令全 is_outbound 命中 + 全可 dispatch（注册数==命令数，无重复/漏）
//!
//! 可证伪（HX-C011）：改错任一 endpoint/casing → 对应断言 FAIL；把 change/source 的 `id` 写成
//! `channelId` → casing 断言 FAIL；给 props 加 outbound 命令 → #15 断言 FAIL；把 unread bump 改成
//! 先 SELECT 再写回（RMW）→ 反 RMW 断言 FAIL；channel 命令自拼 post 帧产 Persist → 解耦断言 FAIL。

mod common;
use common::*;

use helix_core::effect::{Effect, StorageOp};
use helix_core::tick::InboundBytes;
use helix_core::Correlation;
use helix_core::Tick;
use helix_im::commands::{handle_outbound, is_outbound};
use serde_json::{json, Value};

const BASE: &str = "http://x/api/cses";

fn corr() -> Correlation {
    Correlation::from_raw(83)
}

/// 取出单条 Http 的 (url, body-json)；非单条 Http → panic（channel 写命令恒产单条 Effect::Http）。
fn one_http(effects: Vec<Effect>) -> (String, Value) {
    assert_eq!(effects.len(), 1, "channel 写命令当前恒产单条 Effect::Http");
    match effects.into_iter().next().unwrap() {
        Effect::Http { req, .. } => {
            let body: Value = serde_json::from_slice(&req.body.unwrap()).unwrap();
            (req.url, body)
        }
        other => panic!("expected Effect::Http, got {other:?}"),
    }
}

fn run(name: &str, payload: Value) -> Vec<Effect> {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        Some("conn-3b"),
        corr(),
    )
    .expect("handle_outbound should not fail")
}

fn err(name: &str, payload: Value) -> bool {
    handle_outbound(
        name,
        &serde_json::to_vec(&payload).unwrap(),
        BASE,
        BASE,
        None,
        corr(),
    )
    .is_err()
}

// ── createSpecifyOwner（#8，raw 透传 + snake_case direct_key 保真）───────────────────

#[test]
fn create_specify_owner_passthrough_preserves_snake_case_direct_key() {
    // 真 curl §4 形态 + ent.Channel.DirectKey snake_case `direct_key`（迁移注意点 3）。
    // raw 透传：helix 不改写嵌套结构，snake_case direct_key 保真（写错成 directKey → Go 丢该字段）。
    let (url, body) = one_http(run(
        "im_create_specify_owner",
        json!({
            "teamId": "t1",
            "displayName": "群名",
            "type": "P",
            "direct_key": "u1:u2",
            "users": [{ "id": "u1", "teamId": "t1", "role": "CREATOR" }],
            "picturetype": "USER",
            "picture": { "userIds": ["u1", "u2"] },
            "forceCreate": true,
            "owner": { "id": "u2", "teamId": "t1", "role": "CREATOR" }
        }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/createSpecifyOwner");
    assert_eq!(
        body["direct_key"], "u1:u2",
        "direct_key 必须 snake_case 保真（ent.Channel tag 异常，写错成 directKey 即丢字段）"
    );
    assert!(
        body.get("directKey").is_none(),
        "禁 camelCase directKey（命名陷阱）"
    );
    assert_eq!(body["users"][0]["role"], "CREATOR");
    assert_eq!(body["forceCreate"], true);
    assert_eq!(body["owner"]["id"], "u2");
}

#[test]
fn create_specify_owner_requires_display_name() {
    assert!(err("im_create_specify_owner", json!({ "teamId": "t" }))); // 缺 displayName
}

// ── change/info（#11，channelId 键 + 可选字段 omitempty 透传）───────────────────────

#[test]
fn channel_settings_change_info_builds_channel_id_key_with_optional_fields() {
    let (url, body) = one_http(run(
        "im_channel_change_info",
        json!({
            "channel_id": "cI",
            "display_name": "新名",
            "purpose": "新简介",
            "picture": { "userIds": ["u1"] }
        }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/change/info");
    assert_eq!(body["channelId"], "cI", "info 用 channelId 键");
    assert_eq!(
        body["displayName"], "新名",
        "display_name → camelCase displayName"
    );
    assert_eq!(body["purpose"], "新简介");
    let camel_version_key = ["expected", "Version"].concat();
    assert!(body.get(camel_version_key).is_none());
    assert_eq!(body["picture"]["userIds"][0], "u1");
    // 未提供的可选字段不下发（Go 指针 omitempty）。
    assert!(body.get("name").is_none());
    assert!(body.get("header").is_none());

    // 显式 null 的可选 map 字段（put_opt_val is_null 分支）→ 不下发（透传 null 会被 Go 当清空）。
    let (_u, body2) = one_http(run(
        "im_channel_change_info",
        json!({ "channel_id": "cI", "name": "n", "header": "h", "module": "m", "source": null }),
    ));
    assert_eq!(body2["name"], "n");
    assert_eq!(body2["header"], "h");
    assert_eq!(body2["module"], "m");
    assert!(
        body2.get("source").is_none(),
        "显式 null 的 source 不下发（put_opt_val is_null 跳过分支）"
    );
}

#[test]
fn change_info_requires_channel_id() {
    assert!(err(
        "im_channel_change_info",
        json!({ "display_name": "x" })
    ));
}

// ── change/source（#12，id 键陷阱）──────────────────────────────────────────────────

#[test]
fn change_source_uses_id_key_trap() {
    // **命名陷阱**：command.ChangeChannelSource `json:"id"`（id=channelId，非 channelId 键）。
    let (url, body) = one_http(run(
        "im_channel_change_source",
        json!({ "channel_id": "cS", "title": "来源标题" }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/change/source");
    assert_eq!(body["id"], "cS", "source 用 id 键（非 channelId）");
    assert!(
        body.get("channelId").is_none(),
        "source 禁 channelId 键（命名陷阱）"
    );
    assert_eq!(body["title"], "来源标题");
}

#[test]
fn change_source_title_defaults_empty_and_requires_channel() {
    let (_url, body) = one_http(run(
        "im_channel_change_source",
        json!({ "channel_id": "c" }),
    ));
    assert_eq!(body["title"], "", "title 缺省空串（Go string 零值）");
    assert!(err("im_channel_change_source", json!({ "title": "t" }))); // 缺 channel_id
}

// ── change/picture（#14，id 键 + pictureType + picture map 必填）─────────────────────

#[test]
fn change_picture_builds_id_picture_type_picture() {
    let (url, body) = one_http(run(
        "im_channel_change_picture",
        json!({
            "channel_id": "cP",
            "picture_type": "USER",
            "picture": { "userIds": ["u1", "u2"] }
        }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/change/picture");
    assert_eq!(body["id"], "cP", "picture 用 id 键（非 channelId）");
    assert_eq!(body["pictureType"], "USER");
    assert_eq!(body["picture"]["userIds"][1], "u2");
}

#[test]
fn change_picture_requires_type_and_picture() {
    assert!(err(
        "im_channel_change_picture",
        json!({ "channel_id": "c", "picture": {} })
    )); // 缺 picture_type
    assert!(err(
        "im_channel_change_picture",
        json!({ "channel_id": "c", "picture_type": "USER" })
    )); // 缺 picture
    assert!(err(
        "im_channel_change_picture",
        json!({ "channel_id": "c", "picture_type": "USER", "picture": null })
    )); // picture null
    assert!(err(
        "im_channel_change_picture",
        json!({ "picture_type": "USER", "picture": {} })
    )); // 缺 channel_id（覆盖 require_str channel_id `?` 错误分支）
}

// ── change/notice（#10，id 键 + notice map）─────────────────────────────────────────

#[test]
fn change_notice_builds_id_notice() {
    let (url, body) = one_http(run(
        "im_channel_change_notice",
        json!({ "channel_id": "cN", "notice": { "text": "公告内容" } }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/change/notice");
    assert_eq!(
        body["id"], "cN",
        "notice 用 id 键（command.ChangeChannelNotice json:id）"
    );
    assert!(body.get("channelId").is_none());
    assert_eq!(body["notice"]["text"], "公告内容");
    let camel_version_key = ["expected", "Version"].concat();
    assert!(body.get(camel_version_key).is_none());
}

#[test]
fn change_notice_requires_channel_and_notice() {
    assert!(err(
        "im_channel_change_notice",
        json!({ "channel_id": "c" })
    )); // 缺 notice
    assert!(err(
        "im_channel_change_notice",
        json!({ "notice": { "text": "x" } })
    )); // 缺 channel_id
    assert!(err(
        "im_channel_change_notice",
        json!({ "channel_id": "c", "notice": null })
    )); // notice null
}

// ── change/top（#18，channelId 键 + top bool）───────────────────────────────────────

#[test]
fn change_top_builds_channel_id_and_top_bool() {
    let (url, body) = one_http(run(
        "im_channel_change_top",
        json!({ "channel_id": "cT", "top": true }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/change/top");
    assert_eq!(body["channelId"], "cT", "top 用 channelId 键");
    assert_eq!(body["top"], true);
    // 取消置顶 top=false 也须显式下发（区分于缺省）。
    let (_u, body2) = one_http(run(
        "im_channel_change_top",
        json!({ "channel_id": "cT", "top": false }),
    ));
    assert_eq!(body2["top"], false);
}

#[test]
fn change_top_requires_channel_and_bool_top() {
    assert!(err("im_channel_change_top", json!({ "channel_id": "c" }))); // 缺 top
    assert!(err(
        "im_channel_change_top",
        json!({ "channel_id": "c", "top": "yes" })
    )); // top 非 bool
    assert!(err("im_channel_change_top", json!({ "top": true }))); // 缺 channel_id
}

// ── member/change/role（#29）────────────────────────────────────────────────────────

#[test]
fn member_role_builds_channel_userids_role() {
    let (url, body) = one_http(run(
        "im_channel_member_role",
        json!({ "channel_id": "cR", "user_ids": ["u1", "u2"], "role": "ADMIN" }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/member/change/role");
    assert_eq!(body["channelId"], "cR");
    assert_eq!(body["userIds"][0], "u1");
    assert_eq!(body["userIds"][1], "u2");
    assert!(body["userIds"].is_array());
    assert_eq!(body["role"], "ADMIN");
}

#[test]
fn member_role_requires_channel_nonempty_userids_role() {
    assert!(err(
        "im_channel_member_role",
        json!({ "channel_id": "c", "user_ids": [], "role": "ADMIN" })
    )); // 空 userIds
    assert!(err(
        "im_channel_member_role",
        json!({ "channel_id": "c", "user_ids": [1], "role": "ADMIN" })
    )); // 非 string
    assert!(err(
        "im_channel_member_role",
        json!({ "channel_id": "c", "user_ids": ["u"] })
    )); // 缺 role
    assert!(err(
        "im_channel_member_role",
        json!({ "user_ids": ["u"], "role": "ADMIN" })
    )); // 缺 channel_id
}

// ── member/change/notify（#30，request 用 channelId；WS 响应才是 id）────────────────

#[test]
fn member_notify_request_uses_channel_id_not_response_id_key() {
    // request body 用 channelId（command.UpdateChannelNotify tag）；WS **响应** payload 才经
    // UpdateChannelNotifyVo.ToMap() 变 `id` 键——那是 P4 WS 事件解析侧，**不混进** request body。
    let (url, body) = one_http(run(
        "im_channel_member_notify",
        json!({ "channel_id": "cNo", "notify": "NORMAL" }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/member/change/notify");
    assert_eq!(body["channelId"], "cNo", "notify request 用 channelId 键");
    assert!(
        body.get("id").is_none(),
        "notify request body 禁 id 键（id 是 WS 响应 payload 键，非 request）"
    );
    assert_eq!(body["notify"], "NORMAL");
}

/// NotifyType 只允许 Go 真源的三个大写字符串，并拒绝未知/整数值及缺失字段。
#[test]
fn member_notify_accepts_only_strict_enum_and_requires_fields() {
    for notify in ["NORMAL", "STRONG", "IGNORE"] {
        let (_u, body) = one_http(run(
            "im_channel_member_notify",
            json!({ "channel_id": "c", "notify": notify }),
        ));
        assert_eq!(body["notify"], notify);
    }
    assert!(err(
        "im_channel_member_notify",
        json!({ "channel_id": "c", "notify": 2 })
    )); // 禁止整数枚举
    assert!(err(
        "im_channel_member_notify",
        json!({ "channel_id": "c", "notify": "MENTION" })
    )); // 禁止未知字符串
    assert!(err(
        "im_channel_member_notify",
        json!({ "channel_id": "c", "notify": "" })
    )); // 禁止空字符串
    assert!(err(
        "im_channel_member_notify",
        json!({ "channel_id": "c" })
    )); // 缺 notify
    assert!(err("im_channel_member_notify", json!({ "notify": "x" }))); // 缺 channel_id
    assert!(err(
        "im_channel_member_notify",
        json!({ "channel_id": "c", "notify": null })
    )); // notify null
}

// ── G-15 / G-15a canonical command → Go HTTP boundary adapter ──

#[test]
fn make_topic_maps_only_canonical_fields() {
    let (url, body) = one_http(run(
        "im_make_topic",
        json!({
            "root_id": "r1",
            "user_ids": ["u1", "u2"],
            "req_id": "topic-request-1"
        }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/makeTopic");
    assert_eq!(
        body,
        json!({
            "rootId": "r1",
            "userIds": ["u1", "u2"]
        })
    );
}

#[test]
fn make_topic_forwards_optional_user_confirmed_display_name() {
    let (url, body) = one_http(run(
        "im_make_topic",
        json!({
            "root_id": "r1",
            "user_ids": ["u1", "u2"],
            "display_name": "设计评审",
            "req_id": "topic-request-2"
        }),
    ));
    assert_eq!(url, "http://x/api/cses/posts/makeTopic");
    assert_eq!(
        body,
        json!({
            "rootId": "r1",
            "userIds": ["u1", "u2"],
            "displayName": "设计评审"
        })
    );
}

#[test]
fn create_channel_maps_only_canonical_fields() {
    let (url, body) = one_http(run(
        "im_create_channel",
        json!({
            "type": "O",
            "user_ids": ["u1", "u2"],
            "display_name": "群",
            "force_create": true
        }),
    ));
    assert_eq!(url, "http://x/api/cses/channel/create");
    assert_eq!(
        body,
        json!({
            "type": "O",
            "userIds": ["u1", "u2"],
            "displayName": "群",
            "forceCreate": true
        })
    );
}

#[test]
fn create_channel_and_make_topic_reject_legacy_unknown_and_wrong_types() {
    assert!(err(
        "im_create_channel",
        json!({
            "type": "O", "user_ids": ["u1"], "display_name": "群",
            "force_create": true, "picture": "legacy-client-avatar"
        })
    ));
    assert!(err(
        "im_create_channel",
        json!({
            "type": "O", "user_ids": [], "display_name": "群",
            "force_create": true
        })
    ));
    assert!(err(
        "im_create_channel",
        json!({
            "type": "O", "user_ids": ["u1", 2], "display_name": "群",
            "force_create": true
        })
    ));
    assert!(err(
        "im_create_channel",
        json!({
            "type": "O", "user_ids": ["u1"], "display_name": "群",
            "force_create": "true"
        })
    ));

    assert!(err(
        "im_make_topic",
        json!({
            "root_id": "r1", "user_ids": ["u1"], "req_id": "topic-request-1",
            "post_id": "legacy"
        })
    ));
    assert!(err(
        "im_make_topic",
        json!({ "root_id": "r1", "user_ids": [], "req_id": "topic-request-1" })
    ));
    assert!(err(
        "im_make_topic",
        json!({ "root_id": "r1", "user_ids": [""], "req_id": "topic-request-1" })
    ));
    assert!(err(
        "im_make_topic",
        json!({ "root_id": "r1", "user_ids": ["u1"] })
    ));
    assert!(err(
        "im_make_topic",
        json!({
            "root_id": "r1", "user_ids": ["u1"],
            "display_name": 42, "req_id": "topic-request-1"
        })
    ));
}

// ── #15 change/props 故意不广播：im_update_channel_props 不该注册 ──────────────────────

#[test]
fn channel_props_change_has_no_outbound_command_15_not_broadcast() {
    // 真源 cses_channel.go:1226-1250 落库 Props **无 a.Publish**（迁移注意点 2）——故 helix
    // **不**为 change/props 造 outbound 语义命令（也勿补 update_channel WS handler 凭空广播）。
    assert!(
        !is_outbound("im_update_channel_props"),
        "#15 change/props 故意不广播 → 不该有 outbound 命令"
    );
    assert!(!is_outbound("im_channel_change_props"));
}

// ── P0-2 系统消息消费编排核验（已在 main 落地，本 Phase 不重造，验关键不变量）────────────

/// 他人 `post` 帧（真源 action=="post"，event_seq 在 data 顶层），用于触发 unread bump。
fn other_user_post_frame(channel_id: u64, seq: u64, post_id: u64) -> Tick {
    let payload = json!({
        "v": 1,
        "action": "post",
        "data": {
            "id": sid(post_id),
            "channelId": cid(channel_id),
            "event_seq": seq,
            "message": "他人消息",
            "type": "P",
            "createAt": 1000,
            "userId": "00000000000000000000000099",  // 他人（非 echo，无 temporary_id）
            "viewers": ["all"],
        },
        "seq": 1
    });
    Tick::Inbound(InboundBytes(serde_json::to_vec(&payload).unwrap().into()))
}

/// **反 RMW 性质（B5 / HX-C005）**：他人 post 帧 → 当前 viewer 的未读走单条
/// `ScopedGuardedBump(channel_member, channel_id, user_id)`，不先读绝对值再写回。
///
/// 可证伪：若把 unread +1 改成「BatchUpsert 全列覆盖」或先 Get 再 BatchUpdate 绝对值 → 本测试
/// 找不到复合键 GuardedBump（或找到读后绝对值覆盖）→ FAIL。
#[test]
fn post_unread_bump_is_single_guarded_bump_not_rmw_b5() {
    // 注入与消息作者不同的 viewer；MessageV3 原子写明确要求运行时身份。
    let mut h = Harness::new_with_identity(&[1], "viewer-user", "company", "", "", "");
    h.step(other_user_post_frame(1, 1, 7), 1000);

    // 收集本批所有 StorageOp（包含 MessageV3 原子写屏障）。
    let mut guarded_bumps = Vec::new();
    let mut read_before_bump = false;
    let mut absolute_unread_updates = 0;
    for e in h.sink.as_slice() {
        let ops: &[StorageOp] = match e {
            Effect::Persist { ops, .. } => ops,
            Effect::PersistAtomic { ops, .. } => ops,
            Effect::PersistFire { ops } => ops,
            _ => &[],
        };
        for op in ops {
            match op {
                StorageOp::ScopedGuardedBump(spec) if spec.table == "channel_member" => {
                    guarded_bumps.push(spec.clone());
                }
                StorageOp::Get(spec) if spec.table == "channel_member" => {
                    read_before_bump = true;
                }
                StorageOp::ScopedGet(spec) if spec.table == "channel_member" => {
                    read_before_bump = true;
                }
                StorageOp::BatchUpdate(spec) if spec.table == "channel_member" => {
                    absolute_unread_updates += usize::from(
                        spec.patch
                            .iter()
                            .any(|(column, _)| column == "unread_count"),
                    );
                }
                _ => {}
            }
        }
    }

    assert_eq!(
        guarded_bumps.len(),
        1,
        "他人 post → 恰 1 条 channel_member ScopedGuardedBump（unread +1）"
    );
    let bump = &guarded_bumps[0];
    assert_eq!(bump.scope_col, "channel_id");
    assert!(
        matches!(&bump.scope_val, helix_core::effect::SqlValue::Text(value) if value == &cid(1))
    );
    assert_eq!(bump.key_col, "user_id");
    assert_eq!(bump.bump_col, "unread_count", "自增列 = unread_count");
    assert_eq!(bump.bump_delta, 1, "他人可见消息 +1");
    assert_eq!(
        bump.guard_col, "last_unread_event_seq",
        "前进守卫列 = last_unread_event_seq（按频道事件序列防重复/乱序回退）"
    );
    assert!(
        bump.set_cols.iter().any(|(column, value)| {
            column == "last_post_at"
                && matches!(
                    value,
                    helix_core::effect::SqlValue::Integer(timestamp)
                        if *timestamp == 1000
                )
        }),
        "可见消息必须同步推进 last_post_at，确保恢复后的会话进入有界首屏并刷新预览"
    );
    assert!(
        !read_before_bump,
        "unread bump 路径禁 Get-before-write（RMW 反模式：读出 +1 写回）"
    );
    assert_eq!(
        absolute_unread_updates, 0,
        "未读递增不得退化为 channel_member 绝对值 BatchUpdate"
    );
}

/// channel **写命令**（outbound）只触发 HTTP，**不自拼 post 帧 / 不产 Persist**（DDD 聚合根解耦）。
///
/// 真源：系统加群/退群 post 帧由服务端推回（channel_member_update + post），helix channel
/// 命令只发 HTTP；落库由后续 WS `post` 帧经 gate 走（聚合根 = channel gate，外部经它改状态）。
///
/// 可证伪：若某 channel outbound 命令在 build 阶段产 Persist / 自造 post → effects 含非 Http → FAIL。
#[test]
fn channel_write_commands_only_fire_http_no_self_fabricated_post() {
    for (name, mk) in ALL_P3B_CHANNEL {
        let effects = run(name, mk());
        assert_eq!(
            effects.len(),
            1,
            "{name} 应只产单条 Effect（不自拼 post / 不落库）"
        );
        assert!(
            matches!(effects[0], Effect::Http { .. }),
            "{name} 应只产 Effect::Http（DDD 聚合根解耦：channel 命令只触发，落库走 WS 回帧）"
        );
    }
}

// ── registry 性质：P3b channel 新增命令全 is_outbound 命中 + 全可 dispatch ──────────────

/// P3b channel 写族**新增**命令全集（8：createSpecifyOwner + change×5 + member×2）。
/// 改这个列表必须同步 channel_create/channel_change/channel_member_admin 注册文件。
const ALL_P3B_CHANNEL: &[(&str, fn() -> Value)] = &[
    (
        "im_create_specify_owner",
        || json!({ "displayName": "g", "users": [{ "id": "u" }] }),
    ),
    (
        "im_channel_change_info",
        || json!({ "channel_id": "c", "display_name": "n" }),
    ),
    (
        "im_channel_change_source",
        || json!({ "channel_id": "c", "title": "t" }),
    ),
    (
        "im_channel_change_picture",
        || json!({ "channel_id": "c", "picture_type": "USER", "picture": {} }),
    ),
    (
        "im_channel_change_notice",
        || json!({ "channel_id": "c", "notice": { "text": "x" } }),
    ),
    (
        "im_channel_change_top",
        || json!({ "channel_id": "c", "top": true }),
    ),
    (
        "im_channel_member_role",
        || json!({ "channel_id": "c", "user_ids": ["u"], "role": "ADMIN" }),
    ),
    (
        "im_channel_member_notify",
        || json!({ "channel_id": "c", "notify": "NORMAL" }),
    ),
];

#[test]
fn registry_covers_all_p3b_channel_commands() {
    for (name, _) in ALL_P3B_CHANNEL {
        assert!(is_outbound(name), "{name} 应被 registry 认领");
    }
    // 既有 P2 channel 命令仍在册（迁移未丢）。
    assert!(is_outbound("im_create_channel"));
    assert!(is_outbound("im_make_topic"));
    assert!(is_outbound("im_channel_close"));
    assert!(is_outbound("im_channel_member_change"));
    // 非 channel 命令不被误纳。
    assert!(!is_outbound("im_send_message"));
    assert!(!is_outbound("bogus_channel"));
}

/// 注册数==命令数性质：每个 P3b 新增命令都能成功 dispatch（无重复 name 致 registry 构建失败）。
#[test]
fn every_p3b_channel_command_is_dispatchable() {
    assert_eq!(ALL_P3B_CHANNEL.len(), 8, "P3b channel 新增应为 8 命令");
    for (name, mk) in ALL_P3B_CHANNEL {
        let effects = run(name, mk());
        assert_eq!(effects.len(), 1, "{name} 应产单条 Effect::Http");
        assert!(
            matches!(effects[0], Effect::Http { .. }),
            "{name} 应产 Effect::Http"
        );
    }
}

/// handle_outbound 错误闭包路径（边界零信任，不 panic）：坏 JSON payload。
#[test]
fn handle_outbound_bad_payload_is_error_not_panic() {
    assert!(handle_outbound(
        "im_channel_change_top",
        b"{not json",
        BASE,
        BASE,
        None,
        corr()
    )
    .is_err());
}
