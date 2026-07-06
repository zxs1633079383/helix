//! C2 强制对账：`im:__bus__` 投影 emit payload 字段集冻结测试。
//!
//! 回归锚: C2（接管架构 01 文档 §2「纯渲染证伪标准」）
//!
//! ## 这测什么（为什么红=纯渲染破功）
//!
//! 前端退化为纯渲染：只 `listen("im:__bus__")` → dispatch → 渲染，**不算** unread/lastPost/
//! readBits/cursor。前端能不能纯渲染，取决于**每个 channel 的 `payload.data` 字段是否齐**——
//! 字段缺 = 前端补算 = 纯渲染破功。本测试把 helix 全部 24 个 emit 工厂的 `data` 键集
//! **逐 channel 冻结**，断言 emit 出的键集 == `docs/projection-schema.md` §1 冻结集
//! （**缺失或多余都 fail**）。
//!
//! ## 真源镜像铁律
//!
//! 本文件是 `docs/projection-schema.md` 的**可执行镜像**：改任一工厂的 data 键集，
//! 必须同步改文档冻结表 + 本测试；测试红 = 漂移未同步，**禁直接改测试迁就代码**
//! （先确认是有意契约变更）。
//!
//! ## message-row fat vs channel-row patch vs 透传类（§2 · spec06 FixB 重写）
//!
//! - **message-row fat 类**（im:post:received/updated/deleted/read / im:channel:read_echo）：携**完整
//!   可渲染 MessageItemData 核心字段**（`message_item_data` 产物，数据在手零回读）。断言**精确等于**
//!   完整集 `FAT_KEYS`——**缺**任一字段 = 前端回退「半份回读」蠢模式（缺陷B）→ fail（反转旧契约：旧
//!   「多一即漂移」，新「**少一即漂移**」）。
//! - **channel-row 增量类**：`im:channel:update-by-post` 保留瘦兼容信号；`im:channel:changed`
//!   携单会话 patch（写入意图在手，零回读），避免发消息后刷新整份 `im:channels:projection`。
//! - **透传类**（im:channel:increment/created 等）：断言外层包装键集（载荷内部 wire 字段由现网契约保真）。

mod common;
use common::channel_id;

use helix_core::Effect;
use helix_im::acl::{to_effect, to_effect_s1};
use helix_im::query;
use helix_im::state::{ChannelId, Seq};
use helix_im::sync_session::PostFields;

/// spec06 FixB：message-row 信号的**完整可渲染键集**（= `to_effect::message_item_data` 产物）。
/// 信号锚 snake（channel_id/event_seq/msg_id）+ 渲染核 camel（MessageItemData ∩ PostFields）。
/// 缺任一 → message-row 信号断言 fail（前端回退回读蠢模式 = 缺陷B 复发）。
const FAT_KEYS: &[&str] = &[
    "channel_id",
    "event_seq",
    "msg_id",
    "temporaryId",
    "channelId",
    "userId",
    "type",
    "message",
    "props",
    "createAt",
    "updateAt",
    "readBits",
    "viewers",
    // —— render-ready 终态字段（issue #53·C013 业务规则下沉 helix·前端纯绑定）——
    "sendStatus",
    "reactions",
    "templateReceived",
    "systemNotice",
    "pinned",
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
        props: r#"{"k":"v"}"#.to_string(),
        create_at: 1_700_000_000,
        read_bits: "11".to_string(),
        viewers: vec!["all".to_string()],
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
    assert_frozen(&e, "im:post:read", FAT_KEYS);
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
    assert_eq!(data["readBits"], "11", "readBits 来自在手 PostFields");
    assert_eq!(
        data["createAt"], 1_700_000_000,
        "createAt 来自在手 PostFields"
    );
    // props 由 JSON 字符串 parse 回对象（前端 props:any 期望对象，非字符串）。
    assert_eq!(data["props"]["k"], "v", "props parse 回对象");
    assert_eq!(data["viewers"][0], "all", "viewers 数组直出");
    // render-ready 终态字段（issue #53）：sample_fields 无 quickReply/template·type=TEXT →
    // sendStatus=sent / reactions=null / templateReceived=false / systemNotice=false。
    assert_eq!(data["sendStatus"], "sent", "sendStatus 恒 sent（已对账态）");
    assert!(
        data["reactions"].is_null(),
        "无 quickReply → reactions=null"
    );
    assert_eq!(
        data["templateReceived"], false,
        "无 template → templateReceived=false"
    );
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
        data["systemNotice"], true,
        "type=NOTICE ∈ NOTICE_TYPES → systemNotice=true"
    );
    assert!(data["pinned"].is_null(), "无 pinned props → pinned=null");
    assert_eq!(data["type"], "NOTICE", "NOTICE type 原样透传");
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
// §1b. channel-row 增量类（热路径单会话 patch，避免整表 dialogList）
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn im_channel_update_by_post_stays_thin_signal_triplet() {
    // 兼容旧消费方的瘦信号；新 UI 应优先消费 im:channel:changed 单会话 patch。
    let e = to_effect::emit_channel_update_by_post(ch(), SEQ, MSG_ID);
    assert_frozen(
        &e,
        "im:channel:update-by-post",
        &["channel_id", "event_seq", "msg_id"],
    );
}

