//! C2 强制对账：`im:__bus__` 投影 emit payload 字段集冻结测试。
//!
//! 回归锚: C2（接管架构 01 文档 §2「纯渲染证伪标准」）
//!
//! ## 这测什么（为什么红=纯渲染破功）
//!
//! 前端退化为纯渲染：只 `listen("im:__bus__")` → dispatch → 渲染，**不算** unread/lastPost/
//! readBits/cursor。前端能不能纯渲染，取决于**每个 channel 的 `payload.data` 字段是否齐**——
//! 字段缺 = 前端补算 = 纯渲染破功。本测试把 helix 全部 23 个 emit 工厂的 `data` 键集
//! **逐 channel 冻结**，断言 emit 出的键集 == `docs/projection-schema.md` §1 冻结集
//! （**缺失或多余都 fail**）。
//!
//! ## 真源镜像铁律
//!
//! 本文件是 `docs/projection-schema.md` 的**可执行镜像**：改任一工厂的 data 键集，
//! 必须同步改文档冻结表 + 本测试；测试红 = 漂移未同步，**禁直接改测试迁就代码**
//! （先确认是有意契约变更）。
//!
//! ## message-row fat vs channel-row thin vs 透传类（§2 · spec06 FixB 重写）
//!
//! - **message-row fat 类**（im:post:received/updated/deleted/read / im:channel:read_echo）：携**完整
//!   可渲染 MessageItemData 核心字段**（`message_item_data` 产物，数据在手零回读）。断言**精确等于**
//!   完整集 `FAT_KEYS`——**缺**任一字段 = 前端回退「半份回读」蠢模式（缺陷B）→ fail（反转旧契约：旧
//!   「多一即漂移」，新「**少一即漂移**」）。
//! - **channel-row thin 类**（im:channel:update / update-by-post）：**故意留瘦**（完整 DialogItem 是累计
//!   DB 行非在手，fat 会触发每事件热路径回读，违 SPEC §2 铁律）→ 断言瘦键集。
//! - **透传类**（im:channel:increment/created 等）：断言外层包装键集（载荷内部 wire 字段由现网契约保真）。

mod common;
use common::channel_id;

use helix_core::Effect;
use helix_im::acl::{to_effect, to_effect_s1};
use helix_im::query;
use helix_im::state::{ChannelId, Seq};
use helix_im::sync_session::PostFields;

/// spec06 FixB：message-row 信号的**完整可渲染键集**（= `to_effect::message_item_data` 产物）。
/// 信号锚 snake（channel_id/event_seq/msg_id）+ canonical render-ready camel aliases
///（id/msgId/eventSeq/text/createdAt）+ 兼容现网 MessageItemData 字段。
/// 缺任一 → message-row 信号断言 fail（前端回退回读蠢模式 = 缺陷B 复发）。
const FAT_KEYS: &[&str] = &[
    "channel_id",
    "event_seq",
    "msg_id",
    "id",
    "msgId",
    "temporaryId",
    "channelId",
    "eventSeq",
    "userId",
    "type",
    "message",
    "text",
    "props",
    "createAt",
    "createdAt",
    "updateAt",
    "readBits",
    "viewers",
    // —— render-ready 终态字段（issue #53·C013 业务规则下沉 helix·前端纯绑定）——
    "sendStatus",
    "serverId",
    "isSelf",
    "actionCapabilities",
    "reactions",
    "reactionCount",
    "urgent",
    "urgentTargetCount",
    "urgentConfirmedCount",
    "expediteMap",
    "templateReceived",
    "templateConfirmedCount",
    "readerIds",
    "systemNotice",
    "pinned",
    "topic",
    "replyId",
    "replyRootId",
    "replyFirstLevelId",
    "repliedMessage",
    "replyMessages",
    "replyCount",
];

/// 构造一份非空 `PostFields`（fat emit 在手数据样本）——字段非空，证明 fat 工厂真把在手数据吐出。
fn sample_fields() -> PostFields {
    PostFields {
        temporary_id: TMP_ID.to_string(),
        id: MSG_ID.to_string(),
        channel_id: "c".to_string(),
        user_id: "u_sender".to_string(),
        msg_type: "TEXT".to_string(),
        message: "hello".to_string(),
        simple_message: "hello".to_string(),
        props: r#"{"k":"v"}"#.to_string(),
        user_snapshot: r#"{"userId":"u_sender"}"#.to_string(),
        create_at: 1_700_000_000,
        read_bits: "11".to_string(),
        viewers: vec!["all".to_string()],
        mentions: vec!["u_me".to_string()],
        expedite_map: r#"{"u_me":true}"#.to_string(),
        topic: r#"{"channelId":"topic-1"}"#.to_string(),
        reply_id: "root-1".to_string(),
        reply_root_id: "root-1".to_string(),
        reply_first_level_id: "first-1".to_string(),
        replied_message: r#"{"id":"root-1","message":"原消息"}"#.to_string(),
        reply_messages: r#"[{"id":"reply-1","message":"回复"}]"#.to_string(),
        reply_count: 1,
    }
}

