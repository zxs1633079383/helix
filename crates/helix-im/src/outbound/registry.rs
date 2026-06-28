//! Outbound 命令注册表（`linkme` distributed_slice，对齐 `ws/registry.rs` Strategy 模式）。
//!
//! P2-C4 收口：把 `commands.rs::is_outbound` 的单 `match` 拆成**分布式注册**——每个 outbound
//! 命令一个 `OutboundCommand` 实现 + 一条 `#[distributed_slice(OUTBOUND_COMMANDS)]` 注册，
//! 各命令文件**零交集**（P3a/P3b 可各加独立注册文件并行，不再共改一处 match）。
//!
//! 命令只产 `(endpoint_path, wire_body)`（业务契约 / ubiquitous language 在各命令文件，锚现网
//! Go 真源）；身份头（connectionId）+ Content-Type + Effect 组装在本文件单点收口（DRY），
//! 各命令不碰 `Effect`/headers。

use std::collections::HashMap;
use std::sync::OnceLock;

use bytes::Bytes;
use helix_core::effect::{Effect, HttpRequest};
use helix_core::{AuthKind, Correlation, AUTH_KIND_HEADER};
use linkme::distributed_slice;
use serde_json::Value;

use crate::error::ImError;

/// 出站网关**意图**（spec06 vote/score 第二网关，方案 A）：core 只标「这条命令走哪个网关 base」，
/// 真实 base host 由 host 经 config 注入（HX-C001：core 零 host 硬编码，base 仍 host 注入、确定性）。
///
/// 与 `AuthKind` 意图维度同构（命令标意图、driver/host 给真值），最小惊讶；既有 88 命令零改动
/// （默认 `Im`）。两个 base host **不同**：IM 含 `/api/cses` 前缀，Default 是 `env.restHost` 无前缀。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Gateway {
    /// IM 网关（既有全部命令）：base = `api_base_url`（= `http://host:8065/api/cses`，含 `/api/cses`）。
    Im,
    /// 第二（默认业务）网关：base = `default_api_base_url`（= `env.restHost`，**无** `/api/cses`）。
    /// vote/score（`/vote/*` `/average/*`）走它（真源 docs/case/11-投票评分.md:119）。
    Default,
}

/// 一个 outbound 应用命令：把通用入参（snake_case JSON）翻成现网 endpoint + camelCase Go body。
///
/// 解析失败 / 缺必填 → `Err(ImError::Parse)`（边界零信任，HX-C 不变量 4），**不 panic**。
pub(crate) trait OutboundCommand: Sync {
    /// 命令名（`im_send_message` 例外不在此；本表全是纯 HTTP-fire 命令）。
    fn name(&self) -> &'static str;

    /// 翻译：通用入参 `args` → `(endpoint_path, wire_body)`。`path` 不含 `api_base_url` 前缀。
    ///
    /// `path` 为**静态模板**（`&'static str`，零分配，绝大多数命令的 endpoint 是定值）。
    /// 含 REST path/query param 的命令（bot-agent `{botUserId}`/`?botUserId=`，P5b）走 `path_override`
    /// 产动态 path——`build` 仍返回静态模板做 fallback（不破既有 11 文件返回类型，零 ripple）。
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError>;

    /// 动态 path 覆盖（默认 `None` = 用 `build` 的静态 path）。REST path/query param 命令 override
    /// 此方法产 `Some(Ok(format!(...)))`；wire body 仍由 `build` 产（path 与 body 各管一摊，DRY）。
    /// 解析失败（缺/坏 path 段）→ `Some(Err)`（边界零信任，零 panic）。
    fn path_override(&self, _args: &Value) -> Option<Result<String, ImError>> {
        None
    }

    /// 出站鉴权**意图**（generic 两类凭据路由，HX-C001/C6）：core 只标意图、绝不拼 token，
    /// 真凭据由 driver 横切层据 `X-Auth-Kind` 注入。默认 `Session`（绝大多数命令走用户会话）；
    /// bot-agent bot-token 调用命令（P5b #22-27）override 为 `Bot`。
    fn auth_kind(&self) -> AuthKind {
        AuthKind::Session
    }

    /// HTTP method（Task1：已知 GET/DELETE 命令按现网契约映射，其余默认 POST）。
    fn method(&self) -> &'static str {
        outbound_method(self.name())
    }

    /// 是否**读族**命令（spec06 缺陷A）：读 = 无 WS 回声、HTTP 响应体即数据 → 读命令 override `true`，
    /// dispatch 注册 `OutboundReadReply` 回灌响应给前端。默认 `false`（写族走 WS 回声，HTTP 响应可丢）。
    fn is_read(&self) -> bool {
        false
    }

    /// 出站**网关意图**（spec06 vote/score 第二网关，方案 A）：默认 `Im`（既有 88 命令零改动）；
    /// vote/score（`/vote/*` `/average/*`）override `Gateway::Default` 走第二 base。`http_post` 据此选 base。
    fn gateway(&self) -> Gateway {
        Gateway::Im
    }
}

