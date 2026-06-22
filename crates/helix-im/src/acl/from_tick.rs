//! core Tick → IM 事件翻译（ACL-1 读入端）

use helix_core::Tick;

/// 判断此 Tick 是否为 IM 模块感兴趣的入站帧或命令。
///
/// MIN-8：原名 `is_im_inbound` 语义不准确（也处理 Command），改为 `accepts_tick`。
/// PortReply / Timer 通过 corr_map 定向路由，不走此方法。
///
/// R4-tombstone：旧 `parse_inbound_tick`（重构 A 前的二趟解析入口）已删——
/// 解析在 `ws::WsFrame::parse` + registry handler 内一次分到底（见 module.rs `Tick::Inbound` 臂），
/// 全仓 src+tests 零调用方。
pub fn accepts_tick(tick: &Tick) -> bool {
    match tick {
        Tick::Inbound(_) => true,
        Tick::Command(cmd) => {
            matches!(
                cmd.name.as_ref(),
                "im_send_message" | "im_read_channel" | "im_reconnect"
            ) || crate::commands::is_outbound(cmd.name.as_ref())
        }
        // 连接状态变化：IM 要响应（cursor reset / proactive resync 策略在 im 层）
        Tick::Connected(_) | Tick::Disconnected(_) => true,
        // PortReply / Timer 通过 corr_map 定向路由，不走 accepts()
        _ => false,
    }
}