fn sample_post_update() -> helix_im::channel_write::PostChannelUpdate {
    helix_im::channel_write::post_updates_from_fields(ch(), &sample_fields(), "u_me")
}

fn sample_dialog_projection() -> helix_im::dialog_patch::DialogPatchProjection {
    helix_im::dialog_patch::DialogPatchProjection {
        unread_count: 3,
        mention_count: 1,
        urgent_count: 1,
        mention_list: vec![MSG_ID.to_string()],
        urgent_post_list: vec![MSG_ID.to_string()],
        unread_post_id: Some(MSG_ID.to_string()),
        last_root_post_at: 1_700_000_000,
    }
}

// ─── 提取工具：从 Emit Effect 取 `data` 对象的键集 ────────────────────────────

/// 从 `Effect::Emit` 解出 `{event, data}`，返回 (event_name, data_keys_sorted)。
/// 非 Emit / 非法 JSON / data 非对象 → None（调用方 `.expect`，覆盖测试构造保证命中）。
fn emit_event_and_data_keys(e: &Effect) -> Option<(String, Vec<String>)> {
    let Effect::Emit { event } = e else {
        return None;
    };
    let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).ok()?;
    let name = v.get("event")?.as_str()?.to_string();
    let data = v.get("data")?.as_object()?;
    let mut keys: Vec<String> = data.keys().cloned().collect();
    keys.sort();
    Some((name, keys))
}

/// 断言 emit 工厂产出的 channel + data 键集 == 冻结期望（缺失/多余即 fail）。
///
/// `expected_keys` 无序传入，内部排序后逐字比对——**精确集合相等**（非子集）。
fn assert_frozen(effect: &Effect, expected_channel: &str, expected_keys: &[&str]) {
    let (channel, keys) =
        emit_event_and_data_keys(effect).expect("工厂必产 Effect::Emit{event:{event,data:{}}}");
    assert_eq!(
        channel, expected_channel,
        "channel 名漂移：projection-schema.md §1 冻结 {expected_channel}，实得 {channel}"
    );
    let mut want: Vec<String> = expected_keys.iter().map(|s| s.to_string()).collect();
    want.sort();
    assert_eq!(
        keys, want,
        "channel {expected_channel} 的 data 键集漂移：冻结 {want:?}，实得 {keys:?}\
         （缺失=前端补算=纯渲染破功；多余=可能塞半份 DialogItem 覆盖整份）"
    );
}

// 测试用固定参数（Id26 由 common::channel_id 产合法 26 字符）。
const MSG_ID: &str = "msg0000000000000000000001";
const TMP_ID: &str = "tmp0000000000000000000001";
const SEQ: u64 = 42;

fn ch() -> ChannelId {
    channel_id(7)
}

// ════════════════════════════════════════════════════════════════════════════
// §1a. message-row fat 类（精确 = FAT_KEYS 完整集）—— spec06 FixB 核心证伪点
//      缺任一字段 = 前端回退回读蠢模式（缺陷B）→ fail（反转旧「多一即漂移」契约）。
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn im_post_received_freezes_fat_renderable_row() {
    // FixB 核心：前端 fat 直渲，零回读；完整集齐才能渲染，缺一字段=漂移（回退回读）。
    let f = sample_fields();
    let e = to_effect::emit_post_received(ch(), SEQ, MSG_ID, &f);
    assert_frozen(&e, "im:post:received", FAT_KEYS);
}

#[test]
fn im_post_read_freezes_fat_renderable_row() {
    let f = sample_fields();
    let e = to_effect::emit_post_read(ch(), SEQ, MSG_ID, &f);
    let mut keys = FAT_KEYS.to_vec();
    keys.push("receiptRevision");
    assert_frozen(&e, "im:post:read", &keys);
}

#[test]
fn im_channel_read_echo_freezes_fat_renderable_row() {
    let f = sample_fields();
    let e = to_effect::emit_channel_read_echo(ch(), SEQ, MSG_ID, &f);
    assert_frozen(&e, "im:channel:read_echo", FAT_KEYS);
}

#[test]
fn im_post_updated_freezes_fat_renderable_row() {
    let f = sample_fields();
    let e = to_effect::emit_post_updated(ch(), SEQ, MSG_ID, &f);
    assert_frozen(&e, "im:post:updated", FAT_KEYS);
}

#[test]
fn im_post_deleted_freezes_fat_renderable_row() {
    // 撤回路径 PostFields 通常稀疏，但 fat 形状一致（同 FAT_KEYS）——前端按 msg_id 标删，字段空串无碍。
    let f = sample_fields();
    let e = to_effect::emit_post_deleted(ch(), SEQ, MSG_ID, &f);
    assert_frozen(&e, "im:post:deleted", FAT_KEYS);
}

