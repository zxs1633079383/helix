use std::collections::BTreeMap;

use crate::state::{ChannelId, ImState};
use helix_core::Correlation;

pub(crate) struct ImWsContext<'a> {
    pub(crate) state: &'a mut ImState,
    pub(crate) now_ms: u64,
    pub(crate) api_base_url: &'a str,
    /// 当前登录用户 Id26（host 注入，与 `api_base_url` 同机制）。空串 = 无身份。
    /// 用于 path3 未读 sender 豁免 + 可见性门控、path1 channel `user_id` 列（BLOCKING-1）。
    pub(crate) auth_user_id: &'a str,
    alloc_corr: &'a mut dyn FnMut() -> Correlation,
    /// WS handler 只能判定某个 gate 已经接受了可见消息；最终的 local-first
    /// 时间线读回必须等 handler 返回后由 `ImModule` 调度，避免在这里复制
    /// query/coverage/correlation 状态机。
    attached_timeline_refreshes: BTreeMap<ChannelId, Option<String>>,
}

impl<'a> ImWsContext<'a> {
    pub(crate) fn new(
        state: &'a mut ImState,
        now_ms: u64,
        api_base_url: &'a str,
        auth_user_id: &'a str,
        alloc_corr: &'a mut dyn FnMut() -> Correlation,
    ) -> Self {
        Self {
            state,
            now_ms,
            api_base_url,
            auth_user_id,
            alloc_corr,
            attached_timeline_refreshes: BTreeMap::new(),
        }
    }

    pub(crate) fn alloc_corr(&mut self) -> Correlation {
        (self.alloc_corr)()
    }

    /// 请求在当前 WS step 完成后重读已 attach 的最新时间线。集合去重保证一次
    /// 连续 gate flush 至多追加一次 readback，不让原始 post payload 越过 Frame 边界。
    pub(crate) fn request_attached_timeline_refresh(&mut self, channel_id: ChannelId) {
        self.attached_timeline_refreshes
            .entry(channel_id)
            .or_insert(None);
    }

    pub(crate) fn request_attached_timeline_refresh_with_causation(
        &mut self,
        channel_id: ChannelId,
        causation_id: Option<String>,
    ) {
        let entry = self
            .attached_timeline_refreshes
            .entry(channel_id)
            .or_insert(None);
        if causation_id.is_some() {
            *entry = causation_id;
        }
    }

    pub(crate) fn take_attached_timeline_refreshes(
        &mut self,
    ) -> BTreeMap<ChannelId, Option<String>> {
        std::mem::take(&mut self.attached_timeline_refreshes)
    }

    /// 拆借：同时拿到 `&mut ImState` 与 corr 分配器（split borrow），供 SyncScheduler
    /// `enqueue_and_drain` 用——避免 `|| ctx.alloc_corr()` 闭包整借 ctx 与 `ctx.state` 冲突。
    pub(crate) fn split_state_alloc(
        &mut self,
    ) -> (&mut ImState, &mut (dyn FnMut() -> Correlation + 'a)) {
        (self.state, self.alloc_corr)
    }
}
