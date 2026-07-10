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
            // F1-gap1：`im_read_channel` 已从 allowlist 删除——它是 `im_post_read` 的悬空别名。
            // 行为真源 cses-client/src-tauri/src/features/im/ 无 read_channel 概念：唯一「已读」
            // outbound = `post/read`（= 已注册的 `im_post_read`，走 is_outbound 分支）；channel 级
            // 已读回声 `im:channel:read_echo` 仅由 sync apply 路径（gate apply type6 read）产出，
            // 不是命令。保留它只会让 accepts 放行一个 module.rs 无臂的悬空命令 → 必落 unhandled
            // no-op。`im_reconnect` 是真连接控制命令（module.rs 有臂 emit 控制信号），保留。
            matches!(
                cmd.name.as_ref(),
                "im_send_message" | "im_retry_send" | "im_retry_upload" | "im_reconnect"
            )
                || crate::commands::is_outbound(cmd.name.as_ref())
                // query 族（im_load_older_context / dialog_list 等纯读投影命令）：handle 分发
                // 有臂（module.rs `is_query(name)` 分支），但此 accepts 闸原漏放行 → accept()==false
                // 静默丢弃整族（Round-5 e2e 揪出 UC-2.2 上拉历史不可达·单测直调纯逻辑绕过本闸故漏网）。
                || crate::query::is_query(cmd.name.as_ref())
        }
        // 连接状态变化：IM 要响应（cursor reset / proactive resync 策略在 im 层）
        Tick::Connected(_) | Tick::Disconnected(_) => true,
        // PortReply / Timer 通过 corr_map 定向路由，不走 accepts()
        _ => false,
    }
}
