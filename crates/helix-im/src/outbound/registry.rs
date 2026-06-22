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
use helix_core::Correlation;
use linkme::distributed_slice;
use serde_json::Value;

use crate::error::ImError;

/// 一个 outbound 应用命令：把通用入参（snake_case JSON）翻成现网 endpoint + camelCase Go body。
///
/// 解析失败 / 缺必填 → `Err(ImError::Parse)`（边界零信任，HX-C 不变量 4），**不 panic**。
pub(crate) trait OutboundCommand: Sync {
    /// 命令名（`im_send_message` 例外不在此；本表全是纯 HTTP-fire 命令）。
    fn name(&self) -> &'static str;

    /// 翻译：通用入参 `args` → `(endpoint_path, wire_body)`。`path` 不含 `api_base_url` 前缀。
    fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError>;
}

/// 注册条目（`name` 与 `cmd.name()` 必须一致，build_registry 会校验）。
pub(crate) struct OutboundRegistration {
    pub(crate) name: &'static str,
    pub(crate) command: &'static dyn OutboundCommand,
}

#[distributed_slice]
pub(crate) static OUTBOUND_COMMANDS: [OutboundRegistration];

static OUTBOUND_MAP: OnceLock<Result<HashMap<&'static str, &'static dyn OutboundCommand>, ImError>> =
    OnceLock::new();

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

/// 把一条 outbound 命令翻成 Effect 列表（当前恒为单条 `Effect::Http`）。
pub fn handle_outbound(
    name: &str,
    payload: &[u8],
    api_base_url: &str,
    connection_id: Option<&str>,
    corr: Correlation,
) -> Result<Vec<Effect>, ImError> {
    let command = *registry()?
        .get(name)
        .ok_or_else(|| ImError::Parse(format!("未认领的 outbound 命令 '{name}'")))?;

    let args: Value = serde_json::from_slice(payload)
        .map_err(|e| ImError::Parse(format!("{name} payload: {e}")))?;

    let (path, body) = command.build(&args)?;

    Ok(vec![http_post(api_base_url, path, body, connection_id, corr)])
}

/// 取必填字符串字段，缺/类型错/空 → `ImError::Parse`（边界零信任）。供各命令文件复用。
pub(crate) fn require_str<'a>(args: &'a Value, key: &str, cmd: &str) -> Result<&'a str, ImError> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ImError::Parse(format!("{cmd}: 缺/空必填字段 '{key}'")))
}

/// 组装一条 `Effect::Http` POST（Content-Type + connectionId 身份头）。
fn http_post(
    api_base_url: &str,
    path: &str,
    body: Value,
    connection_id: Option<&str>,
    corr: Correlation,
) -> Effect {
    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    headers.extend(crate::acl::to_effect::connection_id_headers(connection_id));
    Effect::Http {
        corr,
        req: HttpRequest {
            method: "POST".to_string(),
            url: format!("{api_base_url}/{path}"),
            headers,
            body: Some(Bytes::from(serde_json::to_vec(&body).unwrap_or_default())),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 测试桩命令：build 直回固定 (path, body)（不涉真现网 body）。
    struct StubCommand {
        name: &'static str,
    }
    impl OutboundCommand for StubCommand {
        fn name(&self) -> &'static str {
            self.name
        }
        fn build(&self, _args: &Value) -> Result<(&'static str, Value), ImError> {
            Ok(("stub/path", json!({ "ok": true })))
        }
    }

    static STUB_A: StubCommand = StubCommand { name: "im_stub_a" };
    static STUB_DUP: StubCommand = StubCommand { name: "im_stub_a" };
    static STUB_MISMATCH: StubCommand = StubCommand {
        name: "handler_name",
    };

    fn reg(name: &'static str, cmd: &'static dyn OutboundCommand) -> OutboundRegistration {
        OutboundRegistration { name, command: cmd }
    }

    /// build_registry 正常路径 + name/handler 一致校验通过。
    #[test]
    fn build_registry_ok() {
        let entries: &'static [OutboundRegistration] = Box::leak(Box::new([reg("im_stub_a", &STUB_A)]));
        let map = build_registry(entries).expect("should build");
        assert!(map.contains_key("im_stub_a"));
    }

    /// 注册 name 与 command.name() 错配 → InvalidWsHandlerRegistration（registry.rs:59-60）。
    #[test]
    fn build_registry_rejects_name_mismatch() {
        let entries: &'static [OutboundRegistration] =
            Box::leak(Box::new([reg("wrong_name", &STUB_MISMATCH)]));
        match build_registry(entries) {
            Err(ImError::InvalidWsHandlerRegistration { registered, handler }) => {
                assert_eq!(registered, "wrong_name");
                assert_eq!(handler, "handler_name");
            }
            Err(e) => panic!("expected InvalidWsHandlerRegistration, got {e:?}"),
            Ok(_) => panic!("expected InvalidWsHandlerRegistration, got Ok"),
        }
    }

    /// 重复命令名 → DuplicateWsAction（registry.rs:65）。
    #[test]
    fn build_registry_rejects_duplicate() {
        let entries: &'static [OutboundRegistration] =
            Box::leak(Box::new([reg("im_stub_a", &STUB_A), reg("im_stub_a", &STUB_DUP)]));
        match build_registry(entries) {
            Err(ImError::DuplicateWsAction(name)) => assert_eq!(name, "im_stub_a"),
            Err(e) => panic!("expected DuplicateWsAction, got {e:?}"),
            Ok(_) => panic!("expected DuplicateWsAction, got Ok"),
        }
    }

    /// require_str：命中 / 缺失 / 空串 三路。
    #[test]
    fn require_str_paths() {
        let args = json!({ "k": "v", "empty": "" });
        assert_eq!(require_str(&args, "k", "cmd").unwrap(), "v");
        assert!(require_str(&args, "missing", "cmd").is_err());
        assert!(require_str(&args, "empty", "cmd").is_err());
    }

    /// is_outbound：真实注册表（distributed_slice 全集）命中既有命令、不命中未知。
    #[test]
    fn is_outbound_hits_registered() {
        assert!(is_outbound("im_revoke"));
        assert!(is_outbound("im_urgent_post"));
        assert!(!is_outbound("not_a_command"));
    }

    /// handle_outbound：正常路径产单条 Http（覆盖 http_post）+ 未知命令 Err + 坏 payload Err。
    #[test]
    fn handle_outbound_dispatch_and_errors() {
        let corr = Correlation::from_raw(1);
        let payload = serde_json::to_vec(&json!({ "post_id": "p" })).unwrap();
        let effects = handle_outbound("im_revoke", &payload, "http://h/api", Some("c1"), corr)
            .expect("revoke should dispatch");
        match &effects[0] {
            Effect::Http { req, .. } => {
                assert_eq!(req.url, "http://h/api/posts/revoke");
                assert!(req
                    .headers
                    .iter()
                    .any(|(k, v)| k == "Content-Type" && v == "application/json"));
            }
            other => panic!("expected Http, got {other:?}"),
        }
        // 未知命令 → Err
        assert!(handle_outbound("bogus", &payload, "http://h", None, corr).is_err());
        // 坏 JSON payload → Err（不 panic）
        assert!(handle_outbound("im_revoke", b"not-json", "http://h", None, corr).is_err());
    }
}
