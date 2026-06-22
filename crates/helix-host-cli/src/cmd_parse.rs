//! 命令行解析：一行 stdin → [`Parsed`]（纯函数，可单测）。从 `repl.rs` 抽出守 ≤300 行。
//!
//! verb 映射**固定 `&'static str`** 命令名（`AppCommand::new` 要求 'static）；
//! 业务契约/endpoint 不在此——helix-im::commands 才是真源，本文件只把用户参数翻成 payload JSON。

use bytes::Bytes;
use helix_core::tick::AppCommand;
use helix_core::Tick;

/// 一行的解析结果（tagged union，make-invalid-states-unrepresentable）。
pub(crate) enum Parsed {
    /// 已构造好的应用命令，待入泵。
    Command(Tick),
    /// 退出（`quit`/`exit`）。
    Quit,
    /// 打印用法（`help`/`?`）。
    Help,
    /// 解析失败诊断（不崩泵）。
    Bad(String),
}

/// 一行 → [`Parsed`]。`tmp_seq` 由 host 单调铸造 temporary_id（echo 对账锚），调用方持有跨行计数。
pub(crate) fn parse_line(line: &str, tmp_seq: &mut u64) -> Parsed {
    let mut it = line.splitn(2, char::is_whitespace);
    let verb = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match verb {
        "quit" | "exit" => Parsed::Quit,
        "help" | "?" => Parsed::Help,
        "send" => parse_send(rest, tmp_seq),
        // ── Tier 1 outbound 自驱动命令（业务契约/endpoint 在 helix-im::commands）──
        "read" => parse_read(rest),
        "revoke" => one_str_cmd("im_revoke", "post_id", rest, "revoke <postId>"),
        "leave" => one_str_cmd("im_channel_leave", "channel_id", rest, "leave <channelId>"),
        "cancel" => one_str_cmd(
            "im_cancel_schedule",
            "channel_id",
            rest,
            "cancel <channelId>",
        ),
        "schedule" => parse_schedule(rest),
        "close" => one_str_cmd("im_channel_close", "channel_id", rest, "close <channelId>"),
        "nickname" => parse_nickname(rest),
        "create" => parse_raw_json("im_create_channel", rest, "create <rawJSON body>"),
        "makeTopic" | "maketopic" => {
            parse_raw_json("im_make_topic", rest, "makeTopic <rawJSON 含 postId>")
        }
        "member" => parse_raw_json(
            "im_channel_member_change",
            rest,
            "member <rawJSON 含 channelId + joinUsers/leaveUsers>",
        ),
        other => Parsed::Bad(format!("未知命令 '{other}'（help 看用法）")),
    }
}

/// `send <channelId> <text...>` → `im_send_message`。
///
/// temporary_id 由 host 铸造（`hostcli_tmp_<n>`，单调），作 echo 对账锚（对齐
/// handle_send_message 的 PendingSend 注册）。text 允许含空格（取首空白后全部）。
fn parse_send(rest: &str, tmp_seq: &mut u64) -> Parsed {
    let mut it = rest.splitn(2, char::is_whitespace);
    let Some(channel) = it.next().filter(|s| !s.is_empty()) else {
        return Parsed::Bad("send 缺 channelId：用法 send <channelId> <text>".into());
    };
    let text = it.next().unwrap_or("").trim();
    if text.is_empty() {
        return Parsed::Bad("send 缺 text：用法 send <channelId> <text>".into());
    }
    *tmp_seq += 1;
    let payload = serde_json::json!({
        "channel_id": channel,
        "temporary_id": format!("hostcli_tmp_{}", *tmp_seq),
        "text": text,
    });
    Parsed::Command(command("im_send_message", payload))
}

/// `read <channelId> [endTimeMs]` → im_post_read（post/read → post_read + update_channel）。
fn parse_read(rest: &str) -> Parsed {
    let mut it = rest.split_whitespace();
    let Some(channel) = it.next() else {
        return Parsed::Bad("read 缺 channelId：用法 read <channelId> [endTimeMs]".into());
    };
    let mut payload = serde_json::json!({ "channel_id": channel });
    if let Some(end) = it.next() {
        match end.parse::<i64>() {
            Ok(ms) => payload["end_time"] = serde_json::json!(ms),
            Err(_) => return Parsed::Bad(format!("read endTimeMs 非整数：'{end}'")),
        }
    }
    Parsed::Command(command("im_post_read", payload))
}

