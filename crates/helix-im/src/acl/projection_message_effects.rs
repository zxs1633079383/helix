//! Message-row projection emit factories.

use crate::state::ChannelId;
use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;

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

/// 预解 props.quickReply 成 render-ready emoji 串（UC-1.8·data-reactions 渲染源）。
///
/// server post_update echo 把 quickReply 携进 props（parser merge_props_with_quickreply）：
/// 形态 `[{emoji, userIds:[…]}]`。把命中的 emoji 聚成串（逗号连·多 emoji）。无 quickReply / 空 → None
/// （前端绑 null 即不渲染该属性）。业务解析下沉 helix（C013·前端不再 `extractReactions(props)`）。
fn resolve_reactions(props: &serde_json::Value) -> Option<String> {
    let arr = props.get("quickReply")?.as_array()?;
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

fn resolve_reaction_count(props: &serde_json::Value) -> usize {
    props
        .get("quickReply")
        .and_then(|value| value.as_array())
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

/// 把持久化的 expedite_map 预解为消息加急终态。空/null/空容器均为 false，
/// 其余形态为 true；Angular 只绑定结果，不解析 sender/recipients 业务结构。
fn resolve_urgent(expedite_map: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(expedite_map) else {
        return false;
    };
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(value) => value,
        serde_json::Value::String(value) => !value.is_empty(),
        serde_json::Value::Array(value) => !value.is_empty(),
        serde_json::Value::Object(value) => !value.is_empty(),
        serde_json::Value::Number(value) => value.as_i64().unwrap_or_default() != 0,
    }
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

fn resolve_urgent_counts(expedite_map: &serde_json::Value) -> (usize, usize) {
    let Some(recipients) = expedite_map
        .get("recipients")
        .and_then(|value| value.as_object())
    else {
        return (0, 0);
    };
    let confirmed = recipients
        .values()
        .filter(|recipient| {
            recipient
                .get("status")
                .and_then(|value| value.as_i64())
                .unwrap_or_default()
                > 0
        })
        .count();
    (recipients.len(), confirmed)
}

/// 预解 props.pinned 成消息置顶态（UC-5.5b·data-pinned 渲染源）。
///
/// post_pin 帧只携 `{postId, channelId, operation}`，handler 将 operation 归一为 props.pinned。
/// 普通消息事件不携 pinned → None → JSON null，前端保持既有 pinned 状态不被普通 echo 清空。
fn resolve_pinned(props: &serde_json::Value) -> Option<bool> {
    props.get("pinned").and_then(|v| v.as_bool())
}

/// spec06 FixB：message-row 信号的完整可渲染 payload 构造（数据在手，零回读）。
///
/// issue #53（S4 首条竖切·C013 纯渲染壳）：在 fat 渲染核之外补 4 个 **render-ready 终态字段**——
/// `sendStatus`（已对账态恒 "sent"·入库成功=消息收到）/ `reactions`（预解 emoji 串）/
/// `templateReceived`（模板已收到 bool）/ `systemNotice`（系统通知 bool）—— 把原本在前端壳里做的
/// shaping（extractReactions / extractTemplateReceived / isSystemNotice）下沉到 helix，前端退化为纯绑定。
/// `type` 空串容错为 "TEXT"（与 outbound send_build.rs 一致·解析下沉 parser 后此处为防御冗余）。
fn message_item_data(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
    revoked: bool,
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
    let is_self = !viewer_user_id.is_empty() && fields.user_id == viewer_user_id;
    let has_server_id = !msg_id.is_empty();
    let action_capabilities = crate::message_capabilities::build(
        has_server_id,
        is_self,
        viewer_user_id,
        &props,
        &expedite_map,
        false,
        revoked,
    );
    let (urgent_target_count, urgent_confirmed_count) = resolve_urgent_counts(&expedite_map);
    // type 空串容错 "TEXT"（render-ready·前端不再 `d.type || "TEXT"` 兜底）。
    let msg_type = if fields.msg_type.is_empty() {
        "TEXT"
    } else {
        fields.msg_type.as_str()
    };
    json!({
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
        "type": msg_type,
        "message": fields.message.as_str(),
        "text": fields.message.as_str(),
        "props": props,
        "topic": topic,
        "createAt": fields.create_at,
        "createdAt": fields.create_at,
        "updateAt": fields.create_at,
        "readBits": fields.read_bits.as_str(),
        "viewers": fields.viewers,
        // —— render-ready 终态字段（issue #53·C013 业务规则下沉 helix·前端纯绑定）——
        "sendStatus": "sent",
        "serverId": if has_server_id { serde_json::Value::String(msg_id.to_string()) } else { serde_json::Value::Null },
        "isSelf": is_self,
        "actionCapabilities": action_capabilities,
        "reactions": resolve_reactions(&props),
        "reactionCount": resolve_reaction_count(&props),
        "urgent": resolve_urgent(&fields.expedite_map),
        "urgentTargetCount": urgent_target_count,
        "urgentConfirmedCount": urgent_confirmed_count,
        "expediteMap": expedite_map,
        "templateReceived": resolve_template_received(&props),
        "templateConfirmedCount": resolve_template_confirmed_count(&props),
        "systemNotice": is_system_notice(msg_type),
        "pinned": resolve_pinned(&props),
    })
}

/// 序列化 ImDomainEvent 为 DomainEventBytes（Emit 的载荷）。
pub fn emit_post_received(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    emit_post_received_for_viewer(channel_id, event_seq, msg_id, fields, "")
}

pub fn emit_post_received_for_viewer(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
    viewer_user_id: &str,
) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:received",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, viewer_user_id, false),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_received: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 在线 `post_read`（type=6）→ `im:post:read`。
pub fn emit_post_read(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:read",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, "", false),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_read: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// 离线 sync 应用的 type=6 read → `im:channel:read_echo`。
pub fn emit_channel_read_echo(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:channel:read_echo",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, "", false),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_channel_read_echo: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// C3：type=2 编辑 → `im:post:updated`（仅可见时 emit）。
pub fn emit_post_updated(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:updated",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, "", false),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_updated: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}

/// C3：type=3 撤回 → `im:post:deleted`。
pub fn emit_post_deleted(
    channel_id: ChannelId,
    event_seq: u64,
    msg_id: &str,
    fields: &crate::sync_session::PostFields,
) -> Effect {
    use serde_json::json;
    let payload = json!({
        "event": "im:post:deleted",
        "data": message_item_data(channel_id, event_seq, msg_id, fields, "", true),
    });
    let bytes = Bytes::from(
        serde_json::to_vec(&payload)
            .expect("emit_post_deleted: static JSON shape must not fail to serialize"),
    );
    Effect::Emit {
        event: DomainEventBytes(bytes),
    }
}
