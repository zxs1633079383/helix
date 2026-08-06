//! MessageV3 时间线窗口的纯业务状态。
//!
//! 本模块只保存分页、定位与已附着窗口事实，不构造平台 UI schema 或 action binding。

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

/// 单个时间线窗口最多保留的消息数。
pub const MAX_TIMELINE_WINDOW_ITEMS: usize = 60;
/// 时间线查询默认页长。
pub const DEFAULT_TIMELINE_PAGE_SIZE: u32 = 40;
/// 时间线查询最大页长。
pub const MAX_TIMELINE_PAGE_SIZE: u32 = MAX_TIMELINE_WINDOW_ITEMS as u32;

/// 校验后的时间线页长。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelinePageSize(u32);

impl TimelinePageSize {
    /// 只接受 1..=60 的页长。
    pub fn new(value: u32) -> Result<Self, TimelineStateError> {
        if (1..=MAX_TIMELINE_PAGE_SIZE).contains(&value) {
            Ok(Self(value))
        } else {
            Err(TimelineStateError::InvalidPageSize(value))
        }
    }

    /// 从可选 JSON 值解析页长，缺失时使用默认值。
    pub fn parse(value: Option<&Value>) -> Result<Self, TimelineStateError> {
        let Some(value) = value else {
            return Ok(Self(DEFAULT_TIMELINE_PAGE_SIZE));
        };
        let raw = value
            .as_u64()
            .ok_or(TimelineStateError::PageSizeNotUnsigned)?;
        let raw = u32::try_from(raw).map_err(|_| TimelineStateError::PageSizeTooLarge(raw))?;
        Self::new(raw)
    }

    /// 返回已校验页长。
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

/// 一条消息的稳定分页键。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineEntityKey {
    pub create_at: i64,
    pub temporary_id: String,
}

/// 时间线页边界。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowPage {
    pub window_token: String,
    pub has_older: bool,
    pub has_newer: bool,
    pub has_more: bool,
}

/// 时间线锚点模式。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TimelineAnchorMode {
    #[default]
    Latest,
    Locate,
}

/// 时间线锚点事实。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TimelineAnchor {
    pub message_id: Option<String>,
    pub mode: TimelineAnchorMode,
}

/// 一条窗口消息的最小稳定事实。
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineItem {
    pub id: String,
    pub created_at: i64,
    pub temporary_id: String,
    pub row: Value,
}

/// 当前附着窗口的业务状态。
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineWindow {
    pub channel_id: String,
    pub page: WindowPage,
    pub anchor: TimelineAnchor,
    pub items: Vec<TimelineItem>,
}

/// 时间线窗口的唯一业务作用域。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineScope {
    pub channel_id: String,
    pub window_token: String,
}

impl TimelineScope {
    /// 返回只用于内核索引的稳定键。
    fn key(&self) -> String {
        format!("{}\u{1f}{}", self.channel_id, self.window_token)
    }
}

/// 远端页相对当前窗口的合并方式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimelinePageMutation {
    Older,
    Newer,
    Locate {
        target_message_id: String,
        navigation_token: String,
        activate: bool,
    },
}

/// 构造窗口状态所需的确定性输入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineWindowRequest {
    pub channel_id: String,
    pub window_token: String,
    pub page: WindowPage,
    pub target_message_id: Option<String>,
    pub page_size: u32,
}

impl TimelineWindowRequest {
    /// 为频道建立 latest 窗口请求。
    pub fn latest(channel_id: impl Into<String>) -> Self {
        Self::latest_with_window_token(channel_id, "latest")
    }

    /// 为频道建立带指定 token 的 latest 窗口请求。
    pub fn latest_with_window_token(
        channel_id: impl Into<String>,
        window_token: impl Into<String>,
    ) -> Self {
        let window_token = window_token.into();
        Self {
            channel_id: channel_id.into(),
            page: WindowPage {
                window_token: window_token.clone(),
                ..WindowPage::default()
            },
            window_token,
            target_message_id: None,
            page_size: DEFAULT_TIMELINE_PAGE_SIZE,
        }
    }
}

/// 时间线状态错误。
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TimelineStateError {
    #[error("pageSize must be an unsigned integer")]
    PageSizeNotUnsigned,
    #[error("pageSize {0} is outside 1..=60")]
    InvalidPageSize(u32),
    #[error("pageSize {0} cannot be represented")]
    PageSizeTooLarge(u64),
    #[error("timeline row is missing id")]
    MissingId,
    #[error("timeline window is stale or unattached: channel_id={channel_id}, window_token={window_token}")]
    StaleWindow {
        channel_id: String,
        window_token: String,
    },
}

