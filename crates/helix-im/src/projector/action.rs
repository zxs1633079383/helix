//! SurfaceRuntime action 的 receipt / causation 决策。
//!
//! 这里仅裁决 binding 与 opaque invocation 是否可受理；accepted 不表示业务完成。完成态必须等
//! 后续 `InteractionFrame` 携带同一 `causationId`，因此端侧不能把 HTTP 成功或 receipt 当成 VM 更新。

use thiserror::Error;

use crate::interaction_frame::{
    ActionBinding, ActionInvocation, ActionReceipt, ActionReceiptStatus, ActionValidationError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedAction {
    pub receipt: ActionReceipt,
    /// 与异步领域变更 Frame 中 `causationId` 完全相同的 opaque 值。
    pub causation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionEvaluation {
    Accepted(AcceptedAction),
    Rejected(ActionReceipt),
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActionProjectionError {
    #[error("action requestId must not be empty")]
    EmptyRequestId,
    #[error("action binding is not a valid v1 binding: {0}")]
    InvalidBinding(String),
    #[error("action invocation is not a valid v1 invocation: {0}")]
    InvalidInvocation(String),
    #[error("action receipt is not a valid v1 receipt: {0}")]
    InvalidReceipt(String),
}

/// 受理一个由 renderer 从 `ActionBinding` 原样回传的动作。
///
/// action ref 不匹配、binding 禁用或 input shape 不支持均返回 `rejected` receipt；只有 receipt
/// `accepted` 才携带可用于后续 Frame 的 causation id。该 id 就是 caller 的 opaque `requestId`，不
/// 在 UI/FFI 层重编码或拆解。
pub fn evaluate_action(
    binding: &ActionBinding,
    request_id: impl Into<String>,
    invocation: &ActionInvocation,
) -> Result<ActionEvaluation, ActionProjectionError> {
    binding
        .validate()
        .map_err(|error| ActionProjectionError::InvalidBinding(error.to_string()))?;
    invocation
        .validate()
        .map_err(|error| ActionProjectionError::InvalidInvocation(error.to_string()))?;

    let request_id = request_id.into();
    if request_id.is_empty() {
        return Err(ActionProjectionError::EmptyRequestId);
    }

    let mut errors = Vec::new();
    if invocation.action_ref != binding.action_ref {
        errors.push(ActionValidationError {
            field: "actionRef".to_string(),
            message: "does not match the bound action".to_string(),
        });
    }
    if !binding.enabled {
        errors.push(ActionValidationError {
            field: "actionRef".to_string(),
            message: binding
                .disabled_reason
                .clone()
                .unwrap_or_else(|| "not available".to_string()),
        });
    }
    if binding.input_shape != "object" {
        errors.push(ActionValidationError {
            field: "input".to_string(),
            message: "unsupported input shape".to_string(),
        });
    }

    if errors.is_empty() {
        let receipt = ActionReceipt {
            request_id: request_id.clone(),
            status: ActionReceiptStatus::Accepted,
            validation_errors: None,
        };
        receipt
            .validate()
            .map_err(|error| ActionProjectionError::InvalidReceipt(error.to_string()))?;
        return Ok(ActionEvaluation::Accepted(AcceptedAction {
            receipt,
            causation_id: request_id,
        }));
    }

    let receipt = ActionReceipt {
        request_id,
        status: ActionReceiptStatus::Rejected,
        validation_errors: Some(errors),
    };
    receipt
        .validate()
        .map_err(|error| ActionProjectionError::InvalidReceipt(error.to_string()))?;
    Ok(ActionEvaluation::Rejected(receipt))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::interaction_frame::SurfaceDataVm;

    fn binding(enabled: bool) -> ActionBinding {
        ActionBinding {
            action_ref: "message:message-1:createTopic".to_string(),
            kind: "createTopic".to_string(),
            input_shape: "object".to_string(),
            enabled,
            disabled_reason: (!enabled).then(|| "notAvailable".to_string()),
        }
    }

    fn invocation(action_ref: &str) -> ActionInvocation {
        ActionInvocation {
            action_ref: action_ref.to_string(),
            input: BTreeMap::from([(
                "title".to_string(),
                SurfaceDataVm::String("Roadmap".to_string()),
            )]),
        }
    }

    #[test]
    fn accepted_action_only_exposes_a_pending_causation_key() {
        let ActionEvaluation::Accepted(accepted) = evaluate_action(
            &binding(true),
            "request-1",
            &invocation("message:message-1:createTopic"),
        )
        .unwrap() else {
            panic!("enabled matching action must be accepted")
        };
        assert_eq!(accepted.receipt.status, ActionReceiptStatus::Accepted);
        assert_eq!(accepted.causation_id, "request-1");
        assert!(accepted.receipt.validation_errors.is_none());
    }

    #[test]
    fn rejected_action_preserves_validation_errors_and_has_no_causation() {
        let ActionEvaluation::Rejected(receipt) = evaluate_action(
            &binding(false),
            "request-2",
            &invocation("message:message-1:revoke"),
        )
        .unwrap() else {
            panic!("mismatched disabled action must be rejected")
        };
        assert_eq!(receipt.status, ActionReceiptStatus::Rejected);
        assert_eq!(receipt.validation_errors.unwrap().len(), 2);
    }
}
