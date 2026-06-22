//! helix-host-cli REPL —— stdin 命令遥控器（终端版「Tauri 前端 invoke」）。
//!
//! 每行 stdin → [`cmd_parse::parse_line`] → `Tick::Command(AppCommand)` → 复用 main 的 `tick_tx`
//! 入**同一条泵**（与 `Tick::Inbound` 同路，串行处理，零并发进 core，守 HX-C002）。
//! 坏命令 → 打印用法**不崩泵**；`quit`/EOF → 触发 shutdown 优雅退出。
//!
//! host 是 ambient API 合法落点（stdin/println allow，clippy disallowed_methods=allow）。
//! 命令解析在 [`crate::cmd_parse`]（纯函数 + 单测）；本文件只管 stdin I/O 循环 + 用法打印。

use helix_core::Tick;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::cmd_parse::{parse_line, Parsed};

/// 启动 stdin 读取任务。`shutdown_tx` 在 `quit`/EOF 时 fire，让泵优雅退出。
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

/// 打印命令目录（启动 + 坏命令 + help 时）。
fn print_usage() {
    println!(
        "\n── helix-host-cli 遥控器 ──\n\
         send <channelId> <text>            发消息 → posted-echo + posts_update\n\
         read <channelId> [endTimeMs]       标记已读 → post_read + update_channel\n\
         revoke <postId>                    撤回 → post_update\n\
         leave <channelId>                  退群 → channel_member_update + leave 删行(BatchDelete)\n\
         schedule <channelId> <atMs> <text> 定时消息 → post_schedule_created\n\
         cancel <channelId>                 取消定时 → post_schedule_canceled\n\
         create <rawJSON body>              建群 → channel_created（body 含 users[] 等现网字段）\n\
         makeTopic <rawJSON 含 postId>      转话题 → channel_created + post_update\n\
         help | ?                           打印本用法\n\
         quit | exit                        退出（EOF/管道结束亦优雅退出）\n"
    );
}