/// 跨 Tick 保存时间线分页与定位状态。
#[derive(Debug, Default)]
pub struct TimelineState {
    slots: BTreeMap<String, TimelineWindow>,
    attachments: BTreeMap<String, (String, String)>,
    navigation_tokens: BTreeMap<String, String>,
    active_window_by_channel: BTreeMap<String, String>,
}

impl TimelineState {
    /// 清除登录身份绑定的全部窗口状态。
    pub fn reset(&mut self) {
        self.slots.clear();
        self.attachments.clear();
        self.navigation_tokens.clear();
        self.active_window_by_channel.clear();
    }

    /// 注册一个已附着窗口。
    pub fn register_attachment(&mut self, scope: &TimelineScope) {
        self.retire_other_windows(scope.channel_id.as_str(), scope.window_token.as_str());
        self.attachments.insert(
            scope.key(),
            (scope.channel_id.clone(), scope.window_token.clone()),
        );
        self.active_window_by_channel
            .insert(scope.channel_id.clone(), scope.window_token.clone());
    }

    /// 判断窗口是否已附着。
    pub fn is_attached(&self, scope: &TimelineScope) -> bool {
        self.attachments.contains_key(&scope.key())
    }

    /// 判断窗口是否仍是频道当前唯一活动窗口。
    pub fn is_active_window(&self, channel_id: &str, window_token: &str) -> bool {
        let scope = TimelineScope {
            channel_id: channel_id.to_string(),
            window_token: window_token.to_string(),
        };
        self.active_window_by_channel
            .get(channel_id)
            .is_some_and(|active| active == window_token)
            && self.is_attached(&scope)
            && self.current_view(&scope).is_some()
    }

    /// 返回日志所需的内存窗口与 attachment 数量，不暴露消息内容。
    pub fn window_counts(&self) -> (usize, usize) {
        (self.slots.len(), self.attachments.len())
    }

    /// 返回当前窗口状态。
    pub fn current_view(&self, scope: &TimelineScope) -> Option<&TimelineWindow> {
        self.slots.get(&scope.key())
    }

    /// 返回包含目标消息的唯一频道。
    pub fn located_target_channel(&self, message_id: &str) -> Option<&str> {
        let mut matches = self
            .slots
            .iter()
            .filter(|(key, slot)| {
                let active_key = self
                    .active_window_by_channel
                    .get(slot.channel_id.as_str())
                    .map(|active| format!("{}\u{1f}{active}", slot.channel_id));
                active_key.as_deref() == Some(key.as_str())
                    && slot.items.iter().any(|item| item.id == message_id)
            })
            .map(|(_, slot)| slot);
        let first = matches.next()?;
        matches
            .next()
            .is_none()
            .then_some(first.channel_id.as_str())
    }

    /// 返回频道唯一已附着窗口及当前行数。
    pub fn unique_attached_window_for_channel(&self, channel_id: &str) -> Option<(String, usize)> {
        let token = self.active_window_by_channel.get(channel_id)?.clone();
        let scope = TimelineScope {
            channel_id: channel_id.to_string(),
            window_token: token.clone(),
        };
        if !self.is_active_window(channel_id, token.as_str()) {
            return None;
        }
        Some((token, self.current_view(&scope)?.items.len()))
    }

    /// 返回锚点所在窗口的页事实与稳定复合游标。
    pub fn paging_context_for_anchor(
        &self,
        channel_id: &str,
        anchor_message_id: &str,
    ) -> Option<(WindowPage, usize, TimelineEntityKey)> {
        let window_token = self.active_window_by_channel.get(channel_id)?;
        let scope = TimelineScope {
            channel_id: channel_id.to_string(),
            window_token: window_token.clone(),
        };
        let slot = self
            .is_active_window(channel_id, window_token.as_str())
            .then(|| self.current_view(&scope))??;
        let message = slot
            .items
            .iter()
            .find(|item| item.id == anchor_message_id)?;
        (!message.temporary_id.is_empty()).then_some((
            slot.page.clone(),
            slot.items.len(),
            TimelineEntityKey {
                create_at: message.created_at,
                temporary_id: message.temporary_id.clone(),
            },
        ))
    }

    /// 把一次定位请求登记为窗口当前导航。
    pub fn begin_locate_navigation(
        &mut self,
        channel_id: &str,
        window_token: &str,
        navigation_token: &str,
    ) {
        let scope = TimelineScope {
            channel_id: channel_id.to_string(),
            window_token: window_token.to_string(),
        };
        self.navigation_tokens
            .insert(scope.key(), navigation_token.to_string());
    }

