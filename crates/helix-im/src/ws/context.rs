use crate::state::ImState;
use helix_core::Correlation;

pub(crate) struct ImWsContext<'a> {
    pub(crate) state: &'a mut ImState,
    pub(crate) now_ms: u64,
    pub(crate) api_base_url: &'a str,
    /// 当前登录用户 Id26（host 注入，与 `api_base_url` 同机制）。空串 = 无身份。
    /// 用于 path3 未读 sender 豁免 + 可见性门控、path1 channel `user_id` 列（BLOCKING-1）。
    pub(crate) auth_user_id: &'a str,
    alloc_corr: &'a mut dyn FnMut() -> Correlation,
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
        }
    }

    pub(crate) fn alloc_corr(&mut self) -> Correlation {
        (self.alloc_corr)()
    }
}
