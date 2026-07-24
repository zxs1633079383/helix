//! SurfaceRuntime action 的权威受理、receipt 与 causation 决策。
//!
//! renderer 只能回送已经从 Frame 取得的 opaque `actionRef`。注册表把它绑定到 actor、
//! surface/scope、epoch、revision 和 TTL；这里不执行 HTTP/WS，也不把 accepted 误写成业务完成。
//! 只有后续已校验 Frame 带同一 `causationId`，平台在原子 apply 后才可确认 UI 终态。

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use bytes::Bytes;
use helix_core::effect::{DomainEventBytes, Effect};
use thiserror::Error;

use crate::interaction_frame::{
    ActionBinding, ActionInvocation, ActionInvocationEnvelope, ActionReceipt, ActionReceiptStatus,
    ActionValidationError, InteractionFrameError, InteractionFrameV2, SurfaceId, SurfaceScope,
    INTERACTION_ACTION_RECEIPT_EVENT,
};

/// 上行 action registry 与 replay ledger 共享的有界容量。
///
/// action path 不是 WS message 热路径，但 registry 仍必须 bounded：满时 fail closed，而不是保留无限
/// requestId 或把旧 binding 当作新页面的能力。已受理 requestId 是 in-flight causation key，必须等对应
/// corr_map 消费 HTTP reply 并发出终态 action Frame 后才回收；actionRef 的 revoke/TTL 不能提前释放它。
pub const DEFAULT_ACTION_REGISTRY_CAPACITY: usize = 1_024;
pub const DEFAULT_ACTION_RECEIPT_CAPACITY: usize = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedAction {
    pub receipt: ActionReceipt,
    /// 与后续领域事实 Frame 中 `causationId` 完全相同的 opaque 值。
    pub causation_id: String,
}

/// 已被 binding 固定的 application use case。它永不由 renderer 的 input 推导，也不携带 raw
/// command name；新增可执行 kind 必须在这里显式映射到已有 canonical domain command。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionDispatch {
    /// 查询当前消息的 read receipt。
    ViewReceipt {
        channel_id: String,
        message_id: String,
    },
    MarkRead {
        channel_id: String,
        message_id: String,
    },
    LocatePost {
        channel_id: String,
        message_id: String,
    },
    RevokeMessage {
        channel_id: String,
        message_id: String,
    },
    ForwardMessages {
        channel_id: String,
        source_message_ids: Vec<String>,
    },
    ToggleReaction {
        channel_id: String,
        message_id: String,
    },
    UrgentMessage {
        channel_id: String,
        message_id: String,
    },
    ConfirmUrgent {
        channel_id: String,
        message_id: String,
    },
    CancelUrgent {
        channel_id: String,
        message_id: String,
    },
    /// 没有安全 canonical 映射的 action 永远不能被受理。
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthorityMutationKind {
    Revoke,
    Reaction,
    Urgent,
    Read,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAuthorityAction {
    pub request_id: String,
    pub channel_id: String,
    pub message_id: String,
    pub kind: AuthorityMutationKind,
}

impl ActionDispatch {
    /// 将 timeline VM 中的语义 action kind 固定到已经显式审计的 authority dispatch。
    ///
    /// 这个映射是 projector 与 application registry 的共同真源。只存在 legacy HTTP/read-result
    /// 或尚未有 PersistOk + causation Frame 的动作必须返回 `None`，即使 render-ready row
    /// 宣称了 capability 也只能下发 disabled binding。尤其 `templateConfirm` 不是
    /// `templateReceived`，当前 Go 没有 confirmation authority，必须保持 fail closed。
    pub(crate) fn from_timeline_binding(
        kind: &str,
        channel_id: &str,
        message_id: &str,
    ) -> Option<Self> {
        let channel_id = channel_id.to_string();
        let message_id = message_id.to_string();
        match kind {
            "markRead" => Some(Self::MarkRead {
                channel_id,
                message_id,
            }),
            "viewReceipt" => Some(Self::ViewReceipt {
                channel_id,
                message_id,
            }),
            "locate" => Some(Self::LocatePost {
                channel_id,
                message_id,
            }),
            "revoke" => Some(Self::RevokeMessage {
                channel_id,
                message_id,
            }),
            "forward" => Some(Self::ForwardMessages {
                channel_id,
                source_message_ids: vec![message_id],
            }),
            "react" => Some(Self::ToggleReaction {
                channel_id,
                message_id,
            }),
            "urgent" => Some(Self::UrgentMessage {
                channel_id,
                message_id,
            }),
            "urgentConfirm" => Some(Self::ConfirmUrgent {
                channel_id,
                message_id,
            }),
            "urgentCancel" => Some(Self::CancelUrgent {
                channel_id,
                message_id,
            }),
            _ => None,
        }
    }

    pub(crate) fn supports_timeline_kind(kind: &str) -> bool {
        matches!(
            kind,
            "markRead"
                | "viewReceipt"
                | "locate"
                | "revoke"
                | "forward"
                | "react"
                | "urgent"
                | "urgentConfirm"
                | "urgentCancel"
        )
    }

    pub(crate) fn pending_authority_action(
        &self,
        request_id: &str,
    ) -> Option<PendingAuthorityAction> {
        let (kind, channel_id, message_id) = match self {
            Self::MarkRead {
                channel_id,
                message_id,
            } => (AuthorityMutationKind::Read, channel_id, message_id),
            Self::RevokeMessage {
                channel_id,
                message_id,
            } => (AuthorityMutationKind::Revoke, channel_id, message_id),
            Self::ToggleReaction {
                channel_id,
                message_id,
            } => (AuthorityMutationKind::Reaction, channel_id, message_id),
            Self::UrgentMessage {
                channel_id,
                message_id,
            }
            | Self::ConfirmUrgent {
                channel_id,
                message_id,
            }
            | Self::CancelUrgent {
                channel_id,
                message_id,
            } => (AuthorityMutationKind::Urgent, channel_id, message_id),
            _ => return None,
        };
        Some(PendingAuthorityAction {
            request_id: request_id.to_string(),
            channel_id: channel_id.clone(),
            message_id: message_id.clone(),
            kind,
        })
    }
}

impl ActionDispatch {
    fn validate_input(&self, invocation: &ActionInvocation) -> Result<(), ActionProjectionError> {
        match self {
            Self::MarkRead { .. }
            | Self::ViewReceipt { .. }
            | Self::LocatePost { .. }
            | Self::RevokeMessage { .. }
            | Self::ConfirmUrgent { .. }
            | Self::CancelUrgent { .. }
                if !invocation.input.is_empty() =>
            {
                Err(ActionProjectionError::InvalidInvocation(
                    "action does not accept input fields".to_string(),
                ))
            }
            Self::MarkRead { .. }
            | Self::ViewReceipt { .. }
            | Self::LocatePost { .. }
            | Self::RevokeMessage { .. }
            | Self::ConfirmUrgent { .. }
            | Self::CancelUrgent { .. } => Ok(()),
            Self::ForwardMessages { .. } => validate_forward_input(&invocation.input),
            Self::ToggleReaction { .. } => validate_reaction_input(&invocation.input),
            Self::UrgentMessage { .. } => validate_urgent_input(&invocation.input),
            Self::Unsupported => Err(ActionProjectionError::UnsupportedDispatch),
        }
    }
}

fn validate_forward_input(
    input: &BTreeMap<String, crate::interaction_frame::SurfaceDataVm>,
) -> Result<(), ActionProjectionError> {
    require_exact_input_keys(input, &["targetChannelIds", "mode"])?;
    require_string_array(input, "targetChannelIds", 50)?;
    match input.get("mode") {
        Some(crate::interaction_frame::SurfaceDataVm::String(mode))
            if mode == "individual" || mode == "merged" =>
        {
            Ok(())
        }
        _ => Err(ActionProjectionError::InvalidInvocation(
            "mode must be 'individual' or 'merged'".to_string(),
        )),
    }
}

fn validate_reaction_input(
    input: &BTreeMap<String, crate::interaction_frame::SurfaceDataVm>,
) -> Result<(), ActionProjectionError> {
    require_exact_input_keys(input, &["emoji"])?;
    require_non_empty_string(input, "emoji").map(|_| ())
}

fn validate_urgent_input(
    input: &BTreeMap<String, crate::interaction_frame::SurfaceDataVm>,
) -> Result<(), ActionProjectionError> {
    require_exact_input_keys(input, &["targetIds", "message"])?;
    require_string_array(input, "targetIds", 100)?;
    if input.contains_key("message") {
        require_non_empty_string(input, "message")?;
    }
    Ok(())
}

fn require_exact_input_keys(
    input: &BTreeMap<String, crate::interaction_frame::SurfaceDataVm>,
    allowed: &[&str],
) -> Result<(), ActionProjectionError> {
    if let Some(key) = input.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(ActionProjectionError::InvalidInvocation(format!(
            "unknown input field '{key}'"
        )));
    }
    Ok(())
}

