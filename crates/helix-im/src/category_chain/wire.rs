//! 分类接龙 HTTP wire 边界。
//!
//! 这里把公开命令的 snake_case intent 收敛为 Go 的 lowerCamelCase 请求，并在回灌
//! 进入 runtime 前剥掉 host 的 ADR-007 信封。服务端才是 actor、tenant、计数、版本和
//! 截止时间的权威；因此本模块只接受用户可编辑字段，所有 authority 字段均拒绝输入。

use std::collections::HashSet;

use serde_json::{Map, Value};

use crate::error::ImError;

/// 分类接龙的 canonical command 闭集；capabilities 是独立的只读能力探针。
pub(crate) const COMMANDS: &[&str] = &[
    "category_chain_capabilities",
    "category_chain_create_draft",
    "category_chain_update_draft",
    "category_chain_publish",
    "category_chain_get",
    "category_chain_get_mine",
    "category_chain_set_participation",
    "category_chain_entries",
    "category_chain_close",
    "category_chain_retract",
    "category_chain_reconcile",
];

const MAX_HTTP_BODY_BYTES: usize = 256 * 1024;
const MAX_CATEGORIES: usize = 10;
const MIN_CATEGORIES: usize = 2;
const MAX_ENTRIES_PAGE: usize = 100;

/// 分类语义保持原样，仅在 HTTP 边界统一注入 post-chain v3 的 query/body mode 与 view。
///
/// req_id 只用于 Helix 本地 read correlation，永远不进入 Go body；嵌套 audience、
/// categories 和 selections 已经是 camelCase 合同，校验后原样保留。未知字段和服务端
/// 权威字段 fail closed，避免 UI 误把本地状态写成服务端事实。
pub(crate) fn wire_body(command: &str, args: &Value) -> Result<(&'static str, Value), ImError> {
    if command == "category_chain_capabilities" {
        let object = object(args, command)?;
        reject_unknown(object, &["req_id"], command)?;
        check_req_id(object, command)?;
        return Ok((
            "post-chain/capabilities?mode=CATEGORY",
            serde_json::json!({"mode":"CATEGORY"}),
        ));
    }

    if !COMMANDS.contains(&command) {
        return Err(parse_error(command, "unknown command"));
    }

    let input = object(args, command)?;
    check_req_id(input, command)?;
    let mut body = match command {
        "category_chain_create_draft" => build_draft(command, input, false)?,
        "category_chain_update_draft" => build_draft(command, input, true)?,
        "category_chain_publish" => build_publish(command, input)?,
        "category_chain_get" => build_scope(command, input)?,
        "category_chain_get_mine" => build_scope(command, input)?,
        "category_chain_set_participation" => build_participation(command, input)?,
        "category_chain_entries" => build_entries(command, input)?,
        "category_chain_reconcile" => build_reconcile(command, input)?,
        "category_chain_close" => build_lifecycle(command, input)?,
        "category_chain_retract" => build_lifecycle(command, input)?,
        _ => return Err(parse_error(command, "unknown command")),
    };

    let path = match command {
        "category_chain_create_draft" => "post-chain/create?mode=CATEGORY",
        "category_chain_update_draft" => "post-chain/update-draft?mode=CATEGORY",
        "category_chain_publish" => "post-chain/publish?mode=CATEGORY",
        "category_chain_get" => "post-chain/get?mode=CATEGORY",
        "category_chain_get_mine" => "post-chain/get?mode=CATEGORY",
        "category_chain_set_participation" => "post-chain/append?mode=CATEGORY",
        "category_chain_entries" => "post-chain/get?mode=CATEGORY",
        "category_chain_reconcile" => "post-chain/reconcile?mode=CATEGORY",
        "category_chain_close" => "post-chain/close?mode=CATEGORY",
        "category_chain_retract" => "post-chain/retract?mode=CATEGORY",
        _ => return Err(parse_error(command, "unknown command")),
    };

    // Transport discriminators are derived from the registered semantic command, never caller authority.
    body["mode"] = Value::String("CATEGORY".to_owned());
    let view = match command {
        "category_chain_get" => Some("card"),
        "category_chain_get_mine" => Some("mine"),
        "category_chain_entries" => Some("entries"),
        _ => None,
    };
    if let Some(view) = view {
        body["view"] = Value::String(view.to_owned());
    }

    let body_size = serde_json::to_vec(&body)
        .map_err(|error| serialize_error(command, error.to_string()))?
        .len();
    if body_size > MAX_HTTP_BODY_BYTES {
        return Err(parse_error(command, "request body exceeds 256 KiB"));
    }
    Ok((path, body))
}