/// `schedule <channelId> <schedulePostAtMs> <text...>` → im_create_schedule。
fn parse_schedule(rest: &str) -> Parsed {
    let mut it = rest.splitn(3, char::is_whitespace);
    let (Some(channel), Some(ts), Some(text)) = (it.next(), it.next(), it.next()) else {
        return Parsed::Bad("用法 schedule <channelId> <schedulePostAtMs> <text>".into());
    };
    let Ok(ms) = ts.parse::<i64>() else {
        return Parsed::Bad(format!("schedule schedulePostAtMs 非整数：'{ts}'"));
    };
    let text = text.trim();
    if text.is_empty() {
        return Parsed::Bad("schedule 缺 text".into());
    }
    Parsed::Command(command(
        "im_create_schedule",
        serde_json::json!({ "channel_id": channel, "schedule_post_at": ms, "message": text }),
    ))
}

/// `nickname <channelId> <新昵称...>` → im_update_member_nickname。
/// 昵称允许空（清空昵称，Go 侧 trim 空→nil）；userId 缺省 → Go 用 session 兜底。
fn parse_nickname(rest: &str) -> Parsed {
    let mut it = rest.splitn(2, char::is_whitespace);
    let Some(channel) = it.next().filter(|s| !s.is_empty()) else {
        return Parsed::Bad("用法 nickname <channelId> <新昵称>".into());
    };
    let nick = it.next().unwrap_or("").trim();
    Parsed::Command(command(
        "im_update_member_nickname",
        serde_json::json!({ "channel_id": channel, "nickname": nick }),
    ))
}

/// 单字符串参数命令（revoke/leave/cancel/close）：`<verb> <value>` → {field: value}。
fn one_str_cmd(name: &'static str, field: &str, rest: &str, usage: &str) -> Parsed {
    let value = rest.split_whitespace().next().unwrap_or("");
    if value.is_empty() {
        return Parsed::Bad(format!("用法 {usage}"));
    }
    // field 是运行时变量 → 手建 Map（serde_json::json! 只接受字面量键）。
    let mut map = serde_json::Map::new();
    map.insert(
        field.to_string(),
        serde_json::Value::String(value.to_string()),
    );
    Parsed::Command(command(name, serde_json::Value::Object(map)))
}

/// 复杂 body 命令（create/makeTopic）：rest 为 raw JSON，直传作 payload。
/// 成员对象结构现网较复杂（ChannelMemberMinimal 等），由用户给完整 JSON，不在 repl 臆造。
fn parse_raw_json(name: &'static str, rest: &str, usage: &str) -> Parsed {
    if rest.is_empty() {
        return Parsed::Bad(format!("用法 {usage}"));
    }
    match serde_json::from_str::<serde_json::Value>(rest) {
        Ok(v) => Parsed::Command(command(name, v)),
        Err(e) => Parsed::Bad(format!("{name} body 非合法 JSON：{e}")),
    }
}

