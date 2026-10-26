//! 通用命令逃生口 `cmd <name> <jsonBody>`：从 `cmd_parse.rs` 抽出（结构闸 ≤300 行）。
//!
//! **存在理由**：host-cli repl 原只暴露 14 个专用 verb（send/read/revoke/…），CI-4 联调
//! V5/V7/V8 反复命中「读族/编辑/bot 族命令无 stdin verb → 无法经 host-cli 驱动」工具缺口
//! （记于矩阵 §2 阶段⑤⑦⑧ blockers）。本 verb 是**根治**：直接按命令名 + JSON body 驱动
//! helix-im 注册的**任意 89 命令**（im_top20 / im_get_updated_posts / im_bot_create / …），
//! 无需为每条命令写专用解析臂。业务契约/endpoint/casing 真源仍在 `helix-im::outbound`，
//! 本文件零臆造——只把「命令名 + 用户给的 JSON」原样翻成 `Tick::Command`。
//!
//! name 运行时取自 stdin → `Cow::Owned`（`AppCommand.name` 是 `Cow<'static,str>`，owned 合法，
//! 无需 `Box::leak`）。子模块经 `super::Parsed` 复用父 tagged union。

use std::borrow::Cow;

use bytes::Bytes;
use helix_core::tick::AppCommand;
use helix_core::Tick;

use super::Parsed;

/// `cmd <commandName> <jsonBody?>` → 驱动任意已注册命令。
/// - body 省略 → 空对象 `{}`（覆盖 im_get_all_modules / im_bot_agent_info 等空 body 命令）。
/// - body 非法 JSON → `Bad`（不崩泵）。
/// - 命令名是否真注册由泵侧 `Module::accepts()` 判定；未注册 → 引擎走 unhandled（不在此预校验，
///   保持 repl 薄壳：真源是 helix-im registry，不在 host-cli 复制一份命令白名单）。
pub(super) fn parse_generic(rest: &str) -> Parsed {
    let mut it = rest.splitn(2, char::is_whitespace);
    let name = it.next().unwrap_or("").trim();
    let body = it.next().unwrap_or("").trim();
    if name.is_empty() {
        return Parsed::Bad(
            "用法 cmd <commandName> <jsonBody>（如 cmd im_top20 {\"channel_id\":\"o1g…\"}）".into(),
        );
    }
    let payload = if body.is_empty() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        match serde_json::from_str::<serde_json::Value>(body) {
            Ok(v) => v,
            Err(e) => return Parsed::Bad(format!("cmd {name} body 非合法 JSON：{e}")),
        }
    };
    Parsed::Command(command_owned(name.to_string(), payload))
}

/// 构造 `Tick::Command`，命令名取运行时 `String`（`Cow::Owned`，对照父 `command()` 的 `&'static`）。
fn command_owned(name: String, payload: serde_json::Value) -> Tick {
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    Tick::Command(AppCommand {
        name: Cow::Owned(name),
        payload: Bytes::from(bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unwrap_cmd(p: Parsed) -> AppCommand {
        match p {
            Parsed::Command(Tick::Command(c)) => c,
            _ => panic!("期望 Parsed::Command(Tick::Command)"),
        }
    }

    #[test]
    fn generic_drives_arbitrary_command_with_json() {
        let c = unwrap_cmd(parse_generic("im_top20 {\"channel_id\":\"o1guy\"}"));
        assert_eq!(c.name, "im_top20");
        let v: serde_json::Value = serde_json::from_slice(&c.payload).unwrap();
        assert_eq!(v["channel_id"], "o1guy");
    }

    #[test]
    fn generic_empty_body_becomes_empty_object() {
        let c = unwrap_cmd(parse_generic("im_get_all_modules"));
        assert_eq!(c.name, "im_get_all_modules");
        assert_eq!(&c.payload[..], b"{}");
    }

    #[test]
    fn generic_missing_name_is_bad_not_panic() {
        assert!(matches!(parse_generic("   "), Parsed::Bad(_)));
    }

    #[test]
    fn generic_bad_json_is_bad_not_panic() {
        assert!(matches!(
            parse_generic("im_top20 {not json}"),
            Parsed::Bad(_)
        ));
    }
}