fn require_non_empty_string<'a>(
    input: &'a BTreeMap<String, crate::interaction_frame::SurfaceDataVm>,
    key: &str,
) -> Result<&'a str, ActionProjectionError> {
    match input.get(key) {
        Some(crate::interaction_frame::SurfaceDataVm::String(value)) if !value.is_empty() => {
            Ok(value)
        }
        _ => Err(ActionProjectionError::InvalidInvocation(format!(
            "{key} must be a non-empty string"
        ))),
    }
}

fn require_string_array(
    input: &BTreeMap<String, crate::interaction_frame::SurfaceDataVm>,
    key: &str,
    max: usize,
) -> Result<(), ActionProjectionError> {
    match input.get(key) {
        Some(crate::interaction_frame::SurfaceDataVm::Array(values))
            if !values.is_empty()
                && values.len() <= max
                && values.iter().all(|value| {
                    matches!(value, crate::interaction_frame::SurfaceDataVm::String(text) if !text.is_empty())
                }) =>
        {
            Ok(())
        }
        _ => Err(ActionProjectionError::InvalidInvocation(format!(
            "{key} must contain 1..={max} non-empty strings"
        ))),
    }
}

/// 已通过 binding/actor/scope/epoch/revision/TTL/replay 预检、但尚未写入 replay ledger 的请求。
///
/// 模块先用 `dispatch` 建立既有 canonical command 的 effect plan，再调用 registry commit。这样
/// effect 建模失败不会留下“accepted 但无后续动作”的 requestId。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedAction {
    pub accepted: AcceptedAction,
    pub dispatch: ActionDispatch,
    /// 已认证的 capability 是 replay key 的所有者；只在 registry 内用于随 capability 一起回收。
    action_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionAuthorization {
    Accepted(AuthorizedAction),
    Rejected(ActionReceipt),
}