/// 剥 host 的 numeric-status/base64 HTTP envelope，严格接受 Go status=SUCCESS。
///
/// Go 业务失败仍可能通过 HTTP 200 返回；此时保留 data.code（或根 code）到稳定的
/// CATEGORY_CHAIN_REJECTED:<code> 错误文本，供 runtime 区分业务拒绝和待对账故障。
pub(crate) fn decode_response(reply: &[u8]) -> Result<Value, ImError> {
    let raw = crate::http_envelope::unwrap_success_envelope(reply, "category_chain")?;
    let response: Value = serde_json::from_slice(&raw)
        .map_err(|error| parse_error("category_chain", format!("response JSON: {error}")))?;
    let root = response
        .as_object()
        .ok_or_else(|| parse_error("category_chain", "CommonRes must be an object"))?;
    let status = root
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| parse_error("category_chain", "CommonRes status must be a string"))?;
    if status != "SUCCESS" {
        let code = root
            .get("data")
            .and_then(Value::as_object)
            .and_then(|data| data.get("code"))
            .and_then(Value::as_str)
            .or_else(|| root.get("code").and_then(Value::as_str))
            .filter(|value| !value.is_empty())
            .or_else(|| {
                root.get("message")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or("UNKNOWN");
        return Err(parse_error(
            "category_chain",
            format!("CATEGORY_CHAIN_REJECTED:{code}"),
        ));
    }
    root.get("data")
        .cloned()
        .filter(|data| !data.is_null())
        .ok_or_else(|| parse_error("category_chain", "SUCCESS response missing data"))
}

/// 校验服务端 authority 的 scope、schema/version 和有界计数字段。
///
/// channelId 在 myParticipation/entries 这类直接响应中按合同可以省略，所以只要
/// 它出现就必须与请求频道一致；chain 已知时必须在响应中出现且每个出现的 chainId
/// 都必须匹配。is_private 只控制私有返回允许省略 channelId，以及禁止公开投影泄露
/// myParticipation/audience；它不把客户端字段升级成 authority。
pub(crate) fn validate_authority(
    data: &Value,
    channel: &str,
    chain: Option<&str>,
    is_private: bool,
) -> Result<(), ImError> {
    if channel.is_empty() {
        return Err(authority_error("empty channel scope"));
    }
    let root = data
        .as_object()
        .ok_or_else(|| authority_error("data must be an object"))?;
    let mut seen_channel = false;
    let mut seen_chain = false;
    validate_node(
        data,
        "$",
        channel,
        chain,
        is_private,
        &mut seen_channel,
        &mut seen_chain,
    )?;
    validate_response_shape(root, channel, chain, is_private)?;

    if let Some(expected_chain) = chain {
        if expected_chain.is_empty() || (!seen_chain && !is_reconcile_result(root)) {
            return Err(authority_error("missing chainId"));
        }
    } else if !seen_chain && !is_reconcile_result(root) {
        return Err(authority_error("missing chainId"));
    }
    Ok(())
}

/// 构造草稿创建/更新 body，并校验冻结的文本、分类和 audience 边界。
fn build_draft(
    command: &str,
    input: &Map<String, Value>,
    updating: bool,
) -> Result<Value, ImError> {
    let mut allowed = vec![
        "channel_id",
        "title",
        "description",
        "example",
        "deadline_at",
        "audience",
        "categories",
        "max_categories",
        "client_mutation_id",
        "req_id",
    ];
    if updating {
        allowed.extend(["chain_id", "expected_definition_revision"]);
    }
    reject_unknown(input, &allowed, command)?;
    if !updating && input.contains_key("chain_id") {
        return Err(parse_error(command, "create-draft must not carry chain_id"));
    }

    let mut body = Map::new();
    put_string(
        &mut body,
        "channelId",
        required_text(input, "channel_id", command, 1, 512)?,
    );
    if updating {
        put_string(
            &mut body,
            "chainId",
            required_text(input, "chain_id", command, 1, 512)?,
        );
        put_revision(
            &mut body,
            "expectedDefinitionRevision",
            required_revision(input, "expected_definition_revision", command)?,
        );
    }
    put_string(
        &mut body,
        "title",
        required_text(input, "title", command, 1, 30)?,
    );
    put_string(
        &mut body,
        "description",
        bounded_text(input, "description", command, 2_000)?,
    );
    put_string(
        &mut body,
        "example",
        bounded_text(input, "example", command, 200)?,
    );
    put_integer(
        &mut body,
        "deadlineAt",
        required_millis(input, "deadline_at", command)?,
    );
    body.insert(
        "audience".to_string(),
        validate_audience(input.get("audience"), command)?,
    );
    let categories = validate_categories(input.get("categories"), command, updating)?;
    let category_count = categories.as_array().map_or(0, Vec::len);
    body.insert("categories".to_string(), categories);
    body.insert(
        "maxCategories".to_string(),
        validate_max_categories_input(input, category_count, command)?,
    );
    put_string(
        &mut body,
        "clientMutationId",
        required_text(input, "client_mutation_id", command, 1, 512)?,
    );
    Ok(Value::Object(body))
}

/// 构造发布 body；temporaryId 由 Helix 生成，作者展示快照仅是 Go 校验前的提示。
fn build_publish(command: &str, input: &Map<String, Value>) -> Result<Value, ImError> {
    reject_unknown(
        input,
        &[
            "channel_id",
            "chain_id",
            "expected_definition_revision",
            "client_mutation_id",
            "temporary_id",
            "user_snapshot",
            "req_id",
        ],
        command,
    )?;
    let mut body = Map::new();
    put_string(
        &mut body,
        "channelId",
        required_text(input, "channel_id", command, 1, 512)?,
    );
    put_string(
        &mut body,
        "chainId",
        required_text(input, "chain_id", command, 1, 512)?,
    );
    put_revision(
        &mut body,
        "expectedDefinitionRevision",
        required_revision(input, "expected_definition_revision", command)?,
    );
    put_string(
        &mut body,
        "clientMutationId",
        required_text(input, "client_mutation_id", command, 1, 512)?,
    );
    put_string(
        &mut body,
        "temporaryId",
        required_text(input, "temporary_id", command, 1, 512)?,
    );
    copy_user_snapshot(input, &mut body, command)?;
    Ok(Value::Object(body))
}

/// 构造 get/get_mine 的频道与接龙作用域。
fn build_scope(command: &str, input: &Map<String, Value>) -> Result<Value, ImError> {
    reject_unknown(input, &["channel_id", "chain_id", "req_id"], command)?;
    let mut body = Map::new();
    put_string(
        &mut body,
        "channelId",
        required_text(input, "channel_id", command, 1, 512)?,
    );
    put_string(
        &mut body,
        "chainId",
        required_text(input, "chain_id", command, 1, 512)?,
    );
    Ok(Value::Object(body))
}

/// 构造本人分类选择集及展示提示；空数组仍是全量取消，身份始终由 Go 会话决定。
fn build_participation(command: &str, input: &Map<String, Value>) -> Result<Value, ImError> {
    reject_unknown(
        input,
        &[
            "channel_id",
            "chain_id",
            "expected_participation_revision",
            "selections",
            "client_mutation_id",
            "user_snapshot",
            "req_id",
        ],
        command,
    )?;
    let mut body = Map::new();
    put_string(
        &mut body,
        "channelId",
        required_text(input, "channel_id", command, 1, 512)?,
    );
    put_string(
        &mut body,
        "chainId",
        required_text(input, "chain_id", command, 1, 512)?,
    );
    put_revision(
        &mut body,
        "expectedParticipationRevision",
        required_revision(input, "expected_participation_revision", command)?,
    );
    body.insert(
        "selections".to_string(),
        validate_selections(input.get("selections"), command)?,
    );
    put_string(
        &mut body,
        "clientMutationId",
        required_text(input, "client_mutation_id", command, 1, 512)?,
    );
    copy_user_snapshot(input, &mut body, command)?;
    Ok(Value::Object(body))
}

/// 对齐 TEXT 的可选快照透传；只接收展示字符串，不把客户端 userId 当作 actor authority。
fn copy_user_snapshot(
    input: &Map<String, Value>,
    body: &mut Map<String, Value>,
    command: &str,
) -> Result<(), ImError> {
    let Some(value) = input.get("user_snapshot").filter(|value| !value.is_null()) else {
        return Ok(());
    };
    let snapshot = value
        .as_object()
        .ok_or_else(|| parse_error(command, "user_snapshot must be an object"))?;
    reject_unknown(
        snapshot,
        &["userId", "userName", "deptName", "orgName"],
        command,
    )?;
    for (key, value) in snapshot {
        let text = value
            .as_str()
            .ok_or_else(|| parse_error(command, format!("user_snapshot.{key} must be a string")))?;
        if text.chars().count() > 2_000 {
            return Err(parse_error(
                command,
                format!("user_snapshot.{key} exceeds 2000 characters"),
            ));
        }
    }
    // Copy once at the HTTP boundary; preserve empty strings and nested camelCase exactly.
    body.insert("userSnapshot".to_owned(), value.clone());
    Ok(())
}

/// 构造分类分页查询，cursor 保持 opaque，不由客户端解释或生成。
fn build_entries(command: &str, input: &Map<String, Value>) -> Result<Value, ImError> {
    reject_unknown(
        input,
        &[
            "channel_id",
            "chain_id",
            "category_id",
            "cursor",
            "limit",
            "req_id",
        ],
        command,
    )?;
    let mut body = Map::new();
    put_string(
        &mut body,
        "channelId",
        required_text(input, "channel_id", command, 1, 512)?,
    );
    put_string(
        &mut body,
        "chainId",
        required_text(input, "chain_id", command, 1, 512)?,
    );
    put_string(
        &mut body,
        "categoryId",
        required_text(input, "category_id", command, 1, 512)?,
    );
    if let Some(cursor) = optional_text(input, "cursor", command, 1, 8_192)? {
        put_string(&mut body, "cursor", cursor);
    }
    if let Some(limit) = optional_limit(input, command)? {
        put_integer(&mut body, "limit", limit);
    }
    Ok(Value::Object(body))
}

/// 构造 reconcile；创建对账可以在 chainId 未知时只带 mutation key。
fn build_reconcile(command: &str, input: &Map<String, Value>) -> Result<Value, ImError> {
    reject_unknown(
        input,
        &["channel_id", "chain_id", "client_mutation_id", "req_id"],
        command,
    )?;
    let mut body = Map::new();
    put_string(
        &mut body,
        "channelId",
        required_text(input, "channel_id", command, 1, 512)?,
    );
    if let Some(chain) = optional_text(input, "chain_id", command, 1, 512)? {
        put_string(&mut body, "chainId", chain);
    }
    put_string(
        &mut body,
        "clientMutationId",
        required_text(input, "client_mutation_id", command, 1, 512)?,
    );
    Ok(Value::Object(body))
}

/// 构造 close/retract 的 CAS body。
fn build_lifecycle(command: &str, input: &Map<String, Value>) -> Result<Value, ImError> {
    reject_unknown(
        input,
        &[
            "channel_id",
            "chain_id",
            "expected_revision",
            "client_mutation_id",
            "req_id",
        ],
        command,
    )?;
    let mut body = Map::new();
    put_string(
        &mut body,
        "channelId",
        required_text(input, "channel_id", command, 1, 512)?,
    );
    put_string(
        &mut body,
        "chainId",
        required_text(input, "chain_id", command, 1, 512)?,
    );
    put_revision(
        &mut body,
        "expectedRevision",
        required_revision(input, "expected_revision", command)?,
    );
    put_string(
        &mut body,
        "clientMutationId",
        required_text(input, "client_mutation_id", command, 1, 512)?,
    );
    Ok(Value::Object(body))
}

/// 校验 audience 模式；userIds 是唯一允许由调用方提供的用户身份集合。
fn validate_audience(value: Option<&Value>, command: &str) -> Result<Value, ImError> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| parse_error(command, "audience must be an object"))?;
    reject_unknown(object, &["mode", "userIds"], command)?;
    let mode = object
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| parse_error(command, "audience.mode must be a string"))?;
    match mode {
        "CHANNEL_SNAPSHOT" => {
            if object.get("userIds").is_some_and(|value| !value.is_null()) {
                return Err(parse_error(
                    command,
                    "CHANNEL_SNAPSHOT must not carry userIds",
                ));
            }
            Ok(serde_json::json!({"mode": mode}))
        }
        "SELECTED" => {
            let user_ids = object
                .get("userIds")
                .and_then(Value::as_array)
                .ok_or_else(|| parse_error(command, "SELECTED audience requires userIds"))?;
            let mut seen = HashSet::with_capacity(user_ids.len());
            for user_id in user_ids {
                let id = user_id
                    .as_str()
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        parse_error(command, "audience.userIds must be non-empty strings")
                    })?;
                if !seen.insert(id) {
                    return Err(parse_error(command, "audience.userIds must be unique"));
                }
            }
            Ok(serde_json::json!({"mode": mode, "userIds": user_ids}))
        }
        _ => Err(parse_error(command, "unknown audience.mode")),
    }
}