/// 构造 `Tick::Command`（payload = JSON bytes）。
fn command(name: &'static str, payload: serde_json::Value) -> Tick {
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    Tick::Command(AppCommand::new(name, Bytes::from(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 取出 Command 的 (name, payload-json)，非 Command 则 panic（测试期望明确）。
    fn unwrap_cmd(p: Parsed) -> (String, serde_json::Value) {
        match p {
            Parsed::Command(Tick::Command(c)) => {
                let v: serde_json::Value = serde_json::from_slice(&c.payload).unwrap();
                (c.name.into_owned(), v)
            }
            _ => panic!("expected Parsed::Command(Tick::Command)"),
        }
    }

    #[test]
    fn send_builds_im_send_message_with_minted_tmp_id() {
        let mut seq = 0;
        let (name, v) = unwrap_cmd(parse_line("send ch_42 hello world", &mut seq));
        assert_eq!(name, "im_send_message");
        assert_eq!(v["channel_id"], "ch_42");
        // text 保留空格（splitn(2) 只切首空白）
        assert_eq!(v["text"], "hello world");
        assert_eq!(v["temporary_id"], "hostcli_tmp_1");
        assert_eq!(seq, 1);
    }

    #[test]
    fn quit_and_help_route_correctly() {
        let mut seq = 0;
        assert!(matches!(parse_line("quit", &mut seq), Parsed::Quit));
        assert!(matches!(parse_line("exit", &mut seq), Parsed::Quit));
        assert!(matches!(parse_line("help", &mut seq), Parsed::Help));
        assert!(matches!(parse_line("?", &mut seq), Parsed::Help));
    }

    /// 坏命令一律 Parsed::Bad（→ repl 不崩泵），且**不**推进 tmp_seq。
    #[test]
    fn bad_lines_do_not_panic_or_advance_seq() {
        let mut seq = 7;
        assert!(matches!(parse_line("bogus", &mut seq), Parsed::Bad(_)));
        assert!(matches!(parse_line("send", &mut seq), Parsed::Bad(_))); // 缺 channel
        assert!(matches!(parse_line("send ch_1", &mut seq), Parsed::Bad(_))); // 缺 text
        assert_eq!(seq, 7, "坏命令不应铸造 temporary_id");
    }

    /// Tier 1 单参命令（revoke/leave/cancel）：动态键手建 Map（非 json! 字面量键）。
    #[test]
    fn tier1_single_arg_commands_map_to_names_and_fields() {
        let mut seq = 0;
        let (name, v) = unwrap_cmd(parse_line("revoke p_9", &mut seq));
        assert_eq!(name, "im_revoke");
        assert_eq!(v["post_id"], "p_9");

        let (name, v) = unwrap_cmd(parse_line("leave ch_7", &mut seq));
        assert_eq!(name, "im_channel_leave");
        assert_eq!(v["channel_id"], "ch_7");

        let (name, v) = unwrap_cmd(parse_line("cancel ch_3", &mut seq));
        assert_eq!(name, "im_cancel_schedule");
        assert_eq!(v["channel_id"], "ch_3");

        let (name, v) = unwrap_cmd(parse_line("close ch_8", &mut seq));
        assert_eq!(name, "im_channel_close");
        assert_eq!(v["channel_id"], "ch_8");

        // nickname 保留空格 + 允许空昵称（清空）
        let (name, v) = unwrap_cmd(parse_line("nickname ch_9 New Name", &mut seq));
        assert_eq!(name, "im_update_member_nickname");
        assert_eq!(v["channel_id"], "ch_9");
        assert_eq!(v["nickname"], "New Name");
    }

    #[test]
    fn tier1_read_and_schedule_parse_numeric_args() {
        let mut seq = 0;
        let (name, v) = unwrap_cmd(parse_line("read ch_1 1700", &mut seq));
        assert_eq!(name, "im_post_read");
        assert_eq!(v["channel_id"], "ch_1");
        assert_eq!(v["end_time"], 1700);

        let (name, v) = unwrap_cmd(parse_line("schedule ch_2 999 see you later", &mut seq));
        assert_eq!(name, "im_create_schedule");
        assert_eq!(v["schedule_post_at"], 999);
        assert_eq!(v["message"], "see you later"); // text 保留空格

        // 非整数时间 → Bad，不崩
        assert!(matches!(
            parse_line("read ch_1 abc", &mut seq),
            Parsed::Bad(_)
        ));
        assert!(matches!(
            parse_line("schedule ch xx t", &mut seq),
            Parsed::Bad(_)
        ));
    }

    #[test]
    fn tier1_raw_json_commands_validate_json() {
        let mut seq = 0;
        let (name, v) = unwrap_cmd(parse_line(r#"create {"displayName":"g"}"#, &mut seq));
        assert_eq!(name, "im_create_channel");
        assert_eq!(v["displayName"], "g");

        // member：raw JSON 透传（含 channelId + joinUsers/leaveUsers，真机curl真源 §5）。
        let (name, v) = unwrap_cmd(parse_line(
            r#"member {"channelId":"ch_m","joinUsers":[{"id":"u9"}]}"#,
            &mut seq,
        ));
        assert_eq!(name, "im_channel_member_change");
        assert_eq!(v["channelId"], "ch_m");
        assert_eq!(v["joinUsers"][0]["id"], "u9");

        // 非法 JSON → Bad
        assert!(matches!(
            parse_line("create {bad", &mut seq),
            Parsed::Bad(_)
        ));
        assert!(matches!(parse_line("makeTopic", &mut seq), Parsed::Bad(_))); // 缺 body
    }
}
