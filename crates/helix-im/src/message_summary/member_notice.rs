//! 成员通知的唯一文案计划；姓名仅作旧字符串兼容，不进入结构化用户引用。
use serde::Serialize;
use serde_json::Value;

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(super) enum Part {
    Text {
        text: String,
    },
    User {
        #[serde(rename = "userId")]
        user_id: String,
        #[serde(rename = "companyId")]
        company_id: String,
        #[serde(skip)]
        fallback: String,
    },
}

impl Part {
    /// 仅旧 simpleMessage 使用已知快照；新消费者按 ID 查询自己的用户目录。
    pub(super) fn legacy_text(&self) -> &str {
        match self {
            Self::Text { text } => text,
            Self::User { fallback, .. } => fallback,
        }
    }

    /// 缺少真实用户 ID 的历史对象不发布无法解析的结构化引用。
    fn valid_reference(&self) -> bool {
        !matches!(self, Self::User { user_id, .. } if user_id.is_empty())
    }
}

/// 兼容 id/userId 与 teamId/companyId；空字段不覆盖有效别名或频道公司。
fn identity_text<'a>(user: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| user[*key].as_str().map(str::trim).filter(|v| !v.is_empty()))
}

/// 投影稳定身份与公司作用域，避免姓名被持久化为客户端身份权威。
fn user_part(user: &Value, company_id: &str) -> Part {
    Part::User {
        user_id: identity_text(user, &["id", "userId"])
            .unwrap_or("")
            .to_owned(),
        company_id: identity_text(user, &["teamId", "companyId"])
            .unwrap_or(company_id)
            .to_owned(),
        fallback: super::notice_name(user).to_owned(),
    }
}

/// 将固定业务文案封装为无身份的文本段。
fn text(value: impl Into<String>) -> Part {
    Part::Text { text: value.into() }
}

/// O(n) 统计目标，O(min(n, 10)) 空间；同一有序计划供旧摘要与各入站路径共用。
pub(super) fn plan(props: &Value, company_id: &str) -> Option<Vec<Part>> {
    let action = props["type"].as_str()?;
    if !matches!(action, "join" | "leave") {
        return None;
    }
    let operator = &props["operator"];
    let users = props["users"].as_array()?;
    let actor_id = identity_text(operator, &["id", "userId"]);
    if action == "leave"
        && users.len() == 1
        && actor_id.is_some()
        && identity_text(&users[0], &["id", "userId"]) == actor_id
    {
        return Some(vec![user_part(&users[0], company_id), text("退出了群聊")]);
    }
    let mut parts = vec![
        user_part(operator, company_id),
        text(if action == "join" { "邀请" } else { "将" }),
    ];
    let mut count = 0;
    for user in users {
        if actor_id.is_some() && identity_text(user, &["id", "userId"]) == actor_id {
            continue;
        }
        if count < 10 {
            if count > 0 {
                parts.push(text("、"));
            }
            parts.push(user_part(user, company_id));
        }
        count += 1;
    }
    if count > 10 {
        parts.push(text(format!("等{count}人")));
    }
    parts.push(text(if action == "join" {
        "加入群聊"
    } else {
        "移除了群聊"
    }));
    Some(parts)
}

/// 只发送完整可解析的计划；旧客户端仍使用 simpleMessage，未知类型不新增字段。
pub(super) fn wire(props: &Value, company_id: &str) -> Option<Value> {
    let parts = plan(props, company_id)?;
    if !parts.iter().all(Part::valid_reference) {
        return None;
    }
    serde_json::to_value(parts).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 去重只负责排除操作者，保留服务端给出的成员顺序与总数；输出空间与群规模无关。
    #[test]
    fn bounded_ordered_plan_preserves_cross_company_ids() {
        let mut users = vec![json!({"userId":"actor"})];
        users.extend(
            (0..1000).map(|i| json!({"id":format!("u{i}"),"companyId":"other","name":"旧姓名"})),
        );
        let props = json!({"type":"join","operator":{"userId":"actor","teamId":" ","companyId":"actor-company"},"users":users});
        let wire = wire(&props, "channel-company").unwrap();
        let parts = wire.as_array().unwrap();
        assert_eq!(parts.len(), 23);
        assert_eq!(
            parts[0],
            json!({"kind":"user","userId":"actor","companyId":"actor-company"})
        );
        let refs: Vec<_> = parts.iter().filter(|part| part["kind"] == "user").collect();
        assert_eq!(refs.len(), 11);
        for (i, part) in refs[1..].iter().enumerate() {
            assert_eq!(part["userId"], format!("u{i}"));
            assert_eq!(part["companyId"], "other");
        }
        assert_eq!(parts[21], json!({"kind":"text","text":"等1000人"}));
        assert!(!wire.to_string().contains("旧姓名"));
    }

    /// 自退仍使用目标身份，接收者“您”留给目录展示层，不写入共享持久化摘要。
    #[test]
    fn self_leave_and_legacy_string_share_one_plan() {
        let props = json!({"type":"leave","operator":{"id":"a","name":"甲"},"users":[{"userId":"a","name":"甲"}]});
        assert_eq!(super::super::notice_text(&props), "甲退出了群聊");
        assert_eq!(
            wire(&props, "c"),
            Some(json!([
                {"kind":"user","userId":"a","companyId":"c"},
                {"kind":"text","text":"退出了群聊"}
            ]))
        );
        let legacy =
            json!({"type":"join","operator":{"name":"甲"},"users":[{"id":"b","name":"乙"}]});
        assert_eq!(super::super::notice_text(&legacy), "甲邀请乙加入群聊");
        assert_eq!(wire(&legacy, "c"), None);
    }

    /// 撤回、未知和损坏 props 不得复活旧姓名计划；已知通知只从权威 props 重建。
    #[test]
    fn cached_plan_is_rebuilt_or_removed() {
        let props = json!({"type":"join","operator":{"id":"a"},"users":[{"id":"b"}]});
        let valid = json!({"type":"NOTICE","teamId":"c","props":props,"summaryParts":[{"kind":"text","text":"错误缓存"}]});
        let fixed = super::super::prepare_post(&valid);
        assert_eq!(fixed["summaryParts"], wire(&props, "c").unwrap());
        let mut aliases = valid.clone();
        aliases["teamId"] = json!(" ");
        aliases["team_id"] = json!("other-company");
        assert_eq!(
            super::super::prepare_post(&aliases)["summaryParts"],
            wire(&props, "other-company").unwrap()
        );
        for revoke in [json!(true), json!(1)] {
            let mut post = valid.clone();
            post["revoke"] = revoke;
            assert!(super::super::prepare_post(&post)
                .get("summaryParts")
                .is_none());
        }
        for props in [
            json!({"type":"unknown"}),
            json!("malformed"),
            json!({"type":"join","operator":{"id":"a"}}),
        ] {
            let mut post = valid.clone();
            post["props"] = props;
            assert!(super::super::prepare_post(&post)
                .get("summaryParts")
                .is_none());
        }
        let mut post = valid;
        post["type"] = json!("TEXT");
        assert!(super::super::prepare_post(&post)
            .get("summaryParts")
            .is_none());
    }
}
