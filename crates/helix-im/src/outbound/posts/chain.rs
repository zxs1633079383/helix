//! 文字接龙 outbound HTTP registry。
//!
//! 每个命令只负责把 canonical snake_case intent 翻译为 Go endpoint/body；认证头、base URL
//! 和 `Effect::Http` 仍由公共 registry 统一组装。

use serde_json::Value;

use crate::error::ImError;
use crate::outbound::registry::{OutboundCommand, OutboundRegistration};

macro_rules! chain_command {
    ($ty:ident, $name:literal) => {
        struct $ty;

        impl OutboundCommand for $ty {
            /// 返回文字接龙 canonical 命令名。
            fn name(&self) -> &'static str {
                $name
            }

            /// 把接龙 intent 交给领域 builder 进行字段校验和 wire 编码。
            fn build(&self, args: &Value) -> Result<(&'static str, Value), ImError> {
                crate::chain::wire_body($name, args)
            }

            /// get 是 HTTP response projection，其余命令由 chain PortReply 结算。
            fn is_read(&self) -> bool {
                $name == "post_chain_get"
            }
        }

        inventory::submit! {
            OutboundRegistration {
                name: $name,
                command: &$ty,
            }
        }
    };
}

chain_command!(CreateChainCommand, "post_chain_create");
chain_command!(PublishChainCommand, "post_chain_publish");
chain_command!(AppendChainCommand, "post_chain_append");
chain_command!(GetChainCommand, "post_chain_get");
chain_command!(ReconcileChainCommand, "post_chain_reconcile");
chain_command!(CloseChainCommand, "post_chain_close");
chain_command!(RetractChainCommand, "post_chain_retract");
chain_command!(MarkChainReadCommand, "post_chain_mark_read");
