//! `message.timeline` 的有界窗口投影。
//!
//! 输入是现有 `query::render_ready` 已经完成身份、顺序和 capability 裁决的消息行；本模块
//! 只在 Helix 内把它们收紧成 `TimelineWindowVm`。客户端不会再读取 raw row 或自行排序。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use thiserror::Error;

use crate::interaction_frame::{
    ActionBinding, DeliveryVm, ForwardModeVm, ForwardSourceVm, ForwardedItemContentVm,
    ForwardedItemVm, InteractionFrameError, MediaItemVm, MessageContentVm, MessageQuickActionVm,
    MessageReactionVm, MessageVm, ReferenceAvailabilityVm, ReplyPreviewVm, ReplySummaryVm,
    ReplyThreadPreviewVm, ReplyThreadSummaryVm, SurfaceDataVm, SurfaceId, SurfaceLoadState,
    SurfaceMutation, SurfaceScope, TimelineActiveWindowVm, TimelineAnchorMode, TimelineAnchorVm,
    TimelineDeltaOperationVm, TimelineEntityKeyVm, TimelineEntityVm, TimelineInsertPositionVm,
    TimelineRangeVm, TimelineSnapshotV3Vm, TimelineWindowVm, TypedSurfaceMutation, UserRefVm,
    WindowPageVm, MAX_SAFE_JSON_INTEGER, MIN_SAFE_JSON_INTEGER,
};

/// 单次快照及 Consumer 可见窗口最多承载的消息数。
pub const MAX_TIMELINE_WINDOW_ITEMS: usize = 60;
/// Timeline V3 所有方向的缺省页长。
pub const DEFAULT_TIMELINE_PAGE_SIZE: u32 = 20;
/// Timeline V3 所有方向的单页硬上限。
pub const MAX_TIMELINE_PAGE_SIZE: u32 = MAX_TIMELINE_WINDOW_ITEMS as u32;
/// Helix Range 保留定位页及其上下各一页，渲染窗口仍由 Consumer 限制为单页大小。
const MAX_TIMELINE_RANGE_ITEMS: usize = MAX_TIMELINE_WINDOW_ITEMS * 3;
const MAX_SURFACE_DATA_DEPTH: usize = 8;
const MAX_SURFACE_DATA_ITEMS: usize = 128;

/// 已验证的 Timeline 页长，令非法 0/61 状态无法进入查询编排。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelinePageSize(u32);

impl TimelinePageSize {
    /// 建立 1..=60 的页长；越界值 fail closed。
    pub fn new(value: u32) -> Result<Self, TimelinePagePolicyError> {
        if (1..=MAX_TIMELINE_PAGE_SIZE).contains(&value) {
            Ok(Self(value))
        } else {
            Err(TimelinePagePolicyError::OutOfRange(value))
        }
    }

    /// 从可选 JSON 字段解析页长；缺失时使用 20，非整数与越界值拒绝。
    pub fn parse(value: Option<&Value>) -> Result<Self, TimelinePagePolicyError> {
        let Some(value) = value else {
            return Ok(Self(DEFAULT_TIMELINE_PAGE_SIZE));
        };
        let raw = value
            .as_u64()
            .ok_or(TimelinePagePolicyError::NotUnsignedInteger)?;
        let raw = u32::try_from(raw).map_err(|_| TimelinePagePolicyError::TooLarge(raw))?;
        Self::new(raw)
    }

    /// 返回已验证的协议页长。
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// 定位窗口在目标两侧的确定性配额。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocateWindowAllocation {
    pub before: usize,
    pub after: usize,
}

/// 远端 Timeline V3 分页对既有窗口的唯一合并方向。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelinePageMutation {
    /// 向旧侧扩展 Range；Consumer 独立维护可见窗口淘汰。
    Older,
    /// 向新侧扩展 Range；Consumer 独立维护可见窗口淘汰。
    Newer,
    /// 以远端上下文替换活动 segment，并只对当前导航令牌激活。
    Locate {
        target_message_id: String,
        navigation_token: String,
        activate: bool,
    },
}

impl LocateWindowAllocation {
    /// 返回包含目标在内的窗口总实体数。
    pub const fn total(self) -> usize {
        self.before + 1 + self.after
    }

    /// 返回目标在升序窗口中的确定性下标。
    pub const fn target_index(self) -> usize {
        self.before
    }
}

/// 依据可用两侧数量分配 locate 配额，并把短侧余额补给长侧。
pub fn allocate_locate_window(
    available_before: usize,
    available_after: usize,
    page_size: TimelinePageSize,
) -> LocateWindowAllocation {
    let page_size = page_size.get() as usize;
    let desired_before = page_size / 2;
    let desired_after = page_size - 1 - desired_before;
    let mut before = available_before.min(desired_before);
    let mut after = available_after.min(desired_after);
    let unused_before = desired_before - before;
    let unused_after = desired_after - after;
    after += unused_before.min(available_after.saturating_sub(after));
    before += unused_after.min(available_before.saturating_sub(before));
    LocateWindowAllocation { before, after }
}