/// 校验草稿分类集合、clientId 唯一性和可选服务端 categoryId。
fn validate_categories(
    value: Option<&Value>,
    command: &str,
    updating: bool,
) -> Result<Value, ImError> {
    let categories = value
        .and_then(Value::as_array)
        .ok_or_else(|| parse_error(command, "categories must be an array"))?;
    if !(MIN_CATEGORIES..=MAX_CATEGORIES).contains(&categories.len()) {
        return Err(parse_error(
            command,
            "categories count must be between 2 and 10",
        ));
    }
    let mut clients = HashSet::with_capacity(categories.len());
    let mut names = HashSet::with_capacity(categories.len());
    let mut output = Vec::with_capacity(categories.len());
    for category in categories {
        let source = category
            .as_object()
            .ok_or_else(|| parse_error(command, "category must be an object"))?;
        let allowed = if updating {
            &["categoryId", "clientId", "name"][..]
        } else {
            &["clientId", "name"][..]
        };
        reject_unknown(source, allowed, command)?;
        let client_id = required_nested_string(source, "clientId", command)?;
        if !clients.insert(client_id.to_owned()) {
            return Err(parse_error(command, "category clientId must be unique"));
        }
        let name = normalized_text(
            required_nested_string(source, "name", command)?,
            1,
            30,
            command,
            "category name",
        )?;
        if !names.insert(name.clone()) {
            return Err(parse_error(command, "category names must be unique"));
        }
        let mut item = Map::new();
        if let Some(category_id) = source.get("categoryId") {
            let category_id = category_id
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| parse_error(command, "categoryId must be a non-empty string"))?;
            item.insert(
                "categoryId".to_string(),
                Value::String(category_id.to_owned()),
            );
        }
        item.insert("clientId".to_string(), Value::String(client_id.to_owned()));
        item.insert("name".to_string(), Value::String(name));
        output.push(Value::Object(item));
    }
    Ok(Value::Array(output))
}

/// 新分类接龙必须显式声明参与上限。
fn validate_max_categories_input(
    input: &Map<String, Value>,
    category_count: usize,
    command: &str,
) -> Result<Value, ImError> {
    let value = input
        .get("max_categories")
        .ok_or_else(|| parse_error(command, "max_categories is required"))?;
    let max_categories = value
        .as_u64()
        .filter(|value| *value >= 1 && (*value as usize) <= category_count)
        .ok_or_else(|| {
            parse_error(
                command,
                "max_categories must be an integer between 1 and categories count",
            )
        })?;
    Ok(Value::Number(max_categories.into()))
}

/// 校验完整本人选择集；空数组表示取消全部当前分类记录。
fn validate_selections(value: Option<&Value>, command: &str) -> Result<Value, ImError> {
    let selections = value
        .and_then(Value::as_array)
        .ok_or_else(|| parse_error(command, "selections must be an array"))?;
    if selections.len() > MAX_CATEGORIES {
        return Err(parse_error(command, "selections count must not exceed 10"));
    }
    let mut category_ids = HashSet::with_capacity(selections.len());
    let mut output = Vec::with_capacity(selections.len());
    for selection in selections {
        let source = selection
            .as_object()
            .ok_or_else(|| parse_error(command, "selection must be an object"))?;
        reject_unknown(source, &["categoryId", "content"], command)?;
        let category_id = required_nested_string(source, "categoryId", command)?;
        if !category_ids.insert(category_id) {
            return Err(parse_error(command, "selection categoryId must be unique"));
        }
        let content = normalized_text(
            required_nested_string(source, "content", command)?,
            1,
            200,
            command,
            "selection content",
        )?;
        output.push(serde_json::json!({"categoryId": category_id, "content": content}));
    }
    Ok(Value::Array(output))
}

/// 从 object 取出必填非空文本并按 Unicode 码点限制长度。
fn required_text(
    object: &Map<String, Value>,
    key: &str,
    command: &str,
    min: usize,
    max: usize,
) -> Result<String, ImError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| parse_error(command, format!("{key} must be a string")))?;
    normalized_text(value, min, max, command, key)
}

/// 取可选文本；null/缺省不写入 wire，非 null 必须是边界内字符串。
fn optional_text(
    object: &Map<String, Value>,
    key: &str,
    command: &str,
    min: usize,
    max: usize,
) -> Result<Option<String>, ImError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => normalized_text(value, min, max, command, key).map(Some),
        Some(_) => Err(parse_error(command, format!("{key} must be a string"))),
    }
}

/// 取必填 description/example，允许空串但限制最大码点数。
fn bounded_text(
    object: &Map<String, Value>,
    key: &str,
    command: &str,
    max: usize,
) -> Result<String, ImError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| parse_error(command, format!("{key} must be a string")))?;
    normalized_text(value, 0, max, command, key)
}

/// 读取规范非负十进制 revision，拒绝 number、空串和前导零。
fn required_revision(
    object: &Map<String, Value>,
    key: &str,
    command: &str,
) -> Result<String, ImError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| parse_error(command, format!("{key} must be a decimal string")))?;
    canonical_revision(value)
        .map(str::to_owned)
        .map_err(|reason| parse_error(command, format!("{key}: {reason}")))
}

/// 读取 Unix 毫秒整数；时间由服务端最终裁决，客户端不得用本地 now 授权。
fn required_millis(object: &Map<String, Value>, key: &str, command: &str) -> Result<i64, ImError> {
    let value = object.get(key).and_then(Value::as_i64).ok_or_else(|| {
        parse_error(
            command,
            format!("{key} must be an integer millisecond timestamp"),
        )
    })?;
    if value < 0 {
        return Err(parse_error(command, format!("{key} must not be negative")));
    }
    Ok(value)
}

/// 读取 1..=100 分页大小；服务端仍绑定 cursor 的 scope/version。
fn optional_limit(object: &Map<String, Value>, command: &str) -> Result<Option<i64>, ImError> {
    match object.get("limit") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let limit = value
                .as_i64()
                .ok_or_else(|| parse_error(command, "limit must be an integer"))?;
            if !(1..=MAX_ENTRIES_PAGE as i64).contains(&limit) {
                return Err(parse_error(command, "limit must be between 1 and 100"));
            }
            Ok(Some(limit))
        }
    }
}

/// 取嵌套必填身份字符串；该身份是业务对象 ID，不是 actor/tenant 注入。
fn required_nested_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    command: &str,
) -> Result<&'a str, ImError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| parse_error(command, format!("{key} must be a non-empty string")))
}

/// 统一做 trim 后的 Unicode 码点边界检查；wire 使用 trim 后文本。
fn normalized_text(
    value: &str,
    min: usize,
    max: usize,
    command: &str,
    field: &str,
) -> Result<String, ImError> {
    let trimmed = value.trim();
    let count = trimmed.chars().count();
    if count < min || count > max {
        return Err(parse_error(
            command,
            format!("{field} length must be between {min} and {max} Unicode code points"),
        ));
    }
    Ok(trimmed.to_owned())
}

/// 校验并返回规范非负十进制字符串。
fn canonical_revision(value: &str) -> Result<&str, &'static str> {
    let bytes = value.as_bytes();
    let valid = value == "0"
        || (bytes
            .first()
            .is_some_and(|byte| (b'1'..=b'9').contains(byte))
            && bytes[1..].iter().all(u8::is_ascii_digit));
    if valid && value.parse::<u64>().is_ok() {
        Ok(value)
    } else {
        Err("must be a canonical non-negative decimal string")
    }
}

/// 校验 request correlation 的类型，即使它不会进入 Go body。
fn check_req_id(object: &Map<String, Value>, command: &str) -> Result<(), ImError> {
    if let Some(value) = object.get("req_id") {
        if value.as_str().is_none_or(|value| value.is_empty()) {
            return Err(parse_error(command, "req_id must be a non-empty string"));
        }
    }
    Ok(())
}

/// 把输入 object 转为稳定的业务错误，而不让外部 JSON 触发 panic。
fn object<'a>(value: &'a Value, command: &str) -> Result<&'a Map<String, Value>, ImError> {
    value
        .as_object()
        .ok_or_else(|| parse_error(command, "payload must be an object"))
}

/// 拒绝未登记字段，并明确挡住 actor/tenant/计数等服务端权威输入。
fn reject_unknown(
    object: &Map<String, Value>,
    allowed: &[&str],
    command: &str,
) -> Result<(), ImError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(parse_error(
                command,
                format!("unknown or authority field '{key}'"),
            ));
        }
    }
    Ok(())
}