/// 注册条目（`name` 与 `cmd.name()` 必须一致，build_registry 会校验）。
pub(crate) struct OutboundRegistration {
    pub(crate) name: &'static str,
    pub(crate) command: &'static dyn OutboundCommand,
}

#[distributed_slice]
pub(crate) static OUTBOUND_COMMANDS: [OutboundRegistration];

static OUTBOUND_MAP: OnceLock<
    Result<HashMap<&'static str, &'static dyn OutboundCommand>, ImError>,
> = OnceLock::new();

fn registry() -> Result<&'static HashMap<&'static str, &'static dyn OutboundCommand>, ImError> {
    OUTBOUND_MAP
        .get_or_init(|| build_registry(&OUTBOUND_COMMANDS))
        .as_ref()
        .map_err(Clone::clone)
}

fn build_registry(
    entries: &'static [OutboundRegistration],
) -> Result<HashMap<&'static str, &'static dyn OutboundCommand>, ImError> {
    let mut map = HashMap::with_capacity(entries.len());
    for entry in entries {
        let cmd_name = entry.command.name();
        if entry.name != cmd_name {
            return Err(ImError::InvalidWsHandlerRegistration {
                registered: entry.name.to_string(),
                handler: cmd_name.to_string(),
            });
        }
        if map.insert(entry.name, entry.command).is_some() {
            return Err(ImError::DuplicateWsAction(entry.name.to_string()));
        }
    }
    Ok(map)
}

/// 本模块是否认领该命令名（`accepts_tick` + `module.rs` dispatch 共用单一真源）。
///
/// 注册表构建失败（重复/错配）退化为「不认领」——构建错误会在 `handle_outbound` 显式回报，
/// 此处布尔判定不吞 `Result`（O(1) 查表）。
pub fn is_outbound(name: &str) -> bool {
    registry().map(|m| m.contains_key(name)).unwrap_or(false)
}

/// 返回闭集内该命令名的 **interned `&'static str`**（命中），否则 `None`（G-6 / PI-1）。
///
/// 闭集 = `OUTBOUND_COMMANDS` 注册的全部 88 命令名（registry 的 key 本就是 `&'static str`，
/// 由各命令 `#[distributed_slice]` 注册时的 `&'static` 字面量提供）。FFI driver 边界据此把
/// C 侧传入的命令名零分配映射成 `Cow::Borrowed`（命中），miss 走 `Cow::Owned` fallback（仍正确）。
/// O(1) 查表（复用 cached `registry()` map）。`im_send_message`/`im_reconnect`/query 名不在 88
/// outbound 闭集内 → miss → owned（低频，不影响热路径写命令的零分配保证）。
pub fn canonical_command_name(name: &str) -> Option<&'static str> {
    registry()
        .ok()
        .and_then(|m| m.get_key_value(name))
        .map(|(k, _)| *k)
}

/// 该命令是否**读族**（spec06 缺陷A）：查注册表命令的 `is_read()`。未认领/构建失败 → `false`
/// （写族缺省，不误注册回灌）。O(1) 查表。dispatch 据此决定读命令是否注册 `OutboundReadReply`。
pub fn is_read(name: &str) -> bool {
    registry()
        .ok()
        .and_then(|m| m.get(name))
        .map(|cmd| cmd.is_read())
        .unwrap_or(false)
}

/// 注册表命令总数（`im_send_message` 不在内——它走 module.rs send arm，非 HTTP-fire outbound）。
///
/// 供性质测试断言「outbound 注册总数」不漂移（CP：补 createPosts 后 = 88）；构建失败（重复/错配）
/// 退化为 0（不吞 `Result`，O(1) 取 len）。
pub fn outbound_command_count() -> usize {
    registry().map(HashMap::len).unwrap_or(0)
}

fn outbound_method(name: &str) -> &'static str {
    match name {
        "im_team_quit" | "im_bot_agent_config_delete" | "im_bot_agent_channel_remove" => "DELETE",
        "im_bot_list_visible"
        | "im_bot_token_list"
        | "im_bot_visible_user_list"
        | "im_bot_agent_config_get"
        | "im_bot_agent_configs_enabled"
        | "im_bot_agent_channel_bots"
        | "im_bot_agent_channel_available_bots"
        | "im_bot_agent_info"
        | "im_bot_agent_team_members"
        | "im_bot_agent_get_user"
        | "im_bot_agent_team_channel"
        | "im_webhook_config_get" => "GET",
        _ => "POST",
    }
}