    /// 判断定位回包是否仍对应窗口当前导航。
    pub fn is_current_locate_navigation(
        &self,
        channel_id: &str,
        window_token: &str,
        navigation_token: &str,
    ) -> bool {
        let scope = TimelineScope {
            channel_id: channel_id.to_string(),
            window_token: window_token.to_string(),
        };
        self.navigation_tokens
            .get(&scope.key())
            .is_some_and(|current| current == navigation_token)
    }

    /// 用权威行替换窗口并自动登记 attachment。
    pub fn replace(
        &mut self,
        request: TimelineWindowRequest,
        rows: &[Value],
    ) -> Result<(), TimelineStateError> {
        let old_window_token = self
            .active_window_by_channel
            .get(request.channel_id.as_str())
            .cloned();
        let scope = TimelineScope {
            channel_id: request.channel_id.clone(),
            window_token: request.window_token.clone(),
        };
        let items = rows
            .iter()
            .map(timeline_item)
            .collect::<Result<Vec<_>, _>>()?;
        let channel_prefix = format!("{}\u{1f}", scope.channel_id);
        self.navigation_tokens
            .retain(|key, _| !key.starts_with(channel_prefix.as_str()));
        self.slots.insert(
            scope.key(),
            TimelineWindow {
                channel_id: request.channel_id,
                page: request.page,
                anchor: TimelineAnchor {
                    message_id: request.target_message_id.clone(),
                    mode: if request.target_message_id.is_some() {
                        TimelineAnchorMode::Locate
                    } else {
                        TimelineAnchorMode::Latest
                    },
                },
                items,
            },
        );
        self.register_attachment(&scope);
        tracing::debug!(
            channel_id = scope.channel_id.as_str(),
            old_window_token = old_window_token.as_deref().unwrap_or_default(),
            new_window_token = scope.window_token.as_str(),
            active_window_token = scope.window_token.as_str(),
            slot_count = self.slots.len(),
            attachment_count = self.attachments.len(),
            "timeline active window replaced"
        );
        Ok(())
    }

    /// 兼容带因果键的快照调用；因果键只属于事件，不进入窗口状态。
    pub fn snapshot_from_render_ready_with_causation(
        &mut self,
        request: TimelineWindowRequest,
        rows: &[Value],
        _causation_id: Option<String>,
    ) -> Result<(), TimelineStateError> {
        self.replace(request, rows)
    }

    /// 用新权威行更新当前窗口。
    pub fn patch_from_render_ready(
        &mut self,
        request: TimelineWindowRequest,
        rows: &[Value],
        _causation_id: Option<String>,
    ) -> Result<(), TimelineStateError> {
        self.replace(request, rows)
    }

    /// 合并一个已通过持久屏障的远端页。
    pub fn patch_page_from_render_ready(
        &mut self,
        request: TimelineWindowRequest,
        rows: &[Value],
        mutation: TimelinePageMutation,
        _causation_id: Option<String>,
    ) -> Result<(), TimelineStateError> {
        self.merge_page(request, rows, mutation)
    }

    /// 失败只结束本次请求，不改写上一次权威窗口。
    pub fn patch_failed_for_anchor(
        &mut self,
        _channel_id: &str,
        _anchor_message_id: &str,
    ) -> Result<Option<()>, TimelineStateError> {
        Ok(Some(()))
    }

    /// 合并分页行；只有当前定位 token 可以切换活动锚点。
    pub fn merge_page(
        &mut self,
        request: TimelineWindowRequest,
        rows: &[Value],
        mutation: TimelinePageMutation,
    ) -> Result<(), TimelineStateError> {
        let scope = TimelineScope {
            channel_id: request.channel_id.clone(),
            window_token: request.window_token.clone(),
        };
        let mut incoming = rows
            .iter()
            .map(timeline_item)
            .collect::<Result<Vec<_>, _>>()?;
        if !self.is_active_window(scope.channel_id.as_str(), scope.window_token.as_str()) {
            tracing::debug!(
                channel_id = scope.channel_id.as_str(),
                active_window_token = self
                    .active_window_by_channel
                    .get(scope.channel_id.as_str())
                    .map(String::as_str)
                    .unwrap_or_default(),
                stale_window_token = scope.window_token.as_str(),
                slot_count = self.slots.len(),
                attachment_count = self.attachments.len(),
                "timeline page rejected for stale or unattached window"
            );
            return Err(TimelineStateError::StaleWindow {
                channel_id: scope.channel_id,
                window_token: scope.window_token,
            });
        }
        let slot =
            self.slots
                .get_mut(&scope.key())
                .ok_or_else(|| TimelineStateError::StaleWindow {
                    channel_id: scope.channel_id.clone(),
                    window_token: scope.window_token.clone(),
                })?;
        match mutation {
            TimelinePageMutation::Older => {
                incoming.extend(slot.items.clone());
                slot.items = dedup_and_bound(incoming);
            }
            TimelinePageMutation::Newer => {
                slot.items.extend(incoming);
                slot.items = dedup_and_bound(std::mem::take(&mut slot.items));
            }
            TimelinePageMutation::Locate {
                target_message_id,
                activate,
                ..
            } => {
                slot.items = dedup_and_bound(incoming);
                if activate {
                    slot.anchor = TimelineAnchor {
                        message_id: Some(target_message_id),
                        mode: TimelineAnchorMode::Locate,
                    };
                }
            }
        }
        slot.page = request.page;
        self.register_attachment(&scope);
        Ok(())
    }
}