/// 将已验证文本放进 Go body。
fn put_string(body: &mut Map<String, Value>, key: &str, value: String) {
    body.insert(key.to_string(), Value::String(value));
}

/// 将已验证 revision 放进 Go body。
fn put_revision(body: &mut Map<String, Value>, key: &str, value: String) {
    body.insert(key.to_string(), Value::String(value));
}

/// 将已验证整数放进 Go body。
fn put_integer(body: &mut Map<String, Value>, key: &str, value: i64) {
    body.insert(key.to_string(), Value::Number(value.into()));
}

/// 递归检查已知 authority 字段、schemaVersion、身份 scope 和有界数组。
fn validate_node(
    value: &Value,
    path: &str,
    expected_channel: &str,
    expected_chain: Option<&str>,
    is_private: bool,
    seen_channel: &mut bool,
    seen_chain: &mut bool,
) -> Result<(), ImError> {
    match value {
        Value::Array(items) => {
            let max = if path.ends_with(".entries") && path.contains("categories") {
                5
            } else if path.ends_with(".entries") {
                MAX_ENTRIES_PAGE
            } else {
                usize::MAX
            };
            if items.len() > max {
                return Err(authority_error(format!("{path} exceeds bounded array")));
            }
            for (index, item) in items.iter().enumerate() {
                validate_node(
                    item,
                    &format!("{path}[{index}]"),
                    expected_channel,
                    expected_chain,
                    is_private,
                    seen_channel,
                    seen_chain,
                )?;
            }
        }
        Value::Object(object) => {
            let category_post = object_is_category_post(object);
            for (key, child) in object {
                let child_path = format!("{path}.{key}");
                if is_forbidden_authority_response_key(key) {
                    return Err(authority_error(format!(
                        "forbidden authority field {child_path}"
                    )));
                }
                match key.as_str() {
                    "schemaVersion" => {
                        if child.as_u64() != Some(1) {
                            return Err(authority_error(format!(
                                "unknown schemaVersion at {child_path}"
                            )));
                        }
                    }
                    "channelId" => {
                        let actual = child
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .ok_or_else(|| authority_error(format!("invalid {child_path}")))?;
                        *seen_channel = true;
                        if actual != expected_channel {
                            return Err(authority_error(format!(
                                "channelId mismatch at {child_path}"
                            )));
                        }
                    }
                    "chainId" => {
                        let actual = child
                            .as_str()
                            .filter(|value| !value.is_empty())
                            .ok_or_else(|| authority_error(format!("invalid {child_path}")))?;
                        *seen_chain = true;
                        if let Some(expected) = expected_chain {
                            if actual != expected {
                                return Err(authority_error(format!(
                                    "chainId mismatch at {child_path}"
                                )));
                            }
                        }
                    }
                    "revision"
                    | "definitionRevision"
                    | "participationRevision"
                    | "categoryRevision"
                    | "position"
                    | "eventSeq" => {
                        if !(key == "eventSeq"
                            && (child.is_null() || (category_post && child.as_u64().is_some())))
                            && child
                                .as_str()
                                .is_none_or(|value| canonical_revision(value).is_err())
                        {
                            return Err(authority_error(format!(
                                "invalid decimal version at {child_path}"
                            )));
                        }
                    }
                    "eventId" | "operationId" | "clientMutationId" | "entryId" | "categoryId"
                    | "postId" | "anchorPostId" | "userId" => {
                        if !child.is_null() && child.as_str().is_none_or(str::is_empty) {
                            return Err(authority_error(format!(
                                "invalid identity at {child_path}"
                            )));
                        }
                    }
                    "deadlineAt" | "createdAt" | "updatedAt" => {
                        if child.as_i64().is_none_or(|value| value < 0) {
                            return Err(authority_error(format!(
                                "invalid millisecond time at {child_path}"
                            )));
                        }
                    }
                    "audienceCount" | "participantCount" | "categoryCount" | "entryCount"
                    | "displayNumber" | "ordinal" => {
                        if child.as_u64().is_none() {
                            return Err(authority_error(format!(
                                "invalid non-negative count at {child_path}"
                            )));
                        }
                    }
                    "status" => {
                        if let Some(status) = child.as_str() {
                            if !matches!(
                                status,
                                "DRAFT"
                                    | "OPEN"
                                    | "CLOSED"
                                    | "RETRACTED"
                                    | "ACTIVE"
                                    | "CANCELLED"
                                    | "CONFIRMED"
                                    | "REJECTED"
                                    | "PENDING"
                                    | "NOT_FOUND"
                            ) {
                                return Err(authority_error(format!(
                                    "unknown status at {child_path}"
                                )));
                            }
                        }
                    }
                    "type" => {
                        if category_post && child.as_str() != Some("CATEGORY_CHAIN") {
                            return Err(authority_error(format!(
                                "post type is not CATEGORY_CHAIN at {child_path}"
                            )));
                        }
                    }
                    "categoryChain" => {
                        if !child.is_object() {
                            return Err(authority_error(format!(
                                "categoryChain must be an object at {child_path}"
                            )));
                        }
                    }
                    "myParticipation" if !is_private && !child.is_null() => {
                        return Err(authority_error("public authority leaked myParticipation"));
                    }
                    "audience" if !is_private => {
                        return Err(authority_error("public authority leaked audience"));
                    }
                    "props" => {
                        if !child.is_object() {
                            return Err(authority_error(format!("invalid props at {child_path}")));
                        }
                    }
                    _ => {}
                }
                validate_node(
                    child,
                    &child_path,
                    expected_channel,
                    expected_chain,
                    is_private,
                    seen_channel,
                    seen_chain,
                )?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

/// 根据 Go contract 识别完整响应联合，拒绝只带一个身份字段的伪 authority。
fn validate_response_shape(
    root: &Map<String, Value>,
    channel: &str,
    chain: Option<&str>,
    is_private: bool,
) -> Result<(), ImError> {
    if root.contains_key("outcome") {
        return validate_mutation_result(root, channel, chain, is_private);
    }
    if root.contains_key("state") {
        return validate_reconcile_result_shape(root, channel, chain, is_private);
    }
    if has_all(root, &["chain", "card", "draft", "post"]) {
        return validate_get_result(root, channel, chain, is_private);
    }
    if has_all(
        root,
        &[
            "chainId",
            "participationRevision",
            "entries",
            "canParticipate",
            "canEdit",
            "canCancel",
            "denialReason",
        ],
    ) {
        return validate_my_participation(root, chain);
    }
    if has_all(
        root,
        &[
            "chainId",
            "categoryId",
            "categoryRevision",
            "entries",
            "nextCursor",
            "hasMore",
        ],
    ) {
        return validate_entries_page(root, chain);
    }
    if has_all(
        root,
        &[
            "schemaVersion",
            "channelId",
            "chainId",
            "anchorPostId",
            "temporaryId",
            "revision",
            "eventId",
            "eventSeq",
            "card",
        ],
    ) {
        return validate_projection(root, channel, chain);
    }
    if has_all(
        root,
        &[
            "schemaVersion",
            "chainId",
            "revision",
            "status",
            "title",
            "description",
            "example",
            "deadlineAt",
            "audienceCount",
            "participantCount",
            "categoryCount",
            "categories",
        ],
    ) {
        validate_card(root, channel, chain)?;
        if !is_private && root.contains_key("audience") {
            return Err(authority_error("public card leaked audience"));
        }
        return Ok(());
    }
    Err(authority_error("unknown category-chain response shape"))
}

/// 校验完整 MutationResult，保证 mutation 终态和 nullable 字段没有被省略。
fn validate_mutation_result(
    root: &Map<String, Value>,
    channel: &str,
    chain: Option<&str>,
    is_private: bool,
) -> Result<(), ImError> {
    let required = [
        "outcome",
        "commandKind",
        "clientMutationId",
        "operationId",
        "chainId",
        "revision",
        "eventId",
        "eventSeq",
        "draft",
        "card",
        "post",
        "myParticipation",
    ];
    require_exact_keys(root, &required, "MutationResult")?;
    if root.get("outcome").and_then(Value::as_str) != Some("CONFIRMED") {
        return Err(authority_error("MutationResult outcome must be CONFIRMED"));
    }
    let command_kind = required_nonempty_str(root, "commandKind", "MutationResult")?;
    if !matches!(
        command_kind,
        "create-draft"
            | "update-draft"
            | "publish"
            | "set-participation"
            | "close"
            | "retract"
            | "category_chain_create_draft"
            | "category_chain_update_draft"
            | "category_chain_publish"
            | "category_chain_set_participation"
            | "category_chain_close"
            | "category_chain_retract"
    ) {
        return Err(authority_error("MutationResult commandKind is unknown"));
    }
    let result_chain = required_nonempty_str(root, "chainId", "MutationResult")?;
    if chain.is_some_and(|expected| expected != result_chain) {
        return Err(authority_error("MutationResult chainId mismatch"));
    }
    require_revision(root, "revision", "MutationResult")?;
    required_nonempty_str(root, "clientMutationId", "MutationResult")?;
    required_nonempty_str(root, "operationId", "MutationResult")?;
    require_nonempty_string_or_null(root, "eventId", "MutationResult")?;
    require_revision_or_null(root, "eventSeq", "MutationResult")?;

    let draft = required_value(root, "draft", "MutationResult")?;
    let card = required_value(root, "card", "MutationResult")?;
    let post = required_value(root, "post", "MutationResult")?;
    let mine = required_value(root, "myParticipation", "MutationResult")?;
    match command_kind {
        "create-draft"
        | "update-draft"
        | "category_chain_create_draft"
        | "category_chain_update_draft" => {
            validate_draft_projection(
                draft
                    .as_object()
                    .ok_or_else(|| authority_error("draft is required for draft mutation"))?,
                channel,
                Some(result_chain),
            )?;
            require_null(card, "card", "draft MutationResult")?;
            require_null(post, "post", "draft MutationResult")?;
            require_null(mine, "myParticipation", "draft MutationResult")?;
        }
        "set-participation" | "category_chain_set_participation" => {
            let card = card
                .as_object()
                .ok_or_else(|| authority_error("card is required for participation mutation"))?;
            validate_card(card, channel, Some(result_chain))?;
            validate_post(
                post.as_object().ok_or_else(|| {
                    authority_error("post is required for participation mutation")
                })?,
                channel,
                Some(result_chain),
            )?;
            let mine = mine.as_object().ok_or_else(|| {
                authority_error("myParticipation is required for participation mutation")
            })?;
            validate_my_participation(mine, Some(result_chain))?;
            if mine.get("maxCategories") != card.get("maxCategories") {
                return Err(authority_error(
                    "myParticipation maxCategories differs from card",
                ));
            }
            require_null(draft, "draft", "participation MutationResult")?;
        }
        "publish"
        | "close"
        | "retract"
        | "category_chain_publish"
        | "category_chain_close"
        | "category_chain_retract" => {
            validate_card(
                card.as_object()
                    .ok_or_else(|| authority_error("card is required for lifecycle mutation"))?,
                channel,
                Some(result_chain),
            )?;
            validate_post(
                post.as_object()
                    .ok_or_else(|| authority_error("post is required for lifecycle mutation"))?,
                channel,
                Some(result_chain),
            )?;
            require_null(draft, "draft", "lifecycle MutationResult")?;
            require_null(mine, "myParticipation", "lifecycle MutationResult")?;
        }
        _ => return Err(authority_error("unsupported MutationResult commandKind")),
    }
    if !is_private && !mine.is_null() {
        return Err(authority_error(
            "public MutationResult leaked myParticipation",
        ));
    }
    Ok(())
}

/// 校验 get 的公开/草稿分支，四个字段必须都存在且只允许一条分支非空。
fn validate_get_result(
    root: &Map<String, Value>,
    channel: &str,
    chain: Option<&str>,
    is_private: bool,
) -> Result<(), ImError> {
    require_exact_keys(root, &["chain", "card", "draft", "post"], "get result")?;
    let chain_value = required_value(root, "chain", "get result")?;
    let chain_object = chain_value
        .as_object()
        .ok_or_else(|| authority_error("get result chain is required"))?;
    let chain_id = validate_chain(chain_object, channel, chain)?;
    let card = required_value(root, "card", "get result")?;
    let draft = required_value(root, "draft", "get result")?;
    let post = required_value(root, "post", "get result")?;
    if draft.is_object() {
        if !card.is_null() || !post.is_null() {
            return Err(authority_error(
                "draft get result must not include card/post",
            ));
        }
        validate_draft_projection(
            draft
                .as_object()
                .ok_or_else(|| authority_error("draft must be an object"))?,
            channel,
            Some(chain_id),
        )?;
    } else if !draft.is_null() {
        return Err(authority_error("draft must be object or null"));
    } else {
        let card = card
            .as_object()
            .ok_or_else(|| authority_error("public get result requires card"))?;
        validate_card(card, channel, Some(chain_id))?;
        if chain_object.get("maxCategories") != card.get("maxCategories") {
            return Err(authority_error(
                "CategoryChain maxCategories differs from card",
            ));
        }
        validate_post(
            post.as_object()
                .ok_or_else(|| authority_error("public get result requires post"))?,
            channel,
            Some(chain_id),
        )?;
    }
    if !is_private && root.contains_key("draft") && draft.is_object() {
        return Err(authority_error("public get result leaked draft"));
    }
    Ok(())
}

/// 校验本人私有参与状态的完整字段和当前记录集合。
fn validate_my_participation(
    root: &Map<String, Value>,
    chain: Option<&str>,
) -> Result<(), ImError> {
    require_known_keys(
        root,
        &[
            "chainId",
            "participationRevision",
            "entries",
            "canParticipate",
            "canEdit",
            "canCancel",
            "denialReason",
        ],
        &["maxCategories"],
        "myParticipation",
    )?;
    let chain_id = required_nonempty_str(root, "chainId", "myParticipation")?;
    if chain.is_some_and(|expected| expected != chain_id) {
        return Err(authority_error("myParticipation chainId mismatch"));
    }
    require_revision(root, "participationRevision", "myParticipation")?;
    require_bool(root, "canParticipate", "myParticipation")?;
    require_bool(root, "canEdit", "myParticipation")?;
    require_bool(root, "canCancel", "myParticipation")?;
    require_nonempty_string_or_null(root, "denialReason", "myParticipation")?;
    let entries = required_array(root, "entries", "myParticipation")?;
    if entries.len() > MAX_CATEGORIES {
        return Err(authority_error("myParticipation has too many entries"));
    }
    validate_max_categories(root, MAX_CATEGORIES, "myParticipation")?;
    for entry in entries {
        validate_entry(
            entry
                .as_object()
                .ok_or_else(|| authority_error("myParticipation entry must be object"))?,
            Some(chain_id),
        )?;
    }
    Ok(())
}

/// 校验分类详情页的完整分页游标和 preview 条目。
fn validate_entries_page(root: &Map<String, Value>, chain: Option<&str>) -> Result<(), ImError> {
    require_exact_keys(
        root,
        &[
            "chainId",
            "categoryId",
            "categoryRevision",
            "entries",
            "nextCursor",
            "hasMore",
        ],
        "entries page",
    )?;
    let chain_id = required_nonempty_str(root, "chainId", "entries page")?;
    if chain.is_some_and(|expected| expected != chain_id) {
        return Err(authority_error("entries page chainId mismatch"));
    }
    required_nonempty_str(root, "categoryId", "entries page")?;
    require_revision(root, "categoryRevision", "entries page")?;
    require_bool(root, "hasMore", "entries page")?;
    require_nonempty_string_or_null(root, "nextCursor", "entries page")?;
    let entries = required_array(root, "entries", "entries page")?;
    if entries.len() > MAX_ENTRIES_PAGE {
        return Err(authority_error("entries page exceeds limit 100"));
    }
    for entry in entries {
        validate_entry_preview(
            entry
                .as_object()
                .ok_or_else(|| authority_error("entry preview must be object"))?,
        )?;
    }
    Ok(())
}

/// 校验 WS category-chain projection 的完整共享卡片。
fn validate_projection(
    root: &Map<String, Value>,
    channel: &str,
    chain: Option<&str>,
) -> Result<(), ImError> {
    require_exact_keys(
        root,
        &[
            "schemaVersion",
            "channelId",
            "chainId",
            "anchorPostId",
            "temporaryId",
            "revision",
            "eventId",
            "eventSeq",
            "card",
        ],
        "projection",
    )?;
    require_schema_version(root, "projection")?;
    let actual_channel = required_nonempty_str(root, "channelId", "projection")?;
    if actual_channel != channel {
        return Err(authority_error("projection channelId mismatch"));
    }
    let actual_chain = required_nonempty_str(root, "chainId", "projection")?;
    if chain.is_some_and(|expected| expected != actual_chain) {
        return Err(authority_error("projection chainId mismatch"));
    }
    require_nonempty_string_or_null(root, "anchorPostId", "projection")?;
    required_nonempty_str(root, "temporaryId", "projection")?;
    require_revision(root, "revision", "projection")?;
    require_nonempty_string_or_null(root, "eventId", "projection")?;
    require_revision(root, "eventSeq", "projection")?;
    validate_card(
        required_value(root, "card", "projection")?
            .as_object()
            .ok_or_else(|| authority_error("projection card must be object"))?,
        channel,
        Some(actual_chain),
    )
}

/// 校验共享 Card 的完整绝对态、分类数量和每类五条预览上限。
fn validate_card(
    card: &Map<String, Value>,
    _channel: &str,
    chain: Option<&str>,
) -> Result<(), ImError> {
    require_known_keys(
        card,
        &[
            "schemaVersion",
            "chainId",
            "revision",
            "status",
            "title",
            "description",
            "example",
            "deadlineAt",
            "audienceCount",
            "participantCount",
            "categoryCount",
            "categories",
        ],
        &["maxCategories"],
        "CategoryChainCard",
    )?;
    require_schema_version(card, "CategoryChainCard")?;
    let actual_chain = required_nonempty_str(card, "chainId", "CategoryChainCard")?;
    if chain.is_some_and(|expected| expected != actual_chain) {
        return Err(authority_error("card chainId mismatch"));
    }
    require_revision(card, "revision", "CategoryChainCard")?;
    require_status(card, "status", &["OPEN", "CLOSED", "RETRACTED"])?;
    validate_text_field(card, "title", 1, 30, "CategoryChainCard")?;
    validate_text_field(card, "description", 0, 2_000, "CategoryChainCard")?;
    validate_text_field(card, "example", 0, 200, "CategoryChainCard")?;
    require_nonnegative_i64(card, "deadlineAt", "CategoryChainCard")?;
    require_nonnegative_u64(card, "audienceCount", "CategoryChainCard")?;
    require_nonnegative_u64(card, "participantCount", "CategoryChainCard")?;
    let category_count =
        require_nonnegative_u64(card, "categoryCount", "CategoryChainCard")? as usize;
    if category_count > MAX_CATEGORIES {
        return Err(authority_error("CategoryChainCard has too many categories"));
    }
    let categories = required_array(card, "categories", "CategoryChainCard")?;
    if categories.len() != category_count {
        return Err(authority_error("CategoryChainCard categoryCount mismatch"));
    }
    if required_status(card, "status", &["OPEN", "CLOSED", "RETRACTED"])? == "RETRACTED" {
        if card.contains_key("maxCategories") {
            return Err(authority_error(
                "retracted CategoryChainCard must omit maxCategories",
            ));
        }
    } else {
        validate_max_categories(card, category_count, "CategoryChainCard")?;
    }
    let mut category_ids = HashSet::with_capacity(categories.len());
    for category in categories {
        let object = category
            .as_object()
            .ok_or_else(|| authority_error("CategoryChainCard category must be object"))?;
        validate_category_preview(object, actual_chain, &mut category_ids)?;
    }
    Ok(())
}

/// 校验公开 Category preview 的 tone、revision、统计和 entry preview。
fn validate_category_preview(
    category: &Map<String, Value>,
    chain_id: &str,
    seen_ids: &mut HashSet<String>,
) -> Result<(), ImError> {
    require_exact_keys(
        category,
        &[
            "categoryId",
            "name",
            "ordinal",
            "tone",
            "revision",
            "participantCount",
            "entries",
            "hasMore",
        ],
        "CategoryPreview",
    )?;
    let category_id = required_nonempty_str(category, "categoryId", "CategoryPreview")?;
    if !seen_ids.insert(category_id.to_owned()) {
        return Err(authority_error("duplicate categoryId in card"));
    }
    validate_text_field(category, "name", 1, 30, "CategoryPreview")?;
    require_nonnegative_u64(category, "ordinal", "CategoryPreview")?;
    let tone = required_nonempty_str(category, "tone", "CategoryPreview")?;
    if !matches!(tone, "purple" | "blue" | "green" | "yellow") {
        return Err(authority_error("unknown category tone"));
    }
    require_revision(category, "revision", "CategoryPreview")?;
    require_nonnegative_u64(category, "participantCount", "CategoryPreview")?;
    require_bool(category, "hasMore", "CategoryPreview")?;
    let entries = required_array(category, "entries", "CategoryPreview")?;
    if entries.len() > 5 {
        return Err(authority_error("CategoryPreview exceeds five entries"));
    }
    for entry in entries {
        validate_entry_preview(
            entry
                .as_object()
                .ok_or_else(|| authority_error("CategoryPreview entry must be object"))?,
        )?;
    }
    if chain_id.is_empty() {
        return Err(authority_error("empty card chainId"));
    }
    Ok(())
}

/// 校验服务端 EntryPreview；缺少用户资料时 Go 保留空 displayName，身份仍由非空 userId 校验。
fn validate_entry_preview(entry: &Map<String, Value>) -> Result<(), ImError> {
    require_exact_keys(
        entry,
        &[
            "entryId",
            "userId",
            "displayName",
            "departmentName",
            "content",
            "createdAt",
            "updatedAt",
            "displayNumber",
        ],
        "CategoryEntryPreview",
    )?;
    required_nonempty_str(entry, "entryId", "CategoryEntryPreview")?;
    required_nonempty_str(entry, "userId", "CategoryEntryPreview")?;
    validate_text_field(entry, "displayName", 0, 2_000, "CategoryEntryPreview")?;
    validate_text_field(entry, "departmentName", 0, 2_000, "CategoryEntryPreview")?;
    validate_text_field(entry, "content", 1, 200, "CategoryEntryPreview")?;
    require_nonnegative_i64(entry, "createdAt", "CategoryEntryPreview")?;
    require_nonnegative_i64(entry, "updatedAt", "CategoryEntryPreview")?;
    require_nonnegative_u64(entry, "displayNumber", "CategoryEntryPreview")?;
    Ok(())
}

/// 校验本人 current Entry，允许 CANCELLED 仅用于服务端兼容回读。
fn validate_entry(entry: &Map<String, Value>, chain: Option<&str>) -> Result<(), ImError> {
    require_exact_keys(
        entry,
        &[
            "id",
            "chainId",
            "categoryId",
            "userId",
            "content",
            "position",
            "revision",
            "status",
            "createdAt",
            "updatedAt",
        ],
        "CategoryEntry",
    )?;
    let actual_chain = required_nonempty_str(entry, "chainId", "CategoryEntry")?;
    if chain.is_some_and(|expected| expected != actual_chain) {
        return Err(authority_error("CategoryEntry chainId mismatch"));
    }
    required_nonempty_str(entry, "id", "CategoryEntry")?;
    required_nonempty_str(entry, "categoryId", "CategoryEntry")?;
    required_nonempty_str(entry, "userId", "CategoryEntry")?;
    validate_text_field(entry, "content", 1, 200, "CategoryEntry")?;
    require_revision(entry, "position", "CategoryEntry")?;
    require_revision(entry, "revision", "CategoryEntry")?;
    require_status(entry, "status", &["ACTIVE", "CANCELLED"])?;
    require_nonnegative_i64(entry, "createdAt", "CategoryEntry")?;
    require_nonnegative_i64(entry, "updatedAt", "CategoryEntry")?;
    Ok(())
}

/// 校验草稿 projection 的私有 audience 和 clientId→categoryId 映射。
fn validate_draft_projection(
    draft: &Map<String, Value>,
    channel: &str,
    chain: Option<&str>,
) -> Result<(), ImError> {
    require_exact_keys(
        draft,
        &["chain", "categories", "audience", "categoryIdMap"],
        "DraftProjection",
    )?;
    let chain_object = required_value(draft, "chain", "DraftProjection")?
        .as_object()
        .ok_or_else(|| authority_error("DraftProjection chain must be object"))?;
    let chain_id = validate_chain(chain_object, channel, chain)?;
    let categories = required_array(draft, "categories", "DraftProjection")?;
    if !(MIN_CATEGORIES..=MAX_CATEGORIES).contains(&categories.len()) {
        return Err(authority_error(
            "DraftProjection category count out of range",
        ));
    }
    let mut ids = HashSet::with_capacity(categories.len());
    for category in categories {
        validate_category_definition(
            category
                .as_object()
                .ok_or_else(|| authority_error("DraftProjection category must be object"))?,
            chain_id,
            &mut ids,
        )?;
    }
    validate_max_categories(chain_object, categories.len(), "CategoryChain")?;
    validate_audience_projection(
        required_value(draft, "audience", "DraftProjection")?
            .as_object()
            .ok_or_else(|| authority_error("DraftProjection audience must be object"))?,
        chain_id,
    )?;
    let id_map = required_array(draft, "categoryIdMap", "DraftProjection")?;
    if id_map.len() != categories.len() {
        return Err(authority_error("DraftProjection categoryIdMap mismatch"));
    }
    for item in id_map {
        let item = item
            .as_object()
            .ok_or_else(|| authority_error("categoryIdMap item must be object"))?;
        require_exact_keys(item, &["clientId", "categoryId"], "categoryIdMap")?;
        required_nonempty_str(item, "clientId", "categoryIdMap")?;
        required_nonempty_str(item, "categoryId", "categoryIdMap")?;
    }
    Ok(())
}

/// 校验 CategoryChain 定义的 scope、状态和十进制版本。
fn validate_chain<'a>(
    chain: &'a Map<String, Value>,
    channel: &str,
    expected_chain: Option<&str>,
) -> Result<&'a str, ImError> {
    require_known_keys(
        chain,
        &[
            "id",
            "channelId",
            "anchorPostId",
            "creatorId",
            "title",
            "description",
            "example",
            "deadlineAt",
            "status",
            "closeReason",
            "revision",
            "definitionRevision",
            "createdAt",
            "updatedAt",
        ],
        &["maxCategories"],
        "CategoryChain",
    )?;
    let id = required_nonempty_str(chain, "id", "CategoryChain")?;
    if expected_chain.is_some_and(|expected| expected != id) {
        return Err(authority_error("CategoryChain id mismatch"));
    }
    if required_nonempty_str(chain, "channelId", "CategoryChain")? != channel {
        return Err(authority_error("CategoryChain channelId mismatch"));
    }
    require_nonempty_string_or_null(chain, "anchorPostId", "CategoryChain")?;
    required_nonempty_str(chain, "creatorId", "CategoryChain")?;
    validate_text_field(chain, "title", 1, 30, "CategoryChain")?;
    validate_text_field(chain, "description", 0, 2_000, "CategoryChain")?;
    validate_text_field(chain, "example", 0, 200, "CategoryChain")?;
    require_nonnegative_i64(chain, "deadlineAt", "CategoryChain")?;
    let status = required_status(chain, "status", &["DRAFT", "OPEN", "CLOSED", "RETRACTED"])?;
    require_close_reason(chain)?;
    require_revision(chain, "revision", "CategoryChain")?;
    require_revision(chain, "definitionRevision", "CategoryChain")?;
    require_nonnegative_i64(chain, "createdAt", "CategoryChain")?;
    require_nonnegative_i64(chain, "updatedAt", "CategoryChain")?;
    if status == "RETRACTED" {
        if chain.contains_key("maxCategories") {
            return Err(authority_error(
                "retracted CategoryChain must omit maxCategories",
            ));
        }
    } else {
        validate_max_categories(chain, MAX_CATEGORIES, "CategoryChain")?;
    }
    Ok(id)
}

/// 校验 DraftProjection 中服务端确认的分类定义。
fn validate_category_definition(
    category: &Map<String, Value>,
    chain_id: &str,
    seen_ids: &mut HashSet<String>,
) -> Result<(), ImError> {
    require_exact_keys(
        category,
        &["id", "chainId", "name", "ordinal", "tone", "revision"],
        "Category",
    )?;
    let id = required_nonempty_str(category, "id", "Category")?;
    if !seen_ids.insert(id.to_owned()) {
        return Err(authority_error("duplicate Category id"));
    }
    if required_nonempty_str(category, "chainId", "Category")? != chain_id {
        return Err(authority_error("Category chainId mismatch"));
    }
    validate_text_field(category, "name", 1, 30, "Category")?;
    require_nonnegative_u64(category, "ordinal", "Category")?;
    let tone = required_nonempty_str(category, "tone", "Category")?;
    if !matches!(tone, "purple" | "blue" | "green" | "yellow") {
        return Err(authority_error("unknown Category tone"));
    }
    require_revision(category, "revision", "Category")?;
    Ok(())
}

/// 校验 DraftProjection 的冻结 audience snapshot。
fn validate_audience_projection(
    audience: &Map<String, Value>,
    chain_id: &str,
) -> Result<(), ImError> {
    require_exact_keys(
        audience,
        &["chainId", "mode", "memberIds", "revision"],
        "CategoryAudience",
    )?;
    if required_nonempty_str(audience, "chainId", "CategoryAudience")? != chain_id {
        return Err(authority_error("CategoryAudience chainId mismatch"));
    }
    let mode = required_nonempty_str(audience, "mode", "CategoryAudience")?;
    if !matches!(mode, "CHANNEL_SNAPSHOT" | "SELECTED") {
        return Err(authority_error("unknown CategoryAudience mode"));
    }
    require_revision(audience, "revision", "CategoryAudience")?;
    let members = required_array(audience, "memberIds", "CategoryAudience")?;
    for member in members {
        if member.as_str().is_none_or(str::is_empty) {
            return Err(authority_error(
                "CategoryAudience memberIds must be strings",
            ));
        }
    }
    Ok(())
}

/// 校验 reconcile 的四态结果、可选 mutation result 和业务 error。
fn validate_reconcile_result_shape(
    root: &Map<String, Value>,
    channel: &str,
    chain: Option<&str>,
    is_private: bool,
) -> Result<(), ImError> {
    require_exact_keys(
        root,
        &[
            "state",
            "clientMutationId",
            "operationId",
            "result",
            "error",
        ],
        "ReconcileResult",
    )?;
    let state = required_status(
        root,
        "state",
        &["CONFIRMED", "REJECTED", "PENDING", "NOT_FOUND"],
    )?;
    required_nonempty_str(root, "clientMutationId", "ReconcileResult")?;
    require_nonempty_string_or_null(root, "operationId", "ReconcileResult")?;
    let result = required_value(root, "result", "ReconcileResult")?;
    if state == "CONFIRMED"
        && (result.get("clientMutationId") != root.get("clientMutationId")
            || result.get("operationId") != root.get("operationId"))
    {
        return Err(authority_error("reconcile operation correlation mismatch"));
    }
    if result.is_object() {
        validate_mutation_result(
            result
                .as_object()
                .ok_or_else(|| authority_error("reconcile result must be object"))?,
            channel,
            chain,
            is_private,
        )?;
    } else if !result.is_null() {
        return Err(authority_error(
            "ReconcileResult result must be object or null",
        ));
    } else if state == "CONFIRMED" {
        return Err(authority_error(
            "confirmed reconcile result is missing result",
        ));
    }
    let error = required_value(root, "error", "ReconcileResult")?;
    if error.is_object() {
        let error = error
            .as_object()
            .ok_or_else(|| authority_error("reconcile error must be object"))?;
        require_exact_keys(error, &["code", "message"], "ReconcileError")?;
        required_nonempty_str(error, "code", "ReconcileError")?;
        required_nonempty_str(error, "message", "ReconcileError")?;
    } else if !error.is_null() {
        return Err(authority_error(
            "ReconcileResult error must be object or null",
        ));
    }
    Ok(())
}

/// 校验 CanonicalPost 的稳定消息身份和独立 categoryChain props。
fn validate_post(
    post: &Map<String, Value>,
    channel: &str,
    chain: Option<&str>,
) -> Result<(), ImError> {
    require_keys(
        post,
        &["id", "temporaryId", "channelId", "type", "props"],
        "CanonicalPost",
    )?;
    if required_nonempty_str(post, "channelId", "CanonicalPost")? != channel {
        return Err(authority_error("CanonicalPost channelId mismatch"));
    }
    if required_nonempty_str(post, "type", "CanonicalPost")? != "CATEGORY_CHAIN" {
        return Err(authority_error("CanonicalPost type is not CATEGORY_CHAIN"));
    }
    required_nonempty_str(post, "id", "CanonicalPost")?;
    required_nonempty_str(post, "temporaryId", "CanonicalPost")?;
    let props = required_value(post, "props", "CanonicalPost")?
        .as_object()
        .ok_or_else(|| authority_error("CanonicalPost props must be object"))?;
    let card = props
        .get("categoryChain")
        .ok_or_else(|| authority_error("CanonicalPost missing props.categoryChain"))?
        .as_object()
        .ok_or_else(|| authority_error("props.categoryChain must be object"))?;
    validate_card(card, channel, chain)
}

/// 判断 object 是否含有一组字段。
fn has_all(object: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().all(|key| object.contains_key(*key))
}

/// 要求 object 恰好包含指定字段，阻止未知 schema 静默降级。
fn require_exact_keys(
    object: &Map<String, Value>,
    keys: &[&str],
    shape: &str,
) -> Result<(), ImError> {
    require_keys(object, keys, shape)?;
    if object.keys().any(|key| !keys.contains(&key.as_str())) {
        return Err(authority_error(format!("{shape} contains unknown field")));
    }
    Ok(())
}

/// 要求必需字段存在，且只额外接纳当前投影合同明确允许的可选字段。
fn require_known_keys(
    object: &Map<String, Value>,
    required: &[&str],
    optional: &[&str],
    shape: &str,
) -> Result<(), ImError> {
    require_keys(object, required, shape)?;
    if object
        .keys()
        .any(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(authority_error(format!("{shape} contains unknown field")));
    }
    Ok(())
}

/// 校验服务端参与上限；活动态必须显式提供有效值。
fn validate_max_categories(
    object: &Map<String, Value>,
    category_count: usize,
    shape: &str,
) -> Result<u64, ImError> {
    let value = object
        .get("maxCategories")
        .ok_or_else(|| authority_error(format!("{shape} missing maxCategories")))?;
    let max_categories = value
        .as_u64()
        .filter(|value| *value >= 1 && *value <= category_count as u64)
        .ok_or_else(|| {
            authority_error(format!(
                "{shape}.maxCategories must be integer between 1 and category count"
            ))
        })?;
    Ok(max_categories)
}

/// 要求 object 至少包含指定字段，CanonicalPost 允许现有消息扩展字段。
fn require_keys(object: &Map<String, Value>, keys: &[&str], shape: &str) -> Result<(), ImError> {
    if let Some(missing) = keys.iter().find(|key| !object.contains_key(**key)) {
        return Err(authority_error(format!("{shape} missing {missing}")));
    }
    Ok(())
}

/// 从 object 取必需字段。
fn required_value<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    shape: &str,
) -> Result<&'a Value, ImError> {
    object
        .get(key)
        .ok_or_else(|| authority_error(format!("{shape} missing {key}")))
}