/// 撤回稀疏 PostFields（default）→ fat 形状仍 == FAT_KEYS（键集不随内容稀疏而缺）。
#[test]
fn im_post_deleted_sparse_fields_still_fat_shape() {
    let e = to_effect::emit_post_deleted(ch(), SEQ, MSG_ID, &PostFields::default());
    assert_frozen(&e, "im:post:deleted", FAT_KEYS);
}

/// fat 工厂真把**在手数据**吐出（非空字段透传）——证 message_item_data 不是空壳骨架。
#[test]
fn fat_emit_carries_in_hand_data() {
    let f = sample_fields();
    let e = to_effect::emit_post_received(ch(), SEQ, MSG_ID, &f);
    let Effect::Emit { event } = &e else {
        panic!("必产 Emit")
    };
    let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let data = &v["data"];
    assert_eq!(data["userId"], "u_sender", "userId 来自在手 PostFields");
    assert_eq!(data["message"], "hello", "message 来自在手 PostFields");
    assert_eq!(data["id"], MSG_ID, "id 使用服务端消息锚");
    assert_eq!(data["msgId"], MSG_ID, "msgId 与 id 使用同一服务端消息锚");
    assert_eq!(data["eventSeq"], SEQ, "eventSeq 与 event_seq 同源");
    assert_eq!(data["text"], "hello", "text 与 message 同源");
    assert_eq!(data["readBits"], "11", "readBits 来自在手 PostFields");
    assert_eq!(
        data["createAt"], 1_700_000_000,
        "createAt 来自在手 PostFields"
    );
    assert_eq!(
        data["createdAt"], 1_700_000_000,
        "createdAt 与 createAt 同源"
    );
    // props 由 JSON 字符串 parse 回对象（前端 props:any 期望对象，非字符串）。
    assert_eq!(data["props"]["k"], "v", "props parse 回对象");
    assert_eq!(data["topic"]["channelId"], "topic-1", "topic parse 回对象");
    assert_eq!(data["viewers"][0], "all", "viewers 数组直出");
    assert_eq!(data["replyId"], "root-1");
    assert_eq!(data["replyRootId"], "root-1");
    assert_eq!(data["replyFirstLevelId"], "first-1");
    assert_eq!(data["repliedMessage"]["message"], "原消息");
    assert_eq!(data["replyMessages"][0]["message"], "回复");
    assert_eq!(data["replyCount"], 1);
    // render-ready 终态字段（issue #53）：sample_fields 无 quickReply/template·type=TEXT →
    // sendStatus=sent / reactions=null / templateReceived=false / systemNotice=false。
    assert_eq!(data["sendStatus"], "sent", "sendStatus 恒 sent（已对账态）");
    assert_eq!(data["serverId"], MSG_ID, "serverId 使用服务端消息锚");
    assert_eq!(
        data["isSelf"], false,
        "无 viewer 身份时 fail-closed 为非本人"
    );
    assert_eq!(data["actionCapabilities"]["canForward"], true);
    assert_eq!(data["actionCapabilities"]["canLocate"], true);
    assert_eq!(data["actionCapabilities"]["canRevoke"], false);
    assert!(
        data["reactions"].is_null(),
        "无 quickReply → reactions=null"
    );
    assert_eq!(data["urgent"], true, "非空 expedite_map → urgent=true");
    assert_eq!(
        data["templateReceived"], false,
        "无 template → templateReceived=false"
    );
    assert_eq!(data["templateConfirmedCount"], 0);
    assert_eq!(data["urgentTargetCount"], 0);
    assert_eq!(data["urgentConfirmedCount"], 0);
    assert_eq!(
        data["systemNotice"], false,
        "type=TEXT 非 NOTICE → systemNotice=false"
    );
    assert!(data["pinned"].is_null(), "无 pinned props → pinned=null");
}

/// render-ready 字段真按业务数据派生（非空壳常量）——quickReply→reactions / template→templateReceived /
/// NOTICE type→systemNotice 三条下沉规则可证伪（issue #53·C013 业务下沉 helix 的回归锚）。
#[test]
fn render_ready_fields_derive_from_business_data() {
    let mut f = sample_fields();
    f.props =
        r#"{"quickReply":[{"emoji":"👍","userIds":["u1"]}],"template":{"userIds":["u_self"]}}"#
            .to_string();
    f.msg_type = "NOTICE".to_string();
    let e = to_effect::emit_post_received(ch(), SEQ, MSG_ID, &f);
    let Effect::Emit { event } = &e else {
        panic!("必产 Emit")
    };
    let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let data = &v["data"];
    assert_eq!(
        data["reactions"], "👍",
        "quickReply.emoji → reactions 预解串"
    );
    assert_eq!(
        data["templateReceived"], true,
        "template.userIds 非空 → templateReceived=true"
    );
    assert_eq!(
        data["reactionCount"], 1,
        "reactionCount 必须是绝对 userIds 总数"
    );
    assert_eq!(
        data["templateConfirmedCount"], 1,
        "模板确认必须输出绝对计数"
    );
    assert_eq!(
        data["systemNotice"], true,
        "type=NOTICE ∈ NOTICE_TYPES → systemNotice=true"
    );
    assert!(data["pinned"].is_null(), "无 pinned props → pinned=null");
    assert_eq!(data["type"], "NOTICE", "NOTICE type 原样透传");
}

