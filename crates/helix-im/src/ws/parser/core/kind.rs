use crate::error::ImError;
use crate::sync_session::EventKind;

/// 从 WS type 数字解析 EventKind
///
/// type 7 是 closed terminal tombstone。它只能在 sync parser 的严格 wire 校验后由专属
/// 原子提交路径消费；未知 WS action 会在 registry 边界明确拒绝，不能借普通 post
/// gate 或“仅推进 cursor”的兼容分支半消费终态。
pub(crate) fn parse_event_kind(type_num: u8) -> Result<EventKind, ImError> {
    match type_num {
        1 => Ok(EventKind::PostUpsert),
        2 => Ok(EventKind::PostEdit),
        3 => Ok(EventKind::PostRevoke),
        6 => Ok(EventKind::PostRead),
        7 => Ok(EventKind::ChannelTerminalClosed),
        n => Ok(EventKind::Other(n)),
    }
}