/// 把一条 outbound 命令翻成 Effect 列表（当前恒为单条 `Effect::Http`）。
///
/// 两个 base host 入参（spec06 第二网关，方案 A）：`api_base_url` = IM 网关（含 `/api/cses`）；
/// `default_api_base_url` = 第二（默认业务）网关（`env.restHost`，无 `/api/cses`）。命令的 `gateway()`
/// 意图决定拼哪个 base——既有命令默认 `Im`，vote/score override `Default`。两 base 均由 host 注入
/// （HX-C001：core/im 零 host 硬编码）。
pub fn handle_outbound(
    name: &str,
    payload: &[u8],
    api_base_url: &str,
    default_api_base_url: &str,
    connection_id: Option<&str>,
    corr: Correlation,
) -> Result<Vec<Effect>, ImError> {
    let command = *registry()?
        .get(name)
        .ok_or_else(|| ImError::Parse(format!("未认领的 outbound 命令 '{name}'")))?;

    let args: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("{name} payload: {e}")))?;

    let (static_path, body) = command.build(&args)?;
    // 动态 path（REST path/query param）优先；缺省回落静态模板（零分配，零 ripple）。
    let path: std::borrow::Cow<'static, str> = match command.path_override(&args) {
        Some(dynamic) => std::borrow::Cow::Owned(dynamic?),
        None => std::borrow::Cow::Borrowed(static_path),
    };

    // 网关意图 → 选 base（Im=IM 网关 / Default=第二网关）。base 真值 host 注入，core 仅按意图选。
    let base = match command.gateway() {
        Gateway::Im => api_base_url,
        Gateway::Default => default_api_base_url,
    };

    Ok(vec![http_request(
        base,
        command.method(),
        &path,
        body,
        command.auth_kind(),
        connection_id,
        corr,
    )])
}

/// 取必填字符串字段，缺/类型错/空 → `ImError::Parse`（边界零信任）。供各命令文件复用。
pub(crate) fn require_str<'a>(args: &'a Value, key: &str, cmd: &str) -> Result<&'a str, ImError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空必填字段 '{key}'")))
}

/// 拼建群 / 话题 / 公司大群成员数组（业务赋值下沉 helix：self=CREATOR + 其他=MEMBER）。
///
/// 入参 snake_case 结构化 args（壳只传身份/列表，不拼 wire body）：`self_id`（必填·自身 cookieId）/
/// `team_id`（可选·缺省 ""·companyId）/ `member_ids`（可选·其他成员 userId 列表·过滤空与自身）。
/// 返回 `(users:[{id,teamId,role}], user_ids:[id,...])`——`user_ids` 供 `picture.userIds` 复用。
/// role 赋值（CREATOR/MEMBER）属业务，在此收口；建群（channel/create）、话题（posts/makeTopic）、
/// 公司大群（teams/upsert）三命令共用，对齐真机curl真源 §2/§4。
pub(crate) fn build_creator_member_users(
    args: &Value,
    cmd: &str,
) -> Result<(Vec<Value>, Vec<String>), ImError> {
    let self_id = require_str(args, "self_id", cmd)?;
    let team_id = args.get("team_id").and_then(Value::as_str).unwrap_or("");
    let mut users = vec![serde_json::json!({
        "id": self_id,
        "teamId": team_id,
        "role": "CREATOR",
    })];
    let mut user_ids = vec![self_id.to_string()];
    if let Some(arr) = args.get("member_ids").and_then(Value::as_array) {
        for m in arr {
            if let Some(mid) = m.as_str() {
                if !mid.is_empty() && mid != self_id {
                    users.push(serde_json::json!({
                        "id": mid,
                        "teamId": team_id,
                        "role": "MEMBER",
                    }));
                    user_ids.push(mid.to_string());
                }
            }
        }
    }
    Ok((users, user_ids))
}

/// 组装一条 `Effect::Http` 请求（Content-Type + connectionId 身份头 + 鉴权意图头）。
///
/// 鉴权意图（`X-Auth-Kind`）统一显式下标：session/bot 都只标意图、不拼真凭据；driver 横切层据此
/// 注真 token 并在出网前 strip（C6，core 不拼 token）。
fn http_request(
    base: &str,
    method: &str,
    path: &str,
    body: Value,
    auth_kind: AuthKind,
    connection_id: Option<&str>,
    corr: Correlation,
) -> Effect {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::to_effect::connection_id_headers(connection_id));
    headers.push((AUTH_KIND_HEADER.to_string(), auth_kind.as_str().to_string()));
    Effect::Http {
        corr,
        req: HttpRequest {
            method: method.to_string(),
            url: format!("{base}/{path}"),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).unwrap_or_default())),
        },
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