#[test]
fn message_action_capabilities_are_viewer_relative_and_fail_closed() {
    let mut f = sample_fields();
    f.user_id = "viewer-444".to_string();
    let e = to_effect::emit_post_received_for_viewer(ch(), SEQ, MSG_ID, &f, "viewer-444");
    let Effect::Emit { event } = e else {
        panic!("必产 Emit")
    };
    let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let data = &v["data"];
    assert_eq!(data["isSelf"], true);
    assert_eq!(data["actionCapabilities"]["canRevoke"], true);
    assert_eq!(data["actionCapabilities"]["canRetry"], false);
    assert_eq!(data["actionCapabilities"]["canViewReceipt"], true);
    assert_eq!(data["actionCapabilities"]["canMarkRead"], false);
    assert_eq!(data["actionCapabilities"]["canLocate"], true);
}

#[test]
fn revoked_message_does_not_advertise_locate() {
    let f = sample_fields();
    let e = to_effect::emit_post_deleted(ch(), SEQ, MSG_ID, &f);
    let Effect::Emit { event } = e else {
        panic!("必产 Emit")
    };
    let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(value["data"]["actionCapabilities"]["canLocate"], false);
}

#[test]
fn message_interaction_capabilities_use_viewer_facts() {
    let mut f = sample_fields();
    f.user_id = "sender".to_string();
    f.props = r#"{"template":{"userIds":[]}}"#.to_string();
    f.expedite_map = r#"{"recipients":{"viewer-678":{"status":0}}}"#.to_string();
    let e = to_effect::emit_post_received_for_viewer(ch(), SEQ, MSG_ID, &f, "viewer-678");
    let Effect::Emit { event } = e else {
        panic!("必产 Emit")
    };
    let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let capabilities = &value["data"]["actionCapabilities"];
    assert_eq!(capabilities["canUrgentConfirm"], true);
    assert_eq!(capabilities["canMarkRead"], true);
    assert_eq!(capabilities["canViewReceipt"], false);
    assert_eq!(capabilities["canTemplateConfirm"], true);
}

#[test]
fn post_updated_keeps_viewer_capabilities_and_template_reader_ids() {
    let mut f = sample_fields();
    f.user_id = "sender".to_string();
    f.props = r#"{"template":{"userIds":["viewer-678"]}}"#.to_string();
    f.expedite_map = r#"{"recipients":{"viewer-678":{"status":0}}}"#.to_string();
    let e = to_effect::emit_post_updated_for_viewer(ch(), SEQ, MSG_ID, &f, "viewer-678");
    let Effect::Emit { event } = e else {
        panic!("必产 Emit")
    };
    let value: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(
        value["data"]["readerIds"],
        serde_json::json!(["viewer-678"])
    );
    assert_eq!(
        value["data"]["actionCapabilities"]["canUrgentConfirm"],
        true
    );
    assert_eq!(
        value["data"]["actionCapabilities"]["canTemplateConfirm"],
        false
    );
}

#[test]
fn render_ready_pinned_derives_from_props() {
    let mut f = sample_fields();
    f.props = r#"{"pinned":true}"#.to_string();
    let e = to_effect::emit_post_updated(ch(), SEQ, MSG_ID, &f);
    let Effect::Emit { event } = &e else {
        panic!("必产 Emit")
    };
    let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(v["data"]["pinned"], true, "props.pinned → data.pinned");
}

#[test]
fn render_ready_urgent_is_false_for_empty_expedite_map() {
    let mut f = sample_fields();
    f.expedite_map = "{}".to_string();
    let e = to_effect::emit_post_updated(ch(), SEQ, MSG_ID, &f);
    let Effect::Emit { event } = &e else {
        panic!("必产 Emit")
    };
    let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(v["data"]["urgent"], false, "空 expedite_map → urgent=false");
}

/// type 空串容错 "TEXT"（render-ready 下沉·前端不再 `d.type || "TEXT"` 兜底·issue #53）。
#[test]
fn empty_type_defaults_to_text() {
    let mut f = sample_fields();
    f.msg_type = String::new();
    let e = to_effect::emit_post_received(ch(), SEQ, MSG_ID, &f);
    let Effect::Emit { event } = &e else {
        panic!("必产 Emit")
    };
    let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(v["data"]["type"], "TEXT", "空 type → TEXT");
    assert_eq!(v["data"]["systemNotice"], false, "TEXT 默认非系统通知");
}

