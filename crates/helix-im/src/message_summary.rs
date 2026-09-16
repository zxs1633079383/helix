//! Helix 唯一的消息摘要规则；发送、服务端消息入站与历史投影共用。
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::borrow::Cow;
const DOCUMENT_EMPTY_SIMPLE_MESSAGE: &str = "[文档]";

/// O(n) 准备可直接显示的摘要；保留历史正文型摘要，稀疏事件不伪造新内容。
pub(crate) fn resolve(
    kind: &str,
    message: &str,
    props: &Value,
    saved: &str,
    revoked: bool,
) -> String {
    if revoked {
        return "撤回了一条消息".to_string();
    }
    if message.is_empty()
        && saved.is_empty()
        && !(kind == "NOTICE" && !notice_text(props).is_empty())
        && !props.as_object().is_some_and(|p| {
            p.keys().any(|k| {
                matches!(
                    k.as_str(),
                    "files"
                        | "file"
                        | "forward"
                        | "vote"
                        | "averageScore"
                        | "announcement"
                        | "chain"
                        | "categoryChain"
                        | "birthdayCard"
                        | "thanksCard"
                        | "taskCardBlock"
                        | "meetingCardBlock"
                        | "template"
                        | "document"
                )
            })
        })
    {
        return String::new();
    }
    if let Some(summary) = typed_summary(kind, message, props, saved) {
        return summary.chars().take(50).collect();
    }
    if !saved.trim().is_empty() {
        return saved.chars().take(50).collect();
    }
    simple_message(kind, message, props)
}

