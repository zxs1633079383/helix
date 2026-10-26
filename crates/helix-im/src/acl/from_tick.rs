//! core Tick → IM 事件翻译（ACL-1 读入端）

use helix_core::Tick;

/// 判断此 Tick 是否为 IM 模块感兴趣的入站帧或命令。
///
/// MIN-8：原名 `is_im_inbound` 语义不准确（也处理 Command），改为 `accepts_tick`。
/// PortReply / PortProgress / Timer 通过映射定向路由，不走此方法。
///
/// R4-tombstone：旧 `parse_inbound_tick`（重构 A 前的二趟解析入口）已删——
/// 解析在 `ws::WsFrame::parse` + registry handler 内一次分到底（见 module.rs `Tick::Inbound` 臂），
/// 全仓 src+tests 零调用方。
pub fn accepts_tick(tick: &Tick) -> bool {
    match tick {
        Tick::Inbound(_) => true,
        Tick::Command(cmd) => {
            // `im_read_channel` 在 public contract 层归一为已注册的 `im_channels_view`，不作为 core
            // command 单独放行；单条 `im_mark_read` 则归一后走 `im_post_read`。
            matches!(
                cmd.name.as_ref(),
                crate::module::RUNTIME_IDENTITY_COMMAND
                    | "im_translate_message"
                    | "im_cancel_translation"
                    | "im_send_message"
                    | "im_retry_send"
                    | "im_retry_upload"
                    | "im_reconnect"
                    | "im_sync_channels"
            )
                || crate::query::forward_detail::is_command(cmd.name.as_ref())
                || crate::commands::is_outbound(cmd.name.as_ref())
                || crate::chain::is_command(cmd.name.as_ref())
                // query 族（im_load_older_context / dialog_list 等纯读投影命令）：handle 分发
                // 有臂（module.rs `is_query(name)` 分支），但此 accepts 闸原漏放行 → accept()==false
                // 静默丢弃整族（Round-5 e2e 揪出 UC-2.2 上拉历史不可达·单测直调纯逻辑绕过本闸故漏网）。
                || crate::query::is_query(cmd.name.as_ref())
        }
        // 连接状态变化：IM 要响应（cursor reset / proactive resync 策略在 im 层）
        Tick::Connected(_) | Tick::Disconnected(_) => true,
        // PortReply / PortProgress / Timer 通过映射定向路由，不走 accepts()
        _ => false,
    }
}