/// 取必需非空字符串。
fn required_nonempty_str<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    shape: &str,
) -> Result<&'a str, ImError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| authority_error(format!("{shape}.{key} must be a non-empty string")))
}

/// 取允许 null 的非空字符串。
fn require_nonempty_string_or_null(
    object: &Map<String, Value>,
    key: &str,
    shape: &str,
) -> Result<(), ImError> {
    if object
        .get(key)
        .is_none_or(|value| !value.is_null() && value.as_str().is_none_or(str::is_empty))
    {
        return Err(authority_error(format!(
            "{shape}.{key} must be string or null"
        )));
    }
    Ok(())
}

/// 取规范十进制 revision。
fn require_revision<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    shape: &str,
) -> Result<&'a str, ImError> {
    let value = required_nonempty_str(object, key, shape)?;
    canonical_revision(value)
        .map_err(|reason| authority_error(format!("{shape}.{key}: {reason}")))?;
    Ok(value)
}

/// 取允许 null 的规范十进制 revision。
fn require_revision_or_null(
    object: &Map<String, Value>,
    key: &str,
    shape: &str,
) -> Result<(), ImError> {
    match object.get(key) {
        Some(Value::Null) => Ok(()),
        Some(Value::String(value)) if canonical_revision(value).is_ok() => Ok(()),
        _ => Err(authority_error(format!(
            "{shape}.{key} must be revision string or null"
        ))),
    }
}