// ════════════════════════════════════════════════════════════════════════════
// §1b. channel-row DialogPatch 类（保留 legacy anchor，并补 render-ready patch）
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn im_channel_update_by_post_is_render_ready_dialog_patch() {
    let f = sample_fields();
    let update = sample_post_update();
    let projection = sample_dialog_projection();
    let e = to_effect::emit_channel_update_by_post(
        ch(),
        SEQ,
        MSG_ID,
        &f,
        &update,
        &projection,
        "online_post",
    );
    assert_frozen(
        &e,
        "im:channel:update-by-post",
        &[
            "op",
            "source",
            "channel_id",
            "channelId",
            "event_seq",
            "eventSeq",
            "msg_id",
            "msgId",
            "messageClass",
            "visible",
            "senderUserId",
            "lastPost",
            "lastMessage",
            "unreadDelta",
            "mentionDelta",
            "urgentDelta",
            "mentionedUserIds",
            "urgentUserIds",
            "mentionList",
            "urgentPostList",
            "dialogPatch",
        ],
    );
    let Effect::Emit { event } = &e else {
        panic!("projection must emit JSON");
    };
    let value: serde_json::Value =
        serde_json::from_slice(event.0.as_ref()).expect("projection JSON");
    assert_eq!(value["data"]["dialogPatch"]["eventSeq"], SEQ);
    assert_eq!(value["data"]["dialogPatch"]["lastEventSeq"], SEQ);
}

#[test]
fn im_channel_update_freezes_channel_id_only() {
    // 批次结束信号：只携 channel_id，前端 batch 结束（debounced，非每消息）回读 dialogList 刷 badge。
    let e = to_effect::emit_channel_update(ch());
    assert_frozen(&e, "im:channel:update", &["channel_id"]);
}

#[test]
fn im_post_sending_freezes_channel_and_temporary_id() {
    // 乐观上屏：temporary_id 是 message 表 PK（前端按它定位乐观行）。S6（issue #55·C013）补
    // render-ready 乐观终态行字段（msgId/temporaryId/channelId/eventSeq/sendStatus/readBits/text/type）→
    // 前端 applyPostSending 退纯绑定（不再取 pendingText/pendingType）。snake 锚 channel_id/temporary_id 保留。
    let e = to_effect::emit_post_sending(ch(), TMP_ID, "hello", "TEXT");
    assert_frozen(
        &e,
        "im:post:sending",
        &[
            "channel_id",
            "temporary_id",
            "msgId",
            "temporaryId",
            "channelId",
            "eventSeq",
            "sendStatus",
            "serverId",
            "isSelf",
            "actionCapabilities",
            "readBits",
            "text",
            "type",
            "replyId",
            "replyRootId",
            "replyFirstLevelId",
            "replyCount",
            "replyMessages",
        ],
    );
}

#[test]
fn im_post_send_failed_freezes_temporary_id_failed_patch() {
    // HTTP PortError 真实失败路径：按 temporaryId 锚定乐观行，UI 只直绑 sendStatus="failed"。
    let channel_id = ch();
    let e = to_effect::emit_post_send_failed(channel_id.as_str(), TMP_ID);
    assert_frozen(
        &e,
        "im:post:send-failed",
        &[
            "channelId",
            "sendStatus",
            "temporaryId",
            "serverId",
            "isSelf",
            "actionCapabilities",
        ],
    );
}

// ════════════════════════════════════════════════════════════════════════════
// §2. wire camelCase 信号/控制类（透传现网 emit payload 形态）
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn im_sync_too_long_freezes_channelid_resetto_camelcase() {
    // 命名陷阱（§6）：channelId/resetTo 是 camelCase（对齐现网 post.rs:918），非 snake。
    let e = to_effect::emit_sync_too_long(ch(), Seq(100));
    assert_frozen(
        &e,
        "im:sync:too_long",
        &["channelId", "resetTo", "mode", "dropLocalMessages"],
    );
}

#[test]
fn im_connection_established_freezes_connectionid() {
    let e = to_effect::emit_connection_established("conn-abc");
    assert_frozen(&e, "im:connection:established", &["connectionId"]);
}

#[test]
fn im_channel_closed_freezes_channelid_deleteat_camelcase() {
    let e = to_effect_s1::emit_channel_closed(ch(), 1_700_000_000);
    assert_frozen(
        &e,
        "im:channel:closed",
        &["channelId", "closed", "deleteAt", "id"],
    );
}

#[test]
fn im_channel_left_is_distinct_from_closed() {
    let e = to_effect_s1::emit_channel_left(ch());
    assert_frozen(&e, "im:channel:left", &["channelId", "id", "left"]);
    let Effect::Emit { event } = e else {
        panic!("必须 emit")
    };
    let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(payload["data"]["left"], true);
    assert!(payload["data"].get("closed").is_none());
}

