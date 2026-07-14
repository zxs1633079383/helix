//! Dead-action graceful no-op：`posted` / `post_urgent` / `post_edited`（P4 dead-action 清理）。
//!
//! 行为真源（现网 source of truth）：csesapi send/广播路径**只走 `post`**（WebsocketEventPost），
//! echo 对账与新消息统一经 `post.rs` handler（temporary_id ON CONFLICT 自然对账）。`posted`
//! （WebsocketEventPosted）是 mattermost core 遗留事件，**非** csesapi 业务路径；`post_urgent`/
//! `post_edited` 是内部归一名（wire 实际是 `post_update`），从不作为独立 wire action 出现。
//!
//! P4 前 helix 曾在此做 `posted` 帧的 temporary_id reconcile（legacy 残影），与「send echo 只走
//! `post`」契约冲突（见 `p1_send_build_test.rs:321` 留给 main 裁决的 blocker）。本次按 02 文档 P4
//! 「不要实现 dead action posted/post_urgent/post_edited」下线该 reconcile：echo 对账唯一真路径 =
//! `post.rs::reconcile_post_echo`。
//!
//! 仍**注册**这三个 action（而非删除）——保留 graceful no-op：若线上偶发收到这类遗留帧，经 registry
//! 正常路由不冒 `UnsupportedWsAction` 硬错（module.rs dispatch 把 UnsupportedWsAction 升 CoreError），
//! 对齐真源 `router.rs:46 _ => vec![]` 未知 action 静默口径。无任何业务 Effect。

use helix_core::EffectSink;

use crate::error::ImError;

use super::super::{ImWsContext, WsFrame, WsHandlerRegistration, WsMessageHandler};

const POSTED_ACTION: &str = "posted";
const POST_URGENT_ACTION: &str = "post_urgent";
const POST_EDITED_ACTION: &str = "post_edited";

/// 参数化 dead-action no-op handler（三个遗留 action 共用形态，各自独立注册）。
struct DeadActionHandler {
    action: &'static str,
}

impl WsMessageHandler for DeadActionHandler {
    fn action(&self) -> &'static str {
        self.action
    }

    fn handle(
        &self,
        _ctx: &mut ImWsContext<'_>,
        _frame: &WsFrame,
        _out: &mut EffectSink,
    ) -> Result<(), ImError> {
        // dead action：不实现业务逻辑（echo 真路径 = post.rs）。no-op，无 Effect。
        Ok(())
    }
}

static POSTED_HANDLER: DeadActionHandler = DeadActionHandler {
    action: POSTED_ACTION,
};
static POST_URGENT_HANDLER: DeadActionHandler = DeadActionHandler {
    action: POST_URGENT_ACTION,
};
static POST_EDITED_HANDLER: DeadActionHandler = DeadActionHandler {
    action: POST_EDITED_ACTION,
};

inventory::submit! {
    WsHandlerRegistration {
        action: POSTED_ACTION,
        handler: &POSTED_HANDLER,
    }
}

inventory::submit! {
    WsHandlerRegistration {
        action: POST_URGENT_ACTION,
        handler: &POST_URGENT_HANDLER,
    }
}

inventory::submit! {
    WsHandlerRegistration {
        action: POST_EDITED_ACTION,
        handler: &POST_EDITED_HANDLER,
    }
}