/// pageSize 在进入任何 storage/HTTP 编排前的验证错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimelinePagePolicyError {
    #[error("pageSize must be an unsigned integer")]
    NotUnsignedInteger,
    #[error("pageSize {0} is outside 1..=60")]
    OutOfRange(u32),
    #[error("pageSize {0} cannot be represented")]
    TooLarge(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineWindowRequest {
    pub channel_id: String,
    pub window_token: String,
    pub anchor: TimelineAnchorVm,
    pub first_unread_message_id: Option<String>,
    pub target_message_id: Option<String>,
    pub page: WindowPageVm,
    pub state: SurfaceLoadState,
    /// 当前请求可见页长；默认 20，调用方只能注入已验证的 1..=60。
    pub page_size: u32,
    /// 由 SurfaceRuntime 当前 session epoch 注入；所有 action binding 必须原样暴露该值给 renderer。
    pub surface_epoch: u64,
    /// runtime identity 缺失时不能下发可 invoke 的 binding；renderer 仍能看到显式禁用原因。
    pub action_authority_available: bool,
}

impl TimelineWindowRequest {
    pub fn latest(channel_id: impl Into<String>) -> Self {
        Self::latest_with_window_token(channel_id, "latest")
    }

    pub fn latest_with_window_token(
        channel_id: impl Into<String>,
        window_token: impl Into<String>,
    ) -> Self {
        let channel_id = channel_id.into();
        let window_token = window_token.into();
        Self {
            channel_id,
            window_token: window_token.clone(),
            anchor: TimelineAnchorVm {
                message_id: None,
                mode: TimelineAnchorMode::Latest,
            },
            first_unread_message_id: None,
            target_message_id: None,
            page: WindowPageVm {
                window_token,
                has_older: false,
                has_newer: false,
                has_more: false,
            },
            state: SurfaceLoadState::Ready,
            page_size: DEFAULT_TIMELINE_PAGE_SIZE,
            surface_epoch: 1,
            action_authority_available: true,
        }
    }
}

#[derive(Debug)]
pub struct TimelineProjector {
    slots: BTreeMap<String, TimelineSlot>,
    // An attach request may race the first durable fact for its channel. Keep
    // that intent separately from the first snapshot so terminal writes can
    // schedule the authoritative readback instead of falling back to a raw
    // post event while the initial storage query is in flight.
    attached_slots: BTreeMap<String, (String, String)>,
    /// 每个 attached scope 当前可激活的 locate token；旧 response 只能合并数据。
    current_navigation_tokens: BTreeMap<String, String>,
    /// 仅用于 opaque actionRef 的 Helix 内部发号；绝不包含 channel/message/kind 等业务标识。
    next_action_ref: u64,
}

#[derive(Debug, Clone)]
struct TimelineSlot {
    revision: u64,
    view: TimelineWindowVm,
}

impl Default for TimelineProjector {
    fn default() -> Self {
        Self {
            slots: BTreeMap::new(),
            attached_slots: BTreeMap::new(),
            current_navigation_tokens: BTreeMap::new(),
            next_action_ref: 1,
        }
    }
}

impl TimelineProjector {
    /// 把一次 locate intent 登记为该窗口唯一可激活导航。
    pub(crate) fn begin_locate_navigation(
        &mut self,
        channel_id: &str,
        window_token: &str,
        navigation_token: &str,
    ) {
        let scope = SurfaceScope::Timeline {
            channel_id: channel_id.to_string(),
            window_token: window_token.to_string(),
        };
        self.current_navigation_tokens
            .insert(scope.canonical_key(), navigation_token.to_string());
    }

    /// 判断 response token 是否仍是窗口当前导航，供晚到链路降级为数据合并。
    pub(crate) fn is_current_locate_navigation(
        &self,
        channel_id: &str,
        window_token: &str,
        navigation_token: &str,
    ) -> bool {
        let scope = SurfaceScope::Timeline {
            channel_id: channel_id.to_string(),
            window_token: window_token.to_string(),
        };
        self.current_navigation_tokens
            .get(&scope.canonical_key())
            .is_some_and(|current| current == navigation_token)
    }

    /// Returns the unique attached window that currently contains `anchor_message_id`.
    ///
    /// G10a's wire command intentionally carries no renderer-owned window token. Helix resolves
    /// that token from its attached typed state and rejects ambiguous or detached anchors.
    pub(crate) fn window_context_for_anchor(
        &self,
        channel_id: &str,
        anchor_message_id: &str,
    ) -> Option<(String, usize)> {
        let mut matches = self.slots.values().filter(|slot| {
            slot.view.channel_id == channel_id
                && slot
                    .view
                    .items
                    .iter()
                    .any(|item| item.id == anchor_message_id)
        });
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some((first.view.page.window_token.clone(), first.view.items.len()))
    }

    /// 返回唯一 attached anchor 的窗口、可见数和稳定复合游标。
    pub(crate) fn paging_context_for_anchor(
        &self,
        channel_id: &str,
        anchor_message_id: &str,
    ) -> Option<(WindowPageVm, usize, TimelineEntityKeyVm)> {
        let mut matches = self.slots.values().filter_map(|slot| {
            if slot.view.channel_id != channel_id {
                return None;
            }
            let message = slot
                .view
                .items
                .iter()
                .find(|item| item.id == anchor_message_id)?;
            Some((
                slot.view.page.clone(),
                slot.view.items.len(),
                TimelineEntityKeyVm {
                    create_at: message.created_at,
                    temporary_id: message.temporary_id.clone(),
                },
            ))
        });
        let first = matches.next()?;
        if matches.next().is_some() || first.2.temporary_id.is_empty() {
            return None;
        }
        Some(first)
    }

    /// 返回频道唯一 attached slot，定位命令不能由 renderer 任意选择内部 Range。
    pub(crate) fn unique_attached_window_for_channel(
        &self,
        channel_id: &str,
    ) -> Option<(String, usize)> {
        let mut matches =
            self.attached_slots
                .values()
                .filter_map(|(attached_channel_id, window_token)| {
                    if attached_channel_id != channel_id {
                        return None;
                    }
                    let scope = SurfaceScope::Timeline {
                        channel_id: attached_channel_id.clone(),
                        window_token: window_token.clone(),
                    };
                    Some((
                        window_token.clone(),
                        self.current_view(&scope).map_or(0, |view| view.items.len()),
                    ))
                });
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    /// Advances one attached G10a window to a failure state without dropping its existing items.
    ///
    /// HTTP or persistence failure ends the loading transaction but cannot erase the last
    /// authoritative timeline merely to signal failure.
    pub(crate) fn patch_failed_for_anchor(
        &mut self,
        channel_id: &str,
        anchor_message_id: &str,
    ) -> Result<Option<SurfaceMutation>, TimelineProjectionError> {
        let Some((slot_key, slot)) = self
            .slots
            .iter()
            .filter(|(_, slot)| {
                slot.view.channel_id == channel_id
                    && slot
                        .view
                        .items
                        .iter()
                        .any(|item| item.id == anchor_message_id)
            })
            .map(|(key, slot)| (key.clone(), slot.clone()))
            .next()
        else {
            return Ok(None);
        };
        let previous_view = slot.view.clone();
        let mut view = slot.view;
        view.state = SurfaceLoadState::Failed;
        let revision = next_revision(slot.revision)?;
        let scope = SurfaceScope::Timeline {
            channel_id: view.channel_id.clone(),
            window_token: view.page.window_token.clone(),
        };
        let mutation = SurfaceMutation {
            surface: SurfaceId::MessageTimeline,
            scope,
            revision,
            base_revision: Some(slot.revision),
            causation_id: None,
            mutation: TypedSurfaceMutation::TimelineDelta {
                operations: delta_between(&previous_view, &view, None)?,
            },
        };
        mutation.validate()?;
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(Some(mutation))
    }

    /// 清空 identity/session 绑定的窗口、attachment 与当前导航裁决状态。
    pub fn reset(&mut self) {
        self.slots.clear();
        self.attached_slots.clear();
        self.current_navigation_tokens.clear();
    }

    pub fn register_attachment(&mut self, scope: &SurfaceScope) {
        if let SurfaceScope::Timeline {
            channel_id,
            window_token,
        } = scope
        {
            self.attached_slots.insert(
                scope.canonical_key(),
                (channel_id.clone(), window_token.clone()),
            );
        }
    }

    pub fn is_attached(&self, scope: &SurfaceScope) -> bool {
        self.attached_slots.contains_key(&scope.canonical_key())
    }

    /// 建立完整 snapshot。`render_ready_rows` 必须已经是时间正序；本函数不会重新排序。
    pub fn snapshot_from_render_ready(
        &mut self,
        request: TimelineWindowRequest,
        render_ready_rows: &[Value],
    ) -> Result<SurfaceMutation, TimelineProjectionError> {
        self.snapshot_from_render_ready_with_causation(request, render_ready_rows, None)
    }

    pub fn snapshot_from_render_ready_with_causation(
        &mut self,
        request: TimelineWindowRequest,
        render_ready_rows: &[Value],
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, TimelineProjectionError> {
        let (scope, slot_key) = scope_and_slot_key(&request)?;
        let view = self.build_view(&request, render_ready_rows)?;
        let revision = self
            .slots
            .get(&slot_key)
            .map_or(Ok(1), |slot| next_revision(slot.revision))?;
        let mutation = SurfaceMutation {
            surface: SurfaceId::MessageTimeline,
            scope,
            revision,
            base_revision: None,
            causation_id,
            mutation: TypedSurfaceMutation::TimelineSnapshot {
                view: snapshot_v3(&view)?,
            },
        };
        mutation.validate()?;
        self.attached_slots.insert(
            slot_key.clone(),
            (view.channel_id.clone(), view.page.window_token.clone()),
        );
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(mutation)
    }

    /// 对已 attach slot 生成有界 V3 Delta；普通消息更新不覆盖整个窗口。
    pub fn patch_from_render_ready(
        &mut self,
        request: TimelineWindowRequest,
        render_ready_rows: &[Value],
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, TimelineProjectionError> {
        let (scope, slot_key) = scope_and_slot_key(&request)?;
        let slot =
            self.slots
                .get(&slot_key)
                .ok_or_else(|| TimelineProjectionError::MissingSnapshot {
                    slot: slot_key.clone(),
                })?;
        let base_revision = slot.revision;
        let previous_view = slot.view.clone();
        let mut view = self.build_view(&request, render_ready_rows)?;
        // 普通分页/增量回复只能扩展当前窗口；仅携带 targetMessageId 的 locate 可切 anchor。
        if request.target_message_id.is_none() {
            view.anchor = previous_view.anchor.clone();
            view.first_unread_message_id = previous_view.first_unread_message_id.clone();
            view.target_message_id = previous_view.target_message_id.clone();
        }
        preserve_receipt_snapshot(&previous_view, &mut view);
        preserve_stable_action_bindings(&previous_view, &mut view);
        let revision = next_revision(base_revision)?;
        let mutation = SurfaceMutation {
            surface: SurfaceId::MessageTimeline,
            scope,
            revision,
            base_revision: Some(base_revision),
            causation_id,
            mutation: TypedSurfaceMutation::TimelineDelta {
                operations: delta_between(&previous_view, &view, None)?,
            },
        };
        mutation.validate()?;
        self.attached_slots.insert(
            slot_key.clone(),
            (view.channel_id.clone(), view.page.window_token.clone()),
        );
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(mutation)
    }

    /// 把已通过持久屏障的远端页合并进既有 typed slot，不重建全量数组语义。
    pub(crate) fn patch_page_from_render_ready(
        &mut self,
        mut request: TimelineWindowRequest,
        render_ready_rows: &[Value],
        page_mutation: TimelinePageMutation,
        causation_id: Option<String>,
    ) -> Result<SurfaceMutation, TimelineProjectionError> {
        let (scope, slot_key) = scope_and_slot_key(&request)?;
        let slot = self.slots.get(&slot_key).cloned().ok_or_else(|| {
            TimelineProjectionError::MissingSnapshot {
                slot: slot_key.clone(),
            }
        })?;
        TimelinePageSize::new(request.page_size)
            .map_err(TimelineProjectionError::InvalidPageSize)?;

        let previous_view = slot.view.clone();
        let mut incoming = Vec::with_capacity(render_ready_rows.len());
        for row in render_ready_rows {
            incoming.push(self.message_from_render_ready(
                row,
                request.channel_id.as_str(),
                request.surface_epoch,
                request.action_authority_available,
            )?);
        }
        if let TimelinePageMutation::Locate {
            target_message_id,
            navigation_token,
            activate: false,
        } = &page_mutation
        {
            // 旧 locate 使用独立 Range 安装已持久化事实，但绝不改写当前活动 view。
            let mut stale_view = previous_view.clone();
            stale_view.range_id =
                located_range_id(scope.canonical_key().as_str(), navigation_token);
            stale_view.items = incoming;
            stale_view.anchor = TimelineAnchorVm {
                message_id: Some(target_message_id.clone()),
                mode: TimelineAnchorMode::Locate,
            };
            stale_view.target_message_id = Some(target_message_id.clone());
            stale_view.page = request.page;
            if !stale_view
                .items
                .iter()
                .any(|message| message.id == *target_message_id)
            {
                return Err(TimelineProjectionError::TargetNotInWindow {
                    target: target_message_id.clone(),
                });
            }
            let entities = timeline_entities(&stale_view)?;
            let ordered_keys = entities
                .iter()
                .map(|entity| entity.entity_key.clone())
                .collect::<Vec<_>>();
            let mut operations = entities
                .iter()
                .cloned()
                .map(|entity| TimelineDeltaOperationVm::UpsertMessage { entity })
                .collect::<Vec<_>>();
            operations.push(TimelineDeltaOperationVm::InstallSegment {
                range: timeline_range(&stale_view, ordered_keys),
                entities,
            });
            let revision = next_revision(slot.revision)?;
            let mutation = SurfaceMutation {
                surface: SurfaceId::MessageTimeline,
                scope,
                revision,
                base_revision: Some(slot.revision),
                causation_id,
                mutation: TypedSurfaceMutation::TimelineDelta { operations },
            };
            mutation.validate()?;
            self.slots.insert(
                slot_key,
                TimelineSlot {
                    revision,
                    view: previous_view,
                },
            );
            return Ok(mutation);
        }
        let mut by_key = BTreeMap::new();
        for message in previous_view.items.iter().chain(incoming.iter()) {
            if message.temporary_id.is_empty() {
                return Err(TimelineProjectionError::MissingRowField {
                    field: "temporaryId",
                });
            }
            by_key.insert(
                TimelineEntityKeyVm {
                    create_at: message.created_at,
                    temporary_id: message.temporary_id.clone(),
                },
                message.clone(),
            );
        }

        let mut view = previous_view.clone();
        let navigation_token = match &page_mutation {
            TimelinePageMutation::Locate {
                target_message_id,
                navigation_token,
                activate,
            } => {
                if *activate {
                    view.range_id =
                        located_range_id(scope.canonical_key().as_str(), navigation_token);
                    by_key.clear();
                    for message in incoming {
                        by_key.insert(
                            TimelineEntityKeyVm {
                                create_at: message.created_at,
                                temporary_id: message.temporary_id.clone(),
                            },
                            message,
                        );
                    }
                    request.anchor = TimelineAnchorVm {
                        message_id: Some(target_message_id.clone()),
                        mode: TimelineAnchorMode::Locate,
                    };
                    request.target_message_id = Some(target_message_id.clone());
                    view.anchor = request.anchor.clone();
                    view.target_message_id = request.target_message_id.clone();
                    Some(navigation_token.clone())
                } else {
                    // 旧 response 的消息已通过 cache barrier，可扩充数据但不能回滚活动窗口状态。
                    request.anchor = previous_view.anchor.clone();
                    request.target_message_id = previous_view.target_message_id.clone();
                    request.page = previous_view.page.clone();
                    None
                }
            }
            TimelinePageMutation::Older => {
                request.page.has_newer |= previous_view.page.has_newer;
                request.page.has_more = request.page.has_older || request.page.has_newer;
                None
            }
            TimelinePageMutation::Newer => {
                request.page.has_older |= previous_view.page.has_older;
                request.page.has_more = request.page.has_older || request.page.has_newer;
                None
            }
        };
        let mut items = by_key.into_values().collect::<Vec<_>>();
        let overflow = items.len().saturating_sub(MAX_TIMELINE_RANGE_ITEMS);
        if overflow > 0 {
            match page_mutation {
                TimelinePageMutation::Older => {
                    items.truncate(MAX_TIMELINE_RANGE_ITEMS);
                    request.page.has_newer = true;
                    request.page.has_more = true;
                }
                TimelinePageMutation::Newer => {
                    items.drain(..overflow);
                    request.page.has_older = true;
                    request.page.has_more = true;
                }
                TimelinePageMutation::Locate { .. } => {
                    return Err(TimelineProjectionError::SurfaceDataLimit);
                }
            }
        }
        view.items = items;
        view.page = request.page;
        view.state = request.state;
        if let Some(target) = view.target_message_id.as_deref() {
            if !view.items.iter().any(|message| message.id == target) {
                return Err(TimelineProjectionError::TargetNotInWindow {
                    target: target.to_string(),
                });
            }
        }
        preserve_receipt_snapshot(&previous_view, &mut view);
        preserve_stable_action_bindings(&previous_view, &mut view);

        let revision = next_revision(slot.revision)?;
        let mutation = SurfaceMutation {
            surface: SurfaceId::MessageTimeline,
            scope,
            revision,
            base_revision: Some(slot.revision),
            causation_id,
            mutation: TypedSurfaceMutation::TimelineDelta {
                operations: delta_between(&previous_view, &view, navigation_token)?,
            },
        };
        mutation.validate()?;
        self.attached_slots.insert(
            slot_key.clone(),
            (view.channel_id.clone(), view.page.window_token.clone()),
        );
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(mutation)
    }

    pub fn current_view(&self, scope: &SurfaceScope) -> Option<&TimelineWindowVm> {
        self.slots
            .get(&scope.canonical_key())
            .map(|slot| &slot.view)
    }

    /// 返回目标所在的唯一 attached channel，供 locate HTTP authority 做同 scope 校验。
    pub(crate) fn located_target_channel(&self, target_message_id: &str) -> Option<&str> {
        self.slots
            .values()
            .find(|slot| {
                slot.view
                    .items
                    .iter()
                    .any(|item| item.id == target_message_id)
            })
            .map(|slot| slot.view.channel_id.as_str())
    }

    pub(crate) fn patch_located_target(
        &mut self,
        target_message_id: &str,
        navigation_token: Option<&str>,
    ) -> Result<Option<SurfaceMutation>, TimelineProjectionError> {
        let Some((slot_key, slot)) = self
            .slots
            .iter()
            .find(|(_, slot)| {
                slot.view
                    .items
                    .iter()
                    .any(|item| item.id == target_message_id)
            })
            .map(|(key, slot)| (key.clone(), slot.clone()))
        else {
            return Ok(None);
        };
        let previous_view = slot.view.clone();
        let mut view = slot.view;
        view.anchor = TimelineAnchorVm {
            message_id: Some(target_message_id.to_string()),
            mode: TimelineAnchorMode::Locate,
        };
        view.target_message_id = Some(target_message_id.to_string());
        let revision = next_revision(slot.revision)?;
        let scope = SurfaceScope::Timeline {
            channel_id: view.channel_id.clone(),
            window_token: view.page.window_token.clone(),
        };
        let mutation = SurfaceMutation {
            surface: SurfaceId::MessageTimeline,
            scope,
            revision,
            base_revision: Some(slot.revision),
            causation_id: None,
            mutation: TypedSurfaceMutation::TimelineDelta {
                operations: delta_between(
                    &previous_view,
                    &view,
                    Some(
                        navigation_token
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("locate:{target_message_id}")),
                    ),
                )?,
            },
        };
        mutation.validate()?;
        self.slots.insert(slot_key, TimelineSlot { revision, view });
        Ok(Some(mutation))
    }

    pub(crate) fn patch_receipts(
        &mut self,
        receipts: &[Value],
    ) -> Result<Vec<SurfaceMutation>, TimelineProjectionError> {
        let by_post: BTreeMap<&str, &Value> = receipts
            .iter()
            .filter_map(|receipt| {
                receipt
                    .get("postId")
                    .and_then(Value::as_str)
                    .map(|id| (id, receipt))
            })
            .collect();
        if by_post.is_empty() {
            return Ok(Vec::new());
        }
        let keys: Vec<String> = self.slots.keys().cloned().collect();
        let mut mutations = Vec::new();
        for slot_key in keys {
            let Some(slot) = self.slots.get(&slot_key).cloned() else {
                continue;
            };
            let previous_view = slot.view.clone();
            let mut view = slot.view;
            let mut changed = false;
            for item in &mut view.items {
                let Some(receipt) = by_post.get(item.id.as_str()) else {
                    continue;
                };
                item.read_receipt = surface_data(receipt, 0)?;
                changed = true;
            }
            if !changed {
                continue;
            }
            let revision = next_revision(slot.revision)?;
            let scope = SurfaceScope::Timeline {
                channel_id: view.channel_id.clone(),
                window_token: view.page.window_token.clone(),
            };
            let mutation = SurfaceMutation {
                surface: SurfaceId::MessageTimeline,
                scope,
                revision,
                base_revision: Some(slot.revision),
                causation_id: None,
                mutation: TypedSurfaceMutation::TimelineDelta {
                    operations: delta_between(&previous_view, &view, None)?,
                },
            };
            mutation.validate()?;
            self.slots.insert(slot_key, TimelineSlot { revision, view });
            mutations.push(mutation);
        }
        Ok(mutations)
    }

    /// Builds one bounded timeline and collapses topic mirror replies into their root card.
    fn build_view(
        &mut self,
        request: &TimelineWindowRequest,
        render_ready_rows: &[Value],
    ) -> Result<TimelineWindowVm, TimelineProjectionError> {
        let page_size = TimelinePageSize::new(request.page_size)
            .map_err(TimelineProjectionError::InvalidPageSize)?;
        let (bounded_rows, page) = bounded_rows(
            render_ready_rows,
            &request.anchor,
            &request.page,
            page_size.get() as usize,
        );
        let mut items = Vec::with_capacity(bounded_rows.len());
        for row in bounded_rows {
            items.push(self.message_from_render_ready(
                row,
                request.channel_id.as_str(),
                request.surface_epoch,
                request.action_authority_available,
            )?);
        }
        enrich_reply_threads(&mut items);
        collapse_topic_reply_children(&mut items);
        if let Some(target) = request.target_message_id.as_deref() {
            if !items.iter().any(|item| item.id == target) {
                return Err(TimelineProjectionError::TargetNotInWindow {
                    target: target.to_string(),
                });
            }
        }

        let mut anchor = request.anchor.clone();
        if anchor.message_id.is_none() {
            anchor.message_id = items.last().map(|item| item.id.clone());
        }
        Ok(TimelineWindowVm {
            channel_id: request.channel_id.clone(),
            range_id: request.page.window_token.clone(),
            anchor,
            items,
            first_unread_message_id: request.first_unread_message_id.clone(),
            target_message_id: request.target_message_id.clone(),
            page,
            state: request.state.clone(),
        })
    }

    fn message_from_render_ready(
        &mut self,
        row: &Value,
        expected_channel_id: &str,
        surface_epoch: u64,
        action_authority_available: bool,
    ) -> Result<MessageVm, TimelineProjectionError> {
        message_from_render_ready(
            row,
            expected_channel_id,
            surface_epoch,
            action_authority_available,
            || self.issue_action_ref(),
        )
    }

    fn issue_action_ref(&mut self) -> Result<String, TimelineProjectionError> {
        let sequence = self.next_action_ref;
        self.next_action_ref = self
            .next_action_ref
            .checked_add(1)
            .ok_or(TimelineProjectionError::ActionRefOverflow)?;
        // 两段 reversible-mix 的序列不暴露 domain identity 或可由 renderer 拼出的 command context。
        // 授权安全仍完全由 registry 的 actor/scope/epoch/revision/TTL/replay 校验承担，而非猜测 ref。
        Ok(format!(
            "act.v1.{:016x}{:016x}",
            mix_action_ref(sequence),
            mix_action_ref(sequence ^ 0x9e37_79b9_7f4a_7c15)
        ))
    }
}

/// 将内部有界窗口转换为标准化 V3 Snapshot。
fn snapshot_v3(view: &TimelineWindowVm) -> Result<TimelineSnapshotV3Vm, TimelineProjectionError> {
    let entities = timeline_entities(view)?;
    let ordered_keys = entities
        .iter()
        .map(|entity| entity.entity_key.clone())
        .collect::<Vec<_>>();
    let range = timeline_range(view, ordered_keys);
    let persisted_watermark = range.latest_watermark.clone();
    Ok(TimelineSnapshotV3Vm {
        channel_id: view.channel_id.clone(),
        items: view.items.clone(),
        anchor: view.anchor.clone(),
        first_unread_message_id: view.first_unread_message_id.clone(),
        target_message_id: view.target_message_id.clone(),
        page: view.page.clone(),
        state: view.state.clone(),
        entities,
        ranges: vec![range],
        active_window: active_window(view),
        persisted_watermark,
    })
}

/// 比较两个 Helix 权威窗口，生成 closed、有界且有序的 Timeline Delta。
fn delta_between(
    previous: &TimelineWindowVm,
    next: &TimelineWindowVm,
    navigation_token: Option<String>,
) -> Result<Vec<TimelineDeltaOperationVm>, TimelineProjectionError> {
    let previous_entities = timeline_entities(previous)?;
    let next_entities = timeline_entities(next)?;
    let previous_by_key = previous_entities
        .iter()
        .map(|entity| (entity.entity_key.clone(), entity))
        .collect::<BTreeMap<_, _>>();
    let previous_keys = previous_entities
        .iter()
        .map(|entity| entity.entity_key.clone())
        .collect::<Vec<_>>();
    let next_keys = next_entities
        .iter()
        .map(|entity| entity.entity_key.clone())
        .collect::<Vec<_>>();
    let mut operations = next_entities
        .iter()
        .filter(|entity| previous_by_key.get(&entity.entity_key).copied() != Some(*entity))
        .cloned()
        .map(|entity| TimelineDeltaOperationVm::UpsertMessage { entity })
        .collect::<Vec<_>>();
    let range_id = next.range_id.clone();
    let range_changed = previous.range_id != next.range_id;
    if range_changed {
        operations.push(TimelineDeltaOperationVm::InstallSegment {
            range: timeline_range(next, next_keys.clone()),
            entities: next_entities.clone(),
        });
    } else if previous_keys != next_keys {
        if next_keys.starts_with(&previous_keys) {
            let keys = next_keys[previous_keys.len()..].to_vec();
            if !keys.is_empty() {
                operations.push(TimelineDeltaOperationVm::InsertKeys {
                    range_id: range_id.clone(),
                    position: TimelineInsertPositionVm::End,
                    keys,
                });
            }
        } else if next_keys.ends_with(&previous_keys) {
            let prefix_len = next_keys.len().saturating_sub(previous_keys.len());
            let keys = next_keys[..prefix_len].to_vec();
            if !keys.is_empty() {
                operations.push(TimelineDeltaOperationVm::InsertKeys {
                    range_id: range_id.clone(),
                    position: TimelineInsertPositionVm::Start,
                    keys,
                });
            }
        } else if previous_keys.len() == next_keys.len()
            && previous_keys.len() > 1
            && previous_keys[1..] == next_keys[..next_keys.len() - 1]
        {
            operations.push(TimelineDeltaOperationVm::InsertKeys {
                range_id: range_id.clone(),
                position: TimelineInsertPositionVm::End,
                keys: vec![next_keys[next_keys.len() - 1].clone()],
            });
        } else {
            operations.push(TimelineDeltaOperationVm::InstallSegment {
                range: timeline_range(next, next_keys.clone()),
                entities: next_entities.clone(),
            });
        }
    }
    let previous_range = timeline_range(previous, previous_keys);
    let next_range = timeline_range(next, next_keys.clone());
    if !range_changed
        && (previous.page != next.page
            || previous_range.older_cursor != next_range.older_cursor
            || previous_range.newer_cursor != next_range.newer_cursor
            || previous_range.latest_watermark != next_range.latest_watermark)
    {
        operations.push(TimelineDeltaOperationVm::SetRangeBounds {
            range_id: next_range.range_id,
            older_cursor: next_range.older_cursor,
            newer_cursor: next_range.newer_cursor,
            has_older: next_range.has_older,
            has_newer: next_range.has_newer,
            latest_watermark: next_range.latest_watermark,
        });
    }
    if previous.anchor != next.anchor
        || previous.first_unread_message_id != next.first_unread_message_id
        || previous.target_message_id != next.target_message_id
        || previous.state != next.state
        || navigation_token.is_some()
    {
        operations.push(TimelineDeltaOperationVm::ActivateWindow {
            window: active_window(next),
            navigation_token,
        });
    }
    if operations.is_empty() {
        return Err(TimelineProjectionError::EmptyDelta);
    }
    Ok(operations)
}

/// 为窗口消息建立稳定 EntityKey，并保持 Helix 已裁决的顺序。
fn timeline_entities(
    view: &TimelineWindowVm,
) -> Result<Vec<TimelineEntityVm>, TimelineProjectionError> {
    view.items
        .iter()
        .map(|message| {
            if message.temporary_id.is_empty() {
                return Err(TimelineProjectionError::MissingRowField {
                    field: "temporaryId",
                });
            }
            Ok(TimelineEntityVm {
                entity_key: TimelineEntityKeyVm {
                    create_at: message.created_at,
                    temporary_id: message.temporary_id.clone(),
                },
                message: message.clone(),
            })
        })
        .collect()
}

/// 从 Helix scope 与导航 token 派生不可由 renderer 直接指定的稳定 RangeId。
fn located_range_id(scope_key: &str, navigation_token: &str) -> String {
    let mut first = 0xcbf2_9ce4_8422_2325_u64;
    let mut second = 0x8422_2325_cbf2_9ce4_u64;
    for byte in scope_key
        .bytes()
        .chain(std::iter::once(0xff))
        .chain(navigation_token.bytes())
    {
        first = (first ^ u64::from(byte)).wrapping_mul(0x1000_0000_01b3);
        second = (second ^ u64::from(byte).rotate_left(1)).wrapping_mul(0x1000_0000_01b3);
    }
    format!("range.located.{first:016x}{second:016x}")
}

/// 从当前窗口构造单 Range 的双向边界与最新水位。
fn timeline_range(
    view: &TimelineWindowVm,
    ordered_keys: Vec<TimelineEntityKeyVm>,
) -> TimelineRangeVm {
    TimelineRangeVm {
        range_id: view.range_id.clone(),
        older_cursor: ordered_keys.first().cloned(),
        newer_cursor: ordered_keys.last().cloned(),
        latest_watermark: ordered_keys.last().cloned(),
        ordered_keys,
        has_older: view.page.has_older,
        has_newer: view.page.has_newer,
    }
}

/// 把内部窗口元数据收紧为不复制消息数组的活动窗口引用。
fn active_window(view: &TimelineWindowVm) -> TimelineActiveWindowVm {
    TimelineActiveWindowVm {
        window_token: view.page.window_token.clone(),
        range_ids: vec![view.range_id.clone()],
        anchor: view.anchor.clone(),
        first_unread_message_id: view.first_unread_message_id.clone(),
        target_message_id: view.target_message_id.clone(),
        state: view.state.clone(),
    }
}

/// 合并既有已读快照，避免分页回包清空更晚到达的 receipt fact。
fn preserve_receipt_snapshot(previous: &TimelineWindowVm, next: &mut TimelineWindowVm) {
    let receipts: BTreeMap<&str, &BTreeMap<String, SurfaceDataVm>> = previous
        .items
        .iter()
        .filter_map(|item| match &item.read_receipt {
            SurfaceDataVm::Record(receipt) => Some((item.id.as_str(), receipt)),
            _ => None,
        })
        .collect();

    for item in &mut next.items {
        let Some(previous_receipt) = receipts.get(item.id.as_str()) else {
            continue;
        };
        let SurfaceDataVm::Record(current_receipt) = &item.read_receipt else {
            continue;
        };
        let mut merged = (*previous_receipt).clone();
        merged.extend(current_receipt.clone());
        let Some((reader_count, unread_count)) = merged.get("readBits").and_then(|value| {
            let SurfaceDataVm::String(read_bits) = value else {
                return None;
            };
            read_bits
                .bytes()
                .all(|bit| bit == b'0' || bit == b'1')
                .then(|| {
                    let reader_count = read_bits.bytes().filter(|bit| *bit == b'1').count();
                    (reader_count, read_bits.len() - reader_count)
                })
        }) else {
            item.read_receipt = SurfaceDataVm::Record(merged);
            continue;
        };
        merged.insert(
            "readerCount".to_string(),
            SurfaceDataVm::Number(serde_json::Number::from(reader_count as u64)),
        );
        merged.insert(
            "unreadCount".to_string(),
            SurfaceDataVm::Number(serde_json::Number::from(unread_count as u64)),
        );
        item.read_receipt = SurfaceDataVm::Record(merged);
    }
}

/// 保留语义未变行的 opaque actionRef，避免分页导致既有实体产生无意义 upsert。
fn preserve_stable_action_bindings(previous: &TimelineWindowVm, next: &mut TimelineWindowVm) {
    let previous_actions = previous
        .items
        .iter()
        .map(|message| {
            (
                (message.created_at, message.temporary_id.as_str()),
                &message.actions,
            )
        })
        .collect::<BTreeMap<_, _>>();
    for message in &mut next.items {
        let key = (message.created_at, message.temporary_id.as_str());
        let Some(actions) = previous_actions.get(&key) else {
            continue;
        };
        if action_semantics_match(actions, &message.actions) {
            message.actions.clone_from(actions);
        }
    }
}

/// 比较 action 能力而忽略可复用的 opaque ref 身份。
fn action_semantics_match(previous: &[ActionBinding], next: &[ActionBinding]) -> bool {
    previous.len() == next.len()
        && previous.iter().zip(next).all(|(left, right)| {
            left.kind == right.kind
                && left.input_shape == right.input_shape
                && left.surface_epoch == right.surface_epoch
                && left.enabled == right.enabled
                && left.disabled_reason == right.disabled_reason
        })
}

fn enrich_reply_threads(items: &mut [MessageVm]) {
    let mut children: BTreeMap<String, Vec<ReplyThreadPreviewVm>> = BTreeMap::new();
    for item in items.iter() {
        let root_id = item.reply_root_id.as_deref().or(item.reply_id.as_deref());
        let Some(root_id) = root_id.filter(|id| !id.is_empty() && *id != item.id) else {
            continue;
        };
        let text = match &item.content {
            MessageContentVm::Text { text }
            | MessageContentVm::SystemEvent { text, .. }
            | MessageContentVm::Recalled { text } => text.clone(),
            MessageContentVm::Document { title, .. } => title.clone(),
            MessageContentVm::Media { file_name, .. } => file_name.clone(),
            MessageContentVm::Forwarded { summary, .. } => summary.clone(),
        };
        children
            .entry(root_id.to_string())
            .or_default()
            .push(ReplyThreadPreviewVm {
                id: item.id.clone(),
                author: item.author.clone(),
                text,
            });
    }
    for item in items.iter_mut() {
        let Some(replies) = children.get(&item.id) else {
            continue;
        };
        let projected_count = replies.len().min(u32::MAX as usize) as u32;
        let existing_count = item
            .reply_thread
            .as_ref()
            .map_or(0, |thread| thread.reply_count);
        item.reply_thread = Some(ReplyThreadSummaryVm {
            items: replies
                .iter()
                .rev()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect(),
            reply_count: existing_count.max(projected_count),
        });
    }
}

/// Removes topic mirror replies after their previews have been folded into the topic root.
fn collapse_topic_reply_children(items: &mut Vec<MessageVm>) {
    let topic_root_ids: BTreeSet<String> = items
        .iter()
        .filter(|item| item.topic_link.is_some())
        .map(|item| item.id.clone())
        .collect();
    if topic_root_ids.is_empty() {
        return;
    }
    items.retain(|item| {
        let root_id = item.reply_root_id.as_deref().or(item.reply_id.as_deref());
        !root_id.is_some_and(|root_id| topic_root_ids.contains(root_id))
    });
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimelineProjectionError {
    #[error("timeline channelId must not be empty")]
    EmptyChannelId,
    #[error("timeline windowToken must not be empty")]
    EmptyWindowToken,
    #[error("timeline row is missing required {field}")]
    MissingRowField { field: &'static str },
    #[error("timeline row channelId {actual} does not match scope channelId {expected}")]
    ChannelMismatch { expected: String, actual: String },
    #[error("timeline patch requires a snapshot for slot {slot}")]
    MissingSnapshot { slot: String },
    #[error("timeline surface data exceeds the v1 bounded contract")]
    SurfaceDataLimit,
    #[error("timeline revision reached the exact-safe wire limit")]
    RevisionOverflow,
    #[error("timeline actionRef allocator exhausted its unique sequence")]
    ActionRefOverflow,
    #[error("timeline locate target {target} is absent from the bounded window")]
    TargetNotInWindow { target: String },
    #[error(transparent)]
    InvalidPageSize(#[from] TimelinePagePolicyError),
    #[error("timeline projector attempted to advance revision without a semantic delta")]
    EmptyDelta,
    #[error(transparent)]
    InvalidInteractionFrame(#[from] InteractionFrameError),
}

fn scope_and_slot_key(
    request: &TimelineWindowRequest,
) -> Result<(SurfaceScope, String), TimelineProjectionError> {
    if request.channel_id.is_empty() {
        return Err(TimelineProjectionError::EmptyChannelId);
    }
    if request.window_token.is_empty() {
        return Err(TimelineProjectionError::EmptyWindowToken);
    }
    let scope = SurfaceScope::Timeline {
        channel_id: request.channel_id.clone(),
        window_token: request.window_token.clone(),
    };
    Ok((scope.clone(), scope.canonical_key()))
}

fn bounded_rows<'a>(
    rows: &'a [Value],
    anchor: &TimelineAnchorVm,
    requested_page: &WindowPageVm,
    page_size: usize,
) -> (&'a [Value], WindowPageVm) {
    if rows.len() <= page_size {
        return (rows, requested_page.clone());
    }

    let start = match anchor.mode {
        TimelineAnchorMode::Latest => rows.len() - page_size,
        TimelineAnchorMode::Locate | TimelineAnchorMode::Unread => anchor
            .message_id
            .as_deref()
            .and_then(|id| row_id_index(rows, id))
            .map(|index| {
                index
                    .saturating_sub(page_size / 2)
                    .min(rows.len() - page_size)
            })
            .unwrap_or(rows.len() - page_size),
    };
    let end = start + page_size;
    let mut page = requested_page.clone();
    page.has_older |= start > 0;
    page.has_newer |= end < rows.len();
    page.has_more |= page.has_older || page.has_newer;
    (&rows[start..end], page)
}

fn row_id_index(rows: &[Value], target: &str) -> Option<usize> {
    rows.iter()
        .position(|row| row_string(row, &["msgId", "id"]) == target)
}

/// Converts an already render-ready message fact into the shared closed `MessageVm`.
///
/// Non-timeline surfaces reuse this only with `action_authority_available=false`, so every
/// capability without an audited authority dispatch remains explicitly disabled rather than
/// becoming a renderer-selected command.
pub(crate) fn message_from_render_ready(
    row: &Value,
    expected_channel_id: &str,
    surface_epoch: u64,
    action_authority_available: bool,
    mut issue_action_ref: impl FnMut() -> Result<String, TimelineProjectionError>,
) -> Result<MessageVm, TimelineProjectionError> {
    let id = required_string(row, &["msgId", "id"], "id")?;
    // render_ready 生产路径始终提供 temporaryId；末尾别名仅兼容本模块历史单元样本。
    let temporary_id = required_string(
        row,
        &["temporaryId", "temporary_id", "id", "msgId"],
        "temporaryId",
    )?;
    let server_id = optional_non_empty_string(row, &["serverId", "server_id"]);
    let channel_id = required_string(row, &["channelId", "channel_id"], "channelId")?;
    if channel_id != expected_channel_id {
        return Err(TimelineProjectionError::ChannelMismatch {
            expected: expected_channel_id.to_string(),
            actual: channel_id,
        });
    }
    let author_id = author_id_from_row(row)?;
    let revoked = bool_at(row, &["revoked", "revoke"]);
    let props = row.get("props").unwrap_or(&Value::Null);
    let content = content_from_row(row, props, revoked)?;
    let delivery = delivery_from_row(row, revoked)?;
    let (actions, quick_actions, reply_summary, reply_thread, reactions, topic_link) = if revoked {
        (
            Vec::new(),
            Vec::new(),
            ReplySummaryVm::None,
            None,
            Vec::new(),
            None,
        )
    } else {
        (
            action_bindings(
                row,
                surface_epoch,
                action_authority_available,
                &mut issue_action_ref,
            )?,
            quick_actions_from_row(row),
            reply_summary_from_row(row),
            reply_thread_from_row(row),
            reactions_from_props(props),
            topic_link_from_row(row, expected_channel_id, &id)?,
        )
    };
    Ok(MessageVm {
        id,
        temporary_id,
        server_id,
        channel_id,
        event_seq: unsigned_integer_at(row, &["eventSeq", "event_seq"], "eventSeq")?
            .unwrap_or_default(),
        author: UserRefVm { user_id: author_id },
        created_at: integer_at(row, &["createdAt", "createAt", "create_at"], "createdAt")?
            .unwrap_or_default(),
        content,
        delivery,
        actions,
        quick_actions,
        reply_id: optional_non_empty_string(row, &["replyId", "reply_id"]),
        reply_root_id: optional_non_empty_string(row, &["replyRootId", "reply_root_id"]),
        reply_first_level_id: optional_non_empty_string(
            row,
            &["replyFirstLevelId", "reply_first_level_id"],
        ),
        reply_summary,
        reply_thread,
        reactions,
        read_receipt: read_receipt_from_row(row, props)?,
        template_confirmation: optional_data(
            row.get("templateConfirmation")
                .or_else(|| props.get("template")),
        )?,
        urgency: urgency_data(row)?,
        pinned: bool_at(row, &["pinned", "isTop"]),
        bookmarked: bool_at(row, &["bookmarked"]),
        topic_link,
    })
}

fn quick_actions_from_row(row: &Value) -> Vec<MessageQuickActionVm> {
    let Some(capabilities) = row.get("actionCapabilities").and_then(Value::as_object) else {
        return Vec::new();
    };
    [
        ("canReact", MessageQuickActionVm::React),
        ("canReply", MessageQuickActionVm::QuoteReply),
        ("canCreateTopic", MessageQuickActionVm::CreateTopic),
    ]
    .into_iter()
    .filter_map(|(key, action)| {
        capabilities
            .get(key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
            .then_some(action)
    })
    .collect()
}

/// 将 root durable topic JSON 收紧为 strict TopicLink，历史行补 authority-safe activity 默认值。
fn topic_link_from_row(
    row: &Value,
    expected_channel_id: &str,
    message_id: &str,
) -> Result<Option<SurfaceDataVm>, TimelineProjectionError> {
    let Some(topic) = row.get("topic").and_then(Value::as_object) else {
        return Ok(None);
    };
    let Some(topic_channel_id) = topic
        .get("topicChannelId")
        .or_else(|| topic.get("channelId"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let Some(title) = topic
        .get("title")
        .or_else(|| topic.get("displayName"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        // A bare Go `topic.channelId` is a durable relation, but not a closed
        // TopicLink VM. Keep the message renderable until Helix can join the
        // authoritative topic dialog metadata.
        return Ok(None);
    };
    let root_message_id = topic
        .get("rootMessageId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(message_id);
    let parent_channel_id = topic
        .get("parentChannelId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or(expected_channel_id);
    let reply_count = topic
        .get("replyCount")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let last_activity_at = topic
        .get("lastActivityAt")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| {
            row.get("createAt")
                .or_else(|| row.get("create_at"))
                .and_then(Value::as_i64)
                .unwrap_or_default()
        });
    let latest_reply = topic.get("latestReply").cloned();
    let mut topic_link = serde_json::json!({
        "parentChannelId": parent_channel_id,
        "topicChannelId": topic_channel_id,
        "rootMessageId": root_message_id,
        "title": title,
        "replyCount": reply_count,
        "lastActivityAt": last_activity_at,
    });
    if let Some(latest_reply) = latest_reply {
        topic_link["latestReply"] = latest_reply;
    }
    surface_data(&topic_link, 0).map(Some)
}

/// 将持久行里的当前/历史引用字段统一成封闭的 V2 reply summary。
fn reply_summary_from_row(row: &Value) -> ReplySummaryVm {
    let replied = row
        .get("repliedMessage")
        .or_else(|| row.get("replied_message"))
        .filter(|value| value.is_object());
    let target_message_id =
        optional_non_empty_string(row, &["replyId", "reply_id"]).or_else(|| {
            replied.and_then(|message| optional_non_empty_string(message, &["msgId", "id"]))
        });
    let Some(target_message_id) = target_message_id else {
        return ReplySummaryVm::None;
    };
    let Some(replied) = replied else {
        return ReplySummaryVm::Unavailable {
            target_message_id,
            availability: ReferenceAvailabilityVm::Unavailable {
                reason: "referencedMessageUnavailable".to_string(),
            },
        };
    };
    if bool_at(replied, &["isRevoke", "is_revoke", "revoked", "revoke"]) {
        return ReplySummaryVm::Unavailable {
            target_message_id,
            availability: ReferenceAvailabilityVm::Recalled {
                text: "消息已撤回".to_string(),
            },
        };
    }
    let author_id = row_string(
        replied,
        &["repliedUserId", "replied_user_id", "userId", "user_id"],
    );
    let Some(preview) = reply_preview_from_row(replied) else {
        return ReplySummaryVm::Unavailable {
            target_message_id,
            availability: ReferenceAvailabilityVm::Unavailable {
                reason: "referencedMessageUnavailable".to_string(),
            },
        };
    };
    if author_id.is_empty() {
        return ReplySummaryVm::Unavailable {
            target_message_id,
            availability: ReferenceAvailabilityVm::Unavailable {
                reason: "referencedAuthorUnavailable".to_string(),
            },
        };
    }
    ReplySummaryVm::Available {
        target_message_id,
        author: UserRefVm { user_id: author_id },
        preview,
        availability: ReferenceAvailabilityVm::Available,
    }
}

fn reply_preview_from_row(row: &Value) -> Option<ReplyPreviewVm> {
    let text = row_string(row, &["text", "message"]);
    let kind = row_string(row, &["type"]).to_ascii_uppercase();
    if kind.contains("FORWARD") {
        return (!text.is_empty()).then_some(ReplyPreviewVm::Forwarded { summary: text });
    }
    if kind.contains("DOCUMENT") {
        let title = row_string(row, &["fileName", "title", "text", "message"]);
        return (!title.is_empty()).then_some(ReplyPreviewVm::Document { title });
    }
    if matches!(
        kind.as_str(),
        "IMAGE" | "VIDEO" | "AUDIO" | "FILE" | "MEDIA"
    ) {
        let file_name = row_string(row, &["fileName", "title", "text", "message"]);
        return (!kind.is_empty() && !file_name.is_empty()).then_some(ReplyPreviewVm::Media {
            media_type: kind.to_ascii_lowercase(),
            file_name,
        });
    }
    if matches!(
        kind.as_str(),
        "NOTICE"
            | "SYSTEM"
            | "SYSTEN"
            | "MEETING_NOTICE"
            | "TODO_NOTICE"
            | "APPROVAL_NOTICE"
            | "ANNOUNCEMENT"
    ) {
        return (!text.is_empty()).then_some(ReplyPreviewVm::System { text });
    }
    (!text.is_empty()).then_some(ReplyPreviewVm::Text { text })
}

fn reply_thread_from_row(row: &Value) -> Option<ReplyThreadSummaryVm> {
    let reply_count = row
        .get("replyCount")
        .or_else(|| row.get("reply_count"))
        .and_then(Value::as_u64)
        .unwrap_or_default()
        .min(u32::MAX as u64) as u32;
    let source = row
        .get("replyMessages")
        .or_else(|| row.get("reply_messages"));
    let mut rows: Vec<&Value> = match source {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    rows.sort_by_key(|item| {
        (
            item.get("createAt")
                .or_else(|| item.get("createdAt"))
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            row_string(item, &["msgId", "id"]),
        )
    });
    let items: Vec<ReplyThreadPreviewVm> = rows
        .into_iter()
        .filter_map(|item| {
            let id = row_string(item, &["msgId", "id"]);
            let user_id = row_string(item, &["userId", "user_id"]);
            if id.is_empty() || user_id.is_empty() {
                return None;
            }
            Some(ReplyThreadPreviewVm {
                id,
                author: UserRefVm { user_id },
                text: row_string(item, &["text", "message"]),
            })
        })
        .take(3)
        .collect();
    let reply_count = reply_count.max(items.len() as u32);
    (reply_count > 0).then_some(ReplyThreadSummaryVm { items, reply_count })
}

fn reactions_from_props(props: &Value) -> Vec<MessageReactionVm> {
    let mut reactions = Vec::new();
    match props.get("quickReply") {
        Some(Value::Array(items)) => {
            for item in items {
                let emoji = row_string(item, &["emoji"]);
                let count = item
                    .get("userIds")
                    .or_else(|| item.get("user_ids"))
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or_default();
                if !emoji.is_empty() && count > 0 {
                    reactions.push(MessageReactionVm {
                        emoji,
                        count: count.min(u32::MAX as usize) as u32,
                    });
                }
            }
        }
        Some(Value::Object(items)) => {
            for (emoji, users) in items {
                let count = users.as_array().map(Vec::len).unwrap_or_default();
                if !emoji.is_empty() && count > 0 {
                    reactions.push(MessageReactionVm {
                        emoji: emoji.clone(),
                        count: count.min(u32::MAX as usize) as u32,
                    });
                }
            }
        }
        _ => {}
    }
    reactions
}

fn content_from_row(
    row: &Value,
    props: &Value,
    revoked: bool,
) -> Result<MessageContentVm, TimelineProjectionError> {
    let text = row_string(row, &["text", "message"]);
    if revoked {
        return Ok(MessageContentVm::Recalled { text });
    }
    let kind = row_string(row, &["type"]).to_ascii_uppercase();
    if kind.contains("FORWARD")
        || row
            .get("forwardDetail")
            .is_some_and(|value| !value.is_null())
    {
        return forwarded_content_from_row(row.get("forwardDetail").unwrap_or(&Value::Null), text);
    }
    if kind.contains("DOCUMENT") {
        let document = props.get("document").unwrap_or(&Value::Null);
        return Ok(MessageContentVm::Document {
            document_id: optional_non_empty_string(document, &["id", "documentId"])
                .unwrap_or_else(|| row_string(row, &["documentId", "id", "msgId"])),
            title: optional_non_empty_string(document, &["title"])
                .unwrap_or_else(|| row_string(row, &["fileName", "title", "text"])),
            summary: optional_non_empty_string(document, &["url", "summary"])
                .unwrap_or_else(|| text.clone()),
            access: surface_data(document, 0)?,
        });
    }
    if matches!(
        kind.as_str(),
        "IMAGE" | "VIDEO" | "AUDIO" | "FILE" | "MEDIA"
    ) {
        let (resource, primary_resource) = media_resource_from_props(kind.as_str(), props);
        let items = media_items_from_props(kind.as_str(), props)?;
        let file_name = {
            let nested = row_string(primary_resource, &["name", "fileName", "title"]);
            if nested.is_empty() {
                row_string(row, &["fileName", "title", "text"])
            } else {
                nested
            }
        };
        let size = integer_at(primary_resource, &["size"], "size")?
            .or(integer_at(row, &["size"], "size")?)
            .unwrap_or_default()
            .max(0) as u64;
        return Ok(MessageContentVm::Media {
            media_type: kind.to_ascii_lowercase(),
            resource: surface_data(resource, 0)?,
            thumbnail: optional_data(
                primary_resource
                    .get("thumbnail")
                    .or_else(|| row.get("thumbnail")),
            )?,
            file_name,
            size,
            availability: SurfaceDataVm::Record(BTreeMap::new()),
            items,
            caption: (!text.is_empty()).then_some(text),
            image_position: (kind == "IMAGE")
                .then(|| row_string(props, &["imagePosition", "image_position"]))
                .filter(|position| !position.is_empty()),
        });
    }
    if matches!(
        kind.as_str(),
        "NOTICE"
            | "SYSTEM"
            | "SYSTEN"
            | "MEETING_NOTICE"
            | "TODO_NOTICE"
            | "APPROVAL_NOTICE"
            | "ANNOUNCEMENT"
    ) {
        let event_type = row_string(props, &["eventType", "event_type", "type"]);
        let event_kind = match event_type.to_ascii_lowercase().as_str() {
            "leave" => "member-leave".to_string(),
            "join" => "member-join".to_string(),
            value if !value.is_empty() => value.to_string(),
            _ => kind.to_ascii_lowercase(),
        };
        let member = props
            .get("users")
            .or_else(|| props.get("members"))
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .unwrap_or(&Value::Null);
        let member_id = optional_non_empty_string(member, &["id", "userId", "user_id"]);
        let member_name = row_string(member, &["name", "displayName", "display_name"]);
        let text = if text.is_empty() {
            let subject = if member_name.is_empty() {
                member_id.as_deref().unwrap_or("成员")
            } else {
                member_name.as_str()
            };
            match event_kind.as_str() {
                "member-leave" => format!("{subject}退出群聊"),
                "member-join" => format!("{subject}加入群聊"),
                _ => String::new(),
            }
        } else {
            text
        };
        return Ok(MessageContentVm::SystemEvent {
            event_kind,
            member_id,
            text,
        });
    }
    Ok(MessageContentVm::Text { text })
}

/// 将 render-ready forwardDetail 收紧成最多二十条 typed 记录，卡片前四条由 renderer 只读截取。
fn forwarded_content_from_row(
    detail: &Value,
    fallback_summary: String,
) -> Result<MessageContentVm, TimelineProjectionError> {
    let mode = match row_string(detail, &["mode"]).as_str() {
        "merged" => ForwardModeVm::Merged,
        _ => ForwardModeVm::Single,
    };
    let title =
        optional_non_empty_string(detail, &["title"]).unwrap_or_else(|| "聊天记录".to_string());
    let summary = optional_non_empty_string(detail, &["summary"]).unwrap_or(fallback_summary);
    let source = match (
        optional_non_empty_string(detail, &["sourceChannelId"]),
        optional_non_empty_string(detail, &["sourceChannelName"]),
    ) {
        (Some(channel_id), Some(channel_title)) => Some(ForwardSourceVm {
            channel_id,
            channel_title,
        }),
        _ => None,
    };
    let items = detail
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(20)
        .filter_map(forwarded_item_from_value)
        .collect();
    Ok(MessageContentVm::Forwarded {
        mode,
        title,
        summary,
        source,
        items,
    })
}

/// 单条合并记录缺少稳定 id 或 author authority 时不进入 typed card，避免伪造身份。
fn forwarded_item_from_value(item: &Value) -> Option<ForwardedItemVm> {
    let id = optional_non_empty_string(item, &["id", "temporaryId", "temporary_id"])?;
    let snapshot = item
        .get("userSnapshot")
        .or_else(|| item.get("user_snapshot"))
        .unwrap_or(&Value::Null);
    let user_id = optional_non_empty_string(item, &["userId", "user_id"])
        .or_else(|| optional_non_empty_string(snapshot, &["userId", "user_id"]))?;
    let created_at = integer_at(item, &["createAt", "create_at"], "forward createdAt")
        .ok()
        .flatten()
        .unwrap_or_default();
    let text = row_string(
        item,
        &["text", "message", "simpleMessage", "simple_message"],
    );
    let msg_type = row_string(item, &["type"]).to_ascii_uppercase();
    let content = match msg_type.as_str() {
        "DOCUMENT" => ForwardedItemContentVm::Document { title: text },
        "IMAGE" | "VIDEO" | "AUDIO" | "FILE" | "MEDIA" => ForwardedItemContentVm::Media {
            media_type: msg_type.to_ascii_lowercase(),
            file_name: text,
        },
        "NOTICE" | "SYSTEM" | "SYSTEN" => ForwardedItemContentVm::SystemEvent { text },
        _ => ForwardedItemContentVm::Text { text },
    };
    Some(ForwardedItemVm {
        id,
        author: UserRefVm { user_id },
        created_at,
        content,
        availability: ReferenceAvailabilityVm::Available,
    })
}

/// Selects the stable media resource shape used by IMAGE and single-file rows.
fn media_resource_from_props<'a>(kind: &str, props: &'a Value) -> (&'a Value, &'a Value) {
    if kind == "IMAGE" {
        let resources = props.get("files").unwrap_or(props);
        let primary = resources
            .as_array()
            .and_then(|items| items.first())
            .unwrap_or(resources);
        (resources, primary)
    } else {
        let resource = props.get("file").unwrap_or(props);
        (resource, resource)
    }
}

/// 从兼容 props 提取字段完整的稳定 typed items，临时节点只保留在父级 raw resource。
fn media_items_from_props(
    kind: &str,
    props: &Value,
) -> Result<Vec<MediaItemVm>, TimelineProjectionError> {
    let resources = if kind == "IMAGE" {
        props
            .get("files")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
    } else {
        std::slice::from_ref(props.get("file").unwrap_or(props))
    };

    resources
        .iter()
        .map(|resource| {
            let file_name = row_string(resource, &["name", "fileName", "title"]);
            let file_id = optional_non_empty_string(resource, &["id", "fileId", "file_id"])
                .or_else(|| {
                    optional_non_empty_string(resource, &["stablePath", "url", "uri"]).and_then(
                        |value| {
                            value
                                .rsplit('/')
                                .find(|part| !part.is_empty())
                                .map(str::to_string)
                        },
                    )
                });
            let content_type = row_string(
                resource,
                &["contentType", "content_type", "mimeType", "mime_type"],
            );
            let uri = row_string(resource, &["uri", "url", "stablePath"]);
            let Some(file_id) = file_id else {
                return Ok(None);
            };
            if file_name.is_empty() || content_type.is_empty() || uri.is_empty() {
                return Ok(None);
            }
            let size = integer_at(resource, &["size"], "size")?
                .unwrap_or_default()
                .max(0) as u64;
            let bucket = optional_non_empty_string(resource, &["bucket"]).unwrap_or_else(|| {
                if kind == "IMAGE" {
                    "picture"
                } else {
                    "attachment"
                }
                .to_string()
            });
            Ok(Some(MediaItemVm {
                file_id,
                file_name,
                size,
                content_type,
                bucket,
                uri,
                thumbnail: optional_data(resource.get("thumbnail"))?,
            }))
        })
        .filter_map(Result::transpose)
        .collect()
}

/// 将持久行状态投影成 renderer delivery，并复用同一 capability 真源判定 retryable。
fn delivery_from_row(row: &Value, revoked: bool) -> Result<DeliveryVm, TimelineProjectionError> {
    if revoked {
        return Ok(DeliveryVm::Recalled);
    }
    let send_status = row_string(row, &["sendStatus", "send_status"]);
    if matches!(send_status.as_str(), "failed" | "unsend") {
        return Ok(DeliveryVm::Failed {
            reason: row_string(row, &["failureReason", "error"]),
            retryable: retryable_from_row(row),
        });
    }
    let is_file_uploading = row_string(row, &["type"]) == "FILE"
        && row
            .get("props")
            .and_then(|props| props.get("file"))
            .and_then(|file| file.get("upload"))
            .and_then(|upload| upload.get("status"))
            .and_then(Value::as_str)
            .is_some_and(|status| matches!(status, "preparing" | "uploading" | "completing"));
    if is_file_uploading {
        return Ok(DeliveryVm::Uploading {
            progress_percent: integer_at(
                row,
                &["progressPercent", "upload_progress_percent"],
                "progressPercent",
            )?
            .unwrap_or_default()
            .clamp(0, 100) as u8,
        });
    }
    match send_status.as_str() {
        "sending" => Ok(DeliveryVm::Sending),
        _ => Ok(DeliveryVm::Sent),
    }
}

/// 兼容 direct row 与 render-ready `actionCapabilities` 两种 canRetry 位置。
fn retryable_from_row(row: &Value) -> bool {
    bool_at(row, &["canRetry"])
        || row
            .get("actionCapabilities")
            .is_some_and(|capabilities| bool_at(capabilities, &["canRetry"]))
}

fn action_bindings(
    row: &Value,
    surface_epoch: u64,
    action_authority_available: bool,
    issue_action_ref: &mut impl FnMut() -> Result<String, TimelineProjectionError>,
) -> Result<Vec<ActionBinding>, TimelineProjectionError> {
    let Some(capabilities) = row.get("actionCapabilities").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut actions = Vec::new();
    for (capability, kind) in [
        ("canReply", "reply"),
        ("canForward", "forward"),
        ("canReact", "react"),
        ("canUrgent", "urgent"),
        ("canCreateTopic", "createTopic"),
        ("canRevoke", "revoke"),
        ("canRetry", "retry"),
        ("canUrgentConfirm", "urgentConfirm"),
        ("canUrgentCancel", "urgentCancel"),
        ("canMarkRead", "markRead"),
        ("canViewReceipt", "viewReceipt"),
        ("canTemplateConfirm", "templateConfirm"),
        ("canLocate", "locate"),
    ] {
        let capability_enabled = capabilities
            .get(capability)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let enabled = crate::projector::action::ActionDispatch::supports_timeline_kind(kind)
            && capability_enabled
            && action_authority_available;
        let disabled_reason = if enabled {
            None
        } else if !capability_enabled {
            Some("notAvailable".to_string())
        } else if !action_authority_available {
            Some("runtimeIdentityRequired".to_string())
        } else {
            Some("unsupportedByAuthority".to_string())
        };
        actions.push(ActionBinding {
            action_ref: issue_action_ref()?,
            kind: kind.to_string(),
            input_shape: "object".to_string(),
            surface_epoch,
            enabled,
            disabled_reason,
        });
    }
    Ok(actions)
}

fn mix_action_ref(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn record_from_row(
    row: &Value,
    fields: &[(&str, &str)],
) -> Result<SurfaceDataVm, TimelineProjectionError> {
    let mut record = BTreeMap::new();
    for (output, input) in fields {
        if let Some(value) = row.get(*input) {
            record.insert((*output).to_string(), surface_data(value, 0)?);
        }
    }
    Ok(SurfaceDataVm::Record(record))
}

/// 隔离模板确认 identities 与普通已读回执，后者只接受独立 receipt authority。
fn read_receipt_from_row(
    row: &Value,
    props: &Value,
) -> Result<SurfaceDataVm, TimelineProjectionError> {
    if let Some(value) = row.get("readReceipt").filter(|value| !value.is_null()) {
        return surface_data(value, 0);
    }
    if row.get("templateConfirmation").is_none() && props.get("template").is_none() {
        return record_from_row(row, &[("readBits", "readBits"), ("readerIds", "readerIds")]);
    }
    let mut record = BTreeMap::new();
    if let Some(value) = row.get("readBits") {
        record.insert("readBits".to_string(), surface_data(value, 0)?);
    }
    record.insert("readerIds".to_string(), SurfaceDataVm::Array(Vec::new()));
    Ok(SurfaceDataVm::Record(record))
}

fn urgency_data(row: &Value) -> Result<Option<SurfaceDataVm>, TimelineProjectionError> {
    let mut record = BTreeMap::new();
    for key in [
        "urgent",
        "urgentTargetCount",
        "urgentConfirmedCount",
        "urgentRequesterId",
        "urgentTargetIds",
        "urgentConfirmedIds",
        "urgentRequiredCount",
        "urgentState",
        "canConfirmUrgent",
    ] {
        if let Some(value) = row.get(key) {
            record.insert(key.to_string(), surface_data(value, 0)?);
        }
    }
    if record.is_empty() {
        Ok(None)
    } else {
        Ok(Some(SurfaceDataVm::Record(record)))
    }
}

fn optional_data(value: Option<&Value>) -> Result<Option<SurfaceDataVm>, TimelineProjectionError> {
    match value.filter(|value| !value.is_null()) {
        Some(value) => Ok(Some(surface_data(value, 0)?)),
        None => Ok(None),
    }
}

fn surface_data(value: &Value, depth: usize) -> Result<SurfaceDataVm, TimelineProjectionError> {
    if depth > MAX_SURFACE_DATA_DEPTH {
        return Err(TimelineProjectionError::SurfaceDataLimit);
    }
    match value {
        Value::Null => Ok(SurfaceDataVm::Null(())),
        Value::Bool(value) => Ok(SurfaceDataVm::Bool(*value)),
        Value::Number(value) => Ok(SurfaceDataVm::Number(value.clone())),
        Value::String(value) => Ok(SurfaceDataVm::String(value.clone())),
        Value::Array(items) => {
            if items.len() > MAX_SURFACE_DATA_ITEMS {
                return Err(TimelineProjectionError::SurfaceDataLimit);
            }
            items
                .iter()
                .map(|item| surface_data(item, depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(SurfaceDataVm::Array)
        }
        Value::Object(values) => {
            if values.len() > MAX_SURFACE_DATA_ITEMS {
                return Err(TimelineProjectionError::SurfaceDataLimit);
            }
            let mut record = BTreeMap::new();
            for (key, nested) in values {
                record.insert(key.clone(), surface_data(nested, depth + 1)?);
            }
            Ok(SurfaceDataVm::Record(record))
        }
    }
}

fn required_string(
    row: &Value,
    keys: &[&str],
    field: &'static str,
) -> Result<String, TimelineProjectionError> {
    let value = row_string(row, keys);
    (!value.is_empty())
        .then_some(value)
        .ok_or(TimelineProjectionError::MissingRowField { field })
}

fn author_id_from_row(row: &Value) -> Result<String, TimelineProjectionError> {
    let direct = row_string(row, &["userId", "user_id"]);
    if !direct.is_empty() {
        return Ok(direct);
    }
    let snapshot = row
        .get("userSnapshot")
        .or_else(|| row.get("user_snapshot"))
        .map(|value| row_string(value, &["userId", "user_id"]))
        .unwrap_or_default();
    if !snapshot.is_empty() {
        return Ok(snapshot);
    }
    // Historical system/reconnect rows can predate userId while still carrying
    // the authoritative creator identity. They remain renderable as SYS rows;
    // a row with no identity at all still fails closed below.
    required_string(row, &["createBy", "create_by"], "author.userId")
}

/// 返回候选字段中的首个非空字符串，避免空主键遮蔽后续兼容别名。
fn row_string(row: &Value, keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|key| row.get(*key).and_then(Value::as_str))
        .find(|value| !value.is_empty())
        .unwrap_or_default()
        .to_string()
}

fn optional_non_empty_string(row: &Value, keys: &[&str]) -> Option<String> {
    let value = row_string(row, keys);
    (!value.is_empty()).then_some(value)
}

fn integer_at(
    row: &Value,
    keys: &[&str],
    field: &'static str,
) -> Result<Option<i64>, TimelineProjectionError> {
    for key in keys {
        let Some(Value::Number(number)) = row.get(*key) else {
            continue;
        };
        let Some(value) = number.as_i64() else {
            return Err(unsafe_wire_number(field));
        };
        if !(MIN_SAFE_JSON_INTEGER..=MAX_SAFE_JSON_INTEGER as i64).contains(&value) {
            return Err(unsafe_wire_number(field));
        }
        return Ok(Some(value));
    }
    Ok(None)
}

fn unsigned_integer_at(
    row: &Value,
    keys: &[&str],
    field: &'static str,
) -> Result<Option<u64>, TimelineProjectionError> {
    for key in keys {
        let Some(Value::Number(number)) = row.get(*key) else {
            continue;
        };
        let Some(value) = number.as_u64() else {
            return Err(unsafe_wire_number(field));
        };
        if value > MAX_SAFE_JSON_INTEGER {
            return Err(unsafe_wire_number(field));
        }
        return Ok(Some(value));
    }
    Ok(None)
}

fn unsafe_wire_number(field: &'static str) -> TimelineProjectionError {
    TimelineProjectionError::InvalidInteractionFrame(InteractionFrameError::UnsafeWireNumber {
        surface: "message.timeline",
        field,
    })
}

fn bool_at(row: &Value, keys: &[&str]) -> bool {
    keys.iter()
        .find_map(|key| row.get(*key))
        .and_then(|value| match value {
            Value::Bool(value) => Some(*value),
            Value::Number(value) => value.as_i64().map(|value| value != 0),
            _ => None,
        })
        .unwrap_or(false)
}

fn next_revision(current: u64) -> Result<u64, TimelineProjectionError> {
    current
        .checked_add(1)
        .filter(|revision| *revision <= MAX_SAFE_JSON_INTEGER)
        .ok_or(TimelineProjectionError::RevisionOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(index: usize) -> Value {
        serde_json::json!({
            "msgId": format!("message-{index}"),
            "channelId": "channel-1",
            "eventSeq": index,
            "userId": "user-1",
            "createAt": index,
            "text": format!("text-{index}"),
            "type": "TEXT",
            "sendStatus": "sent",
            "props": {},
            "actionCapabilities": {},
        })
    }

    #[test]
    fn historical_row_uses_snapshot_then_creator_as_author_identity() {
        let from_snapshot = serde_json::json!({
            "msgId": "snapshot-author",
            "channelId": "channel-1",
            "userSnapshot": { "userId": "user-from-snapshot" },
            "text": "legacy",
            "type": "TEXT",
            "props": {}
        });
        let from_creator = serde_json::json!({
            "msgId": "system-author",
            "channelId": "channel-1",
            "createBy": "SYS",
            "userSnapshot": { "userId": "" },
            "text": "legacy",
            "type": "TEXT",
            "props": {}
        });

        let snapshot_vm =
            message_from_render_ready(&from_snapshot, "channel-1", 1, false, || unreachable!())
                .unwrap();
        let creator_vm =
            message_from_render_ready(&from_creator, "channel-1", 1, false, || unreachable!())
                .unwrap();

        assert_eq!(snapshot_vm.author.user_id, "user-from-snapshot");
        assert_eq!(creator_vm.author.user_id, "SYS");
    }

    /// 合并转发必须输出严格 typed card，不能把整个 forwardDetail object 塞进 items。
    #[test]
    fn merged_forward_projects_mode_title_and_typed_items() {
        let source = serde_json::json!({
            "msgId": "forward-1",
            "channelId": "channel-1",
            "userId": "sender-1",
            "createAt": 20,
            "text": "",
            "type": "MULTIPLY",
            "sendStatus": "sent",
            "props": {},
            "actionCapabilities": {},
            "forwardDetail": {
                "mode": "merged",
                "title": "项目群聊天记录",
                "summary": "共2条消息",
                "sourceChannelId": "source-channel",
                "sourceChannelName": "项目群",
                "items": [
                    {"id":"item-1","userId":"user-1","text":"第一条","type":"TEXT","createAt":1},
                    {"id":"item-2","userId":"user-2","text":"文档","type":"DOCUMENT","createAt":2}
                ]
            }
        });

        let message = message_from_render_ready(&source, "channel-1", 1, false, || {
            Ok("action-ref".to_string())
        })
        .unwrap();
        let encoded = serde_json::to_value(&message.content).unwrap();

        assert_eq!(encoded["kind"], "forwarded");
        assert_eq!(encoded["mode"], "merged");
        assert_eq!(encoded["title"], "项目群聊天记录");
        assert_eq!(encoded["source"]["channelId"], "source-channel");
        assert_eq!(encoded["items"][0]["content"]["kind"], "text");
        assert_eq!(encoded["items"][1]["content"]["kind"], "document");
        assert_eq!(encoded["items"].as_array().map(Vec::len), Some(2));
    }

    #[test]
    fn attachment_is_visible_before_the_first_snapshot_arrives() {
        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        let mut projector = TimelineProjector::default();

        projector.register_attachment(&scope);

        assert!(projector.is_attached(&scope));
        assert!(projector.current_view(&scope).is_none());
    }

    /// 证明模板确认 userIds 不会被误投影为普通消息已读回执。
    #[test]
    fn template_confirmation_ids_do_not_fabricate_read_receipts() {
        let source = serde_json::json!({
            "msgId": "template-1",
            "channelId": "channel-1",
            "userId": "author-1",
            "createAt": 1,
            "text": "template",
            "type": "TEMPLATE",
            "sendStatus": "sent",
            "readBits": "",
            "readerIds": ["recipient-1"],
            "templateConfirmation": {
                "templateReceived": true,
                "confirmedCount": 1,
                "readerIds": ["recipient-1"]
            },
            "props": {"template": {"userIds": ["recipient-1"]}},
            "actionCapabilities": {}
        });

        let message = message_from_render_ready(&source, "channel-1", 1, false, || {
            Ok("disabled-template-action".to_string())
        })
        .unwrap();
        let SurfaceDataVm::Record(receipt) = message.read_receipt else {
            panic!("read receipt remains a record")
        };
        assert_eq!(
            receipt.get("readerIds"),
            Some(&SurfaceDataVm::Array(Vec::new()))
        );
    }

    #[test]
    fn locate_and_receipt_authority_patch_the_existing_timeline_vm() {
        let mut projector = TimelineProjector::default();
        projector
            .snapshot_from_render_ready(TimelineWindowRequest::latest("channel-1"), &[row(1)])
            .unwrap();

        let locate = projector
            .patch_located_target("message-1", None)
            .unwrap()
            .unwrap();
        assert_eq!(locate.revision, 2);
        let receipts = serde_json::json!([{
            "postId": "message-1", "state": "ready", "readBits": "1",
            "readerCount": 1, "unreadCount": 0, "snapshotId": "snapshot-1",
            "snapshotVersion": 3, "authorExcluded": true, "contractVersion": 2
        }]);
        let receipt_patches = projector
            .patch_receipts(receipts.as_array().unwrap())
            .unwrap();
        assert_eq!(receipt_patches.len(), 1);
        assert_eq!(receipt_patches[0].base_revision, Some(2));
        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        let view = projector.current_view(&scope).unwrap();
        assert_eq!(view.target_message_id.as_deref(), Some("message-1"));
        let SurfaceDataVm::Record(receipt) = &view.items[0].read_receipt else {
            panic!("receipt record")
        };
        assert_eq!(
            receipt.get("snapshotId"),
            Some(&SurfaceDataVm::String("snapshot-1".to_string()))
        );

        let mut refreshed = row(1);
        refreshed["readBits"] = serde_json::json!("1");
        refreshed["text"] = serde_json::json!("updated message");
        projector
            .patch_from_render_ready(
                TimelineWindowRequest::latest("channel-1"),
                &[refreshed],
                None,
            )
            .unwrap();
        let view = projector.current_view(&scope).unwrap();
        let SurfaceDataVm::Record(receipt) = &view.items[0].read_receipt else {
            panic!("receipt record")
        };
        assert_eq!(
            receipt.get("snapshotId"),
            Some(&SurfaceDataVm::String("snapshot-1".to_string())),
            "timeline readback must preserve the queried member snapshot"
        );
        assert_eq!(
            receipt.get("readerCount"),
            Some(&SurfaceDataVm::Number(serde_json::Number::from(1))),
        );
        assert_eq!(
            receipt.get("unreadCount"),
            Some(&SurfaceDataVm::Number(serde_json::Number::from(0))),
        );
    }

    #[test]
    fn bounded_timeline_derives_reply_preview_from_authoritative_reply_anchors() {
        let root = row(1);
        let mut reply = row(2);
        reply["replyId"] = serde_json::json!("message-1");
        reply["replyRootId"] = serde_json::json!("message-1");
        let mut projector = TimelineProjector::default();
        let mutation = projector
            .snapshot_from_render_ready(TimelineWindowRequest::latest("channel-1"), &[root, reply])
            .unwrap();
        let TypedSurfaceMutation::TimelineSnapshot { view } = mutation.mutation else {
            panic!("timeline")
        };
        let thread = view.items[0].reply_thread.as_ref().expect("reply thread");
        assert_eq!(thread.reply_count, 1);
        assert_eq!(thread.items[0].id, "message-2");
    }

    #[test]
    fn message_projection_caps_reply_preview_and_preserves_authoritative_count_and_reactions() {
        let mut source = row(1);
        source["replyCount"] = serde_json::json!(5);
        source["replyMessages"] = serde_json::json!([
            {"id":"reply-1","userId":"user-1","message":"一","createAt":1},
            {"id":"reply-2","userId":"user-2","message":"二","createAt":2},
            {"id":"reply-3","userId":"user-3","message":"三","createAt":3},
            {"id":"reply-4","userId":"user-4","message":"四","createAt":4}
        ]);
        source["props"] = serde_json::json!({
            "quickReply": [
                {"emoji":"😊","userIds":["user-1","user-2"]},
                {"emoji":"👍","userIds":["user-3"]}
            ]
        });
        source["actionCapabilities"] = serde_json::json!({
            "canReact": true,
            "canReply": true,
            "canCreateTopic": true
        });

        let message = message_from_render_ready(&source, "channel-1", 1, false, || {
            Ok("action-ref".to_string())
        })
        .expect("render-ready message");

        assert_eq!(serde_json::to_value(&message).unwrap()["eventSeq"], 1);
        let thread = message.reply_thread.expect("reply summary");
        assert_eq!(thread.items.len(), 3);
        assert_eq!(thread.reply_count, 5);
        assert_eq!(thread.items[2].text, "三");
        assert_eq!(message.reactions[0].emoji, "😊");
        assert_eq!(message.reactions[0].count, 2);
        assert_eq!(message.reactions[1].count, 1);
        assert_eq!(message.event_seq, 1);
        assert_eq!(
            message.quick_actions,
            vec![
                MessageQuickActionVm::React,
                MessageQuickActionVm::QuoteReply,
                MessageQuickActionVm::CreateTopic
            ]
        );
    }

    #[test]
    fn document_projection_uses_typed_document_props_instead_of_placeholder_text() {
        let mut source = row(1);
        source["type"] = serde_json::json!("DOCUMENT");
        source["message"] = serde_json::json!("【文档】 http://doc.cses7.com/doc/document-1");
        source["props"] = serde_json::json!({
            "document": {
                "id": "document-1",
                "title": "项目周报",
                "url": "http://doc.cses7.com/doc/document-1",
                "version": 1,
                "placeholder": true
            }
        });

        let message =
            message_from_render_ready(&source, "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("document message");

        let MessageContentVm::Document {
            document_id,
            title,
            summary,
            access,
        } = message.content
        else {
            panic!("document content")
        };
        assert_eq!(document_id, "document-1");
        assert_eq!(title, "项目周报");
        assert_eq!(summary, "http://doc.cses7.com/doc/document-1");
        assert_eq!(
            access,
            SurfaceDataVm::Record(BTreeMap::from([
                (
                    "id".to_string(),
                    SurfaceDataVm::String("document-1".to_string())
                ),
                ("placeholder".to_string(), SurfaceDataVm::Bool(true)),
                (
                    "title".to_string(),
                    SurfaceDataVm::String("项目周报".to_string())
                ),
                (
                    "url".to_string(),
                    SurfaceDataVm::String("http://doc.cses7.com/doc/document-1".to_string())
                ),
                (
                    "version".to_string(),
                    SurfaceDataVm::Number(serde_json::Number::from(1))
                ),
            ]))
        );
    }

    /// 闭合 topic relation 同时补足历史 activity 默认值，避免 strict client 拒绝整帧。
    #[test]
    fn topic_relation_requires_and_normalizes_a_closed_topic_link() {
        let mut source = row(1);
        source["topic"] = serde_json::json!({"channelId":"topic-1"});
        let unresolved =
            message_from_render_ready(&source, "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("bare topic relation keeps the timeline row renderable");
        assert!(unresolved.topic_link.is_none());

        source["topic"] = serde_json::json!({
            "channelId":"topic-1",
            "displayName":"design review",
        });
        let resolved =
            message_from_render_ready(&source, "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("closed topic link");
        let encoded = serde_json::to_value(resolved).expect("serialize topic link");
        assert_eq!(
            encoded["topicLink"],
            serde_json::json!({
                "parentChannelId":"channel-1",
                "topicChannelId":"topic-1",
                "rootMessageId":"message-1",
                "title":"design review",
                "replyCount":0,
                "lastActivityAt":1,
            })
        );
    }

    /// 父群 timeline 只保留 topic root 卡片，topic 的镜像回复不得再次平铺成回复气泡。
    #[test]
    fn topic_reply_children_are_collapsed_into_root_card() {
        let mut root = row(1);
        root["topic"] = serde_json::json!({
            "channelId":"topic-1",
            "displayName":"design review",
            "replyCount":2,
            "lastActivityAt":3,
        });
        let mut first_reply = row(2);
        first_reply["replyId"] = serde_json::json!("message-1");
        first_reply["replyRootId"] = serde_json::json!("message-1");
        let mut second_reply = row(3);
        second_reply["replyId"] = serde_json::json!("message-1");
        second_reply["replyRootId"] = serde_json::json!("message-1");
        let mut projector = TimelineProjector::default();
        let view = projector
            .build_view(
                &TimelineWindowRequest::latest("channel-1"),
                &[root, first_reply, second_reply],
            )
            .unwrap();

        assert_eq!(view.items.len(), 1);
        assert_eq!(view.items[0].id, "message-1");
        assert_eq!(
            view.items[0]
                .reply_thread
                .as_ref()
                .map(|thread| thread.reply_count),
            Some(2)
        );
    }

    #[test]
    /// 证明 failed delivery 与 retry action 读取同一 render-ready capability。
    fn failed_delivery_is_retryable_when_retry_action_is_available() {
        let mut source = row(1);
        source["sendStatus"] = serde_json::json!("unsend");
        source["actionCapabilities"] = serde_json::json!({"canRetry": true});

        let message = message_from_render_ready(&source, "channel-1", 1, true, || {
            Ok("retry-action-ref".to_string())
        })
        .expect("failed render-ready message");

        assert!(matches!(
            message.delivery,
            DeliveryVm::Failed {
                retryable: true,
                ..
            }
        ));
        assert!(message
            .actions
            .iter()
            .any(|action| action.kind == "retry" && action.enabled));
    }

    #[test]
    fn recalled_message_is_a_terminal_non_interactive_system_row() {
        let mut source = row(1);
        source["revoke"] = serde_json::json!(true);
        source["replyCount"] = serde_json::json!(1);
        source["replyMessages"] = serde_json::json!([
            {"id":"reply-1","userId":"user-2","message":"reply","createAt":2}
        ]);
        source["props"] = serde_json::json!({
            "quickReply": [{"emoji":"👍","userIds":["user-2"]}]
        });
        source["topic"] = serde_json::json!({"topicChannelId":"topic-1"});
        source["actionCapabilities"] = serde_json::json!({
            "canReact": true,
            "canReply": true,
            "canForward": true,
            "canRevoke": true
        });
        let mut issued_refs = 0;

        let message = message_from_render_ready(&source, "channel-1", 1, true, || {
            issued_refs += 1;
            Ok(format!("action-ref-{issued_refs}"))
        })
        .expect("recalled render-ready message");
        let encoded = serde_json::to_value(&message).expect("serialize recalled message");

        assert!(matches!(message.content, MessageContentVm::Recalled { .. }));
        assert!(matches!(message.delivery, DeliveryVm::Recalled));
        assert!(message.actions.is_empty());
        assert!(message.quick_actions.is_empty());
        assert!(message.reply_thread.is_none());
        assert!(message.reactions.is_empty());
        assert!(message.topic_link.is_none());
        assert_eq!(message.reply_summary, ReplySummaryVm::None);
        assert_eq!(
            issued_refs, 0,
            "recalled rows must not mint executable action refs"
        );
        assert!(encoded.get("quickActions").is_none());
        assert!(encoded.get("replyThread").is_none());
        assert!(encoded.get("reactions").is_none());
        assert!(encoded.get("topicLink").is_none());
    }

    #[test]
    fn reply_summary_is_a_closed_v2_union() {
        let plain =
            message_from_render_ready(&row(1), "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("plain message");
        assert_eq!(plain.reply_summary, ReplySummaryVm::None);
        assert_eq!(
            serde_json::to_value(&plain).unwrap()["replySummary"],
            serde_json::json!({"kind":"none"})
        );

        let mut quoted = row(2);
        quoted["replyId"] = serde_json::json!("message-1");
        quoted["repliedMessage"] = serde_json::json!({
            "id": "message-1",
            "userId": "user-1",
            "type": "TEXT",
            "message": "被引用正文"
        });
        let available =
            message_from_render_ready(&quoted, "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("quoted message");
        assert_eq!(
            serde_json::to_value(&available).unwrap()["replySummary"],
            serde_json::json!({
                "kind": "available",
                "targetMessageId": "message-1",
                "author": {"userId": "user-1"},
                "preview": {"kind": "text", "text": "被引用正文"},
                "availability": {"kind": "available"}
            })
        );

        quoted["repliedMessage"] = Value::Null;
        let unavailable =
            message_from_render_ready(&quoted, "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("unavailable quoted message remains renderable");
        assert_eq!(
            serde_json::to_value(&unavailable).unwrap()["replySummary"],
            serde_json::json!({
                "kind": "unavailable",
                "targetMessageId": "message-1",
                "availability": {
                    "kind": "unavailable",
                    "reason": "referencedMessageUnavailable"
                }
            })
        );
    }

    #[test]
    /// 证明 Go RepliedMessage 的 camel/snake readback 均能生成 available 与 recalled union。
    fn reply_summary_accepts_go_replied_message_wire_aliases() {
        for (author_key, revoke_key) in [
            ("repliedUserId", "isRevoke"),
            ("replied_user_id", "is_revoke"),
        ] {
            let mut quoted = row(2);
            quoted["replyId"] = serde_json::json!("message-1");
            quoted["repliedMessage"] = serde_json::json!({
                "repliedUserName": "被回复者",
                "type": "TEXT",
                "message": "刚发送的引用快照",
                "props": {},
                "viewers": ["all"]
            });
            quoted["repliedMessage"][author_key] = Value::String("user-1".to_string());
            quoted["repliedMessage"][revoke_key] = Value::Bool(false);

            let available = message_from_render_ready(&quoted, "channel-1", 1, false, || {
                Ok("unused".to_string())
            })
            .expect("Go repliedMessage readback remains renderable");
            assert_eq!(
                serde_json::to_value(&available).unwrap()["replySummary"],
                serde_json::json!({
                    "kind": "available",
                    "targetMessageId": "message-1",
                    "author": {"userId": "user-1"},
                    "preview": {"kind": "text", "text": "刚发送的引用快照"},
                    "availability": {"kind": "available"}
                })
            );

            quoted["repliedMessage"][revoke_key] = Value::Bool(true);
            let recalled = message_from_render_ready(&quoted, "channel-1", 1, false, || {
                Ok("unused".to_string())
            })
            .expect("recalled Go repliedMessage remains renderable");
            assert_eq!(
                serde_json::to_value(&recalled).unwrap()["replySummary"],
                serde_json::json!({
                    "kind": "unavailable",
                    "targetMessageId": "message-1",
                    "availability": {
                        "kind": "recalled",
                        "text": "消息已撤回"
                    }
                })
            );
        }
    }

    #[test]
    fn notice_member_leave_is_a_render_ready_system_event() {
        let mut source = row(1);
        source["type"] = serde_json::json!("NOTICE");
        source["text"] = serde_json::json!("");
        source["props"] = serde_json::json!({
            "type": "leave",
            "users": [{"id": "678", "name": "擎天柱"}]
        });

        let message =
            message_from_render_ready(&source, "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("notice member leave");
        let encoded = serde_json::to_value(&message).expect("serialize member leave");

        assert_eq!(encoded["content"]["kind"], "systemEvent");
        assert_eq!(encoded["content"]["eventKind"], "member-leave");
        assert_eq!(encoded["content"]["memberId"], "678");
        assert_eq!(encoded["content"]["text"], "擎天柱退出群聊");
    }

    #[test]
    fn only_file_upload_status_projects_percent_delivery() {
        let mut file = row(1);
        file["type"] = serde_json::json!("FILE");
        file["sendStatus"] = serde_json::json!("sending");
        file["progressPercent"] = serde_json::json!(35);
        file["props"] = serde_json::json!({
            "file": {"upload": {"status": "uploading"}}
        });
        let file_message =
            message_from_render_ready(&file, "channel-1", 1, false, || Ok("unused".to_string()))
                .unwrap();
        assert_eq!(
            file_message.delivery,
            DeliveryVm::Uploading {
                progress_percent: 35
            }
        );

        let mut image = row(2);
        image["type"] = serde_json::json!("IMAGE");
        image["sendStatus"] = serde_json::json!("sending");
        image["progressPercent"] = serde_json::json!(35);
        image["props"] = serde_json::json!({
            "files": [{"upload": {"status": "uploading"}}]
        });
        let image_message =
            message_from_render_ready(&image, "channel-1", 1, false, || Ok("unused".to_string()))
                .unwrap();
        assert_eq!(image_message.delivery, DeliveryVm::Sending);
    }

    #[test]
    fn recovered_unsend_overrides_stale_file_uploading_substate() {
        let mut file = row(1);
        file["type"] = serde_json::json!("FILE");
        file["sendStatus"] = serde_json::json!("unsend");
        file["progressPercent"] = serde_json::json!(35);
        file["canRetry"] = serde_json::json!(true);
        file["actionCapabilities"] = serde_json::json!({
            "canRetry": true
        });
        file["props"] = serde_json::json!({
            "file": {"upload": {"status": "uploading"}}
        });

        let message =
            message_from_render_ready(&file, "channel-1", 1, true, || Ok("retry-ref".to_string()))
                .unwrap();
        assert_eq!(
            message.delivery,
            DeliveryVm::Failed {
                reason: String::new(),
                retryable: true,
            }
        );
        assert!(message
            .actions
            .iter()
            .any(|action| action.kind == "retry" && action.enabled));
    }

    #[test]
    fn final_file_content_projects_the_stable_props_file_reference() {
        let mut file = row(1);
        file["type"] = serde_json::json!("FILE");
        file["fileName"] = serde_json::json!("stale-top-level.pdf");
        file["size"] = serde_json::json!(1);
        file["props"] = serde_json::json!({
            "file": {
                "bucket": "file",
                "name": "report.pdf",
                "size": 4096,
                "contentType": "application/pdf",
                "url": "https://java.example/oss/media/object/mf_report",
                "uri": "https://java.example/oss/media/object/mf_report"
            },
            "unrelated": "must-not-leak-into-resource"
        });

        let message =
            message_from_render_ready(&file, "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("stable FILE props must project");
        let content = serde_json::to_value(message.content).expect("serialize FILE content");

        assert_eq!(content["kind"], "media");
        assert_eq!(content["mediaType"], "file");
        assert_eq!(content["fileName"], "report.pdf");
        assert_eq!(content["size"], 4096);
        assert_eq!(
            content["resource"],
            serde_json::json!({
                "bucket": "file",
                "name": "report.pdf",
                "size": 4096,
                "contentType": "application/pdf",
                "url": "https://java.example/oss/media/object/mf_report",
                "uri": "https://java.example/oss/media/object/mf_report"
            })
        );
        assert!(content["resource"].get("file").is_none());
        assert!(content["resource"].get("unrelated").is_none());
    }

    #[test]
    fn final_image_content_projects_the_complete_props_files_gallery() {
        let mut image = row(2);
        image["type"] = serde_json::json!("IMAGE");
        image["text"] = serde_json::json!("两张现场图片");
        image["fileName"] = serde_json::json!("stale-top-level.png");
        image["size"] = serde_json::json!(1);
        image["props"] = serde_json::json!({
            "imagePosition": "bottom",
            "files": [
                {
                    "bucket": "picture",
                    "name": "a.png",
                    "size": 11,
                    "contentType": "image/png",
                    "uri": "media/mf_a"
                },
                {
                    "bucket": "picture",
                    "name": "b.png",
                    "size": 22,
                    "contentType": "image/png",
                    "uri": "media/mf_b"
                }
            ]
        });

        let message =
            message_from_render_ready(&image, "channel-1", 1, false, || Ok("unused".to_string()))
                .expect("stable IMAGE props must project");
        let content = serde_json::to_value(message.content).expect("serialize IMAGE content");

        assert_eq!(content["kind"], "media");
        assert_eq!(content["mediaType"], "image");
        assert_eq!(content["fileName"], "a.png");
        assert_eq!(content["size"], 11);
        assert_eq!(content["resource"], image["props"]["files"]);
        assert!(content["resource"].is_array());
        assert_eq!(content["caption"], "两张现场图片");
        assert_eq!(content["imagePosition"], "bottom");
        assert_eq!(content["items"].as_array().map(Vec::len), Some(2));
        assert_eq!(content["items"][0]["fileId"], "mf_a");
        assert_eq!(content["items"][0]["fileName"], "a.png");
        assert_eq!(content["items"][0]["contentType"], "image/png");
        assert_eq!(content["items"][0]["uri"], "media/mf_a");
        assert_eq!(content["items"][0]["bucket"], "picture");
        assert_eq!(content["items"][1]["fileId"], "mf_b");
    }

    #[test]
    /// 验证最新窗口边界正确，新增消息只产生 bounded TimelineDelta。
    fn snapshot_bounds_latest_window_and_delta_advances_changed_view() {
        let mut projector = TimelineProjector::default();
        let rows = (0..80).map(row).collect::<Vec<_>>();
        let mut request = TimelineWindowRequest::latest("channel-1");
        request.page_size = MAX_TIMELINE_PAGE_SIZE;
        let snapshot = projector
            .snapshot_from_render_ready(request.clone(), &rows)
            .expect("valid timeline snapshot");
        assert_eq!(snapshot.revision, 1);
        let TypedSurfaceMutation::TimelineSnapshot { view } = snapshot.mutation else {
            panic!("timeline snapshot mutation")
        };
        assert_eq!(view.items.len(), MAX_TIMELINE_WINDOW_ITEMS);
        assert_eq!(view.items.first().unwrap().id, "message-20");
        assert_eq!(view.items.last().unwrap().id, "message-79");
        assert!(view.page.has_older);

        let advanced_rows = (1..=80).map(row).collect::<Vec<_>>();
        let patch = projector
            .patch_from_render_ready(request, &advanced_rows, Some("post:80".to_string()))
            .expect("existing slot can advance");
        assert_eq!(patch.base_revision, Some(1));
        assert_eq!(patch.revision, 2);
        assert_eq!(patch.causation_id.as_deref(), Some("post:80"));
        assert!(matches!(
            patch.mutation,
            TypedSurfaceMutation::TimelineDelta { .. }
        ));
    }

    #[test]
    /// 验证最大页长定位后可向两侧各扩一页，且分页不会激活或丢失目标窗口。
    fn located_max_page_keeps_target_across_bidirectional_range_pagination() {
        let mut projector = TimelineProjector::default();
        let mut locate_request =
            TimelineWindowRequest::latest_with_window_token("channel-1", "locate-60");
        locate_request.page_size = MAX_TIMELINE_PAGE_SIZE;
        locate_request.anchor = TimelineAnchorVm {
            message_id: Some("message-90".to_string()),
            mode: TimelineAnchorMode::Locate,
        };
        locate_request.target_message_id = Some("message-90".to_string());
        projector
            .snapshot_from_render_ready(
                locate_request.clone(),
                &(60..120).map(row).collect::<Vec<_>>(),
            )
            .expect("max-size locate page attaches");

        let older = projector
            .patch_page_from_render_ready(
                locate_request.clone(),
                &(0..60).map(row).collect::<Vec<_>>(),
                TimelinePageMutation::Older,
                Some("older-60".to_string()),
            )
            .expect("older page extends the located range");
        let TypedSurfaceMutation::TimelineDelta {
            operations: older_operations,
        } = older.mutation
        else {
            panic!("older page must produce a timeline delta")
        };
        assert!(!older_operations
            .iter()
            .any(|operation| matches!(operation, TimelineDeltaOperationVm::ActivateWindow { .. })));

        let newer = projector
            .patch_page_from_render_ready(
                locate_request.clone(),
                &(120..180).map(row).collect::<Vec<_>>(),
                TimelinePageMutation::Newer,
                Some("newer-60".to_string()),
            )
            .expect("newer page extends the located range");
        let TypedSurfaceMutation::TimelineDelta {
            operations: newer_operations,
        } = newer.mutation
        else {
            panic!("newer page must produce a timeline delta")
        };
        assert!(!newer_operations
            .iter()
            .any(|operation| matches!(operation, TimelineDeltaOperationVm::ActivateWindow { .. })));

        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "locate-60".to_string(),
        };
        let view = projector.current_view(&scope).expect("located slot");
        assert_eq!(view.items.len(), MAX_TIMELINE_RANGE_ITEMS);
        assert_eq!(view.items.first().unwrap().id, "message-0");
        assert_eq!(view.items.last().unwrap().id, "message-179");
        assert_eq!(view.target_message_id.as_deref(), Some("message-90"));
    }

    #[test]
    /// 验证有实际实体变化时 revision 可到达 JSON 安全整数上限但不会溢出。
    fn revision_overflow_is_rejected_without_wrapping_the_timeline_slot() {
        let mut projector = TimelineProjector::default();
        let rows = vec![row(1)];
        projector
            .snapshot_from_render_ready(TimelineWindowRequest::latest("channel-1"), &rows)
            .unwrap();
        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        projector
            .slots
            .get_mut(&scope.canonical_key())
            .expect("snapshot slot")
            .revision = MAX_SAFE_JSON_INTEGER - 1;
        let mut changed_rows = rows.clone();
        changed_rows[0]["text"] = serde_json::json!("changed-at-boundary");
        let boundary_patch = projector
            .patch_from_render_ready(
                TimelineWindowRequest::latest("channel-1"),
                &changed_rows,
                None,
            )
            .expect("the exact-safe maximum itself remains representable");
        assert_eq!(boundary_patch.revision, MAX_SAFE_JSON_INTEGER);
        changed_rows[0]["text"] = serde_json::json!("changed-after-boundary");
        assert!(matches!(
            projector.patch_from_render_ready(
                TimelineWindowRequest::latest("channel-1"),
                &changed_rows,
                None,
            ),
            Err(TimelineProjectionError::RevisionOverflow)
        ));
    }

    #[test]
    fn unsafe_persisted_number_is_rejected_before_timeline_slot_commits() {
        let mut projector = TimelineProjector::default();
        let request = TimelineWindowRequest::latest("channel-1");
        let scope = SurfaceScope::Timeline {
            channel_id: "channel-1".to_string(),
            window_token: "latest".to_string(),
        };
        let mut just_outside_safe_range = row(1);
        just_outside_safe_range["createAt"] = serde_json::json!(MAX_SAFE_JSON_INTEGER as i64 + 1);

        assert!(matches!(
            projector.snapshot_from_render_ready(request.clone(), &[just_outside_safe_range]),
            Err(TimelineProjectionError::InvalidInteractionFrame(
                InteractionFrameError::UnsafeWireNumber {
                    field: "createdAt",
                    ..
                }
            ))
        ));
        assert!(projector.current_view(&scope).is_none());

        let valid = row(2);
        projector
            .snapshot_from_render_ready(request.clone(), std::slice::from_ref(&valid))
            .expect("valid row attaches the timeline slot");
        let before = projector.current_view(&scope).cloned();
        let mut unsafe_created_at = row(3);
        unsafe_created_at["createAt"] = serde_json::json!(u64::MAX);

        assert!(matches!(
            projector.patch_from_render_ready(request.clone(), &[unsafe_created_at], None),
            Err(TimelineProjectionError::InvalidInteractionFrame(
                InteractionFrameError::UnsafeWireNumber {
                    field: "createdAt",
                    ..
                }
            ))
        ));
        assert_eq!(projector.current_view(&scope).cloned(), before);

        let mut unsafe_media_size = row(4);
        unsafe_media_size["type"] = serde_json::json!("FILE");
        unsafe_media_size["size"] = serde_json::json!(u64::MAX);
        assert!(matches!(
            projector.patch_from_render_ready(request.clone(), &[unsafe_media_size], None),
            Err(TimelineProjectionError::InvalidInteractionFrame(
                InteractionFrameError::UnsafeWireNumber { field: "size", .. }
            ))
        ));
        assert_eq!(projector.current_view(&scope).cloned(), before);

        let mut unsafe_upload_progress = row(5);
        unsafe_upload_progress["type"] = serde_json::json!("FILE");
        unsafe_upload_progress["props"] = serde_json::json!({
            "file": {"upload": {"status": "uploading"}}
        });
        unsafe_upload_progress["progressPercent"] = serde_json::json!(u64::MAX);
        assert!(matches!(
            projector.patch_from_render_ready(request, &[unsafe_upload_progress], None),
            Err(TimelineProjectionError::InvalidInteractionFrame(
                InteractionFrameError::UnsafeWireNumber {
                    field: "progressPercent",
                    ..
                }
            ))
        ));
        assert_eq!(projector.current_view(&scope).cloned(), before);
    }

    #[test]
    fn action_refs_are_opaque_and_only_the_live_authority_mapping_is_enabled() {
        let mut projector = TimelineProjector::default();
        let mut capability_row = row(1);
        capability_row["actionCapabilities"] = serde_json::json!({
            "canReply": true,
            "canCreateTopic": true,
            "canMarkRead": true,
            "canViewReceipt": true,
            "canTemplateConfirm": true,
            "canLocate": true,
        });

        let mut request = TimelineWindowRequest::latest("channel-1");
        request.surface_epoch = 42;
        let snapshot = projector
            .snapshot_from_render_ready(request, &[capability_row])
            .expect("valid capability facts project");
        let TypedSurfaceMutation::TimelineSnapshot { view } = snapshot.mutation else {
            panic!("timeline snapshot expected")
        };
        let receipt = view.items[0]
            .actions
            .iter()
            .find(|action| action.kind == "viewReceipt")
            .expect("receipt binding is present");
        assert!(receipt.enabled);
        assert!(receipt.disabled_reason.is_none());
        assert_eq!(receipt.surface_epoch, 42);
        assert!(!receipt.action_ref.contains("message-1"));
        assert!(!receipt.action_ref.contains("viewReceipt"));
        let locate = view.items[0]
            .actions
            .iter()
            .find(|action| action.kind == "locate")
            .expect("locate binding is present");
        assert!(locate.enabled);
        assert!(locate.disabled_reason.is_none());
        let mark_read = view.items[0]
            .actions
            .iter()
            .find(|action| action.kind == "markRead")
            .expect("mark-read binding is present");
        assert!(mark_read.enabled);
        assert!(mark_read.disabled_reason.is_none());

        for kind in ["reply", "createTopic", "templateConfirm"] {
            let action = view.items[0]
                .actions
                .iter()
                .find(|action| action.kind == kind)
                .unwrap_or_else(|| panic!("{kind} binding is present"));
            assert!(!action.enabled, "{kind}");
            assert_eq!(
                action.disabled_reason.as_deref(),
                Some("unsupportedByAuthority"),
                "{kind} cannot be enabled by a row capability before the terminal authority Frame exists"
            );
        }
    }

    #[test]
    fn locate_target_must_be_present_in_the_complete_bounded_vm() {
        let mut projector = TimelineProjector::default();
        let mut request = TimelineWindowRequest::latest_with_window_token("channel-1", "locate-1");
        request.anchor = TimelineAnchorVm {
            message_id: Some("message-2".to_string()),
            mode: TimelineAnchorMode::Locate,
        };
        request.target_message_id = Some("message-2".to_string());
        let mutation = projector
            .snapshot_from_render_ready(request.clone(), &[row(1), row(2), row(3)])
            .unwrap();
        let TypedSurfaceMutation::TimelineSnapshot { view } = mutation.mutation else {
            panic!("timeline snapshot expected")
        };
        assert_eq!(view.target_message_id.as_deref(), Some("message-2"));

        request.window_token = "locate-missing".to_string();
        request.page.window_token = "locate-missing".to_string();
        request.target_message_id = Some("message-99".to_string());
        assert!(matches!(
            projector.snapshot_from_render_ready(request, &[row(1), row(2)]),
            Err(TimelineProjectionError::TargetNotInWindow { target })
                if target == "message-99"
        ));
    }
}