impl TimelineState {
    /// 淘汰频道旧窗口，避免迟到回包再次命中或复活旧分页作用域。
    fn retire_other_windows(&mut self, channel_id: &str, keep_window_token: &str) {
        let keep_key = format!("{channel_id}\u{1f}{keep_window_token}");
        let stale_slot_keys = self
            .slots
            .iter()
            .filter(|(key, slot)| slot.channel_id == channel_id && key.as_str() != keep_key)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        let channel_prefix = format!("{channel_id}\u{1f}");
        for key in stale_slot_keys {
            self.slots.remove(&key);
            self.attachments.remove(&key);
            self.navigation_tokens.remove(&key);
        }
        self.attachments.retain(|key, (attached, token)| {
            attached != channel_id || (token == keep_window_token && key == &keep_key)
        });
        self.navigation_tokens
            .retain(|key, _| !key.starts_with(channel_prefix.as_str()) || key == &keep_key);
        self.active_window_by_channel
            .insert(channel_id.to_string(), keep_window_token.to_string());
    }
}

/// 从 render-ready 行提取分页所需的最小事实。
fn timeline_item(row: &Value) -> Result<TimelineItem, TimelineStateError> {
    let id = row
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or(TimelineStateError::MissingId)?
        .to_string();
    let created_at = row
        .get("createAt")
        .or_else(|| row.get("create_at"))
        .or_else(|| row.get("createdAt"))
        .and_then(Value::as_i64)
        .unwrap_or_default();
    let temporary_id = row
        .get("temporaryId")
        .or_else(|| row.get("temporary_id"))
        .and_then(Value::as_str)
        .unwrap_or(id.as_str())
        .to_string();
    Ok(TimelineItem {
        id,
        created_at,
        temporary_id,
        row: row.clone(),
    })
}