impl ActionAuthorization {
    pub fn receipt(&self) -> &ActionReceipt {
        match self {
            Self::Accepted(authorized) => &authorized.accepted.receipt,
            Self::Rejected(receipt) => receipt,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionEvaluation {
    Accepted(AcceptedAction),
    Rejected(ActionReceipt),
}

impl ActionEvaluation {
    pub fn receipt(&self) -> &ActionReceipt {
        match self {
            Self::Accepted(accepted) => &accepted.receipt,
            Self::Rejected(receipt) => receipt,
        }
    }
}

/// 在发出某一 Frame 时由 Helix 赋予的 action lease。
///
/// `binding.action_ref` 必须来自 Helix 的 action-id 分配边界，端侧只透传它。SurfaceRuntime 的
/// `TimelineProjector` 在一个 runtime session 内单调发号且不重新签发 retired ref；registry 依赖此
/// producer invariant，才能在 lease 失权时安全回收其 replay key。这里不依赖随机数或系统时间；ref
/// 与 expiry 都由上层在确定性 Tick/IdSource 边界传入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionBindingGrant {
    pub actor_id: String,
    pub surface: SurfaceId,
    pub scope: SurfaceScope,
    pub surface_epoch: u64,
    pub revision: u64,
    pub expires_at_ms: u64,
    pub binding: ActionBinding,
    pub dispatch: ActionDispatch,
}

impl ActionBindingGrant {
    fn validate(&self, now_ms: u64) -> Result<(), ActionProjectionError> {
        if self.actor_id.is_empty() {
            return Err(ActionProjectionError::InvalidGrant(
                "actorId must not be empty".to_string(),
            ));
        }
        if !self.scope.valid_for(self.surface) {
            return Err(ActionProjectionError::InvalidGrant(
                "scope does not match surface".to_string(),
            ));
        }
        if self.surface_epoch == 0 {
            return Err(ActionProjectionError::InvalidGrant(
                "surfaceEpoch must be greater than zero".to_string(),
            ));
        }
        if self.revision == 0 {
            return Err(ActionProjectionError::InvalidGrant(
                "revision must be greater than zero".to_string(),
            ));
        }
        if self.expires_at_ms <= now_ms {
            return Err(ActionProjectionError::InvalidGrant(
                "expiresAtMs must be later than the current Tick time".to_string(),
            ));
        }
        self.binding
            .validate()
            .map_err(|error| ActionProjectionError::InvalidBinding(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RegisteredActionBinding {
    actor_id: String,
    surface: SurfaceId,
    scope_key: String,
    surface_epoch: u64,
    revision: u64,
    expires_at_ms: u64,
    binding: ActionBinding,
    dispatch: ActionDispatch,
}

/// actor/surface-scoped action capability registry.
///
/// 所有 map/set 都只按 key 查找或按 expiry 有序回收；不会从 HashMap 的随机迭代顺序生成 Effect。
/// `grant`、`invoke` 与 `prune` 的复杂度分别为 O(log n)、O(log n) 和 O(expired * log n)，其中 n
/// 由 `capacity` 严格上界限制。
#[derive(Debug)]
pub struct ActionBindingRegistry {
    capacity: usize,
    grants: BTreeMap<String, RegisteredActionBinding>,
    grants_by_expiry: BTreeSet<(u64, String)>,
    /// `requestId -> actionRef` 的有界 in-flight causation ledger。它绝不由 capability revoke/TTL
    /// 回收：只允许终态 action Frame 或已删除相应 corr_map 的 session/identity reset 释放 requestId。
    in_flight_by_request_id: BTreeMap<String, String>,
    /// `actionRef -> requestId` 的反向索引。一个 live binding 同一时刻只能拥有一个 HTTP authority
    /// round-trip，避免不同 requestId 并发后让同一 UI capability 产生无法区分的终态竞争。
    in_flight_by_action_ref: BTreeMap<String, String>,
}

impl ActionBindingRegistry {
    pub fn new(capacity: usize) -> Result<Self, ActionProjectionError> {
        if capacity == 0 {
            return Err(ActionProjectionError::ZeroRegistryCapacity);
        }
        Ok(Self {
            capacity,
            grants: BTreeMap::new(),
            grants_by_expiry: BTreeSet::new(),
            in_flight_by_request_id: BTreeMap::new(),
            in_flight_by_action_ref: BTreeMap::new(),
        })
    }

    pub fn len(&self) -> usize {
        self.grants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    /// 注册或刷新同一 action ref 的 lease。刷新不会改变 ref 的 actor/scope 绑定；上层需要变更上下文时
    /// 必须发出新的 opaque ref，避免陈旧 renderer 继续命中新业务对象。
    pub fn grant(
        &mut self,
        grant: ActionBindingGrant,
        now_ms: u64,
    ) -> Result<(), ActionProjectionError> {
        self.prune_expired(now_ms);
        grant.validate(now_ms)?;

        let action_ref = grant.binding.action_ref.clone();
        if let Some(previous) = self.grants.get(&action_ref) {
            if previous.actor_id != grant.actor_id
                || previous.surface != grant.surface
                || previous.scope_key != grant.scope.canonical_key()
                || previous.surface_epoch != grant.surface_epoch
                || previous.dispatch != grant.dispatch
            {
                return Err(ActionProjectionError::ActionRefContextChanged(action_ref));
            }
        } else if self.grants.len() >= self.capacity {
            return Err(ActionProjectionError::RegistryFull {
                capacity: self.capacity,
            });
        }

        if let Some(previous) = self.grants.remove(&action_ref) {
            self.grants_by_expiry
                .remove(&(previous.expires_at_ms, action_ref.clone()));
        }
        self.grants_by_expiry
            .insert((grant.expires_at_ms, action_ref.clone()));
        self.grants.insert(
            action_ref,
            RegisteredActionBinding {
                actor_id: grant.actor_id,
                surface: grant.surface,
                scope_key: grant.scope.canonical_key(),
                surface_epoch: grant.surface_epoch,
                revision: grant.revision,
                expires_at_ms: grant.expires_at_ms,
                binding: grant.binding,
                dispatch: grant.dispatch,
            },
        );
        Ok(())
    }

    /// 按当前受信 runtime identity 预检一次 action，但不记录 replay ledger。
    ///
    /// `actorId` 是协议回放字段，不是前端授权来源；它必须与 driver 经 header/profile 注入的 runtime
    /// identity 完全一致。空 runtime identity 一律 fail closed。
    pub fn authorize_for_actor(
        &mut self,
        envelope: &ActionInvocationEnvelope,
        trusted_actor_id: &str,
        now_ms: u64,
    ) -> Result<ActionAuthorization, ActionProjectionError> {
        envelope
            .validate()
            .map_err(ActionProjectionError::InvalidEnvelope)?;
        if trusted_actor_id.is_empty() {
            return rejected_authorization(
                &envelope.request_id,
                "actorId",
                "runtime actor is unavailable",
            );
        }
        if envelope.actor_id != trusted_actor_id {
            return rejected_authorization(
                &envelope.request_id,
                "actorId",
                "actor does not match runtime identity",
            );
        }
        self.authorize(envelope, now_ms)
    }

    /// 将已预检的请求记入 replay ledger。调用方必须先成功建立 canonical effect plan，再调用本函数。
    pub fn commit_authorization(
        &mut self,
        authorization: ActionAuthorization,
        receipts: &mut ActionReceiptBus,
    ) -> Result<ActionEvaluation, ActionProjectionError> {
        receipts.ensure_space()?;
        match authorization {
            ActionAuthorization::Rejected(receipt) => {
                receipts.publish(receipt.clone())?;
                Ok(ActionEvaluation::Rejected(receipt))
            }
            ActionAuthorization::Accepted(authorized) => {
                let evaluation = self.commit_without_receipt(authorized)?;
                receipts.publish(evaluation.receipt().clone())?;
                Ok(evaluation)
            }
        }
    }

    /// 受理一条版本化 invoke envelope。
    ///
    /// 格式错误仍作为 protocol error 返回；格式正确但无法执行的请求一律返回可序列化 `rejected`
    /// receipt，因此端侧不会把 transport 成功或空响应误认作 action 已完成。
    pub fn invoke(
        &mut self,
        envelope: &ActionInvocationEnvelope,
        now_ms: u64,
    ) -> Result<ActionEvaluation, ActionProjectionError> {
        match self.authorize(envelope, now_ms)? {
            ActionAuthorization::Rejected(receipt) => Ok(ActionEvaluation::Rejected(receipt)),
            ActionAuthorization::Accepted(authorized) => self.commit_without_receipt(authorized),
        }
    }

    fn authorize(
        &mut self,
        envelope: &ActionInvocationEnvelope,
        now_ms: u64,
    ) -> Result<ActionAuthorization, ActionProjectionError> {
        envelope
            .validate()
            .map_err(ActionProjectionError::InvalidEnvelope)?;

        let action_ref = envelope.invocation.action_ref.as_str();
        let Some(grant) = self.grants.get(action_ref).cloned() else {
            self.prune_expired(now_ms);
            return rejected_authorization(
                &envelope.request_id,
                "actionRef",
                "unknown action binding",
            );
        };
        if grant.expires_at_ms <= now_ms {
            self.remove_grant(action_ref);
            self.prune_expired(now_ms);
            return rejected_authorization(
                &envelope.request_id,
                "actionRef",
                "action binding expired",
            );
        }
        self.prune_expired(now_ms);

        if self
            .in_flight_by_request_id
            .contains_key(&envelope.request_id)
        {
            return rejected_authorization(
                &envelope.request_id,
                "requestId",
                "requestId was already accepted",
            );
        }
        if self
            .in_flight_by_action_ref
            .contains_key(envelope.invocation.action_ref.as_str())
        {
            return rejected_authorization(
                &envelope.request_id,
                "actionRef",
                "action binding already has an action in flight",
            );
        }
        if grant.actor_id != envelope.actor_id {
            return rejected_authorization(
                &envelope.request_id,
                "actorId",
                "actor does not own action binding",
            );
        }
        if grant.surface != envelope.surface {
            return rejected_authorization(
                &envelope.request_id,
                "surface",
                "surface does not own action binding",
            );
        }
        if grant.scope_key != envelope.scope.canonical_key() {
            return rejected_authorization(
                &envelope.request_id,
                "scope",
                "scope does not own action binding",
            );
        }
        if grant.surface_epoch != envelope.surface_epoch {
            return rejected_authorization(
                &envelope.request_id,
                "surfaceEpoch",
                "surface epoch does not own action binding",
            );
        }
        if grant.revision != envelope.observed_revision {
            return rejected_authorization(
                &envelope.request_id,
                "observedRevision",
                "action binding belongs to a different revision",
            );
        }

        let evaluation = evaluate_action(
            &grant.binding,
            envelope.request_id.clone(),
            &envelope.invocation,
        )?;
        let ActionEvaluation::Accepted(accepted) = evaluation else {
            let ActionEvaluation::Rejected(receipt) = evaluation else {
                unreachable!("ActionEvaluation has two variants")
            };
            return Ok(ActionAuthorization::Rejected(receipt));
        };
        match grant.dispatch.validate_input(&envelope.invocation) {
            Ok(()) => Ok(ActionAuthorization::Accepted(AuthorizedAction {
                accepted,
                dispatch: grant.dispatch,
                action_ref: envelope.invocation.action_ref.clone(),
            })),
            Err(ActionProjectionError::UnsupportedDispatch) => rejected_authorization(
                &envelope.request_id,
                "actionRef",
                "action binding has no authority mapping",
            ),
            Err(ActionProjectionError::InvalidInvocation(message)) => {
                rejected_authorization(&envelope.request_id, "input", &message)
            }
            Err(error) => Err(error),
        }
    }

    /// 受理和 receipt publish 是一个连续的纯内存步骤。先检查 bus 容量，避免已经记录 accepted
    /// requestId 却无法把 receipt 交给平台的半完成状态。
    pub fn invoke_and_publish(
        &mut self,
        envelope: &ActionInvocationEnvelope,
        now_ms: u64,
        receipts: &mut ActionReceiptBus,
    ) -> Result<ActionEvaluation, ActionProjectionError> {
        let authorization = self.authorize(envelope, now_ms)?;
        self.commit_authorization(authorization, receipts)
    }

    /// 身份切换或 session reset 时调用；旧 actor 的 action ref 与 replay key 都不能泄漏到新 scope。
    pub fn clear(&mut self) {
        self.grants.clear();
        self.grants_by_expiry.clear();
        self.in_flight_by_request_id.clear();
        self.in_flight_by_action_ref.clear();
    }

    /// 新的同一 surface/scope revision 发出时撤销旧 renderer 所持有的 action lease。
    ///
    /// `grant` / `authorize` 仍是 O(log n) 查找；这里仅在 Frame projection 边界运行一次，扫描上界
    /// 为 registry capacity，避免高频重投影把仍未过期的旧 ref 堆满 registry。
    pub fn revoke_scope(
        &mut self,
        actor_id: &str,
        surface: SurfaceId,
        scope: &SurfaceScope,
        surface_epoch: u64,
    ) {
        let scope_key = scope.canonical_key();
        let action_refs = self
            .grants
            .iter()
            .filter(|(_, grant)| {
                grant.actor_id == actor_id
                    && grant.surface == surface
                    && grant.scope_key == scope_key
                    && grant.surface_epoch == surface_epoch
            })
            .map(|(action_ref, _)| action_ref.clone())
            .collect::<Vec<_>>();
        for action_ref in action_refs {
            self.remove_grant(&action_ref);
        }
    }

    /// 仅在包含相同 causation 的终态 action Frame 已成功加入本 Tick 输出后调用。
    ///
    /// `handle_port_reply` 会先从 corr_map 移除对应上下文，再构造/emit Frame，再调用本函数；因此一旦
    /// requestId 可复用，旧 HTTP reply 已没有路由入口，不可能把同一 causation 误确认给新动作。
    /// session/identity reset 也会在清 registry 前移除 action corr，构成另一条安全释放路径。
    pub fn complete_after_terminal_frame(&mut self, request_id: &str) -> bool {
        let Some(action_ref) = self.in_flight_by_request_id.get(request_id).cloned() else {
            return false;
        };
        let indexes_match = self
            .in_flight_by_action_ref
            .get(&action_ref)
            .is_some_and(|mapped_request_id| mapped_request_id == request_id);
        debug_assert!(
            indexes_match,
            "in-flight action indexes must keep a one-to-one mapping"
        );
        if !indexes_match {
            return false;
        }

        self.in_flight_by_request_id.remove(request_id);
        self.in_flight_by_action_ref.remove(&action_ref);
        true
    }

    fn commit_without_receipt(
        &mut self,
        authorized: AuthorizedAction,
    ) -> Result<ActionEvaluation, ActionProjectionError> {
        let request_id = authorized.accepted.receipt.request_id.clone();
        if self.in_flight_by_request_id.contains_key(&request_id) {
            return rejected(&request_id, "requestId", "requestId was already accepted");
        }
        if self
            .in_flight_by_action_ref
            .contains_key(authorized.action_ref.as_str())
        {
            return rejected(
                &request_id,
                "actionRef",
                "action binding already has an action in flight",
            );
        }
        if self.in_flight_by_request_id.len() >= self.capacity {
            return rejected(
                &request_id,
                "requestId",
                "in-flight action ledger is at capacity",
            );
        }
        let action_ref = authorized.action_ref;
        let previous_request = self
            .in_flight_by_request_id
            .insert(request_id.clone(), action_ref.clone());
        debug_assert!(
            previous_request.is_none(),
            "replay check and insert must stay atomic in the state machine"
        );
        let previous_action = self.in_flight_by_action_ref.insert(action_ref, request_id);
        debug_assert!(
            previous_action.is_none(),
            "single-flight check and insert must stay atomic in the state machine"
        );
        Ok(ActionEvaluation::Accepted(authorized.accepted))
    }

    fn remove_grant(&mut self, action_ref: &str) {
        if let Some(previous) = self.grants.remove(action_ref) {
            self.grants_by_expiry
                .remove(&(previous.expires_at_ms, action_ref.to_string()));
        }
    }

    fn prune_expired(&mut self, now_ms: u64) {
        loop {
            let Some((expires_at_ms, action_ref)) = self.grants_by_expiry.first().cloned() else {
                break;
            };
            if expires_at_ms > now_ms {
                break;
            }
            if self
                .grants
                .get(&action_ref)
                .is_some_and(|grant| grant.expires_at_ms == expires_at_ms)
            {
                self.remove_grant(&action_ref);
            } else {
                self.grants_by_expiry.remove(&(expires_at_ms, action_ref));
            }
        }
    }
}

/// 有界 action receipt 队列。driver/FFI 可以在一个 Effect batch 边界 drain 它，但不能让 receipt
/// 自己更新业务 VM。
#[derive(Debug)]
pub struct ActionReceiptBus {
    capacity: usize,
    pending: VecDeque<ActionReceipt>,
}

/// 把 action receipt 作为单独、可订阅的 protocol event 下行。它不是业务 Frame，accepted 也不代表
/// action 完成；renderer 必须继续等待 matching causation Frame 的原子 apply。
pub fn emit_action_receipt(receipt: ActionReceipt) -> Result<Effect, ActionProjectionError> {
    receipt
        .validate()
        .map_err(|error| ActionProjectionError::InvalidReceipt(error.to_string()))?;
    let payload = serde_json::json!({
        "event": INTERACTION_ACTION_RECEIPT_EVENT,
        "data": receipt,
    });
    let bytes = serde_json::to_vec(&payload)
        .expect("ActionReceipt only contains JSON-serializable contract fields");
    Ok(Effect::Emit {
        event: DomainEventBytes(Bytes::from(bytes)),
    })
}

impl ActionReceiptBus {
    pub fn new(capacity: usize) -> Result<Self, ActionProjectionError> {
        if capacity == 0 {
            return Err(ActionProjectionError::ZeroReceiptBusCapacity);
        }
        Ok(Self {
            capacity,
            pending: VecDeque::with_capacity(capacity),
        })
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn drain(&mut self) -> Vec<ActionReceipt> {
        self.pending.drain(..).collect()
    }

    pub fn clear(&mut self) {
        self.pending.clear();
    }

    fn ensure_space(&self) -> Result<(), ActionProjectionError> {
        if self.pending.len() >= self.capacity {
            return Err(ActionProjectionError::ReceiptBusFull {
                capacity: self.capacity,
            });
        }
        Ok(())
    }

    fn publish(&mut self, receipt: ActionReceipt) -> Result<(), ActionProjectionError> {
        self.ensure_space()?;
        receipt
            .validate()
            .map_err(|error| ActionProjectionError::InvalidReceipt(error.to_string()))?;
        self.pending.push_back(receipt);
        Ok(())
    }
}

/// 下行 Frame 中能证明某 action 有了**权威候选结果**的 mutation 集。
///
/// 该匹配不等于 renderer 已确认；平台必须先让对应 Frame 原子 apply 成功，才可以从 pending overlay
/// 切到最终 UI。此函数仅防止 HTTP 成功或不相关 Frame 误触发确认。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionFrameConfirmation {
    pub request_id: String,
    pub transaction_id: String,
    pub mutations: Vec<ActionFrameMutation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionFrameMutation {
    pub surface: SurfaceId,
    pub scope: SurfaceScope,
    pub revision: u64,
}

pub fn matching_frame_confirmation(
    frame: &InteractionFrameV2,
    request_id: &str,
) -> Result<Option<ActionFrameConfirmation>, ActionProjectionError> {
    if request_id.is_empty() {
        return Err(ActionProjectionError::EmptyRequestId);
    }
    frame
        .validate()
        .map_err(ActionProjectionError::InvalidFrame)?;
    let mutations = frame
        .mutations
        .iter()
        .filter(|mutation| mutation.mutation.causation_id.as_deref() == Some(request_id))
        .map(|mutation| ActionFrameMutation {
            surface: mutation.mutation.surface,
            scope: mutation.mutation.scope.clone(),
            revision: mutation.mutation.revision,
        })
        .collect::<Vec<_>>();
    if mutations.is_empty() {
        return Ok(None);
    }
    Ok(Some(ActionFrameConfirmation {
        request_id: request_id.to_string(),
        transaction_id: frame.transaction_id.clone(),
        mutations,
    }))
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
    #[error("action invocation envelope is invalid: {0}")]
    InvalidEnvelope(InteractionFrameError),
    #[error("action frame is invalid: {0}")]
    InvalidFrame(InteractionFrameError),
    #[error("action binding grant is invalid: {0}")]
    InvalidGrant(String),
    #[error("action registry capacity must be greater than zero")]
    ZeroRegistryCapacity,
    #[error("action receipt bus capacity must be greater than zero")]
    ZeroReceiptBusCapacity,
    #[error("action registry is full (capacity {capacity})")]
    RegistryFull { capacity: usize },
    #[error("action receipt bus is full (capacity {capacity})")]
    ReceiptBusFull { capacity: usize },
    #[error("actionRef {0} cannot change actor, surface, scope or epoch while it is active")]
    ActionRefContextChanged(String),
    #[error("action binding has no supported canonical dispatch")]
    UnsupportedDispatch,
}

/// 受理一个由 renderer 从 `ActionBinding` 原样回传的动作。
///
/// 这个纯函数保留给 registry 与单元测试复用。它只检查 binding 自身和 input shape；actor/scope/TTL/
/// revision/replay 由 [`ActionBindingRegistry`] 在调用前完成。
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

    rejected_with_errors(request_id, errors)
}

fn rejected(
    request_id: &str,
    field: &str,
    message: &str,
) -> Result<ActionEvaluation, ActionProjectionError> {
    rejected_with_errors(
        request_id.to_string(),
        vec![ActionValidationError {
            field: field.to_string(),
            message: message.to_string(),
        }],
    )
}

fn rejected_with_errors(
    request_id: String,
    errors: Vec<ActionValidationError>,
) -> Result<ActionEvaluation, ActionProjectionError> {
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

fn rejected_authorization(
    request_id: &str,
    field: &str,
    message: &str,
) -> Result<ActionAuthorization, ActionProjectionError> {
    let ActionEvaluation::Rejected(receipt) = rejected(request_id, field, message)? else {
        unreachable!("rejected helper always returns a rejection")
    };
    Ok(ActionAuthorization::Rejected(receipt))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::interaction_frame::{
        ActionInvocationEnvelope, InteractionFrameV2, InteractionRuntimeVm, RuntimeConnectionState,
        RuntimeRecoveryState, RuntimeSyncState, SurfaceMutation, SurfaceMutationV2,
        TypedSurfaceMutation, UnreadSummaryVm, ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
        INTERACTION_FRAME_SCHEMA_VERSION,
    };

    const ACTION_REF: &str = "act.v1.5BXd5bpZ4w6zWZbL";

    fn binding(enabled: bool) -> ActionBinding {
        ActionBinding {
            action_ref: ACTION_REF.to_string(),
            kind: "viewReceipt".to_string(),
            input_shape: "object".to_string(),
            surface_epoch: 3,
            enabled,
            disabled_reason: (!enabled).then(|| "notAvailable".to_string()),
        }
    }

    fn scope() -> SurfaceScope {
        SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        }
    }

    fn invocation(action_ref: &str) -> ActionInvocation {
        ActionInvocation {
            action_ref: action_ref.to_string(),
            input: BTreeMap::new(),
        }
    }

    fn envelope(request_id: &str) -> ActionInvocationEnvelope {
        ActionInvocationEnvelope {
            schema_version: ACTION_INVOCATION_ENVELOPE_SCHEMA_VERSION,
            request_id: request_id.to_string(),
            actor_id: "viewer-1".to_string(),
            surface: SurfaceId::MessageTimeline,
            scope: scope(),
            surface_epoch: 3,
            observed_revision: 7,
            invocation: invocation(ACTION_REF),
        }
    }

    fn envelope_for_action_ref(request_id: &str, action_ref: &str) -> ActionInvocationEnvelope {
        let mut envelope = envelope(request_id);
        envelope.invocation = invocation(action_ref);
        envelope
    }

    fn grant(enabled: bool, expires_at_ms: u64) -> ActionBindingGrant {
        ActionBindingGrant {
            actor_id: "viewer-1".to_string(),
            surface: SurfaceId::MessageTimeline,
            scope: scope(),
            surface_epoch: 3,
            revision: 7,
            expires_at_ms,
            binding: binding(enabled),
            dispatch: ActionDispatch::ViewReceipt {
                channel_id: "channel-1".to_string(),
                message_id: "message-1".to_string(),
            },
        }
    }

    fn grant_for_action_ref(action_ref: &str, expires_at_ms: u64) -> ActionBindingGrant {
        let mut grant = grant(true, expires_at_ms);
        grant.binding.action_ref = action_ref.to_string();
        grant
    }

    fn registry() -> ActionBindingRegistry {
        ActionBindingRegistry::new(4).expect("nonzero bounded registry")
    }

    #[test]
    fn timeline_dispatch_map_only_admits_explicit_authority_paths() {
        for kind in [
            "markRead",
            "viewReceipt",
            "locate",
            "revoke",
            "forward",
            "react",
            "urgent",
            "urgentConfirm",
            "urgentCancel",
        ] {
            assert!(ActionDispatch::supports_timeline_kind(kind), "{kind}");
            assert!(
                ActionDispatch::from_timeline_binding(kind, "channel-1", "message-1").is_some(),
                "{kind} must share the projector/application dispatch source"
            );
        }

        for kind in [
            "reply",
            "createTopic",
            "retry",
            "templateConfirm",
            "createSchedule",
            "cancelSchedule",
            "markChannelRead",
            "closeChannel",
            "leaveChannel",
            "changeMembers",
            "changeRole",
            "changeSettings",
        ] {
            assert!(!ActionDispatch::supports_timeline_kind(kind), "{kind}");
            assert!(
                ActionDispatch::from_timeline_binding(kind, "channel-1", "message-1").is_none(),
                "{kind} must remain disabled until authority, persistence and causation Frame exist"
            );
        }
    }

    #[test]
    fn accepted_action_only_exposes_a_pending_causation_key() {
        let ActionEvaluation::Accepted(accepted) =
            evaluate_action(&binding(true), "request-1", &invocation(ACTION_REF)).unwrap()
        else {
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
            &invocation("act.v1.different"),
        )
        .unwrap() else {
            panic!("mismatched disabled action must be rejected")
        };
        assert_eq!(receipt.status, ActionReceiptStatus::Rejected);
        assert_eq!(receipt.validation_errors.unwrap().len(), 2);
    }

    #[test]
    fn registry_rejects_unknown_expired_and_replayed_action_refs() {
        let mut registry = registry();
        let unknown = registry.invoke(&envelope("request-unknown"), 10).unwrap();
        assert!(matches!(&unknown, ActionEvaluation::Rejected(_)));
        assert_eq!(
            unknown.receipt().validation_errors.as_ref().unwrap()[0].field,
            "actionRef"
        );

        registry
            .grant(grant_for_action_ref("act.v1.expired", 20), 10)
            .unwrap();
        let expired = registry
            .invoke(
                &envelope_for_action_ref("request-expired", "act.v1.expired"),
                20,
            )
            .unwrap();
        assert!(matches!(&expired, ActionEvaluation::Rejected(_)));
        assert_eq!(
            expired.receipt().validation_errors.as_ref().unwrap()[0].message,
            "action binding expired"
        );

        registry.grant(grant(true, 40), 21).unwrap();
        let accepted = registry.invoke(&envelope("request-once"), 22).unwrap();
        assert!(matches!(&accepted, ActionEvaluation::Accepted(_)));
        registry.grant(grant(true, 60), 23).unwrap();
        let replay = registry.invoke(&envelope("request-once"), 24).unwrap();
        assert!(matches!(&replay, ActionEvaluation::Rejected(_)));
        assert_eq!(
            replay.receipt().validation_errors.as_ref().unwrap()[0].field,
            "requestId"
        );
    }

    // 回归锚: T079 —— 旧 actionRef 失权不能提前释放仍有 corr_map 的 requestId。
    #[test]
    fn registry_keeps_request_reserved_across_scope_revocation_until_terminal_frame() {
        let mut registry = registry();
        let old_ref = "act.v1.revoked.old";
        let new_ref = "act.v1.revoked.new";
        let request_id = "request-revoked-old";
        registry
            .grant(grant_for_action_ref(old_ref, 40), 10)
            .unwrap();
        assert!(matches!(
            registry
                .invoke(&envelope_for_action_ref(request_id, old_ref), 11)
                .unwrap(),
            ActionEvaluation::Accepted(_)
        ));

        registry.revoke_scope("viewer-1", SurfaceId::MessageTimeline, &scope(), 3);
        assert!(registry.is_empty());
        assert_eq!(
            registry
                .in_flight_by_request_id
                .get(request_id)
                .map(String::as_str),
            Some(old_ref)
        );
        registry
            .grant(grant_for_action_ref(new_ref, 50), 12)
            .unwrap();
        let replay = registry
            .invoke(&envelope_for_action_ref(request_id, new_ref), 13)
            .unwrap();
        assert_eq!(
            replay.receipt().validation_errors.as_ref().unwrap()[0].field,
            "requestId"
        );

        assert!(registry.complete_after_terminal_frame(request_id));
        assert!(!registry.complete_after_terminal_frame(request_id));
        assert!(matches!(
            registry
                .invoke(&envelope_for_action_ref(request_id, new_ref), 14)
                .unwrap(),
            ActionEvaluation::Accepted(_)
        ));
    }

    // 回归锚: T079 —— TTL 同样只撤 capability，不可让旧 HTTP causation 与新动作碰撞。
    #[test]
    fn registry_keeps_request_reserved_across_ttl_expiry_until_terminal_frame() {
        let mut registry = ActionBindingRegistry::new(1).expect("nonzero bounded registry");
        let old_ref = "act.v1.ttl.old";
        let new_ref = "act.v1.ttl.new";
        let request_id = "request-ttl-old";
        registry
            .grant(grant_for_action_ref(old_ref, 20), 10)
            .unwrap();
        assert!(matches!(
            registry
                .invoke(&envelope_for_action_ref(request_id, old_ref), 11)
                .unwrap(),
            ActionEvaluation::Accepted(_)
        ));

        // grant() 的 now=20 会 prune 旧 lease；in-flight request 仍必须保留。
        registry
            .grant(grant_for_action_ref(new_ref, 40), 20)
            .unwrap();
        let replay = registry
            .invoke(&envelope_for_action_ref(request_id, new_ref), 21)
            .unwrap();
        assert_eq!(
            replay.receipt().validation_errors.as_ref().unwrap()[0].field,
            "requestId"
        );

        assert!(registry.complete_after_terminal_frame(request_id));
        assert!(matches!(
            registry
                .invoke(&envelope_for_action_ref(request_id, new_ref), 22)
                .unwrap(),
            ActionEvaluation::Accepted(_)
        ));
    }

    // 回归锚: T080 —— 同一个 capability 只能有一条 authority round-trip 在途。
    #[test]
    fn registry_rejects_a_second_request_for_the_same_live_action_ref() {
        let mut registry = registry();
        registry.grant(grant(true, 40), 10).unwrap();
        assert!(matches!(
            registry.invoke(&envelope("request-first"), 11).unwrap(),
            ActionEvaluation::Accepted(_)
        ));

        let second = registry.invoke(&envelope("request-second"), 12).unwrap();
        assert_eq!(
            second.receipt().validation_errors.as_ref().unwrap()[0].field,
            "actionRef"
        );
        assert!(registry.complete_after_terminal_frame("request-first"));
        assert!(matches!(
            registry.invoke(&envelope("request-second"), 13).unwrap(),
            ActionEvaluation::Accepted(_)
        ));
    }

    #[test]
    fn registry_bounds_in_flight_requests_until_their_terminal_frames_arrive() {
        let mut registry = ActionBindingRegistry::new(2).expect("nonzero bounded registry");
        let first_ref = "act.v1.ledger.1";
        let second_ref = "act.v1.ledger.2";
        registry
            .grant(grant_for_action_ref(first_ref, 40), 10)
            .unwrap();
        registry
            .grant(grant_for_action_ref(second_ref, 40), 10)
            .unwrap();
        assert!(matches!(
            registry
                .invoke(&envelope_for_action_ref("request-ledger-1", first_ref), 11)
                .unwrap(),
            ActionEvaluation::Accepted(_)
        ));
        assert!(matches!(
            registry
                .invoke(&envelope_for_action_ref("request-ledger-2", second_ref), 11)
                .unwrap(),
            ActionEvaluation::Accepted(_)
        ));

        registry.revoke_scope("viewer-1", SurfaceId::MessageTimeline, &scope(), 3);
        registry
            .grant(grant_for_action_ref("act.v1.ledger.fresh", 50), 12)
            .unwrap();
        let full = registry
            .invoke(
                &envelope_for_action_ref("request-ledger-fresh", "act.v1.ledger.fresh"),
                13,
            )
            .unwrap();
        assert_eq!(
            full.receipt().validation_errors.as_ref().unwrap()[0].message,
            "in-flight action ledger is at capacity"
        );

        assert!(registry.complete_after_terminal_frame("request-ledger-1"));
        assert!(registry.complete_after_terminal_frame("request-ledger-2"));
        assert!(matches!(
            registry
                .invoke(
                    &envelope_for_action_ref("request-ledger-fresh", "act.v1.ledger.fresh"),
                    14,
                )
                .unwrap(),
            ActionEvaluation::Accepted(_)
        ));
    }

    #[test]
    fn registry_rejects_actor_scope_epoch_and_revision_mismatch_before_dispatch() {
        let mut registry = registry();
        registry.grant(grant(true, 40), 10).unwrap();

        let mut wrong_actor = envelope("request-actor");
        wrong_actor.actor_id = "viewer-2".to_string();
        let actor = registry.invoke(&wrong_actor, 11).unwrap();
        assert_eq!(
            actor.receipt().validation_errors.as_ref().unwrap()[0].field,
            "actorId"
        );

        let mut wrong_scope = envelope("request-scope");
        wrong_scope.scope = SurfaceScope::Timeline {
            channel_id: "channel-2".to_string(),
            window_token: "latest".to_string(),
        };
        let scoped = registry.invoke(&wrong_scope, 11).unwrap();
        assert_eq!(
            scoped.receipt().validation_errors.as_ref().unwrap()[0].field,
            "scope"
        );

        let mut wrong_epoch = envelope("request-epoch");
        wrong_epoch.surface_epoch = 4;
        let epoch = registry.invoke(&wrong_epoch, 11).unwrap();
        assert_eq!(
            epoch.receipt().validation_errors.as_ref().unwrap()[0].field,
            "surfaceEpoch"
        );

        let mut wrong_revision = envelope("request-revision");
        wrong_revision.observed_revision = 8;
        let revision = registry.invoke(&wrong_revision, 11).unwrap();
        assert_eq!(
            revision.receipt().validation_errors.as_ref().unwrap()[0].field,
            "observedRevision"
        );
    }

    #[test]
    fn registry_requires_the_host_runtime_actor_before_binding_lookup() {
        let mut registry = registry();
        registry.grant(grant(true, 40), 10).unwrap();

        let authorization = registry
            .authorize_for_actor(&envelope("request-runtime-actor"), "viewer-2", 11)
            .unwrap();
        let ActionAuthorization::Rejected(receipt) = authorization else {
            panic!("caller actor is not an authority source")
        };
        assert_eq!(receipt.status, ActionReceiptStatus::Rejected);
        assert_eq!(
            receipt.validation_errors.as_ref().unwrap()[0].field,
            "actorId"
        );
    }

    #[test]
    fn receipt_bus_is_bounded_and_does_not_lose_an_accepted_request() {
        let mut registry = registry();
        registry.grant(grant(true, 40), 10).unwrap();
        let mut receipts = ActionReceiptBus::new(1).unwrap();

        let accepted = registry
            .invoke_and_publish(&envelope("request-bus-1"), 11, &mut receipts)
            .unwrap();
        assert!(matches!(&accepted, ActionEvaluation::Accepted(_)));
        assert_eq!(receipts.len(), 1);
        assert!(matches!(
            registry.invoke_and_publish(&envelope("request-bus-2"), 12, &mut receipts),
            Err(ActionProjectionError::ReceiptBusFull { capacity: 1 })
        ));
        let second = registry.invoke(&envelope("request-bus-2"), 12).unwrap();
        assert_eq!(
            second.receipt().validation_errors.as_ref().unwrap()[0].field,
            "actionRef"
        );
        assert!(registry.complete_after_terminal_frame("request-bus-1"));
        assert!(matches!(
            registry.invoke(&envelope("request-bus-2"), 13).unwrap(),
            ActionEvaluation::Accepted(_)
        ));
    }

    #[test]
    fn matching_causation_requires_a_valid_frame_and_exact_request_id() {
        let frame = InteractionFrameV2 {
            schema_version: INTERACTION_FRAME_SCHEMA_VERSION,
            transaction_id: "transaction-1".to_string(),
            causation_id: Some("request-1".to_string()),
            runtime: InteractionRuntimeVm {
                connection: RuntimeConnectionState::Online,
                initial_sync: RuntimeSyncState::Ready,
                recovery: RuntimeRecoveryState::Idle,
            },
            mutations: vec![SurfaceMutationV2 {
                surface_epoch: 1,
                mutation: SurfaceMutation {
                    surface: SurfaceId::MessageUnreadSummary,
                    scope: SurfaceScope::User {
                        user_id: "viewer-1".to_string(),
                        team_id: "team-1".to_string(),
                    },
                    revision: 1,
                    base_revision: None,
                    causation_id: Some("request-1".to_string()),
                    mutation: TypedSurfaceMutation::UnreadSummarySnapshot {
                        view: UnreadSummaryVm {
                            unread_total: 1,
                            mention_total: 0,
                            urgent_total: 0,
                            unread_channel_count: 1,
                        },
                    },
                },
            }],
            directives: Vec::new(),
        };

        let confirmation = matching_frame_confirmation(&frame, "request-1")
            .unwrap()
            .expect("matching causation must be discoverable after frame validation");
        assert_eq!(confirmation.transaction_id, "transaction-1");
        assert_eq!(confirmation.mutations.len(), 1);
        assert!(matching_frame_confirmation(&frame, "request-2")
            .unwrap()
            .is_none());

        let mut invalid = frame;
        invalid.mutations[0].mutation.revision = 0;
        assert!(matches!(
            matching_frame_confirmation(&invalid, "request-1"),
            Err(ActionProjectionError::InvalidFrame(_))
        ));
    }
}