/// 取数组字段。
fn required_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    shape: &str,
) -> Result<&'a Vec<Value>, ImError> {
    required_value(object, key, shape)?
        .as_array()
        .ok_or_else(|| authority_error(format!("{shape}.{key} must be an array")))
}

/// 要求 nullable 联合中的字段确实为 JSON null。
fn require_null(value: &Value, key: &str, shape: &str) -> Result<(), ImError> {
    if !value.is_null() {
        return Err(authority_error(format!("{shape}.{key} must be null")));
    }
    Ok(())
}

/// 取布尔字段。
fn require_bool(object: &Map<String, Value>, key: &str, shape: &str) -> Result<bool, ImError> {
    object
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| authority_error(format!("{shape}.{key} must be boolean")))
}

/// 取非负有符号整数。
fn require_nonnegative_i64(
    object: &Map<String, Value>,
    key: &str,
    shape: &str,
) -> Result<i64, ImError> {
    let value = object
        .get(key)
        .and_then(Value::as_i64)
        .filter(|value| *value >= 0)
        .ok_or_else(|| authority_error(format!("{shape}.{key} must be non-negative integer")))?;
    Ok(value)
}

/// 取非负无符号整数。
fn require_nonnegative_u64(
    object: &Map<String, Value>,
    key: &str,
    shape: &str,
) -> Result<u64, ImError> {
    object
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| authority_error(format!("{shape}.{key} must be non-negative integer")))
}

