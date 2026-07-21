//! 左侧会话列表的跨端 render-ready 投影。
//!
//! 这个模块把持久化 channel 行与写后 DialogPatch 转换为唯一的 UI ViewModel。
//! 客户端只验证 scope/revision 后原子替换 `replaceView`，不解析 post、计算 badge 或重排。

use std::collections::HashMap;

use helix_core::Effect;
use serde::Serialize;
use serde_json::Value;

use crate::interaction_frame;
use crate::projector::unread_summary::{
    UnreadSummaryPatch, UnreadSummaryProjector, UnreadSummarySnapshot, ViewerChannelUnread,
};

pub const EVENT_NAME: &str = "im:interaction:frame";
pub const SURFACE: &str = "message.left-dialog-list";
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogListScope {
    pub viewer_user_id: String,
    pub team_id: String,
}

impl DialogListScope {
    pub fn new(viewer_user_id: &str, team_id: &str) -> Self {
        Self {
            viewer_user_id: viewer_user_id.to_string(),
            team_id: team_id.to_string(),
        }
    }

    fn is_complete(&self) -> bool {
        !self.viewer_user_id.is_empty() && !self.team_id.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogAvatarVm {
    pub kind: String,
    pub user_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogPreviewVm {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sender_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sent_at: Option<i64>,
    pub system: bool,
    pub recalled: bool,
    pub urgent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogBadgesVm {
    pub unread_count: i64,
    pub mention_count: i64,
    pub pending_urgent_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogStateVm {
    pub pinned: bool,
    pub muted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online: Option<bool>,
    pub closed: bool,
    pub left: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogListRowVm {
    pub channel_id: String,
    pub title: String,
    pub channel_kind: String,
    pub member_count: i64,
    pub avatar: DialogAvatarVm,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<DialogPreviewVm>,
    pub badges: DialogBadgesVm,
    pub state: DialogStateVm,
    pub activity_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_count: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DialogRowSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<DialogAvatarVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<DialogPreviewVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badges: Option<DialogBadgesVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<DialogStateVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic_count: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum DialogListPatchOperation {
    UpsertRow {
        channel_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        row: Option<DialogListRowVm>,
        set: DialogRowSet,
        clear: Vec<String>,
    },
    MoveRow {
        channel_id: String,
        to_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DialogListPatch {
    pub operations: Vec<DialogListPatchOperation>,
}

/// 当前 scope 的最后已发布列表。它只保存 render-ready VM，绝不反向读取 storage。
#[derive(Debug, Default)]
pub struct DialogListFrameState {
    revision: u64,
    scope: Option<DialogListScope>,
    rows_by_channel: HashMap<String, DialogListRowVm>,
    ordered_channel_ids: Vec<String>,
    unread_summary: UnreadSummaryProjector,
}

impl DialogListFrameState {
    pub fn scope(&self) -> Option<&DialogListScope> {
        self.scope.as_ref()
    }

    /// identity 切换后不能保留旧 viewer 已 attach 的 dialog/unread slots。
    pub(crate) fn reset(&mut self) {
        self.revision = 0;
        self.scope = None;
        self.rows_by_channel.clear();
        self.ordered_channel_ids.clear();
        self.unread_summary = UnreadSummaryProjector::default();
    }

    pub fn snapshot(
        &mut self,
        scope: DialogListScope,
        rows: Vec<DialogListRowVm>,
    ) -> Option<Effect> {
        if !scope.is_complete() {
            tracing::warn!(
                surface = SURFACE,
                "dialog list snapshot requires a complete viewer scope; suppressing frame"
            );
            return None;
        }
        if rows
            .iter()
            .any(|row| ensure_dialog_row_wire_safe(row).is_none())
        {
            return None;
        }
        let revision = self.next_revision()?;
        self.ensure_unread_can_advance()?;
        self.rows_by_channel.clear();
        self.ordered_channel_ids.clear();
        self.scope = Some(scope.clone());
        for mut row in rows {
            if self.rows_by_channel.contains_key(&row.channel_id) {
                continue;
            }
            select_avatar_users_for_viewer(
                &mut row.avatar,
                &row.channel_kind,
                scope.viewer_user_id.as_str(),
            );
            self.ordered_channel_ids.push(row.channel_id.clone());
            self.rows_by_channel.insert(row.channel_id.clone(), row);
        }
        // 频道行携带的 unread/mention/urgent 是当前 viewer 的权威绝对值。首次快照可 O(n)
        // 建 aggregate；之后的每条 row 更新只对对应贡献做 O(1) replace，客户端不参与推导。
        let unread_snapshot = self
            .unread_summary
            .snapshot(
                self.rows_by_channel
                    .iter()
                    .map(|(channel_id, row)| (channel_id.clone(), unread_contribution(row))),
            )
            .map_err(|error| {
                tracing::warn!(
                    surface = "message.unread-summary",
                    error = %error,
                    "unread snapshot rejected after dialog preflight"
                );
            })
            .ok()?;
        self.revision = revision;
        self.emit_snapshot(scope, unread_snapshot)
    }

    /// 在写后投影已经提供绝对累计值时，仅改变这条 row 的相关属性并一次性发布。
    /// 未完成 snapshot 的 channel 不发 patch，等待权威 snapshot，避免制造不可应用的增量。
    pub fn patch_from_post_projection(
        &mut self,
        scope: DialogListScope,
        data: &Value,
    ) -> Option<Effect> {
        if !scope.is_complete() {
            return None;
        }
        ensure_dialog_projection_numbers_safe(data)?;
        let channel_id = text(data, &["channelId", "channel_id"])?;
        let previous = self.rows_by_channel.get(&channel_id)?.clone();
        let mut next = previous.clone();
        let mut set = DialogRowSet::default();
        apply_post_projection_to_row(data, &previous, &mut next, &mut set);
        self.publish_existing_patch(scope, channel_id, previous, next, set)
    }

    /// 频道生命周期或设置推送只能修改在场行明确携带的属性；缺字段不是零值更新。
    pub fn patch_from_channel_update(
        &mut self,
        scope: DialogListScope,
        value: &Value,
    ) -> Option<Effect> {
        if !scope.is_complete() {
            return None;
        }
        ensure_dialog_projection_numbers_safe(value)?;
        let channel_id = text(value, &["channelId", "channel_id", "id"])?;
        let previous = self.rows_by_channel.get(&channel_id)?.clone();
        let mut next = previous.clone();
        let mut set = DialogRowSet::default();
        apply_channel_update_to_row(&scope, value, &previous, &mut next, &mut set);
        self.publish_existing_patch(scope, channel_id, previous, next, set)
    }

    /// 合并同一 `update_channel` 领域事实中的频道属性与成员绝对 dialog 属性，只递增一次 revision。
    pub fn patch_from_channel_and_dialog_projection(
        &mut self,
        scope: DialogListScope,
        channel: &Value,
        dialog: &Value,
    ) -> Option<Effect> {
        if !scope.is_complete() {
            return None;
        }
        ensure_dialog_projection_numbers_safe(channel)?;
        ensure_dialog_projection_numbers_safe(dialog)?;
        let channel_id = text(channel, &["channelId", "channel_id", "id"])?;
        if text(dialog, &["channelId", "channel_id"])
            .as_deref()
            .is_some_and(|id| id != channel_id)
        {
            return None;
        }
        let previous = self.rows_by_channel.get(&channel_id)?.clone();
        let mut next = previous.clone();
        let mut set = DialogRowSet::default();
        apply_channel_update_to_row(&scope, channel, &previous, &mut next, &mut set);
        apply_post_projection_to_row(dialog, &previous, &mut next, &mut set);
        self.publish_existing_patch(scope, channel_id, previous, next, set)
    }

    pub fn patch_insert_channel(
        &mut self,
        scope: DialogListScope,
        value: &Value,
    ) -> Option<Effect> {
        if !scope.is_complete() {
            return None;
        }
        let mut row = row_from_dialog_projection(value)?;
        select_avatar_users_for_viewer(
            &mut row.avatar,
            &row.channel_kind,
            scope.viewer_user_id.as_str(),
        );
        ensure_dialog_row_wire_safe(&row)?;
        if self.rows_by_channel.contains_key(&row.channel_id) {
            return self.patch_from_channel_update(scope, value);
        }
        let revision = self.next_revision()?;
        self.ensure_unread_can_advance()?;
        let base_revision = self.revision;
        let channel_id = row.channel_id.clone();
        let before_len = self.ordered_channel_ids.len();
        let unread_patch = self
            .unread_summary
            .upsert(channel_id.clone(), unread_contribution(&row))
            .map_err(|error| {
                tracing::warn!(
                    surface = "message.unread-summary",
                    error = %error,
                    "unread patch rejected after dialog preflight"
                );
            })
            .ok()?;
        self.rows_by_channel.insert(channel_id.clone(), row.clone());
        let target = self.reposition(&channel_id);
        let mut operations = vec![DialogListPatchOperation::UpsertRow {
            channel_id: channel_id.clone(),
            row: Some(row),
            set: DialogRowSet::default(),
            clear: Vec::new(),
        }];
        if target != before_len {
            operations.push(DialogListPatchOperation::MoveRow {
                channel_id,
                to_index: target,
            });
        }
        self.revision = revision;
        self.emit_patch(scope, base_revision, operations, unread_patch)
    }

    pub fn patch_closed(&mut self, scope: DialogListScope, channel_id: &str) -> Option<Effect> {
        let previous = self.rows_by_channel.get(channel_id)?.clone();
        if previous.state.closed {
            return None;
        }
        let mut next = previous.clone();
        next.state.closed = true;
        self.publish_existing_patch(
            scope,
            channel_id.to_string(),
            previous,
            next.clone(),
            DialogRowSet {
                state: Some(next.state),
                ..DialogRowSet::default()
            },
        )
    }

    fn publish_existing_patch(
        &mut self,
        scope: DialogListScope,
        channel_id: String,
        previous: DialogListRowVm,
        next: DialogListRowVm,
        set: DialogRowSet,
    ) -> Option<Effect> {
        if set_is_empty(&set) {
            return None;
        }
        ensure_dialog_row_wire_safe(&next)?;
        let old_index = self
            .ordered_channel_ids
            .iter()
            .position(|id| id == &channel_id)?;
        let revision = self.next_revision()?;
        self.ensure_unread_can_advance()?;
        let base_revision = self.revision;
        let unread_patch = self
            .unread_summary
            .upsert(channel_id.clone(), unread_contribution(&next))
            .map_err(|error| {
                tracing::warn!(
                    surface = "message.unread-summary",
                    error = %error,
                    "unread patch rejected after dialog preflight"
                );
            })
            .ok()?;
        self.rows_by_channel.insert(channel_id.clone(), next);
        let next_index = self.reposition(&channel_id);
        let mut operations = vec![DialogListPatchOperation::UpsertRow {
            channel_id: channel_id.clone(),
            row: None,
            set,
            clear: Vec::new(),
        }];
        if old_index != next_index
            || previous.state.pinned != self.rows_by_channel.get(&channel_id)?.state.pinned
        {
            operations.push(DialogListPatchOperation::MoveRow {
                channel_id,
                to_index: next_index,
            });
        }
        self.revision = revision;
        self.emit_patch(scope, base_revision, operations, unread_patch)
    }

    fn emit_patch(
        &self,
        scope: DialogListScope,
        base_revision: u64,
        operations: Vec<DialogListPatchOperation>,
        unread_patch: Option<UnreadSummaryPatch>,
    ) -> Option<Effect> {
        // 旧 row operation 只用于本 adapter 的确定性状态更新；正式 v1 wire 一律以
        // 同 surface 的 replaceView 发送完整 VM，平台端不再消费该 legacy delta。
        let _legacy_patch = DialogListPatch { operations };
        let user_scope = interaction_frame::SurfaceScope::User {
            user_id: scope.viewer_user_id,
            team_id: scope.team_id,
        };
        let mut mutations = vec![interaction_frame::SurfaceMutation {
            surface: interaction_frame::SurfaceId::MessageLeftDialogList,
            scope: user_scope.clone(),
            revision: self.revision,
            base_revision: Some(base_revision),
            causation_id: None,
            mutation: interaction_frame::TypedSurfaceMutation::DialogListPatch {
                patch: interaction_frame::ReplaceView {
                    kind: interaction_frame::ReplaceViewKind::ReplaceView,
                    view: self.interaction_view(),
                },
            },
        }];
        if let Some(unread_patch) = unread_patch {
            mutations.push(interaction_frame::SurfaceMutation {
                surface: interaction_frame::SurfaceId::MessageUnreadSummary,
                scope: user_scope.clone(),
                revision: unread_patch.revision,
                base_revision: Some(unread_patch.base_revision),
                causation_id: None,
                mutation: interaction_frame::TypedSurfaceMutation::UnreadSummaryPatch {
                    patch: interaction_frame::ReplaceView {
                        kind: interaction_frame::ReplaceViewKind::ReplaceView,
                        view: unread_patch.view,
                    },
                },
            });
        }
        self.emit_interaction_frame(&user_scope, mutations)
    }

    fn emit_snapshot(
        &self,
        scope: DialogListScope,
        unread_snapshot: UnreadSummarySnapshot,
    ) -> Option<Effect> {
        let user_scope = interaction_frame::SurfaceScope::User {
            user_id: scope.viewer_user_id,
            team_id: scope.team_id,
        };
        self.emit_interaction_frame(
            &user_scope,
            vec![
                interaction_frame::SurfaceMutation {
                    surface: interaction_frame::SurfaceId::MessageLeftDialogList,
                    scope: user_scope.clone(),
                    revision: self.revision,
                    base_revision: None,
                    causation_id: None,
                    mutation: interaction_frame::TypedSurfaceMutation::DialogListSnapshot {
                        view: self.interaction_view(),
                    },
                },
                interaction_frame::SurfaceMutation {
                    surface: interaction_frame::SurfaceId::MessageUnreadSummary,
                    scope: user_scope.clone(),
                    revision: unread_snapshot.revision,
                    base_revision: None,
                    causation_id: None,
                    mutation: interaction_frame::TypedSurfaceMutation::UnreadSummarySnapshot {
                        view: unread_snapshot.view,
                    },
                },
            ],
        )
    }

    /// Dialog 和 unread 是同一次 `InteractionFrame` 内的 paired slots。任一 slot 无法推进时，
    /// 整个 frame 不发布也不改本地行状态，等待重置后重新 attach。
    fn next_revision(&self) -> Option<u64> {
        self.revision
            .checked_add(1)
            .filter(|revision| *revision <= interaction_frame::MAX_SAFE_JSON_INTEGER)
            .or_else(|| {
                tracing::warn!(
                    surface = SURFACE,
                    revision = self.revision,
                    "dialog list revision exceeds the exact-safe wire range; suppressing frame until reset"
                );
                None
            })
    }

    fn ensure_unread_can_advance(&self) -> Option<()> {
        self.unread_summary
            .ensure_can_advance()
            .map_err(|error| {
                tracing::warn!(
                    surface = "message.unread-summary",
                    error = %error,
                    "unread revision overflow; suppressing paired dialog frame until reset"
                );
            })
            .ok()
    }

    fn emit_interaction_frame(
        &self,
        scope: &interaction_frame::SurfaceScope,
        mutations: Vec<interaction_frame::SurfaceMutation>,
    ) -> Option<Effect> {
        let scope_key = scope.canonical_key();
        let frame = crate::projector::build_interaction_frame(
            format!("dialog-list-{scope_key}-{}", self.revision),
            mutations,
        )
        .map_err(|error| {
            tracing::warn!(surface = SURFACE, error = %error, "dialog frame validation failed");
        })
        .ok()?;
        crate::projector::emit_interaction_frame(frame)
            .map_err(|error| {
                tracing::warn!(surface = SURFACE, error = %error, "dialog frame encoding failed");
            })
            .ok()
    }

    fn interaction_view(&self) -> interaction_frame::DialogListVm {
        let mut pinned = Vec::new();
        let mut normal = Vec::new();
        let mut closed = Vec::new();
        for channel_id in &self.ordered_channel_ids {
            let Some(row) = self.rows_by_channel.get(channel_id) else {
                continue;
            };
            let target = if row.state.closed || row.state.left {
                &mut closed
            } else if row.state.pinned {
                &mut pinned
            } else {
                &mut normal
            };
            target.push(dialog_row_to_surface_vm(row));
        }

        let mut sections = Vec::new();
        push_dialog_section(
            &mut sections,
            interaction_frame::DialogSection::Pinned,
            pinned,
            self.revision,
        );
        push_dialog_section(
            &mut sections,
            interaction_frame::DialogSection::Normal,
            normal,
            self.revision,
        );
        push_dialog_section(
            &mut sections,
            interaction_frame::DialogSection::Closed,
            closed,
            self.revision,
        );

        interaction_frame::DialogListVm {
            sections,
            state: interaction_frame::SurfaceLoadState::Ready,
        }
    }

    fn reposition(&mut self, channel_id: &str) -> usize {
        self.ordered_channel_ids.retain(|id| id != channel_id);
        let row = self.rows_by_channel.get(channel_id);
        let target = row.map_or(self.ordered_channel_ids.len(), |candidate| {
            self.ordered_channel_ids
                .iter()
                .position(|other_id| {
                    let other = self.rows_by_channel.get(other_id);
                    should_precede(candidate, other)
                })
                .unwrap_or(self.ordered_channel_ids.len())
        });
        self.ordered_channel_ids
            .insert(target, channel_id.to_string());
        target
    }
}

fn push_dialog_section(
    sections: &mut Vec<interaction_frame::DialogSectionVm>,
    kind: interaction_frame::DialogSection,
    items: Vec<interaction_frame::DialogRowVm>,
    revision: u64,
) {
    if items.is_empty() {
        return;
    }
    let suffix = match kind {
        interaction_frame::DialogSection::Pinned => "pinned",
        interaction_frame::DialogSection::Normal => "normal",
        interaction_frame::DialogSection::Closed => "closed",
    };
    sections.push(interaction_frame::DialogSectionVm {
        kind,
        items,
        page: interaction_frame::WindowPageVm {
            window_token: format!("dialog-list-{revision}-{suffix}"),
            has_older: false,
            has_newer: false,
            has_more: false,
        },
    });
}

fn dialog_row_to_surface_vm(row: &DialogListRowVm) -> interaction_frame::DialogRowVm {
    interaction_frame::DialogRowVm {
        channel_id: row.channel_id.clone(),
        channel_kind: match row.channel_kind.as_str() {
            "direct" => interaction_frame::ChannelKind::Direct,
            "topic" => interaction_frame::ChannelKind::Topic,
            _ => interaction_frame::ChannelKind::Group,
        },
        title: interaction_frame::ChannelTitleRef {
            text: row.title.clone(),
        },
        avatar: interaction_frame::ChannelAvatarRef {
            resource: row.avatar.image_ref.clone(),
            fallback_text: row.title.clone(),
        },
        company_tag: None,
        preview: dialog_preview_to_surface_vm(row.preview.as_ref()),
        last_activity_at: row.activity_at,
        unread_count: nonnegative_u32(row.badges.unread_count),
        mention_count: nonnegative_u32(row.badges.mention_count),
        urgent_count: nonnegative_u32(row.badges.pending_urgent_count),
        topic_count: row.topic_count.map_or(0, nonnegative_u32),
        has_scheduled_message: false,
        is_top: row.state.pinned,
        notification_mode: if row.state.muted {
            interaction_frame::NotificationMode::Muted
        } else {
            interaction_frame::NotificationMode::All
        },
        lifecycle: interaction_frame::ChannelLifecycleVm {
            state: if row.state.left {
                "left".to_string()
            } else if row.state.closed {
                "closed".to_string()
            } else {
                "active".to_string()
            },
            reason: None,
        },
        actions: Vec::new(),
    }
}

fn unread_contribution(row: &DialogListRowVm) -> ViewerChannelUnread {
    ViewerChannelUnread::new(
        nonnegative_u32(row.badges.unread_count),
        nonnegative_u32(row.badges.mention_count),
        nonnegative_u32(row.badges.pending_urgent_count),
    )
}

fn dialog_preview_to_surface_vm(preview: Option<&DialogPreviewVm>) -> interaction_frame::PreviewVm {
    let Some(preview) = preview else {
        return interaction_frame::PreviewVm::Empty;
    };
    let author = preview
        .sender_user_id
        .as_ref()
        .map(|user_id| interaction_frame::UserRefVm {
            user_id: user_id.clone(),
        });
    let text = preview.text.clone().unwrap_or_default();
    if preview.recalled {
        return interaction_frame::PreviewVm::Recalled {
            actor: author,
            text,
        };
    }
    if preview.system {
        return interaction_frame::PreviewVm::System { text };
    }
    match preview.kind.as_str() {
        "document" => interaction_frame::PreviewVm::Document {
            author,
            title: text,
        },
        "media" | "non-text" => interaction_frame::PreviewVm::Media {
            author,
            media_kind: "media".to_string(),
            label: text,
        },
        "forwarded" => interaction_frame::PreviewVm::Forwarded {
            author,
            summary: text,
        },
        "empty" => interaction_frame::PreviewVm::Empty,
        _ => interaction_frame::PreviewVm::Text { author, text },
    }
}

fn nonnegative_u32(value: i64) -> u32 {
    if value <= 0 {
        0
    } else if value > i64::from(u32::MAX) {
        u32::MAX
    } else {
        value as u32
    }
}

fn apply_channel_update_to_row(
    scope: &DialogListScope,
    value: &Value,
    previous: &DialogListRowVm,
    next: &mut DialogListRowVm,
    set: &mut DialogRowSet,
) {
    if let Some(title) = text(value, &["displayName", "display_name", "name"])
        .filter(|value| value != &previous.title)
    {
        next.title = title.clone();
        set.title = Some(title);
    }
    if has_any(
        value,
        &[
            "channelKind",
            "channel_kind",
            "channelType",
            "channel_type",
            "type",
        ],
    ) {
        let channel_kind = normalize_channel_kind(text(
            value,
            &[
                "channelKind",
                "channel_kind",
                "channelType",
                "channel_type",
                "type",
            ],
        ));
        if channel_kind != previous.channel_kind {
            next.channel_kind = channel_kind.clone();
            set.channel_kind = Some(channel_kind);
        }
    }
    if let Some(member_count) = integer(value, &["memberCount", "member_count"])
        .map(|value| value.max(0))
        .filter(|value| value != &previous.member_count)
    {
        next.member_count = member_count;
        set.member_count = Some(member_count);
    }
    if has_avatar_source(value) {
        let mut avatar = avatar_from_projection(value, &next.channel_kind);
        select_avatar_users_for_viewer(
            &mut avatar,
            &next.channel_kind,
            scope.viewer_user_id.as_str(),
        );
        if avatar != previous.avatar {
            next.avatar = avatar.clone();
            set.avatar = Some(avatar);
        }
    }
    let mut state = previous.state.clone();
    let mut changed_state = false;
    for (keys, assign) in [
        (&["isTop", "is_top", "top"][..], StateField::Pinned),
        (&["muted", "mute"][..], StateField::Muted),
        (&["online", "isOnline", "is_online"][..], StateField::Online),
        (&["closed", "deleteAt", "delete_at"][..], StateField::Closed),
        (&["left", "isRemove", "is_remove"][..], StateField::Left),
    ] {
        if let Some(value) = bool_value(value, keys) {
            changed_state |= assign_state_field(&mut state, assign, value);
        }
    }
    if changed_state {
        next.state = state.clone();
        set.state = Some(state);
    }
    if let Some(activity_at) = integer(
        value,
        &[
            "lastPostAt",
            "last_post_at",
            "lastActivityAt",
            "last_activity_at",
        ],
    )
    .filter(|value| value != &previous.activity_at)
    {
        next.activity_at = activity_at;
        set.activity_at = Some(activity_at);
    }
    if let Some(topic_count) = integer(
        value,
        &[
            "topicCount",
            "topic_count",
            "topicMsgCount",
            "topic_msg_count",
        ],
    )
    .map(|value| value.max(0))
    .filter(|value| previous.topic_count != Some(*value))
    {
        next.topic_count = Some(topic_count);
        set.topic_count = Some(topic_count);
    }
}

fn apply_post_projection_to_row(
    data: &Value,
    previous: &DialogListRowVm,
    next: &mut DialogListRowVm,
    set: &mut DialogRowSet,
) {
    let dialog_patch = data.get("dialogPatch").unwrap_or(data);
    let preview = preview_from_post_projection(data);
    let badges = DialogBadgesVm {
        unread_count: integer(dialog_patch, &["unreadCount", "unread_count"])
            .unwrap_or(previous.badges.unread_count),
        mention_count: integer(dialog_patch, &["mentionCount", "mention_count"])
            .unwrap_or(previous.badges.mention_count),
        pending_urgent_count: integer(dialog_patch, &["urgentCount", "urgent_count"])
            .unwrap_or(previous.badges.pending_urgent_count),
    };
    let activity_at = integer(
        data,
        &["lastPostAt", "last_post_at", "createdAt", "create_at"],
    )
    .or_else(|| preview.as_ref().and_then(|item| item.sent_at))
    .unwrap_or(previous.activity_at);

    next.preview = preview.clone().or(next.preview.clone());
    next.badges = badges.clone();
    next.activity_at = activity_at;
    set.preview = preview;
    set.badges = Some(badges);
    set.activity_at = Some(activity_at);
}

pub fn patch_from_dialog_projection(
    state: &mut DialogListFrameState,
    data: &Value,
) -> Option<Effect> {
    let scope = state.scope()?.clone();
    state.patch_from_post_projection(scope, data)
}

/// Converts one scan transaction into a complete dialog snapshot input.
///
/// A single row that cannot be represented by the v1 exact-safe wire contract
/// invalidates the whole scan. Dropping it would publish a false authoritative
/// list and silently remove a channel from an already attached surface.
pub fn rows_from_dialog_scan(reply_bytes: &[u8]) -> Option<Vec<DialogListRowVm>> {
    let raw: Value = serde_json::from_slice(reply_bytes).unwrap_or_else(|_| serde_json::json!([]));
    let Value::Array(rows) = raw else {
        return Some(Vec::new());
    };
    let mut projected = Vec::with_capacity(rows.len());
    for row in rows {
        // Render-ready conversion intentionally defaults malformed legacy fields in a few
        // places. Validate the raw row first so an unrepresentable persisted integer cannot
        // be mistaken for a missing value before this authoritative snapshot is committed.
        ensure_dialog_projection_numbers_safe(&row)?;
        let render_ready = crate::query::dialog_render::render_ready_dialog_row(row);
        projected.push(row_from_dialog_projection(&render_ready)?);
    }
    Some(projected)
}

pub fn row_from_dialog_projection(value: &Value) -> Option<DialogListRowVm> {
    ensure_dialog_projection_numbers_safe(value)?;
    let channel_id = text(value, &["channelId", "channel_id", "id"])?;
    let channel_kind = normalize_channel_kind(text(
        value,
        &[
            "channelKind",
            "channel_kind",
            "channelType",
            "channel_type",
            "type",
        ],
    ));
    let avatar = avatar_from_projection(value, &channel_kind);
    let preview = preview_from_dialog(value);
    let unread_count = integer(value, &["unread", "unreadCount", "unread_count"])
        .unwrap_or(0)
        .max(0);
    let mention_count = integer(
        value,
        &[
            "mentionCount",
            "mention_count",
            "mentionCountRoot",
            "mention_count_root",
        ],
    )
    .unwrap_or_else(|| bool_value(value, &["mention"]).map(i64::from).unwrap_or(0))
    .max(0);
    let pending_urgent_count = integer(value, &["urgentCount", "urgent_count"])
        .unwrap_or_else(|| bool_value(value, &["urgent"]).map(i64::from).unwrap_or(0))
        .max(0);
    let state = DialogStateVm {
        pinned: bool_value(value, &["isTop", "is_top", "top"]).unwrap_or(false),
        muted: bool_value(value, &["muted", "mute"]).unwrap_or(false),
        online: bool_value(value, &["online", "isOnline", "is_online"]),
        closed: bool_value(value, &["closed"]).unwrap_or(false),
        left: bool_value(value, &["left", "isRemove", "is_remove"]).unwrap_or(false),
    };
    let activity_at = integer(
        value,
        &[
            "lastPostAt",
            "last_post_at",
            "lastActivityAt",
            "last_activity_at",
            "createAt",
            "create_at",
        ],
    )
    .or_else(|| preview.as_ref().and_then(|item| item.sent_at))
    .unwrap_or(0);
    let row = DialogListRowVm {
        channel_id,
        title: text(value, &["displayName", "display_name", "name"]).unwrap_or_default(),
        channel_kind,
        member_count: integer(value, &["memberCount", "member_count"])
            .unwrap_or(0)
            .max(0),
        avatar,
        preview,
        badges: DialogBadgesVm {
            unread_count,
            mention_count,
            pending_urgent_count,
        },
        state,
        activity_at,
        topic_count: integer(value, &["topicCount", "topic_count"]).map(|value| value.max(0)),
    };
    ensure_dialog_row_wire_safe(&row)?;
    Some(row)
}

fn ensure_dialog_row_wire_safe(row: &DialogListRowVm) -> Option<()> {
    if row.channel_id.is_empty() {
        tracing::warn!(
            surface = SURFACE,
            "dialog row has an empty channelId; suppressing frame"
        );
        return None;
    }
    if !(interaction_frame::MIN_SAFE_JSON_INTEGER..=interaction_frame::MAX_SAFE_JSON_INTEGER as i64)
        .contains(&row.activity_at)
    {
        tracing::warn!(
            surface = SURFACE,
            channel_id = row.channel_id.as_str(),
            activity_at = row.activity_at,
            "dialog row lastActivityAt exceeds the exact-safe wire range; suppressing frame"
        );
        return None;
    }
    if row
        .preview
        .as_ref()
        .and_then(|preview| preview.sender_user_id.as_deref())
        .is_some_and(str::is_empty)
    {
        tracing::warn!(
            surface = SURFACE,
            channel_id = row.channel_id.as_str(),
            "dialog preview has an empty senderUserId; suppressing frame"
        );
        return None;
    }
    Some(())
}

/// `serde_json::Number::as_i64()` returns `None` for positive `u64` values
/// above `i64::MAX`. Treating that as a missing field lets a later default or
/// prior value hide an unrepresentable persisted number. Keep this preflight at
/// the raw projection boundary so no partial dialog Frame is ever published.
fn ensure_dialog_projection_numbers_safe(value: &Value) -> Option<()> {
    const INTEGER_FIELDS: &[&str] = &[
        "lastPostAt",
        "last_post_at",
        "lastActivityAt",
        "last_activity_at",
        "createAt",
        "create_at",
        "sentAt",
        "sent_at",
        "unread",
        "unreadCount",
        "unread_count",
        "mentionCount",
        "mention_count",
        "mentionCountRoot",
        "mention_count_root",
        "urgentCount",
        "urgent_count",
        "topicCount",
        "topic_count",
        "topicMsgCount",
        "topic_msg_count",
        "memberCount",
        "member_count",
        "deleteAt",
        "delete_at",
        "isTop",
        "is_top",
        "top",
        "muted",
        "mute",
        "online",
        "isOnline",
        "is_online",
        "closed",
        "left",
        "isRemove",
        "is_remove",
        "urgent",
        "isUrgent",
        "isExpedite",
    ];

    if !named_numbers_are_exact_safe(value, INTEGER_FIELDS) {
        tracing::warn!(
            surface = SURFACE,
            "dialog projection contains a number outside the exact-safe wire range; suppressing frame"
        );
        return None;
    }
    if let Some(dialog_patch) = value.get("dialogPatch") {
        if !named_numbers_are_exact_safe(dialog_patch, INTEGER_FIELDS) {
            tracing::warn!(
                surface = SURFACE,
                "dialog patch contains a number outside the exact-safe wire range; suppressing frame"
            );
            return None;
        }
    }
    if let Some(last_post) = embedded_object(value, &["lastPost", "last_post"]) {
        if !named_numbers_are_exact_safe(&last_post, INTEGER_FIELDS) {
            tracing::warn!(
                surface = SURFACE,
                "dialog preview contains a number outside the exact-safe wire range; suppressing frame"
            );
            return None;
        }
    }
    Some(())
}

fn named_numbers_are_exact_safe(value: &Value, keys: &[&str]) -> bool {
    keys.iter().all(|key| {
        value
            .get(*key)
            .is_none_or(number_is_exact_safe_or_not_an_integer)
    })
}

fn number_is_exact_safe_or_not_an_integer(value: &Value) -> bool {
    let parsed = match value {
        Value::Number(number) => number.as_i64(),
        Value::String(value) if looks_like_integer(value) => value.parse::<i64>().ok(),
        _ => return true,
    };
    parsed.is_some_and(|number| {
        (interaction_frame::MIN_SAFE_JSON_INTEGER..=interaction_frame::MAX_SAFE_JSON_INTEGER as i64)
            .contains(&number)
    })
}

fn looks_like_integer(value: &str) -> bool {
    let digits = value.strip_prefix('-').unwrap_or(value);
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn should_precede(candidate: &DialogListRowVm, other: Option<&DialogListRowVm>) -> bool {
    let Some(other) = other else {
        return false;
    };
    candidate.state.pinned && !other.state.pinned
        || (candidate.state.pinned == other.state.pinned
            && (candidate.activity_at > other.activity_at
                || (candidate.activity_at == other.activity_at
                    && candidate.channel_id < other.channel_id)))
}

#[derive(Clone, Copy)]
enum StateField {
    Pinned,
    Muted,
    Online,
    Closed,
    Left,
}

fn assign_state_field(state: &mut DialogStateVm, field: StateField, value: bool) -> bool {
    match field {
        StateField::Pinned if state.pinned != value => {
            state.pinned = value;
            true
        }
        StateField::Muted if state.muted != value => {
            state.muted = value;
            true
        }
        StateField::Online if state.online != Some(value) => {
            state.online = Some(value);
            true
        }
        StateField::Closed if state.closed != value => {
            state.closed = value;
            true
        }
        StateField::Left if state.left != value => {
            state.left = value;
            true
        }
        _ => false,
    }
}

fn set_is_empty(set: &DialogRowSet) -> bool {
    set.title.is_none()
        && set.channel_kind.is_none()
        && set.member_count.is_none()
        && set.avatar.is_none()
        && set.preview.is_none()
        && set.badges.is_none()
        && set.state.is_none()
        && set.activity_at.is_none()
        && set.topic_count.is_none()
}

fn has_any(value: &Value, keys: &[&str]) -> bool {
    keys.iter().any(|key| value.get(*key).is_some())
}

fn has_avatar_source(value: &Value) -> bool {
    has_any(
        value,
        &[
            "userIds",
            "user_ids",
            "memberIds",
            "member_ids",
            "members",
            "picture",
            "avatarUrl",
            "avatar_url",
            "pictureUrl",
            "picture_url",
        ],
    )
}

fn avatar_from_projection(value: &Value, channel_kind: &str) -> DialogAvatarVm {
    let picture = embedded_object(value, &["picture"]);
    // `pictureType: USER` carries the authoritative mosaic members in
    // `picture.userIds`; a channel's unrelated top-level member fields must
    // not replace that explicit avatar selection.
    let user_ids = picture
        .as_ref()
        .map(user_ids)
        .filter(|ids| !ids.is_empty())
        .unwrap_or_else(|| user_ids(value));
    let image_ref = text(
        value,
        &["avatarUrl", "avatar_url", "pictureUrl", "picture_url"],
    )
    .or_else(|| {
        picture.and_then(|picture| text(&picture, &["url", "uri", "imageRef", "image_ref"]))
    });
    DialogAvatarVm {
        kind: if image_ref.is_some() {
            "image".to_string()
        } else if channel_kind == "direct" || user_ids.len() == 1 {
            "single-user".to_string()
        } else if user_ids.is_empty() {
            "fallback".to_string()
        } else {
            "member-grid".to_string()
        },
        user_ids,
        image_ref,
    }
}

fn preview_from_dialog(value: &Value) -> Option<DialogPreviewVm> {
    let post = embedded_object(value, &["lastPost", "last_post"]);
    let preview_text = text(value, &["lastMessage", "last_message"]).or_else(|| {
        post.as_ref().and_then(|post| {
            text(
                post,
                &[
                    "message",
                    "content",
                    "text",
                    "simpleMessage",
                    "simple_message",
                ],
            )
        })
    });
    let post_type = post
        .as_ref()
        .and_then(|post| text(post, &["type", "msgType", "msg_type"]))
        .or_else(|| text(value, &["lastMessageType", "last_message_type"]));
    let urgent = bool_value(value, &["urgent"])
        .or_else(|| {
            post.as_ref()
                .and_then(|post| bool_value(post, &["isUrgent", "isExpedite", "urgent"]))
        })
        .unwrap_or(false);
    preview_text.as_ref()?;
    Some(DialogPreviewVm {
        source_id: post
            .as_ref()
            .and_then(|post| text(post, &["id", "msgId", "msg_id"])),
        text: preview_text,
        kind: preview_kind(post_type.as_deref()),
        sender_user_id: post.as_ref().and_then(|post| {
            text(
                post,
                &["userId", "user_id", "senderUserId", "sender_user_id"],
            )
        }),
        sent_at: post
            .as_ref()
            .and_then(|post| integer(post, &["createAt", "create_at", "sentAt", "sent_at"])),
        system: is_system_type(post_type.as_deref()),
        recalled: is_recalled_type(post_type.as_deref()),
        urgent,
    })
}

fn preview_from_post_projection(value: &Value) -> Option<DialogPreviewVm> {
    let preview_text = text(value, &["lastMessage", "last_message", "message", "text"]);
    let kind_source = text(value, &["messageClass", "type", "msgType", "msg_type"]);
    preview_text.as_ref()?;
    Some(DialogPreviewVm {
        source_id: text(value, &["msgId", "msg_id", "id"]),
        text: preview_text,
        kind: preview_kind(kind_source.as_deref()),
        sender_user_id: text(
            value,
            &["senderUserId", "sender_user_id", "userId", "user_id"],
        ),
        sent_at: integer(
            value,
            &["createAt", "create_at", "lastPostAt", "last_post_at"],
        ),
        system: is_system_type(kind_source.as_deref()),
        recalled: is_recalled_type(kind_source.as_deref()),
        urgent: bool_value(value, &["urgent"]).unwrap_or(false),
    })
}

fn user_ids(value: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    for key in [
        "userIds",
        "user_ids",
        "memberIds",
        "member_ids",
        "targetUsers",
        "target_users",
    ] {
        if let Some(items) = array_or_json_array(value, key) {
            for item in items {
                let id = item
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| text(&item, &["userId", "user_id", "id"]));
                if let Some(id) = id.filter(|id| !id.is_empty()) {
                    if !ids.iter().any(|existing| existing == &id) {
                        ids.push(id);
                    }
                }
            }
        }
    }
    if let Some(items) = value.get("members").and_then(Value::as_array) {
        for item in items {
            if let Some(id) = text(item, &["userId", "user_id", "id"]).filter(|id| !id.is_empty()) {
                if !ids.iter().any(|existing| existing == &id) {
                    ids.push(id);
                }
            }
        }
    }
    if let Some(items) = value.get("members").and_then(Value::as_object) {
        for (key, item) in items {
            let id = text(item, &["userId", "user_id", "id"]).unwrap_or_else(|| key.clone());
            if !id.is_empty() && !ids.iter().any(|existing| existing == &id) {
                ids.push(id);
            }
        }
    }
    if ids.is_empty() {
        if let Some(id) = text(value, &["userId", "user_id", "ownerId", "owner_id"]) {
            ids.push(id);
        }
    }
    ids
}

fn array_or_json_array(value: &Value, key: &str) -> Option<Vec<Value>> {
    value
        .get(key)
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            value
                .get(key)
                .and_then(Value::as_str)
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .and_then(|parsed| parsed.as_array().cloned())
        })
}

/// 头像成员选择是 Helix 的业务裁决：单聊只展示对端，拼图至多四人。
/// 客户端只将这里提供的身份映射为 `faceUri(userId)`，不得再过滤或重排。
fn select_avatar_users_for_viewer(
    avatar: &mut DialogAvatarVm,
    channel_kind: &str,
    viewer_user_id: &str,
) {
    let original = avatar.user_ids.clone();
    if channel_kind == "direct" {
        let peers: Vec<String> = original
            .iter()
            .filter(|id| id.as_str() != viewer_user_id)
            .take(1)
            .cloned()
            .collect();
        avatar.user_ids = if peers.is_empty() {
            original.into_iter().take(1).collect()
        } else {
            peers
        };
    } else {
        avatar.user_ids.truncate(4);
    }
}

fn embedded_object(value: &Value, keys: &[&str]) -> Option<Value> {
    for key in keys {
        let Some(item) = value.get(*key) else {
            continue;
        };
        if item.is_object() {
            return Some(item.clone());
        }
        if let Some(raw) = item.as_str() {
            if let Ok(parsed) = serde_json::from_str::<Value>(raw) {
                if parsed.is_object() {
                    return Some(parsed);
                }
            }
        }
    }
    None
}

fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            Value::String(value) if !value.is_empty() => Some(value.clone()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
    })
}

fn integer(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            Value::Number(value) => value.as_i64(),
            Value::String(value) => value.parse().ok(),
            _ => None,
        })
    })
}

fn bool_value(value: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            Value::Bool(value) => Some(*value),
            Value::Number(value) => value.as_i64().map(|value| value != 0),
            Value::String(value) if value == "1" || value.eq_ignore_ascii_case("true") => {
                Some(true)
            }
            Value::String(value) if value == "0" || value.eq_ignore_ascii_case("false") => {
                Some(false)
            }
            _ => None,
        })
    })
}

