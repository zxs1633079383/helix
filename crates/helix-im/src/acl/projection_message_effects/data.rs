//! Render-ready message data shaping.

use crate::state::ChannelId;

/// 系统/通知消息类型集（UC-10.2·data-system-notice 渲染源·权威真源冻结）。
///
/// 镜像现网前端 `enum.ts` NOTICE_TYPES（full-map partials/7--client-ui-rendering.md §3）：
/// 命中则该行走「系统消息」渲染（前端 `<message-system>` 分隔条），否则普通气泡。
/// `SYSTEM` 枚举值拼写陷阱保真（partial7 §4.2 `SYSTEM: 'SYSTEN'`）—— 两种拼写都判系统消息。
/// 业务规则下沉 helix（C013 纯渲染壳铁律 issue #53）：前端不再 `isSystemNotice(type)` 自判。
const NOTICE_TYPES: &[&str] = &[
    "TIME",
    "NOTICE",
    "MEETING_NOTICE",
    "SYSTEM_KR_EDIT",
    "SYSTEM_KR_STATE_EDIT",
    "SYSTEM_KR_VOTE",
    "SYSTEM_KR_PUBLISH",
    "SYSTEM_MONTH_ORIENT_SHARE",
    "SYSTEM",
    "SYSTEN",
];

/// 判消息类型是否系统通知行（大写归一·严格匹配 NOTICE_TYPES）。普通类型（TEXT/DOCUMENT…）→ false。
fn is_system_notice(msg_type: &str) -> bool {
    let upper = msg_type.to_ascii_uppercase();
    NOTICE_TYPES.contains(&upper.as_str())
}

/// 读取独立 durable quick_reply；仅为空时兼容历史 props.quickReply 行。
fn resolve_quick_reply(raw: &str, props: &serde_json::Value) -> Option<serde_json::Value> {
    if !raw.is_empty() {
        return serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .filter(serde_json::Value::is_array);
    }
    props
        .get("quickReply")
        .filter(|value| value.is_array())
        .cloned()
}

/// 把 quickReply 绝对态预解成 render-ready emoji 串。
fn resolve_reactions(quick_reply: Option<&serde_json::Value>) -> Option<String> {
    let arr = quick_reply?.as_array()?;
    let emojis: Vec<&str> = arr
        .iter()
        .filter_map(|e| e.get("emoji").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .collect();
    if emojis.is_empty() {
        None
    } else {
        Some(emojis.join(","))
    }
}

/// 汇总 quickReply 绝对态中的成员数，不对同一用户做本地推断。
fn resolve_reaction_count(quick_reply: Option<&serde_json::Value>) -> usize {
    quick_reply
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.get("userIds")
                        .or_else(|| item.get("user_ids"))
                        .and_then(|value| value.as_array())
                        .map(Vec::len)
                        .unwrap_or(0)
                })
                .sum()
        })
        .unwrap_or(0)
}

/// 预解 props.template.userIds 成模板已收到态（UC-3.3·data-template-received 渲染源）。
///
/// server templateReceived 回执后 post_update echo 把已收到回执列表携进 props.template.userIds。
/// 非空数组 → true。无 template / userIds 空 → false。业务规则下沉 helix（C013·前端不再
/// `extractTemplateReceived(props)`）。
fn resolve_template_received(props: &serde_json::Value) -> bool {
    props
        .get("template")
        .and_then(|t| t.get("userIds"))
        .and_then(|u| u.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
}

fn resolve_template_confirmed_count(props: &serde_json::Value) -> usize {
    props
        .get("template")
        .and_then(|value| value.get("userIds").or_else(|| value.get("user_ids")))
        .and_then(|value| value.as_array())
        .map(Vec::len)
        .unwrap_or(0)
}

fn resolve_template_reader_ids(props: &serde_json::Value) -> Vec<&str> {
    props
        .get("template")
        .and_then(|value| value.get("userIds").or_else(|| value.get("user_ids")))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())
        .collect()
}

/// 预解 props.pinned 成消息置顶态（UC-5.5b·data-pinned 渲染源）。
///
/// post_pin 帧只携 `{postId, channelId, operation}`，handler 将 operation 归一为 props.pinned。
/// 普通消息事件不携 pinned → None → JSON null，前端保持既有 pinned 状态不被普通 echo 清空。
fn resolve_pinned(props: &serde_json::Value) -> Option<bool> {
    props.get("pinned").and_then(|v| v.as_bool())
}

fn parse_json_field(raw: &str, fallback: serde_json::Value) -> serde_json::Value {
    if raw.is_empty() {
        fallback
    } else {
        serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .filter(|value| !value.is_null())
            .unwrap_or(fallback)
    }
}