/// 取 schemaVersion=1。
fn require_schema_version(object: &Map<String, Value>, shape: &str) -> Result<(), ImError> {
    if object.get("schemaVersion").and_then(Value::as_u64) != Some(1) {
        return Err(authority_error(format!("{shape}.schemaVersion must be 1")));
    }
    Ok(())
}

/// 校验有限状态集合。
fn require_status<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<&'a str, ImError> {
    let value = object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| allowed.contains(value))
        .ok_or_else(|| authority_error(format!("{key} has unknown status")))?;
    Ok(value)
}

/// 校验状态字段并返回借用值。
fn required_status<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<&'a str, ImError> {
    require_status(object, key, allowed)
}

/// 校验分类接龙允许的 closeReason。
fn require_close_reason(object: &Map<String, Value>) -> Result<(), ImError> {
    match object.get("closeReason") {
        Some(Value::Null) => Ok(()),
        Some(Value::String(value))
            if value.is_empty() || value == "DEADLINE" || value == "OWNER" =>
        {
            Ok(())
        }
        _ => Err(authority_error(
            "CategoryChain.closeReason has unknown value",
        )),
    }
}

/// 校验 bounded text 字段。
fn validate_text_field(
    object: &Map<String, Value>,
    key: &str,
    min: usize,
    max: usize,
    shape: &str,
) -> Result<(), ImError> {
    let value = required_value(object, key, shape)?
        .as_str()
        .ok_or_else(|| authority_error(format!("{shape}.{key} must be a string")))?;
    let count = value.trim().chars().count();
    if count < min || count > max {
        return Err(authority_error(format!(
            "{shape}.{key} length out of bounds"
        )));
    }
    Ok(())
}

