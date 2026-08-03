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
}

/// 跨 Tick 保存时间线分页与定位状态。
#[derive(Debug, Default)]
pub struct TimelineState {
    slots: BTreeMap<String, TimelineWindow>,
    attachments: BTreeMap<String, (String, String)>,
    navigation_tokens: BTreeMap<String, String>,
}

impl TimelineState {
    /// 清除登录身份绑定的全部窗口状态。
    pub fn reset(&mut self) {
        self.slots.clear();
        self.attachments.clear();
        self.navigation_tokens.clear();
    }

    /// 注册一个已附着窗口。
    pub fn register_attachment(&mut self, scope: &TimelineScope) {
        self.attachments.insert(
            scope.key(),
            (scope.channel_id.clone(), scope.window_token.clone()),
        );
    }

    /// 判断窗口是否已附着。
    pub fn is_attached(&self, scope: &TimelineScope) -> bool {
        self.attachments.contains_key(&scope.key())
    }

    /// 返回当前窗口状态。
    pub fn current_view(&self, scope: &TimelineScope) -> Option<&TimelineWindow> {
        self.slots.get(&scope.key())
    }

    /// 返回包含目标消息的唯一频道。
    pub fn located_target_channel(&self, message_id: &str) -> Option<&str> {
        let mut matches = self
            .slots
            .values()
            .filter(|slot| slot.items.iter().any(|item| item.id == message_id));
        let first = matches.next()?;
        matches
            .next()
            .is_none()
            .then_some(first.channel_id.as_str())
    }

    /// 返回频道唯一已附着窗口及当前行数。
    pub fn unique_attached_window_for_channel(&self, channel_id: &str) -> Option<(String, usize)> {
        let mut matches = self.attachments.values().filter_map(|(attached, token)| {
            if attached != channel_id {
                return None;
            }
            let scope = TimelineScope {
                channel_id: attached.clone(),
                window_token: token.clone(),
            };
            Some((
                token.clone(),
                self.current_view(&scope).map_or(0, |view| view.items.len()),
            ))
        });
        let first = matches.next()?;
        (matches.next().is_none()).then_some(first)
    }

    /// 返回锚点所在窗口的页事实与稳定复合游标。
    pub fn paging_context_for_anchor(
        &self,
        channel_id: &str,
        anchor_message_id: &str,
    ) -> Option<(WindowPage, usize, TimelineEntityKey)> {
        let mut matches = self.slots.values().filter_map(|slot| {
            if slot.channel_id != channel_id {
                return None;
            }
            let message = slot
                .items
                .iter()
                .find(|item| item.id == anchor_message_id)?;
            Some((
                slot.page.clone(),
                slot.items.len(),
                TimelineEntityKey {
                    create_at: message.created_at,
                    temporary_id: message.temporary_id.clone(),
                },
            ))
        });
        let first = matches.next()?;
        (matches.next().is_none() && !first.2.temporary_id.is_empty()).then_some(first)
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
        let scope = TimelineScope {
            channel_id: request.channel_id.clone(),
            window_token: request.window_token.clone(),
        };
        let items = rows
            .iter()
            .map(timeline_item)
            .collect::<Result<Vec<_>, _>>()?;
        self.register_attachment(&scope);
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
        let slot = self
            .slots
            .entry(scope.key())
            .or_insert_with(|| TimelineWindow {
                channel_id: request.channel_id.clone(),
                page: request.page.clone(),
                anchor: TimelineAnchor::default(),
                items: Vec::new(),
            });
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