/// spec06 FixB：message-row 信号的完整可渲染 payload 构造（数据在手，零回读）。
///
/// issue #53（S4 首条竖切·C013 纯渲染壳）：在 fat 渲染核之外补 4 个 **render-ready 终态字段**——
/// `sendStatus`（已对账态恒 "sent"·入库成功=消息收到）/ `reactions`（预解 emoji 串）/
/// `templateReceived`（模板已收到 bool）/ `systemNotice`（系统通知 bool）—— 把原本在前端壳里做的
/// shaping（extractReactions / extractTemplateReceived / isSystemNotice）下沉到 helix，前端退化为纯绑定。
/// `type` 空串容错为 "TEXT"（与 outbound send_build.rs 一致·解析下沉 parser 后此处为防御冗余）。
pub(super) fn message_item_data(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
    _revoked: bool,
) -> serde_json::Value {
    use serde_json::json;
    let props: serde_json::Value = if fields.props.is_empty() {
        json!({})
    } else {
        serde_json::from_str(&fields.props).unwrap_or_else(|_| json!({}))
    };
    let topic: serde_json::Value = if fields.topic.is_empty() {
        json!({})
    } else {
        serde_json::from_str(&fields.topic).unwrap_or_else(|_| json!({}))
    };
    let expedite_map: serde_json::Value = if fields.expedite_map.is_empty() {
        json!({})
    } else {
        serde_json::from_str(&fields.expedite_map).unwrap_or_else(|_| json!({}))
    };
    let quick_reply = resolve_quick_reply(fields.quick_reply.as_str(), &props);
    let is_self = !viewer_user_id.is_empty() && fields.user_id == viewer_user_id;
    let has_server_id = !msg_id.is_empty();
    let urgent =
        crate::query::render_ready::urgent::project(&expedite_map, viewer_user_id, has_server_id);
    // type 空串容错 "TEXT"（render-ready·前端不再 `d.type || "TEXT"` 兜底）。
    let msg_type = if fields.msg_type.is_empty() {
        "TEXT"
    } else {
        fields.msg_type.as_str()
    };
    let forward_detail = crate::query::render_ready::forward::detail(msg_type, &props);
    let user_snapshot = crate::query::render_ready::core::normalize_user_snapshot(
        &parse_json_field(&fields.user_snapshot, json!({})),
    );
    let replied_message = crate::query::render_ready::core::normalize_replied_message(
        &parse_json_field(&fields.replied_message, serde_json::Value::Null),
    );
    let urgent_fields = json!({
        "urgent": urgent.required_count > 0,
        "urgentTargetCount": urgent.required_count,
        "urgentConfirmedCount": urgent.confirmed_ids.len(),
        "urgentRequesterId": urgent.requester_id,
        "urgentTargetIds": urgent.target_ids,
        "urgentConfirmedIds": urgent.confirmed_ids,
        "urgentRequiredCount": urgent.required_count,
        "urgentState": urgent.state,
        "canConfirmUrgent": urgent.can_confirm,
    });
    let mut data = json!({
        "channel_id": channel_id.as_str(),
        "event_seq": event_seq,
        "msg_id": msg_id,
        // 与本地消息查询的 render-ready 行共享 canonical aliases；旧字段继续保留兼容现网。
        "id": msg_id,
        "msgId": msg_id,
        "temporaryId": fields.temporary_id.as_str(),
        "channelId": channel_id.as_str(),
        "eventSeq": event_seq,
        "userId": fields.user_id.as_str(),
        "userSnapshot": user_snapshot,
        "type": msg_type,
        "message": fields.message.as_str(),
        "simpleMessage": fields.simple_message.as_str(),
        "text": fields.message.as_str(),
        "props": props,
        "forwardDetail": forward_detail,
        "topic": topic,
        "createAt": fields.create_at,
        "createdAt": fields.create_at,
        "updateAt": if fields.update_at > 0 { fields.update_at } else { fields.create_at },
        "readBits": fields.read_bits.as_str(),
        "viewers": fields.viewers,
        "mentions": fields.mentions,
        // —— render-ready 终态字段（issue #53·C013 业务规则下沉 helix·前端纯绑定）——
        "sendStatus": "sent",
        "serverId": if has_server_id { serde_json::Value::String(msg_id.to_string()) } else { serde_json::Value::Null },
        "isSelf": is_self,
        "reactions": resolve_reactions(quick_reply.as_ref()),
        "reactionCount": resolve_reaction_count(quick_reply.as_ref()),
        "expediteMap": expedite_map,
        "templateReceived": resolve_template_received(&props),
        "templateConfirmedCount": resolve_template_confirmed_count(&props),
        "readerIds": resolve_template_reader_ids(&props),
        "systemNotice": is_system_notice(msg_type),
        "pinned": resolve_pinned(&props),
        "replyId": fields.reply_id.as_str(),
        "replyRootId": fields.reply_root_id.as_str(),
        "replyFirstLevelId": fields.reply_first_level_id.as_str(),
        "repliedMessage": replied_message,
        "replyMessages": parse_json_field(&fields.reply_messages, json!([])),
        "replyCount": fields.reply_count,
    });
    if let (serde_json::Value::Object(data), serde_json::Value::Object(urgent_fields)) =
        (&mut data, urgent_fields)
    {
        data.extend(urgent_fields);
    }
    if let (serde_json::Value::Object(data), Some(quick_reply)) = (&mut data, quick_reply) {
        data.insert("quickReply".to_string(), quick_reply);
    }
    data
}