#[test]
fn im_channel_member_nickname_freezes_camelcase_triplet() {
    let e = to_effect_s1::emit_member_nickname(ch(), "u123", "小明");
    assert_frozen(
        &e,
        "im:channel:memberNickname",
        &["channelId", "nickName", "userId"],
    );
}

#[test]
fn im_channel_settings_updated_freezes_render_ready_settings_and_version() {
    let e = to_effect_s1::emit_channel_settings_updated(
        ch(),
        &serde_json::json!({"displayName":"新群名","purpose":"简介"}),
        77,
        serde_json::json!({"canCloseChannel":false}),
    )
    .expect("存在设置字段应 emit");
    assert_frozen(
        &e,
        "im:channel:settings-updated",
        &[
            "channelCapabilities",
            "channelId",
            "settings",
            "settingVersion",
        ],
    );
}

#[test]
fn im_schedule_created_freezes_render_ready_schedule_fields() {
    let e = to_effect_s1::emit_schedule_created(
        ch(),
        &serde_json::json!({
            "scheduleId": "schedule-1",
            "sendAt": 1_700_000_000_000_i64,
            "status": "scheduled",
            "revision": 7,
            "messagePreview": "稍后发送"
        }),
        None,
        true,
    );
    assert_frozen(
        &e,
        "im:channel:schedule-created",
        &[
            "channelId",
            "hasSchedulePost",
            "id",
            "messagePreview",
            "revision",
            "scheduleId",
            "sendAt",
            "status",
        ],
    );
}

#[test]
fn im_schedule_canceled_freezes_render_ready_schedule_fields_without_preview_leak() {
    let e = to_effect_s1::emit_schedule_canceled(
        ch(),
        &serde_json::json!({
            "schedule_id": "schedule-1",
            "send_at": 1_700_000_000_000_i64,
            "revision": 8,
            "message_preview": "不得泄露"
        }),
        None,
        false,
    );
    assert_frozen(
        &e,
        "im:channel:schedule-canceled",
        &[
            "channelId",
            "hasSchedulePost",
            "id",
            "messagePreview",
            "revision",
            "scheduleId",
            "sendAt",
            "status",
        ],
    );
    let Effect::Emit { event } = e else {
        panic!("必须 emit")
    };
    let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert!(payload["data"]["messagePreview"].is_null());
}

// ════════════════════════════════════════════════════════════════════════════
// §3. 透传/投影类（外层包装键集冻结；载荷内部 wire 字段由现网契约保真）
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn im_channel_increment_freezes_channel_id_and_increment() {
    let raw = serde_json::to_vec(&serde_json::json!({"mentionList": ["a"]})).unwrap();
    let e = to_effect::emit_channel_increment(ch(), &raw);
    assert_frozen(&e, "im:channel:increment", &["channel_id", "increment"]);
}

#[test]
fn im_channel_increment_invalid_raw_still_freezes_keys() {
    // 边界零信任：非法 JSON raw → increment 降级 null，键集不变（不 panic）。
    let e = to_effect::emit_channel_increment(ch(), b"not json{");
    assert_frozen(&e, "im:channel:increment", &["channel_id", "increment"]);
}

#[test]
fn im_channel_created_freezes_render_ready_dialog_and_wire_payload() {
    let raw = serde_json::to_vec(&serde_json::json!({
        "displayName": "群",
        "memberCount": 2,
        "type": "T"
    }))
    .unwrap();
    let e = to_effect_s1::emit_channel_created(ch(), &raw);
    assert_frozen(
        &e,
        "im:channel:created",
        &[
            "channel",
            "channelId",
            "channel_id",
            "displayName",
            "group",
            "id",
            "memberCount",
            "props",
        ],
    );
    let Effect::Emit { event } = e else {
        panic!("必产 Emit")
    };
    let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(payload["data"]["displayName"], "群");
    assert_eq!(payload["data"]["group"], "topic");
    assert_eq!(payload["data"]["memberCount"], 2);
    assert_eq!(payload["data"]["props"]["type"], "T");
}

#[test]
fn im_channel_member_updated_freezes_channel_id_and_channel() {
    let raw = serde_json::to_vec(&serde_json::json!({"memberChange": {}})).unwrap();
    let e = to_effect_s1::emit_channel_member_updated(ch(), &raw);
    assert_frozen(&e, "im:channel:member-updated", &["channel", "channel_id"]);
}

#[test]
fn im_post_batch_updated_freezes_channel_id_and_posts() {
    let posts = serde_json::json!([{"id": "p1"}, {"id": "p2"}]);
    let e = to_effect::emit_post_batch_updated(ch(), &posts);
    assert_frozen(&e, "im:post:batch-updated", &["channel_id", "posts"]);
}