/// 以稳定身份去重并约束窗口大小。
fn dedup_and_bound(items: Vec<TimelineItem>) -> Vec<TimelineItem> {
    let mut by_id = BTreeMap::new();
    for item in items {
        by_id.insert(item.id.clone(), item);
    }
    let mut items: Vec<_> = by_id.into_values().collect();
    items.sort_by_key(|item| (item.created_at, item.temporary_id.clone()));
    if items.len() > MAX_TIMELINE_WINDOW_ITEMS {
        items.drain(..items.len() - MAX_TIMELINE_WINDOW_ITEMS);
    }
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CHANNEL_ID: &str = "chtimeline000000000000000001";
    const OTHER_CHANNEL_ID: &str = "chtimeline000000000000000002";

    /// 构造只含时间线稳定身份的测试行。
    fn row(id: &str, create_at: i64) -> Value {
        json!({
            "id": id,
            "createAt": create_at,
            "temporaryId": format!("tmp-{id}"),
        })
    }

    /// 构造带唯一窗口 token 的 latest 请求。
    fn latest(channel_id: &str, window_token: &str) -> TimelineWindowRequest {
        TimelineWindowRequest::latest_with_window_token(channel_id, window_token)
    }

    /// 同频道重复 replace 只保留第二个窗口，旧 anchor 不再参与分页。
    #[test]
    fn replacing_same_channel_retires_old_window() {
        let mut state = TimelineState::default();
        state
            .replace(latest(CHANNEL_ID, "window-a"), &[row("message-a", 1)])
            .expect("first window replaces");
        state.begin_locate_navigation(CHANNEL_ID, "window-a", "navigation-a");
        state
            .replace(latest(CHANNEL_ID, "window-b"), &[row("message-b", 2)])
            .expect("second window replaces");

        assert_eq!(state.window_counts(), (1, 1));
        assert!(!state.is_active_window(CHANNEL_ID, "window-a"));
        assert!(state.is_active_window(CHANNEL_ID, "window-b"));
        assert!(state
            .paging_context_for_anchor(CHANNEL_ID, "message-a")
            .is_none());
        assert!(state
            .paging_context_for_anchor(CHANNEL_ID, "message-b")
            .is_some());
        assert!(!state.is_current_locate_navigation(CHANNEL_ID, "window-a", "navigation-a"));
    }

    /// 旧窗口的迟到分页不得通过 entry-or-insert 复活已淘汰 slot。
    #[test]
    fn stale_merge_cannot_revive_retired_window() {
        let mut state = TimelineState::default();
        state
            .replace(latest(CHANNEL_ID, "window-a"), &[row("message-a", 1)])
            .expect("first window replaces");
        state
            .replace(latest(CHANNEL_ID, "window-b"), &[row("message-b", 2)])
            .expect("second window replaces");
        let mut stale_request = latest(CHANNEL_ID, "window-a");
        stale_request.page.has_older = true;

        let error = state
            .merge_page(
                stale_request,
                &[row("message-before-a", 0)],
                TimelinePageMutation::Older,
            )
            .expect_err("retired window is rejected");
        assert!(matches!(error, TimelineStateError::StaleWindow { .. }));
        assert_eq!(state.window_counts(), (1, 1));
        assert!(state
            .current_view(&TimelineScope {
                channel_id: CHANNEL_ID.to_string(),
                window_token: "window-a".to_string(),
            })
            .is_none());
    }

    /// 当前活动窗口的 older 页仍可合并并保持单一 attachment。
    #[test]
    fn active_window_accepts_older_page() {
        let mut state = TimelineState::default();
        state
            .replace(
                latest(CHANNEL_ID, "window-current"),
                &[row("message-current", 2)],
            )
            .expect("current window replaces");
        let mut request = latest(CHANNEL_ID, "window-current");
        request.page.has_older = true;
        state
            .merge_page(
                request,
                &[row("message-older", 1)],
                TimelinePageMutation::Older,
            )
            .expect("active older page merges");

        let scope = TimelineScope {
            channel_id: CHANNEL_ID.to_string(),
            window_token: "window-current".to_string(),
        };
        let view = state.current_view(&scope).expect("active view exists");
        assert_eq!(
            view.items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["message-older", "message-current",]
        );
    }

    /// 同 token 的 latest 替换也必须撤销旧 locate 导航，防止旧回包被当作当前导航。
    #[test]
    fn replacing_same_token_clears_old_navigation() {
        let mut state = TimelineState::default();
        state
            .replace(
                latest(CHANNEL_ID, "window-current"),
                &[row("message-old", 1)],
            )
            .expect("first window replaces");
        state.begin_locate_navigation(CHANNEL_ID, "window-current", "navigation-old");
        state
            .replace(
                latest(CHANNEL_ID, "window-current"),
                &[row("message-new", 2)],
            )
            .expect("same-token latest replaces");

        assert!(!state.is_current_locate_navigation(
            CHANNEL_ID,
            "window-current",
            "navigation-old"
        ));
    }

    /// 不同频道的活动窗口互不淘汰。
    #[test]
    fn replacing_one_channel_does_not_affect_another() {
        let mut state = TimelineState::default();
        state
            .replace(latest(CHANNEL_ID, "window-a"), &[row("message-a", 1)])
            .expect("first channel replaces");
        state
            .replace(
                latest(OTHER_CHANNEL_ID, "window-other"),
                &[row("message-other", 1)],
            )
            .expect("second channel replaces");
        state
            .replace(latest(CHANNEL_ID, "window-b"), &[row("message-b", 2)])
            .expect("first channel replaces again");

        assert!(state.is_active_window(OTHER_CHANNEL_ID, "window-other"));
        assert!(state
            .paging_context_for_anchor(OTHER_CHANNEL_ID, "message-other")
            .is_some());
    }

    /// reset 清除 active index、slot、attachment 与导航 token。
    #[test]
    fn reset_invalidates_all_timeline_windows() {
        let mut state = TimelineState::default();
        state
            .replace(
                latest(CHANNEL_ID, "window-current"),
                &[row("message-current", 1)],
            )
            .expect("window replaces");
        state.begin_locate_navigation(CHANNEL_ID, "window-current", "navigation-current");
        state.reset();

        assert_eq!(state.window_counts(), (0, 0));
        assert!(!state.is_active_window(CHANNEL_ID, "window-current"));
        assert!(!state.is_current_locate_navigation(
            CHANNEL_ID,
            "window-current",
            "navigation-current"
        ));
        assert!(state
            .paging_context_for_anchor(CHANNEL_ID, "message-current")
            .is_none());
    }
}