/// 判断当前 post object 是否携带分类接龙专用 props。
fn object_is_category_post(object: &Map<String, Value>) -> bool {
    object
        .get("props")
        .and_then(Value::as_object)
        .and_then(|props| props.get("categoryChain"))
        .is_some()
}

/// reconcile 的未知 chain 结果允许没有 chainId，但仍需有合法状态/关联键。
fn is_reconcile_result(object: &Map<String, Value>) -> bool {
    matches!(
        object.get("state").and_then(Value::as_str),
        Some("CONFIRMED" | "REJECTED" | "PENDING" | "NOT_FOUND")
    ) && object
        .get("clientMutationId")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty())
}

/// authority 中禁止把输入侧 snake_case 身份/操作元数据当成服务端事实。
fn is_forbidden_authority_response_key(key: &str) -> bool {
    matches!(
        key,
        "actor"
            | "actorId"
            | "tenant"
            | "tenantId"
            | "tenant_id"
            | "actor_id"
            | "authorSnapshot"
            | "event_seq"
            | "operation_id"
            | "client_mutation_id"
            | "channel_id"
            | "chain_id"
            | "category_id"
            | "entry_id"
            | "user_id"
            | "count"
    )
}

/// 生成输入解析错误。
fn parse_error(command: &str, message: impl Into<String>) -> ImError {
    ImError::Parse(format!("{command}: {}", message.into()))
}

/// 生成服务端 authority 校验错误。
fn authority_error(message: impl Into<String>) -> ImError {
    ImError::Parse(format!("CATEGORY_CHAIN_AUTHORITY: {}", message.into()))
}

/// 生成 JSON 序列化边界错误。
fn serialize_error(command: &str, message: impl Into<String>) -> ImError {
    ImError::Serialize(format!("{command}: {}", message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// 全字段公开WS投影必须通过；缺字段和私有状态必须失败。
    #[test]
    fn category_projection_shape_is_closed_and_valid_card_is_accepted() {
        let card = json!({"schemaVersion":1,"chainId":"chain-a","revision":"1","status":"OPEN","title":"活动","description":"","example":"","deadlineAt":1789113600000_i64,"audienceCount":5,"participantCount":0,"categoryCount":2,"maxCategories":2,"categories":[{"categoryId":"a","name":"室内","ordinal":0,"tone":"purple","revision":"1","participantCount":0,"entries":[],"hasMore":false},{"categoryId":"b","name":"户外","ordinal":1,"tone":"blue","revision":"1","participantCount":0,"entries":[],"hasMore":false}]});
        let mut projection = json!({"schemaVersion":1,"channelId":"channel-a","chainId":"chain-a","revision":"1","eventId":"e1","eventSeq":"1","anchorPostId":"post-a","temporaryId":"temporary-a","card":card});
        validate_authority(&projection, "channel-a", Some("chain-a"), false).unwrap();
        projection["myParticipation"] = json!({"entries":[]});
        assert!(validate_authority(&projection, "channel-a", Some("chain-a"), false).is_err());
    }
}