#[test]
fn im_post_batch_updated_non_array_posts_degrades_to_empty_keeps_keys() {
    // posts 非数组 → 降级空数组，键集不变（健壮性，对齐工厂实现）。
    let posts = serde_json::json!({"not": "array"});
    let e = to_effect::emit_post_batch_updated(ch(), &posts);
    assert_frozen(&e, "im:post:batch-updated", &["channel_id", "posts"]);
}

#[test]
fn im_channel_replies_freezes_explicit_merge_and_paging_contract() {
    let projection = helix_im::render_ready_replies::FlatReplyProjection {
        channel_id: "c1".to_string(),
        root_message_id: "root".to_string(),
        projection_mode: helix_im::render_ready_replies::ReplyProjectionMode::Snapshot,
        revision: 7,
        reply_ids: vec!["r1".to_string()],
        nodes: vec![serde_json::json!({"msgId":"r1"})],
        reply_count: 1,
        has_more: false,
    };
    let effect = helix_im::render_ready_replies::emit_channel_replies("req-1", projection);
    assert_frozen(
        &effect,
        "im:channel:replies",
        &[
            "reqId",
            "channelId",
            "rootMessageId",
            "projectionMode",
            "revision",
            "replyIds",
            "nodes",
            "replyCount",
            "cursor",
            "hasMore",
        ],
    );
}

#[test]
fn im_messages_query_result_freezes_channel_id_and_messages() {
    // C2 必测核心之三：投影命令结果（前端直接渲染 messages 数组）。
    let cid = channel_id(7);
    let rows = serde_json::to_vec(&serde_json::json!([
        {"temporary_id": "t1", "message": "hi"}
    ]))
    .unwrap();
    let e = query::emit_message_query_result(&cid, &rows);
    assert_frozen(&e, "im:messages:query_result", &["channel_id", "messages"]);
}

#[test]
fn im_messages_query_result_invalid_reply_still_freezes_keys() {
    let cid = channel_id(7);
    let e = query::emit_message_query_result(&cid, b"garbage{");
    assert_frozen(&e, "im:messages:query_result", &["channel_id", "messages"]);
}

#[test]
fn im_channels_projection_freezes_dialoglist() {
    // UC-3：会话列表完整投影（前端零计算 unread/badge，直接渲染 dialogList）。
    let rows = serde_json::to_vec(&serde_json::json!([
        {
            "id": "ch1",
            "display_name": "群A",
            "type": "P",
            "team_id": "team-1",
            "unread_count": 3,
            "mention_count": 1,
            "member_count": 2,
            "last_event_seq": 7,
            "props": "{\"canEdit\":true}"
        }
    ]))
    .unwrap();
    let e = query::emit_dialog_list_result(&rows);
    assert_frozen(&e, "im:channels:projection", &["dialogList"]);
    let Effect::Emit { event } = e else {
        panic!("dialog query 必产 Emit")
    };
    let payload: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    let dialog = &payload["data"]["dialogList"][0];
    assert_eq!(dialog["id"], "ch1");
    assert_eq!(dialog["channelId"], "ch1");
    assert_eq!(dialog["displayName"], "群A");
    assert_eq!(dialog["group"], "group");
    assert_eq!(dialog["teamId"], "team-1");
    assert_eq!(dialog["unreadCount"], 3);
    assert_eq!(dialog["mentionCount"], 1);
    assert_eq!(dialog["memberCount"], 2);
    assert_eq!(dialog["eventSeq"], 7);
    assert_eq!(dialog["props"], serde_json::json!({"canEdit": true}));
}

#[test]
fn im_channels_projection_invalid_reply_still_freezes_keys() {
    let e = query::emit_dialog_list_result(b"garbage{");
    assert_frozen(&e, "im:channels:projection", &["dialogList"]);
}

// ════════════════════════════════════════════════════════════════════════════
// §4. im:channels:loaded 双形态（同 channel 名两工厂，各冻各的键集）
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn im_channels_loaded_hello_path_freezes_items() {
    // hello 冷启动补齐：{items:[]}（列表刷新信号）。
    let e = to_effect::emit_channels_loaded();
    assert_frozen(&e, "im:channels:loaded", &["items"]);
}

#[test]
fn im_channels_loaded_delete_path_freezes_reason() {
    // delete-all-dialogs：{reason}（清库刷新信号）——同 channel 名第二形态。
    let e = query::emit_dialogs_cleared();
    assert_frozen(&e, "im:channels:loaded", &["reason"]);
}

// ════════════════════════════════════════════════════════════════════════════
// §5. 镜像完整性：24 工厂全覆盖 + 提取工具负路径
// ════════════════════════════════════════════════════════════════════════════

