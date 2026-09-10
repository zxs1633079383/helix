//! 分类接龙 outbound 命令注册。
//!
//! 请求体和 endpoint 只由 category_chain::wire 收口；本文件只把十一个 canonical
//! command 接入既有 outbound registry，并标出 HTTP response 即结果的查询命令。

use serde_json::Value;

use crate::category_chain::wire;
use crate::error::ImError;
use crate::outbound::registry::{OutboundCommand, OutboundRegistration};

/// 为一个 canonical category-chain command 生成 registry 条目。
macro_rules! category_command {
    ($type:ident, $name:literal) => {
        struct $type;

        impl OutboundCommand for $type {
            /// 返回稳定的分类接龙 canonical 命令名。
            fn name(&self) -> &'static str {
                $name
            }

            /// 让 wire 模块完成输入校验、snake→camel 映射和路径选择。
            fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
                wire::wire_body($name, args)
            }

            /// 查询命令把 HTTP data 透传为 read result；mutation 需等待 authority/persist。
            fn is_read(&self) -> bool {
                crate::category_chain::is_read($name)
            }
        }

        inventory::submit! {
            OutboundRegistration {
                name: $name,
                command: &$type,
            }
        }
    };
}

category_command!(
    CategoryChainCreateDraftCommand,
    "category_chain_create_draft"
);
category_command!(
    CategoryChainUpdateDraftCommand,
    "category_chain_update_draft"
);
category_command!(CategoryChainPublishCommand, "category_chain_publish");
category_command!(CategoryChainGetCommand, "category_chain_get");
category_command!(CategoryChainGetMineCommand, "category_chain_get_mine");
category_command!(
    CategoryChainSetParticipationCommand,
    "category_chain_set_participation"
);
category_command!(CategoryChainEntriesCommand, "category_chain_entries");
category_command!(CategoryChainCloseCommand, "category_chain_close");
category_command!(CategoryChainRetractCommand, "category_chain_retract");
category_command!(CategoryChainReconcileCommand, "category_chain_reconcile");

category_command!(
    CategoryChainCapabilitiesCommand,
    "category_chain_capabilities"
);
