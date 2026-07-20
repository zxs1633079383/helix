//! 左侧会话列表的跨端 render-ready 投影。
//!
//! 这个模块把持久化 channel 行与写后 DialogPatch 转换为唯一的 UI ViewModel。
//! 客户端只验证 scope/revision 后原子替换或应用属性 patch，不解析 post、计算 badge 或重排。

use std::collections::HashMap;

use bytes::Bytes;
use helix_core::effect::DomainEventBytes;
use helix_core::Effect;
use serde::Serialize;
use serde_json::Value;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogListSnapshot {
    pub rows: Vec<DialogListRowVm>,
    pub ordered_channel_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogRowSet {
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
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DialogListPatchOperation {
    UpsertRow {
        channel_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        row: Option<DialogListRowVm>,
        set: DialogRowSet,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        clear: Vec<String>,
    },
    RemoveRow {
        channel_id: String,
    },
    MoveRow {
        channel_id: String,
        to_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogListPatch {
    pub operations: Vec<DialogListPatchOperation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DialogListFrame {
    pub schema_version: u32,
    pub surface: &'static str,
    pub frame_id: String,
    pub scope: DialogListScope,
    pub mode: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_revision: Option<u64>,
    pub revision: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot: Option<DialogListSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<DialogListPatch>,
}

/// 当前 scope 的最后已发布列表。它只保存 render-ready VM，绝不反向读取 storage。
#[derive(Debug, Default)]
pub struct DialogListFrameState {
    revision: u64,
    scope: Option<DialogListScope>,
    rows_by_channel: HashMap<String, DialogListRowVm>,
    ordered_channel_ids: Vec<String>,
}

impl DialogListFrameState {
    pub fn scope(&self) -> Option<&DialogListScope> {
        self.scope.as_ref()
    }

    pub fn snapshot(&mut self, scope: DialogListScope, rows: Vec<DialogListRowVm>) -> Effect {
        self.revision = self.revision.wrapping_add(1);
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
        let snapshot = DialogListSnapshot {
            rows: self
                .ordered_channel_ids
                .iter()
                .filter_map(|channel_id| self.rows_by_channel.get(channel_id).cloned())
                .collect(),
            ordered_channel_ids: self.ordered_channel_ids.clone(),
        };
        emit_frame(DialogListFrame {
            schema_version: SCHEMA_VERSION,
            surface: SURFACE,
            frame_id: format!("dialog-list-{}", self.revision),
            scope,
            mode: "snapshot",
            base_revision: None,
            revision: self.revision,
            causation_id: None,
            snapshot: Some(snapshot),
            patch: None,
        })
    }

    /// 在写后投影已经提供绝对累计值时，仅改变这条 row 的相关属性并一次性发布。
    /// 未完成 snapshot 的 channel 不发 patch，继续让旧投影承担过渡展示，避免制造不可应用的增量。
    pub fn patch_from_post_projection(
        &mut self,
        scope: DialogListScope,
        data: &Value,
    ) -> Option<Effect> {
        if !scope.is_complete() {
            return None;
        }
        let channel_id = text(data, &["channelId", "channel_id"])?;
        let previous = self.rows_by_channel.get(&channel_id)?.clone();
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
        .or_else(|| preview.as_ref().and_then(|value| value.sent_at))
        .unwrap_or(previous.activity_at);
        let mut next = previous.clone();
        next.preview = preview.clone().or(previous.preview);
        next.badges = badges.clone();
        next.activity_at = activity_at;

        let base_revision = self.revision;
        self.revision = self.revision.wrapping_add(1);
        self.rows_by_channel.insert(channel_id.clone(), next);
        let mut operations = vec![DialogListPatchOperation::UpsertRow {
            channel_id: channel_id.clone(),
            row: None,
            set: DialogRowSet {
                title: None,
                channel_kind: None,
                member_count: None,
                avatar: None,
                preview,
                badges: Some(badges),
                state: None,
                activity_at: Some(activity_at),
                topic_count: None,
            },
            clear: Vec::new(),
        }];
        let old_index = self
            .ordered_channel_ids
            .iter()
            .position(|id| id == &channel_id)?;
        let next_index = self.reposition(&channel_id);
        if old_index != next_index {
            operations.push(DialogListPatchOperation::MoveRow {
                channel_id,
                to_index: next_index,
            });
        }
        Some(emit_frame(DialogListFrame {
            schema_version: SCHEMA_VERSION,
            surface: SURFACE,
            frame_id: format!("dialog-list-{}", self.revision),
            scope,
            mode: "patch",
            base_revision: Some(base_revision),
            revision: self.revision,
            causation_id: None,
            snapshot: None,
            patch: Some(DialogListPatch { operations }),
        }))
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
        let channel_id = text(value, &["channelId", "channel_id", "id"])?;
        let previous = self.rows_by_channel.get(&channel_id)?.clone();
        let mut next = previous.clone();
        let mut set = DialogRowSet::default();
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
        if set_is_empty(&set) {
            return None;
        }
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
        if self.rows_by_channel.contains_key(&row.channel_id) {
            return self.patch_from_channel_update(scope, value);
        }
        let base_revision = self.revision;
        self.revision = self.revision.wrapping_add(1);
        let channel_id = row.channel_id.clone();
        let before_len = self.ordered_channel_ids.len();
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
        Some(self.emit_patch(scope, base_revision, operations))
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
        let old_index = self
            .ordered_channel_ids
            .iter()
            .position(|id| id == &channel_id)?;
        let base_revision = self.revision;
        self.revision = self.revision.wrapping_add(1);
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
        Some(self.emit_patch(scope, base_revision, operations))
    }

    fn emit_patch(
        &self,
        scope: DialogListScope,
        base_revision: u64,
        operations: Vec<DialogListPatchOperation>,
    ) -> Effect {
        emit_frame(DialogListFrame {
            schema_version: SCHEMA_VERSION,
            surface: SURFACE,
            frame_id: format!("dialog-list-{}", self.revision),
            scope,
            mode: "patch",
            base_revision: Some(base_revision),
            revision: self.revision,
            causation_id: None,
            snapshot: None,
            patch: Some(DialogListPatch { operations }),
        })
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

pub fn patch_from_legacy_dialog_effect(
    state: &mut DialogListFrameState,
    effect: &Effect,
) -> Option<Effect> {
    let scope = state.scope()?.clone();
    let Effect::Emit { event } = effect else {
        return None;
    };
    let payload: Value = serde_json::from_slice(event.0.as_ref()).ok()?;
    state.patch_from_post_projection(scope, payload.get("data")?)
}

pub fn rows_from_dialog_scan(reply_bytes: &[u8]) -> Vec<DialogListRowVm> {
    let raw: Value = serde_json::from_slice(reply_bytes).unwrap_or_else(|_| serde_json::json!([]));
    let Value::Array(rows) = raw else {
        return Vec::new();
    };
    rows.into_iter()
        .map(crate::query::dialog_render::render_ready_dialog_row)
        .filter_map(|row| row_from_dialog_projection(&row))
        .collect()
}

pub fn row_from_dialog_projection(value: &Value) -> Option<DialogListRowVm> {
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
    Some(DialogListRowVm {
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
    })
}

fn emit_frame(frame: DialogListFrame) -> Effect {
    let payload = serde_json::json!({ "event": EVENT_NAME, "data": frame });
    Effect::Emit {
        event: DomainEventBytes(Bytes::from(
            serde_json::to_vec(&payload).expect("dialog list frame is serializable"),
        )),
    }
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
    let user_ids = user_ids(value);
    let image_ref = text(
        value,
        &["avatarUrl", "avatar_url", "pictureUrl", "picture_url"],
    )
    .or_else(|| {
        embedded_object(value, &["picture"])
            .and_then(|picture| text(&picture, &["url", "uri", "imageRef", "image_ref"]))
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

    const CANONICAL_FIXTURE: &str = include_str!("../tests/fixtures/dialog_list_frame_v1.json");

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

    #[test]
    fn snapshot_is_complete_render_ready_frame() {
        let mut state = DialogListFrameState::default();
        let frame = data(&state.snapshot(scope(), vec![row("c2", 2), row("c1", 1)]));
        assert_eq!(frame["event"], EVENT_NAME);
        assert_eq!(frame["data"]["mode"], "snapshot");
        assert_eq!(
            frame["data"]["snapshot"]["orderedChannelIds"],
            serde_json::json!(["c2", "c1"])
        );
        assert_eq!(
            frame["data"]["snapshot"]["rows"][0]["badges"]["unreadCount"],
            0
        );
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
        assert_eq!(data["data"]["baseRevision"], 1);
        assert_eq!(
            data["data"]["patch"]["operations"][0]["set"]["badges"]["unreadCount"],
            4
        );
        assert_eq!(data["data"]["patch"]["operations"][1]["kind"], "moveRow");
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
        let frame = data(&state.snapshot(scope(), vec![direct, group]));
        let rows = frame["data"]["snapshot"]["rows"].as_array().unwrap();

        assert_eq!(rows[0]["channelKind"], "direct");
        assert_eq!(rows[0]["avatar"]["userIds"], serde_json::json!(["peer"]));
        assert_eq!(
            rows[1]["avatar"]["userIds"],
            serde_json::json!(["u1", "u2", "u3", "u4"])
        );
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
        let set = &data(&frame)["data"]["patch"]["operations"][0]["set"];

        assert_eq!(set["memberCount"], 0);
        assert_eq!(set["state"]["pinned"], false);
        assert_eq!(set["state"]["muted"], false);
    }

    #[test]
    fn canonical_fixture_covers_snapshot_atomic_patch_and_gap_recovery() {
        let fixture: Value =
            serde_json::from_str(CANONICAL_FIXTURE).expect("valid canonical fixture");
        assert_eq!(fixture["schemaVersion"], SCHEMA_VERSION);
        assert_eq!(fixture["surface"], SURFACE);
        assert_eq!(fixture["scope"]["viewerUserId"], "444");
        assert_eq!(fixture["scope"]["teamId"], "team-a");

        let cases = fixture["cases"].as_array().expect("fixture cases");
        assert_eq!(cases[0]["mode"], "snapshot");
        assert_eq!(
            cases[0]["orderedChannelIds"],
            serde_json::json!(["group-1", "direct-1"])
        );
        assert_eq!(cases[1]["baseRevision"], 1);
        assert_eq!(cases[1]["revision"], 2);
        assert_eq!(cases[1]["operations"][0]["set"]["badges"]["unreadCount"], 0);
        assert_eq!(
            cases[1]["operations"][0]["clear"],
            serde_json::json!(["topicCount"])
        );
        assert_eq!(cases[2]["expected"], "reject-and-request-snapshot");
    }
}