/// 冻结全集计数：本测试覆盖的 distinct (工厂调用) 数 == projection-schema.md §1 表行数 == 23。
/// 任何新增 emit 工厂未在本测试加 case → 此断言提示补齐（防漏测漂移）。
#[test]
fn all_24_projection_factories_covered() {
    // 逐工厂构造一次，收集 (channel, keys)——证明 23 个出口全可达且产合法 Emit。
    let cid = channel_id(7);
    let raw = serde_json::to_vec(&serde_json::json!({})).unwrap();
    let posts = serde_json::json!([]);
    let f = sample_fields();
    let effects: Vec<Effect> = vec![
        to_effect::emit_post_received(ch(), SEQ, MSG_ID, &f),
        to_effect::emit_post_sending(ch(), TMP_ID, "", "TEXT"),
        to_effect::emit_post_send_failed(ch().as_str(), TMP_ID),
        to_effect::emit_sync_too_long(ch(), Seq(1)),
        to_effect::emit_connection_established("c"),
        to_effect::emit_channels_loaded(),
        to_effect::emit_channel_increment(ch(), &raw),
        to_effect::emit_channel_update(ch()),
        to_effect::emit_post_read(ch(), SEQ, MSG_ID, &f),
        to_effect::emit_channel_read_echo(ch(), SEQ, MSG_ID, &f),
        to_effect::emit_post_updated(ch(), SEQ, MSG_ID, &f),
        to_effect::emit_post_batch_updated(ch(), &posts),
        to_effect::emit_post_deleted(ch(), SEQ, MSG_ID, &f),
        to_effect::emit_channel_update_by_post(
            ch(),
            SEQ,
            MSG_ID,
            &f,
            &sample_post_update(),
            &sample_dialog_projection(),
            "online_post",
        ),
        to_effect_s1::emit_channel_created(ch(), &raw),
        to_effect_s1::emit_channel_closed(ch(), 0),
        to_effect_s1::emit_channel_member_updated(ch(), &raw),
        to_effect_s1::emit_channel_settings_updated(
            ch(),
            &serde_json::json!({"displayName":"群"}),
            1,
            serde_json::json!({"canCloseChannel":false}),
        )
        .unwrap(),
        to_effect_s1::emit_member_nickname(ch(), "u", "n"),
        to_effect_s1::emit_schedule_created(ch(), &serde_json::json!({}), None, false),
        to_effect_s1::emit_schedule_canceled(ch(), &serde_json::json!({}), None, false),
        query::emit_message_query_result(&cid, b"[]"),
        query::emit_dialog_list_result(b"[]"),
        query::emit_dialogs_cleared(),
    ];
    assert_eq!(
        effects.len(),
        24,
        "projection-schema.md §1 冻结 24 工厂——新增/删除工厂须同步本测试 + 文档表"
    );
    // 全部产合法 Emit（event 非空 + data 是对象）。
    for e in &effects {
        let (name, _keys) =
            emit_event_and_data_keys(e).expect("每个投影工厂必产 Emit{event,data:obj}");
        assert!(name.starts_with("im:"), "channel 必 im: 前缀：{name}");
        assert!(
            !name.starts_with("im:__"),
            "业务 channel 禁撞 im:__ 总线前缀（防自环）：{name}"
        );
    }
    // distinct channel 名 = 23（im:channels:loaded 双工厂复用 → 24 工厂收敛成 23 distinct channel）。
    let mut names: Vec<String> = effects
        .iter()
        .map(|e| emit_event_and_data_keys(e).unwrap().0)
        .collect();
    names.sort();
    names.dedup();
    assert_eq!(
        names.len(),
        23,
        "23 distinct channel（im:channels:loaded 被 hello/delete 两工厂复用，故 24 工厂 → 23 channel）"
    );
}

/// 连接控制信号 `im:net:reconnect_requested`（F1-gap1）——**不计入** 23 投影工厂集（无渲染字段，
/// 走连接生命周期通道，与 driver 侧 im:net:offline 同族）。冻结：channel 名 + data 严格空对象。
#[test]
fn reconnect_requested_is_control_signal() {
    let e = to_effect::emit_reconnect_requested();
    let (name, keys) = emit_event_and_data_keys(&e).expect("控制信号必产 Emit{event,data:{}}");
    assert_eq!(
        name, "im:net:reconnect_requested",
        "控制信号 channel 冻结（连接控制族）"
    );
    assert!(
        keys.is_empty(),
        "reconnect 控制信号 data 必空（无参，非投影渲染字段）：{keys:?}"
    );
    // 守边界：控制信号不撞 im:__ 总线前缀（防自环），也不混进 im:post/im:channel 投影命名空间。
    assert!(!name.starts_with("im:__"), "禁撞总线前缀：{name}");
    assert!(
        name.starts_with("im:net:"),
        "连接控制族必 im:net: 前缀：{name}"
    );
}

/// 提取工具负路径（消除 never-taken 分支 → 覆盖率）：非 Emit Effect → None。
#[test]
fn extractor_returns_none_on_non_emit() {
    // PersistFire 非 Emit → None（覆盖 let-else 假分支）。
    let not_emit = Effect::PersistFire { ops: vec![] };
    assert!(emit_event_and_data_keys(&not_emit).is_none());
}