/// 频道末条准备摘要；MULTIPLY 只投影有界卡片，不生成消息身份或覆盖正文。
pub(crate) fn prepare_post(value: &Value) -> Value {
    let mut post = decoded(value).into_owned();
    let Some(object) = post.as_object() else {
        return value.clone();
    };
    if object.is_empty() {
        return post;
    }
    let kind = object.get("type").and_then(Value::as_str).unwrap_or("TEXT");
    let message = object
        .get("message")
        .or_else(|| object.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let saved = object
        .get("simpleMessage")
        .or_else(|| object.get("simple_message"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let props = decoded(object.get("props").unwrap_or(&Value::Null));
    let revoked = object
        .get("revoke")
        .is_some_and(|v| v.as_bool() == Some(true) || v.as_i64() == Some(1));
    let summary = resolve(kind, message, &props, saved, revoked);
    let forward_props = kind
        .eq_ignore_ascii_case("MULTIPLY")
        .then(|| crate::query::render_ready::forward::props(kind, props.into_owned()));
    if !summary.is_empty() {
        if object.contains_key("simple_message") {
            post["simple_message"] = Value::String(summary.clone());
        }
        post["simpleMessage"] = Value::String(summary);
    }
    if let Some(props) = forward_props {
        post["props"] = props;
        if let Some(object) = post.as_object_mut() {
            object.remove("forwardDetail");
        }
    }
    post
}

/// 兼容服务端 JSON 字符串型卡片字段，不改动持久化 props。
fn decoded(value: &Value) -> Cow<'_, Value> {
    match value.as_str() {
        Some(raw) => Cow::Owned(serde_json::from_str(raw).unwrap_or(Value::Null)),
        None => Cow::Borrowed(value),
    }
}

/// 业务类型的标题和标签在 Helix 一次生成；已带标签的服务端摘要不会重复加前缀。
fn typed_summary(kind: &str, message: &str, props: &Value, saved: &str) -> Option<String> {
    if kind == "MULTIPLY" {
        return Some("[聊天记录]".to_string());
    }
    if kind == "VOICE"
        || (kind == "AUDIO"
            && props
                .pointer("/voiceRecording/version")
                .is_some_and(|v| v.as_u64() == Some(1) || v.as_str() == Some("1")))
    {
        return Some("[语音]".to_string());
    }
    if kind == "NOTICE" {
        let notice = notice_text(props);
        return Some(if notice.is_empty() {
            if saved.is_empty() { message } else { saved }.to_string()
        } else {
            notice
        });
    }
    let (label, field, keys): (&str, &str, &[&str]) = match kind {
        "ANNOUNCEMENT" => ("群公告", "announcement", &["/title", "/content", "/text"]),
        "VOTE" => ("投票", "vote", &["/title", "/name", "/content"]),
        "AVERAGE_SCORE" => ("平均分", "averageScore", &["/title", "/name", "/content"]),
        "TEXT_CHAIN" | "CHAIN" => ("文字接龙", "chain", &["/title"]),
        "CATEGORY_CHAIN" => ("分类接龙", "categoryChain", &["/title"]),
        "BIRTHDAY_CARD" => ("生日祝福", "birthdayCard", &["/recipientName"]),
        "THANKS_CARD" => ("感谢卡片", "thanksCard", &["/senderName"]),
        "TASK_CARD_BLOCK" => ("任务卡片", "taskCardBlock", &["/detail/title", "/title"]),
        "MEETING_CARD_BLOCK" => ("会议卡片", "meetingCardBlock", &["/detail/title", "/title"]),
        _ => return None,
    };
    let card = decoded(&props[field]);
    let text = keys
        .iter()
        .find_map(|key| {
            card.pointer(key)
                .and_then(Value::as_str)
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or(if saved.is_empty() { message } else { saved });
    let prefix = format!("[{label}]");
    let text = markdown_text(text.trim().strip_prefix(&prefix).unwrap_or(text).trim());
    Some(if text.is_empty() {
        prefix
    } else {
        format!("{prefix} {text}")
    })
}

/// 系统通知使用服务端携带的姓名事实，不在摘要中编码某个接收者的“您”。
fn notice_name(user: &Value) -> &str {
    ["nickName", "userName", "name"]
        .iter()
        .find_map(|key| user[*key].as_str().filter(|v| !v.trim().is_empty()))
        .unwrap_or("群成员")
}

/// O(n) 转换已知群通知；未知通知回退服务端原摘要，不输出事件代码。
fn notice_text(props: &Value) -> String {
    let operator = &props["operator"];
    let name = notice_name(operator);
    let users = props["users"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    let names = |exclude_operator: bool| {
        let selected: Vec<_> = users
            .iter()
            .filter(|u| !exclude_operator || u["id"] != operator["id"])
            .collect();
        let text = selected
            .iter()
            .take(10)
            .map(|u| notice_name(u))
            .collect::<Vec<_>>()
            .join("、");
        if selected.len() > 10 {
            format!("{text}等{}人", selected.len())
        } else {
            text
        }
    };
    match props["type"].as_str().unwrap_or("") {
        "join" => format!("{name}邀请{}加入群聊", names(true)),
        "leave" if users.len() == 1 && users[0]["id"] == operator["id"] => {
            format!("{}退出了群聊", notice_name(&users[0]))
        }
        "leave" => format!("{name}将{}移除了群聊", names(true)),
        "addManager" => format!("{name}将{}设为管理员", names(false)),
        "removeManager" => format!("{name}将{}移除管理员", names(false)),
        "creatorChange" => format!("{}已成为新群主", notice_name(&props["user"])),
        "addPostPin" => format!("{name}置顶了一条消息"),
        "removePostPin" => format!("{name}取消置顶了一条消息"),
        "channelUpdate" => {
            let content = decoded(&props["content"]);
            let field = props["field"].as_str().unwrap_or("");
            if field == "close" && props["content"].as_str() == Some("closed") {
                return format!("{name}关闭了群聊");
            }
            if field == "owner" {
                return format!("{name}将群主移交给{}", notice_name(&content));
            }
            let title = match field {
                "orient" => "群导向",
                "purpose" => "群简介",
                "displayName" => "群名称",
                "noticePermission" => "群公告权限",
                "topPermission" => "消息置顶权限",
                "mentionPermission" => "@全体权限",
                _ => return String::new(),
            };
            let value = props["content"].as_str().unwrap_or("");
            let value = if field.ends_with("Permission") {
                match value {
                    "CREATOR" => "创建者",
                    "MANAGER" => "创建人及管理员",
                    "MEMBER" => "所有人",
                    other => other,
                }
            } else {
                value
            };
            format!("{name}修改了{title}: {value}")
        }
        _ => String::new(),
    }
}

/// 从原文派生摘要，绝不覆盖消息正文或改变搜索索引输入。
pub(crate) fn simple_message(msg_type: &str, message: &str, props: &Value) -> String {
    if let Some(summary) = typed_summary(msg_type, message, props, "") {
        return summary.chars().take(50).collect();
    }
    let visible_text;
    let message = if matches!(msg_type, "TEXT" | "RICH" | "IMAGE")
        || (msg_type == "TEMPLATE"
            && props.pointer("/template/type").and_then(Value::as_str) == Some("TEXT"))
    {
        visible_text = markdown_text(message);
        if visible_text.is_empty() && !matches!(msg_type, "RICH" | "IMAGE") {
            "[消息]"
        } else {
            visible_text.as_str()
        }
    } else {
        message
    };
    if matches!(msg_type, "RICH" | "IMAGE") {
        let files = props.get("files").and_then(Value::as_array);
        let kind = if media_files_are(files, "image/") {
            "图片"
        } else if media_files_are(files, "video/") {
            "视频"
        } else {
            "媒体"
        };
        let mut summary = format!("[{kind}");
        if let Some(count) = files.filter(|files| files.len() > 1).map(Vec::len) {
            summary.push_str(&format!(" {count}个"));
        }
        summary.push(']');
        if !message.trim().is_empty() {
            summary.push(' ');
            summary.push_str(message.trim());
        }
        return summary.chars().take(50).collect();
    }
    let (prefix, content) = match msg_type {
        "FILE" => (
            "[文件]",
            props
                .pointer("/file/name")
                .or_else(|| props.pointer("/file/mediaInput/fileName"))
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .unwrap_or(message),
        ),
        "AUDIO" => ("[音频]", message),
        "VIDEO" => ("[视频]", message),
        // MV3-G01f：正文前 50 字；空文档兜底 `[文档]`（会话列表摘要不得为空串）。
        "DOCUMENT" => (
            "",
            if message.trim().is_empty() {
                DOCUMENT_EMPTY_SIMPLE_MESSAGE
            } else {
                message
            },
        ),
        _ => ("", message),
    };
    prefix.chars().chain(content.chars()).take(50).collect()
}

fn media_files_are(files: Option<&Vec<Value>>, prefix: &str) -> bool {
    files.is_some_and(|files| {
        !files.is_empty()
            && files.iter().all(|file| {
                file.get("contentType")
                    .or_else(|| file.pointer("/mediaInput/contentType"))
                    .and_then(Value::as_str)
                    .is_some_and(|content_type| content_type.starts_with(prefix))
            })
    })
}

/// O(n) 提取 CommonMark 可见文本；代码/HTML 保留字面量，块边界折叠为单个空格。
fn markdown_text(source: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, TagEnd};
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut text = String::with_capacity(source.len());
    for event in Parser::new_ext(source, options) {
        match event {
            Event::Text(value)
            | Event::Code(value)
            | Event::Html(value)
            | Event::InlineHtml(value) => text.push_str(&value),
            Event::SoftBreak | Event::HardBreak | Event::Rule => text.push(' '),
            Event::End(
                TagEnd::Paragraph
                | TagEnd::Heading(_)
                | TagEnd::CodeBlock
                | TagEnd::Item
                | TagEnd::TableCell
                | TagEnd::TableRow,
            ) => text.push(' '),
            Event::TaskListMarker(checked) => text.push_str(if checked { "[x] " } else { "[ ] " }),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 多附件数量属于媒体标签；正文折行、单附件和 50 字上限保持原规则。
#[cfg(test)]
#[test]
fn media_summary_counts_stay_inside_brackets() {
    for (types, expected) in [
        (vec!["image/png"], "[图片]"),
        (vec!["video/mp4"], "[视频]"),
        (vec!["image/png"; 3], "[图片 3个]"),
        (vec!["video/mp4"; 2], "[视频 2个]"),
        (vec!["image/png", "video/mp4"], "[媒体 2个]"),
    ] {
        let props = json!({"files": types.iter().map(|kind| json!({"contentType": kind})).collect::<Vec<_>>()});
        assert_eq!(simple_message("RICH", "", &props), expected);
        assert_eq!(
            simple_message("RICH", "你好\n世界", &props),
            format!("{expected} 你好 世界")
        );
        let long = simple_message("RICH", &"字".repeat(80), &props);
        assert_eq!(long.chars().count(), 50);
        assert!(long.starts_with(&format!("{expected} ")));
    }
}

#[cfg(test)]
#[test]
fn summary_rules_cover_business_messages_and_sparse_updates() {
    for (kind, props, expected) in [
        (
            "MULTIPLY",
            json!({"forward": {"snapshotId":"s","title":"记录","itemCount":1,"preview":[]}}),
            "[聊天记录]",
        ),
        (
            "ANNOUNCEMENT",
            json!({"announcement": {"content":"标题"}}),
            "[群公告] 标题",
        ),
        (
            "VOTE",
            json!({"vote": "{\"title\":\"标题\"}"}),
            "[投票] 标题",
        ),
        (
            "AVERAGE_SCORE",
            json!({"averageScore":{"title":"标题"}}),
            "[平均分] 标题",
        ),
        (
            "TEXT_CHAIN",
            json!({"chain":{"title":"标题"}}),
            "[文字接龙] 标题",
        ),
        (
            "CATEGORY_CHAIN",
            json!({"categoryChain":{"title":"标题"}}),
            "[分类接龙] 标题",
        ),
        (
            "BIRTHDAY_CARD",
            json!({"birthdayCard":{"recipientName":"标题"}}),
            "[生日祝福] 标题",
        ),
        (
            "THANKS_CARD",
            json!({"thanksCard":{"senderName":"标题"}}),
            "[感谢卡片] 标题",
        ),
        (
            "TASK_CARD_BLOCK",
            json!({"taskCardBlock":{"detail":{"title":"标题"}}}),
            "[任务卡片] 标题",
        ),
        (
            "MEETING_CARD_BLOCK",
            json!({"meetingCardBlock":{"title":"标题"}}),
            "[会议卡片] 标题",
        ),
        ("AUDIO", json!({"voiceRecording":{"version":1}}), "[语音]"),
        (
            "NOTICE",
            json!({"type":"channelUpdate","field":"close","content":"closed","operator":{"name":"张三"}}),
            "张三关闭了群聊",
        ),
    ] {
        assert_eq!(
            simple_message(kind, "正文", &props),
            expected,
            "send {kind}"
        );
        assert_eq!(
            resolve(kind, "正文", &props, "旧摘要", false),
            expected,
            "projection {kind}"
        );
        assert_eq!(
            resolve(kind, "", &Value::Null, expected, false),
            expected,
            "idempotent {kind}"
        );
    }
    assert_eq!(
        resolve("CUSTOM", "正文", &Value::Null, "服务端摘要", false),
        "服务端摘要"
    );
    assert_eq!(
        resolve("TEXT", "**粗体**\n下一行", &Value::Null, "", false),
        "粗体 下一行"
    );
    assert_eq!(resolve("RICH", "", &json!({"pin":true}), "", false), "");
    assert_eq!(
        resolve("TEXT", "正文", &Value::Null, "摘要", true),
        "撤回了一条消息"
    );
    assert_eq!(
        resolve("VOTE", "标题", &json!({"vote":"invalid"}), "", false),
        "[投票] 标题"
    );
}

#[cfg(test)]
#[test]
fn summary_is_shared_by_inbound_history_last_post_and_forward_detail() {
    use crate::state::{ChannelId, Seq};
    use crate::sync_session::{EventEnvelope, EventKind, POST_FIELD_SIMPLE_MESSAGE};
    let channel_id = ChannelId::from_str("yo9n4iud8fyt78zwjkonsya87e").unwrap();
    for (kind, message, props, expected) in [
        ("TEXT", "**你好**", json!({}), "你好"),
        (
            "MULTIPLY",
            "",
            json!({"forward": {"snapshotId":"s","title":"记录","itemCount":1,"preview":[]}}),
            "[聊天记录]",
        ),
        (
            "RICH",
            "文字",
            json!({"files":[{"contentType":"image/png"},{"contentType":"video/mp4"}]}),
            "[媒体 2个] 文字",
        ),
        ("VOTE", "", json!({"vote":{"title":"晚饭"}}), "[投票] 晚饭"),
        (
            "NOTICE",
            "",
            json!({"type":"addPostPin","operator":{"name":"张三"}}),
            "张三置顶了一条消息",
        ),
    ] {
        let row = json!({"id":"p", "type":kind,"message":message,"props":props});
        let fields = crate::ws::parser::extract_post_fields(&row);
        assert_eq!(fields.simple_message, expected);
        assert_ne!(fields.present_fields & POST_FIELD_SIMPLE_MESSAGE, 0);
        let event = EventEnvelope::new(channel_id, Seq(1), EventKind::PostUpsert, fields);
        let live = crate::event::post::authority_projection(&event).unwrap();
        assert_eq!(live.received_data["simpleMessage"], expected);
        assert_eq!(live.last_post["simpleMessage"], expected);
        assert_eq!(
            crate::query::render_ready::core::shape_row(&row, "viewer")["simpleMessage"],
            expected
        );
        // Snapshot page projection is exercised through the public forward-detail lifecycle.
    }
    let sparse = crate::ws::parser::extract_post_fields(
        &json!({"id":"p","props":{"pin":true,"type":"pin"}}),
    );
    assert_eq!(sparse.simple_message, "");
    assert_eq!(sparse.present_fields & POST_FIELD_SIMPLE_MESSAGE, 0);
}

#[cfg(test)]
#[test]
fn channel_summary_survives_full_patch_member_and_event_paths() {
    use crate::state::ChannelId;
    use helix_core::effect::SqlValue;
    let id = ChannelId::from_str("nfds4ncb8pyb98thscnuujgmuw").unwrap();
    let post = json!({"id":"notice-1","userId":"SYS","type":"NOTICE","message":"","simpleMessage":"",
        "props":{"type":"join","operator":{"id":"a","name":"甲"},"users":[{"id":"a","name":"甲"},{"id":"b","name":"乙"}]}});
    let expected = "甲邀请乙加入群聊";
    for encoded in [post.clone(), Value::String(post.to_string())] {
        let channel = json!({"id":id.as_str(),"lastPost":encoded,"lastPostAt":123,
            "mentionList":["notice-1"],"urgentPostList":["notice-1"],"urgentCount":1,"mentionUser":"甲","draft":{"currentText":"草稿"}});
        let (full, _) = crate::channel_write::program_full(&channel, "b", 123).unwrap();
        let patch = crate::channel_write::collect_present(&channel);
        for row in [&full, &patch] {
            let raw = row
                .iter()
                .find_map(|(key, value)| match value {
                    SqlValue::Text(raw) if *key == "last_post" => Some(raw),
                    _ => None,
                })
                .unwrap();
            let saved: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(saved["simpleMessage"], expected);
            assert_eq!(saved["props"], post["props"]);
            assert_eq!(saved["message"], "");
        }
        let (member_row, projection) =
            crate::channel_update::member_channel_from_update_channel(&channel, id, "b", 123)
                .unwrap();
        assert!(member_row.iter().any(|(key, value)| key == "last_post"
            && matches!(value, SqlValue::Text(raw) if raw.contains(expected))));
        let data = crate::acl::to_effect::member_channel_update_data(id, &projection, "test");
        assert_eq!(data["lastPost"]["simpleMessage"], expected);
        assert_eq!(data["dialogPatch"]["lastPost"]["simpleMessage"], expected);
        assert_eq!(data["messageClass"], "mention_urgent");
        for event in [
            crate::event::channel::created(channel.clone()),
            crate::event::channel::update(channel.clone()),
        ] {
            let envelope: Value = serde_json::from_slice(&event.unwrap().into_bytes()).unwrap();
            assert_eq!(envelope["data"]["lastPost"]["simpleMessage"], expected);
            for key in [
                "mentionList",
                "urgentPostList",
                "urgentCount",
                "mentionUser",
                "draft",
            ] {
                assert_eq!(envelope["data"][key], channel[key]);
            }
        }
    }
    for empty in [Value::Null, json!({}), json!("bad-json")] {
        assert_eq!(prepare_post(&empty), empty);
    }
    let sparse = json!({"id":id.as_str(),"urgentCount":2});
    assert!(!crate::channel_write::collect_present(&sparse)
        .iter()
        .any(|(key, _)| *key == "last_post"));
    let event: Value = serde_json::from_slice(
        &crate::event::channel::update(sparse.clone())
            .unwrap()
            .into_bytes(),
    )
    .unwrap();
    assert_eq!(event["data"], sparse);
}
