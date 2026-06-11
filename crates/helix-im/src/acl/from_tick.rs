//! core Tick → IM 事件翻译（ACL-1 读入端）

use helix_core::Tick;
use crate::sync_session::EventEnvelope;
use crate::error::ImError;

/// 解析 Tick::Inbound 的不透明字节为 IM 强类型事件。
///
/// 外部输入 fail-fast：无法解析时返回 ImError::Parse，不 unwrap。
pub fn parse_inbound_tick(tick: &Tick) -> Result<EventEnvelope, ImError> {
    match tick {
        Tick::Inbound(bytes) => crate::parser::parse_inbound(bytes.as_bytes()),
        _ => Err(ImError::Parse("expected Inbound tick".to_string())),
    }
}

/// 判断此 Tick 是否为 IM 模块感兴趣的入站帧或命令。
///
/// MIN-8：原名 `is_im_inbound` 语义不准确（也处理 Command），改为 `accepts_tick`。
/// PortReply / Timer 通过 corr_map 定向路由，不走此方法。
pub fn accepts_tick(tick: &Tick) -> bool {
    match tick {
        Tick::Inbound(_) => true,
        Tick::Command(cmd) => matches!(
            cmd.name.as_ref(),
            "im_send_message" | "im_read_channel" | "im_reconnect"
        ),
        // PortReply / Timer 通过 corr_map 定向路由，不走 accepts()
        _ => false,
    }
}
