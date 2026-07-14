use crate::sync_session::EventKind;

/// 从 WS type 数字解析 EventKind
pub(super) fn parse_event_kind(type_num: u8) -> EventKind {
    match type_num {
        1 => EventKind::PostUpsert,
        2 => EventKind::PostEdit,
        3 => EventKind::PostRevoke,
        6 => EventKind::PostRead,
        n => EventKind::Other(n),
    }
}
