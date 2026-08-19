//! 文字接龙 command 入口：生成稳定 operationId，注册 HTTP correlation，并保持 host 只做通用 I/O。

use crate::chain::{self, ChainMutation, ChainMutationState, ChainRequest};
use crate::error::ImError;
use crate::state::CorrelationContext;
use helix_core::EffectSink;

impl super::ImModule {
    /// 处理文字接龙 canonical command，业务字段与 operationId 在 Helix 边界收敛。
    pub(crate) fn handle_chain_command(
        &mut self,
        command: &str,
        payload: &[u8],
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let args: serde_json::Value = serde_json::from_slice(payload)
            .map_err(|error| ImError::Parse(format!("{command} payload: {error}")))?;
        let operation_id = if matches!(
            command,
            "post_chain_create"
                | "post_chain_publish"
                | "post_chain_append"
                | "post_chain_reconcile"
                | "post_chain_close"
                | "post_chain_retract"
        ) {
            chain::stable_operation_id(command, &args).or_else(|| {
                args.get("operation_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
        } else {
            None
        };
        let enriched_payload = chain::add_operation_id(payload, operation_id.as_deref())?;
        let request = chain::request_from_command(command, &enriched_payload)?;
        if request.is_mutation() {
            self.ensure_chain_pending(&request);
        }
        self.start_chain_http(request, out)
    }

    /// 在 transport 请求发出前登记 PENDING，但不提前发布业务终态事件。
    fn ensure_chain_pending(&mut self, request: &ChainRequest) {
        let (Some(client_mutation_id), Some(operation_id)) = (
            request.client_mutation_id.as_ref(),
            request.operation_id.as_ref(),
        ) else {
            return;
        };
        let state = self
            .state
            .chain_mutations
            .get(client_mutation_id)
            .map(|mutation| mutation.state)
            .unwrap_or(ChainMutationState::Pending);
        self.state.chain_mutations.insert(
            client_mutation_id.clone(),
            ChainMutation {
                client_mutation_id: client_mutation_id.clone(),
                operation_id: operation_id.clone(),
                state,
                entry_id: self
                    .state
                    .chain_mutations
                    .get(client_mutation_id)
                    .and_then(|mutation| mutation.entry_id.clone()),
                error_code: None,
            },
        );
    }

    /// 把接龙请求注册为单一 ChainHttp correlation，并把 outbound Effect 原样交给 host。
    pub(crate) fn start_chain_http(
        &mut self,
        request: ChainRequest,
        out: &mut EffectSink,
    ) -> Result<(), ImError> {
        let corr = self.alloc_corr_internal();
        let effects = crate::commands::handle_outbound(
            request.command.as_str(),
            &serde_json::to_vec(&request.payload)
                .map_err(|error| ImError::Serialize(error.to_string()))?,
            self.config.api_base_url.as_str(),
            self.config.default_api_base_url.as_str(),
            self.state.connection_id.as_deref(),
            corr,
        )?;
        self.state.corr_map.insert(
            corr,
            CorrelationContext::ChainHttp {
                request: Box::new(request),
            },
        );
        for effect in effects {
            out.push(effect);
        }
        Ok(())
    }
}