fn normalize_channel_kind(value: Option<String>) -> String {
    match value.as_deref().map(str::to_ascii_lowercase).as_deref() {
        Some("direct") | Some("single") | Some("private") | Some("d") => "direct".to_string(),
        Some("company") | Some("company_group") | Some("o") => "company".to_string(),
        Some("topic") | Some("subchannel") | Some("t") => "topic".to_string(),
        Some("group") | Some("channel") | Some("p") => "group".to_string(),
        _ => "unknown".to_string(),
    }
}

fn preview_kind(value: Option<&str>) -> String {
    if is_recalled_type(value) {
        "recalled".to_string()
    } else if is_system_type(value) {
        "system".to_string()
    } else if value
        .map(|value| value.eq_ignore_ascii_case("TEXT"))
        .unwrap_or(true)
    {
        "text".to_string()
    } else {
        "non-text".to_string()
    }
}

fn is_system_type(value: Option<&str>) -> bool {
    matches!(
        value.map(str::to_ascii_uppercase).as_deref(),
        Some("NOTICE") | Some("SYSTEM") | Some("SYSTEN")
    )
}

fn is_recalled_type(value: Option<&str>) -> bool {
    matches!(
        value.map(str::to_ascii_uppercase).as_deref(),
        Some("RECALL") | Some("REVOKE") | Some("DELETE")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> DialogListScope {
        DialogListScope::new("444", "team-a")
    }

    fn row(channel_id: &str, activity_at: i64) -> DialogListRowVm {
        DialogListRowVm {
            channel_id: channel_id.to_string(),
            title: channel_id.to_string(),
            channel_kind: "group".to_string(),
            member_count: 2,
            avatar: DialogAvatarVm {
                kind: "member-grid".to_string(),
                user_ids: vec!["u1".to_string()],
                image_ref: None,
            },
            preview: None,
            badges: DialogBadgesVm {
                unread_count: 0,
                mention_count: 0,
                pending_urgent_count: 0,
            },
            state: DialogStateVm {
                pinned: false,
                muted: false,
                online: None,
                closed: false,
                left: false,
            },
            activity_at,
            topic_count: None,
        }
    }

    fn data(effect: &Effect) -> Value {
        let Effect::Emit { event } = effect else {
            panic!("expected emit")
        };
        serde_json::from_slice(event.0.as_ref()).unwrap()
    }

    fn surface_mutation(frame: &Value) -> &Value {
        &frame["data"]["mutations"][0]
    }

    fn dialog_view(frame: &Value) -> &Value {
        let mutation = &surface_mutation(frame)["mutation"];
        mutation
            .get("view")
            .unwrap_or_else(|| &mutation["patch"]["view"])
    }

    #[test]
    fn snapshot_is_complete_render_ready_frame() {
        let mut state = DialogListFrameState::default();
        let snapshot = state
            .snapshot(scope(), vec![row("c2", 2), row("c1", 1)])
            .expect("initial revision is available");
        let frame = data(&snapshot);
        assert_eq!(frame["event"], EVENT_NAME);
        assert_eq!(frame["data"]["schemaVersion"], SCHEMA_VERSION);
        assert_eq!(surface_mutation(&frame)["surface"], SURFACE);
        assert_eq!(
            surface_mutation(&frame)["mutation"]["kind"],
            "dialogListSnapshot"
        );
        interaction_frame::InteractionFrame::decode(
            &serde_json::to_vec(&frame["data"]).expect("interaction frame JSON"),
        )
        .expect("live snapshot must satisfy the public InteractionFrame contract");
        assert_eq!(
            dialog_view(&frame)["sections"][0]["items"]
                .as_array()
                .expect("normal rows")
                .iter()
                .map(|row| row["channelId"].clone())
                .collect::<Vec<_>>(),
            vec![serde_json::json!("c2"), serde_json::json!("c1")]
        );
        assert_eq!(
            dialog_view(&frame)["sections"][0]["items"][0]["unreadCount"],
            0
        );
        let unread = &frame["data"]["mutations"][1];
        assert_eq!(unread["surface"], "message.unread-summary");
        assert_eq!(unread["mutation"]["kind"], "unreadSummarySnapshot");
        assert_eq!(unread["mutation"]["view"]["unreadTotal"], 0);
    }

    #[test]
    fn same_revision_in_different_user_scopes_has_distinct_transaction_ids() {
        let mut first = DialogListFrameState::default();
        let first_snapshot = first
            .snapshot(DialogListScope::new("user-a", "team-a"), vec![row("c1", 1)])
            .expect("initial revision is available");
        let first_frame = data(&first_snapshot);
        let mut second = DialogListFrameState::default();
        let second_snapshot = second
            .snapshot(DialogListScope::new("user-b", "team-a"), vec![row("c1", 1)])
            .expect("initial revision is available");
        let second_frame = data(&second_snapshot);

        assert_eq!(first_frame["data"]["mutations"][0]["revision"], 1);
        assert_eq!(second_frame["data"]["mutations"][0]["revision"], 1);
        assert_ne!(
            first_frame["data"]["transactionId"], second_frame["data"]["transactionId"],
            "PC recent-transaction cache is runtime-wide, so scope must participate in the id"
        );
    }

    #[test]
    fn revision_overflow_suppresses_the_paired_frame_without_mutating_slots() {
        let mut state = DialogListFrameState::default();
        state
            .snapshot(scope(), vec![row("c1", 1)])
            .expect("initial revision is available");
        state.revision = interaction_frame::MAX_SAFE_JSON_INTEGER - 1;
        state
            .snapshot(scope(), vec![row("c1", 2)])
            .expect("the exact-safe maximum itself remains representable");
        assert_eq!(state.revision, interaction_frame::MAX_SAFE_JSON_INTEGER);
        let before_rows = state.rows_by_channel.clone();
        let before_order = state.ordered_channel_ids.clone();
        let before_unread = state.unread_summary.snapshot_view();

        assert!(
            state.snapshot(scope(), vec![row("c2", 2)]).is_none(),
            "overflow must not create a wrapped dialog snapshot"
        );
        assert_eq!(state.revision, interaction_frame::MAX_SAFE_JSON_INTEGER);
        assert_eq!(state.rows_by_channel, before_rows);
        assert_eq!(state.ordered_channel_ids, before_order);
        assert_eq!(state.unread_summary.snapshot_view(), before_unread);

        state.revision = 1;
        state.unread_summary.set_revision_for_test(u64::MAX);
        assert!(
            state
                .patch_from_post_projection(
                    scope(),
                    &serde_json::json!({
                        "channelId": "c1",
                        "lastMessage": "must not commit",
                        "lastPostAt": 2,
                        "dialogPatch": { "unreadCount": 1 }
                    }),
                )
                .is_none(),
            "unread overflow must suppress the paired dialog patch"
        );
        assert_eq!(state.revision, 1);
        assert_eq!(state.rows_by_channel, before_rows);
    }

    #[test]
    fn unsafe_persisted_activity_is_rejected_before_dialog_slots_mutate() {
        let unsafe_activity = interaction_frame::MAX_SAFE_JSON_INTEGER as i64 + 1;
        let mut state = DialogListFrameState::default();

        assert!(row_from_dialog_projection(&serde_json::json!({
            "channelId": "unsafe-row",
            "lastPostAt": unsafe_activity,
        }))
        .is_none());
        assert!(row_from_dialog_projection(&serde_json::json!({
            "channelId": "unsafe-u64-row",
            "lastPostAt": u64::MAX,
        }))
        .is_none());
        assert!(state
            .snapshot(scope(), vec![row("unsafe-row", unsafe_activity)])
            .is_none());
        assert_eq!(state.revision, 0);
        assert!(state.scope().is_none());
        assert!(state.rows_by_channel.is_empty());
        assert!(state.unread_summary.snapshot_view().is_none());

        state
            .snapshot(scope(), vec![row("c1", 1)])
            .expect("valid snapshot attaches the slot");
        let before_rows = state.rows_by_channel.clone();
        let before_order = state.ordered_channel_ids.clone();
        let before_unread = state.unread_summary.snapshot_view();

        assert!(state
            .patch_from_post_projection(
                scope(),
                &serde_json::json!({
                    "channelId": "c1",
                    "lastPostAt": unsafe_activity,
                    "dialogPatch": { "unreadCount": 8 }
                }),
            )
            .is_none());
        assert_eq!(state.revision, 1);
        assert_eq!(state.rows_by_channel, before_rows);
        assert_eq!(state.ordered_channel_ids, before_order);
        assert_eq!(state.unread_summary.snapshot_view(), before_unread);
    }

    #[test]
    fn post_patch_uses_absolute_badges_and_moves_row_atomically() {
        let mut state = DialogListFrameState::default();
        state.snapshot(scope(), vec![row("c2", 2), row("c1", 1)]);
        let frame = state
            .patch_from_post_projection(
                scope(),
                &serde_json::json!({
                    "channelId": "c1",
                    "msgId": "post-1",
                    "lastMessage": "hello",
                    "senderUserId": "u1",
                    "lastPostAt": 3,
                    "dialogPatch": { "unreadCount": 4, "mentionCount": 1, "urgentCount": 2 }
                }),
            )
            .expect("known row gets patch");
        let data = data(&frame);
        assert_eq!(surface_mutation(&data)["baseRevision"], 1);
        assert_eq!(
            surface_mutation(&data)["mutation"]["kind"],
            "dialogListPatch"
        );
        assert_eq!(
            surface_mutation(&data)["mutation"]["patch"]["kind"],
            "replaceView"
        );
        assert_eq!(
            dialog_view(&data)["sections"][0]["items"][0]["unreadCount"],
            4
        );
        assert_eq!(
            dialog_view(&data)["sections"][0]["items"][0]["channelId"],
            "c1"
        );
        let unread = &data["data"]["mutations"][1];
        assert_eq!(unread["surface"], "message.unread-summary");
        assert_eq!(unread["baseRevision"], 1);
        assert_eq!(unread["revision"], 2);
        assert_eq!(unread["mutation"]["kind"], "unreadSummaryPatch");
        assert_eq!(unread["mutation"]["patch"]["view"]["unreadTotal"], 4);
    }

    #[test]
    fn snapshot_owns_direct_peer_selection_and_group_avatar_bounds() {
        let direct = row_from_dialog_projection(&serde_json::json!({
            "id": "direct",
            "type": "D",
            "targetUsers": "[\"444\",\"peer\"]"
        }))
        .expect("direct channel row");
        let mut group = row("group", 1);
        group.avatar.user_ids = vec![
            "u1".to_string(),
            "u2".to_string(),
            "u3".to_string(),
            "u4".to_string(),
            "u5".to_string(),
        ];

        let mut state = DialogListFrameState::default();
        let snapshot = state
            .snapshot(scope(), vec![direct, group])
            .expect("initial revision is available");
        let frame = data(&snapshot);
        let rows = dialog_view(&frame)["sections"][0]["items"]
            .as_array()
            .unwrap();

        assert_eq!(rows[0]["channelKind"], "direct");
        assert_eq!(rows[0]["avatar"]["fallbackText"], "");
        assert_eq!(rows[1]["avatar"]["fallbackText"], "group");
    }

    #[test]
    fn user_picture_owns_group_avatar_members_and_channel_patch() {
        let value = serde_json::json!({
            "id": "group-picture-users",
            "type": "P",
            "pictureType": "USER",
            "picture": { "userIds": ["444", "u1", "u2", "u3", "u4"] }
        });
        let group_row = row_from_dialog_projection(&value).expect("group row");
        assert_eq!(group_row.avatar.kind, "member-grid");
        assert_eq!(
            group_row.avatar.user_ids,
            vec!["444", "u1", "u2", "u3", "u4"]
        );

        let mut snapshot_state = DialogListFrameState::default();
        let snapshot_effect = snapshot_state
            .snapshot(scope(), vec![group_row])
            .expect("initial revision is available");
        let snapshot = data(&snapshot_effect);
        let snapshot_avatar = &dialog_view(&snapshot)["sections"][0]["items"][0]["avatar"];
        assert_eq!(snapshot_avatar["resource"], Value::Null);
        assert_eq!(snapshot_avatar["fallbackText"], "");

        let mut state = DialogListFrameState::default();
        state.snapshot(scope(), vec![row("group-picture-users", 1)]);
        let patch = data(
            &state
                .patch_from_channel_update(scope(), &value)
                .expect("picture update patches the avatar"),
        );
        let avatar = &dialog_view(&patch)["sections"][0]["items"][0]["avatar"];
        assert_eq!(avatar["resource"], Value::Null);
        assert_eq!(avatar["fallbackText"], "group-picture-users");
    }

    #[test]
    fn channel_patch_preserves_false_and_zero_as_authoritative_values() {
        let mut state = DialogListFrameState::default();
        let mut initial = row("c1", 1);
        initial.member_count = 2;
        initial.state.pinned = true;
        initial.state.muted = true;
        state.snapshot(scope(), vec![initial]);

        let frame = state
            .patch_from_channel_update(
                scope(),
                &serde_json::json!({
                    "channelId": "c1",
                    "memberCount": 0,
                    "isTop": false,
                    "muted": false
                }),
            )
            .expect("explicit false and zero must be emitted");
        let data = data(&frame);
        let view = dialog_view(&data);

        assert_eq!(view["sections"][0]["items"][0]["isTop"], false);
        assert_eq!(view["sections"][0]["items"][0]["notificationMode"], "all");
    }

    #[test]
    fn combined_channel_and_member_dialog_update_emits_one_atomic_frame() {
        let mut state = DialogListFrameState::default();
        state.snapshot(scope(), vec![row("c1", 1)]);

        let frame = state
            .patch_from_channel_and_dialog_projection(
                scope(),
                &serde_json::json!({
                    "channelId": "c1",
                    "displayName": "renamed",
                    "isTop": true
                }),
                &serde_json::json!({
                    "channelId": "c1",
                    "msgId": "post-1",
                    "lastMessage": "member absolute",
                    "senderUserId": "u1",
                    "lastPostAt": 2,
                    "dialogPatch": {
                        "unreadCount": 3,
                        "mentionCount": 1,
                        "urgentCount": 1
                    }
                }),
            )
            .expect("known row gets one combined patch");
        let frame_data = data(&frame);
        let mutation = surface_mutation(&frame_data);
        let view = dialog_view(&frame_data);
        let row = &view["sections"][0]["items"][0];

        assert_eq!(mutation["baseRevision"], 1);
        assert_eq!(mutation["revision"], 2);
        assert_eq!(mutation["mutation"]["kind"], "dialogListPatch");
        assert_eq!(mutation["mutation"]["patch"]["kind"], "replaceView");
        assert_eq!(frame_data["data"]["mutations"].as_array().unwrap().len(), 2);
        let unread = &frame_data["data"]["mutations"][1];
        assert_eq!(unread["surface"], "message.unread-summary");
        assert_eq!(unread["mutation"]["patch"]["view"]["unreadTotal"], 3);
        assert_eq!(unread["mutation"]["patch"]["view"]["mentionTotal"], 1);
        assert_eq!(unread["mutation"]["patch"]["view"]["urgentTotal"], 1);
        assert_eq!(row["title"]["text"], "renamed");
        assert_eq!(row["isTop"], true);
        assert_eq!(row["preview"]["text"], "member absolute");
        assert_eq!(row["unreadCount"], 3);
        interaction_frame::InteractionFrame::decode(
            &serde_json::to_vec(&frame_data["data"]).expect("interaction frame JSON"),
        )
        .expect("live post/member frame must satisfy the public InteractionFrame contract");
    }
}