#[test]
fn im_channel_changed_by_post_freezes_single_dialog_patch() {
    let upd = helix_im::channel_write::PostChannelUpdate {
        channel_id: ch(),
        unread_delta: 0,
        unread_post_id: None,
        last_post: r#"{"message":"hello patch"}"#.to_string(),
        has_schedule_post: false,
        msg_create_at: 123,
    };
    let e = to_effect::emit_channel_changed_by_post(&upd, SEQ, MSG_ID);
    assert_frozen(
        &e,
        "im:channel:changed",
        &[
            "channel_id",
            "channelId",
            "id",
            "event_seq",
            "msg_id",
            "reason",
            "patch",
        ],
    );
    let Effect::Emit { event } = &e else {
        panic!("必产 Emit")
    };
    let v: serde_json::Value = serde_json::from_slice(event.0.as_ref()).unwrap();
    assert_eq!(v["data"]["patch"]["lastMessage"], "hello patch");
    assert_eq!(v["data"]["patch"]["unreadDelta"], 0);
}

#[test]
fn im_channel_update_freezes_channel_id_only() {
    // 批次结束信号：只携 channel_id，保留给兼容/控制类刷新触发。
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
            "readBits",
            "text",
            "type",
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
        &["channelId", "sendStatus", "temporaryId"],
    );
}

// ════════════════════════════════════════════════════════════════════════════
// §2. wire camelCase 信号/控制类（透传现网 emit payload 形态）
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn im_sync_too_long_freezes_channelid_resetto_camelcase() {
    // 命名陷阱（§6）：channelId/resetTo 是 camelCase（对齐现网 post.rs:918），非 snake。
    let e = to_effect::emit_sync_too_long(ch(), Seq(100));
    assert_frozen(&e, "im:sync:too_long", &["channelId", "resetTo"]);
}

#[test]
fn im_connection_established_freezes_connectionid() {
    let e = to_effect::emit_connection_established("conn-abc");
    assert_frozen(&e, "im:connection:established", &["connectionId"]);
}

#[test]
fn im_channel_closed_freezes_channelid_deleteat_camelcase() {
    let e = to_effect_s1::emit_channel_closed(ch(), 1_700_000_000);
    assert_frozen(&e, "im:channel:closed", &["channelId", "deleteAt"]);
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
fn im_schedule_created_freezes_channelid_hasschedulepost() {
    let e = to_effect_s1::emit_schedule_created(ch());
    assert_frozen(
        &e,
        "im:channel:schedule-created",
        &["channelId", "hasSchedulePost"],
    );
}

#[test]
fn im_schedule_canceled_freezes_channelid_hasschedulepost() {
    let e = to_effect_s1::emit_schedule_canceled(ch());
    assert_frozen(
        &e,
        "im:channel:schedule-canceled",
        &["channelId", "hasSchedulePost"],
    );
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
fn im_channel_created_freezes_channel_id_and_channel() {
    let raw = serde_json::to_vec(&serde_json::json!({"displayName": "群"})).unwrap();
    let e = to_effect_s1::emit_channel_created(ch(), &raw);
    assert_frozen(&e, "im:channel:created", &["channel", "channel_id"]);
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
        {"id": "ch1", "display_name": "群A", "unread_count": 3, "mention_count": 1}
    ]))
    .unwrap();
    let e = query::emit_dialog_list_result(&rows);
    assert_frozen(&e, "im:channels:projection", &["dialogList"]);
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

/// 冻结全集计数：本测试覆盖的 distinct (工厂调用) 数 == projection-schema.md §1 表行数 == 24。
/// 任何新增 emit 工厂未在本测试加 case → 此断言提示补齐（防漏测漂移）。
#[test]
fn all_24_projection_factories_covered() {
    // 逐工厂构造一次，收集 (channel, keys)——证明 24 个出口全可达且产合法 Emit。
    let cid = channel_id(7);
    let raw = serde_json::to_vec(&serde_json::json!({})).unwrap();
    let posts = serde_json::json!([]);
    let f = sample_fields();
    let upd = helix_im::channel_write::PostChannelUpdate {
        channel_id: ch(),
        unread_delta: 1,
        unread_post_id: Some(MSG_ID.to_string()),
        last_post: r#"{"message":"hello"}"#.to_string(),
        has_schedule_post: false,
        msg_create_at: 123,
    };
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
        to_effect::emit_channel_update_by_post(ch(), SEQ, MSG_ID),
        to_effect::emit_channel_changed_by_post(&upd, SEQ, MSG_ID),
        to_effect_s1::emit_channel_created(ch(), &raw),
        to_effect_s1::emit_channel_closed(ch(), 0),
        to_effect_s1::emit_channel_member_updated(ch(), &raw),
        to_effect_s1::emit_member_nickname(ch(), "u", "n"),
        to_effect_s1::emit_schedule_created(ch()),
        to_effect_s1::emit_schedule_canceled(ch()),
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

/// 连接控制信号 `im:net:reconnect_requested`（F1-gap1）——**不计入** 24 投影工厂集（无渲染字段，
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
