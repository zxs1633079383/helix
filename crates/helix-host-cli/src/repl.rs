//! helix-host-cli REPL —— stdin 命令遥控器（终端版「Tauri 前端 invoke」）。
//!
//! 每行 stdin → 解析 verb+args → `Tick::Command(AppCommand)` → 复用 main 的 `tick_tx`
//! 入**同一条泵**（与 `Tick::Inbound` 同路，串行处理，零并发进 core，守 HX-C002）。
//! 坏命令 → 打印用法**不崩泵**；`quit`/EOF → 触发 shutdown 优雅退出。
//!
//! host 是 ambient API 合法落点（stdin/println allow，clippy disallowed_methods=allow）。
//! 命令目录见 [`print_usage`]；Tier 1 outbound 命令在 [`parse_line`] 续接。

use bytes::Bytes;
use helix_core::tick::AppCommand;
use helix_core::Tick;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// 启动 stdin 读取任务。命令解析见 [`parse_line`]。
///
/// `shutdown_tx` 在 `quit`/EOF 时 fire，让 `run_engine_loop_with_transports` 优雅退出。
pub fn spawn_stdin_repl(tick_tx: mpsc::Sender<Tick>, shutdown_tx: oneshot::Sender<()>) {
    tokio::spawn(async move {
        let mut shutdown_tx = Some(shutdown_tx);
        let mut tmp_seq: u64 = 0;
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        print_usage();
        loop {
            match lines.next_line().await {
                Ok(Some(raw)) => {
                    let line = raw.trim();
                    if line.is_empty() {
                        continue;
                    }
                    match parse_line(line, &mut tmp_seq) {
                        Parsed::Command(tick) => {
                            if tick_tx.send(tick).await.is_err() {
                                tracing::warn!("tick_tx 已关闭，stdin repl 退出");
                                break;
                            }
                        }
                        Parsed::Quit => {
                            if let Some(tx) = shutdown_tx.take() {
                                let _ = tx.send(());
                            }
                            break;
                        }
                        Parsed::Help => print_usage(),
                        Parsed::Bad(msg) => {
                            // 坏命令不崩泵：打印诊断 + 用法，继续读下一行。
                            println!("⚠️  {msg}");
                            print_usage();
                        }
                    }
                }
                // EOF（stdin 关 / 管道脚本跑完）→ 优雅 shutdown，不让泵悬挂。
                Ok(None) => {
                    if let Some(tx) = shutdown_tx.take() {
                        let _ = tx.send(());
                    }
                    break;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "stdin 读取错误，repl 退出");
                    break;
                }
            }
        }
    });
}

/// 一行的解析结果（tagged union，make-invalid-states-unrepresentable）。
enum Parsed {
    /// 已构造好的应用命令，待入泵。
    Command(Tick),
    /// 退出（`quit`/`exit`）。
    Quit,
    /// 打印用法（`help`/`?`）。
    Help,
    /// 解析失败诊断（不崩泵）。
    Bad(String),
}

/// 一行 → [`Parsed`]。verb 映射**固定 `&'static str`** 命令名（`AppCommand::new` 要求 'static）。
///
/// `tmp_seq` 由 host 单调铸造 temporary_id（echo 对账锚），调用方持有跨行计数。
fn parse_line(line: &str, tmp_seq: &mut u64) -> Parsed {
    let mut it = line.splitn(2, char::is_whitespace);
    let verb = it.next().unwrap_or("");
    let rest = it.next().unwrap_or("").trim();
    match verb {
        "quit" | "exit" => Parsed::Quit,
        "help" | "?" => Parsed::Help,
        "send" => parse_send(rest, tmp_seq),
        // ── Tier 1 outbound 命令在此续接（read/edit/leave/create/schedule/cancel/reconnect）──
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

/// 构造 `Tick::Command`（payload = JSON bytes）。
fn command(name: &'static str, payload: serde_json::Value) -> Tick {
    let bytes = serde_json::to_vec(&payload).unwrap_or_default();
    Tick::Command(AppCommand::new(name, Bytes::from(bytes)))
}

/// 打印命令目录（启动 + 坏命令 + help 时）。
fn print_usage() {
    println!(
        "\n── helix-host-cli 遥控器（Tier 0）──\n\
         send <channelId> <text>   发消息（im_send_message → echo 回填 + posts_update）\n\
         help | ?                  打印本用法\n\
         quit | exit               退出（EOF/管道结束亦优雅退出）\n\
         （Tier 1 续接 read/edit/leave/create/schedule/cancel/reconnect）\n"
    );
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
        // temporary_id 单调铸造
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
}
